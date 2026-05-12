// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "PanopsAsrMac",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "panops-asr-mac", targets: ["PanopsAsrMac"]),
    ],
    dependencies: [
        // Verified 2026-05-12: `argmaxinc/argmax-oss-swift` and
        // `argmaxinc/WhisperKit` resolve to the same Package.swift
        // (package name: "argmax-oss-swift"). Latest tag: v1.0.0
        // (2026-05-01). The package exposes WhisperKit + TTSKit +
        // SpeakerKit + CLI executables; we only consume WhisperKit.
        .package(url: "https://github.com/argmaxinc/WhisperKit.git", from: "1.0.0"),
    ],
    targets: [
        .executableTarget(
            name: "PanopsAsrMac",
            dependencies: [
                .product(name: "WhisperKit", package: "WhisperKit"),
            ],
            path: "Sources/PanopsAsrMac"
        ),
    ]
)
