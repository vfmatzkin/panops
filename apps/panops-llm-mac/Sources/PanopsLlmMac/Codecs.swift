import Foundation
import FoundationModels

/// JSON value carrier used for both raw JSON-RPC params and structured
/// FoundationModels output. It intentionally avoids `Any` so codecs remain
/// deterministic and testable without involving a live model.
enum JSONValue: Codable, Equatable, Sendable {
    case object([String: JSONValue])
    case array([JSONValue])
    case string(String)
    case int(Int)
    case double(Double)
    case bool(Bool)
    case null

    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() {
            self = .null
        } else if let v = try? c.decode(Bool.self) {
            self = .bool(v)
        } else if let v = try? c.decode(Int.self) {
            self = .int(v)
        } else if let v = try? c.decode(Double.self) {
            self = .double(v)
        } else if let v = try? c.decode(String.self) {
            self = .string(v)
        } else if let v = try? c.decode([JSONValue].self) {
            self = .array(v)
        } else if let v = try? c.decode([String: JSONValue].self) {
            self = .object(v)
        } else {
            throw DecodingError.dataCorruptedError(in: c, debugDescription: "unsupported JSON value")
        }
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .object(let v): try c.encode(v)
        case .array(let v): try c.encode(v)
        case .string(let v): try c.encode(v)
        case .int(let v): try c.encode(v)
        case .double(let v): try c.encode(v)
        case .bool(let v): try c.encode(v)
        case .null: try c.encodeNil()
        }
    }

    var objectValue: [String: JSONValue]? {
        if case .object(let v) = self { return v }
        return nil
    }

    var arrayValue: [JSONValue]? {
        if case .array(let v) = self { return v }
        return nil
    }

    var stringValue: String? {
        if case .string(let v) = self { return v }
        return nil
    }

    static func decode<T: Decodable>(_ type: T.Type, from value: JSONValue) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }

    static func fromEncodable<T: Encodable>(_ value: T) throws -> JSONValue {
        try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
    }
}

enum CodecError: Error, CustomStringConvertible, Equatable {
    case invalidSchema(String)
    case unsupportedSchema(String)
    case invalidGeneratedJSON(String)

    var description: String {
        switch self {
        case .invalidSchema(let message): "invalid schema: \(message)"
        case .unsupportedSchema(let message): "unsupported schema: \(message)"
        case .invalidGeneratedJSON(let message): "invalid generated JSON: \(message)"
        }
    }
}

struct JsonRpcRequest: Decodable, Sendable {
    let jsonrpc: String
    let id: JSONValue?
    let method: String
    let params: JSONValue?
}

struct JsonRpcResponse: Encodable {
    let jsonrpc: String
    let id: JSONValue?
    let result: JSONValue?
    let error: JsonRpcError?

    init(id: JSONValue?, result: JSONValue? = nil, error: JsonRpcError? = nil) {
        self.jsonrpc = "2.0"
        self.id = id
        self.result = result
        self.error = error
    }

    enum CodingKeys: String, CodingKey { case jsonrpc, id, result, error }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(jsonrpc, forKey: .jsonrpc)
        if let id { try c.encode(id, forKey: .id) } else { try c.encodeNil(forKey: .id) }
        try c.encodeIfPresent(result, forKey: .result)
        try c.encodeIfPresent(error, forKey: .error)
    }
}

struct JsonRpcError: Encodable {
    let code: Int
    let message: String
}

struct ProbeResult: Codable, Equatable, Sendable {
    let available: Bool
    let reason: String?
}

struct CompleteParams: Decodable, Equatable, Sendable {
    let system: String?
    let user: String
    let schema: JSONValue?
    let temperature: Double
    let maxTokens: Int

    enum CodingKeys: String, CodingKey {
        case system, user, schema, temperature
        case maxTokens = "max_tokens"
    }
}

struct CompleteResult: Codable, Equatable, Sendable {
    let json: JSONValue?
    let text: String?

    init(json: JSONValue) {
        self.json = json
        self.text = nil
    }

    init(text: String) {
        self.json = nil
        self.text = text
    }
}

@available(macOS 26.0, *)
enum FoundationModelCodecs {
    static func generationSchema(from schema: JSONValue, rootName: String = "PanopsLlmResponse") throws -> GenerationSchema {
        let root = try dynamicGenerationSchema(from: schema, name: sanitizedTypeName(rootName))
        return try GenerationSchema(root: root, dependencies: [])
    }

    static func jsonValue(from content: GeneratedContent) throws -> JSONValue {
        guard let data = content.jsonString.data(using: .utf8) else {
            throw CodecError.invalidGeneratedJSON("GeneratedContent.jsonString was not UTF-8")
        }
        do {
            return try JSONDecoder().decode(JSONValue.self, from: data)
        } catch {
            throw CodecError.invalidGeneratedJSON(error.localizedDescription)
        }
    }

    private static func dynamicGenerationSchema(from raw: JSONValue, name: String) throws -> DynamicGenerationSchema {
        guard let schema = raw.objectValue else {
            throw CodecError.invalidSchema("schema root for \(name) must be an object")
        }

        if let enumValues = schema["enum"]?.arrayValue {
            let choices = try enumValues.map { value -> String in
                guard let s = value.stringValue else {
                    throw CodecError.unsupportedSchema("enum for \(name) must contain only strings")
                }
                return s
            }
            guard !choices.isEmpty else {
                throw CodecError.invalidSchema("enum for \(name) must not be empty")
            }
            return DynamicGenerationSchema(name: name, anyOf: choices)
        }

        if let anyOf = schema["anyOf"]?.arrayValue ?? schema["oneOf"]?.arrayValue {
            let nonNull = anyOf.filter { !isNullSchema($0) }
            guard !nonNull.isEmpty else { return try nullSchema(name: name) }
            if nonNull.count == 1 {
                return try dynamicGenerationSchema(from: nonNull[0], name: name)
            }
            let choices = try nonNull.enumerated().map { index, choice in
                try dynamicGenerationSchema(from: choice, name: "\(name)Choice\(index + 1)")
            }
            return DynamicGenerationSchema(name: name, anyOf: choices)
        }

        let types = try typeNames(in: schema, name: name)
        let nonNullTypes = types.filter { $0 != "null" }
        if nonNullTypes.isEmpty { return try nullSchema(name: name) }
        if nonNullTypes.count > 1 {
            let choices = try nonNullTypes.map { typeName in
                var narrowed = schema
                narrowed["type"] = .string(typeName)
                return try dynamicGenerationSchema(from: .object(narrowed), name: "\(name)\(sanitizedTypeName(typeName))")
            }
            return DynamicGenerationSchema(name: name, anyOf: choices)
        }

        switch nonNullTypes[0] {
        case "object":
            let properties = schema["properties"]?.objectValue ?? [:]
            let required = Set((schema["required"]?.arrayValue ?? []).compactMap(\.stringValue))
            let dynamicProperties = try properties.keys.sorted().map { propertyName in
                guard let propertySchema = properties[propertyName] else {
                    throw CodecError.invalidSchema("missing schema for property \(propertyName)")
                }
                let stripped = stripNullable(propertySchema)
                return DynamicGenerationSchema.Property(
                    name: propertyName,
                    description: description(in: stripped),
                    schema: try dynamicGenerationSchema(
                        from: stripped,
                        name: sanitizedTypeName("\(name)_\(propertyName)")
                    ),
                    isOptional: !required.contains(propertyName)
                )
            }
            return DynamicGenerationSchema(
                name: name,
                description: description(in: raw),
                properties: dynamicProperties
            )
        case "array":
            guard let items = schema["items"] else {
                throw CodecError.invalidSchema("array schema \(name) is missing items")
            }
            return DynamicGenerationSchema(
                arrayOf: try dynamicGenerationSchema(from: stripNullable(items), name: "\(name)Item"),
                minimumElements: intValue(schema["minItems"]),
                maximumElements: intValue(schema["maxItems"])
            )
        case "string":
            return DynamicGenerationSchema(type: String.self)
        case "integer":
            return DynamicGenerationSchema(type: Int.self)
        case "number":
            return DynamicGenerationSchema(type: Double.self)
        case "boolean":
            return DynamicGenerationSchema(type: Bool.self)
        default:
            throw CodecError.unsupportedSchema("type \(nonNullTypes[0]) for \(name)")
        }
    }

    private static func typeNames(in schema: [String: JSONValue], name: String) throws -> [String] {
        guard let typeValue = schema["type"] else {
            if schema["properties"] != nil { return ["object"] }
            if schema["items"] != nil { return ["array"] }
            if schema["enum"] != nil { return ["string"] }
            throw CodecError.invalidSchema("schema \(name) is missing type")
        }
        if let s = typeValue.stringValue { return [s] }
        if let a = typeValue.arrayValue {
            let names = a.compactMap(\.stringValue)
            guard names.count == a.count else {
                throw CodecError.invalidSchema("type array for \(name) must contain only strings")
            }
            return names
        }
        throw CodecError.invalidSchema("type for \(name) must be a string or string array")
    }

    private static func stripNullable(_ value: JSONValue) -> JSONValue {
        guard var schema = value.objectValue else { return value }
        if let types = schema["type"]?.arrayValue {
            let nonNull = types.filter { $0 != .string("null") }
            if nonNull.count == 1, let type = nonNull[0].stringValue {
                schema["type"] = .string(type)
            } else {
                schema["type"] = .array(nonNull)
            }
        }
        if let anyOf = schema["anyOf"]?.arrayValue {
            schema["anyOf"] = .array(anyOf.filter { !isNullSchema($0) })
        }
        if let oneOf = schema["oneOf"]?.arrayValue {
            schema["oneOf"] = .array(oneOf.filter { !isNullSchema($0) })
        }
        return .object(schema)
    }

    private static func isNullSchema(_ value: JSONValue) -> Bool {
        guard let schema = value.objectValue else { return value == .null }
        if schema["type"] == .string("null") { return true }
        return false
    }

    private static func nullSchema(name: String) throws -> DynamicGenerationSchema {
        if #available(macOS 26.4, *) {
            return .null
        }
        throw CodecError.unsupportedSchema("null schema for \(name) requires macOS 26.4")
    }

    private static func description(in value: JSONValue) -> String? {
        value.objectValue?["description"]?.stringValue
    }

    private static func intValue(_ value: JSONValue?) -> Int? {
        switch value {
        case .int(let i): i
        case .double(let d): Int(d)
        default: nil
        }
    }

    private static func sanitizedTypeName(_ raw: String) -> String {
        let parts = raw.split { !$0.isLetter && !$0.isNumber }
        let joined = parts.map { part -> String in
            guard let first = part.first else { return "" }
            return String(first).uppercased() + String(part.dropFirst())
        }.joined()
        let candidate = joined.isEmpty ? "PanopsSchema" : joined
        if candidate.first?.isNumber == true { return "Panops\(candidate)" }
        return candidate
    }
}
