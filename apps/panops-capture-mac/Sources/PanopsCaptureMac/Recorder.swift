import AVFoundation
import CoreMedia
import Foundation
import ScreenCaptureKit

/// Pure routing decision: which audio tracks to open for a given
/// `audio_sources` mode. Kept side-effect-free so it is unit-testable without
/// ScreenCaptureKit. Unknown values default to `system_and_mic` (the engine
/// only ever sends the three canonical strings).
struct TrackPlan {
    let wantsSystem: Bool
    let wantsMic: Bool

    init(audioSources: String) {
        switch audioSources {
        case "mic_only": (wantsSystem, wantsMic) = (false, true)
        case "system_only": (wantsSystem, wantsMic) = (true, false)
        default: (wantsSystem, wantsMic) = (true, true)   // system_and_mic
        }
    }
}

/// Quantize 16 kHz mono Float32 samples (range nominally [-1, 1]) to 16-bit
/// PCM, clamping out-of-range values to the Int16 limits. This is the final
/// step of the per-source resample path (AVAudioConverter does the rate +
/// channel conversion; this does the format quantization) and is pure so the
/// resample math is unit-testable.
func floatToInt16(_ samples: [Float]) -> [Int16] {
    samples.map { sample in
        // Scale by 32768 so the full negative range is used (-1.0 → -32768),
        // then clamp the positive side to Int16.max (32767).
        let scaled = (sample * 32768.0).rounded()
        if scaled >= 32767.0 { return Int16.max }
        if scaled <= -32768.0 { return Int16.min }
        return Int16(scaled)
    }
}

/// Owns the `SCStream` and writes each requested audio source to its own
/// 16 kHz mono WAV — **no cross-source mixing** (slice 11, Decision §2).
/// System audio (`.audio`) → `system.wav`; microphone (`.microphone`) →
/// `mic.wav`. The `.screen` output is routed to a `Screenshotter`.
///
/// `@unchecked Sendable`: the `SCStreamOutput` callback fires on a background
/// dispatch queue; all mutable state (writers, converters) is guarded by
/// `lock`, so cross-thread access is safe despite the class not being
/// structurally `Sendable`.
final class Recorder: NSObject, SCStreamOutput, @unchecked Sendable {
    /// Target format for both tracks: 16 kHz, mono, deinterleaved Float32.
    static let outputFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 16_000, channels: 1, interleaved: false
    )!

    private let plan: TrackPlan
    private let videoPath: String?
    private let lock = NSLock()
    private var stream: SCStream?
    private var systemWriter: WavWriter?
    private var micWriter: WavWriter?
    private var systemConverter: AVAudioConverter?
    private var micConverter: AVAudioConverter?
    private var videoWriter: VideoWriter?
    private let sampleQueue = DispatchQueue(label: "ar.tzk.panops.capture.audio")
    private(set) var startedAtMs: UInt64 = 0

    init(plan: TrackPlan, systemPath: String?, micPath: String?, videoPath: String?) throws {
        self.plan = plan
        self.videoPath = videoPath
        super.init()

        guard !plan.wantsSystem || systemPath != nil else {
            throw CaptureFailure.invalidParams("missing system_audio_path for requested system audio")
        }
        guard !plan.wantsMic || micPath != nil else {
            throw CaptureFailure.invalidParams("missing mic_audio_path for requested microphone audio")
        }

        // Build writers in a temp array first, then assign only if all succeed
        // This prevents FileHandle leak if one track initialization fails
        var writers: [String: WavWriter] = [:]
        if plan.wantsSystem, let p = systemPath {
            let writer = try WavWriter(url: URL(fileURLWithPath: p))
            writers["system"] = writer
        }
        if plan.wantsMic, let p = micPath {
            let writer = try WavWriter(url: URL(fileURLWithPath: p))
            writers["mic"] = writer
        }

        self.systemWriter = writers["system"]
        self.micWriter = writers["mic"]
    }

    /// Configure + start the `SCStream`. The optional `screenshotter` is added
    /// as the `.screen` output (frame-tap, avoiding a second stream).
    func start(screenshotter: Screenshotter?) async throws {
        let content: SCShareableContent
        do {
            content = try await SCShareableContent.current
        } catch {
            throw Self.mapSCStreamError(error)
        }
        guard let display = content.displays.first else {
            throw CaptureFailure.noDisplay
        }
        let filter = SCContentFilter(display: display, excludingWindows: [])

        let config = SCStreamConfiguration()
        config.capturesAudio = plan.wantsSystem
        config.captureMicrophone = plan.wantsMic
        if plan.wantsMic {
            // nil = system default input device (slice 11 captures the user's
            // default mic; device selection is not exposed yet).
            config.microphoneCaptureDeviceID = nil
        }
        config.excludesCurrentProcessAudio = true
        config.sampleRate = 16_000        // request 16 kHz where supported; still resample defensively
        config.channelCount = 1
        // Screen frames feed the screenshotter. Cadence is bounded by the
        // minimum frame interval; the screenshotter dedups beyond that.
        config.width = display.width
        config.height = display.height
        config.pixelFormat = kCVPixelFormatType_32BGRA
        config.queueDepth = 5
        // Frame cadence: a video recording needs a smooth rate, but the
        // screenshotter alone wants only its (coarse) interval. The
        // screenshotter keeps its own internal interval gate either way, so
        // raising the rate for video does not change screenshot cadence.
        if videoPath != nil {
            config.minimumFrameInterval = VideoWriter.frameInterval
        } else if let s = screenshotter {
            config.minimumFrameInterval = CMTime(value: Int64(s.intervalMs), timescale: 1000)
        }

        // Build the video writer up front (when requested) so a setup failure is
        // visible before the stream starts. Once running, writer errors are
        // best-effort: audio + screenshots continue regardless (see VideoWriter).
        var writer: VideoWriter?
        if let path = videoPath {
            do {
                writer = try VideoWriter(
                    url: URL(fileURLWithPath: path), width: display.width, height: display.height
                )
            } catch {
                FileHandle.standardError.write(
                    Data("video writer init failed; recording disabled: \(error)\n".utf8))
            }
        }

        let stream = SCStream(filter: filter, configuration: config, delegate: nil)
        if plan.wantsSystem {
            try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: sampleQueue)
        }
        if plan.wantsMic {
            try stream.addStreamOutput(self, type: .microphone, sampleHandlerQueue: sampleQueue)
        }
        if let s = screenshotter {
            // Tap the EXISTING `.screen` callback: the screenshotter forwards
            // each complete frame to the video writer. No second stream/output.
            if let w = writer {
                s.videoTap = { [weak w] sample in
                    guard VideoWriter.isCompleteFrame(sample) else { return }
                    w?.appendVideo(sample)
                }
            }
            try stream.addStreamOutput(s, type: .screen, sampleHandlerQueue: s.queue)
        }
        do {
            try await stream.startCapture()
        } catch {
            throw Self.mapSCStreamError(error)
        }

        let now = UInt64(Date().timeIntervalSince1970 * 1000)
        // Scoped `withLock`: NSLock's bare lock()/unlock() are unavailable from
        // async contexts (holding a lock across a suspension is unsound).
        lock.withLock {
            self.stream = stream
            self.startedAtMs = now
            self.videoWriter = writer
        }
        screenshotter?.markStarted(atMs: now)
    }

    /// Stop the stream, finalize each open WAV, and report the track paths +
    /// elapsed duration.
    func stop() async throws -> (system: String?, mic: String?, durationMs: UInt64) {
        let stream = lock.withLock { self.stream }
        try await stream?.stopCapture()

        // Finalize the recording after the stream is fully stopped (no more
        // sample callbacks). Best-effort: the WAV finalize below runs regardless.
        let writer = lock.withLock { self.videoWriter }
        await writer?.finish()

        return try lock.withLock {
            try systemWriter?.finalize()
            try micWriter?.finalize()
            let endMs = UInt64(Date().timeIntervalSince1970 * 1000)
            let durationMs = endMs >= startedAtMs ? endMs - startedAtMs : 0
            return (systemWriter?.url.path, micWriter?.url.path, durationMs)
        }
    }

    // MARK: - SCStreamOutput

    func stream(
        _ stream: SCStream,
        didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
        of type: SCStreamOutputType
    ) {
        guard CMSampleBufferDataIsReady(sampleBuffer) else { return }
        // Snapshot the writer under lock (set once before frames flow), then
        // forward ONE source's raw buffers to it: prefer system audio (the
        // remote participants); fall back to mic when system isn't captured.
        // A single audio track keeps the recording universally playable.
        let writer = lock.withLock { self.videoWriter }
        switch type {
        case .audio:
            if plan.wantsSystem { writer?.appendAudio(sampleBuffer) }
            appendAudio(sampleBuffer, system: true)
        case .microphone:
            if !plan.wantsSystem { writer?.appendAudio(sampleBuffer) }
            appendAudio(sampleBuffer, system: false)
        default: break   // `.screen` is handled by the Screenshotter output
        }
    }

    private static func mapSCStreamError(_ error: Error) -> Error {
        // SCStreamError with code 1001/1002 is TCC denial (screen/mic).
        // Use the rawValue to compare against expected TCC codes.
        if let scError = error as? SCStreamError {
            let errorCode = Int(scError.code.rawValue)
            switch errorCode {
            case 1001: return CaptureFailure.permissionDenied("screen recording")
            case 1002: return CaptureFailure.permissionDenied("microphone")
            default: break
            }
        }
        return error
    }

    // MARK: - Audio path

    private func appendAudio(_ sampleBuffer: CMSampleBuffer, system: Bool) {
        lock.lock()
        defer { lock.unlock() }
        guard let writer = system ? systemWriter : micWriter else { return }
        guard let input = Self.makeInputBuffer(from: sampleBuffer) else { return }

        let converter: AVAudioConverter
        if system {
            if systemConverter == nil {
                systemConverter = AVAudioConverter(from: input.format, to: Self.outputFormat)
            }
            guard let c = systemConverter else { return }
            converter = c
        } else {
            if micConverter == nil {
                micConverter = AVAudioConverter(from: input.format, to: Self.outputFormat)
            }
            guard let c = micConverter else { return }
            converter = c
        }

        guard let samples = Self.resample(input, with: converter) else { return }
        do {
            try writer.append(samples)
        } catch {
            // Log the error to stderr - this indicates I/O failure (disk full, etc)
            FileHandle.standardError.write(Data("WAV write error: \(error)\n".utf8))
        }
    }

    /// Wrap a ScreenCaptureKit audio `CMSampleBuffer` as an `AVAudioPCMBuffer`
    /// in its native format (no conversion yet).
    static func makeInputBuffer(from sampleBuffer: CMSampleBuffer) -> AVAudioPCMBuffer? {
        guard let fmtDesc = CMSampleBufferGetFormatDescription(sampleBuffer),
              let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(fmtDesc),
              let format = AVAudioFormat(streamDescription: asbd) else { return nil }
        let frames = CMSampleBufferGetNumSamples(sampleBuffer)
        guard frames > 0,
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames))
        else { return nil }
        buffer.frameLength = AVAudioFrameCount(frames)
        let status = CMSampleBufferCopyPCMDataIntoAudioBufferList(
            sampleBuffer, at: 0, frameCount: Int32(frames), into: buffer.mutableAudioBufferList
        )
        guard status == noErr else { return nil }
        return buffer
    }

    /// Resample one input buffer to 16 kHz mono Float32 and quantize to Int16.
    /// Stereo→mono is AVAudioConverter's per-source channel downmix, never a
    /// cross-source sum.
    static func resample(_ input: AVAudioPCMBuffer, with converter: AVAudioConverter) -> [Int16]? {
        let ratio = Self.outputFormat.sampleRate / input.format.sampleRate
        let capacity = AVAudioFrameCount((Double(input.frameLength) * ratio).rounded(.up)) + 1024
        guard let output = AVAudioPCMBuffer(pcmFormat: Self.outputFormat, frameCapacity: capacity)
        else { return nil }

        // The input block is `@Sendable`; feed the whole buffer once via a
        // reference wrapper so the closure captures a Sendable box rather than
        // a mutable var + non-Sendable buffer. AVAudioConverter invokes the
        // block synchronously on this thread.
        let feed = FeedState(input)
        var convError: NSError?
        let status = converter.convert(to: output, error: &convError) { _, inStatus in
            guard let buffer = feed.take() else {
                inStatus.pointee = .noDataNow
                return nil
            }
            inStatus.pointee = .haveData
            return buffer
        }
        guard status != .error, let channel = output.floatChannelData else { return nil }
        let count = Int(output.frameLength)
        guard count > 0 else { return [] }
        let floats = Array(UnsafeBufferPointer(start: channel[0], count: count))
        return floatToInt16(floats)
    }
}

enum CaptureFailure: Error {
    case noDisplay
    case permissionDenied(String)
    case invalidParams(String)
}

/// One-shot input box for `AVAudioConverter`'s `@Sendable` input block:
/// returns the buffer exactly once, then `nil`. `@unchecked Sendable` because
/// the block runs synchronously on the calling thread (no real concurrency),
/// so the single mutation needs no further guarding.
private final class FeedState: @unchecked Sendable {
    private var buffer: AVAudioPCMBuffer?
    init(_ buffer: AVAudioPCMBuffer) { self.buffer = buffer }
    func take() -> AVAudioPCMBuffer? {
        defer { buffer = nil }
        return buffer
    }
}
