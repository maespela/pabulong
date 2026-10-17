import CoreAudio
import Foundation
#if NL_HAS_RUST_CORE
import NoiselessAudioCoreC
#endif

/// How strongly quieter background voices (people near you, not on the call)
/// are turned down. Implemented by the shared Rust core.
public enum VoiceIsolationLevel: String, Codable, CaseIterable, Sendable {
    case off
    case light
    case balanced
    case strong

    public var displayName: String {
        switch self {
        case .off: "Off"
        case .light: "Light"
        case .balanced: "Balanced"
        case .strong: "Strong"
        }
    }

    var rustValue: UInt32 {
        switch self {
        case .off: 0
        case .light: 1
        case .balanced: 2
        case .strong: 3
        }
    }
}

extension SuppressionMode {
    var rustValue: UInt32 {
        switch self {
        case .light: 0
        case .balanced: 1
        case .max: 2
        }
    }
}

public struct EchoMetrics: Equatable, Sendable {
    /// Echo removed by the canceller (dB), measured while only the far end talks.
    public var echoReductionDB: Float = 0
    /// Speaker → mic delay the canceller found (ms).
    public var echoDelayMS: Float = 0
    /// How far behind the mic the speaker reference is read (ms).
    public var referenceLookbackMS: Float = 0
    /// Background-voice gate gain (1 = open).
    public var voiceGateGain: Float = 1
    public var farEndActive = false

    public init() {}
}

/// A processor that can cancel speaker echo given the speaker output as a reference.
public protocol EchoCancellingProcessor: AnyObject, Sendable {
    var voiceIsolation: VoiceIsolationLevel { get set }
    var echoCancellationEnabled: Bool { get set }
    var echoMetrics: EchoMetrics { get }
    /// Speaker output, 48 kHz mono, whose first sample was played at `hostTimeNanos`.
    func pushReference(_ samples: UnsafeBufferPointer<Float>, hostTimeNanos: UInt64)
}

public enum NoiseProcessorFactory {
    /// True when the shared Rust core (the same DSP as the Windows app) is linked.
    public static var usesSharedCore: Bool {
        #if NL_HAS_RUST_CORE
        true
        #else
        false
        #endif
    }

    public static var engineDescription: String {
        usesSharedCore ? "RNNoise + echo cancellation (shared core)" : (RealtimeNoiseProcessor.neuralModelAvailable ? "RNNoise (neural)" : "Spectral DSP")
    }

    public static func make(settings: AppSettings) -> NoiseProcessor {
        #if NL_HAS_RUST_CORE
        if settings.processorType == .rnnoise {
            return RustVoiceProcessor(
                mode: settings.suppressionMode,
                echoCancellation: settings.echoCancellationEnabled,
                voiceIsolation: settings.voiceIsolation
            )
        }
        #endif
        return RealtimeNoiseProcessor(type: settings.processorType, mode: settings.suppressionMode)
    }
}

/// Nanoseconds on the host clock for a Core Audio host time.
@inline(__always)
func hostTimeNanos(_ hostTime: UInt64) -> UInt64 {
    AudioConvertHostTimeToNanos(hostTime)
}

@inline(__always)
func nowHostTimeNanos() -> UInt64 {
    AudioConvertHostTimeToNanos(AudioGetCurrentHostTime())
}

#if NL_HAS_RUST_CORE
/// The shared mic chain (echo canceller → RNNoise → background-voice gate),
/// implemented in `rust/noiseless-core`. The Windows app links the same library,
/// so both platforms process audio identically.
///
/// Threading: `process` is called on the pipeline's processing queue; settings,
/// metrics and `pushReference` may be used from any thread (the core applies
/// setting changes at the next frame).
public final class RustVoiceProcessor: NoiseProcessor, EchoCancellingProcessor, @unchecked Sendable {
    public let type: NoiseProcessorType = .rnnoise
    private let handle: OpaquePointer

    public var mode: SuppressionMode {
        didSet { nlr_pipeline_set_mode(handle, mode.rustValue) }
    }
    public var voiceIsolation: VoiceIsolationLevel {
        didSet { nlr_pipeline_set_voice_isolation(handle, voiceIsolation.rustValue) }
    }
    public var echoCancellationEnabled: Bool {
        didSet { nlr_pipeline_set_echo_cancellation(handle, echoCancellationEnabled) }
    }

    public init(mode: SuppressionMode = .balanced, echoCancellation: Bool = true, voiceIsolation: VoiceIsolationLevel = .balanced) {
        self.mode = mode
        self.voiceIsolation = voiceIsolation
        self.echoCancellationEnabled = echoCancellation
        guard let created = nlr_pipeline_create(mode.rustValue, echoCancellation, voiceIsolation.rustValue) else {
            fatalError("nlr_pipeline_create returned null")
        }
        handle = created
    }

    deinit {
        nlr_pipeline_destroy(handle)
    }

    public var engineName: String {
        nlr_pipeline_uses_neural(handle) ? "RNNoise (neural) · shared core" : "Spectral DSP · shared core"
    }

    public var metrics: ProcessorMetrics {
        let m = nlr_pipeline_metrics(handle)
        return ProcessorMetrics(inputRMS: m.input_rms, outputRMS: m.output_rms, voiceActive: m.voice_probability > 0.5)
    }

    public var echoMetrics: EchoMetrics {
        let m = nlr_pipeline_metrics(handle)
        var e = EchoMetrics()
        e.echoReductionDB = m.echo_reduction_db
        e.echoDelayMS = m.echo_delay_ms
        e.referenceLookbackMS = m.reference_lookback_ms
        e.voiceGateGain = m.voice_gate_gain
        e.farEndActive = m.far_end_active
        return e
    }

    public func prepare(sampleRate: Double = AudioFrame.sampleRate, channels: Int = 1) throws {
        guard sampleRate == AudioFrame.sampleRate else { throw NoiseProcessorError.unsupportedSampleRate(sampleRate) }
        guard channels == 1 else { throw NoiseProcessorError.unsupportedChannelCount(channels) }
    }

    public func process(_ input: AudioFrame) throws -> AudioFrame {
        try process(input, hostTimeNanos: nowHostTimeNanos())
    }

    public func process(_ input: AudioFrame, hostTimeNanos: UInt64) throws -> AudioFrame {
        var output = [Float](repeating: 0, count: AudioFrame.frameLength)
        let ok = input.samples.withUnsafeBufferPointer { inBuf in
            output.withUnsafeMutableBufferPointer { outBuf in
                nlr_pipeline_process(handle, inBuf.baseAddress, outBuf.baseAddress, UInt32(AudioFrame.frameLength), hostTimeNanos)
            }
        }
        guard ok else { throw NoiseProcessorError.processingFailed }
        return AudioFrame(samples: output)
    }

    public func pushReference(_ samples: UnsafeBufferPointer<Float>, hostTimeNanos: UInt64) {
        guard let base = samples.baseAddress, !samples.isEmpty else { return }
        nlr_pipeline_push_reference(handle, base, UInt32(samples.count), hostTimeNanos)
    }

    public func reset() {
        nlr_pipeline_reset(handle)
    }
}
#endif
