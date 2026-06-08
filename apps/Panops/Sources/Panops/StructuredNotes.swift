import Foundation

/// Swift mirror of `panops_core::notes::ir::StructuredNotes` — the structured
/// notes IR written to `notes.json` next to `notes.md` in each meeting dir.
///
/// Keep field names/types in sync with `crates/panops-core/src/notes/ir.rs`.
/// Decoding is lenient on purpose: the engine writes `notes.json` from a
/// separately-landing change, so fields the engine may add later (or omit on
/// older files) must not break decode. Date/time fields arrive as RFC3339 /
/// `YYYY-MM-DD` strings (serde serializes chrono types as strings); they are
/// decoded as `String` and parsed for display only.
struct StructuredNotes: Decodable {
    let schemaVersion: UInt32
    let frontmatter: NotesFrontmatter
    let sections: [NotesSection]
    let language: String
    let generatedAt: String

    enum CodingKeys: String, CodingKey {
        case schemaVersion = "schema_version"
        case frontmatter
        case sections
        case language
        case generatedAt = "generated_at"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        schemaVersion = try c.decodeIfPresent(UInt32.self, forKey: .schemaVersion) ?? 0
        frontmatter = try c.decode(NotesFrontmatter.self, forKey: .frontmatter)
        sections = try c.decodeIfPresent([NotesSection].self, forKey: .sections) ?? []
        language = try c.decodeIfPresent(String.self, forKey: .language) ?? ""
        generatedAt = try c.decodeIfPresent(String.self, forKey: .generatedAt) ?? ""
    }
}

struct NotesFrontmatter: Decodable {
    let title: String
    let date: String
    let startedAt: String
    let durationMs: UInt64
    let speakers: [String]
    let languages: [String]
    let tags: [String]
    let template: String
    let dialect: String
    let panopsVersion: String
    let sourceAudio: String?

    enum CodingKeys: String, CodingKey {
        case title
        case date
        case startedAt = "started_at"
        case durationMs = "duration_ms"
        case speakers
        case languages
        case tags
        case template
        case dialect
        case panopsVersion = "panops_version"
        case sourceAudio = "source_audio"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        date = try c.decodeIfPresent(String.self, forKey: .date) ?? ""
        startedAt = try c.decodeIfPresent(String.self, forKey: .startedAt) ?? ""
        durationMs = try c.decodeIfPresent(UInt64.self, forKey: .durationMs) ?? 0
        speakers = try c.decodeIfPresent([String].self, forKey: .speakers) ?? []
        languages = try c.decodeIfPresent([String].self, forKey: .languages) ?? []
        tags = try c.decodeIfPresent([String].self, forKey: .tags) ?? []
        template = try c.decodeIfPresent(String.self, forKey: .template) ?? ""
        // `dialect` serializes kebab-case ("notion-enhanced"); decode as a plain
        // string and present it humanized rather than mapping a Swift enum.
        dialect = try c.decodeIfPresent(String.self, forKey: .dialect) ?? ""
        panopsVersion = try c.decodeIfPresent(String.self, forKey: .panopsVersion) ?? ""
        sourceAudio = try c.decodeIfPresent(String.self, forKey: .sourceAudio)
    }
}

struct NotesSection: Decodable, Identifiable {
    let index: UInt32
    let title: String
    /// `time_range_ms` is a serde 2-tuple → encoded as a JSON array `[start, end]`.
    let timeRangeMs: [UInt64]
    let narrativeMd: String
    let keyPoints: [String]
    let actionItems: [ActionItem]
    let screenshots: [Screenshot]

    var id: UInt32 { index }

    enum CodingKeys: String, CodingKey {
        case index
        case title
        case timeRangeMs = "time_range_ms"
        case narrativeMd = "narrative_md"
        case keyPoints = "key_points"
        case actionItems = "action_items"
        case screenshots
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        index = try c.decodeIfPresent(UInt32.self, forKey: .index) ?? 0
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        timeRangeMs = try c.decodeIfPresent([UInt64].self, forKey: .timeRangeMs) ?? []
        narrativeMd = try c.decodeIfPresent(String.self, forKey: .narrativeMd) ?? ""
        keyPoints = try c.decodeIfPresent([String].self, forKey: .keyPoints) ?? []
        actionItems = try c.decodeIfPresent([ActionItem].self, forKey: .actionItems) ?? []
        screenshots = try c.decodeIfPresent([Screenshot].self, forKey: .screenshots) ?? []
    }
}

struct ActionItem: Decodable, Identifiable {
    let description: String
    let owner: String?
    let due: String?

    let id = UUID()

    enum CodingKeys: String, CodingKey {
        case description
        case owner
        case due
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        owner = try c.decodeIfPresent(String.self, forKey: .owner)
        due = try c.decodeIfPresent(String.self, forKey: .due)
    }
}

struct Screenshot: Decodable, Identifiable {
    let msSinceStart: UInt64
    let path: String
    let caption: String?

    var id: String { "\(msSinceStart)-\(path)" }

    enum CodingKeys: String, CodingKey {
        case msSinceStart = "ms_since_start"
        case path
        case caption
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        msSinceStart = try c.decodeIfPresent(UInt64.self, forKey: .msSinceStart) ?? 0
        path = try c.decodeIfPresent(String.self, forKey: .path) ?? ""
        caption = try c.decodeIfPresent(String.self, forKey: .caption)
    }
}
