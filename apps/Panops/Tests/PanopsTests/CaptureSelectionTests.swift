import Foundation
import Testing
@testable import Panops

@Suite("ResolutionPreset math")
struct ResolutionPresetTests {
    @Test("a preset derives 16:9 dimensions from its height")
    func presetDimensions() {
        #expect(ResolutionPreset.p720.dimensions(nativeHeight: 1440) == Dimensions(width: 1280, height: 720))
        #expect(ResolutionPreset.p1080.dimensions(nativeHeight: 1440) == Dimensions(width: 1920, height: 1080))
        #expect(ResolutionPreset.p480.dimensions(nativeHeight: 1440) == Dimensions(width: 854, height: 480))
    }

    @Test("native never overrides the output size")
    func nativeIsNil() {
        #expect(ResolutionPreset.native.dimensions(nativeHeight: 1440) == nil)
    }

    @Test("a preset that would upscale falls back to native")
    func noUpscale() {
        // 1080p on a 720-tall source: don't render more pixels than the source.
        #expect(ResolutionPreset.p1080.dimensions(nativeHeight: 720) == nil)
        // The exact-match boundary also collapses to native (no resampling gain).
        #expect(ResolutionPreset.p720.dimensions(nativeHeight: 720) == nil)
    }

    @Test("an unknown native height still applies the preset")
    func unknownNativeHeight() {
        #expect(ResolutionPreset.p720.dimensions(nativeHeight: 0) == Dimensions(width: 1280, height: 720))
    }
}

@Suite("CaptureTargetDTO wire encoding")
struct CaptureTargetDTOTests {
    private var encoder: JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.sortedKeys]
        return e
    }
    private let decoder = JSONDecoder()

    @Test("window target encodes kind-tagged with window_id")
    func windowEncodes() throws {
        let data = try encoder.encode(CaptureTargetDTO.window(windowID: 42))
        let s = String(data: data, encoding: .utf8)!
        #expect(s.contains("\"kind\":\"window\""))
        #expect(s.contains("\"window_id\":42"))
    }

    @Test("display target encodes kind + display_id")
    func displayEncodes() throws {
        let data = try encoder.encode(CaptureTargetDTO.display(displayID: 3))
        let s = String(data: data, encoding: .utf8)!
        #expect(s.contains("\"kind\":\"display\""))
        #expect(s.contains("\"display_id\":3"))
    }

    @Test("app target encodes kind + bundle_id")
    func appEncodes() throws {
        let data = try encoder.encode(CaptureTargetDTO.app(bundleID: "com.apple.Safari"))
        let s = String(data: data, encoding: .utf8)!
        #expect(s.contains("\"kind\":\"app\""))
        #expect(s.contains("\"bundle_id\":\"com.apple.Safari\""))
    }

    @Test("region target encodes kind + display_id + x/y/w/h")
    func regionEncodes() throws {
        let data = try encoder.encode(
            CaptureTargetDTO.region(displayID: 1, x: 10, y: 20, w: 640, h: 480)
        )
        let s = String(data: data, encoding: .utf8)!
        #expect(s.contains("\"kind\":\"region\""))
        #expect(s.contains("\"display_id\":1"))
        #expect(s.contains("\"w\":640"))
        #expect(s.contains("\"h\":480"))
    }

    @Test("every variant round-trips through JSON")
    func roundTrips() throws {
        let targets: [CaptureTargetDTO] = [
            .display(displayID: 0),
            .window(windowID: 7),
            .app(bundleID: "com.example.App"),
            .region(displayID: 2, x: 1, y: 2, w: 3, h: 4),
        ]
        for target in targets {
            let back = try decoder.decode(CaptureTargetDTO.self, from: encoder.encode(target))
            #expect(back == target)
        }
    }

    @Test("a display target with no display_id decodes to the primary display")
    func displayDefaultsToPrimary() throws {
        let target = try decoder.decode(
            CaptureTargetDTO.self, from: Data(#"{"kind":"display"}"#.utf8)
        )
        #expect(target == .display(displayID: 0))
    }
}

@Suite("CaptureSelection aggregate")
struct CaptureSelectionAggregateTests {
    @Test("a selection carries target, resolution, audio, and screenshots")
    func aggregates() {
        let selection = CaptureSelection(
            target: .window(windowID: 9),
            resolution: .p720,
            audioSources: .micOnly,
            captureScreenshots: false
        )
        #expect(selection.target == .window(windowID: 9))
        #expect(selection.resolution == .p720)
        #expect(selection.audioSources == .micOnly)
        #expect(selection.captureScreenshots == false)
    }
}
