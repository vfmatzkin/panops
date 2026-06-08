import Testing
import CoreGraphics
import ScreenCaptureKit
@testable import PanopsCaptureMac

struct CapturePlanTests {
    // MARK: - Resolution-only plan

    @Test func testResolutionSetsWidthHeight() {
        let plan = CapturePlan(outputWidth: 1280, outputHeight: 720, region: nil)
        #expect(plan.outputWidth == 1280)
        #expect(plan.outputHeight == 720)
        #expect(plan.sourceRect == nil)
    }

    // MARK: - Region-only plan

    @Test func testRegionMapsToSourceRect() {
        let region = CGRect(x: 10, y: 20, width: 640, height: 480)
        let plan = CapturePlan(outputWidth: nil, outputHeight: nil, region: region)
        #expect(plan.sourceRect?.minX == 10)
        #expect(plan.sourceRect?.minY == 20)
        #expect(plan.sourceRect?.width == 640)
        #expect(plan.sourceRect?.height == 480)
    }

    @Test func testNoRegionNoSourceRect() {
        let plan = CapturePlan(outputWidth: nil, outputHeight: nil, region: nil)
        #expect(plan.sourceRect == nil)
    }

    // MARK: - Combined plan (resolution + region)

    @Test func testResolutionAndRegionCombined() {
        let region = CGRect(x: 100, y: 50, width: 1920, height: 1080)
        let plan = CapturePlan(outputWidth: 1280, outputHeight: 720, region: region)
        #expect(plan.outputWidth == 1280)
        #expect(plan.outputHeight == 720)
        #expect(plan.sourceRect?.minX == 100)
        #expect(plan.sourceRect?.minY == 50)
        #expect(plan.sourceRect?.width == 1920)
        #expect(plan.sourceRect?.height == 1080)
    }

    // MARK: - CaptureRect initialization (from wire protocol)

    @Test func testCaptureRectFromUInt32() {
        let rect = CapturePlan.CaptureRect(x: 10, y: 20, w: 640, h: 480)
        #expect(rect.x == 10)
        #expect(rect.y == 20)
        #expect(rect.w == 640)
        #expect(rect.h == 480)
    }

    @Test func testCaptureRectToCGRect() {
        let rect = CapturePlan.CaptureRect(x: 10, y: 20, w: 640, h: 480)
        #expect(rect.cgRect == CGRect(x: 10, y: 20, width: 640, height: 480))
    }

    // MARK: - Apply to SCStreamConfiguration

    @Test func testApplyToConfigSetsWidthHeight() throws {
        let plan = CapturePlan(outputWidth: 1280, outputHeight: 720, region: nil)
        let config = SCStreamConfiguration()
        plan.apply(to: config)
        #expect(config.width == 1280)
        #expect(config.height == 720)
        #expect(config.scalesToFit == true)
    }

    @Test func testApplyToConfigSetsSourceRect() throws {
        let region = CGRect(x: 100, y: 100, width: 800, height: 600)
        let plan = CapturePlan(outputWidth: nil, outputHeight: nil, region: region)
        let config = SCStreamConfiguration()
        plan.apply(to: config)
        #expect(config.sourceRect == CGRect(x: 100, y: 100, width: 800, height: 600))
        #expect(config.scalesToFit == true)
    }

    @Test func testApplyToConfigWithNothing() throws {
        let plan = CapturePlan(outputWidth: nil, outputHeight: nil, region: nil)
        let config = SCStreamConfiguration()
        plan.apply(to: config)
        // config should be unchanged (no width/height/region set)
    }
}
