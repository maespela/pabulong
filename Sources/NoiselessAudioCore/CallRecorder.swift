@preconcurrency import AVFoundation
@preconcurrency import CoreMedia
import Foundation

public enum CallRecorderError: Error {
    case missingAudioPath(AudioTrackRole)
    case fileOpenFailed(AudioTrackRole)
    case sampleBufferFormatUnavailable
}

public final class CallRecorder: @unchecked Sendable {
    /// Voice recordings use 16 kHz PCM to keep local files compact while
    /// preserving the speech band used for calls.
    public static let storageSampleRate: Double = 16_000

    private let queue = DispatchQueue(label: "com.nextmatterpro.pabulong.call-recorder", qos: .utility)
    /// In-memory processing/buffer format: 16 kHz mono float.
    private let format = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: CallRecorder.storageSampleRate,
        channels: 1,
        interleaved: false
    )!
    /// Format of the live mic frames arriving from the pipeline (48 kHz mono).
    private let captureFrameFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: AudioFrame.sampleRate,
        channels: 1,
        interleaved: false
    )!
    /// On-disk format: 16 kHz 16-bit PCM (6× smaller than 48 kHz float32).
    private let fileSettings: [String: Any] = [
        AVFormatIDKey: kAudioFormatLinearPCM,
        AVSampleRateKey: CallRecorder.storageSampleRate,
        AVNumberOfChannelsKey: 1,
        AVLinearPCMBitDepthKey: 16,
        AVLinearPCMIsFloatKey: false,
        AVLinearPCMIsBigEndianKey: false,
        AVLinearPCMIsNonInterleaved: false
    ]
    /// Persistent 48 kHz→16 kHz converter for the mic frames; keeping one
    /// instance preserves the resampler's filter state across frames.
    private var localConverter: AVAudioConverter?

    private var localFile: AVAudioFile?
    private var remoteFile: AVAudioFile?
    private var isRecording = false
    private var remoteWriteFailed = false
    private var localURL: URL?
    private var remoteURL: URL?
    private var mixedURL: URL?

    public init() {}

    public func start(session: CallSession) throws {
        try queue.sync {
            try closeUnlocked()
            guard let localPath = session.localMicAudioPath else { throw CallRecorderError.missingAudioPath(.localMic) }
            guard let mixedPath = session.mixedPreviewAudioPath else { throw CallRecorderError.missingAudioPath(.mixedPreview) }
            localURL = URL(fileURLWithPath: localPath)
            mixedURL = URL(fileURLWithPath: mixedPath)
            localFile = try AVAudioFile(forWriting: localURL!, settings: fileSettings)
            if let remotePath = session.remoteSystemAudioPath {
                remoteURL = URL(fileURLWithPath: remotePath)
                remoteFile = try AVAudioFile(forWriting: remoteURL!, settings: fileSettings)
            }
            remoteWriteFailed = false
            isRecording = true
        }
    }

    public func recordLocalFrame(_ frame: AudioFrame) {
        queue.async { [weak self] in
            guard let self, self.isRecording else { return }
            guard let buffer = self.buffer(from: frame) else { return }
            do {
                try self.localFile?.write(from: buffer)
            } catch {
                self.isRecording = false
            }
        }
    }

    /// Records an already-decoded audio buffer (from the Core Audio tap) as the
    /// remote/call track, converting to the recording format.
    public func recordRemoteBuffer(_ buffer: AVAudioPCMBuffer) {
        guard let converted = Self.convert(buffer, to: format) else {
            queue.async { [weak self] in self?.remoteWriteFailed = true }
            return
        }
        let sendableBuffer = SendableAudioBuffer(converted)
        queue.async { [weak self, sendableBuffer] in
            guard let self, self.isRecording, !self.remoteWriteFailed else { return }
            do {
                try self.remoteFile?.write(from: sendableBuffer.buffer)
            } catch {
                self.remoteWriteFailed = true
            }
        }
    }

    public func recordRemoteSampleBuffer(_ sampleBuffer: CMSampleBuffer) {
        guard let buffer = Self.audioBuffer(from: sampleBuffer, targetFormat: format) else {
            queue.async { [weak self] in
                self?.remoteWriteFailed = true
            }
            return
        }
        let sendableBuffer = SendableAudioBuffer(buffer)
        queue.async { [weak self, sendableBuffer] in
            guard let self, self.isRecording, !self.remoteWriteFailed else { return }
            do {
                try self.remoteFile?.write(from: sendableBuffer.buffer)
            } catch {
                self.remoteWriteFailed = true
            }
        }
    }

    public func stop() {
        queue.sync {
            isRecording = false
            localFile = nil
            remoteFile = nil
            try? buildMixedPreviewUnlocked()
            clearUnlocked()
        }
    }

    public var didLoseRemoteAudio: Bool {
        queue.sync { remoteWriteFailed }
    }

    private func closeUnlocked() throws {
        isRecording = false
        localFile = nil
        remoteFile = nil
        clearUnlocked()
    }

    private func clearUnlocked() {
        localFile = nil
        remoteFile = nil
        localURL = nil
        remoteURL = nil
        mixedURL = nil
        localConverter = nil
    }

    /// Builds a 16 kHz mono buffer from a 48 kHz mic frame, resampling with a
    /// persistent converter so filter state carries across consecutive frames.
    private func buffer(from frame: AudioFrame) -> AVAudioPCMBuffer? {
        guard let source = AVAudioPCMBuffer(pcmFormat: captureFrameFormat, frameCapacity: AVAudioFrameCount(AudioFrame.frameLength)),
              let sourceChannel = source.floatChannelData?[0] else {
            return nil
        }
        source.frameLength = AVAudioFrameCount(AudioFrame.frameLength)
        frame.samples.withUnsafeBufferPointer { input in
            if let baseAddress = input.baseAddress {
                sourceChannel.update(from: baseAddress, count: AudioFrame.frameLength)
            }
        }

        if localConverter == nil {
            localConverter = AVAudioConverter(from: captureFrameFormat, to: format)
        }
        guard let converter = localConverter else { return nil }
        let ratio = format.sampleRate / captureFrameFormat.sampleRate
        let capacity = AVAudioFrameCount((Double(AudioFrame.frameLength) * ratio).rounded(.up)) + 16
        guard let output = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        let state = ConverterInputState()
        let sendableSource = SendableAudioBuffer(source)
        var conversionError: NSError?
        converter.convert(to: output, error: &conversionError) { _, status in
            if state.didFeed {
                // Keep the converter alive for the next frame (don't end the stream).
                status.pointee = .noDataNow
                return nil
            }
            state.didFeed = true
            status.pointee = .haveData
            return sendableSource.buffer
        }
        return conversionError == nil ? output : nil
    }

    private static func audioBuffer(from sampleBuffer: CMSampleBuffer, targetFormat: AVAudioFormat) -> AVAudioPCMBuffer? {
        guard let description = CMSampleBufferGetFormatDescription(sampleBuffer),
              let streamDescriptionPointer = CMAudioFormatDescriptionGetStreamBasicDescription(description) else {
            return nil
        }
        var streamDescription = streamDescriptionPointer.pointee
        guard let format = AVAudioFormat(streamDescription: &streamDescription) else {
            return nil
        }
        let frameCount = CMSampleBufferGetNumSamples(sampleBuffer)
        guard frameCount > 0,
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frameCount)) else {
            return nil
        }
        buffer.frameLength = AVAudioFrameCount(frameCount)
        let status = CMSampleBufferCopyPCMDataIntoAudioBufferList(
            sampleBuffer,
            at: 0,
            frameCount: Int32(frameCount),
            into: buffer.mutableAudioBufferList
        )
        guard status == noErr else { return nil }
        return convert(buffer, to: targetFormat)
    }

    private static func convert(_ source: AVAudioPCMBuffer, to targetFormat: AVAudioFormat) -> AVAudioPCMBuffer? {
        if source.format.sampleRate == targetFormat.sampleRate,
           source.format.channelCount == targetFormat.channelCount,
           source.format.commonFormat == targetFormat.commonFormat,
           source.format.isInterleaved == targetFormat.isInterleaved {
            return source
        }

        guard let converter = AVAudioConverter(from: source.format, to: targetFormat) else {
            return nil
        }
        let ratio = targetFormat.sampleRate / source.format.sampleRate
        let capacity = AVAudioFrameCount((Double(source.frameLength) * ratio).rounded(.up)) + 16
        guard let output = AVAudioPCMBuffer(pcmFormat: targetFormat, frameCapacity: capacity) else {
            return nil
        }
        let state = ConverterInputState()
        let sendableSource = SendableAudioBuffer(source)
        var conversionError: NSError?
        converter.convert(to: output, error: &conversionError) { _, status in
            if state.didFeed {
                status.pointee = .endOfStream
                return nil
            }
            state.didFeed = true
            status.pointee = .haveData
            return sendableSource.buffer
        }
        return conversionError == nil ? output : nil
    }

    private func buildMixedPreviewUnlocked() throws {
        guard let localURL, let mixedURL, FileManager.default.fileExists(atPath: localURL.path) else { return }
        let hasRemoteAudio = remoteURL.flatMap { audioFrameCount(at: $0) }.map { $0 > 0 } ?? false
        guard hasRemoteAudio else {
            try? FileManager.default.removeItem(at: mixedURL)
            try FileManager.default.copyItem(at: localURL, to: mixedURL)
            return
        }

        let localInput = try AVAudioFile(forReading: localURL)
        guard localInput.length > 0 else {
            try? FileManager.default.removeItem(at: mixedURL)
            try FileManager.default.copyItem(at: localURL, to: mixedURL)
            return
        }
        let remoteInput: AVAudioFile?
        if let remoteURL, hasRemoteAudio {
            remoteInput = try? AVAudioFile(forReading: remoteURL)
        } else {
            remoteInput = nil
        }

        let output = try AVAudioFile(forWriting: mixedURL, settings: fileSettings)
        let frameCapacity: AVAudioFrameCount = 4096
        let totalFrames = max(localInput.length, remoteInput?.length ?? 0)
        var written: AVAudioFramePosition = 0

        while written < totalFrames {
            let framesThisPass = AVAudioFrameCount(min(Int64(frameCapacity), totalFrames - written))
            guard let localBuffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: framesThisPass),
                  let outputBuffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: framesThisPass) else {
                return
            }
            try localInput.read(into: localBuffer, frameCount: framesThisPass)

            var remoteBuffer: AVAudioPCMBuffer?
            if let remoteInput,
               let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: framesThisPass) {
                try remoteInput.read(into: buffer, frameCount: framesThisPass)
                remoteBuffer = buffer
            }

            outputBuffer.frameLength = framesThisPass
            guard let out = outputBuffer.floatChannelData?[0] else { return }
            let local = localBuffer.floatChannelData?[0]
            let remote = remoteBuffer?.floatChannelData?[0]
            let localCount = Int(localBuffer.frameLength)
            let remoteCount = Int(remoteBuffer?.frameLength ?? 0)
            for index in 0..<Int(framesThisPass) {
                let localSample = index < localCount ? (local?[index] ?? 0) : 0
                let remoteSample = index < remoteCount ? (remote?[index] ?? 0) : 0
                out[index] = max(-0.98, min(0.98, (localSample * 0.8) + (remoteSample * 0.8)))
            }
            try output.write(from: outputBuffer)
            written += AVAudioFramePosition(framesThisPass)
        }
    }

    private func audioFrameCount(at url: URL) -> AVAudioFramePosition? {
        guard FileManager.default.fileExists(atPath: url.path),
              ((try? FileManager.default.attributesOfItem(atPath: url.path)[.size] as? UInt64) ?? 0) > 44 else {
            return nil
        }
        return (try? AVAudioFile(forReading: url))?.length
    }
}

private struct SendableAudioBuffer: @unchecked Sendable {
    let buffer: AVAudioPCMBuffer

    init(_ buffer: AVAudioPCMBuffer) {
        self.buffer = buffer
    }
}

private final class ConverterInputState: @unchecked Sendable {
    var didFeed = false
}
