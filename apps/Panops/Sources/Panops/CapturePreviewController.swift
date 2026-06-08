import AVFoundation
import CoreMedia
import Foundation
import ScreenCaptureKit

/// Lifecycle of the in-app capture preview, surfaced to the New Recording sheet.
enum CapturePreviewState: Equatable {
    /// No source picked yet.
    case idle
    /// A source is picked; the preview stream is starting.
    case starting
    /// Frames are flowing into the preview layer.
    case live
    /// The app process lacks Screen Recording permission.
    case permissionDenied
    /// The preview stream failed to start for another reason.
    case failed(String)
}

/// Owns the system `SCContentSharingPicker` and a low-cost in-app `SCStream`
/// that renders a smooth preview of exactly what will be recorded. Frames feed
/// an `AVSampleBufferDisplayLayer` (hardware-accelerated). The controller
/// publishes the serializable target + the source's native pixel height so the
/// sheet can resolve a resolution preset and (later) a drag-crop region.
///
/// "App previews, sidecar records": this is the app's own stream, separate from
/// the capture sidecar that records the identical selection to disk.
@MainActor
final class CapturePreviewController: NSObject, ObservableObject {
    /// The layer the preview view hosts. Frames are enqueued from the stream's
    /// background sample callback (the renderer tolerates background enqueues).
    let displayLayer = AVSampleBufferDisplayLayer()

    @Published private(set) var state: CapturePreviewState = .idle
    /// Serializable target from the current picker selection, or `nil` until a
    /// source is chosen.
    @Published private(set) var target: CaptureTargetDTO?
    /// Native pixel height of the picked source, for resolution math. `0` = unknown.
    @Published private(set) var nativePixelHeight: Int = 0
    /// True only for whole-display targets — drag-crop applies to those alone.
    @Published private(set) var isDisplayTarget = false

    private let sampleQueue = DispatchQueue(label: "ar.tzk.panops.preview.samples")
    private lazy var observer = CapturePickerObserver(controller: self)
    private var stream: SCStream?
    private var output: PreviewStreamOutput?
    private var currentFilter: SCContentFilter?
    /// Sub-rectangle to preview, in display points; set by the drag-crop overlay.
    private var sourceRect: CGRect?

    override init() {
        super.init()
        displayLayer.videoGravity = .resizeAspect
    }

    /// Open the system content-sharing picker. The chosen filter arrives via the
    /// observer and drives the preview.
    func presentPicker() {
        let picker = SCContentSharingPicker.shared
        var config = SCContentSharingPickerConfiguration()
        config.allowedPickerModes = [.singleWindow, .singleApplication, .singleDisplay]
        picker.defaultConfiguration = config
        picker.add(observer)
        picker.isActive = true
        picker.present()
    }

    /// Re-attempt the preview after a permission grant or transient failure.
    func retry() {
        if let filter = currentFilter {
            Task { await restartPreview(with: filter) }
        } else {
            presentPicker()
        }
    }

    /// Stop the picker observer + preview stream. Call when the sheet closes.
    func teardown() {
        SCContentSharingPicker.shared.remove(observer)
        let stopping = stream
        stream = nil
        output = nil
        Task { try? await stopping?.stopCapture() }
    }

    // MARK: - Picker observer callbacks (hopped to the main actor)

    fileprivate func didPick(_ filter: SCContentFilter) {
        currentFilter = filter
        let dto = Self.captureTarget(from: filter)
        target = dto
        if case .display = dto { isDisplayTarget = true } else { isDisplayTarget = false }
        // Native pixel height straight off the filter — avoids `SCShareableContent`
        // and the global screen-recording prompt it triggers.
        nativePixelHeight = Int((filter.contentRect.height * CGFloat(filter.pointPixelScale)).rounded())
        sourceRect = nil
        Task { await restartPreview(with: filter) }
    }

    fileprivate func didCancelPick() {
        // Dismissing the picker keeps any prior selection; nothing to change.
    }

    fileprivate func pickerFailed(_ error: Error) {
        state = Self.mapStartError(error)
    }

    // MARK: - Preview stream

    private func restartPreview(with filter: SCContentFilter) async {
        await stopStream()
        state = .starting
        let config = makeConfig(for: filter)
        let newOutput = PreviewStreamOutput(displayLayer: displayLayer)
        let newStream = SCStream(filter: filter, configuration: config, delegate: nil)
        do {
            try newStream.addStreamOutput(newOutput, type: .screen, sampleHandlerQueue: sampleQueue)
            try await newStream.startCapture()
            stream = newStream
            output = newOutput
            state = .live
        } catch {
            state = Self.mapStartError(error)
        }
    }

    private func stopStream() async {
        guard let stopping = stream else { return }
        stream = nil
        output = nil
        try? await stopping.stopCapture()
    }

    private func makeConfig(for filter: SCContentFilter) -> SCStreamConfiguration {
        let scale = CGFloat(filter.pointPixelScale)
        let nativeW = max(filter.contentRect.width * scale, 1)
        let nativeH = max(filter.contentRect.height * scale, 1)
        // Cap preview width so the preview stream stays cheap; keep aspect.
        let previewW = min(CGFloat(1280), nativeW)
        let previewH = previewW * (nativeH / nativeW)

        let config = SCStreamConfiguration()
        config.width = max(Int(previewW.rounded()), 2)
        config.height = max(Int(previewH.rounded()), 2)
        config.minimumFrameInterval = CMTime(value: 1, timescale: 30)
        config.pixelFormat = kCVPixelFormatType_32BGRA
        config.queueDepth = 5
        config.showsCursor = true
        config.scalesToFit = true
        if let rect = sourceRect {
            config.sourceRect = rect
        }
        return config
    }

    // MARK: - Helpers

    private static func mapStartError(_ error: Error) -> CapturePreviewState {
        // SCStreamError 1001 = screen-recording TCC denial (see Recorder.swift).
        if let sc = error as? SCStreamError, Int(sc.code.rawValue) == 1001 {
            return .permissionDenied
        }
        return .failed(error.localizedDescription)
    }

    /// Extract a serializable target from the picker's opaque filter. The exact
    /// ids need `includedWindows`/`includedDisplays`/`includedApplications`
    /// (macOS 15.2+); on older systems only `style` is readable, so window/app
    /// selections fall back to the primary display.
    private static func captureTarget(from filter: SCContentFilter) -> CaptureTargetDTO {
        if #available(macOS 15.2, *) {
            switch filter.style {
            case .window:
                if let window = filter.includedWindows.first {
                    return .window(windowID: UInt32(window.windowID))
                }
            case .application:
                if let app = filter.includedApplications.first {
                    return .app(bundleID: app.bundleIdentifier)
                }
            case .display:
                if let display = filter.includedDisplays.first {
                    return .display(displayID: display.displayID)
                }
            default:
                break
            }
        }
        return .display(displayID: 0)
    }
}

/// Bridges the system picker's Objective-C observer callbacks (delivered off the
/// main actor) onto the `@MainActor` controller. The non-`Sendable` filter/error
/// are immutable post-creation, so the unchecked box hop is safe.
private final class CapturePickerObserver: NSObject, SCContentSharingPickerObserver {
    weak var controller: CapturePreviewController?

    init(controller: CapturePreviewController) {
        self.controller = controller
        super.init()
    }

    func contentSharingPicker(
        _ picker: SCContentSharingPicker, didUpdateWith filter: SCContentFilter, for stream: SCStream?
    ) {
        let box = UncheckedSendableBox(filter)
        let controller = controller
        Task { @MainActor in controller?.didPick(box.value) }
    }

    func contentSharingPicker(_ picker: SCContentSharingPicker, didCancelFor stream: SCStream?) {
        let controller = controller
        Task { @MainActor in controller?.didCancelPick() }
    }

    func contentSharingPickerStartDidFailWithError(_ error: any Error) {
        let box = UncheckedSendableBox(error)
        let controller = controller
        Task { @MainActor in controller?.pickerFailed(box.value) }
    }
}

/// Receives preview frames on a background queue and enqueues complete ones into
/// the display layer. `@unchecked Sendable`: the only shared state is the layer,
/// touched solely from the single serial sample queue.
private final class PreviewStreamOutput: NSObject, SCStreamOutput, @unchecked Sendable {
    private let displayLayer: AVSampleBufferDisplayLayer

    init(displayLayer: AVSampleBufferDisplayLayer) {
        self.displayLayer = displayLayer
        super.init()
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen,
            CMSampleBufferDataIsReady(sampleBuffer),
            Self.isCompleteFrame(sampleBuffer)
        else { return }
        if displayLayer.status == .failed { displayLayer.flush() }
        displayLayer.enqueue(sampleBuffer)
    }

    /// A `.screen` frame is renderable only when ScreenCaptureKit marks it
    /// `.complete`; idle/blank frames repeat the prior image and are skipped.
    private static func isCompleteFrame(_ sampleBuffer: CMSampleBuffer) -> Bool {
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false)
            as? [[SCStreamFrameInfo: Any]],
            let info = attachments.first,
            let raw = info[.status] as? Int,
            let status = SCFrameStatus(rawValue: raw)
        else { return false }
        return status == .complete
    }
}

/// Carries a non-`Sendable` value across an actor hop where the value is known
/// to be effectively immutable.
private struct UncheckedSendableBox<T>: @unchecked Sendable {
    let value: T
    init(_ value: T) { self.value = value }
}
