import Foundation
import FoundationModels

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
            let response = try await session.respond(
                to: params.user,
                schema: schema,
                includeSchemaInPrompt: true,
                options: options
            )
            let json = try FoundationModelCodecs.jsonValue(from: response.rawContent)
            return CompleteResult(json: json)
        }

        let response = try await session.respond(to: params.user, options: options)
        return CompleteResult(text: response.content)
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
