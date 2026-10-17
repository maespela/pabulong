import Foundation
import NoiselessAudioCoreC

public struct ProcessorMetrics: Codable, Equatable, Sendable {
    public var inputRMS: Float
    public var outputRMS: Float
    public var voiceActive: Bool

    public init(inputRMS: Float = 0, outputRMS: Float = 0, voiceActive: Bool = false) {
        self.inputRMS = inputRMS
        self.outputRMS = outputRMS
        self.voiceActive = voiceActive
    }
}

public protocol NoiseProcessor: AnyObject {
    var type: NoiseProcessorType { get }
    var mode: SuppressionMode { get set }
    var metrics: ProcessorMetrics { get }
    /// Human-readable name of the suppression engine actually running.
    var engineName: String { get }

    func prepare(sampleRate: Double, channels: Int) throws
    func process(_ input: AudioFrame) throws -> AudioFrame
    /// Processes one frame whose first sample was captured at `hostTimeNanos`
    /// (needed to align the echo canceller's speaker reference).
    func process(_ input: AudioFrame, hostTimeNanos: UInt64) throws -> AudioFrame
    func reset()
}

public extension NoiseProcessor {
    /// Prepares for the standard 48 kHz mono geometry.
    func prepare() throws {
        try prepare(sampleRate: AudioFrame.sampleRate, channels: 1)
    }

    func process(_ input: AudioFrame, hostTimeNanos: UInt64) throws -> AudioFrame {
        try process(input)
    }
}

public enum NoiseProcessorError: Error, Equatable {
    case unsupportedSampleRate(Double)
    case unsupportedChannelCount(Int)
    case allocationFailed
    case processingFailed
}

public final class RealtimeNoiseProcessor: NoiseProcessor {
    /// Whether the neural (RNNoise) suppressor was vendored in at build time.
    /// See `Scripts/vendor-rnnoise.sh`.
    public static var neuralModelAvailable: Bool { nl_rnnoise_available() }

    /// Human-readable name of the suppression engine actually running.
    public var engineName: String {
        if let handle, nl_noise_processor_uses_neural(handle) {
            return "RNNoise (neural)"
        }
        return "Spectral DSP"
    }

    public let type: NoiseProcessorType
    public var mode: SuppressionMode {
        didSet {
            guard let handle else { return }
            nl_noise_processor_set_mode(handle, mode.cMode)
        }
    }

    public private(set) var metrics = ProcessorMetrics()

    private var handle: OpaquePointer?

    public init(type: NoiseProcessorType = .rnnoise, mode: SuppressionMode = .balanced) {
        self.type = type
        self.mode = mode
    }

    deinit {
        if let handle {
            nl_noise_processor_destroy(handle)
        }
    }

    public func prepare(sampleRate: Double = AudioFrame.sampleRate, channels: Int = 1) throws {
        guard sampleRate == AudioFrame.sampleRate else {
            throw NoiseProcessorError.unsupportedSampleRate(sampleRate)
        }
        guard channels == 1 else {
            throw NoiseProcessorError.unsupportedChannelCount(channels)
        }

        if let handle {
            nl_noise_processor_destroy(handle)
        }
        guard let created = nl_noise_processor_create(mode.cMode, UInt32(AudioFrame.sampleRate), UInt32(AudioFrame.frameLength)) else {
            throw NoiseProcessorError.allocationFailed
        }
        handle = created
    }

    public func process(_ input: AudioFrame) throws -> AudioFrame {
        guard let handle else {
            throw NoiseProcessorError.processingFailed
        }

        var output = Array(repeating: Float(0), count: AudioFrame.frameLength)
        var inputSamples = input.samples
        let processed = inputSamples.withUnsafeMutableBufferPointer { inputBuffer in
            output.withUnsafeMutableBufferPointer { outputBuffer in
                nl_noise_processor_process(
                    handle,
                    inputBuffer.baseAddress,
                    outputBuffer.baseAddress,
                    UInt32(AudioFrame.frameLength)
                )
            }
        }
        guard processed else {
            return input
        }

        metrics = ProcessorMetrics(
            inputRMS: nl_noise_processor_last_input_rms(handle),
            outputRMS: nl_noise_processor_last_output_rms(handle),
            voiceActive: nl_noise_processor_last_voice_active(handle)
        )
        return AudioFrame(samples: output)
    }

    public func reset() {
        guard let handle else { return }
        nl_noise_processor_reset(handle)
        metrics = ProcessorMetrics()
    }
}
