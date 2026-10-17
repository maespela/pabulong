@preconcurrency import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation
import os

public enum AudioPipelineError: Error, Equatable {
    case inputFormatUnavailable
    case converterUnavailable
    case alreadyRunning
    case inputDeviceUnavailable
    case wouldCaptureVirtualDevice
    case deviceBindingFailed(OSStatus)
    case producerAlreadyRunning
    case captureUnitUnavailable(OSStatus)
}

public protocol AudioFrameSink: AnyObject {
    func attachProducer()
    func detachProducer()
    @discardableResult func write(_ frame: AudioFrame) -> Bool
    @discardableResult func writeLatest(_ frame: AudioFrame) -> Bool
    func clear()
}

extension VirtualMicBridge: AudioFrameSink {}
extension SharedVirtualMicBridge: AudioFrameSink {}

/// Captures the physical microphone via a low-level AUHAL input unit (small
/// ~10 ms buffers), runs each frame through the noise processor, and feeds the
/// shared virtual-mic ring.
///
/// AUHAL is used deliberately instead of `AVAudioEngine.installTap`: the engine
/// tap batches input into ~100 ms buffers, which both adds latency and, at the
/// ring's frame rate, arrives in bursts. AUHAL delivers the device's native IO
/// buffer (a few hundred frames), so the producer feeds the ring smoothly and
/// at low latency.
public final class AudioPipeline: @unchecked Sendable {
    public private(set) var isRunning = false
    public var processedFrameHandler: (@Sendable (AudioFrame) -> Void)?
    /// Receives each raw mic frame before processing (used by the speaker echo test).
    public var inputFrameHandler: (@Sendable (AudioFrame) -> Void)?

    /// Safety valve only. In steady state the accumulator holds < 1 frame; this
    /// bound just prevents unbounded growth if the processing queue ever stalls.
    private static let maxBacklogFrames = AudioFrame.frameLength * 25
    /// Scratch sizing hint.
    private static let ioBufferFrameSize: UInt32 = 512
    /// Requested device IO buffer size in frames. Smaller = lower capture
    /// latency; clamped to the device's supported range at runtime.
    private static let targetIOBufferFrames: UInt32 = 128

    private let processor: NoiseProcessor
    private let bridge: AudioFrameSink
    private let deviceManager: DeviceManager
    private let preferredInputUID: String?
    private let processingQueue = DispatchQueue(label: "com.nextmatterpro.pabulong.audio-pipeline", qos: .userInteractive)

    // Target format the processor and ring operate in.
    private var captureUnit: AudioUnit?
    private var renderScratch: UnsafeMutablePointer<Float>?
    private var renderScratchCapacity: Int = 0
    // Device-native capture format and converter to 48 kHz mono. AUHAL cannot do
    // sample-rate conversion on input (e.g. 16 kHz Bluetooth mics), so we capture
    // native and resample in software on the processing queue.
    private var captureFormat: AVAudioFormat?
    private var captureChannels: Int = 1
    private let outputFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: AudioFrame.sampleRate,
        channels: 1,
        interleaved: true
    )!
    private var converter: AVAudioConverter?

    // Only ever touched on `processingQueue`.
    private var frameAccumulator: [Float] = []
    /// Host time (ns) at which `frameAccumulator[0]` was captured.
    private var accumulatorStartNanos: Double = 0
    private static let nanosPerSample = 1e9 / AudioFrame.sampleRate
    // When true, the mic is passed through unprocessed (suppression disabled).
    private let bypass = OSAllocatedUnfairLock(initialState: false)
    // Held for the lifetime of a running pipeline to keep this the sole producer.
    private var producerGuard: ProducerGuard?
    private var didAttachProducer = false

    /// Enables/disables raw pass-through (no noise suppression) while the
    /// pipeline keeps feeding the virtual mic. Safe to call while running.
    public func setBypass(_ enabled: Bool) {
        bypass.withLock { $0 = enabled }
    }

    public init(
        processor: NoiseProcessor,
        bridge: AudioFrameSink,
        deviceManager: DeviceManager = DeviceManager(),
        preferredInputUID: String? = nil
    ) {
        self.processor = processor
        self.bridge = bridge
        self.deviceManager = deviceManager
        self.preferredInputUID = preferredInputUID
    }

    deinit {
        renderScratch?.deallocate()
    }

    public func start() throws {
        guard !isRunning else {
            throw AudioPipelineError.alreadyRunning
        }

        // Become the sole producer before touching the shared ring. If another
        // process already owns it, fail without disturbing its stream.
        guard let producerGuard = ProducerGuard() else {
            throw AudioPipelineError.producerAlreadyRunning
        }
        self.producerGuard = producerGuard

        do {
            try startCapture()
        } catch {
            teardownCaptureUnit()
            detachProducerIfNeeded(clear: true)
            self.producerGuard = nil
            throw error
        }

        isRunning = true
    }

    private func startCapture() throws {
        // Resolve an explicit *physical* capture device — never our own virtual
        // mic, which would feed the processed signal straight back in.
        guard let device = deviceManager.resolvePhysicalInputDevice(uid: preferredInputUID) else {
            throw AudioPipelineError.inputDeviceUnavailable
        }
        guard device.uid != DeviceManager.virtualDeviceUID else {
            throw AudioPipelineError.wouldCaptureVirtualDevice
        }

        try processor.prepare(sampleRate: AudioFrame.sampleRate, channels: 1)
        bridge.clear()

        try buildCaptureUnit(deviceID: device.id)

        guard let unit = captureUnit else {
            throw AudioPipelineError.captureUnitUnavailable(kAudioUnitErr_Uninitialized)
        }
        let startStatus = AudioOutputUnitStart(unit)
        guard startStatus == noErr else {
            throw AudioPipelineError.captureUnitUnavailable(startStatus)
        }
        applyLowLatencyBuffer(deviceID: device.id)
        bridge.attachProducer()
        didAttachProducer = true
    }

    private func buildCaptureUnit(deviceID: AudioDeviceID) throws {
        // Noiseless must remain input-only. VoiceProcessingIO is duplex and can
        // put the physical output into a communications/ducked state even when
        // its render callback emits silence, making all speaker audio quieter.
        try buildUnit(deviceID: deviceID)
    }

    private func buildUnit(deviceID: AudioDeviceID) throws {
        var description = AudioComponentDescription(
            componentType: kAudioUnitType_Output,
            componentSubType: kAudioUnitSubType_HALOutput,
            componentManufacturer: kAudioUnitManufacturer_Apple,
            componentFlags: 0,
            componentFlagsMask: 0
        )
        guard let component = AudioComponentFindNext(nil, &description) else {
            throw AudioPipelineError.captureUnitUnavailable(kAudioUnitErr_FailedInitialization)
        }
        var unit: AudioUnit?
        try check(AudioComponentInstanceNew(component, &unit), "InstanceNew")
        guard let unit else {
            throw AudioPipelineError.captureUnitUnavailable(kAudioUnitErr_FailedInitialization)
        }
        captureUnit = unit

        // Enable input (bus 1) and explicitly disable output (bus 0).
        var enable: UInt32 = 1
        try check(AudioUnitSetProperty(unit, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Input, 1, &enable, 4), "EnableInput")
        var disable: UInt32 = 0
        try check(AudioUnitSetProperty(unit, kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Output, 0, &disable, 4), "DisableOutput")

        // Bind the specific physical device.
        var mutableDevice = deviceID
        try check(AudioUnitSetProperty(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &mutableDevice, 4), "CurrentDevice")

        // Read the device's native input format. AUHAL cannot sample-rate
        // convert on input, so we capture at this rate/channel count (float) and
        // resample to 48 kHz mono ourselves.
        var nativeFormat = AudioStreamBasicDescription()
        var nativeSize = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
        try check(AudioUnitGetProperty(unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 1, &nativeFormat, &nativeSize), "GetNativeFormat")
        let nativeRate = nativeFormat.mSampleRate > 0 ? nativeFormat.mSampleRate : AudioFrame.sampleRate
        let nativeChannels = max(1, Int(nativeFormat.mChannelsPerFrame))
        captureChannels = nativeChannels

        // Client format = native rate/channels, interleaved float32. AUHAL only
        // converts sample encoding to float (cheap), never the rate.
        var format = AudioStreamBasicDescription(
            mSampleRate: nativeRate,
            mFormatID: kAudioFormatLinearPCM,
            mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked | kAudioFormatFlagsNativeEndian,
            mBytesPerPacket: UInt32(4 * nativeChannels),
            mFramesPerPacket: 1,
            mBytesPerFrame: UInt32(4 * nativeChannels),
            mChannelsPerFrame: UInt32(nativeChannels),
            mBitsPerChannel: 32,
            mReserved: 0
        )
        try check(AudioUnitSetProperty(unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 1, &format, UInt32(MemoryLayout<AudioStreamBasicDescription>.size)), "StreamFormat")

        guard let capFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32, sampleRate: nativeRate, channels: AVAudioChannelCount(nativeChannels), interleaved: true) else {
            throw AudioPipelineError.converterUnavailable
        }
        captureFormat = capFormat
        // No resampling needed when the device is already 48 kHz mono.
        if nativeRate == AudioFrame.sampleRate && nativeChannels == 1 {
            converter = nil
        } else {
            guard let conv = AVAudioConverter(from: capFormat, to: outputFormat) else {
                throw AudioPipelineError.converterUnavailable
            }
            converter = conv
        }
        try finishCaptureUnit(unit)
    }

    /// Shared tail: preallocate the render scratch, install the mic input
    /// callback, and initialize the unit.
    private func finishCaptureUnit(_ unit: AudioUnit) throws {
        let channels = max(1, captureChannels)
        var maxFrames: UInt32 = 4096
        var maxSize = UInt32(MemoryLayout<UInt32>.size)
        _ = AudioUnitGetProperty(unit, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0, &maxFrames, &maxSize)
        allocateRenderScratch(capacity: Int(max(maxFrames, AudioPipeline.ioBufferFrameSize)) * channels)

        var callback = AURenderCallbackStruct(
            inputProc: auhalInputCallback,
            inputProcRefCon: Unmanaged.passUnretained(self).toOpaque()
        )
        try check(AudioUnitSetProperty(unit, kAudioOutputUnitProperty_SetInputCallback, kAudioUnitScope_Global, 0, &callback, UInt32(MemoryLayout<AURenderCallbackStruct>.size)), "InputCallback")

        try check(AudioUnitInitialize(unit), "Initialize")
    }

    /// Requests a small device IO buffer to cut capture latency. Best-effort:
    /// clamped to the device's supported range, and some devices (notably
    /// Bluetooth SCO headset mics) enforce a larger fixed size.
    private func applyLowLatencyBuffer(deviceID: AudioDeviceID) {
        var bufRange = AudioValueRange()
        var bufRangeSize = UInt32(MemoryLayout<AudioValueRange>.size)
        var rangeAddr = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyBufferFrameSizeRange,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var targetFrames = AudioPipeline.targetIOBufferFrames
        if AudioObjectGetPropertyData(deviceID, &rangeAddr, 0, nil, &bufRangeSize, &bufRange) == noErr {
            targetFrames = UInt32(min(max(Double(targetFrames), bufRange.mMinimum), bufRange.mMaximum))
        }
        var bufAddr = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyBufferFrameSize,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        _ = AudioObjectSetPropertyData(deviceID, &bufAddr, 0, nil, 4, &targetFrames)
    }

    private func allocateRenderScratch(capacity: Int) {
        renderScratch?.deallocate()
        renderScratch = UnsafeMutablePointer<Float>.allocate(capacity: capacity)
        renderScratchCapacity = capacity
    }

    private func check(_ status: OSStatus, _ what: String = "") throws {
        guard status == noErr else {
            throw AudioPipelineError.captureUnitUnavailable(status)
        }
    }

    public func stop() {
        guard isRunning || didAttachProducer || producerGuard != nil else { return }
        teardownCaptureUnit()
        detachProducerIfNeeded(clear: true)
        processingQueue.sync {
            frameAccumulator.removeAll(keepingCapacity: true)
        }
        processor.reset()
        producerGuard = nil
        isRunning = false
    }

    private func teardownCaptureUnit() {
        guard let unit = captureUnit else { return }
        AudioOutputUnitStop(unit)
        AudioUnitUninitialize(unit)
        AudioComponentInstanceDispose(unit)
        captureUnit = nil
    }

    private func detachProducerIfNeeded(clear: Bool) {
        guard didAttachProducer else { return }
        bridge.detachProducer()
        if clear {
            bridge.clear()
        }
        didAttachProducer = false
    }

    // MARK: - Real-time render

    /// Called on the AUHAL real-time thread. Pulls the input into scratch, copies
    /// it out, and hands it to the processing queue. Kept minimal.
    fileprivate func renderInput(
        ioActionFlags: UnsafeMutablePointer<AudioUnitRenderActionFlags>,
        timestamp: UnsafePointer<AudioTimeStamp>,
        frames: UInt32
    ) -> OSStatus {
        guard let unit = captureUnit, let scratch = renderScratch, Int(frames) <= renderScratchCapacity else {
            return noErr
        }

        let channels = max(1, captureChannels)
        var bufferList = AudioBufferList(
            mNumberBuffers: 1,
            mBuffers: AudioBuffer(
                mNumberChannels: UInt32(channels),
                mDataByteSize: frames * UInt32(channels) * 4,
                mData: UnsafeMutableRawPointer(scratch)
            )
        )
        let status = AudioUnitRender(unit, ioActionFlags, timestamp, 1, frames, &bufferList)
        guard status == noErr else { return status }

        let samples = Array(UnsafeBufferPointer(start: scratch, count: Int(frames) * channels))
        // Capture time of the first sample, for aligning the echo reference.
        let stamp = timestamp.pointee
        let captureNanos = hostTimeNanos(stamp.mFlags.contains(.hostTimeValid) ? stamp.mHostTime : AudioGetCurrentHostTime())
        processingQueue.async { [weak self] in
            self?.consume(samples, captureNanos: captureNanos)
        }
        return noErr
    }

    /// Resamples device-native interleaved samples to 48 kHz mono.
    private func convertToOutput(_ native: [Float]) -> [Float] {
        guard let converter, let capFormat = captureFormat else { return native }
        let frameCount = AVAudioFrameCount(native.count / max(captureChannels, 1))
        guard frameCount > 0,
              let inBuf = AVAudioPCMBuffer(pcmFormat: capFormat, frameCapacity: frameCount) else { return [] }
        inBuf.frameLength = frameCount
        native.withUnsafeBytes { raw in
            if let dst = inBuf.mutableAudioBufferList.pointee.mBuffers.mData, let src = raw.baseAddress {
                memcpy(dst, src, raw.count)
            }
        }
        let ratio = outputFormat.sampleRate / capFormat.sampleRate
        let outCapacity = AVAudioFrameCount(Double(frameCount) * ratio) + 64
        guard let outBuf = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: outCapacity) else { return [] }
        let state = ConverterInputState(buffer: inBuf)
        var error: NSError?
        converter.convert(to: outBuf, error: &error) { _, status in
            if state.didProvideInput { status.pointee = .noDataNow; return nil }
            state.didProvideInput = true
            status.pointee = .haveData
            return state.buffer
        }
        guard error == nil, let channel = outBuf.floatChannelData?[0] else { return [] }
        return Array(UnsafeBufferPointer(start: channel, count: Int(outBuf.frameLength)))
    }

    private func consume(_ nativeSamples: [Float], captureNanos: UInt64) {
        let samples = convertToOutput(nativeSamples)
        // Anchor the accumulator's first sample to this chunk's capture time.
        let pending = Double(frameAccumulator.count) * AudioPipeline.nanosPerSample
        accumulatorStartNanos = Double(captureNanos) - pending
        frameAccumulator.append(contentsOf: samples)
        // Safety valve: only trims if the queue has stalled badly.
        if frameAccumulator.count > AudioPipeline.maxBacklogFrames {
            let dropped = frameAccumulator.count - AudioPipeline.maxBacklogFrames
            frameAccumulator.removeFirst(dropped)
            accumulatorStartNanos += Double(dropped) * AudioPipeline.nanosPerSample
        }
        while frameAccumulator.count >= AudioFrame.frameLength {
            let frameSamples = Array(frameAccumulator.prefix(AudioFrame.frameLength))
            frameAccumulator.removeFirst(AudioFrame.frameLength)
            let frameNanos = UInt64(max(0, accumulatorStartNanos))
            accumulatorStartNanos += Double(AudioFrame.frameLength) * AudioPipeline.nanosPerSample

            let inputFrame = AudioFrame(samples: frameSamples)
            inputFrameHandler?(inputFrame)
            let outputFrame: AudioFrame
            if bypass.withLock({ $0 }) {
                outputFrame = inputFrame
            } else {
                do {
                    outputFrame = try processor.process(inputFrame, hostTimeNanos: frameNanos)
                } catch {
                    outputFrame = inputFrame
                }
            }
            processedFrameHandler?(outputFrame)
            _ = bridge.writeLatest(outputFrame)
        }
    }
}

private let auhalInputCallback: AURenderCallback = { (inRefCon, ioActionFlags, inTimeStamp, _, inNumberFrames, _) -> OSStatus in
    let pipeline = Unmanaged<AudioPipeline>.fromOpaque(inRefCon).takeUnretainedValue()
    return pipeline.renderInput(ioActionFlags: ioActionFlags, timestamp: inTimeStamp, frames: inNumberFrames)
}

private final class ConverterInputState: @unchecked Sendable {
    var didProvideInput = false
    let buffer: AVAudioPCMBuffer
    init(buffer: AVAudioPCMBuffer) { self.buffer = buffer }
}
