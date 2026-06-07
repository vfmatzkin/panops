import Foundation

// Stateful capture sidecar. Unlike the request/response ASR sidecar, capture
// is a session: `capture.start` opens an SCStream + screenshotter and acks;
// `capture.stop` finalizes and returns the paths. One session at a time.

FileHandle.standardError.write(Data("panops-capture-mac starting\n".utf8))

/// A live capture: the SCStream recorder + the screen-frame screenshotter,
/// keyed by the meeting that started it.
final class CaptureSession {
    let meetingId: String
    let recorder: Recorder
    let screenshotter: Screenshotter
    init(meetingId: String, recorder: Recorder, screenshotter: Screenshotter) {
        self.meetingId = meetingId
        self.recorder = recorder
        self.screenshotter = screenshotter
    }
}

var current: CaptureSession?

let decoder = JSONDecoder()
let encoder = JSONEncoder()

func emit<R: Encodable>(_ response: JsonRpcResponse<R>) {
    guard let body = try? encoder.encode(response),
          let line = String(data: body, encoding: .utf8) else {
        let id = response.id.map(String.init) ?? "null"
        print("{\"jsonrpc\":\"2.0\",\"id\":\(id),\"error\":{\"code\":-32603,\"message\":\"response encode failed\"}}")
        fflush(stdout)
        return
    }
    print(line)
    fflush(stdout)
}

func emitError(id: UInt64?, code: Int, message: String) {
    emit(JsonRpcResponse<Empty>(id: id, error: JsonRpcError(code: code, message: message)))
}

/// Map a capture failure to an opaque JSON-RPC code. Full detail goes to
/// stderr (Console.app); the wire carries a code-only message so the engine
/// adapter never echoes ScreenCaptureKit/TCC internals to clients.
func failureCode(_ error: Error) -> (Int, String) {
    if let failure = error as? CaptureFailure {
        switch failure {
        case .noDisplay: return (-32000, "no display available")
        case .permissionDenied(let what):
            return what == "microphone" ? (-32002, "microphone") : (-32001, "screen recording")
        }
    }
    // ScreenCaptureKit surfaces TCC denial as an SCStream error; treat an
    // unrecognized start/stop failure as a generic sidecar error.
    return (-32000, "capture failed")
}

FileHandle.standardError.write(Data("panops-capture-mac ready\n".utf8))

while let line = readLine(strippingNewline: true) {
    if line.isEmpty { continue }
    let data = Data(line.utf8)
    let request: JsonRpcRequest
    do {
        request = try decoder.decode(JsonRpcRequest.self, from: data)
    } catch {
        FileHandle.standardError.write(Data("parse error: \(error)\n".utf8))
        emitError(id: nil, code: -32700, message: "parse error")
        continue
    }

    guard let params = request.params.first else {
        emitError(id: request.id, code: -32602, message: "missing params")
        continue
    }

    switch request.method {
    case "capture.start":
        guard let meetingId = params.meetingId else {
            emitError(id: request.id, code: -32602, message: "missing meeting_id")
            continue
        }
        if current != nil {
            emitError(id: request.id, code: -32000, message: "capture already running")
            continue
        }
        let plan = TrackPlan(audioSources: params.audioSources ?? "system_and_mic")
        do {
            let recorder = try Recorder(
                plan: plan,
                systemPath: params.systemAudioPath,
                micPath: params.micAudioPath
            )
            let screenshotter = Screenshotter(
                dir: params.screenshotsDir ?? FileManager.default.temporaryDirectory.path,
                intervalMs: params.screenshotIntervalMs ?? 500,
                threshold: params.screenshotThreshold ?? 0.15
            )
            try await recorder.start(screenshotter: screenshotter)
            current = CaptureSession(
                meetingId: meetingId, recorder: recorder, screenshotter: screenshotter
            )
            emit(JsonRpcResponse(
                id: request.id,
                result: StartedResult(startedAtMs: recorder.startedAtMs)
            ))
        } catch {
            FileHandle.standardError.write(Data("capture.start failed: \(error)\n".utf8))
            let (code, message) = failureCode(error)
            emitError(id: request.id, code: code, message: message)
        }

    case "capture.stop":
        guard let meetingId = params.meetingId,
              let session = current, session.meetingId == meetingId else {
            emitError(id: request.id, code: -32004, message: "session not found")
            continue
        }
        do {
            let (systemPath, micPath, durationMs) = try await session.recorder.stop()
            let result = StoppedResult(
                systemAudioPath: systemPath,
                micAudioPath: micPath,
                screenshotPaths: session.screenshotter.keptPaths(),
                durationMs: durationMs
            )
            current = nil
            emit(JsonRpcResponse(id: request.id, result: result))
        } catch {
            FileHandle.standardError.write(Data("capture.stop failed: \(error)\n".utf8))
            current = nil
            let (code, message) = failureCode(error)
            emitError(id: request.id, code: code, message: message)
        }

    default:
        emitError(id: request.id, code: -32601, message: "method not found")
    }
}

FileHandle.standardError.write(Data("panops-capture-mac EOF; exiting\n".utf8))
