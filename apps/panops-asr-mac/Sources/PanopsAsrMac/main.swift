import Foundation

FileHandle.standardError.write(Data("panops-asr-mac starting\n".utf8))

let transcriber: Transcriber
do {
    transcriber = try await Transcriber.makeShared()
} catch {
    FileHandle.standardError.write(Data("WhisperKit init failed: \(error)\n".utf8))
    exit(2)
}

FileHandle.standardError.write(Data("panops-asr-mac ready\n".utf8))

let decoder = JSONDecoder()
let encoder = JSONEncoder()

func emit(_ response: JsonRpcResponse) {
    guard let body = try? encoder.encode(response),
          let line = String(data: body, encoding: .utf8) else {
        // Encoder failure or non-utf8 bytes (shouldn't happen for our
        // own response types). Fall back to a hand-rolled minimal
        // error envelope so the Rust caller still sees a valid line
        // instead of a process crash.
        let id = response.id.map(String.init) ?? "null"
        print("{\"jsonrpc\":\"2.0\",\"id\":\(id),\"error\":{\"code\":-32603,\"message\":\"response encode failed\"}}")
        fflush(stdout)
        return
    }
    print(line)
    fflush(stdout)
}

while let line = readLine(strippingNewline: true) {
    if line.isEmpty { continue }
    let data = Data(line.utf8)
    let request: JsonRpcRequest
    do {
        request = try decoder.decode(JsonRpcRequest.self, from: data)
    } catch {
        // JSON-RPC 2.0 §4: parse-error responses use `null` id since
        // we can't recover the request id from an unparsable line.
        // Full parse-error detail goes to stderr (Console.app) so we
        // don't leak it back over the wire.
        FileHandle.standardError.write(Data("parse error: \(error)\n".utf8))
        emit(JsonRpcResponse(
            id: nil,
            error: JsonRpcError(code: -32700, message: "parse error")
        ))
        continue
    }

    guard request.method == "asr.transcribe",
          let params = request.params.first
    else {
        emit(JsonRpcResponse(
            id: request.id,
            error: JsonRpcError(
                code: -32601,
                message: "method not found or missing params"
            )
        ))
        continue
    }

    do {
        let transcript = try await transcriber.transcribe(
            audioPath: params.audio,
            languageHint: params.languageHint
        )
        emit(JsonRpcResponse(id: request.id, result: transcript))
    } catch {
        // Full WhisperKit failure detail (model paths, CoreML state)
        // goes to stderr; an opaque message goes over the wire so the
        // engine's IPC `Internal` error doesn't echo sidecar internals
        // to clients.
        FileHandle.standardError.write(Data("transcribe failed: \(error)\n".utf8))
        emit(JsonRpcResponse(
            id: request.id,
            error: JsonRpcError(code: -32000, message: "transcribe failed")
        ))
    }
}

FileHandle.standardError.write(Data("panops-asr-mac EOF; exiting\n".utf8))
