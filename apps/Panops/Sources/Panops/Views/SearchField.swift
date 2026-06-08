import SwiftUI

/// A search input row: a magnifying-glass icon, a plain text field, and a
/// clear button that appears once there's text. Outer padding / background
/// styling is left to the call site so it can sit bare in the sidebar or
/// inside a rounded pill.
struct SearchField: View {
    let placeholder: String
    @Binding var text: String

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
    }
}
