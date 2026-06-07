import Foundation
import Testing
@testable import PanopsCaptureMac

/// Covers the pure routing + format math: which tracks each `audio_sources`
/// mode opens, the Float32→Int16 resample quantization, and the WAV header.
/// The live SCStream/AVAudioConverter path is exercised by the manual Mac
/// smoke (no screen/mic/TCC in CI).
struct AudioRoutingTests {
    @Test func systemAndMicOpensBothTracks() {
        let plan = TrackPlan(audioSources: "system_and_mic")
        #expect(plan.wantsSystem)
        #expect(plan.wantsMic)
    }

    @Test func systemOnlyOpensOnlySystem() {
        let plan = TrackPlan(audioSources: "system_only")
        #expect(plan.wantsSystem)
        #expect(!plan.wantsMic)
    }

    @Test func micOnlyOpensOnlyMic() {
        let plan = TrackPlan(audioSources: "mic_only")
        #expect(!plan.wantsSystem)
        #expect(plan.wantsMic)
    }

    @Test func unknownSourcesDefaultsToBoth() {
        let plan = TrackPlan(audioSources: "garbage")
        #expect(plan.wantsSystem)
        #expect(plan.wantsMic)
    }

    // MARK: - Resample quantization (Float32 → Int16)

    @Test func floatToInt16BoundsAndSign() {
        #expect(floatToInt16([0.0]) == [0])
        #expect(floatToInt16([1.0]) == [32767])
        #expect(floatToInt16([-1.0]) == [-32768])
    }

    @Test func floatToInt16ClampsOutOfRange() {
        // Values beyond [-1, 1] clamp instead of wrapping.
        #expect(floatToInt16([2.0, -2.0, 10.0, -10.0]) == [32767, -32768, 32767, -32768])
    }

    @Test func floatToInt16MidScale() {
        // 0.5 * 32767 = 16383.5 → rounds to 16384; -0.5 → -16384.
        #expect(floatToInt16([0.5]) == [16384])
        #expect(floatToInt16([-0.5]) == [-16384])
    }

    @Test func floatToInt16EmptyIsEmpty() {
        #expect(floatToInt16([]) == [])
    }

    // MARK: - WAV writer

    @Test func wavWriterRoundTrips() throws {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("panops-test-\(UUID().uuidString).wav")
        defer { try? FileManager.default.removeItem(at: url) }

        let writer = try WavWriter(url: url)
        try writer.append([0, 16384, -16384, 32767, -32768])
        try writer.finalize()

        let data = try Data(contentsOf: url)
        #expect(data.count == 44 + 5 * 2)                  // 44-byte header + 5 samples
        #expect(Array(data[0..<4]) == Array("RIFF".utf8))
        #expect(Array(data[8..<12]) == Array("WAVE".utf8))
        #expect(Array(data[36..<40]) == Array("data".utf8))
    }

    @Test func wavHeaderDeclares16kMono16bit() {
        let header = WavWriter.header(dataBytes: 0)
        #expect(header.count == 44)
        // channels (offset 22, u16 LE) == 1
        #expect(header[22] == 1)
        #expect(header[23] == 0)
        // sample rate (offset 24, u32 LE) == 16000 == 0x3E80
        #expect(header[24] == 0x80)
        #expect(header[25] == 0x3E)
        // bits per sample (offset 34, u16 LE) == 16
        #expect(header[34] == 16)
    }
}
