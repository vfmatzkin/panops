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

    @Test("unknown event type does not throw")
    func unknownEventType_doesNotThrow() throws {
        let json = #"""
        { "type": "asr.partial", "job_id": "j", "text": "..." }
        """#
        let event = try decoder.decode(IpcEvent.self, from: json.data(using: .utf8)!)
        guard case .unknown(let type) = event else {
            Issue.record("expected .unknown, got \(event)")
            return
        }
        #expect(type == "asr.partial")
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
}
