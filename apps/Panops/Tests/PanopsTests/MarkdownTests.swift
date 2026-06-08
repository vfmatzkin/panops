import Foundation
import Testing
@testable import Panops

@Suite("Markdown helpers")
struct MarkdownTests {
    @Test("stripFrontmatter removes leading YAML block")
    func stripsFrontmatter() {
        let src = """
        ---
        title: Team sync
        date: 2026-06-05
        ---
        # Summary
        Body text.
        """
        let body = Markdown.stripFrontmatter(src)
        #expect(!body.contains("title: Team sync"))
        #expect(body.hasPrefix("# Summary"))
    }

    @Test("stripFrontmatter leaves body without frontmatter untouched")
    func noFrontmatter() {
        let src = "# Summary\nBody text."
        #expect(Markdown.stripFrontmatter(src) == src)
    }

    @Test("stripFrontmatter leaves unterminated frontmatter untouched")
    func unterminatedFrontmatter() {
        let src = "---\ntitle: oops\nno closing fence"
        #expect(Markdown.stripFrontmatter(src) == src)
    }

    @Test("parseBlocks classifies headings, bullets, ordered, paragraphs")
    func parsesBlocks() {
        let src = """
        # Heading one
        A paragraph line
        spanning two lines.

        - bullet a
        - bullet b
        1. first
        2) second
        > a quote
        """
        let blocks = Markdown.parseBlocks(src)
        #expect(blocks.contains(.heading(level: 1, text: "Heading one")))
        #expect(blocks.contains(.paragraph(text: "A paragraph line spanning two lines.")))
        #expect(blocks.contains(.bullet(text: "bullet a")))
        #expect(blocks.contains(.bullet(text: "bullet b")))
        #expect(blocks.contains(.ordered(number: 1, text: "first")))
        #expect(blocks.contains(.ordered(number: 2, text: "second")))
        #expect(blocks.contains(.quote(text: "a quote")))
    }

    @Test("parseBlocks captures fenced code verbatim")
    func parsesCode() {
        let src = """
        intro

        ```
        let x = 1
        let y = 2
        ```
        """
        let blocks = Markdown.parseBlocks(src)
        #expect(blocks.contains(.code(text: "let x = 1\nlet y = 2")))
    }

    // MARK: - NotionEnhanced container tags

    private func callouts(in blocks: [MarkdownBlock]) -> [(icon: String?, body: String)] {
        blocks.compactMap { block in
            if case let .callout(icon, body) = block { return (icon, body) }
            return nil
        }
    }

    @Test("parseBlocks renders a callout with an icon")
    func calloutWithIcon() {
        let blocks = Markdown.parseBlocks(#"<callout icon="🎯">Watch the budget.</callout>"#)
        let found = callouts(in: blocks)
        #expect(found.count == 1)
        #expect(found.first?.icon == "🎯")
        #expect(found.first?.body == "Watch the budget.")
        #expect(!blocks.contains { describe($0).contains("<callout") })
    }

    @Test("parseBlocks renders a callout without an icon")
    func calloutWithoutIcon() {
        let blocks = Markdown.parseBlocks("<callout>Heads up.</callout>")
        let found = callouts(in: blocks)
        #expect(found.count == 1)
        #expect(found.first?.icon == nil)
        #expect(found.first?.body == "Heads up.")
    }

    @Test("parseBlocks parses a multi-line details/summary block")
    func disclosureMultiLine() {
        let src = """
        <details><summary>Rollout plan</summary>
        First we ship the parser.
        Then we wire the view.
        </details>
        """
        let blocks = Markdown.parseBlocks(src)
        let found: (summary: String, body: String)? = blocks.compactMap { block in
            if case let .disclosure(summary, body) = block { return (summary: summary, body: body) }
            return nil
        }.first
        #expect(found?.summary == "Rollout plan")
        #expect(found?.body.contains("First we ship the parser.") == true)
        #expect(found?.body.contains("Then we wire the view.") == true)
        #expect(!blocks.contains { describe($0).contains("<summary") })
    }

    @Test("parseBlocks parses table rows and cells")
    func tableRows() {
        let src = "<table><tr><th>Name</th><th>Role</th></tr><tr><td>Ana</td><td>PM</td></tr></table>"
        let blocks = Markdown.parseBlocks(src)
        let rows: [[String]]? = blocks.compactMap { block in
            if case let .table(rows) = block { return rows }
            return nil
        }.first
        #expect(rows == [["Name", "Role"], ["Ana", "PM"]])
    }

    @Test("parseBlocks strips an unknown angle-bracket tag")
    func stripsUnknownTag() {
        let blocks = Markdown.parseBlocks("See <span>this</span> note.")
        #expect(blocks == [.paragraph(text: "See this note.")])
        #expect(!blocks.contains { describe($0).contains("<") })
    }

    @Test("parseBlocks lifts an inline callout out of a paragraph")
    func inlineCalloutDoesNotLeak() {
        let blocks = Markdown.parseBlocks(#"Intro text <callout icon="🎯">be careful</callout> outro text"#)
        #expect(!blocks.contains { describe($0).contains("<callout") })
        #expect(!blocks.contains { describe($0).contains("</callout>") })
        let found = callouts(in: blocks)
        #expect(found.count == 1)
        #expect(found.first?.body == "be careful")
        #expect(blocks.contains(.paragraph(text: "Intro text")))
        #expect(blocks.contains(.paragraph(text: "outro text")))
    }

    @Test("parseBlocks leaves plain `<` in prose untouched")
    func keepsBareLessThan() {
        let blocks = Markdown.parseBlocks("The value a < b holds.")
        #expect(blocks == [.paragraph(text: "The value a < b holds.")])
    }

    @Test("parseBlocks keeps a NotionEnhanced tag inside fenced code as code")
    func fencedTagStaysCode() {
        let src = """
        Before the block

        ```
        <table><tr><td>x</td></tr></table>
        ```

        After the block
        """
        let blocks = Markdown.parseBlocks(src)
        // The fenced tag must NOT be lifted into a table block.
        #expect(!blocks.contains { if case .table = $0 { return true }; return false })
        // It stays verbatim inside the code block.
        #expect(blocks.contains { block in
            guard case let .code(text) = block else { return false }
            return text.contains("<table>") && text.contains("</table>")
        })
        #expect(blocks.contains(.paragraph(text: "Before the block")))
        #expect(blocks.contains(.paragraph(text: "After the block")))
    }

    /// Render any block to a string for raw-tag-leak assertions.
    private func describe(_ block: MarkdownBlock) -> String {
        switch block {
        case .heading(_, let text): return text
        case .bullet(let text): return text
        case .ordered(_, let text): return text
        case .paragraph(let text): return text
        case .code(let text): return text
        case .quote(let text): return text
        case .callout(let icon, let body): return (icon ?? "") + body
        case .disclosure(let summary, let body): return summary + body
        case .table(let rows): return rows.flatMap { $0 }.joined(separator: " ")
        }
    }
}
