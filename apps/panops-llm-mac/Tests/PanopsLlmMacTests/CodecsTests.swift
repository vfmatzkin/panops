#if canImport(Testing)
import FoundationModels
import Testing
@testable import PanopsLlmMac

@available(macOS 26.0, *)
struct CodecsTests {
    @Test func objectSchemaAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["title", "count"],
          "properties": {
            "title": {"type": "string"},
            "count": {"type": "integer"}
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        #expect(!generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"title":"Daily notes","count":2}"#, equals: .object([
            "title": .string("Daily notes"),
            "count": .int(2),
        ]))
    }

    @Test func enumSchemaMapsToAnyOfAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["status"],
          "properties": {
            "status": {"type": "string", "enum": ["ok", "blocked"]}
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        #expect(!generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"status":"blocked"}"#, equals: .object([
            "status": .string("blocked"),
        ]))
    }

    @Test func nestedSchemaAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["action_items"],
          "properties": {
            "action_items": {
              "type": "array",
              "items": {
                "type": "object",
                "required": ["description"],
                "properties": {
                  "description": {"type": "string"},
                  "owner": {"type": ["string", "null"]}
                }
              }
            }
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        #expect(!generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"action_items":[{"description":"Send notes","owner":"Fran"}]}"#, equals: .object([
            "action_items": .array([
                .object([
                    "description": .string("Send notes"),
                    "owner": .string("Fran"),
                ]),
            ]),
        ]))
    }

    @Test func optionalPropertySchemaAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["title"],
          "properties": {
            "title": {"type": "string"},
            "summary": {"type": "string"}
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        #expect(!generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"title":"Anchor A"}"#, equals: .object([
            "title": .string("Anchor A"),
        ]))
    }

    private func json(_ raw: String) throws -> JSONValue {
        try JSONDecoder().decode(JSONValue.self, from: Data(raw.utf8))
    }

    private func assertGeneratedContent(_ rawJSON: String, equals expected: JSONValue) throws {
        let content = try GeneratedContent(json: rawJSON)
        let actual = try FoundationModelCodecs.jsonValue(from: content)
        #expect(actual == expected)
    }
}
#elseif canImport(XCTest)
import XCTest
import FoundationModels
@testable import PanopsLlmMac

@available(macOS 26.0, *)
final class CodecsTests: XCTestCase {
    func testObjectSchemaAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["title", "count"],
          "properties": {
            "title": {"type": "string"},
            "count": {"type": "integer"}
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        XCTAssertFalse(generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"title":"Daily notes","count":2}"#, equals: .object([
            "title": .string("Daily notes"),
            "count": .int(2),
        ]))
    }

    func testEnumSchemaMapsToAnyOfAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["status"],
          "properties": {
            "status": {"type": "string", "enum": ["ok", "blocked"]}
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        XCTAssertFalse(generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"status":"blocked"}"#, equals: .object([
            "status": .string("blocked"),
        ]))
    }

    func testNestedSchemaAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["action_items"],
          "properties": {
            "action_items": {
              "type": "array",
              "items": {
                "type": "object",
                "required": ["description"],
                "properties": {
                  "description": {"type": "string"},
                  "owner": {"type": ["string", "null"]}
                }
              }
            }
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        XCTAssertFalse(generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"action_items":[{"description":"Send notes","owner":"Fran"}]}"#, equals: .object([
            "action_items": .array([
                .object([
                    "description": .string("Send notes"),
                    "owner": .string("Fran"),
                ]),
            ]),
        ]))
    }

    func testOptionalPropertySchemaAndGeneratedContentRoundTrip() throws {
        let schema = try json(#"""
        {
          "type": "object",
          "required": ["title"],
          "properties": {
            "title": {"type": "string"},
            "summary": {"type": "string"}
          }
        }
        """#)
        let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
        XCTAssertFalse(generationSchema.debugDescription.isEmpty)

        try assertGeneratedContent(#"{"title":"Anchor A"}"#, equals: .object([
            "title": .string("Anchor A"),
        ]))
    }

    private func json(_ raw: String) throws -> JSONValue {
        try JSONDecoder().decode(JSONValue.self, from: Data(raw.utf8))
    }

    private func assertGeneratedContent(_ rawJSON: String, equals expected: JSONValue, file: StaticString = #filePath, line: UInt = #line) throws {
        let content = try GeneratedContent(json: rawJSON)
        let actual = try FoundationModelCodecs.jsonValue(from: content)
        XCTAssertEqual(actual, expected, file: file, line: line)
    }
}
#else
import Foundation
import FoundationModels
@testable import PanopsLlmMac

// Command Line Tools on this host exposes FoundationModels but no XCTest or
// Swift Testing modules. Run the same codec checks through a module initializer
// so `swift test` is still a meaningful gate under this CLT-only toolchain.
@available(macOS 26.0, *)
private let _panopsCodecsSelfTest: Void = {
    do {
        try assertSchema(#"""
        {
          "type": "object",
          "required": ["title", "count"],
          "properties": {
            "title": {"type": "string"},
            "count": {"type": "integer"}
          }
        }
        """#)
        try assertGeneratedContent(#"{"title":"Daily notes","count":2}"#, equals: .object([
            "title": .string("Daily notes"),
            "count": .int(2),
        ]))

        try assertSchema(#"""
        {
          "type": "object",
          "required": ["status"],
          "properties": {
            "status": {"type": "string", "enum": ["ok", "blocked"]}
          }
        }
        """#)
        try assertGeneratedContent(#"{"status":"blocked"}"#, equals: .object([
            "status": .string("blocked"),
        ]))

        try assertSchema(#"""
        {
          "type": "object",
          "required": ["action_items"],
          "properties": {
            "action_items": {
              "type": "array",
              "items": {
                "type": "object",
                "required": ["description"],
                "properties": {
                  "description": {"type": "string"},
                  "owner": {"type": ["string", "null"]}
                }
              }
            }
          }
        }
        """#)
        try assertGeneratedContent(#"{"action_items":[{"description":"Send notes","owner":"Fran"}]}"#, equals: .object([
            "action_items": .array([
                .object([
                    "description": .string("Send notes"),
                    "owner": .string("Fran"),
                ]),
            ]),
        ]))

        try assertSchema(#"""
        {
          "type": "object",
          "required": ["title"],
          "properties": {
            "title": {"type": "string"},
            "summary": {"type": "string"}
          }
        }
        """#)
        try assertGeneratedContent(#"{"title":"Anchor A"}"#, equals: .object([
            "title": .string("Anchor A"),
        ]))
    } catch {
        fatalError("PanopsLlmMac codec self-test failed: \(error)")
    }
}()

@available(macOS 26.0, *)
private func assertSchema(_ raw: String) throws {
    let schema = try JSONDecoder().decode(JSONValue.self, from: Data(raw.utf8))
    let generationSchema = try FoundationModelCodecs.generationSchema(from: schema)
    precondition(!generationSchema.debugDescription.isEmpty)
}

@available(macOS 26.0, *)
private func assertGeneratedContent(_ rawJSON: String, equals expected: JSONValue) throws {
    let content = try GeneratedContent(json: rawJSON)
    let actual = try FoundationModelCodecs.jsonValue(from: content)
    precondition(actual == expected, "expected \(expected), got \(actual)")
}
#endif
