import Foundation
import Testing
@testable import Panops

@Suite("Transcript decoding")
struct TranscriptTests {
    @Test("Transcript decodes numeric schema_version from engine JSON")
    func transcriptDecodesNumericSchemaVersion() throws {
        let json = #"""
        {
          "schema_version": 2,
          "model": "whisper-large-v3-turbo",
          "audio_path": "/Users/fran/Library/Application Support/panops/meetings/meeting-123/audio.wav",
          "audio_duration_ms": 42420,
          "diarized": true,
          "segments": [
            {
              "start_ms": 1200,
              "end_ms": 3450,
              "text": "Hello from a real transcript.",
              "speaker": "SPEAKER_01"
            },
            {
              "start_ms": 3500,
              "end_ms": 5200,
              "text": "Second segment loaded.",
              "speaker": null
            }
          ]
        }
        """#

        let transcript = try JSONDecoder().decode(Transcript.self, from: Data(json.utf8))

        #expect(transcript.schemaVersion == 2)
        #expect(transcript.segments.count == 2)
        #expect(transcript.segments[0].text == "Hello from a real transcript.")
        #expect(transcript.segments[0].speaker == "SPEAKER_01")
        #expect(transcript.segments[1].text == "Second segment loaded.")
    }
}
