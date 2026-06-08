import SwiftUI

/// Drag-to-crop overlay drawn over a live display preview. While uncropped, a
/// drag rubber-bands a rectangle; on release it becomes the `region` target and
/// the preview reframes to it. Once cropped, a "Reset crop" control returns to
/// the full display. Shown only for display targets.
struct CropOverlay: View {
    @ObservedObject var controller: CapturePreviewController

    @State private var dragStart: CGPoint?
    @State private var dragCurrent: CGPoint?

    var body: some View {
        GeometryReader { geo in
            if controller.isCropped {
                resetControl
            } else {
                dragLayer(viewSize: geo.size)
            }
        }
    }

    private func dragLayer(viewSize: CGSize) -> some View {
        Rectangle()
            // Nearly-invisible but hit-testable, so the drag is captured.
            .fill(Color.white.opacity(0.001))
            .overlay(selectionBox)
            .gesture(dragGesture(viewSize: viewSize))
    }

    @ViewBuilder
    private var selectionBox: some View {
        if let start = dragStart, let current = dragCurrent {
            let rect = Self.normalizedRect(start, current)
            Rectangle()
                .strokeBorder(Color.accentColor, lineWidth: 2)
                .background(Color.accentColor.opacity(0.15))
                .frame(width: rect.width, height: rect.height)
                .position(x: rect.midX, y: rect.midY)
        }
    }

    private var resetControl: some View {
        VStack {
            HStack {
                Spacer()
                Button {
                    controller.clearCrop()
                } label: {
                    Label("Reset crop", systemImage: "crop")
                }
                .controlSize(.small)
                .padding(8)
            }
            Spacer()
        }
    }

    private func dragGesture(viewSize: CGSize) -> some Gesture {
        DragGesture(minimumDistance: 4)
            .onChanged { value in
                if dragStart == nil { dragStart = value.startLocation }
                dragCurrent = value.location
            }
            .onEnded { value in
                defer { dragStart = nil; dragCurrent = nil }
                guard let start = dragStart else { return }
                let rect = Self.normalizedRect(start, value.location)
                // Ignore stray taps / tiny rectangles.
                guard rect.width > 8, rect.height > 8 else { return }
                // The preview uses `.resizeAspect`, so the video occupies only a
                // letterboxed sub-rect of `viewSize`; map against that, not the box.
                let crop = cropRectLetterboxed(
                    previewRect: rect,
                    boxSize: viewSize,
                    displaySize: controller.sourceContentSize
                )
                controller.applyCrop(crop)
            }
    }

    /// A normalized (non-negative size) rect from two corner points.
    private static func normalizedRect(_ a: CGPoint, _ b: CGPoint) -> CGRect {
        CGRect(
            x: min(a.x, b.x),
            y: min(a.y, b.y),
            width: abs(a.x - b.x),
            height: abs(a.y - b.y)
        )
    }
}
