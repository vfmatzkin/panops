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
