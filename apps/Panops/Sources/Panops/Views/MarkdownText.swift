import SwiftUI

/// A parsed markdown block. Block-level structure (headings, lists, quotes,
/// code) is parsed here; inline formatting (**bold**, *italic*, `code`) is
/// rendered per line via `AttributedString(markdown:)` at display time.
enum MarkdownBlock: Equatable {
    case heading(level: Int, text: String)
    case bullet(text: String)
    case ordered(number: Int, text: String)
    case paragraph(text: String)
    case code(text: String)
    case quote(text: String)
}

enum Markdown {
    /// Strip a leading YAML frontmatter block delimited by a `---` line at the
    /// very start and the next `---` line. Returns the body unchanged when no
    /// frontmatter is present. Mirrors what the engine writes into `notes.md`.
    static func stripFrontmatter(_ source: String) -> String {
        let normalized = source.replacingOccurrences(of: "\r\n", with: "\n")
        let lines = normalized.components(separatedBy: "\n")
        guard let first = lines.first, first.trimmingCharacters(in: .whitespaces) == "---" else {
            return source
        }
        // Find the closing fence after line 0.
        for index in 1..<lines.count where lines[index].trimmingCharacters(in: .whitespaces) == "---" {
            let body = lines[(index + 1)...].joined(separator: "\n")
            return body.trimmingCharacters(in: .whitespacesAndNewlines)
        }
        // Unterminated frontmatter: return original rather than eating the doc.
        return source
    }

    /// Parse markdown text into block-level structure. Consecutive plain lines
    /// coalesce into a single paragraph; blank lines separate blocks.
    static func parseBlocks(_ source: String) -> [MarkdownBlock] {
        let normalized = source.replacingOccurrences(of: "\r\n", with: "\n")
        let lines = normalized.components(separatedBy: "\n")
        var blocks: [MarkdownBlock] = []
        var paragraph: [String] = []
        var codeLines: [String] = []
        var inCode = false

        func flushParagraph() {
            if !paragraph.isEmpty {
                blocks.append(.paragraph(text: paragraph.joined(separator: " ")))
                paragraph.removeAll()
            }
        }

        for rawLine in lines {
            let line = rawLine
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if trimmed.hasPrefix("```") {
                if inCode {
                    blocks.append(.code(text: codeLines.joined(separator: "\n")))
                    codeLines.removeAll()
                    inCode = false
                } else {
                    flushParagraph()
                    inCode = true
                }
                continue
            }
            if inCode {
                codeLines.append(line)
                continue
            }

            if trimmed.isEmpty {
                flushParagraph()
                continue
            }

            if let heading = parseHeading(trimmed) {
                flushParagraph()
                blocks.append(heading)
                continue
            }
            if let (number, text) = parseOrdered(trimmed) {
                flushParagraph()
                blocks.append(.ordered(number: number, text: text))
                continue
            }
            if let bullet = parseBullet(trimmed) {
                flushParagraph()
                blocks.append(.bullet(text: bullet))
                continue
            }
            if trimmed.hasPrefix(">") {
                flushParagraph()
                let text = String(trimmed.dropFirst()).trimmingCharacters(in: .whitespaces)
                blocks.append(.quote(text: text))
                continue
            }

            paragraph.append(trimmed)
        }
        if inCode, !codeLines.isEmpty {
            blocks.append(.code(text: codeLines.joined(separator: "\n")))
        }
        flushParagraph()
        return blocks
    }

    private static func parseHeading(_ trimmed: String) -> MarkdownBlock? {
        var level = 0
        for ch in trimmed {
            if ch == "#" { level += 1 } else { break }
        }
        guard level >= 1, level <= 6 else { return nil }
        let rest = trimmed.dropFirst(level)
        guard rest.first == " " else { return nil }
        let text = rest.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return nil }
        return .heading(level: level, text: text)
    }

    private static func parseBullet(_ trimmed: String) -> String? {
        for marker in ["- ", "* ", "+ "] where trimmed.hasPrefix(marker) {
            return String(trimmed.dropFirst(marker.count)).trimmingCharacters(in: .whitespaces)
        }
        return nil
    }

    private static func parseOrdered(_ trimmed: String) -> (Int, String)? {
        var digits = ""
        var idx = trimmed.startIndex
        while idx < trimmed.endIndex, trimmed[idx].isNumber {
            digits.append(trimmed[idx])
            idx = trimmed.index(after: idx)
        }
        guard !digits.isEmpty, let number = Int(digits), idx < trimmed.endIndex else { return nil }
        let sep = trimmed[idx]
        guard sep == "." || sep == ")" else { return nil }
        let afterSep = trimmed.index(after: idx)
        guard afterSep < trimmed.endIndex, trimmed[afterSep] == " " else { return nil }
        let text = String(trimmed[trimmed.index(after: afterSep)...]).trimmingCharacters(in: .whitespaces)
        return (number, text)
    }

    /// Render a single line of inline markdown, falling back to plain text.
    static func inlineAttributed(_ source: String) -> AttributedString {
        if let parsed = try? AttributedString(
            markdown: source,
            options: AttributedString.MarkdownParsingOptions(
                interpretedSyntax: .inlineOnlyPreservingWhitespace
            )
        ) {
            return parsed
        }
        return AttributedString(source)
    }
}

/// Renders block-structured markdown: headings as styled titles, lists with
/// bullets/numbers, paragraphs and quotes with inline formatting, code in a
/// monospaced block. No literal `##` or list markers leak through.
struct MarkdownBlocksView: View {
    let markdown: String

    var body: some View {
        let blocks = Markdown.parseBlocks(markdown)
        VStack(alignment: .leading, spacing: 8) {
            // Positional identity: two blocks with identical text must stay
            // distinct rows, so key on the parse order rather than the content.
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in
                switch block {
                case .heading(let level, let text):
                    Text(text)
                        .font(headingFont(level))
                        .fontWeight(.semibold)
                        .padding(.top, level <= 2 ? 4 : 0)
                case .bullet(let text):
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text("•").foregroundStyle(.secondary)
                        Text(Markdown.inlineAttributed(text))
                    }
                case .ordered(let number, let text):
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text("\(number).").foregroundStyle(.secondary).monospacedDigit()
                        Text(Markdown.inlineAttributed(text))
                    }
                case .paragraph(let text):
                    Text(Markdown.inlineAttributed(text))
                        .fixedSize(horizontal: false, vertical: true)
                case .quote(let text):
                    HStack(spacing: 8) {
                        Rectangle().fill(Color.secondary.opacity(0.4)).frame(width: 3)
                        Text(Markdown.inlineAttributed(text))
                            .italic()
                            .foregroundStyle(.secondary)
                    }
                case .code(let text):
                    Text(text)
                        .font(.system(.callout, design: .monospaced))
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.secondary.opacity(0.08))
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                }
            }
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func headingFont(_ level: Int) -> Font {
        switch level {
        case 1: return .title2
        case 2: return .title3
        default: return .headline
        }
    }
}
