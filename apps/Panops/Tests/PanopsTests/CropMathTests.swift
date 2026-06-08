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
}
