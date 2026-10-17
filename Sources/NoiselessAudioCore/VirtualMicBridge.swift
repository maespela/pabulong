import Foundation
import NoiselessAudioCoreC

public struct BridgeHealth: Codable, Equatable, Sendable {
    public var availableReadFrames: Int
    public var availableWriteFrames: Int
    public var underrunCount: UInt64
    public var overrunCount: UInt64
    public var producerAttached: Bool
    public var consumerRunningCount: Int

    public init(
        availableReadFrames: Int = 0,
        availableWriteFrames: Int = 0,
        underrunCount: UInt64 = 0,
        overrunCount: UInt64 = 0,
        producerAttached: Bool = false,
        consumerRunningCount: Int = 0
    ) {
        self.availableReadFrames = availableReadFrames
        self.availableWriteFrames = availableWriteFrames
        self.underrunCount = underrunCount
        self.overrunCount = overrunCount
        self.producerAttached = producerAttached
        self.consumerRunningCount = consumerRunningCount
    }
}

public final class VirtualMicBridge {
    public static let defaultFrameCapacity = 16

    private let ring: OpaquePointer
    private var underruns: UInt64 = 0
    private var overruns: UInt64 = 0
    private var attached = false

    public init(frameCapacity: Int = VirtualMicBridge.defaultFrameCapacity) throws {
        guard let ring = nl_ring_buffer_create(UInt32(frameCapacity), UInt32(AudioFrame.frameLength)) else {
            throw NoiseProcessorError.allocationFailed
        }
        self.ring = ring
    }

    deinit {
        nl_ring_buffer_destroy(ring)
    }

    public func attachProducer() {
        attached = true
    }

    public func detachProducer() {
        attached = false
    }

    @discardableResult
    public func write(_ frame: AudioFrame) -> Bool {
        var samples = frame.samples
        let didWrite = samples.withUnsafeMutableBufferPointer { buffer in
            nl_ring_buffer_write(ring, buffer.baseAddress, UInt32(AudioFrame.frameLength))
        }
        if !didWrite {
            overruns += 1
        }
        return didWrite
    }

    @discardableResult
    public func writeLatest(_ frame: AudioFrame) -> Bool {
        if write(frame) {
            return true
        }
        _ = readOrSilence()
        return write(frame)
    }

    public func readOrSilence() -> AudioFrame {
        var samples = Array(repeating: Float(0), count: AudioFrame.frameLength)
        let didRead = samples.withUnsafeMutableBufferPointer { buffer in
            nl_ring_buffer_read(ring, buffer.baseAddress, UInt32(AudioFrame.frameLength))
        }
        if !didRead {
            underruns += 1
        }
        return AudioFrame(samples: samples)
    }

    public func clear() {
        nl_ring_buffer_clear(ring)
    }

    public var health: BridgeHealth {
        BridgeHealth(
            availableReadFrames: Int(nl_ring_buffer_available_read(ring)),
            availableWriteFrames: Int(nl_ring_buffer_available_write(ring)),
            underrunCount: underruns,
            overrunCount: overruns,
            producerAttached: attached,
            consumerRunningCount: 0
        )
    }
}

public final class SharedVirtualMicBridge {
    public static let defaultName = String(cString: nl_shared_ring_default_name())
    public static let defaultFrameCapacity = 32

    private let ring: OpaquePointer

    public init(
        name: String = SharedVirtualMicBridge.defaultName,
        create: Bool = true,
        frameCapacity: Int = SharedVirtualMicBridge.defaultFrameCapacity
    ) throws {
        guard let opened = nl_shared_ring_open(name, create, UInt32(frameCapacity), UInt32(AudioFrame.frameLength)) else {
            throw NoiseProcessorError.allocationFailed
        }
        ring = opened
    }

    deinit {
        nl_shared_ring_close(ring)
    }

    public func attachProducer() {
        nl_shared_ring_set_producer_attached(ring, true)
    }

    public func detachProducer() {
        nl_shared_ring_set_producer_attached(ring, false)
    }

    @discardableResult
    public func write(_ frame: AudioFrame) -> Bool {
        var samples = frame.samples
        return samples.withUnsafeMutableBufferPointer { buffer in
            nl_shared_ring_write(ring, buffer.baseAddress, UInt32(AudioFrame.frameLength))
        }
    }

    @discardableResult
    public func writeLatest(_ frame: AudioFrame) -> Bool {
        var samples = frame.samples
        return samples.withUnsafeMutableBufferPointer { buffer in
            nl_shared_ring_write_latest(ring, buffer.baseAddress, UInt32(AudioFrame.frameLength))
        }
    }

    public func readOrSilence() -> AudioFrame {
        var samples = Array(repeating: Float(0), count: AudioFrame.frameLength)
        _ = samples.withUnsafeMutableBufferPointer { buffer in
            nl_shared_ring_read(ring, buffer.baseAddress, UInt32(AudioFrame.frameLength))
        }
        return AudioFrame(samples: samples)
    }

    public func clear() {
        nl_shared_ring_clear(ring)
    }

    public var health: BridgeHealth {
        let value = nl_shared_ring_health(ring)
        return BridgeHealth(
            availableReadFrames: Int(value.available_read_frames),
            availableWriteFrames: Int(value.available_write_frames),
            underrunCount: value.underrun_count,
            overrunCount: value.overrun_count,
            producerAttached: value.producer_attached,
            consumerRunningCount: Int(value.consumer_running_count)
        )
    }
}
