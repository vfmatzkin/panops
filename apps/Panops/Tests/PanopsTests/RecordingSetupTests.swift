import Foundation
import Testing
@testable import Panops

@Suite("RecordingLanguage")
struct RecordingLanguageTests {
    @Test("auto omits the language hint; others map to BCP-47")
    func wireValue() {
        #expect(RecordingLanguage.auto.wireValue == nil)
        #expect(RecordingLanguage.english.wireValue == "en")
        #expect(RecordingLanguage.spanish.wireValue == "es")
    }
}

@Suite("RecordingSetup mapping")
struct RecordingSetupTests {
    @Test("empty title and auto language collapse to omitted config fields")
    func defaultsCollapse() {
        let config = RecordingSetup().meetingConfig
        #expect(config.title == nil)
        #expect(config.language == nil)
    }

    @Test("whitespace-only title collapses to nil; language still maps")
    func whitespaceTitle() {
        let setup = RecordingSetup(title: "   ", language: .english)
        #expect(setup.meetingConfig.title == nil)
        #expect(setup.meetingConfig.language == "en")
    }

    @Test("title is trimmed and language mapped")
    func titleTrimmedLanguageMapped() {
        let setup = RecordingSetup(title: "  Standup  ", language: .spanish)
        #expect(setup.meetingConfig.title == "Standup")
        #expect(setup.meetingConfig.language == "es")
    }

    @Test("recordingOptions carries the chosen audio source at engine defaults")
    func recordingOptionsAudio() {
        let options = RecordingSetup(audioSources: .systemOnly).recordingOptions
        #expect(options.audioSources == .systemOnly)
        #expect(options.screenshotIntervalMs == 500)
        #expect(options.screenshotThreshold == 0.15)
    }

    @Test("recordingOptions carries recordVideo flag")
    func recordingOptionsRecordVideo() {
        let setup = RecordingSetup(recordVideo: true)
        #expect(setup.recordingOptions.recordVideo == true)

        let setupFalse = RecordingSetup(recordVideo: false)
        #expect(setupFalse.recordingOptions.recordVideo == false)
    }

    @Test("recordVideo defaults to false")
    func recordVideoDefaults() {
        let setup = RecordingSetup()
        #expect(setup.recordVideo == false)
        #expect(setup.recordingOptions.recordVideo == false)
    }
}

@Suite("RecordingStartParams encoding")
struct RecordingStartParamsEncodingTests {
    private var encoder: JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return e
    }

    @Test("params encode with snake_case keys including record_video")
    func encoding() throws {
        let params = RecordingStartParams(
            meetingId: "m1",
            audioSources: .systemAndMic,
            screenshotIntervalMs: 500,
            screenshotThreshold: 0.25,
            recordVideo: true
        )
        let data = try encoder.encode(params)
        let json = String(data: data, encoding: .utf8)!
        #expect(json.contains("\"meeting_id\":\"m1\""))
        #expect(json.contains("\"audio_sources\":\"system_and_mic\""))
        #expect(json.contains("\"screenshot_interval_ms\":500"))
        #expect(json.contains("\"screenshot_threshold\":0.25"))
        #expect(json.contains("\"record_video\":true"))
    }
}

@Suite("MeetingConfig encoding")
struct MeetingConfigEncodingTests {
    private var encoder: JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return e
    }

    @Test("all-nil config encodes to an empty object")
    func emptyEncoding() throws {
        let data = try encoder.encode(MeetingConfig(title: nil, language: nil))
        #expect(String(data: data, encoding: .utf8) == "{}")
    }

    @Test("set fields are encoded with snake_case-compatible keys")
    func fullEncoding() throws {
        let data = try encoder.encode(MeetingConfig(title: "Standup", language: "es"))
        #expect(String(data: data, encoding: .utf8) == #"{"language":"es","title":"Standup"}"#)
    }
}

@Suite("RecordingClock")
struct RecordingClockTests {
    @Test("formats MM:SS under an hour")
    func underAnHour() {
        #expect(RecordingClock.label(seconds: 0) == "00:00")
        #expect(RecordingClock.label(seconds: 5) == "00:05")
        #expect(RecordingClock.label(seconds: 65) == "01:05")
        #expect(RecordingClock.label(seconds: 3599) == "59:59")
    }

    @Test("rolls into H:MM:SS at and beyond one hour")
    func atAndBeyondAnHour() {
        #expect(RecordingClock.label(seconds: 3600) == "1:00:00")
        #expect(RecordingClock.label(seconds: 3661) == "1:01:01")
        #expect(RecordingClock.label(seconds: 36000) == "10:00:00")
    }

    @Test("negative input clamps to zero")
    func negativeClamps() {
        #expect(RecordingClock.label(seconds: -10) == "00:00")
    }
}
