import Foundation
import NoiselessAudioCoreC

public struct AudioFrame: Equatable, Sendable {
    public static let sampleRate: Double = 48_000
    public static let frameLength = 480

    public var samples: [Float]

    public init(samples: [Float] = Array(repeating: 0, count: AudioFrame.frameLength)) {
        precondition(samples.count == AudioFrame.frameLength, "AudioFrame must contain exactly \(AudioFrame.frameLength) samples")
        self.samples = samples
    }

    public var rms: Float {
        samples.withUnsafeBufferPointer { buffer in
            nl_frame_rms(buffer.baseAddress, UInt32(buffer.count))
        }
    }
}
