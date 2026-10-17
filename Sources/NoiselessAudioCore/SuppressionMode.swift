import Foundation
import NoiselessAudioCoreC

public enum SuppressionMode: String, Codable, CaseIterable, Sendable {
    case light
    case balanced
    case max

    public var displayName: String {
        switch self {
        case .light: "Light"
        case .balanced: "Balanced"
        case .max: "Max"
        }
    }

    var cMode: NLNoiseMode {
        switch self {
        case .light: NLNoiseModeLight
        case .balanced: NLNoiseModeBalanced
        case .max: NLNoiseModeMax
        }
    }
}

public enum NoiseProcessorType: String, Codable, CaseIterable, Sendable {
    case rnnoise

    public var displayName: String {
        switch self {
        case .rnnoise: "RNNoise"
        }
    }

    /// Values saved by older builds (unimplemented experimental engines) map to
    /// RNNoise instead of failing — a failed decode would reset every setting.
    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = NoiseProcessorType(rawValue: raw) ?? .rnnoise
    }
}
