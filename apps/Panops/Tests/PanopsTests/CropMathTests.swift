import CoreGraphics
import Foundation
import Testing
@testable import Panops

@Suite("CropMath")
struct CropMathTests {
    @Test("a preview-space crop scales to source-space points")
    func previewRectScalesToDisplayPoints() {
        // Preview is 640×360 showing a 1920×1080 source; crop at (64,36)-(320,180).
        let rect = cropRect(
            previewRect: CGRect(x: 64, y: 36, width: 320, height: 180),
            previewSize: CGSize(width: 640, height: 360),
            displaySize: CGSize(width: 1920, height: 1080)
        )
        #expect(rect == CaptureRect(x: 192, y: 108, w: 960, h: 540))
    }

    @Test("a negative origin clamps to zero")
    func negativeOriginClamps() {
        let rect = cropRect(
            previewRect: CGRect(x: -10, y: -5, width: 100, height: 50),
            previewSize: CGSize(width: 200, height: 100),
            displaySize: CGSize(width: 400, height: 200)
        )
        #expect(rect == CaptureRect(x: 0, y: 0, w: 200, h: 100))
    }

    @Test("a 1:1 preview maps through unchanged")
    func identityMapping() {
        let rect = cropRect(
            previewRect: CGRect(x: 10, y: 20, width: 30, height: 40),
            previewSize: CGSize(width: 100, height: 100),
            displaySize: CGSize(width: 100, height: 100)
        )
        #expect(rect == CaptureRect(x: 10, y: 20, w: 30, h: 40))
    }

    // MARK: - Aspect-fit (letterbox) rect

    @Test("a wide source letterboxes top/bottom in a square box")
    func aspectFitLetterboxes() {
        // 2:1 source in a 100×100 box → 100×50 video, 25pt bars top and bottom.
        let rect = aspectFitRect(sourceSize: CGSize(width: 200, height: 100),
                                 boxSize: CGSize(width: 100, height: 100))
        #expect(rect == CGRect(x: 0, y: 25, width: 100, height: 50))
    }

    @Test("a tall source pillarboxes left/right in a square box")
    func aspectFitPillarboxes() {
        // 1:2 source in a 100×100 box → 50×100 video, 25pt bars left and right.
        let rect = aspectFitRect(sourceSize: CGSize(width: 100, height: 200),
                                 boxSize: CGSize(width: 100, height: 100))
        #expect(rect == CGRect(x: 25, y: 0, width: 50, height: 100))
    }

    // MARK: - Letterbox-aware crop mapping

    @Test("a drag over the whole letterboxed video maps to the full source")
    func letterboxedFullDragMapsToFullSource() {
        // Box 100×100, 2:1 source → video at (0,25)-(100,75). A drag covering
        // exactly the video must map to the entire source, not be skewed by the
        // letterbox bars.
        let rect = cropRectLetterboxed(
            previewRect: CGRect(x: 0, y: 25, width: 100, height: 50),
            boxSize: CGSize(width: 100, height: 100),
            displaySize: CGSize(width: 200, height: 100)
        )
        #expect(rect == CaptureRect(x: 0, y: 0, w: 200, h: 100))
    }

    @Test("a drag over the left half of the video maps to the left half of source")
    func letterboxedLeftHalfMaps() {
        let rect = cropRectLetterboxed(
            previewRect: CGRect(x: 0, y: 25, width: 50, height: 50),
            boxSize: CGSize(width: 100, height: 100),
            displaySize: CGSize(width: 200, height: 100)
        )
        #expect(rect == CaptureRect(x: 0, y: 0, w: 100, h: 100))
    }

    @Test("a drag spilling into the letterbox bars clamps to the video area")
    func letterboxedDragClampsToVideo() {
        // Drag the whole box (into both bars) → clamps to the full source.
        let rect = cropRectLetterboxed(
            previewRect: CGRect(x: 0, y: 0, width: 100, height: 100),
            boxSize: CGSize(width: 100, height: 100),
            displaySize: CGSize(width: 200, height: 100)
        )
        #expect(rect == CaptureRect(x: 0, y: 0, w: 200, h: 100))
    }

    @Test("a pillarboxed top-left drag maps to the source top-left quarter")
    func pillarboxedQuarterMaps() {
        // Box 100×100, 1:2 source → video at (25,0)-(75,100).
        let rect = cropRectLetterboxed(
            previewRect: CGRect(x: 25, y: 0, width: 25, height: 50),
            boxSize: CGSize(width: 100, height: 100),
            displaySize: CGSize(width: 100, height: 200)
        )
        #expect(rect == CaptureRect(x: 0, y: 0, w: 50, h: 100))
    }
}
