import Foundation

/// Protocol for recording control. Slice 12 provides MockRecordingController.
/// Slice 11 will provide LiveRecordingController that calls IPC methods.
protocol RecordingController: AnyObject {
    var isRecording: Bool { get }
    func start(meetingId: String) async throws
    func stop() async throws -> URL?  // Returns audio file path
}

/// Mock implementation for Slice 12. Placeholder until Slice 11.
/// Toggles isRecording but shows alert indicating recording not implemented.
final class MockRecordingController: RecordingController, ObservableObject {
    @Published private(set) var isRecording = false
    @Published var showPlaceholderAlert = false

    func start(meetingId: String) async throws {
        isRecording = true
        // Placeholder: show alert after brief delay
        try await Task.sleep(nanoseconds: 500_000_000)
        showPlaceholderAlert = true
    }

    func stop() async throws -> URL? {
        isRecording = false
        return nil
    }
}
