use serde::{Deserialize, Serialize};

/// Markdown flavour the notes are emitted in. Affects both LLM prompts (the
/// cheat-sheet handed to the model) and `MarkdownExporter` rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkdownDialect {
    /// Default. Notion-flavored markdown: `<callout>`, `<details>`, `<table>`,
    /// `{color="..."}` block colors. Renders cleanly in Notion; degrades
    /// gracefully in other viewers.
    #[default]
    NotionEnhanced,
    /// CommonMark only. For Obsidian / GitHub / vanilla viewers.
    Basic,
}

impl MarkdownDialect {
    /// Compact reference of allowed syntax for this dialect. Handed to the LLM
    /// in every prompt that emits markdown so the response stays in-dialect.
    pub fn cheat_sheet(self) -> &'static str {
        match self {
            Self::NotionEnhanced => CHEAT_SHEET_NOTION_ENHANCED,
            Self::Basic => CHEAT_SHEET_BASIC,
        }
    }
}

const CHEAT_SHEET_NOTION_ENHANCED: &str = "\
You are emitting Notion-flavored markdown. Allowed constructs:
- CommonMark: headings (#, ##, ###), lists (-, 1.), tables, fenced code blocks, blockquotes, links, images.
- <callout icon=\"🎯\">…</callout> for highlighted notes.
- <details><summary>Title</summary>…</details> for collapsible blocks.
- <table>…</table> with explicit <tr>/<td> for rich tables.
- {color=\"red\"} after a paragraph or heading for block colors.
- Inline mentions: @user, @date.
NEVER emit raw HTML beyond the listed tags. NEVER nest callouts.
";

const CHEAT_SHEET_BASIC: &str = "\
You are emitting strict CommonMark. Allowed constructs:
- Headings (#, ##, ###).
- Unordered lists (-) and ordered lists (1.).
- Pipe tables.
- Fenced code blocks with language tags.
- Blockquotes (>).
- Links and image embeds.
NEVER emit HTML tags, callouts, color attributes, or Notion-specific extensions.
This is for Obsidian / GitHub / vanilla CommonMark viewers.
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_round_trips_through_kebab_case_serde() {
        let s = serde_json::to_string(&MarkdownDialect::NotionEnhanced).unwrap();
        assert_eq!(s, "\"notion-enhanced\"");
        let s = serde_json::to_string(&MarkdownDialect::Basic).unwrap();
        assert_eq!(s, "\"basic\"");
    }

    #[test]
    fn cheat_sheet_for_notion_enhanced_mentions_callout_and_details() {
        let sheet = MarkdownDialect::NotionEnhanced.cheat_sheet();
        assert!(sheet.contains("<callout"));
        assert!(sheet.contains("<details>"));
    }

    #[test]
    fn cheat_sheet_for_basic_does_not_mention_callout() {
        let sheet = MarkdownDialect::Basic.cheat_sheet();
        assert!(!sheet.contains("<callout"));
        assert!(sheet.contains("CommonMark") || sheet.contains("commonmark"));
    }

    #[test]
    fn default_is_notion_enhanced() {
        assert_eq!(MarkdownDialect::default(), MarkdownDialect::NotionEnhanced);
    }
}
