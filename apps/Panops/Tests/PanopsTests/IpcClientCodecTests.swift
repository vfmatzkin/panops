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
}
