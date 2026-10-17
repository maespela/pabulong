// swift-tools-version: 6.0
import PackageDescription
import Foundation

// Optional RNNoise neural suppressor. Run `Scripts/vendor-rnnoise.sh` to fetch
// and build it; that drops a static library + header under Sources/CRNNoise,
// which this manifest auto-detects to compile and link the neural backend.
// Without it, the package builds cleanly with the spectral-subtraction fallback.
let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let rnnoiseLibDir = packageDir + "/Sources/CRNNoise/lib"
let rnnoiseIncludeDir = packageDir + "/Sources/CRNNoise/include"
let hasRNNoise = FileManager.default.fileExists(atPath: rnnoiseLibDir + "/librnnoise.a")

// Shared Rust DSP core (echo canceller, RNNoise, background-voice gate) — the
// same code the Windows app runs. Built by `Scripts/build-rust-core.sh`. Linked
// only into the Swift audio target, never into the HAL plug-in.
let rustCoreLibDir = packageDir + "/Sources/CNoiselessCore/lib"
let hasRustCore = FileManager.default.fileExists(atPath: rustCoreLibDir + "/libnoiseless_core.a")
var audioCoreSwiftSettings: [SwiftSetting] = []
var audioCoreRustLinker: [LinkerSetting] = []
if hasRustCore {
    audioCoreSwiftSettings.append(.define("NL_HAS_RUST_CORE"))
    audioCoreRustLinker.append(.unsafeFlags(["-L\(rustCoreLibDir)", "-lnoiseless_core"]))
}

var coreCxxSettings: [CXXSetting] = []
var coreLinkerSettings: [LinkerSetting] = []
// With the Rust core present, RNNoise comes from it; linking it into the legacy
// C++ processor as well would duplicate its symbols.
if hasRNNoise && !hasRustCore {
    coreCxxSettings.append(.define("NL_HAS_RNNOISE"))
    coreCxxSettings.append(.unsafeFlags(["-I\(rnnoiseIncludeDir)"]))
    coreLinkerSettings.append(.unsafeFlags(["-L\(rnnoiseLibDir)", "-lrnnoise"]))
}

let package = Package(
    name: "Noiseless",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .library(name: "NoiselessAudioCore", targets: ["NoiselessAudioCore"]),
        .library(name: "NoiselessHALPlugin", type: .dynamic, targets: ["NoiselessHALPlugin"]),
        .executable(name: "NoiselessHelper", targets: ["NoiselessHelper"]),
        .executable(name: "NoiselessApp", targets: ["NoiselessApp"]),
        .executable(name: "NoiselessCoreChecks", targets: ["NoiselessCoreChecks"])
    ],
    targets: [
        .target(
            name: "NoiselessAudioCoreC",
            publicHeadersPath: "include",
            cxxSettings: coreCxxSettings,
            linkerSettings: coreLinkerSettings
        ),
        .target(
            name: "NoiselessAudioCore",
            dependencies: ["NoiselessAudioCoreC"],
            swiftSettings: audioCoreSwiftSettings,
            linkerSettings: [
                .linkedFramework("AVFoundation"),
                .linkedFramework("CoreAudio"),
                .linkedFramework("CoreMedia"),
                .linkedFramework("Security"),
                .linkedLibrary("sqlite3")
            ] + audioCoreRustLinker
        ),
        .executableTarget(
            name: "NoiselessHelper",
            dependencies: ["NoiselessAudioCore"],
            linkerSettings: [
                .linkedFramework("AVFoundation"),
                .linkedFramework("CoreAudio")
            ]
        ),
        .executableTarget(
            name: "NoiselessApp",
            dependencies: ["NoiselessAudioCore"],
            // Consumed by packaging/signing scripts, not SwiftPM build inputs.
            exclude: ["Noiseless.entitlements", "Resources"],
            swiftSettings: audioCoreSwiftSettings,
            linkerSettings: [
                .linkedFramework("AppKit"),
                .linkedFramework("SwiftUI")
            ]
        ),
        .target(
            name: "NoiselessHALPlugin",
            dependencies: ["NoiselessAudioCoreC"],
            publicHeadersPath: "include",
            linkerSettings: [
                .linkedFramework("CoreAudio")
            ]
        ),
        .executableTarget(
            name: "NoiselessCoreChecks",
            dependencies: ["NoiselessAudioCore"]
        )
    ],
    cxxLanguageStandard: .cxx17
)
