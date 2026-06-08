import AVFoundation
import SwiftUI

/// SwiftUI wrapper over an `AVSampleBufferDisplayLayer`-backed `NSView`. The
/// layer is owned by `CapturePreviewController`; the preview stream enqueues
/// frames into it directly, so this view just hosts the layer.
struct CapturePreviewView: NSViewRepresentable {
    let layer: AVSampleBufferDisplayLayer

    func makeNSView(context: Context) -> SampleBufferHostingView {
        SampleBufferHostingView(displayLayer: layer)
    }

    func updateNSView(_ nsView: SampleBufferHostingView, context: Context) {}
}

/// A layer-hosting `NSView` whose backing layer is the supplied
/// `AVSampleBufferDisplayLayer`. Keeps the layer sized to the view.
final class SampleBufferHostingView: NSView {
    private let displayLayer: AVSampleBufferDisplayLayer

    init(displayLayer: AVSampleBufferDisplayLayer) {
        self.displayLayer = displayLayer
        super.init(frame: .zero)
        // Assigning the layer before `wantsLayer` makes this a layer-hosting view.
        self.layer = displayLayer
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func layout() {
        super.layout()
        displayLayer.frame = bounds
    }
}

/// The New Recording sheet's "what to capture" section: a system-picker button,
/// the live preview, and the output-resolution picker (shown once a source is
/// chosen). Driven entirely by `CapturePreviewController`.
struct CaptureSourcePane: View {
    @ObservedObject var controller: CapturePreviewController
    @Binding var resolution: ResolutionPreset

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                controller.presentPicker()
            } label: {
                Label(chooseLabel, systemImage: "rectangle.inset.filled.badge.record")
            }

            previewBox

            if controller.target != nil {
                resolutionPicker
            }
        }
    }

    private var chooseLabel: String {
        controller.target == nil ? "Choose what to capture…" : "Change what to capture…"
    }

    private var previewBox: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 8).fill(Color.black.opacity(0.9))
            CapturePreviewView(layer: controller.displayLayer)
                .opacity(controller.state == .live ? 1 : 0)
            CapturePreviewOverlay(state: controller.state, onRetry: { controller.retry() })
            if controller.state == .live, controller.isDisplayTarget {
                CropOverlay(controller: controller)
            }
        }
        .frame(height: 200)
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var resolutionPicker: some View {
        Picker("Resolution", selection: $resolution) {
            ForEach(ResolutionPreset.allCases) { preset in
                Text(resolutionLabel(preset)).tag(preset)
            }
        }
        .pickerStyle(.menu)
    }

    private func resolutionLabel(_ preset: ResolutionPreset) -> String {
        if let dimensions = preset.dimensions(nativeHeight: controller.nativePixelHeight) {
            return "\(preset.label) — \(dimensions.width)×\(dimensions.height)"
        }
        return preset.label
    }
}

/// The non-live states drawn over the preview box: an empty prompt, a spinner,
/// a Screen Recording permission CTA, or a failure with retry. `.live` draws
/// nothing (the layer shows through).
struct CapturePreviewOverlay: View {
    let state: CapturePreviewState
    let onRetry: () -> Void

    var body: some View {
        switch state {
        case .idle:
            message(icon: "rectangle.dashed", text: "Pick a window, app, or display to preview it here.")
        case .starting:
            ProgressView().controlSize(.small).tint(.white)
        case .live:
            EmptyView()
        case .permissionDenied:
            permissionCTA
        case let .failed(detail):
            VStack(spacing: 8) {
                message(icon: "exclamationmark.triangle", text: "Couldn't start the preview.\n\(detail)")
                Button("Retry", action: onRetry).controlSize(.small)
            }
        }
    }

    private var permissionCTA: some View {
        VStack(spacing: 8) {
            message(
                icon: "lock.shield",
                text: "Panops needs Screen Recording to preview and record.\nGrant it in System Settings, then retry."
            )
            Button("Retry", action: onRetry).controlSize(.small)
        }
    }

    private func message(icon: String, text: String) -> some View {
        VStack(spacing: 8) {
            Image(systemName: icon).font(.system(size: 26))
            Text(text).multilineTextAlignment(.center).font(.callout)
        }
        .foregroundStyle(.white.opacity(0.85))
        .padding(.horizontal, 16)
    }
}
