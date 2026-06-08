import CoreGraphics
import ScreenCaptureKit
import Foundation

/// A pure, testable structure that maps resolution and optional region to
/// output dimensions and source rectangle. Used to configure the
/// SCStreamConfiguration for accurate capture with optional scaling.
///
/// The struct is designed to be:
/// - Pure: no side effects, no dependency on ScreenCaptureKit types
/// - Testable: can be unit tested without the Capabilities framework
/// - Immutable: all fields are let constants
struct CapturePlan {
    /// Output width in pixels. `nil` = native (no scaling).
    let outputWidth: Int?
    /// Output height in pixels. `nil` = native.
    let outputHeight: Int?
    /// Source rectangle to capture from the display/window. `nil` = capture full source.
    let sourceRect: CGRect?

    /// Create a capture plan from resolution and optional region.
    ///
    /// - Parameters:
    ///   - width: Optional output width in pixels. `nil` = native.
    ///   - height: Optional output height in pixels. `nil` = native.
    ///   - region: Optional region to capture. If present, specifies a sub-rectangle
    ///             of the source. Can be either CaptureRect (from wire protocol) or CGRect.
    init(outputWidth: Int?, outputHeight: Int?, region: CGRect?) {
        self.outputWidth = outputWidth
        self.outputHeight = outputHeight
        self.sourceRect = region
    }

    /// Apply this plan to an SCStreamConfiguration, setting width, height,
    /// and sourceRect as appropriate.
    func apply(to config: SCStreamConfiguration) {
        if let w = outputWidth, let h = outputHeight {
            config.width = w
            config.height = h
            config.scalesToFit = true
        }
        if let rect = sourceRect {
            config.sourceRect = rect
            config.scalesToFit = true
        }
    }

    /// A simple region specification: x, y, width, height in pixels.
    struct CaptureRect {
        /// X coordinate of the region's origin.
        let x: Int
        /// Y coordinate of the region's origin.
        let y: Int
        /// Width of the region in pixels.
        let w: Int
        /// Height of the region in pixels.
        let h: Int

        /// Create a capture rect from UInt32 values (from the wire protocol).
        init(x: UInt32, y: UInt32, w: UInt32, h: UInt32) {
            self.x = Int(x)
            self.y = Int(y)
            self.w = Int(w)
            self.h = Int(h)
        }

        /// This rect as a `CGRect` in the source's native coordinate space —
        /// the shape `SCStreamConfiguration.sourceRect` expects.
        var cgRect: CGRect { CGRect(x: x, y: y, width: w, height: h) }
    }
}
