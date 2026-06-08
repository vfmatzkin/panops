import Foundation

/// What a recording captures. Mirrors the engine wire type
/// (`panops-protocol::CaptureTarget`, `#[serde(tag = "kind", rename_all = "snake_case")]`)
/// field-for-field so it encodes straight onto `recording.start`.
///
/// - `display` — a whole display (`display_id` 0 = primary).
/// - `window` — a single window by ScreenCaptureKit window id.
/// - `app` — all windows of an app, by bundle id.
/// - `region` — a sub-rectangle of a display, in display points.
enum CaptureTargetDTO: Equatable {
    case display(displayID: UInt32)
    case window(windowID: UInt32)
    case app(bundleID: String)
    case region(displayID: UInt32, x: UInt32, y: UInt32, w: UInt32, h: UInt32)

    /// The default target when nothing is picked: the primary display.
    static var primaryDisplay: CaptureTargetDTO { .display(displayID: 0) }
}

extension CaptureTargetDTO: Codable {
    private enum CodingKeys: String, CodingKey {
        case kind
        case displayID = "display_id"
        case windowID = "window_id"
        case bundleID = "bundle_id"
        case x, y, w, h
    }

    /// Discriminator matching the wire's `snake_case` `kind` tag.
    private enum Kind: String, Codable {
        case display, window, app, region
    }

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .display(displayID):
            try c.encode(Kind.display, forKey: .kind)
            try c.encode(displayID, forKey: .displayID)
        case let .window(windowID):
            try c.encode(Kind.window, forKey: .kind)
            try c.encode(windowID, forKey: .windowID)
        case let .app(bundleID):
            try c.encode(Kind.app, forKey: .kind)
            try c.encode(bundleID, forKey: .bundleID)
        case let .region(displayID, x, y, w, h):
            try c.encode(Kind.region, forKey: .kind)
            try c.encode(displayID, forKey: .displayID)
            try c.encode(x, forKey: .x)
            try c.encode(y, forKey: .y)
            try c.encode(w, forKey: .w)
            try c.encode(h, forKey: .h)
        }
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decode(Kind.self, forKey: .kind) {
        case .display:
            self = .display(displayID: try c.decodeIfPresent(UInt32.self, forKey: .displayID) ?? 0)
        case .window:
            self = .window(windowID: try c.decode(UInt32.self, forKey: .windowID))
        case .app:
            self = .app(bundleID: try c.decode(String.self, forKey: .bundleID))
        case .region:
            self = .region(
                displayID: try c.decodeIfPresent(UInt32.self, forKey: .displayID) ?? 0,
                x: try c.decode(UInt32.self, forKey: .x),
                y: try c.decode(UInt32.self, forKey: .y),
                w: try c.decode(UInt32.self, forKey: .w),
                h: try c.decode(UInt32.self, forKey: .h)
            )
        }
    }
}

/// Output dimensions in pixels. Both fields move together onto the wire's
/// `width`/`height`.
struct Dimensions: Equatable {
    let width: Int
    let height: Int
}

/// Output-resolution choice offered in the New Recording sheet. `native` keeps
/// the source's own size (no downscale); the rest pin a 16:9 height and derive
/// an even width from it.
enum ResolutionPreset: String, CaseIterable, Identifiable {
    case native
    case p1080
    case p720
    case p480

    var id: String { rawValue }

    var label: String {
        switch self {
        case .native: return "Native"
        case .p1080: return "1080p"
        case .p720: return "720p"
        case .p480: return "480p"
        }
    }

    /// Fixed output height for the preset, or `nil` for native (no override).
    private var targetHeight: Int? {
        switch self {
        case .native: return nil
        case .p1080: return 1080
        case .p720: return 720
        case .p480: return 480
        }
    }

    /// Resolve the preset to concrete output dimensions, given the source's
    /// native pixel height. Returns `nil` for `native`, and also when the preset
    /// would *upscale* (its height ≥ the known native height) — there's no point
    /// rendering more pixels than the source has. A `nativeHeight` of `0`
    /// (unknown) skips the no-upscale guard so the preset still applies.
    func dimensions(nativeHeight: Int) -> Dimensions? {
        guard let height = targetHeight else { return nil }
        if nativeHeight > 0, nativeHeight <= height { return nil }
        // 16:9 width, rounded to an even number (H.264 requires even dimensions).
        let halfWidth = (Double(height) * 16.0 / 9.0 / 2.0).rounded()
        return Dimensions(width: Int(halfWidth) * 2, height: height)
    }
}

/// The full description of what a recording will capture: the source, the
/// output resolution, the audio mix, and whether screenshots are sampled. The
/// New Recording sheet assembles one and the recording pipeline honors it.
struct CaptureSelection: Equatable {
    var target: CaptureTargetDTO
    var resolution: ResolutionPreset
    var audioSources: AudioSourcesWire
    var captureScreenshots: Bool
}
