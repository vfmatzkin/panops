import AVFoundation
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

/// Muxes the **existing** `SCStream`'s `.screen` frames and one audio source's
/// `CMSampleBuffer`s into a single playable H.264 `.mov`. It opens no stream of
/// its own — the `Recorder` forwards the same buffers the `Screenshotter` and
/// WAV writers already receive (slice-11 single-stream rule).
///
/// Best-effort: any `AVAssetWriter` failure flips `failed`, logs to stderr, and
/// silences further appends so the WAVs + screenshots keep running. When
/// `record_video` is false this type is never constructed and capture is
/// byte-for-byte unchanged.
///
/// `@unchecked Sendable`: `appendVideo`/`appendAudio` fire on the SCStream
/// sample queues (screen + audio on different queues); all mutable state is
/// guarded by `lock`.
final class VideoWriter: @unchecked Sendable {
    let url: URL

    private let lock = NSLock()
    private let writer: AVAssetWriter
    private let videoInput: AVAssetWriterInput
    private let audioInput: AVAssetWriterInput
    private var sessionStartTime: CMTime?
    private var failed = false
    private var finishing = false

    /// Frame-cadence cap for the `.screen` output while recording. Without it
    /// the stream is throttled to the screenshot interval (a ~2 fps slideshow).
    /// ~30 fps is smooth for screen content and static frames are cheap under
    /// H.264. The screenshotter keeps its own (coarser) interval gate, so its
    /// cadence is unaffected.
    static let frameInterval = CMTime(value: 1, timescale: 30)

    init(url: URL, width: Int, height: Int) throws {
        self.url = url
        // AVAssetWriter refuses to overwrite an existing file; clear any stale
        // recording first so a restart starts clean.
        try? FileManager.default.removeItem(at: url)
        do {
            writer = try AVAssetWriter(outputURL: url, fileType: .mov)
        } catch {
            throw CaptureFailure.invalidParams("AVAssetWriter init failed: \(error.localizedDescription)")
        }

        let videoSettings: [String: Any] = [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
        ]
        videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: videoSettings)
        videoInput.expectsMediaDataInRealTime = true

        // 16 kHz mono AAC, matching the WAV pipeline rate. AVAssetWriter
        // resamples/downmixes the source PCM to these settings, so whatever
        // sample rate / channel count ScreenCaptureKit hands us is accepted.
        let audioSettings: [String: Any] = [
            AVFormatIDKey: kAudioFormatMPEG4AAC,
            AVSampleRateKey: 16_000,
            AVNumberOfChannelsKey: 1,
        ]
        audioInput = AVAssetWriterInput(mediaType: .audio, outputSettings: audioSettings)
        audioInput.expectsMediaDataInRealTime = true

        guard writer.canAdd(videoInput) else {
            throw CaptureFailure.invalidParams("AVAssetWriter cannot add video input")
        }
        writer.add(videoInput)
        guard writer.canAdd(audioInput) else {
            throw CaptureFailure.invalidParams("AVAssetWriter cannot add audio input")
        }
        writer.add(audioInput)
    }

    // MARK: - Sample intake (called on the SCStream sample queues)

    /// Append one screen frame. Callers filter for `.complete` frames via
    /// `isCompleteFrame(_:)` before forwarding here.
    func appendVideo(_ sampleBuffer: CMSampleBuffer) {
        append(sampleBuffer, to: videoInput)
    }

    /// Append one audio buffer (the raw system-or-mic buffer, before resample).
    func appendAudio(_ sampleBuffer: CMSampleBuffer) {
        append(sampleBuffer, to: audioInput)
    }

    private func append(_ sampleBuffer: CMSampleBuffer, to input: AVAssetWriterInput) {
        guard CMSampleBufferDataIsReady(sampleBuffer) else { return }
        // Do every state mutation under the lock, but NEVER touch stderr here:
        // `FileHandle.standardError.write` blocks if the pipe is full or a
        // debugger is paused, and the audio path forwards its buffers through
        // this same lock — a stalled write would freeze all sample intake.
        // Surface any failure reason and log it once after unlocking.
        let failureReason: String? = lock.withLock {
            guard !failed, !finishing else { return nil }
            switch startSessionLocked(sampleBuffer) {
            case .failed(let reason): return reason
            case .notReady: return nil
            case .ready: break
            }
            // Cross-track delivery can interleave, so PTS is not globally
            // monotonic; drop anything timestamped before the session origin.
            let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
            if let start = sessionStartTime, pts < start { return nil }
            guard input.isReadyForMoreMediaData else { return nil }   // back-pressure: drop
            if !input.append(sampleBuffer) {
                return markFailedLocked("append refused")
            }
            return nil
        }
        if let failureReason { logFailure(failureReason) }
    }

    /// Outcome of a lazy session start, computed under the lock.
    private enum SessionStart {
        case ready              // session active — proceed with the append
        case notReady           // can't start yet (invalid PTS) — skip this sample
        case failed(String)     // startWriting failed — reason to log after unlock
    }

    /// Lazily `startWriting` + start the session on the first valid sample of
    /// either track. Does no I/O; on startup failure it flips `failed` and
    /// returns the reason for the caller to log after releasing the lock.
    private func startSessionLocked(_ sampleBuffer: CMSampleBuffer) -> SessionStart {
        if sessionStartTime != nil { return .ready }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        guard pts.isValid else { return .notReady }
        guard writer.startWriting() else {
            return .failed(markFailedLocked("startWriting"))
        }
        writer.startSession(atSourceTime: pts)
        sessionStartTime = pts
        return .ready
    }

    // MARK: - Finalize

    /// Mark inputs finished and await `finishWriting()`. Call once on stop; a
    /// no-op if the session never started or the writer already failed.
    func finish() async {
        let shouldFinish: Bool = lock.withLock {
            guard !failed, !finishing, sessionStartTime != nil else { return false }
            finishing = true
            videoInput.markAsFinished()
            audioInput.markAsFinished()
            return true
        }
        guard shouldFinish else { return }
        await writer.finishWriting()
        // Logged outside the lock — `lock.withLock` above has already returned.
        if writer.status == .failed {
            let detail = writer.error.map { "\($0)" } ?? "unknown"
            logFailure("video finishWriting failed: \(detail); audio + screenshots unaffected")
        }
    }

    // MARK: - Helpers

    /// Flips `failed` and returns the message to log. Does NO I/O — callers
    /// invoke `logFailure` with the returned string AFTER releasing `lock`
    /// (a blocked stderr write must not stall sample intake). Each call site is
    /// guarded by `!failed`, so the flip and the log happen exactly once.
    private func markFailedLocked(_ reason: String) -> String {
        failed = true
        let detail = writer.error.map { ", error: \($0)" } ?? ""
        return "video writer disabled (\(reason)\(detail)); capture continues"
    }

    /// Write a one-line failure message to stderr. MUST be called outside `lock`.
    private func logFailure(_ message: String) {
        FileHandle.standardError.write(Data("\(message)\n".utf8))
    }

    /// True only for `.complete` frames, which carry fresh pixels.
    /// Idle/blank/suspended frames are skipped — their timeline gap is fine
    /// (`.mov` is variable-frame-rate). A frame with no status attachment is
    /// treated as not-complete.
    static func isCompleteFrame(_ sampleBuffer: CMSampleBuffer) -> Bool {
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false)
            as? [[SCStreamFrameInfo: Any]],
            let info = attachments.first,
            let raw = info[.status] as? Int,
            let status = SCFrameStatus(rawValue: raw)
        else { return false }
        return status == .complete
    }

    /// `<meeting_dir>/recording.mov`. The meeting dir is the parent of the WAV
    /// paths (`<meeting_dir>/system.wav`); falls back to the parent of
    /// `screenshots_dir` (`<meeting_dir>/screenshots`) when no audio path is
    /// present. Nil only when nothing identifies the meeting dir.
    static func outputURL(systemAudioPath: String?, micAudioPath: String?, screenshotsDir: String?) -> URL? {
        if let audio = systemAudioPath ?? micAudioPath {
            return URL(fileURLWithPath: audio)
                .deletingLastPathComponent()
                .appendingPathComponent("recording.mov")
        }
        if let shots = screenshotsDir {
            return URL(fileURLWithPath: shots)
                .deletingLastPathComponent()
                .appendingPathComponent("recording.mov")
        }
        return nil
    }
}
