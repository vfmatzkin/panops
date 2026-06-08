import Foundation
import Testing
@testable import Panops

@Suite("Capture-target resolver")
struct CaptureTargetResolverTests {
    private let windows = [
        WindowInfo(windowId: 42, appName: "Safari", title: "Panops"),
        WindowInfo(windowId: 7, appName: "Xcode", title: "App.swift"),
    ]

    @Test("display choice resolves to .display and can start")
    func displayChoice() {
        let target = CaptureTargetResolver.resolve(
            choice: .display, selectedWindowId: nil, windowList: windows
        )
        #expect(target == .display)
        #expect(CaptureTargetResolver.canStart(target: target))
    }

    @Test("a chosen window maps to capture_target .window(id)")
    func chosenWindowMapsToWindowId() {
        let target = CaptureTargetResolver.resolve(
            choice: .window, selectedWindowId: 42, windowList: windows
        )
        #expect(target == .window(windowId: 42))
        #expect(CaptureTargetResolver.canStart(target: target))
    }

    @Test("window choice with no selection blocks Start")
    func windowChoiceWithoutSelectionBlocksStart() {
        let target = CaptureTargetResolver.resolve(
            choice: .window, selectedWindowId: nil, windowList: windows
        )
        #expect(target == .window(windowId: 0))
        #expect(!CaptureTargetResolver.canStart(target: target))
    }

    @Test("window choice with an empty list falls back to display")
    func windowChoiceWithEmptyListFallsBack() {
        let target = CaptureTargetResolver.resolve(
            choice: .window, selectedWindowId: nil, windowList: []
        )
        #expect(target == .display)
        #expect(CaptureTargetResolver.canStart(target: target))
    }

    @Test("a stale selection not in the list blocks Start")
    func staleSelectionBlocksStart() {
        let target = CaptureTargetResolver.resolve(
            choice: .window, selectedWindowId: 999, windowList: windows
        )
        #expect(target == .window(windowId: 0))
        #expect(!CaptureTargetResolver.canStart(target: target))
    }

    @Test("window_id 0 is never submittable")
    func windowIdZeroNeverSubmittable() {
        #expect(!CaptureTargetResolver.canStart(target: .window(windowId: 0)))
    }
}
