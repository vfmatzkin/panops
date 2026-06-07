import Foundation
import Combine

/// Protocol for recording control.
@MainActor
protocol RecordingController: AnyObject {
    var isRecording: Bool { get }
    func start(meetingId: String) async throws
    func stop() async throws -> URL?  // Returns audio file path
}

/// Mock implementation for previews/tests.
/// Toggles isRecording and can still surface the old placeholder alert.
@MainActor
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
