import Foundation
import Testing
@testable import Panops

@Suite("StructuredNotes decoding")
struct StructuredNotesTests {
    @Test("StructuredNotes decodes engine-shaped notes.json")
    func decodesFullNotesJson() throws {
        let json = #"""
        {
          "schema_version": 1,
          "language": "en",
          "generated_at": "2026-06-05T10:01:00Z",
          "frontmatter": {
            "title": "Team sync",
            "date": "2026-06-05",
            "started_at": "2026-06-05T10:00:00+00:00",
            "duration_ms": 1800000,
            "speakers": ["Speaker 1", "Speaker 2"],
            "languages": ["en", "es"],
            "tags": ["planning", "q3"],
            "template": "default",
            "dialect": "notion-enhanced",
            "panops_version": "0.1.0",
            "source_audio": "system.wav"
          },
          "sections": [
            {
              "index": 1,
              "title": "Intro",
              "time_range_ms": [0, 600000],
              "narrative_md": "We **kicked off** the sync.\n\n## Topics\n- one\n- two",
              "key_points": ["Aligned on scope", "Picked owners"],
              "action_items": [
                { "description": "Draft the brief", "owner": "Fran", "due": "2026-06-10" },
                { "description": "Book the room", "owner": null }
              ],
              "screenshots": [
                { "ms_since_start": 120000, "path": "screenshots/shot-1.png", "caption": "Slide" }
              ]
            }
          ]
        }
        """#

        let notes = try JSONDecoder().decode(StructuredNotes.self, from: Data(json.utf8))

        #expect(notes.schemaVersion == 1)
        #expect(notes.language == "en")
        #expect(notes.frontmatter.title == "Team sync")
        #expect(notes.frontmatter.speakers.count == 2)
        #expect(notes.frontmatter.languages == ["en", "es"])
        #expect(notes.frontmatter.tags == ["planning", "q3"])
        #expect(notes.frontmatter.dialect == "notion-enhanced")
        #expect(notes.frontmatter.sourceAudio == "system.wav")

        #expect(notes.sections.count == 1)
        let section = notes.sections[0]
        #expect(section.title == "Intro")
        #expect(section.timeRangeMs == [0, 600000])
        #expect(section.keyPoints.count == 2)
        #expect(section.actionItems.count == 2)
        #expect(section.actionItems[0].owner == "Fran")
        #expect(section.actionItems[0].due == "2026-06-10")
        #expect(section.actionItems[1].owner == nil)
        #expect(section.screenshots.count == 1)
        #expect(section.screenshots[0].path == "screenshots/shot-1.png")
        #expect(section.screenshots[0].msSinceStart == 120000)
    }

    @Test("StructuredNotes decodes leniently when optional fields are absent")
    func decodesLenient() throws {
        // Minimal frontmatter, no sections, no source_audio, no captions.
        let json = #"""
        {
          "schema_version": 1,
          "language": "es",
          "generated_at": "2026-06-05T10:01:00Z",
          "frontmatter": {
            "title": "Quick call",
            "date": "2026-06-05",
            "started_at": "2026-06-05T10:00:00+00:00",
            "duration_ms": 60000,
            "speakers": [],
            "languages": ["es"],
            "tags": [],
            "template": "default",
            "dialect": "basic",
            "panops_version": "0.1.0"
          },
          "sections": []
        }
        """#

        let notes = try JSONDecoder().decode(StructuredNotes.self, from: Data(json.utf8))
        #expect(notes.frontmatter.sourceAudio == nil)
        #expect(notes.sections.isEmpty)
        #expect(notes.frontmatter.dialect == "basic")
    }
}

@Suite("MeetingSummary lenient fields")
struct MeetingSummaryLenientTests {
    @Test("MeetingSummary decodes new fields when present")
    func decodesNewFields() throws {
        let json = #"""
        {
          "id": "m-1",
          "title": "Sync",
          "started_at": "2026-06-05T10:00:00Z",
          "duration_ms": 1800000,
          "language": "en",
          "ended_at": "2026-06-05T10:30:00Z",
          "has_notes": true
        }
        """#
        let summary = try JSONDecoder().decode(MeetingSummary.self, from: Data(json.utf8))
        #expect(summary.language == "en")
        #expect(summary.endedAt == "2026-06-05T10:30:00Z")
        #expect(summary.hasNotes == true)
    }

    @Test("MeetingSummary defaults new fields when absent (old wire shape)")
    func defaultsWhenAbsent() throws {
        let json = #"""
        {
          "id": "m-2",
          "title": "Old shape",
          "started_at": "2026-06-05T09:00:00Z",
          "duration_ms": 1000
        }
        """#
        let summary = try JSONDecoder().decode(MeetingSummary.self, from: Data(json.utf8))
        #expect(summary.language == "")
        #expect(summary.endedAt == nil)
        #expect(summary.hasNotes == false)
    }
}
