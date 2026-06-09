import Foundation
import ScreenCaptureKit

/// What an `SCStream` should capture: the whole display (default), one window
/// by its `SCWindow.windowID`, all windows of an app by bundle id, or a
/// rectangular region within a display. Parsed from the `capture_target` start
/// param.
enum CaptureTargetKind: Equatable {
    case display
    case window(UInt32)
    case app(String)
    case region(displayId: UInt32, x: UInt32, y: UInt32, w: UInt32, h: UInt32)

    /// Map the wire `capture_target` to a target. A missing param, an
    /// unrecognized kind, or a `"window"` kind without a `window_id` all fall
    /// back to full-display capture (the slice-11 default), so a malformed
    /// target degrades to the safe behavior instead of failing the session.
    init?(wire: CaptureTarget?) {
        switch wire?.kind {
        case "window":
            if let id = wire?.windowId { self = .window(id) } else { self = .display }
        case "app":
            if let bid = wire?.bundleId { self = .app(bid) } else { return nil }
        case "region":
            if let dw = wire?.w, let dh = wire?.h, let dx = wire?.x, let dy = wire?.y {
                let did = wire?.displayId ?? 0
                self = .region(displayId: did, x: dx, y: dy, w: dw, h: dh)
            } else {
                self = .display
            }
        default:
            self = .display
        }
    }

    /// The crop rectangle for a `.region` target, in native source coordinates;
    /// `nil` for whole-display / window / app targets. Lets `main.swift` thread
    /// the wire `x/y/w/h` into the recorder so the SCStream `sourceRect` records
    /// exactly the cropped rectangle.
    var regionRect: CapturePlan.CaptureRect? {
        guard case let .region(_, x, y, w, h) = self else { return nil }
        return CapturePlan.CaptureRect(x: x, y: y, w: w, h: h)
    }
}
