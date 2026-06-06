import SwiftUI

/// Presentational view rendering markdown notes.
/// Uses AttributedString(markdown:) for rendering.
/// Falls back to raw text on parse failure.
/// Empty/nil → placeholder.
struct NotesView: View {
    let content: String?

    var body: some View {
        if let md = content, !md.isEmpty {
            ScrollView {
                if let rendered = try? AttributedString(
                    markdown: md,
                    options: AttributedString.MarkdownParsingOptions(
                        interpretedSyntax: .inlineOnlyPreservingWhitespace
                    )
                ) {
                    Text(rendered)
                        .textSelection(.enabled)
                        .padding()
                } else {
                    // Fallback: show raw text
                    Text(md)
                        .textSelection(.enabled)
                        .font(.system(.body, design: .monospaced))
                        .padding()
                }
            }
        } else {
            placeholder
        }
    }

    private var placeholder: some View {
        VStack {
            Spacer()
            Text("No notes").foregroundStyle(.secondary)
            Spacer()
        }
    }
}
