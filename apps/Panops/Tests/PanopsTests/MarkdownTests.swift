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
}
