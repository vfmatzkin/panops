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
    /// NotionEnhanced `<callout icon="X">…</callout>` — a highlighted note.
    case callout(icon: String?, body: String)
    /// NotionEnhanced `<details><summary>Title</summary>…</details>` — collapsible.
    case disclosure(summary: String, body: String)
    /// NotionEnhanced `<table>…</table>` rows; first row is the header.
    case table(rows: [[String]])
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

    /// Parse markdown text into block-level structure. NotionEnhanced container
    /// tags (`<callout>`, `<details>`, `<table>`) — which may span multiple
    /// lines — are lifted out into dedicated blocks first; the text between them
    /// is parsed as ordinary markdown. Any other angle-bracket tag is stripped
    /// so nothing renders as raw `<tag>` text.
    static func parseBlocks(_ source: String) -> [MarkdownBlock] {
        let normalized = source.replacingOccurrences(of: "\r\n", with: "\n")
        // Fenced code is parsed downstream by `parseTextBlocks`; compute its
        // ranges up front so container scanning skips tags written inside code.
        let fenced = fencedCodeRanges(in: normalized)
        var blocks: [MarkdownBlock] = []
        var cursor = normalized.startIndex
        while cursor < normalized.endIndex {
            guard let match = nextContainerTag(in: normalized, from: cursor, fenced: fenced) else {
                blocks.append(contentsOf: parseTextBlocks(String(normalized[cursor...])))
                break
            }
            if match.range.lowerBound > cursor {
                blocks.append(contentsOf: parseTextBlocks(String(normalized[cursor..<match.range.lowerBound])))
            }
            if let block = match.block {
                blocks.append(block)
            }
            cursor = match.range.upperBound
        }
        return blocks
    }

    /// Parse a run of plain markdown (no container tags). Consecutive plain
    /// lines coalesce into a single paragraph; blank lines separate blocks.
    /// Inline text is sanitized of stray angle-bracket tags.
    private static func parseTextBlocks(_ source: String) -> [MarkdownBlock] {
        let lines = source.components(separatedBy: "\n")
        var blocks: [MarkdownBlock] = []
        var paragraph: [String] = []
        var codeLines: [String] = []
        var openFence: String?

        func flushParagraph() {
            if !paragraph.isEmpty {
                let text = sanitizeInline(paragraph.joined(separator: " "))
                    .trimmingCharacters(in: .whitespaces)
                if !text.isEmpty {
                    blocks.append(.paragraph(text: text))
                }
                paragraph.removeAll()
            }
        }

        for rawLine in lines {
            let line = rawLine
            let trimmed = line.trimmingCharacters(in: .whitespaces)

            if let marker = fenceMarker(of: trimmed) {
                if openFence == nil {
                    flushParagraph()
                    openFence = marker
                    continue
                } else if marker == openFence {
                    blocks.append(.code(text: codeLines.joined(separator: "\n")))
                    codeLines.removeAll()
                    openFence = nil
                    continue
                }
                // A non-matching fence marker inside a block is code content.
            }
            if openFence != nil {
                codeLines.append(line)
                continue
            }

            if trimmed.isEmpty {
                flushParagraph()
                continue
            }

            if let (level, text) = parseHeading(trimmed) {
                flushParagraph()
                blocks.append(.heading(level: level, text: sanitizeInline(text)))
                continue
            }
            if let (number, text) = parseOrdered(trimmed) {
                flushParagraph()
                blocks.append(.ordered(number: number, text: sanitizeInline(text)))
                continue
            }
            if let bullet = parseBullet(trimmed) {
                flushParagraph()
                blocks.append(.bullet(text: sanitizeInline(bullet)))
                continue
            }
            if trimmed.hasPrefix(">") {
                flushParagraph()
                let text = String(trimmed.dropFirst()).trimmingCharacters(in: .whitespaces)
                blocks.append(.quote(text: sanitizeInline(text)))
                continue
            }

            paragraph.append(trimmed)
        }
        if openFence != nil, !codeLines.isEmpty {
            blocks.append(.code(text: codeLines.joined(separator: "\n")))
        }
        flushParagraph()
        return blocks
    }

    private static func parseHeading(_ trimmed: String) -> (Int, String)? {
        var level = 0
        for ch in trimmed {
            if ch == "#" { level += 1 } else { break }
        }
        guard level >= 1, level <= 6 else { return nil }
        let rest = trimmed.dropFirst(level)
        guard rest.first == " " else { return nil }
        let text = rest.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return nil }
        return (level, text)
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

    // MARK: - NotionEnhanced container tags

    /// A located container tag: the full source range it occupies (used to slice
    /// the text around it) and the block it parses into. `block` is nil when the
    /// open tag has no matching close — the range then covers only the orphan
    /// open tag so the caller drops it.
    private struct ContainerMatch {
        let range: Range<String.Index>
        let block: MarkdownBlock?
    }

    private static let containerNames = ["callout", "details", "table"]

    /// Fence markers that open/close a code block (CommonMark + NotionEnhanced).
    private static let fenceMarkers = ["```", "~~~"]

    /// The fence marker a trimmed line opens/closes with, or nil if it is not a
    /// fence line.
    private static func fenceMarker(of trimmed: String) -> String? {
        for marker in fenceMarkers where trimmed.hasPrefix(marker) { return marker }
        return nil
    }

    /// Source ranges covered by fenced code blocks (` ``` ` or `~~~`), fence
    /// lines included, so container scanning ignores tags written inside code.
    /// An unterminated fence runs to the end of the source.
    private static func fencedCodeRanges(in source: String) -> [Range<String.Index>] {
        var ranges: [Range<String.Index>] = []
        var openStart: String.Index?
        var openMarker: String?
        var lineStart = source.startIndex
        while true {
            let lineEnd = source.range(of: "\n", range: lineStart..<source.endIndex)?.lowerBound
                ?? source.endIndex
            let trimmed = source[lineStart..<lineEnd].trimmingCharacters(in: .whitespaces)
            if let marker = openMarker, let start = openStart {
                if trimmed.hasPrefix(marker) {
                    let next = lineEnd < source.endIndex ? source.index(after: lineEnd) : source.endIndex
                    ranges.append(start..<next)
                    openStart = nil
                    openMarker = nil
                }
            } else if let marker = fenceMarker(of: trimmed) {
                openStart = lineStart
                openMarker = marker
            }
            if lineEnd == source.endIndex { break }
            lineStart = source.index(after: lineEnd)
        }
        if let start = openStart {
            ranges.append(start..<source.endIndex)
        }
        return ranges
    }

    /// Find the earliest `<callout>` / `<details>` / `<table>` opening at or
    /// after `from` (skipping any inside a fenced code range), consume through
    /// its matching close tag (which may be many lines later), and parse it into
    /// a block.
    private static func nextContainerTag(
        in text: String, from: String.Index, fenced: [Range<String.Index>]
    ) -> ContainerMatch? {
        var best: (open: Range<String.Index>, name: String)?
        for name in containerNames {
            var searchStart = from
            while let openMark = text.range(
                of: "<\(name)", options: .caseInsensitive, range: searchStart..<text.endIndex
            ) {
                let after = openMark.upperBound
                let boundaryOk = after != text.endIndex && " \t\r\n>/".contains(text[after])
                let inFence = fenced.contains { $0.contains(openMark.lowerBound) }
                if boundaryOk && !inFence {
                    if best == nil || openMark.lowerBound < best!.open.lowerBound {
                        best = (openMark, name)
                    }
                    break
                }
                searchStart = openMark.upperBound
            }
        }
        guard let chosen = best else { return nil }
        let name = chosen.name
        // End of the open tag (its `>`); without one the tag is malformed.
        guard let openEnd = text.range(of: ">", range: chosen.open.upperBound..<text.endIndex) else {
            return ContainerMatch(range: chosen.open, block: nil)
        }
        let openTag = String(text[chosen.open.lowerBound..<openEnd.upperBound])
        guard let close = text.range(
            of: "</\(name)>", options: .caseInsensitive, range: openEnd.upperBound..<text.endIndex
        ) else {
            // Orphan open tag: drop just the open tag, keep the rest as text.
            return ContainerMatch(range: chosen.open.lowerBound..<openEnd.upperBound, block: nil)
        }
        let inner = String(text[openEnd.upperBound..<close.lowerBound])
        let full = chosen.open.lowerBound..<close.upperBound
        return ContainerMatch(range: full, block: makeContainerBlock(name: name, openTag: openTag, inner: inner))
    }

    private static func makeContainerBlock(name: String, openTag: String, inner: String) -> MarkdownBlock {
        switch name {
        case "callout":
            return .callout(
                icon: parseIconAttr(openTag),
                body: inner.trimmingCharacters(in: .whitespacesAndNewlines)
            )
        case "details":
            let (summary, body) = parseDetails(inner)
            return .disclosure(summary: summary, body: body)
        default:
            return .table(rows: parseTableRows(inner))
        }
    }

    /// Extract `icon="…"` (or single-quoted) from a `<callout …>` open tag.
    private static func parseIconAttr(_ openTag: String) -> String? {
        for quote in ["icon=\"", "icon='"] {
            guard let start = openTag.range(of: quote) else { continue }
            let close = quote.hasSuffix("\"") ? "\"" : "'"
            guard let end = openTag.range(of: close, range: start.upperBound..<openTag.endIndex) else { continue }
            let icon = String(openTag[start.upperBound..<end.lowerBound]).trimmingCharacters(in: .whitespaces)
            return icon.isEmpty ? nil : icon
        }
        return nil
    }

    /// Split `<details>` inner content into its `<summary>` label and body.
    private static func parseDetails(_ inner: String) -> (summary: String, body: String) {
        guard let sOpen = inner.range(of: "<summary", options: .caseInsensitive),
              let sOpenEnd = inner.range(of: ">", range: sOpen.upperBound..<inner.endIndex),
              let sClose = inner.range(of: "</summary>", options: .caseInsensitive, range: sOpenEnd.upperBound..<inner.endIndex)
        else {
            return ("", inner.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        let summary = sanitizeInline(String(inner[sOpenEnd.upperBound..<sClose.lowerBound]))
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let body = String(inner[sClose.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines)
        return (summary, body)
    }

    /// Parse `<tr>` rows of `<td>`/`<th>` cells into a string grid.
    private static func parseTableRows(_ inner: String) -> [[String]] {
        var rows: [[String]] = []
        var search = inner.startIndex
        while let trOpen = inner.range(of: "<tr", options: .caseInsensitive, range: search..<inner.endIndex),
              let trGt = inner.range(of: ">", range: trOpen.upperBound..<inner.endIndex),
              let trClose = inner.range(of: "</tr>", options: .caseInsensitive, range: trGt.upperBound..<inner.endIndex) {
            let cells = parseCells(String(inner[trGt.upperBound..<trClose.lowerBound]))
            if !cells.isEmpty { rows.append(cells) }
            search = trClose.upperBound
        }
        return rows
    }

    private static func parseCells(_ rowInner: String) -> [String] {
        var cells: [String] = []
        var search = rowInner.startIndex
        while let lt = rowInner.range(of: "<", range: search..<rowInner.endIndex) {
            let after = lt.upperBound
            let rest = rowInner[after...].lowercased()
            let name = rest.hasPrefix("td") ? "td" : (rest.hasPrefix("th") ? "th" : nil)
            guard let name else { search = after; continue }
            guard let gt = rowInner.range(of: ">", range: after..<rowInner.endIndex),
                  let close = rowInner.range(of: "</\(name)>", options: .caseInsensitive, range: gt.upperBound..<rowInner.endIndex)
            else { break }
            let cell = sanitizeInline(String(rowInner[gt.upperBound..<close.lowerBound]))
                .trimmingCharacters(in: .whitespacesAndNewlines)
            cells.append(cell)
            search = close.upperBound
        }
        return cells
    }

    /// Strip HTML-tag-like sequences (`<tag …>`, `</tag>`) from inline text so
    /// no stray markup renders raw. A `<` not followed by a letter (or `/`+letter)
    /// is left alone, so prose like `a < b` survives. CommonMark autolinks
    /// (`<scheme://…>`) and emails (`<…@…>`) are spared so `inlineAttributed`
    /// can render them as links.
    static func sanitizeInline(_ text: String) -> String {
        guard text.contains("<") else { return text }
        var result = ""
        var i = text.startIndex
        while i < text.endIndex {
            if text[i] == "<" {
                var probe = text.index(after: i)
                if probe < text.endIndex, text[probe] == "/" {
                    probe = text.index(after: probe)
                }
                if probe < text.endIndex, text[probe].isLetter,
                   let gt = text.range(of: ">", range: i..<text.endIndex),
                   !isAutolinkOrEmail(text[text.index(after: i)..<gt.lowerBound]) {
                    i = gt.upperBound
                    continue
                }
            }
            result.append(text[i])
            i = text.index(after: i)
        }
        return result
    }

    /// A CommonMark autolink (`scheme://…`) or email (`local@domain`) inside
    /// angle brackets — never a real tag (those carry attributes or whitespace),
    /// so it must not be stripped.
    private static func isAutolinkOrEmail(_ inner: Substring) -> Bool {
        guard !inner.contains(" ") else { return false }
        return inner.contains("://") || inner.contains("@")
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
                case .callout(let icon, let body):
                    CalloutBlockView(icon: icon, content: body)
                case .disclosure(let summary, let body):
                    DisclosureBlockView(summary: summary, content: body)
                case .table(let rows):
                    TableBlockView(rows: rows)
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

/// A NotionEnhanced callout: a tinted rounded box with an optional leading icon
/// and a markdown-rendered body.
private struct CalloutBlockView: View {
    let icon: String?
    let content: String

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            if let icon, !icon.isEmpty {
                Text(icon)
            }
            MarkdownBlocksView(markdown: content)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.accentColor.opacity(0.08))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }
}

/// A NotionEnhanced `<details>` block as a default-expanded disclosure whose
/// content renders as markdown.
private struct DisclosureBlockView: View {
    let summary: String
    let content: String
    @State private var expanded = true

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            MarkdownBlocksView(markdown: content)
                .padding(.top, 4)
        } label: {
            Text(Markdown.inlineAttributed(summary))
                .fontWeight(.semibold)
        }
    }
}

/// A NotionEnhanced `<table>` rendered as an aligned grid. The first row is
/// treated as the header and rendered bold.
private struct TableBlockView: View {
    let rows: [[String]]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(rows.enumerated()), id: \.offset) { rowIndex, cells in
                HStack(alignment: .top, spacing: 0) {
                    ForEach(Array(cells.enumerated()), id: \.offset) { _, cell in
                        Text(Markdown.inlineAttributed(cell))
                            .fontWeight(rowIndex == 0 ? .semibold : .regular)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                    }
                }
                .background(rowIndex == 0 ? Color.secondary.opacity(0.08) : Color.clear)
                if rowIndex < rows.count - 1 {
                    Divider()
                }
            }
        }
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .stroke(Color.secondary.opacity(0.2))
        )
    }
}
