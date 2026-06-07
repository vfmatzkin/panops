// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Panops",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Panops", targets: ["Panops"]),
    ],
    targets: [
        .executableTarget(
            name: "Panops",
            path: "Sources/Panops"
        ),
        .testTarget(
            name: "PanopsTests",
            dependencies: ["Panops"],
            path: "Tests/PanopsTests"
        ),
    ]
)
