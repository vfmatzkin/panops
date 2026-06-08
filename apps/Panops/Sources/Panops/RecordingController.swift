import Foundation
import Combine

/// Capture options chosen for a recording session and forwarded to
/// `ipc.recording.start`. Defaults mirror the engine's own defaults.
struct RecordingOptions: Equatable {
    var audioSources: AudioSourcesWire
    var screenshotIntervalMs: UInt64
    var screenshotThreshold: Float
    var recordVideo: Bool
    var autoGenerateNotes: Bool

    init(
        audioSources: AudioSourcesWire = .systemAndMic,
        screenshotIntervalMs: UInt64 = 500,
        screenshotThreshold: Float = 0.15,
        recordVideo: Bool = false,
        autoGenerateNotes: Bool = true
    ) {
        self.audioSources = audioSources
        self.screenshotIntervalMs = screenshotIntervalMs
        self.screenshotThreshold = screenshotThreshold
        self.recordVideo = recordVideo
        self.autoGenerateNotes = autoGenerateNotes
    }

    static let `default` = RecordingOptions()
}

/// Language choice offered in the New Recording sheet. Maps to the
/// `MeetingConfig.language` wire value; `.auto` omits it (engine defaults to
/// "auto" and detects per-region).
enum RecordingLanguage: String, CaseIterable, Identifiable {
    case auto
    case english
    case spanish

    var id: String { rawValue }

    var label: String {
        switch self {
        case .auto: return "Auto"
        case .english: return "English"
        case .spanish: return "Spanish"
        }
    }

    /// BCP-47 hint sent to the engine, or `nil` for auto-detect.
    var wireValue: String? {
        switch self {
        case .auto: return nil
        case .english: return "en"
        case .spanish: return "es"
        }
    }
}

/// The user's choices from the New Recording sheet. Bridges the UI to the two
/// backend calls: `meeting.start` (title + language) and `recording.start`
/// (audio sources + screenshot sampling).
struct RecordingSetup: Equatable {
    var title: String = ""
    var language: RecordingLanguage = .auto
    var audioSources: AudioSourcesWire = .systemAndMic
    /// Screenshots are always captured: the engine has no off-switch yet, so
    /// this stays `true` and drives only the recording-screen indicator. See
    /// the screenshots follow-up note.
    var captureScreenshots: Bool = true
    var recordVideo: Bool = false
    var autoGenerateNotes: Bool = true

    static let `default` = RecordingSetup()

    /// `meeting.start` config. Empty title and `.auto` language both collapse to
    /// omitted fields so the engine applies its own defaults.
    var meetingConfig: MeetingConfig {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        return MeetingConfig(
            title: trimmed.isEmpty ? nil : trimmed,
            language: language.wireValue
        )
    }

    /// `recording.start` options. Screenshot sampling stays at engine defaults
    /// (no off-switch on the wire yet).
    var recordingOptions: RecordingOptions {
        RecordingOptions(
            audioSources: audioSources,
            screenshotIntervalMs: RecordingOptions.default.screenshotIntervalMs,
            screenshotThreshold: RecordingOptions.default.screenshotThreshold,
            recordVideo: recordVideo,
            autoGenerateNotes: autoGenerateNotes
        )
    }
}

/// Protocol for recording control.
@MainActor
protocol RecordingController: AnyObject {
    var isRecording: Bool { get }
    /// True only once `recording.start` has been accepted (a recording id
    /// exists) and the recording hasn't been stopped yet. `isRecording` flips
    /// optimistically before acceptance to block a double-start, so the UI must
    /// gate Stop on this signal — not on `isRecording` — to avoid a Stop that
    /// no-ops while the start is still in flight.
    var canStop: Bool { get }
    func start(meetingId: String, options: RecordingOptions) async throws
    func stop() async throws -> URL?  // Returns audio file path
}

extension RecordingController {
    /// Start with the engine-default capture options. Used by the in-meeting
    /// resume affordance, which has no explicit setup to thread.
    func start(meetingId: String) async throws {
        try await start(meetingId: meetingId, options: .default)
    }
}

/// Mock implementation for previews/tests.
/// Toggles isRecording and can still surface the old placeholder alert.
@MainActor
final class MockRecordingController: RecordingController, ObservableObject {
    @Published private(set) var isRecording = false
    @Published private(set) var canStop = false
    @Published var showPlaceholderAlert = false

    func start(meetingId: String, options: RecordingOptions) async throws {
        isRecording = true
        canStop = true
        // Placeholder: show alert after brief delay
        try await Task.sleep(nanoseconds: 500_000_000)
        showPlaceholderAlert = true
    }

    func stop() async throws -> URL? {
        isRecording = false
        canStop = false
        return nil
    }
}
