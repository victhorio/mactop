// swift-tools-version: 6.2
import Foundation
import PackageDescription

let repository = URL(fileURLWithPath: #filePath)
  .deletingLastPathComponent().deletingLastPathComponent()

let package = Package(
  name: "Mactop",
  platforms: [.macOS(.v14)],
  products: [.executable(name: "MactopApp", targets: ["MactopApp"])],
  targets: [
    .systemLibrary(name: "CMactop"),
    .target(
      name: "MetricsCore",
      dependencies: ["CMactop"],
      linkerSettings: [
        .unsafeFlags(["-L", repository.appendingPathComponent("target/release").path]),
        .linkedFramework("IOKit"),
        .linkedLibrary("IOReport"),
        .linkedLibrary("iconv"),
      ]
    ),
    .executableTarget(name: "MactopApp", dependencies: ["MetricsCore"]),
    .testTarget(name: "MetricsCoreTests", dependencies: ["MetricsCore"]),
    .testTarget(name: "MactopAppTests", dependencies: ["MactopApp", "MetricsCore"]),
  ]
)
