import Foundation
import NoiselessAudioCore

@main
struct PabulongHelperMain {
    static func main() throws {
        let arguments = Set(CommandLine.arguments.dropFirst())
        let settingsStore = SettingsStore()
        let settings = settingsStore.load()
        let devices = DeviceManager().listInputDevices()
        let selectedDevice = DeviceManager().resolveInputDevice(uid: settings.selectedInputDeviceUID)

        if arguments.contains("--diagnostics") {
            let processor = NoiseProcessorFactory.make(settings: settings)
            try? processor.prepare()
            let bridge = try SharedVirtualMicBridge()
            let snapshot = DiagnosticsSnapshot(
                settings: settings,
                inputDevices: devices.map { "\($0.name) [\($0.uid)]" },
                bridge: bridge.health,
                processor: processor.metrics
            )
            FileHandle.standardOutput.write(try snapshot.jsonData())
            FileHandle.standardOutput.write(Data("\n".utf8))
            return
        }

        if arguments.contains("--run") {
            let processor = NoiseProcessorFactory.make(settings: settings)
            let bridge = try SharedVirtualMicBridge()
            let pipeline = AudioPipeline(
                processor: processor,
                bridge: bridge,
                preferredInputUID: settings.selectedInputDeviceUID
            )
            do {
                try pipeline.start()
            } catch AudioPipelineError.producerAlreadyRunning {
                fputs("PabulongHelper: another producer (the app or helper) is already feeding Pabulong Mic. Quit it first.\n", stderr)
                exit(1)
            } catch AudioPipelineError.wouldCaptureVirtualDevice {
                fputs("PabulongHelper: refusing to capture the Pabulong virtual mic. Select a physical input device.\n", stderr)
                exit(1)
            } catch AudioPipelineError.inputDeviceUnavailable {
                fputs("PabulongHelper: no physical input device is available.\n", stderr)
                exit(1)
            }
            print("PabulongHelper is feeding Pabulong Mic. Press Control-C to stop.")
            RunLoop.main.run()
            return
        }

        let engineProbe = NoiseProcessorFactory.make(settings: settings)
        try? engineProbe.prepare()

        print("PabulongHelper \(AppVersion.current)")
        print("Selected input: \(selectedDevice?.name ?? "system default unavailable")")
        print("Suppression: \(settings.suppressionMode.displayName)")
        print("Processor: \(settings.processorType.displayName)")
        print("Engine: \(engineProbe.engineName)")
        print("")

        // Report every input device CoreAudio can see, and whether the virtual
        // mic is among them. If it is missing, the HAL plug-in is not installed
        // or coreaudiod refused to load it (usually a signing issue).
        let virtualMicInstalled = devices.contains { $0.uid == DeviceManager.virtualDeviceUID }
        print("Input devices detected by CoreAudio:")
        if devices.isEmpty {
            print("  (none)")
        }
        for device in devices {
            let marker = device.uid == DeviceManager.virtualDeviceUID ? " <- Pabulong Mic" : ""
            print("  - \(device.name) [\(device.uid)]\(marker)")
        }
        print("")
        if virtualMicInstalled {
            print("Pabulong Mic virtual device: DETECTED. Select it in your meeting app.")
        } else {
            print("Pabulong Mic virtual device: NOT FOUND.")
            print("  Install the driver:  ./Scripts/install-hal-plugin.sh")
            print("  Then re-run this command to confirm it appears.")
            print("  If it still does not appear after install, the ad-hoc signature was")
            print("  likely rejected by coreaudiod — sign with a Developer ID (see README).")
        }
        print("")
        print("Run with --run to feed the Pabulong Mic shared audio ring.")
        print("Run with --diagnostics for JSON health output.")
    }
}
