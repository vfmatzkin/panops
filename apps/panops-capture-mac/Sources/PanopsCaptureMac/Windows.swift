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

/// One on-screen window in the wire shape the engine's `--list-windows`
/// consumer binds to: `{"window_id":<u32>,"app_name":"<string>","title":"<string>"}`.
struct WindowInfo: Encodable, Equatable {
    let windowId: UInt32
    let appName: String
    let title: String

    enum CodingKeys: String, CodingKey {
        case windowId = "window_id"
        case appName = "app_name"
        case title = "title"
    }
}

/// Raw `SCWindow` properties in a ScreenCaptureKit-free form so the filter +
/// shape logic is unit-testable without a live display.
struct RawWindow {
    let windowID: UInt32
    let appName: String?
    let bundleID: String?
    let title: String?
    let isEmptyFrame: Bool
}

/// True when a window belongs to panops itself (the app or this sidecar) — by
/// bundle-id prefix or app name — so we never offer panops's own UI as a
/// capture target.
func isPanopsOwned(bundleID: String?, appName: String) -> Bool {
    if let bundleID, bundleID.hasPrefix("ar.tzk.panops") { return true }
    return appName.lowercased().contains("panops")
}

/// Filter raw windows to shareable targets and map to the wire shape: drop
/// zero-frame windows, untitled windows, and panops's own windows. An empty
/// `app_name` is kept (the contract allows it) as long as the window is titled.
func shareableWindows(_ raws: [RawWindow]) -> [WindowInfo] {
    raws.compactMap { raw in
        guard !raw.isEmptyFrame else { return nil }
        let title = raw.title ?? ""
        guard !title.isEmpty else { return nil }
        let appName = raw.appName ?? ""
        guard !isPanopsOwned(bundleID: raw.bundleID, appName: appName) else { return nil }
        return WindowInfo(windowId: raw.windowID, appName: appName, title: title)
    }
}

/// Enumerate on-screen, non-desktop windows as filtered wire-shape entries.
/// Throws if ScreenCaptureKit enumeration fails (e.g. Screen-Recording denied).
func listShareableWindows() async throws -> [WindowInfo] {
    let content = try await SCShareableContent.excludingDesktopWindows(
        true, onScreenWindowsOnly: true
    )
    let raws = content.windows.map { window in
        RawWindow(
            windowID: UInt32(window.windowID),
            appName: window.owningApplication?.applicationName,
            bundleID: window.owningApplication?.bundleIdentifier,
            title: window.title,
            isEmptyFrame: window.frame.isEmpty   // CGRect.isEmpty: width or height <= 0
        )
    }
    return shareableWindows(raws)
}
