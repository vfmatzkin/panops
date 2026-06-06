import SwiftUI

/// Presentational view rendering transcript segments.
/// Shows `[MM:SS–MM:SS] Speaker X: text` per segment.
/// Empty/nil → "No transcript" placeholder.
struct TranscriptView: View {
    let transcript: Transcript?

    var body: some View {
        if let t = transcript, !t.segments.isEmpty {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 8) {
                    ForEach(t.segments, id: \.self) { seg in
                        HStack(alignment: .top, spacing: 8) {
                            Text(seg.timestampRange)
                                .font(.system(.body, design: .monospaced))
                                .foregroundStyle(.secondary)
                            Text(seg.speakerLabel + ":")
                                .fontWeight(.medium)
                            Text(seg.text)
                                .textSelection(.enabled)
                        }
                    }
                }
                .padding()
            }
        } else {
            placeholder
        }
    }

    private var placeholder: some View {
        VStack {
            Spacer()
            Text("No transcript").foregroundStyle(.secondary)
            Spacer()
        }
    }
}

