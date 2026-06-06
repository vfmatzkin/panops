import SwiftUI

/// Record/Stop control bar. DISABLED this slice with placeholder text.
/// Injects RecordingController for future live capture.
struct RecordBar: View {
    @ObservedObject var controller: MockRecordingController

    var body: some View {
        HStack(spacing: 16) {
            Button(action: {
                Task {
                    // Disabled this slice - show placeholder
                    controller.showPlaceholderAlert = true
                }
            }) {
                HStack(spacing: 4) {
                    Image(systemName: "record.circle")
                        .foregroundStyle(.red)
                    Text("Record")
                }
            }
            .disabled(true)  // Disabled per spec
            .help("Live capture coming soon (Anchor B)")

            Spacer()

            Text("Recording requires live capture (Anchor B)")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
        .padding()
        .background(Color.secondary.opacity(0.05))
        .alert("Recording not available", isPresented: $controller.showPlaceholderAlert) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("Live capture is coming in a future update. This slice focuses on the UI framework.")
        }
    }
}