import Foundation
import Testing
@testable import PanopsCaptureMac

/// Covers the pure window-targeting logic that runs without ScreenCaptureKit:
/// the `--list-windows` filter + JSON shape and `capture_target` decoding into a
/// `CaptureTargetKind`. The live `SCShareableContent`/`SCContentFilter` path is
/// exercised by the manual Mac smoke (no screen/TCC in CI).
struct WindowTargetingTests {
    // MARK: - --list-windows filter + shape

    @Test func shareableWindowsMapsAndDropsNonShareable() {
        let raws = [
            RawWindow(windowID: 1, appName: "Zoom", bundleID: "us.zoom.xos",
                      title: "Standup", isEmptyFrame: false),                      // kept
            RawWindow(windowID: 2, appName: "Finder", bundleID: "com.apple.finder",
                      title: "", isEmptyFrame: false),                             // untitled → drop
            RawWindow(windowID: 3, appName: "Safari", bundleID: "com.apple.Safari",
                      title: "News", isEmptyFrame: true),                          // zero frame → drop
            RawWindow(windowID: 4, appName: "panops", bundleID: "ar.tzk.panops.capture-mac",
                      title: "panops", isEmptyFrame: false),                       // own window → drop
        ]
        #expect(shareableWindows(raws) == [WindowInfo(windowId: 1, appName: "Zoom", title: "Standup")])
    }

    @Test func emptyAppNameKeptWhenTitled() {
        // app_name = applicationName ?? "" — an empty app_name is valid output.
        let raws = [RawWindow(windowID: 9, appName: nil, bundleID: nil,
                              title: "Untitled doc", isEmptyFrame: false)]
        #expect(shareableWindows(raws) == [WindowInfo(windowId: 9, appName: "", title: "Untitled doc")])
    }

    @Test func isPanopsOwnedByBundlePrefixOrName() {
        #expect(isPanopsOwned(bundleID: "ar.tzk.panops", appName: ""))
        #expect(isPanopsOwned(bundleID: "ar.tzk.panops.capture-mac", appName: "x"))
        #expect(isPanopsOwned(bundleID: nil, appName: "Panops"))          // name match, any case
        #expect(!isPanopsOwned(bundleID: "us.zoom.xos", appName: "Zoom"))
    }

    @Test func windowInfoEncodesWireShape() throws {
        let enc = JSONEncoder()
        enc.outputFormatting = [.sortedKeys]
        let data = try enc.encode(WindowInfo(windowId: 42, appName: "Zoom", title: "Meeting"))
        let json = String(data: data, encoding: .utf8)!
        #expect(json == #"{"app_name":"Zoom","title":"Meeting","window_id":42}"#)
    }

    // MARK: - capture_target parsing

    @Test func captureParamsDecodesWindowTarget() throws {
        let json = #"{"meeting_id":"m1","capture_target":{"kind":"window","window_id":7}}"#
        let params = try JSONDecoder().decode(CaptureParams.self, from: Data(json.utf8))
        #expect(params.captureTarget?.kind == "window")
        #expect(params.captureTarget?.windowId == 7)
        #expect(CaptureTargetKind(wire: params.captureTarget) == .window(7))
    }

    @Test func captureParamsWithoutTargetDefaultsToDisplay() throws {
        let json = #"{"meeting_id":"m1"}"#
        let params = try JSONDecoder().decode(CaptureParams.self, from: Data(json.utf8))
        #expect(params.captureTarget == nil)
        #expect(CaptureTargetKind(wire: params.captureTarget) == .display)
    }

    @Test func displayKindMapsToDisplay() throws {
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(#"{"kind":"display"}"#.utf8))
        #expect(CaptureTargetKind(wire: target) == .display)
    }

    @Test func windowKindWithoutIdFallsBackToDisplay() throws {
        // Malformed window target (no window_id) returns nil and falls back to display
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(#"{"kind":"window"}"#.utf8))
        #expect(CaptureTargetKind(wire: target) == .display)
    }

    @Test func unknownKindFallsBackToDisplay() throws {
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(#"{"kind":"bogus"}"#.utf8))
        #expect(CaptureTargetKind(wire: target) == .display)
    }
}
