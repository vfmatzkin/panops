import SwiftUI

/// Speaker-grouped, readable transcript.
///
/// Consecutive segments from the same speaker collapse into one block with the
/// speaker label shown once, a block timestamp, and the spoken text. A search
/// field filters segments by text; a speaker menu filters by speaker (when the
/// transcript is diarized into more than one speaker).
struct TranscriptView: View {
    let transcript: Transcript?

    @State private var searchText = ""
    @State private var speakerFilter: String? = nil  // nil == All Speakers

    var body: some View {
        Group {
            if let t = transcript, !t.segments.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    controlBar(distinctSpeakers: distinctSpeakers(t.segments))
                    Divider()
                    transcriptBody(blocks: blocks(from: t.segments))
                }
            } else {
                placeholder
            }
        }
        // Switching meetings reuses this view's @State; clear the speaker filter
        // so a speaker from the previous meeting can't strand the new transcript
        // on "No matching segments".
        .onChange(of: transcript?.audioPath) { _, _ in
            speakerFilter = nil
        }
    }

    // MARK: - Controls

    @ViewBuilder
    private func controlBar(distinctSpeakers: [String]) -> some View {
        HStack(spacing: 12) {
            SearchField(placeholder: "Search transcript", text: $searchText)
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(Color.secondary.opacity(0.1))
                .clipShape(RoundedRectangle(cornerRadius: 7))

            if distinctSpeakers.count > 1 {
                Menu {
                    Button("All Speakers") { speakerFilter = nil }
                    Divider()
                    ForEach(distinctSpeakers, id: \.self) { speaker in
                        Button(speaker) { speakerFilter = speaker }
                    }
                } label: {
                    Label(speakerFilter ?? "All Speakers", systemImage: "person.2")
                        .lineLimit(1)
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
        }
        .padding(12)
    }

    @ViewBuilder
    private func transcriptBody(blocks: [SpeakerBlock]) -> some View {
        if blocks.isEmpty {
            VStack {
                Spacer()
                Text("No matching segments").foregroundStyle(.secondary)
                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(blocks) { block in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 8) {
                                if let label = block.speakerLabel {
                                    Text(label)
                                        .font(.subheadline.weight(.semibold))
                                        .foregroundStyle(Color.accentColor)
                                }
                                Text(timestamp(block.startMs))
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                            Text(block.text)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding(16)
            }
        }
    }

    // MARK: - Grouping / filtering

    private struct SpeakerBlock: Identifiable {
        let id: Int
        let speakerLabel: String?
        let startMs: UInt64
        let text: String
    }

    private func filteredSegments(_ segments: [TranscriptSegment]) -> [TranscriptSegment] {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return segments.filter { segment in
            if let filter = speakerFilter, segment.speakerLabel != filter { return false }
            if !query.isEmpty, !segment.text.lowercased().contains(query) { return false }
            return true
        }
    }

    private func blocks(from segments: [TranscriptSegment]) -> [SpeakerBlock] {
        let filtered = filteredSegments(segments)
        var result: [SpeakerBlock] = []
        var currentLabel: String??  // double optional: unset vs (nil speaker)
        var currentTexts: [String] = []
        var currentStart: UInt64 = 0
        var blockIndex = 0

        func flush() {
            guard let label = currentLabel, !currentTexts.isEmpty else { return }
            let text = currentTexts.joined(separator: " ").trimmingCharacters(in: .whitespaces)
            result.append(SpeakerBlock(id: blockIndex, speakerLabel: label, startMs: currentStart, text: text))
            blockIndex += 1
            currentTexts = []
        }

        for segment in filtered {
            if let existing = currentLabel, existing == segment.speakerLabel {
                currentTexts.append(segment.text)
            } else {
                flush()
                currentLabel = segment.speakerLabel
                currentStart = segment.startMs
                currentTexts = [segment.text]
            }
        }
        flush()
        return result
    }

    private func distinctSpeakers(_ segments: [TranscriptSegment]) -> [String] {
        var seen = Set<String>()
        var ordered: [String] = []
        for segment in segments {
            if let label = segment.speakerLabel, !seen.contains(label) {
                seen.insert(label)
                ordered.append(label)
            }
        }
        return ordered
    }

    private func timestamp(_ ms: UInt64) -> String {
        let totalSec = ms / 1000
        let minutes = totalSec / 60
        let seconds = totalSec % 60
        return "\(minutes):\(String(format: "%02d", seconds))"
    }

    private var placeholder: some View {
        VStack {
            Spacer()
            Text("No transcript").foregroundStyle(.secondary)
            Spacer()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
