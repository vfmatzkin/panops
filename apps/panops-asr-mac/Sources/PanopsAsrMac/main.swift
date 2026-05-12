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

while let line = readLine(strippingNewline: true) {
    if line.isEmpty { continue }
    let data = Data(line.utf8)
    let request: JsonRpcRequest
    do {
        request = try decoder.decode(JsonRpcRequest.self, from: data)
    } catch {
        let err = JsonRpcResponse(
            id: 0,
            error: JsonRpcError(code: -32700, message: "parse error: \(error)")
        )
        if let body = try? encoder.encode(err), let s = String(data: body, encoding: .utf8) {
            print(s)
            fflush(stdout)
        }
        continue
    }

    guard request.method == "asr.transcribe",
          let params = request.params.first
    else {
        let err = JsonRpcResponse(
            id: request.id,
            error: JsonRpcError(
                code: -32601,
                message: "method not found or missing params: \(request.method)"
            )
        )
        if let body = try? encoder.encode(err), let s = String(data: body, encoding: .utf8) {
            print(s)
            fflush(stdout)
        }
        continue
    }

    do {
        let transcript = try await transcriber.transcribe(
            audioPath: params.audio,
            languageHint: params.languageHint
        )
        let response = JsonRpcResponse(id: request.id, result: transcript)
        let body = try encoder.encode(response)
        print(String(data: body, encoding: .utf8)!)
        fflush(stdout)
    } catch {
        let err = JsonRpcResponse(
            id: request.id,
            error: JsonRpcError(code: -32000, message: "transcribe failed: \(error)")
        )
        if let body = try? encoder.encode(err), let s = String(data: body, encoding: .utf8) {
            print(s)
            fflush(stdout)
        }
    }
}

FileHandle.standardError.write(Data("panops-asr-mac EOF; exiting\n".utf8))
