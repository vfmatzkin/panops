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
/// Loads image asynchronously and shows placeholder on error.
struct ThumbnailView: View {
    let url: URL

    var body: some View {
        AsyncImage(url: url) { phase in
            switch phase {
            case .success(let image):
                image
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: 120, height: 80)
                    .clipped()
                    .cornerRadius(4)
            case .failure:
                Rectangle()
                    .fill(Color.secondary.opacity(0.3))
                    .frame(width: 120, height: 80)
                    .cornerRadius(4)
                    .overlay(
                        Image(systemName: "photo")
                            .foregroundStyle(.secondary)
                    )
            case .empty:
                Rectangle()
                    .fill(Color.secondary.opacity(0.2))
                    .frame(width: 120, height: 80)
                    .cornerRadius(4)
            @unknown default:
                Rectangle()
                    .fill(Color.secondary.opacity(0.2))
                    .frame(width: 120, height: 80)
                    .cornerRadius(4)
            }
        }
        .help(url.lastPathComponent)
    }
}
