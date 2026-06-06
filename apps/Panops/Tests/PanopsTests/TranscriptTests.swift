import Foundation
import Testing
@testable import Panops

@Suite("Transcript decoding")
struct TranscriptTests {
    @Test("Transcript decodes engine-shaped JSON with numeric speaker ids")
    func transcriptDecodesEngineJsonWithSpeakerIds() throws {
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
              "language_detected": "en",
              "confidence": 0.92,
              "is_partial": false,
              "speaker_id": 0
            },
            {
              "start_ms": 3500,
              "end_ms": 5200,
              "text": "Segundo segmento cargado.",
              "language_detected": "es",
              "confidence": 0.87,
              "is_partial": false,
              "speaker_id": 1
            },
            {
              "start_ms": 5300,
              "end_ms": 6100,
              "text": "No diarization for this segment.",
              "language_detected": null,
              "confidence": 0.74,
              "is_partial": false,
              "speaker_id": null
            }
          ]
        }
        """#

        let transcript = try JSONDecoder().decode(Transcript.self, from: Data(json.utf8))

        #expect(transcript.schemaVersion == 2)
        #expect(transcript.model == "whisper-large-v3-turbo")
        #expect(transcript.audioDurationMs == 42_420)
        #expect(transcript.diarized)
        #expect(transcript.segments.count == 3)

        let diarizedSegment = transcript.segments[0]
        #expect(diarizedSegment.languageDetected == "en")
        #expect(diarizedSegment.confidence == 0.92)
        #expect(diarizedSegment.isPartial == false)
        #expect(diarizedSegment.speakerId == 0)
        #expect(diarizedSegment.speakerLabel == "Speaker 1")
        #expect(diarizedSegment.speakerLabel != "?")

        let secondSpeaker = transcript.segments[1]
        #expect(secondSpeaker.speakerId == 1)
        #expect(secondSpeaker.speakerLabel == "Speaker 2")

        let nullSpeakerSegment = transcript.segments[2]
        #expect(nullSpeakerSegment.languageDetected == nil)
        #expect(nullSpeakerSegment.speakerId == nil)
        #expect(nullSpeakerSegment.speakerLabel == nil)
    }
}
