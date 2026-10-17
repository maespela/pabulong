import AVFoundation
import Foundation
import NoiselessAudioCore

enum CheckFailure: Error, CustomStringConvertible {
    case failed(String)

    var description: String {
        switch self {
        case .failed(let message): message
        }
    }
}

func check(_ condition: @autoclosure () -> Bool, _ message: String) throws {
    if !condition() {
        throw CheckFailure.failed(message)
    }
}

func runChecks() throws {
    try bridgeReadsSilenceOnUnderrun()
    try bridgeWraparoundAndOverflow()
    try processorAcceptsNominalFrameSize()
    try denoiserAttenuatesSteadyNoise()
    try legacyProcessorTypeDecodesToRNNoise()
    try limiterPreventsClipping()
    try settingsRoundTrip()
    try settingsMigrationKeepsRecordingOff()
    try sharedBridgeRoundTrip()
    try sharedBridgeKeepsLatestOnProducerOverflow()
    try physicalInputNeverResolvesVirtualDevice()
    try producerGuardIsExclusive()
    try sharedRingSurvivesProducerReconnect()
    try callMetadataUsesMeetingTitle()
    try legacyCallMetadataIgnoresRetiredFields()
    try callMetadataRoundTrip()
    try callRecorderCreatesLocalFiles()
    try sharedCoreIsTheDefaultProcessor()
    try sharedCoreSuppressesSteadyNoiseAndLimits()
    try sharedCoreCancelsSpeakerEcho()
    try sharedCoreSettingsPersist()
    try recordingNeedsConsentAcknowledgement()
}

func recordingNeedsConsentAcknowledgement() throws {
    try check(!AppSettings().recordingConsentAcknowledged, "recording consent must default to not acknowledged")
    let paths = try temporaryCallPaths()
    let store = try CallMetadataStore(paths: paths)
    let manager = try CallSessionManager(store: store)
    // Automatic recording requested, but consent not confirmed: nothing may start.
    manager.updateVirtualMicConsumerCount(1, settings: AppSettings(automaticCallRecordingEnabled: true, localRecordingEnabled: true))
    manager.startManualSession(settings: AppSettings(localRecordingEnabled: true))
    Thread.sleep(forTimeInterval: 0.2)
    try check(manager.activeSession == nil, "no recording may start before consent is acknowledged")
}

// MARK: - Shared (Rust) core, called exactly as the app calls it

func sharedCoreIsTheDefaultProcessor() throws {
    try check(NoiseProcessorFactory.usesSharedCore, "the shared Rust core must be linked (run Scripts/build-rust-core.sh)")
    let processor = NoiseProcessorFactory.make(settings: AppSettings())
    try check(processor is EchoCancellingProcessor, "default processor must be the shared echo-cancelling chain")
    try check(processor.engineName.contains("RNNoise"), "shared core must run RNNoise, got \(processor.engineName)")
}

func sharedCoreSuppressesSteadyNoiseAndLimits() throws {
    let processor = NoiseProcessorFactory.make(settings: AppSettings(suppressionMode: .max))
    try processor.prepare()
    var seed: UInt64 = 0x9E3779B97F4A7C15
    var lastIn: Float = 0, lastOut: Float = 0
    for i in 0..<120 {
        let frame = AudioFrame(samples: (0..<AudioFrame.frameLength).map { _ in
            seed = seed &* 6364136223846793005 &+ 1442695040888963407
            return (Float(seed >> 40) / Float(1 << 24) - 0.5) * 0.1
        })
        let out = try processor.process(frame, hostTimeNanos: UInt64(i) * 10_000_000)
        try check(out.samples.count == AudioFrame.frameLength && out.samples.allSatisfy(\.isFinite), "output must be a finite full frame")
        lastIn = frame.rms
        lastOut = out.rms
    }
    try check(lastOut < lastIn * 0.5, "steady noise should be attenuated (out \(lastOut) vs in \(lastIn))")
    let loud = AudioFrame(samples: Array(repeating: 3, count: AudioFrame.frameLength))
    for i in 0..<40 {
        let out = try processor.process(loud, hostTimeNanos: UInt64(200 + i) * 10_000_000)
        try check((out.samples.map(abs).max() ?? 0) <= 0.9801, "limiter must prevent clipping")
    }
}

/// Speaker output is pushed in 512-sample chunks with host times, exactly like
/// the system-output tap; the mic hears it 20 ms later through a short room
/// response. The chain must remove most of it.
func sharedCoreCancelsSpeakerEcho() throws {
    guard let chain = NoiseProcessorFactory.make(settings: AppSettings(voiceIsolation: .off)) as? (NoiseProcessor & EchoCancellingProcessor) else {
        throw CheckFailure.failed("shared core unavailable")
    }
    try chain.prepare()
    let fs = 48_000
    let seconds = 6
    var seed: UInt64 = 42
    // Speech-like reference: noise bursts with a syllable-rate envelope.
    let speaker: [Float] = (0..<(fs * seconds)).map { n in
        seed = seed &* 6364136223846793005 &+ 1442695040888963407
        let envelope = 0.5 + 0.5 * sin(2 * Float.pi * 4 * Float(n) / Float(fs))
        return (Float(seed >> 40) / Float(1 << 24) - 0.5) * 0.4 * envelope
    }
    let delay = fs / 50 // 20 ms
    var mic = [Float](repeating: 0, count: speaker.count)
    for n in delay..<speaker.count {
        mic[n] = 0.7 * speaker[n - delay] + (n - delay >= 30 ? 0.25 * speaker[n - delay - 30] : 0)
    }
    func nanos(_ sample: Int) -> UInt64 { UInt64(Double(sample) * 1e9 / Double(fs)) }

    var pushed = 0
    var micEnergy: Float = 0, outEnergy: Float = 0
    for f in 0..<(mic.count / AudioFrame.frameLength) {
        let start = f * AudioFrame.frameLength
        // Tap chunks arrive ~8 ms after they are played.
        while pushed + 512 <= speaker.count, pushed + 512 + 384 <= start + AudioFrame.frameLength {
            speaker[pushed..<(pushed + 512)].withUnsafeBufferPointer { chain.pushReference($0, hostTimeNanos: nanos(pushed)) }
            pushed += 512
        }
        let frame = AudioFrame(samples: Array(mic[start..<(start + AudioFrame.frameLength)]))
        let out = try chain.process(frame, hostTimeNanos: nanos(start))
        if start >= 3 * fs { // after convergence
            micEnergy += frame.samples.reduce(0) { $0 + $1 * $1 }
            outEnergy += out.samples.reduce(0) { $0 + $1 * $1 }
        }
    }
    let removedDB = 10 * log10(micEnergy / max(outEnergy, 1e-12))
    try check(removedDB > 20, "speaker echo should be cut by > 20 dB, got \(removedDB) dB")
    let m = chain.echoMetrics
    try check(m.echoDelayMS >= 0 && m.echoDelayMS < 40, "echo delay should be found near 20 ms minus lookback, got \(m.echoDelayMS)")
}

func sharedCoreSettingsPersist() throws {
    let defaults = UserDefaults(suiteName: "NoiselessCoreChecks-\(UUID().uuidString)")!
    let store = SettingsStore(defaults: defaults)
    try check(store.load().echoCancellationEnabled, "echo cancellation should default on")
    try check(store.load().voiceIsolation == .balanced, "background voice isolation should default to balanced")
    try store.save(AppSettings(echoCancellationEnabled: false, voiceIsolation: .strong))
    let loaded = store.load()
    try check(!loaded.echoCancellationEnabled && loaded.voiceIsolation == .strong, "echo/voice settings should persist")
}

func producerGuardIsExclusive() throws {
    let path = NSTemporaryDirectory() + "nlchecks-producer-\(UUID().uuidString).lock"
    let first = ProducerGuard(path: path)
    try check(first != nil, "first producer should acquire the lock")
    try check(ProducerGuard(path: path) == nil, "second producer must be refused while the lock is held")
    _ = first // keep the lock held until here
}

func sharedRingSurvivesProducerReconnect() throws {
    let name = "/nlreconnect\(Int.random(in: 1000...9999))"
    let frame = AudioFrame(samples: Array(repeating: 0.5, count: AudioFrame.frameLength))

    let producer = try SharedVirtualMicBridge(name: name, create: true, frameCapacity: 4)
    try check(producer.write(frame), "producer should write a frame")

    // A second producer opening the same ring must NOT wipe buffered audio.
    let reconnect = try SharedVirtualMicBridge(name: name, create: true, frameCapacity: 4)
    try check(reconnect.readOrSilence() == frame, "reconnecting producer must not clobber the ring")
}

func physicalInputNeverResolvesVirtualDevice() throws {
    let manager = DeviceManager()
    // Even when the virtual mic is explicitly requested, capture resolution must
    // never hand back the virtual device — that path is the audio feedback loop.
    let resolved = manager.resolvePhysicalInputDevice(uid: DeviceManager.virtualDeviceUID)
    try check(resolved?.uid != DeviceManager.virtualDeviceUID, "must never resolve the virtual mic for capture")
}

func callMetadataUsesMeetingTitle() throws {
    let paths = try temporaryCallPaths()
    let store = try CallMetadataStore(paths: paths)

    let titled = try store.createSession(title: "Weekly Product Sync", appName: "Microsoft Teams")
    try check(titled.title == "Weekly Product Sync", "call metadata should prefer detected meeting title")
    try check(titled.appName == "Microsoft Teams", "call metadata should preserve detected app name")

    let fallback = try store.createSession(appName: "Google Meet")
    try check(fallback.title.hasPrefix("Google Meet Call "), "call metadata should include app name when meeting title is unavailable")
}

func legacyCallMetadataIgnoresRetiredFields() throws {
    let session = CallSession(title: "Legacy recording", appName: "Microsoft Teams")
    let encoder = JSONEncoder()
    encoder.dateEncodingStrategy = .iso8601
    var payload = try JSONSerialization.jsonObject(with: encoder.encode(session)) as! [String: Any]
    payload["transcriptPath"] = "/tmp/transcript.json"
    payload["summaryPath"] = "/tmp/summary.json"
    payload["transcriptStatus"] = "completed"
    payload["summaryStatus"] = "completed"
    payload["speakerNames"] = ["1": "Speaker 1"]

    let decoder = JSONDecoder()
    decoder.dateDecodingStrategy = .iso8601
    let decoded = try decoder.decode(CallSession.self, from: JSONSerialization.data(withJSONObject: payload))
    try check(decoded.id == session.id, "legacy call metadata should remain readable")
    try check(decoded.localMicAudioPath == session.localMicAudioPath, "legacy recording paths should be preserved")
}

func bridgeReadsSilenceOnUnderrun() throws {
    let bridge = try VirtualMicBridge(frameCapacity: 2)
    let frame = bridge.readOrSilence()
    try check(frame.samples == Array(repeating: Float(0), count: AudioFrame.frameLength), "bridge should output silence on underrun")
    try check(bridge.health.underrunCount == 1, "bridge should count underruns")
}

func bridgeWraparoundAndOverflow() throws {
    let bridge = try VirtualMicBridge(frameCapacity: 2)
    let first = AudioFrame(samples: Array(repeating: 0.1, count: AudioFrame.frameLength))
    let second = AudioFrame(samples: Array(repeating: 0.2, count: AudioFrame.frameLength))
    let overflow = AudioFrame(samples: Array(repeating: 0.3, count: AudioFrame.frameLength))

    try check(bridge.write(first), "first frame should write")
    try check(bridge.write(second), "second frame should write")
    try check(!bridge.write(overflow), "third frame should overflow")
    try check(bridge.health.overrunCount == 1, "bridge should count overruns")
    try check(bridge.readOrSilence() == first, "first frame should read back")
    try check(bridge.readOrSilence() == second, "second frame should read back")
}

func processorAcceptsNominalFrameSize() throws {
    let processor = RealtimeNoiseProcessor(mode: .balanced)
    try processor.prepare()
    let input = AudioFrame(samples: sineFrame(amplitude: 0.2))

    // The STFT suppressor is a streaming filter with a few frames of latency, so
    // drive it past the initial fill before asserting on steady-state output.
    var output = input
    for _ in 0..<40 {
        output = try processor.process(input)
    }

    try check(output.samples.count == AudioFrame.frameLength, "processor should preserve frame length")
    try check(output.samples.allSatisfy { $0.isFinite }, "processor output must be finite")
    try check(processor.metrics.inputRMS > 0, "processor should update input RMS")
    try check(processor.metrics.outputRMS > 0, "processor should update output RMS")
}

func denoiserAttenuatesSteadyNoise() throws {
    let processor = RealtimeNoiseProcessor(mode: .max)
    try processor.prepare()

    var seed: UInt64 = 0x9E3779B97F4A7C15
    func noiseFrame() -> [Float] {
        (0..<AudioFrame.frameLength).map { _ in
            seed = seed &* 6364136223846793005 &+ 1442695040888963407
            return (Float(seed >> 40) / Float(1 << 24) - 0.5) * 0.1
        }
    }

    var lastInput: Float = 0
    var lastOutput: Float = 0
    for _ in 0..<80 {
        let frame = AudioFrame(samples: noiseFrame())
        lastInput = frame.rms
        lastOutput = try processor.process(frame).rms
    }

    // Stationary broadband noise, with no voice present, must be pushed well
    // below the input level once the noise estimate has converged.
    try check(lastOutput < lastInput * 0.5, "steady noise should be attenuated (out \(lastOutput) vs in \(lastInput))")
}

func legacyProcessorTypeDecodesToRNNoise() throws {
    // Older builds could save placeholder engine names; they must not wipe settings.
    let defaults = UserDefaults(suiteName: "NoiselessCoreChecks-\(UUID().uuidString)")!
    let legacy = try JSONSerialization.data(withJSONObject: [
        "processingEnabled": true,
        "processorType": "deepFilterNetExperimental",
        "suppressionMode": "max"
    ])
    defaults.set(legacy, forKey: SettingsStore.defaultKey)
    let loaded = SettingsStore(defaults: defaults).load()
    try check(loaded.processorType == .rnnoise, "legacy engine name should fall back to RNNoise")
    try check(loaded.processingEnabled && loaded.suppressionMode == .max, "other settings must survive a legacy engine name")
}

func limiterPreventsClipping() throws {
    let processor = RealtimeNoiseProcessor(mode: .max)
    try processor.prepare()
    let input = AudioFrame(samples: Array(repeating: Float(3), count: AudioFrame.frameLength))

    // Drive several frames of a very loud signal; no output sample may exceed the
    // limiter ceiling at any point in the stream.
    for _ in 0..<40 {
        let output = try processor.process(input)
        try check((output.samples.map { abs($0) }.max() ?? 0) <= 0.9801, "limiter should prevent clipping")
    }
}

func settingsRoundTrip() throws {
    let defaults = UserDefaults(suiteName: "NoiselessCoreChecks-\(UUID().uuidString)")!
    let store = SettingsStore(defaults: defaults)
    let settings = AppSettings(
        processingEnabled: true,
        selectedInputDeviceUID: "mic-1",
        suppressionMode: .max,
        launchAtLogin: true,
        bypassEnabled: true,
        processorType: .rnnoise,
        automaticCallRecordingEnabled: true,
        captureSystemAudioEnabled: false,
        localRecordingEnabled: true
    )

    try store.save(settings)

    try check(store.load() == settings, "settings should round-trip through UserDefaults")
}

func settingsMigrationKeepsRecordingOff() throws {
    let defaults = UserDefaults(suiteName: "NoiselessCoreChecks-\(UUID().uuidString)")!
    let oldSettings = try JSONSerialization.data(withJSONObject: [
        "processingEnabled": true,
        "selectedInputDeviceUID": "legacy-mic"
    ])
    defaults.set(oldSettings, forKey: SettingsStore.defaultKey)

    let loaded = SettingsStore(defaults: defaults).load()
    try check(loaded.selectedInputDeviceUID == "legacy-mic", "settings migration should preserve existing mic selection")
    try check(!loaded.automaticCallRecordingEnabled, "recording must stay off for existing users")
    try check(loaded.captureSystemAudioEnabled, "system-audio capture default should be available once recording is enabled")
    try check(loaded.localRecordingEnabled, "local recording default should be available once recording is enabled")
}

func sharedBridgeRoundTrip() throws {
    let name = "/nlchecks\(Int.random(in: 1000...9999))"
    let producer = try SharedVirtualMicBridge(name: name, create: true, frameCapacity: 2)
    let consumer = try SharedVirtualMicBridge(name: name, create: false, frameCapacity: 2)
    let frame = AudioFrame(samples: Array(repeating: 0.25, count: AudioFrame.frameLength))

    producer.attachProducer()
    try check(producer.write(frame), "shared bridge should write")
    try check(consumer.readOrSilence() == frame, "shared bridge should read across handles")
    try check(consumer.health.producerAttached, "shared bridge should expose producer attachment")
    producer.detachProducer()
}

func sharedBridgeKeepsLatestOnProducerOverflow() throws {
    let name = "/nllatest\(Int.random(in: 1000...9999))"
    let producer = try SharedVirtualMicBridge(name: name, create: true, frameCapacity: 2)
    let consumer = try SharedVirtualMicBridge(name: name, create: false, frameCapacity: 2)
    let first = AudioFrame(samples: Array(repeating: 0.1, count: AudioFrame.frameLength))
    let second = AudioFrame(samples: Array(repeating: 0.2, count: AudioFrame.frameLength))
    let latest = AudioFrame(samples: Array(repeating: 0.3, count: AudioFrame.frameLength))

    try check(producer.writeLatest(first), "shared bridge should write first frame")
    try check(producer.writeLatest(second), "shared bridge should write second frame")
    try check(producer.writeLatest(latest), "shared bridge should keep latest frame when full")
    try check(consumer.readOrSilence() == second, "oldest frame should be dropped on producer overflow")
    try check(consumer.readOrSilence() == latest, "latest frame should remain available to HAL")
    try check(consumer.health.overrunCount == 1, "shared bridge should count producer-side replacement")
}

func callMetadataRoundTrip() throws {
    let paths = try temporaryCallPaths()
    let store = try CallMetadataStore(paths: paths)
    var session = try store.createSession(appName: "Zoom")
    try check(session.appName == "Zoom", "call metadata should preserve app name")
    try check(FileManager.default.fileExists(atPath: paths.directory(for: session.id).path), "call directory should be created")

    session.status = .completed
    session.endedAt = Date()
    try store.update(session)

    let sessions = try store.listSessions()
    try check(sessions.first?.status == .completed, "call list should return updated session")

    try store.deleteSession(id: session.id)
    let remainingSessions = try store.listSessions()
    try check(remainingSessions.isEmpty, "deleted call should be removed from metadata")
    try check(!FileManager.default.fileExists(atPath: paths.directory(for: session.id).path), "deleted call should remove files")
}

func callRecorderCreatesLocalFiles() throws {
    let paths = try temporaryCallPaths()
    let store = try CallMetadataStore(paths: paths)
    let session = try store.createSession(appName: "Recorder Check")
    let recorder = CallRecorder()
    try recorder.start(session: session)

    // ~0.5 s of 48 kHz mic frames; recordings are downsampled to 16 kHz on write.
    for _ in 0..<50 {
        recorder.recordLocalFrame(AudioFrame(samples: sineFrame(amplitude: 0.15)))
    }
    // Let the recorder queue drain the async frame writes before stopping.
    Thread.sleep(forTimeInterval: 0.05)
    recorder.stop()

    for path in [session.localMicAudioPath, session.mixedPreviewAudioPath] {
        guard let path else { throw CheckFailure.failed("recording path should exist") }
        let attributes = try FileManager.default.attributesOfItem(atPath: path)
        let size = attributes[.size] as? UInt64 ?? 0
        try check(size > 44, "recorded WAV should contain audio data")
        let file = try AVAudioFile(forReading: URL(fileURLWithPath: path))
        try check(file.length > 0, "recorded WAV should have playable audio frames")
    }
}

func sineFrame(amplitude: Float) -> [Float] {
    (0..<AudioFrame.frameLength).map { index in
        amplitude * sin(Float(index) / 480 * Float.pi * 2 * 4)
    }
}

func temporaryCallPaths() throws -> CallDataPaths {
    let root = URL(fileURLWithPath: NSTemporaryDirectory(), isDirectory: true)
        .appending(path: "NoiselessCoreChecks-\(UUID().uuidString)", directoryHint: .isDirectory)
    let paths = CallDataPaths(rootDirectory: root)
    try paths.prepareDirectories()
    return paths
}

do {
    try runChecks()
    print("NoiselessCoreChecks passed")
} catch {
    fputs("NoiselessCoreChecks failed: \(error)\n", stderr)
    exit(1)
}
