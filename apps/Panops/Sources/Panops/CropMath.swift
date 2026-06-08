import CoreGraphics

/// A capture sub-rectangle in display points, matching the wire `region`
/// fields. Produced by the drag-crop overlay and sent as a `region` target.
struct CaptureRect: Equatable {
    let x: UInt32
    let y: UInt32
    let w: UInt32
    let h: UInt32
}

/// Map a crop rectangle drawn in preview space to a `CaptureRect` in the source's
/// own coordinate space, by scaling each axis independently by
/// `displaySize / previewSize`. Negative origins clamp to zero; values round to
/// the nearest pixel.
///
/// `previewRect`/`previewSize` are in the same space (the on-screen preview);
/// `displaySize` is the source size the rect maps onto (display points).
func cropRect(previewRect: CGRect, previewSize: CGSize, displaySize: CGSize) -> CaptureRect {
    let scaleX = displaySize.width / max(previewSize.width, 1)
    let scaleY = displaySize.height / max(previewSize.height, 1)

    func clampedPixel(_ value: CGFloat) -> UInt32 {
        UInt32(max(0, value.rounded()))
    }

    return CaptureRect(
        x: clampedPixel(previewRect.origin.x * scaleX),
        y: clampedPixel(previewRect.origin.y * scaleY),
        w: clampedPixel(previewRect.width * scaleX),
        h: clampedPixel(previewRect.height * scaleY)
    )
}

/// The rectangle a `sourceSize` source occupies, centered, when displayed with
/// `.resizeAspect` (aspect-fit) inside a `boxSize` box. The video fills one axis
/// and is letterboxed (bars top/bottom) or pillarboxed (bars left/right) on the
/// other. Degenerate sizes fall back to the full box.
func aspectFitRect(sourceSize: CGSize, boxSize: CGSize) -> CGRect {
    guard sourceSize.width > 0, sourceSize.height > 0,
          boxSize.width > 0, boxSize.height > 0 else {
        return CGRect(origin: .zero, size: boxSize)
    }
    let scale = min(boxSize.width / sourceSize.width, boxSize.height / sourceSize.height)
    let w = sourceSize.width * scale
    let h = sourceSize.height * scale
    return CGRect(x: (boxSize.width - w) / 2, y: (boxSize.height - h) / 2, width: w, height: h)
}

/// Map a crop rectangle drawn in the preview *box* to a `CaptureRect`, correcting
/// for aspect-fit letterboxing: the video only occupies `aspectFitRect` inside
/// the box, so the drag is clamped to that sub-rect and re-based onto its origin
/// before scaling to the source. Without this the mapping is off by the
/// letterbox margin.
func cropRectLetterboxed(previewRect: CGRect, boxSize: CGSize, displaySize: CGSize) -> CaptureRect {
    let videoRect = aspectFitRect(sourceSize: displaySize, boxSize: boxSize)
    // Clamp the drag to the displayed video area; a drag entirely in the bars
    // yields an empty crop.
    let clamped = previewRect.intersection(videoRect)
    guard !clamped.isNull, !clamped.isEmpty else {
        return CaptureRect(x: 0, y: 0, w: 0, h: 0)
    }
    let rebased = CGRect(
        x: clamped.origin.x - videoRect.origin.x,
        y: clamped.origin.y - videoRect.origin.y,
        width: clamped.width,
        height: clamped.height
    )
    return cropRect(previewRect: rebased, previewSize: videoRect.size, displaySize: displaySize)
}
