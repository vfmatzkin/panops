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
    /// Live system-audio level (dBFS) from the preview stream. `silenceFloorDb` = silence.
    @Published private(set) var systemDb: Float = silenceFloorDb
    /// Live microphone level (dBFS) from the preview stream (macOS 15+).
    @Published private(set) var micDb: Float = silenceFloorDb
    /// Set when the picker returns a selection we can't map to a serializable
    /// target — an unknown style, or macOS < 15.2 where the chosen
    /// window/display/app can't be read back off the filter. The New Recording
    /// sheet surfaces it and no recording starts against a substituted source.
    /// Cleared on the next mappable pick.
    @Published private(set) var selectionError: String?

    private let sampleQueue = DispatchQueue(label: "ar.tzk.panops.preview.samples")
    private let audioQueue = DispatchQueue(label: "ar.tzk.panops.preview.audio")
    private lazy var observer = CapturePickerObserver(controller: self)
    /// Thread-safe `SCStream` storage held `nonisolated(unsafe)`. `SCStream`'s
    /// async lifecycle methods (`updateConfiguration`/`stopCapture`) are
    /// themselves nonisolated, so reading this slot to call them must not pull a
    /// main-actor-region value into a background execution context (Swift 6.1
    /// region isolation flags that). The slot is only ever mutated on the main
    /// actor, so the unsynchronized access is safe in practice.
    private nonisolated(unsafe) var stream: SCStream?
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
        systemDb = silenceFloorDb
        micDb = silenceFloorDb
        selectionError = nil
    }

    // MARK: - Picker observer callbacks (hopped to the main actor)

    fileprivate func didPick(_ filter: SCContentFilter) {
        guard let dto = Self.captureTarget(from: filter) else {
            rejectUnsupportedSelection()
            return
        }
        currentFilter = filter
        selectionError = nil
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

    /// Reject a picker selection we can't map to a serializable target: tear down
    /// any running preview, clear the selection, and surface a user-facing error.
    /// Substituting the primary display (the old silent fallback) would start a
    /// recording against the wrong source, so refuse instead.
    private func rejectUnsupportedSelection() {
        // Invalidate any in-flight restart and stop the prior stream so no stale
        // preview implies this rejected selection is live.
        previewEpoch += 1
        let stopping = stream
        stream = nil
        output = nil
        Task { try? await stopping?.stopCapture() }
        // Don't retain the unmappable filter — Retry should re-open the picker,
        // not re-attempt a selection we still can't map.
        currentFilter = nil
        selectionError = "That selection isn't supported — pick a window, display, or app."
        target = nil
        isDisplayTarget = false
        isCropped = false
        sourceRect = nil
        nativePixelHeight = 0
        sourceContentSize = .zero
        state = .idle
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
        // Build the config before the await: `makeConfig` is nonisolated and
        // returns a fresh (disconnected-region) value, so passing it to the
        // nonisolated `updateConfiguration` doesn't send main-actor state.
        let config = makeConfig(for: filter, sourceRect: sourceRect)
        do {
            try await stream.updateConfiguration(config)
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
        let config = makeConfig(for: filter, sourceRect: sourceRect)
        let newOutput = PreviewStreamOutput(displayLayer: displayLayer, onAudioLevel: makeLevelSink())
        do {
            // Create, wire, and start the stream off the main actor so its
            // nonisolated async lifecycle methods never receive a main-actor
            // value. `filter`/`config` cross in a Sendable box; the started
            // stream comes back in a disconnected region we can adopt or stop.
            let started = try await Self.makeStartedStream(
                filter: UncheckedSendableBox(filter),
                config: UncheckedSendableBox(config),
                output: newOutput,
                sampleQueue: sampleQueue,
                audioQueue: audioQueue
            )
            // If we lost the race during startCapture (newer pick or teardown),
            // discard this stream rather than overwriting the current state.
            guard epoch == previewEpoch else {
                try? await started.stopCapture()
                return
            }
            stream = started
            output = newOutput
            state = .live
        } catch {
            // Only surface the failure if we're still the current attempt.
            guard epoch == previewEpoch else { return }
            state = Self.mapStartError(error)
        }
    }

    /// Build, wire, and start an `SCStream` entirely off the main actor. The
    /// stream and its non-`Sendable` `filter`/`config` are created and consumed
    /// inside this `nonisolated` body, so `addStreamOutput`/`startCapture` (all
    /// nonisolated) never receive a main-actor-isolated value. The
    /// `@unchecked Sendable` `output` and the Sendable queues cross in freely;
    /// the started stream returns in a disconnected region the caller adopts.
    private nonisolated static func makeStartedStream(
        filter: UncheckedSendableBox<SCContentFilter>,
        config: UncheckedSendableBox<SCStreamConfiguration>,
        output: PreviewStreamOutput,
        sampleQueue: DispatchQueue,
        audioQueue: DispatchQueue
    ) async throws -> SCStream {
        let stream = SCStream(filter: filter.value, configuration: config.value, delegate: nil)
        try stream.addStreamOutput(output, type: .screen, sampleHandlerQueue: sampleQueue)
        try stream.addStreamOutput(output, type: .audio, sampleHandlerQueue: audioQueue)
        if #available(macOS 15.0, *) {
            try stream.addStreamOutput(output, type: .microphone, sampleHandlerQueue: audioQueue)
        }
        try await stream.startCapture()
        return stream
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

    /// `nonisolated` so its fresh `SCStreamConfiguration` lands in a
    /// disconnected region and can be sent to the nonisolated stream lifecycle
    /// methods without dragging main-actor state along. It reads only the filter
    /// and the caller-supplied crop rect, never mutable instance state.
    private nonisolated func makeConfig(for filter: SCContentFilter, sourceRect: CGRect?) -> SCStreamConfiguration {
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

    /// Extract a serializable target from the picker's opaque filter, or `nil`
    /// when the selection can't be mapped to a supported Display/Window/App. The
    /// exact ids need `includedWindows`/`includedDisplays`/`includedApplications`
    /// (macOS 15.2+); on older systems only `style` is readable, so every
    /// selection is unmappable and the caller rejects it rather than silently
    /// substituting a default target.
    private static func captureTarget(from filter: SCContentFilter) -> CaptureTargetDTO? {
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
        return nil
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
