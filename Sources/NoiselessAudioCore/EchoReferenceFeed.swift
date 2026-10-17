@preconcurrency import AVFoundation
import CoreAudio
import Foundation

/// Feeds what the speakers are playing to the echo canceller as its reference.
///
/// The system output mix is captured with a Core Audio process tap (the
/// Windows app does the same with WASAPI loopback), converted to 48 kHz mono,
/// and pushed with its host time so the core can align it with the mic.
@available(macOS 14.2, *)
public final class EchoReferenceFeed: @unchecked Sendable {
    private let capture = CoreAudioTapCapture()
    private weak var target: (any EchoCancellingProcessor)?
    private let monoFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: AudioFrame.sampleRate, channels: 1, interleaved: false)!
    private var converter: AVAudioConverter?
    private var mono: [Float] = []

    public init(target: any EchoCancellingProcessor) {
        self.target = target
    }

    public func start() async throws {
        capture.onTimedAudioBuffer = { [weak self] buffer, hostTimeNanos in
            self?.handle(buffer, hostTimeNanos: hostTimeNanos)
        }
        try await capture.start()
    }

    public func stop() async {
        capture.onTimedAudioBuffer = nil
        await capture.stop()
    }

    // Runs on the tap's IO queue.
    private func handle(_ buffer: AVAudioPCMBuffer, hostTimeNanos: UInt64) {
        guard let target, buffer.frameLength > 0 else { return }
        let format = buffer.format
        let frames = Int(buffer.frameLength)

        if format.sampleRate == AudioFrame.sampleRate, format.commonFormat == .pcmFormatFloat32, let data = buffer.floatChannelData {
            // Fast path: already 48 kHz float — just downmix.
            let channels = Int(format.channelCount)
            if mono.count < frames { mono = [Float](repeating: 0, count: frames) }
            let scale = 1 / Float(max(channels, 1))
            if format.isInterleaved {
                let interleaved = data[0]
                for i in 0..<frames {
                    var sum: Float = 0
                    for c in 0..<channels { sum += interleaved[i * channels + c] }
                    mono[i] = sum * scale
                }
            } else {
                for i in 0..<frames {
                    var sum: Float = 0
                    for c in 0..<channels { sum += data[c][i] }
                    mono[i] = sum * scale
                }
            }
            mono.withUnsafeBufferPointer { target.pushReference(UnsafeBufferPointer(rebasing: $0[0..<frames]), hostTimeNanos: hostTimeNanos) }
            return
        }

        // Other rates/formats: convert (rare — taps normally run at 48 kHz).
        if converter == nil || converter?.inputFormat != format {
            converter = AVAudioConverter(from: format, to: monoFormat)
        }
        guard let converter else { return }
        let capacity = AVAudioFrameCount(Double(frames) * AudioFrame.sampleRate / format.sampleRate) + 64
        guard let out = AVAudioPCMBuffer(pcmFormat: monoFormat, frameCapacity: capacity) else { return }
        var fed = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if fed { status.pointee = .noDataNow; return nil }
            fed = true
            status.pointee = .haveData
            return buffer
        }
        guard error == nil, let channel = out.floatChannelData?[0], out.frameLength > 0 else { return }
        target.pushReference(UnsafeBufferPointer(start: channel, count: Int(out.frameLength)), hostTimeNanos: hostTimeNanos)
    }
}
