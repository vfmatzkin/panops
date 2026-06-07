// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "PanopsCaptureMac",
    platforms: [.macOS(.v26)],
    products: [
        .executable(name: "panops-capture-mac", targets: ["PanopsCaptureMac"]),
    ],
    targets: [
        .executableTarget(
            name: "PanopsCaptureMac",
            path: "Sources/PanopsCaptureMac",
            linkerSettings: [
                // Embed a minimal Info.plist into the Mach-O `__TEXT,__info_plist`
                // section so the microphone + screen-capture TCC prompts carry a
                // usage string (the sidecar is a bare SwiftPM executable, not an
                // `.app` bundle). Path is relative to the package root; `swift
                // build` invokes the linker from there. See README "Signing".
                .unsafeFlags([
                    "-Xlinker", "-sectcreate",
                    "-Xlinker", "__TEXT",
                    "-Xlinker", "__info_plist",
                    "-Xlinker", "Resources/Info.plist",
                ]),
            ]
        ),
        .testTarget(
            name: "PanopsCaptureMacTests",
            dependencies: ["PanopsCaptureMac"],
            path: "Tests/PanopsCaptureMacTests"
        ),
    ]
)
