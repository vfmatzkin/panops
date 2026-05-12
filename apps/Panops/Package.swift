// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "Panops",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Panops", targets: ["Panops"]),
    ],
    dependencies: [
        .package(url: "https://github.com/swiftlang/swift-testing.git", from: "0.12.0"),
    ],
    targets: [
        .executableTarget(
            name: "Panops",
            path: "Sources/Panops"
        ),
        .testTarget(
            name: "PanopsTests",
            dependencies: [
                "Panops",
                .product(name: "Testing", package: "swift-testing"),
            ],
            path: "Tests/PanopsTests"
        ),
    ]
)
