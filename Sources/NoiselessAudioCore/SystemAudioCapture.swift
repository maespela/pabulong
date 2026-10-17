@preconcurrency import AVFoundation
import CoreAudio
import Foundation

/// Captures the *other side* of a call — the audio other apps play out (the
/// meeting participants' voices) — and delivers it as 16/48 kHz float buffers.
public protocol SystemAudioCapturing: AnyObject, Sendable {
    var onAudioBuffer: (@Sendable (AVAudioPCMBuffer) -> Void)? { get set }
    func start() async throws
    func stop() async
}

public enum SystemAudioCaptureError: Error {
    case unavailable
    case tapCreationFailed(OSStatus)
    case aggregateCreationFailed(OSStatus)
    case ioProcFailed(OSStatus)
    case formatUnavailable
}

/// System-audio capture via Core Audio process taps (macOS 14.2+). Unlike
/// ScreenCaptureKit this needs **no Screen Recording permission** and uses no
/// GPU — it's a pure audio tap on the system output mix.
@available(macOS 14.2, *)
public final class CoreAudioTapCapture: SystemAudioCapturing, @unchecked Sendable {
    public var onAudioBuffer: (@Sendable (AVAudioPCMBuffer) -> Void)?
    /// Same audio plus the host time (ns) at which its first sample was played.
    public var onTimedAudioBuffer: (@Sendable (AVAudioPCMBuffer, UInt64) -> Void)?

    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var ioProcID: AudioDeviceIOProcID?
    private var tapFormat: AVAudioFormat?
    private let ioQueue = DispatchQueue(label: "com.nextmatterpro.pabulong.coreaudio-tap", qos: .userInitiated)

    public init() {}

    public func start() async throws {
        // Tap the whole system output mix, excluding nothing (we produce no audio).
        let description = CATapDescription(stereoGlobalTapButExcludeProcesses: [])
        description.name = "Pabulong Call Tap"
        description.isPrivate = true
        description.muteBehavior = .unmuted   // the user still hears the call

        var tap = AudioObjectID(kAudioObjectUnknown)
        let tapStatus = AudioHardwareCreateProcessTap(description, &tap)
        guard tapStatus == noErr, tap != kAudioObjectUnknown else {
            throw SystemAudioCaptureError.tapCreationFailed(tapStatus)
        }
        tapID = tap

        let tapUID = try stringProperty(tapID, kAudioTapPropertyUID)
        tapFormat = try readTapFormat(tapID)

        // Build a private aggregate device that includes the tap so we can run an IOProc.
        let aggregateUID = UUID().uuidString
        let aggregateDescription: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Pabulong Tap Device",
            kAudioAggregateDeviceUIDKey: aggregateUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: true,
            kAudioAggregateDeviceTapListKey: [[
                kAudioSubTapUIDKey: tapUID,
                kAudioSubTapDriftCompensationKey: true,
            ]],
        ]
        var aggregate = AudioObjectID(kAudioObjectUnknown)
        let aggStatus = AudioHardwareCreateAggregateDevice(aggregateDescription as CFDictionary, &aggregate)
        guard aggStatus == noErr, aggregate != kAudioObjectUnknown else {
            throw SystemAudioCaptureError.aggregateCreationFailed(aggStatus)
        }
        aggregateID = aggregate

        var proc: AudioDeviceIOProcID?
        let procStatus = AudioDeviceCreateIOProcIDWithBlock(&proc, aggregateID, ioQueue) { [weak self] _, inInputData, inInputTime, _, _ in
            guard let self, let format = self.tapFormat,
                  let buffer = Self.makeBuffer(inInputData, format: format) else { return }
            self.onAudioBuffer?(buffer)
            if let timed = self.onTimedAudioBuffer {
                let stamp = inInputTime.pointee
                let host = stamp.mFlags.contains(.hostTimeValid) ? stamp.mHostTime : AudioGetCurrentHostTime()
                timed(buffer, hostTimeNanos(host))
            }
        }
        guard procStatus == noErr, let proc else {
            throw SystemAudioCaptureError.ioProcFailed(procStatus)
        }
        ioProcID = proc

        let startStatus = AudioDeviceStart(aggregateID, proc)
        guard startStatus == noErr else {
            throw SystemAudioCaptureError.ioProcFailed(startStatus)
        }
    }

    public func stop() async {
        if aggregateID != kAudioObjectUnknown, let proc = ioProcID {
            AudioDeviceStop(aggregateID, proc)
            AudioDeviceDestroyIOProcID(aggregateID, proc)
        }
        ioProcID = nil
        if aggregateID != kAudioObjectUnknown {
            AudioHardwareDestroyAggregateDevice(aggregateID)
            aggregateID = kAudioObjectUnknown
        }
        if tapID != kAudioObjectUnknown {
            AudioHardwareDestroyProcessTap(tapID)
            tapID = kAudioObjectUnknown
        }
    }

    private func stringProperty(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) throws -> CFString {
        var address = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var value: CFString = "" as CFString
        var size = UInt32(MemoryLayout<CFString>.size)
        let status = withUnsafeMutablePointer(to: &value) {
            AudioObjectGetPropertyData(object, &address, 0, nil, &size, $0)
        }
        guard status == noErr else { throw SystemAudioCaptureError.formatUnavailable }
        return value
    }

    private func readTapFormat(_ tap: AudioObjectID) throws -> AVAudioFormat {
        var address = AudioObjectPropertyAddress(mSelector: kAudioTapPropertyFormat, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var asbd = AudioStreamBasicDescription()
        var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        let status = AudioObjectGetPropertyData(tap, &address, 0, nil, &size, &asbd)
        guard status == noErr, let format = AVAudioFormat(streamDescription: &asbd) else {
            throw SystemAudioCaptureError.formatUnavailable
        }
        return format
    }

    private static func makeBuffer(_ inputData: UnsafePointer<AudioBufferList>, format: AVAudioFormat) -> AVAudioPCMBuffer? {
        let source = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inputData))
        guard let first = source.first else { return nil }
        let bytesPerFrame = max(1, Int(format.streamDescription.pointee.mBytesPerFrame))
        let frames = Int(first.mDataByteSize) / bytesPerFrame
        guard frames > 0,
              let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames)) else { return nil }
        buffer.frameLength = AVAudioFrameCount(frames)
        let destination = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)
        for i in 0..<min(source.count, destination.count) {
            if let src = source[i].mData, let dst = destination[i].mData {
                memcpy(dst, src, min(Int(source[i].mDataByteSize), Int(destination[i].mDataByteSize)))
            }
            destination[i].mDataByteSize = source[i].mDataByteSize
        }
        return buffer
    }
}

public final class UnavailableSystemAudioCapture: SystemAudioCapturing, @unchecked Sendable {
    public var onAudioBuffer: (@Sendable (AVAudioPCMBuffer) -> Void)?
    public init() {}
    public func start() async throws { throw SystemAudioCaptureError.unavailable }
    public func stop() async {}
}

public enum SystemAudioCaptureFactory {
    public static func make() -> SystemAudioCapturing {
        if #available(macOS 14.2, *) {
            return CoreAudioTapCapture()
        }
        return UnavailableSystemAudioCapture()
    }
}
