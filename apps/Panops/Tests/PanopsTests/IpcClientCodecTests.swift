import Foundation
import Testing
@testable import Panops

@Suite("IpcClient codec")
struct IpcClientCodecTests {
    private var encoder: JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return e
    }
    private let decoder = JSONDecoder()

    @Test("notesGenerate request encodes params as positional array")
    func notesGenerateRequest_encodesParams() throws {
        let req = JsonRpcRequest(
            id: 1,
            method: "ipc.notes.generate",
            param: NotesGenerateParams(
                audio: "/tmp/x.wav",
                dialect: nil,
                language: nil,
                llmProvider: nil,
                llmModel: nil,
                noDiarize: nil,
                meetingId: nil
            )
        )
        let data = try encoder.encode(req)
        let json = String(data: data, encoding: .utf8)!
        // sortedKeys: alphabetical order.
        // Swift's JSONEncoder escapes forward slashes as \/
        #expect(json.contains("\"audio\":\"\\/tmp\\/x.wav\""), "missing audio field: \(json)")
        #expect(json.contains("\"method\":\"ipc.notes.generate\""), "method missing: \(json)")
        #expect(json.contains("\"jsonrpc\":\"2.0\""), "jsonrpc version missing: \(json)")
        // jsonrpsee uses positional params: the single arg is wrapped in a 1-element array.
        #expect(json.contains("\"params\":[{"), "expected positional-array params, got: \(json)")
        // Swift's JSONEncoder omits nil optionals entirely (not encoded as null).
        #expect(!json.contains("\"language\":"), "expected language field to be omitted: \(json)")
    }

    @Test("ServerInfo decodes LLM provider chip payload")
    func serverInfoDecodes() throws {
        let json = #"""
        {"llm":{"provider":"ollama","model":"gemma3:4b","local":true}}
        """#
        let info = try decoder.decode(ServerInfo.self, from: json.data(using: .utf8)!)
        #expect(info.llm.provider == "ollama")
        #expect(info.llm.model == "gemma3:4b")
        #expect(info.llm.local == true)
    }

    @Test("job.done event decodes")
    func jobDoneEvent_decodes() throws {
        let json = #"""
        {
          "type": "job.done",
          "job_id": "abc-123",
          "result": {
            "primary_file": "/tmp/notes.md",
            "assets": ["/tmp/screenshots/s1.png"],
            "meeting_id": "m1"
          }
        }
        """#
        let event = try decoder.decode(IpcEvent.self, from: json.data(using: .utf8)!)
        guard case .jobDone(let jobId, let result) = event else {
            Issue.record("expected .jobDone, got \(event)")
            return
        }
        #expect(jobId == "abc-123")
        #expect(result.primaryFile == "/tmp/notes.md")
        #expect(result.assets == ["/tmp/screenshots/s1.png"])
        #expect(result.meetingId == "m1")
    }

    @Test("job.progress event decodes")
    func jobProgressEvent_decodes() throws {
        let json = #"""
        {
          "type": "job.progress",
          "job_id": "abc-123",
          "stage": "transcribing",
          "current": 1,
          "total": 3,
          "message": "mic track"
        }
        """#
        let event = try decoder.decode(IpcEvent.self, from: json.data(using: .utf8)!)
        guard case .jobProgress(let progress) = event else {
            Issue.record("expected .jobProgress, got \(event)")
            return
        }
        #expect(progress == JobProgressEvent(
            jobId: "abc-123",
            stage: "transcribing",
            current: 1,
            total: 3,
            message: "mic track"
        ))
    }

    @Test("job.progress event decodes with optional fields omitted")
    func jobProgressEvent_decodesWithOptionalFieldsOmitted() throws {
        let json = #"""
        {
          "type": "job.progress",
          "job_id": "abc-123",
          "stage": "generating_notes"
        }
        """#
        let event = try decoder.decode(IpcEvent.self, from: json.data(using: .utf8)!)
        guard case .jobProgress(let progress) = event else {
            Issue.record("expected .jobProgress, got \(event)")
            return
        }
        #expect(progress.jobId == "abc-123")
        #expect(progress.stage == "generating_notes")
        #expect(progress.current == nil)
        #expect(progress.total == nil)
        #expect(progress.message == nil)
    }

    @Test("job.progress notification decodes")
    func jobProgressNotification_decodes() throws {
        let json = #"""
        {
          "jsonrpc": "2.0",
          "method": "events",
          "params": {
            "subscription": 1,
            "result": {
              "type": "job.progress",
              "job_id": "j",
              "stage": "exporting",
              "message": "notes.md"
            }
          }
        }
        """#
        let notification = try decoder.decode(JsonRpcNotification.self, from: json.data(using: .utf8)!)
        guard case .jobProgress(let progress) = notification.params.result else {
            Issue.record("expected .jobProgress, got \(notification.params.result)")
            return
        }
        #expect(progress.jobId == "j")
        #expect(progress.stage == "exporting")
        #expect(progress.current == nil)
        #expect(progress.total == nil)
        #expect(progress.message == "notes.md")
    }

    @Test("job.error event decodes all kinds", arguments: [
        "input_not_found", "invalid_input", "provider_unavailable", "internal", "cancelled"
    ])
    func jobErrorEvent_decodes(kind: String) throws {
        let json = #"""
        {
          "type": "job.error",
          "job_id": "j",
          "error": { "kind": "\#(kind)", "message": "oops" }
        }
        """#
        let event = try decoder.decode(IpcEvent.self, from: json.data(using: .utf8)!)
        guard case .jobError(let jobId, let payload) = event else {
            Issue.record("expected .jobError, got \(event) for kind \(kind)")
            return
        }
        #expect(jobId == "j")
        #expect(payload.kind == kind)
        #expect(payload.message == "oops")
    }


    @Test("job.error event decodes without message", arguments: [
        "input_not_found", "cancelled"
    ])
    func jobErrorEvent_decodesWithoutMessage(kind: String) throws {
        let json = #"""
        {
          "type": "job.error",
          "job_id": "j",
          "error": { "kind": "\#(kind)", "path": "/missing.wav" }
        }
        """#
        let event = try decoder.decode(IpcEvent.self, from: json.data(using: .utf8)!)
        guard case .jobError(let jobId, let payload) = event else {
            Issue.record("expected .jobError, got \(event) for kind \(kind)")
            return
        }
        #expect(jobId == "j")
        #expect(payload.kind == kind)
        #expect(payload.message == "")
    }

    @Test("job.error notification decodes without message")
    func jobErrorNotification_decodesWithoutMessage() throws {
        let json = #"""
        {
          "jsonrpc": "2.0",
          "method": "events",
          "params": {
            "subscription": 1,
            "result": {
              "type": "job.error",
              "job_id": "j",
              "error": { "kind": "cancelled" }
            }
          }
        }
        """#
        let notification = try decoder.decode(JsonRpcNotification.self, from: json.data(using: .utf8)!)
        guard case .jobError(let jobId, let payload) = notification.params.result else {
            Issue.record("expected .jobError, got \(notification.params.result)")
            return
        }
        #expect(jobId == "j")
        #expect(payload.kind == "cancelled")
        #expect(payload.message == "")
    }

    @Test("unknown event type does not throw")
    func unknownEventType_doesNotThrow() throws {
        let json = #"""
        { "type": "future.event", "job_id": "j", "text": "..." }
        """#
        let event = try decoder.decode(IpcEvent.self, from: json.data(using: .utf8)!)
        guard case .unknown(let type) = event else {
            Issue.record("expected .unknown, got \(event)")
            return
        }
        #expect(type == "future.event")
    }

    // MARK: - WebSocket frame parser tests

    @Test("wsFrameParser_unmasked_text_frame")
    func wsFrameParser_unmaskedTextFrame() throws {
        // Text frame (opcode 0x01), FIN=1, unmasked, payload "Hello"
        // Frame: 0x81 0x05 H e l l o
        let payload = "Hello".data(using: .utf8)!
        var frame = Data([0x81, 0x05]) // FIN=1, opcode=1, length=5
        frame.append(payload)

        let result = try WsFrameParser.parse(frame)
        #expect(result != nil)
        #expect(result == payload)
    }

    @Test("wsFrameParser_masked_text_frame")
    func wsFrameParser_maskedTextFrame() throws {
        // Text frame (opcode 0x01), FIN=1, masked
        // Mask key: [0x01, 0x02, 0x03, 0x04]
        // Payload "Hello" (0x48 0x65 0x6C 0x6C 0x6F) masked becomes:
        // 0x48 ^ 0x01 = 0x49, 0x65 ^ 0x02 = 0x67, etc.
        let maskKey = Data([0x01, 0x02, 0x03, 0x04])
        let originalPayload = "Hello".data(using: .utf8)!
        var maskedPayload = Data(count: originalPayload.count)
        for i in 0..<originalPayload.count {
            maskedPayload[i] = originalPayload[i] ^ maskKey[i % 4]
        }

        var frame = Data([0x81, 0x85]) // FIN=1, opcode=1, masked=1, length=5
        frame.append(maskKey)
        frame.append(maskedPayload)

        let result = try WsFrameParser.parse(frame)
        #expect(result != nil)
        #expect(result == originalPayload)
    }

    @Test("wsFrameParser_ignores_binary_frame")
    func wsFrameParser_ignoresBinaryFrame() throws {
        // Binary frame (opcode 0x02), FIN=1, unmasked
        let frame = Data([0x82, 0x02, 0xAB, 0xCD])
        let result = try WsFrameParser.parse(frame)
        #expect(result == nil, "binary frames should be ignored")
    }

    // MARK: - MeetingSummary decode tests

    @Test("MeetingSummary_decodes_4_fields_only")
    func meetingSummary_decodes() throws {
        // Wire contract: MeetingSummary = {id, title, started_at, duration_ms} — 4 fields, all required
        let json = #"""
        {
          "id": "m-123",
          "title": "Team sync",
          "started_at": "2026-06-05T10:00:00Z",
          "duration_ms": 3600000
        }
        """#
        let summary = try decoder.decode(MeetingSummary.self, from: json.data(using: .utf8)!)
        #expect(summary.id == "m-123")
        #expect(summary.title == "Team sync")
        #expect(summary.startedAt == "2026-06-05T10:00:00Z")
        #expect(summary.durationMs == 3600000)
    }

    @Test("Meeting_decodes_with_optional_fields")
    func meeting_decodes_with_optional_fields() throws {
        let json = #"""
        {
          "id": "m-456",
          "title": "Quick call",
          "started_at": "2026-06-05T09:00:00Z",
          "ended_at": null,
          "duration_ms": null,
          "language": "auto",
          "dir_path": "/tmp/meetings/m-456"
        }
        """#
        let meeting = try decoder.decode(Meeting.self, from: json.data(using: .utf8)!)
        #expect(meeting.id == "m-456")
        #expect(meeting.title == "Quick call")
        #expect(meeting.endedAt == nil)
        #expect(meeting.durationMs == nil)
        #expect(meeting.language == "auto")
    }

    @Test("RecordingStartParams with recordVideo and autoGenerateNotes encodes correctly")
    func recordingStartParamsWithFlags_encodes() throws {
        let params = RecordingStartParams(
            meetingId: "m1",
            audioSources: .systemAndMic,
            screenshotIntervalMs: 500,
            screenshotThreshold: 0.15,
            recordVideo: true,
            autoGenerateNotes: false
        )
        let data = try encoder.encode(params)
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"meeting_id\":\"m1\""))
        #expect(json.contains("\"record_video\":true"))
        #expect(json.contains("\"auto_generate_notes\":false"))
    }

    @Test("MeetingDeleteVideoResult decodes from engine response")
    func meetingDeleteVideoResult_decodes() throws {
        let json = #"""
        {
          "deleted": true,
          "freed_bytes": 1048576
        }
        """#
        let result = try decoder.decode(MeetingDeleteVideoResult.self, from: json.data(using: .utf8)!)
        #expect(result.deleted == true)
        #expect(result.freedBytes == 1048576)
    }

    @Test("MeetingDeleteVideoResult decodes with false deleted")
    func meetingDeleteVideoResult_decodesFalse() throws {
        let json = #"""
        {
          "deleted": false,
          "freed_bytes": 0
        }
        """#
        let result = try decoder.decode(MeetingDeleteVideoResult.self, from: json.data(using: .utf8)!)
        #expect(result.deleted == false)
        #expect(result.freedBytes == 0)
    }

    @Test("recording.start params nest the kind-tagged capture target")
    func recordingStartParams_nestCaptureTarget() throws {
        let params = RecordingStartParams(
            meetingId: "m1",
            audioSources: .systemAndMic,
            screenshotIntervalMs: 500,
            screenshotThreshold: 0.15,
            recordVideo: true,
            autoGenerateNotes: true,
            captureTarget: .window(windowID: 456),
            width: 1280,
            height: 720
        )
        let data = try encoder.encode(params)
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"capture_target\":{"))
        #expect(json.contains("\"kind\":\"window\""))
        #expect(json.contains("\"window_id\":456"))
        #expect(json.contains("\"width\":1280"))
        #expect(json.contains("\"height\":720"))
    }

    @Test("recording.start omits width/height when native")
    func recordingStartParams_omitNativeResolution() throws {
        let params = RecordingStartParams(
            meetingId: "m1",
            audioSources: .systemAndMic,
            screenshotIntervalMs: 500,
            screenshotThreshold: 0.15,
            recordVideo: false,
            autoGenerateNotes: true
        )
        let data = try encoder.encode(params)
        let json = String(data: data, encoding: .utf8)!
        #expect(!json.contains("\"width\""), "width should be omitted for native: \(json)")
        #expect(!json.contains("\"height\""), "height should be omitted for native: \(json)")
        #expect(json.contains("\"kind\":\"display\""))
    }

    // MARK: - Organization (Phase B) decode

    @Test("Space decodes from engine response")
    func space_decodes() throws {
        let json = #"{ "id": "space_1", "name": "Work", "position": 0 }"#
        let space = try decoder.decode(Space.self, from: json.data(using: .utf8)!)
        #expect(space == Space(id: "space_1", name: "Work", position: 0))
    }

    @Test("Project decodes with space_id")
    func project_decodes() throws {
        let json = #"{ "id": "p1", "space_id": "s1", "name": "Launch", "position": 2 }"#
        let project = try decoder.decode(Project.self, from: json.data(using: .utf8)!)
        #expect(project == Project(id: "p1", spaceId: "s1", name: "Launch", position: 2))
    }

    @Test("Tag decodes")
    func tag_decodes() throws {
        // Qualify `Panops.Tag` — swift-testing exports its own `Tag` type.
        let json = #"{ "id": "t1", "name": "urgent" }"#
        let tag = try decoder.decode(Panops.Tag.self, from: json.data(using: .utf8)!)
        #expect(tag == Panops.Tag(id: "t1", name: "urgent"))
    }

    @Test("SpaceListResult decodes the {spaces:[...]} wrapper")
    func spaceListResult_decodes() throws {
        let json = #"""
        { "spaces": [
            { "id": "s1", "name": "Work", "position": 0 },
            { "id": "s2", "name": "Personal", "position": 1 }
        ] }
        """#
        let result = try decoder.decode(SpaceListResult.self, from: json.data(using: .utf8)!)
        #expect(result.spaces.count == 2)
        #expect(result.spaces[0] == Space(id: "s1", name: "Work", position: 0))
        #expect(result.spaces[1].name == "Personal")
    }

    @Test("ProjectListResult decodes the {projects:[...]} wrapper")
    func projectListResult_decodes() throws {
        let json = #"{ "projects": [ { "id": "p1", "space_id": "s1", "name": "Launch", "position": 0 } ] }"#
        let result = try decoder.decode(ProjectListResult.self, from: json.data(using: .utf8)!)
        #expect(result.projects == [Project(id: "p1", spaceId: "s1", name: "Launch", position: 0)])
    }

    @Test("TagListResult decodes the {tags:[...]} wrapper")
    func tagListResult_decodes() throws {
        let json = #"{ "tags": [ { "id": "t1", "name": "urgent" }, { "id": "t2", "name": "bug" } ] }"#
        let result = try decoder.decode(TagListResult.self, from: json.data(using: .utf8)!)
        #expect(result.tags.count == 2)
        #expect(result.tags.map(\.name) == ["urgent", "bug"])
    }

    @Test("MeetingSummary decodes the Phase B org fields")
    func meetingSummary_decodesOrgFields() throws {
        let json = #"""
        {
          "id": "m1",
          "title": "Sync",
          "started_at": "2026-06-05T10:00:00Z",
          "duration_ms": 60000,
          "space_id": "s1",
          "project_id": "p1",
          "tags": ["t1", "t2"]
        }
        """#
        let summary = try decoder.decode(MeetingSummary.self, from: json.data(using: .utf8)!)
        #expect(summary.spaceId == "s1")
        #expect(summary.projectId == "p1")
        #expect(summary.tags == ["t1", "t2"])
    }

    @Test("MeetingSummary defaults org fields when omitted (Inbox)")
    func meetingSummary_defaultsOrgFields() throws {
        let json = #"""
        { "id": "m1", "title": "Sync", "started_at": "2026-06-05T10:00:00Z", "duration_ms": 60000 }
        """#
        let summary = try decoder.decode(MeetingSummary.self, from: json.data(using: .utf8)!)
        #expect(summary.spaceId == nil)
        #expect(summary.projectId == nil)
        #expect(summary.tags.isEmpty)
    }

    // MARK: - Organization (Phase B) encode

    @Test("MeetingListParams encodes only the set filter, omitting unset ones")
    func meetingListParams_encodesFilter() throws {
        let data = try encoder.encode(MeetingListParams(spaceId: "s1"))
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"space_id\":\"s1\""), "space_id missing: \(json)")
        // Unset optional filters must be omitted, not sent as null.
        #expect(!json.contains("project_id"), "project_id should be omitted: \(json)")
        #expect(!json.contains("tag_id"), "tag_id should be omitted: \(json)")
    }

    @Test("filtered meeting.list request encodes params as positional array")
    func meetingListFilteredRequest_encodes() throws {
        let req = JsonRpcRequest(
            id: 1,
            method: "ipc.meeting.list",
            param: MeetingListParams(tagId: "t1")
        )
        let data = try encoder.encode(req)
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"method\":\"ipc.meeting.list\""), "method missing: \(json)")
        #expect(json.contains("\"params\":[{"), "expected positional-array params: \(json)")
        #expect(json.contains("\"tag_id\":\"t1\""), "tag_id missing: \(json)")
    }

    @Test("MeetingAssignParams omits nil refs; move-to-inbox sends only meeting_id")
    func meetingAssignParams_encodes() throws {
        let toSpace = try encoder.encode(MeetingAssignParams(meetingId: "m1", spaceId: "s1", projectId: nil))
        let toSpaceJson = String(data: toSpace, encoding: .utf8)!
        #expect(toSpaceJson.contains("\"meeting_id\":\"m1\""))
        #expect(toSpaceJson.contains("\"space_id\":\"s1\""))
        #expect(!toSpaceJson.contains("project_id"), "nil project_id should be omitted: \(toSpaceJson)")

        let toInbox = try encoder.encode(MeetingAssignParams(meetingId: "m1", spaceId: nil, projectId: nil))
        let toInboxJson = String(data: toInbox, encoding: .utf8)!
        #expect(toInboxJson.contains("\"meeting_id\":\"m1\""))
        #expect(!toInboxJson.contains("space_id"), "nil space_id should be omitted: \(toInboxJson)")
        #expect(!toInboxJson.contains("project_id"), "nil project_id should be omitted: \(toInboxJson)")
    }

    @Test("TagAssignParams encodes meeting_id and tag_id")
    func tagAssignParams_encodes() throws {
        let data = try encoder.encode(TagAssignParams(meetingId: "m1", tagId: "t1"))
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"meeting_id\":\"m1\""))
        #expect(json.contains("\"tag_id\":\"t1\""))
    }

    @Test("ProjectCreateParams encodes space_id")
    func projectCreateParams_encodes() throws {
        let data = try encoder.encode(ProjectCreateParams(spaceId: "s1", name: "Launch"))
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"space_id\":\"s1\""))
        #expect(json.contains("\"name\":\"Launch\""))
    }

    @Test("ProjectListParams omits space_id when listing all projects")
    func projectListParams_encodesAll() throws {
        let all = String(data: try encoder.encode(ProjectListParams(spaceId: nil)), encoding: .utf8)!
        #expect(all == "{}", "all-projects list should encode to {}: \(all)")
        let scoped = String(data: try encoder.encode(ProjectListParams(spaceId: "s1")), encoding: .utf8)!
        #expect(scoped.contains("\"space_id\":\"s1\""))
    }

    // MARK: - Editing-save slice (stage 1/2 wire types)

    @Test("MeetingRenameParams encodes meeting_id + title (snake_case)")
    func meetingRenameParams_encodes() throws {
        let req = JsonRpcRequest(
            id: 7,
            method: "ipc.meeting.rename",
            param: MeetingRenameParams(meetingId: "m-42", title: "Q3 planning")
        )
        let data = try encoder.encode(req)
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"method\":\"ipc.meeting.rename\""))
        #expect(json.contains("\"meeting_id\":\"m-42\""))
        #expect(json.contains("\"title\":\"Q3 planning\""))
        #expect(json.contains("\"params\":[{"), "expected positional-array params: \(json)")
    }

    @Test("MeetingRenameParams allows an empty title (engine validates non-emptiness)")
    func meetingRenameParams_emptyTitleAllowed() throws {
        // Empty title encoding matches the engine's round-trip test which
        // accepts an empty string. Wire-level we just need to confirm the
        // field is present, not rejected by JSONEncoder.
        let data = try encoder.encode(MeetingRenameParams(meetingId: "m", title: ""))
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"title\":\"\""))
        #expect(json.contains("\"meeting_id\":\"m\""))
    }

    @Test("NotesSaveParams encodes meeting_id + markdown with newlines preserved")
    func notesSaveParams_encodes() throws {
        let markdown = "# Heading\n\nBody with \"quotes\" and \\backslash."
        let req = JsonRpcRequest(
            id: 8,
            method: "ipc.notes.save",
            param: NotesSaveParams(meetingId: "m-42", markdown: markdown)
        )
        let data = try encoder.encode(req)
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"method\":\"ipc.notes.save\""))
        #expect(json.contains("\"meeting_id\":\"m-42\""))
        #expect(json.contains("\"markdown\":"))
        // Newlines are JSON-escaped as \n; quotes/backslashes also escaped.
        #expect(json.contains("\\n"), "expected escaped newline in: \(json)")
    }

    @Test("NotesSaveParams allows empty markdown (clears the notes content)")
    func notesSaveParams_emptyMarkdownAllowed() throws {
        let data = try encoder.encode(NotesSaveParams(meetingId: "m", markdown: ""))
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"markdown\":\"\""))
    }
}
