import Foundation
import FoundationModels

FileHandle.standardError.write(Data("panops-llm-mac starting\n".utf8))

let decoder = JSONDecoder()
let encoder = JSONEncoder()

func emit(_ response: JsonRpcResponse) {
    guard let body = try? encoder.encode(response),
          let line = String(data: body, encoding: .utf8) else {
        let id = response.id.map { (try? String(data: encoder.encode($0), encoding: .utf8)) ?? "null" } ?? "null"
        print("{\"jsonrpc\":\"2.0\",\"id\":\(id),\"error\":{\"code\":-32603,\"message\":\"response encode failed\"}}")
        fflush(stdout)
        return
    }
    print(line)
    fflush(stdout)
}

func errorResponse(id: JSONValue?, code: Int, message: String) -> JsonRpcResponse {
    JsonRpcResponse(id: id, error: JsonRpcError(code: code, message: message))
}

func decodeParams<T: Decodable>(_ type: T.Type, from params: JSONValue?) throws -> T {
    guard let params else {
        throw CodecError.invalidSchema("missing params")
    }
    if let array = params.arrayValue {
        guard let first = array.first else {
            throw CodecError.invalidSchema("params array is empty")
        }
        return try JSONValue.decode(type, from: first)
    }
    return try JSONValue.decode(type, from: params)
}

guard #available(macOS 26.0, *) else {
    FileHandle.standardError.write(Data("FoundationModels requires macOS 26.0; probe will report unavailable\n".utf8))
    while let line = readLine(strippingNewline: true) {
        if line.isEmpty { continue }
        let request: JsonRpcRequest
        do {
            request = try decoder.decode(JsonRpcRequest.self, from: Data(line.utf8))
        } catch {
            FileHandle.standardError.write(Data("parse error: \(error)\n".utf8))
            emit(errorResponse(id: nil, code: -32700, message: "parse error"))
            continue
        }
        if request.method == "probe" || request.method == "llm.probe" {
            let result = try JSONValue.fromEncodable(ProbeResult(
                available: false,
                reason: "macos_26_required"
            ))
            emit(JsonRpcResponse(id: request.id, result: result))
        } else {
            emit(errorResponse(id: request.id, code: -32000, message: "FoundationModels unavailable"))
        }
    }
    exit(0)
}

let generator = Generator()
FileHandle.standardError.write(Data("panops-llm-mac ready\n".utf8))

while let line = readLine(strippingNewline: true) {
    if line.isEmpty { continue }
    let request: JsonRpcRequest
    do {
        request = try decoder.decode(JsonRpcRequest.self, from: Data(line.utf8))
    } catch {
        FileHandle.standardError.write(Data("parse error: \(error)\n".utf8))
        emit(errorResponse(id: nil, code: -32700, message: "parse error"))
        continue
    }

    do {
        switch request.method {
        case "probe", "llm.probe":
            let result = try JSONValue.fromEncodable(await generator.probe())
            emit(JsonRpcResponse(id: request.id, result: result))
        case "complete", "llm.complete":
            let params = try decodeParams(CompleteParams.self, from: request.params)
            let result = try JSONValue.fromEncodable(await generator.complete(params))
            emit(JsonRpcResponse(id: request.id, result: result))
        default:
            emit(errorResponse(id: request.id, code: -32601, message: "method not found"))
        }
    } catch let error as CodecError {
        FileHandle.standardError.write(Data("request failed: \(error)\n".utf8))
        emit(errorResponse(id: request.id, code: -32001, message: error.description))
    } catch {
        FileHandle.standardError.write(Data("request failed: \(error)\n".utf8))
        emit(errorResponse(id: request.id, code: -32000, message: "request failed"))
    }
}

FileHandle.standardError.write(Data("panops-llm-mac EOF; exiting\n".utf8))
