@preconcurrency import AVFoundation
import Foundation
import os

#if NL_HAS_RUST_CORE
/// Measures echo cancellation on the real hardware: macOS speaks a sentence
/// through the current speakers while the mic is captured and run through the
/// shared chain with the system-output tap as reference — exactly the path used
/// during a call — then compares the echo before and after.
@available(macOS 14.2, *)
@MainActor
public enum SpeakerEchoTest {
    public struct Result: Sendable {
        /// Level of the speaker echo picked up by the mic (dBFS).
        public var echoAtMicDB: Float
        /// Mic level before playback (dBFS).
        public var noiseFloorDB: Float
        /// Echo removed by the chain (dB) once converged.
        public var removedDB: Float
        public var echoDelayMS: Float
        public var referenceLookbackMS: Float
        public var referenceArrived: Bool

        public var summary: String {
            if !referenceArrived {
                return "The speaker reference never arrived — allow Pabulong to record system audio (System Settings › Privacy & Security) and retry."
            }
            if echoAtMicDB - noiseFloorDB < 10 {
                return String(format: "The mic barely heard the speakers (%.0f dBFS over a %.0f dBFS floor). Headphones in use or volume too low — turn the volume up and retry.", echoAtMicDB, noiseFloorDB)
            }
            let verdict = removedDB >= 25 ? "Excellent" : removedDB >= 15 ? "Good" : removedDB >= 8 ? "Partial" : "Not working"
            return String(format: "%@: removed %.0f dB of speaker echo (echo at mic %.0f dBFS, path %.0f ms, reference read %.0f ms behind).", verdict, removedDB, echoAtMicDB, echoDelayMS, referenceLookbackMS)
        }
    }

    public enum TestError: Error, LocalizedError {
        case micBusy
        case start(Error)

        public var errorDescription: String? {
            switch self {
            case .micBusy: "End the call first — the test needs the mic to itself."
            case .start(let error): "Couldn't start the test: \(error.localizedDescription)"
            }
        }
    }

    private final class Recorder: AudioFrameSink, @unchecked Sendable {
        let raw = OSAllocatedUnfairLock(initialState: [Float]())
        let out = OSAllocatedUnfairLock(initialState: [Float]())
        func attachProducer() {}
        func detachProducer() {}
        func write(_ frame: AudioFrame) -> Bool { true }
        func writeLatest(_ frame: AudioFrame) -> Bool { true }
        func clear() {}
        var count: Int { raw.withLock { $0.count } }
    }

    public static func run(preferredInputUID: String?, deviceManager: DeviceManager = DeviceManager()) async throws -> Result {
        let processor = RustVoiceProcessor(mode: .balanced, echoCancellation: true, voiceIsolation: .off)
        let recorder = Recorder()
        let pipeline = AudioPipeline(processor: processor, bridge: recorder, deviceManager: deviceManager, preferredInputUID: preferredInputUID)
        pipeline.inputFrameHandler = { frame in recorder.raw.withLock { $0.append(contentsOf: frame.samples) } }
        pipeline.processedFrameHandler = { frame in recorder.out.withLock { $0.append(contentsOf: frame.samples) } }

        let feed = EchoReferenceFeed(target: processor)
        do {
            try await feed.start()
        } catch {
            throw TestError.start(error)
        }
        do {
            try pipeline.start()
        } catch AudioPipelineError.producerAlreadyRunning {
            await feed.stop()
            throw TestError.micBusy
        } catch {
            await feed.stop()
            throw TestError.start(error)
        }

        // 1 s of quiet for the noise floor, then speech through the speakers.
        try? await Task.sleep(for: .seconds(1))
        let playbackStart = recorder.count
        let synth = AVSpeechSynthesizer()
        let utterance = AVSpeechUtterance(string: "This is the Pabulong speaker echo test. Please stay quiet for a few seconds while I play this sentence through your speakers. If you can hear me clearly, the test is working, and the echo should disappear from your microphone.")
        synth.speak(utterance)
        try? await Task.sleep(for: .milliseconds(300))
        let deadline = Date().addingTimeInterval(25)
        while synth.isSpeaking, Date() < deadline {
            try? await Task.sleep(for: .milliseconds(100))
        }
        try? await Task.sleep(for: .milliseconds(400))
        let playbackEnd = recorder.count
        let metrics = processor.echoMetrics
        pipeline.stop()
        await feed.stop()

        let raw = recorder.raw.withLock { $0 }
        let out = recorder.out.withLock { $0 }
        func level(_ x: ArraySlice<Float>) -> Float {
            guard !x.isEmpty else { return -120 }
            let ms = x.reduce(Float(0)) { $0 + $1 * $1 } / Float(x.count)
            return 10 * log10(max(ms, 1e-12))
        }
        let fs = Int(AudioFrame.sampleRate)
        let floorRange = max(0, playbackStart - fs / 2)..<playbackStart
        // Skip the first 2.5 s of playback while the canceller converges.
        let measureStart = min(playbackStart + fs * 5 / 2, playbackEnd)
        let rawEcho = level(raw[measureStart..<min(playbackEnd, raw.count)])
        let outEcho = level(out[measureStart..<min(playbackEnd, out.count)])
        return Result(
            echoAtMicDB: rawEcho,
            noiseFloorDB: level(raw[floorRange.clamped(to: 0..<raw.count)]),
            removedDB: rawEcho - outEcho,
            echoDelayMS: metrics.echoDelayMS,
            referenceLookbackMS: metrics.referenceLookbackMS,
            referenceArrived: metrics.referenceLookbackMS > 0 || metrics.farEndActive || metrics.echoReductionDB > 0
        )
    }
}
#endif
