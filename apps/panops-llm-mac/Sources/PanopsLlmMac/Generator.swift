import Foundation
import FoundationModels

@available(macOS 26.0, *)
private let modelResponseTimeoutNanoseconds: UInt64 = 120 * 1_000_000_000

@available(macOS 26.0, *)
enum GeneratorError: Error, CustomStringConvertible {
    case responseTimedOut

    var description: String {
        switch self {
        case .responseTimedOut:
            return "model response timed out"
        }
    }
}

@available(macOS 26.0, *)
private func withModelResponseTimeout<T: Sendable>(
    _ operation: @escaping @Sendable () async throws -> T
) async throws -> T {
    try await withThrowingTaskGroup(of: T.self) { group in
        group.addTask {
            try await operation()
        }
        group.addTask {
            try await Task.sleep(nanoseconds: modelResponseTimeoutNanoseconds)
            throw GeneratorError.responseTimedOut
        }
        guard let result = try await group.next() else {
            throw GeneratorError.responseTimedOut
        }
        group.cancelAll()
        return result
    }
}

@available(macOS 26.0, *)
actor Generator {
    func probe() -> ProbeResult {
        switch SystemLanguageModel.default.availability {
        case .available:
            return ProbeResult(available: true, reason: nil)
        case .unavailable(let reason):
            return ProbeResult(available: false, reason: Self.describeUnavailableReason(reason))
        }
    }

    func complete(_ params: CompleteParams) async throws -> CompleteResult {
        let options = GenerationOptions(
            temperature: params.temperature,
            maximumResponseTokens: params.maxTokens
        )
        let session = LanguageModelSession(
            model: .default,
            tools: [],
            instructions: params.system
        )

        if let schemaValue = params.schema {
            let schema = try FoundationModelCodecs.generationSchema(from: schemaValue)
            let json = try await withModelResponseTimeout {
                let response = try await session.respond(
                    to: params.user,
                    schema: schema,
                    includeSchemaInPrompt: true,
                    options: options
                )
                return try FoundationModelCodecs.jsonValue(from: response.rawContent)
            }
            return CompleteResult(json: json)
        }

        let text = try await withModelResponseTimeout {
            let response = try await session.respond(to: params.user, options: options)
            return response.content
        }
        return CompleteResult(text: text)
    }

    private static func describeUnavailableReason(_ reason: SystemLanguageModel.Availability.UnavailableReason) -> String {
        switch reason {
        case .deviceNotEligible:
            return "device_not_eligible"
        case .appleIntelligenceNotEnabled:
            return "apple_intelligence_not_enabled"
        case .modelNotReady:
            return "model_not_ready"
        @unknown default:
            return "unknown"
        }
    }
}
