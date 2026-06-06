// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "PanopsLlmMac",
    platforms: [.macOS(.v26)],
    products: [
        .executable(name: "panops-llm-mac", targets: ["PanopsLlmMac"]),
    ],
    targets: [
        .executableTarget(
            name: "PanopsLlmMac",
            path: "Sources/PanopsLlmMac"
        ),
        .testTarget(
            name: "PanopsLlmMacTests",
            dependencies: ["PanopsLlmMac"],
            path: "Tests/PanopsLlmMacTests"
        ),
    ]
)
