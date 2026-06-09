import Foundation
import Testing
@testable import PanopsCaptureMac

/// Covers the pure window-targeting logic that runs without ScreenCaptureKit:
/// `capture_target` decoding into a `CaptureTargetKind`. The live
/// `SCShareableContent`/`SCContentFilter` path is exercised by the manual Mac
/// smoke (no screen/TCC in CI).
struct WindowTargetingTests {
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

    // MARK: - region → sourceRect threading

    @Test func regionKindDecodesAllFields() throws {
        let json = #"{"kind":"region","display_id":0,"x":10,"y":20,"w":640,"h":480}"#
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(json.utf8))
        #expect(CaptureTargetKind(wire: target) == .region(displayId: 0, x: 10, y: 20, w: 640, h: 480))
    }

    @Test func regionTargetExposesItsCropRect() {
        // A region target threads its x/y/w/h to the recorder so the SCStream
        // sourceRect records exactly the cropped rectangle.
        let rect = CaptureTargetKind.region(displayId: 0, x: 10, y: 20, w: 640, h: 480).regionRect
        #expect(rect?.x == 10)
        #expect(rect?.y == 20)
        #expect(rect?.w == 640)
        #expect(rect?.h == 480)
    }

    @Test func nonRegionTargetsHaveNoCropRect() {
        #expect(CaptureTargetKind.display.regionRect == nil)
        #expect(CaptureTargetKind.window(7).regionRect == nil)
        #expect(CaptureTargetKind.app("com.example.app").regionRect == nil)
    }
}
