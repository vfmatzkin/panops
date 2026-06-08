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
        lock.lock()
        defer { lock.unlock() }
        guard !failed, !finishing else { return }
        guard startSessionLocked(sampleBuffer) else { return }
        // Cross-track delivery can interleave, so PTS is not globally monotonic;
        // drop anything timestamped before the session origin.
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        if let start = sessionStartTime, pts < start { return }
        guard input.isReadyForMoreMediaData else { return }   // back-pressure: drop
        if !input.append(sampleBuffer) {
            markFailedLocked("append refused")
        }
    }

    /// Lazily `startWriting` + start the session on the first valid sample of
    /// either track. Returns false (silencing the writer) if startup fails.
    private func startSessionLocked(_ sampleBuffer: CMSampleBuffer) -> Bool {
        if sessionStartTime != nil { return true }
        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        guard pts.isValid else { return false }
        guard writer.startWriting() else {
            markFailedLocked("startWriting")
            return false
        }
        writer.startSession(atSourceTime: pts)
        sessionStartTime = pts
        return true
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
        if writer.status == .failed {
            let detail = writer.error.map { "\($0)" } ?? "unknown"
            FileHandle.standardError.write(
                Data("video finishWriting failed: \(detail); audio + screenshots unaffected\n".utf8))
        }
    }

    // MARK: - Helpers

    private func markFailedLocked(_ reason: String) {
        failed = true
        let detail = writer.error.map { ", error: \($0)" } ?? ""
        FileHandle.standardError.write(
            Data("video writer disabled (\(reason)\(detail)); capture continues\n".utf8))
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
