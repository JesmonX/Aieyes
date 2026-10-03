// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Aieyes",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "Aieyes", targets: ["Aieyes"])],
    targets: [.executableTarget(name: "Aieyes")],
    swiftLanguageModes: [.v5]
)

