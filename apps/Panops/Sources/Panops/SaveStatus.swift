import SwiftUI

/// Autosave lifecycle for a single edit operation (title, notes, org).
///
/// Pure data — the retry action lives at the call site, not inside the enum,
/// so `SaveStatus` stays `Equatable`/testable and the view can render a Retry
/// button without the model knowing about closures.
///
/// Transitions the view model drives:
///   idle ──▶ saving ──▶ saved ──▶ idle   (next edit, or on fade-out)
///               └──▶ failed(message) ──▶ saving (Retry) ──▶ …
///
/// `idle` means "no autosave in flight and nothing to report". It's the
/// initial state and the state the view returns to after the `saved` chip
/// fades out.
enum SaveStatus: Equatable {
    case idle
    case saving
    case saved
    case failed(message: String)

    /// True while an autosave RPC is in flight. The view uses this to disable
    /// the edit affordances (toggle, submit) so a second edit can't race the
    /// first.
    var isSaving: Bool {
        if case .saving = self { return true }
        return false
    }

    /// True when a prior autosave failed and the user can retry.
    var isFailed: Bool {
        if case .failed = self { return true }
        return false
    }

    /// Failure message if any. Convenience for the status view.
    var failureMessage: String? {
        if case .failed(let message) = self { return message }
        return nil
    }
}

/// Inline status chip: spinner while saving, green "Saved" that fades, orange
/// error line with a Retry button on failure. Empty when `.idle`.
///
/// The retry closure is caller-supplied — the view model owns what "retry"
/// means for each edit (title re-submits the new title; notes re-saves the
/// current markdown buffer).
struct SaveStatusView: View {
    let status: SaveStatus
    var retry: (() -> Void)? = nil

    @State private var savedOpacity: Double = 1.0

    var body: some View {
        HStack(spacing: 6) {
            switch status {
            case .idle:
                EmptyView()
            case .saving:
                ProgressView().controlSize(.small)
                Text("Saving…")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            case .saved:
                Image(systemName: "checkmark")
                    .foregroundStyle(.green)
                Text("Saved")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .opacity(savedOpacity)
            case .failed(let message):
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(.orange)
                Text(message)
                    .font(.caption)
                    .foregroundStyle(.orange)
                if let retry {
                    Button("Retry", action: retry)
                        .font(.caption)
                        .buttonStyle(.link)
                }
            }
        }
        .animation(.easeOut(duration: 0.2), value: status)
        // Reset + fade on every transition into `.saved`. The task is
        // cancelled automatically if status changes again (e.g. another edit
        // starts before the fade completes), so a fresh `saving` state never
        // races a lingering fade.
        .task(id: status) {
            guard case .saved = status else { return }
            savedOpacity = 1.0
            do {
                try await Task.sleep(for: .seconds(1.5))
            } catch {
                return // cancelled by status change
            }
            guard !Task.isCancelled else { return }
            withAnimation(.easeOut(duration: 0.8)) {
                savedOpacity = 0
            }
        }
    }
}
