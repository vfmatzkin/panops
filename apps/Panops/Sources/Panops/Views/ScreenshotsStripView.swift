import SwiftUI

/// Presentational view rendering a horizontal strip of screenshot thumbnails.
/// LazyHGrid of images from [URL]; click opens via NSWorkspace.
/// Empty/nil → "No screenshots" placeholder.
struct ScreenshotsStripView: View {
    let urls: [URL]?

    var body: some View {
        if let urls, !urls.isEmpty {
            ScrollView(.horizontal) {
                LazyHGrid(rows: [GridItem(.fixed(80))], spacing: 8) {
                    ForEach(urls, id: \.self) { url in
                        ThumbnailView(url: url)
                            .onTapGesture { NSWorkspace.shared.open(url) }
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
            Text("No screenshots").foregroundStyle(.secondary)
            Spacer()
        }
    }
}

/// Thumbnail view for a screenshot URL.
/// Loads local file with NSImage (AsyncImage doesn't support file:// URLs).
struct ThumbnailView: View {
    let url: URL
    @State private var image: NSImage?

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: 120, height: 80)
                    .clipped()
                    .cornerRadius(4)
            } else {
                Rectangle()
                    .fill(Color.secondary.opacity(0.3))
                    .frame(width: 120, height: 80)
                    .cornerRadius(4)
                    .overlay(
                        Image(systemName: "photo")
                            .foregroundStyle(.secondary)
                    )
            }
        }
        .help(url.lastPathComponent)
        .task { await loadImage() }
    }

    private func loadImage() async {
        // Read bytes off the MainActor (sync disk read would hitch the UI per
        // thumbnail), then build NSImage on-main — NSImage isn't Sendable, so
        // only the Sendable Data crosses the actor boundary.
        let url = self.url
        let data = await Task.detached(priority: .utility) {
            try? Data(contentsOf: url)
        }.value
        image = data.flatMap { NSImage(data: $0) }
    }
}
