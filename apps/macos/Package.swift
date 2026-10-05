// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Aieyes",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "Aieyes", targets: ["Aieyes"])],
    dependencies: [.package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.9.6")],
    targets: [.executableTarget(name: "Aieyes", dependencies: [.product(name: "Sparkle", package: "Sparkle")])],
    swiftLanguageModes: [.v5]
)
