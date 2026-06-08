import Foundation
import Testing
@testable import PanopsCaptureMac

struct CodecsTests {
    // MARK: - capture_target decoding

    @Test func captureTargetDecodesWindowTarget() throws {
        let json = #"{"kind":"window","window_id":7}"#
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(json.utf8))
        #expect(target.kind == "window")
        #expect(target.windowId == 7)
    }

    @Test func captureTargetDecodesAppTarget() throws {
        let json = #"{"kind":"app","bundle_id":"com.apple.Safari"}"#
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(json.utf8))
        #expect(target.kind == "app")
        #expect(target.bundleId == "com.apple.Safari")
    }

    @Test func captureTargetDecodesRegionTarget() throws {
        let json = #"{"kind":"region","display_id":0,"x":10,"y":20,"w":640,"h":480}"#
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(json.utf8))
        #expect(target.kind == "region")
        #expect(target.displayId == 0)
        #expect(target.x == 10)
        #expect(target.y == 20)
        #expect(target.w == 640)
        #expect(target.h == 480)
    }

    @Test func captureTargetDecodesDisplayTargetWithDisplayId() throws {
        let json = #"{"kind":"display","display_id":1}"#
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(json.utf8))
        #expect(target.kind == "display")
        #expect(target.displayId == 1)
    }

    @Test func captureTargetWithNoFieldsDecodes() throws {
        let json = #"{"kind":"display"}"#
        let target = try JSONDecoder().decode(CaptureTarget.self, from: Data(json.utf8))
        #expect(target.kind == "display")
        #expect(target.displayId == nil)
    }

    // MARK: - CaptureParams decoding with resolution

    @Test func captureParamsDecodesWidthHeight() throws {
        let json = #"{"meeting_id":"m1","width":1280,"height":720}"#
        let params = try JSONDecoder().decode(CaptureParams.self, from: Data(json.utf8))
        #expect(params.width == 1280)
        #expect(params.height == 720)
    }

    @Test func captureParamsResolutionDefaultsToNil() throws {
        let json = #"{"meeting_id":"m1"}"#
        let params = try JSONDecoder().decode(CaptureParams.self, from: Data(json.utf8))
        #expect(params.width == nil)
        #expect(params.height == nil)
    }

    @Test func captureParamsDecodesRegionTargetWithResolution() throws {
        let json = #"{"meeting_id":"m1","capture_target":{"kind":"region","display_id":0,"x":10,"y":20,"w":640,"h":480},"width":1280,"height":720}"#
        let params = try JSONDecoder().decode(CaptureParams.self, from: Data(json.utf8))
        #expect(params.captureTarget?.kind == "region")
        #expect(params.width == 1280)
        #expect(params.height == 720)
    }

    @Test func captureParamsDecodesAppTargetWithResolution() throws {
        let json = #"{"meeting_id":"m1","capture_target":{"kind":"app","bundle_id":"com.apple.Safari"},"width":1920,"height":1080}"#
        let params = try JSONDecoder().decode(CaptureParams.self, from: Data(json.utf8))
        #expect(params.captureTarget?.kind == "app")
        #expect(params.captureTarget?.bundleId == "com.apple.Safari")
        #expect(params.width == 1920)
        #expect(params.height == 1080)
    }
}
