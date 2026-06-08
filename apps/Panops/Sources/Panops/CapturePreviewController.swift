import AppKit
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
    /// Source size in display points — the space a drag-crop rectangle maps onto.
    @Published private(set) var sourceContentSize: CGSize = .zero
    /// Whether a crop region is currently applied to a display target.
    @Published private(set) var isCropped = false
    /// Live system-audio level (dBFS) from the preview stream. -120 = silence.
    @Published private(set) var systemDb: Float = -120
    /// Live microphone level (dBFS) from the preview stream (macOS 15+).
    @Published private(set) var micDb: Float = -120

    private let sampleQueue = DispatchQueue(label: "ar.tzk.panops.preview.samples")
    private let audioQueue = DispatchQueue(label: "ar.tzk.panops.preview.audio")
    private lazy var observer = CapturePickerObserver(controller: self)
    private var stream: SCStream?
    private var output: PreviewStreamOutput?
    /// Bumped on every preview restart and on teardown. A `restartPreview` whose
    /// epoch is stale when `startCapture()` returns lost the race (a newer pick or
    /// a teardown happened mid-suspension) and must discard its stream instead of
    /// assigning it — otherwise overlapping picks leak streams and a teardown is
    /// silently overwritten by a stream that keeps capturing.
    private var previewEpoch = 0
    private var currentFilter: SCContentFilter?
    /// Sub-rectangle to preview, in display points; set by the drag-crop overlay.
    private var sourceRect: CGRect?
    /// Display id of the current display target, for building a `region` target.
    private var currentDisplayID: UInt32 = 0

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

    /// Open System Settings straight to Privacy → Screen Recording so the user
    /// can grant the app permission, then come back and Retry.
    func openScreenRecordingSettings() {
        guard let url = URL(
            string: "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        ) else { return }
        NSWorkspace.shared.open(url)
    }

    /// Re-attempt the preview when the app returns to the foreground, but only
    /// while blocked on permission — the user may have just granted it.
    func retryIfPermissionDenied() {
        if state == .permissionDenied { retry() }
    }

    /// Stop the picker observer + preview stream and reset to a clean state so
    /// the next sheet opens fresh. Call on sheet cancel and when recording ends.
    func teardown() {
        // Invalidate any in-flight restart so a stream that finishes starting
        // after teardown stops itself instead of resurrecting the preview.
        previewEpoch += 1
        SCContentSharingPicker.shared.remove(observer)
        let stopping = stream
        stream = nil
        output = nil
        currentFilter = nil
        Task { try? await stopping?.stopCapture() }
        state = .idle
        target = nil
        isDisplayTarget = false
        isCropped = false
        sourceRect = nil
        nativePixelHeight = 0
        sourceContentSize = .zero
        systemDb = -120
        micDb = -120
    }

    // MARK: - Picker observer callbacks (hopped to the main actor)

    fileprivate func didPick(_ filter: SCContentFilter) {
        currentFilter = filter
        let dto = Self.captureTarget(from: filter)
        target = dto
        if case let .display(displayID) = dto {
            isDisplayTarget = true
            currentDisplayID = displayID
        } else {
            isDisplayTarget = false
        }
        // Native pixel height straight off the filter — avoids `SCShareableContent`
        // and the global screen-recording prompt it triggers.
        nativePixelHeight = Int((filter.contentRect.height * CGFloat(filter.pointPixelScale)).rounded())
        sourceContentSize = filter.contentRect.size
        // A fresh source clears any prior crop.
        sourceRect = nil
        isCropped = false
        Task { await restartPreview(with: filter) }
    }

    // MARK: - Drag-crop region (display targets only)

    /// Apply a drag-crop rectangle (display points) to a display target: switch
    /// the target to a `region` and live-reframe the preview to it.
    func applyCrop(_ rect: CaptureRect) {
        guard isDisplayTarget else { return }
        sourceRect = CGRect(x: Int(rect.x), y: Int(rect.y), width: Int(rect.w), height: Int(rect.h))
        target = .region(displayID: currentDisplayID, x: rect.x, y: rect.y, w: rect.w, h: rect.h)
        isCropped = true
        // The cropped region — not the whole display — is now the recorded source,
        // so the resolution preset must resolve against the region's pixel height.
        if let filter = currentFilter {
            nativePixelHeight = Int((CGFloat(rect.h) * CGFloat(filter.pointPixelScale)).rounded())
        }
        Task { await updateStreamConfig() }
    }

    /// Drop the crop and reframe the preview back to the full display.
    func clearCrop() {
        sourceRect = nil
        isCropped = false
        target = .display(displayID: currentDisplayID)
        // Restore the full-display pixel height for resolution math.
        if let filter = currentFilter {
            nativePixelHeight = Int((filter.contentRect.height * CGFloat(filter.pointPixelScale)).rounded())
        }
        Task { await updateStreamConfig() }
    }

    private func updateStreamConfig() async {
        guard let filter = currentFilter, let stream else { return }
        do {
            try await stream.updateConfiguration(makeConfig(for: filter))
        } catch {
            state = Self.mapStartError(error)
        }
    }

    fileprivate func didCancelPick() {
        // Dismissing the picker keeps any prior selection; nothing to change.
    }

    fileprivate func pickerFailed(_ error: Error) {
        state = Self.mapStartError(error)
    }

    // MARK: - Preview stream

    private func restartPreview(with filter: SCContentFilter) async {
        previewEpoch += 1
        let epoch = previewEpoch
        await stopStream()
        // A newer pick (or a teardown) superseded us while stopping the old stream.
        guard epoch == previewEpoch else { return }
        state = .starting
        let config = makeConfig(for: filter)
        let newOutput = PreviewStreamOutput(displayLayer: displayLayer, onAudioLevel: makeLevelSink())
        let newStream = SCStream(filter: filter, configuration: config, delegate: nil)
        do {
            try newStream.addStreamOutput(newOutput, type: .screen, sampleHandlerQueue: sampleQueue)
            try newStream.addStreamOutput(newOutput, type: .audio, sampleHandlerQueue: audioQueue)
            if #available(macOS 15.0, *) {
                try newStream.addStreamOutput(newOutput, type: .microphone, sampleHandlerQueue: audioQueue)
            }
            try await newStream.startCapture()
            // If we lost the race during startCapture (newer pick or teardown),
            // discard this stream rather than overwriting the current state.
            guard epoch == previewEpoch else {
                try? await newStream.stopCapture()
                return
            }
            stream = newStream
            output = newOutput
            state = .live
        } catch {
            // Only surface the failure if we're still the current attempt.
            guard epoch == previewEpoch else { return }
            state = Self.mapStartError(error)
        }
    }

    /// A `@Sendable` audio-level sink that hops each per-buffer dBFS reading back
    /// to the main actor and publishes it as the system or mic meter level.
    private func makeLevelSink() -> @Sendable (Float, Bool) -> Void {
        { [weak self] db, isMic in
            Task { @MainActor in
                guard let self else { return }
                if isMic { self.micDb = db } else { self.systemDb = db }
            }
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
        // Capture audio so the meters reflect what's actually flowing. This is
        // the app's own monitor stream; the sidecar records independently.
        config.capturesAudio = true
        config.excludesCurrentProcessAudio = true
        config.sampleRate = 16_000
        config.channelCount = 1
        if #available(macOS 15.0, *) {
            config.captureMicrophone = true
        }
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

/// Receives preview frames + audio on background queues: complete video frames
/// go to the display layer, audio buffers are reduced to a per-buffer dBFS level
/// and reported via `onAudioLevel`. `@unchecked Sendable`: the layer is touched
/// only from the serial sample queue; `onAudioLevel` is itself `@Sendable`.
private final class PreviewStreamOutput: NSObject, SCStreamOutput, @unchecked Sendable {
    private let displayLayer: AVSampleBufferDisplayLayer
    private let onAudioLevel: @Sendable (Float, Bool) -> Void

    init(displayLayer: AVSampleBufferDisplayLayer, onAudioLevel: @escaping @Sendable (Float, Bool) -> Void) {
        self.displayLayer = displayLayer
        self.onAudioLevel = onAudioLevel
        super.init()
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard CMSampleBufferDataIsReady(sampleBuffer) else { return }
        switch type {
        case .screen:
            guard Self.isCompleteFrame(sampleBuffer) else { return }
            if displayLayer.status == .failed { displayLayer.flush() }
            displayLayer.enqueue(sampleBuffer)
        case .audio:
            if let samples = Self.samples(from: sampleBuffer) {
                onAudioLevel(rmsDbFS(samples), false)
            }
        case .microphone:
            if let samples = Self.samples(from: sampleBuffer) {
                onAudioLevel(rmsDbFS(samples), true)
            }
        default:
            break
        }
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

    /// Extract mono Float samples from a ScreenCaptureKit audio buffer (channel 0).
    private static func samples(from sampleBuffer: CMSampleBuffer) -> [Float]? {
        guard let formatDesc = CMSampleBufferGetFormatDescription(sampleBuffer),
            let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(formatDesc),
            let format = AVAudioFormat(streamDescription: asbd)
        else { return nil }
        let frames = CMSampleBufferGetNumSamples(sampleBuffer)
        guard frames > 0,
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames))
        else { return nil }
        buffer.frameLength = AVAudioFrameCount(frames)
        let status = CMSampleBufferCopyPCMDataIntoAudioBufferList(
            sampleBuffer, at: 0, frameCount: Int32(frames), into: buffer.mutableAudioBufferList
        )
        guard status == noErr, let channel = buffer.floatChannelData else { return nil }
        return Array(UnsafeBufferPointer(start: channel[0], count: Int(frames)))
    }
}

/// Carries a non-`Sendable` value across an actor hop where the value is known
/// to be effectively immutable.
private struct UncheckedSendableBox<T>: @unchecked Sendable {
    let value: T
    init(_ value: T) { self.value = value }
}
