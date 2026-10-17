import AudioToolbox
import CoreAudio
import Foundation

public struct AudioInputDevice: Equatable, Identifiable, Sendable {
    public let id: AudioDeviceID
    public let uid: String
    public let name: String
    public let isDefault: Bool

    public init(id: AudioDeviceID, uid: String, name: String, isDefault: Bool) {
        self.id = id
        self.uid = uid
        self.name = name
        self.isDefault = isDefault
    }
}

public final class DeviceManager {
    /// UID of the Noiseless virtual microphone published by the HAL plug-in.
    /// Kept in sync with the device UID in `NoiselessHALPlugin.c`.
    public static let virtualDeviceUID = "com.nextmatterpro.pabulong.mic.device"

    public init() {}

    public func listInputDevices() -> [AudioInputDevice] {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDevices,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var dataSize: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &dataSize) == noErr else {
            return []
        }

        let count = Int(dataSize) / MemoryLayout<AudioDeviceID>.size
        var deviceIDs = Array(repeating: AudioDeviceID(), count: count)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &dataSize, &deviceIDs) == noErr else {
            return []
        }

        let defaultID = defaultInputDeviceID()
        return deviceIDs.compactMap { id in
            guard hasInputStreams(deviceID: id) else { return nil }
            return AudioInputDevice(
                id: id,
                uid: stringProperty(kAudioDevicePropertyDeviceUID, deviceID: id) ?? "\(id)",
                name: stringProperty(kAudioObjectPropertyName, deviceID: id) ?? "Input \(id)",
                isDefault: id == defaultID
            )
        }
        .sorted { lhs, rhs in
            if lhs.isDefault != rhs.isDefault { return lhs.isDefault }
            return lhs.name.localizedCaseInsensitiveCompare(rhs.name) == .orderedAscending
        }
    }

    public func resolveInputDevice(uid: String?) -> AudioInputDevice? {
        let devices = listInputDevices()
        if let uid, let selected = devices.first(where: { $0.uid == uid }) {
            return selected
        }
        return devices.first(where: \.isDefault) ?? devices.first
    }

    /// Resolves a *physical* capture device, never the Noiseless virtual mic.
    ///
    /// The app must never capture its own virtual output: doing so creates an
    /// audio feedback loop (driver output -> engine input -> processing ->
    /// driver output) that saturates coreaudiod and the host process. Any
    /// request for the virtual device — including a stale saved selection or a
    /// system default that has been switched to `Noiseless Mic` — is ignored
    /// in favour of a real input.
    public func resolvePhysicalInputDevice(uid: String?) -> AudioInputDevice? {
        let devices = listInputDevices().filter { $0.uid != DeviceManager.virtualDeviceUID }
        if let uid, uid != DeviceManager.virtualDeviceUID,
           let selected = devices.first(where: { $0.uid == uid }) {
            return selected
        }
        return devices.first(where: \.isDefault) ?? devices.first
    }

    /// True when the system default input device is the Noiseless virtual mic.
    /// As the default it is consumed continuously, keeping the physical mic on.
    public func isVirtualDeviceDefaultInput() -> Bool {
        guard let id = defaultInputDeviceID() else { return false }
        return stringProperty(kAudioDevicePropertyDeviceUID, deviceID: id) == DeviceManager.virtualDeviceUID
    }

    /// Sets the system default input to a physical device (the preferred one if
    /// given, otherwise a sensible physical default). Returns true on success.
    @discardableResult
    public func moveDefaultInputToPhysical(preferredUID: String?) -> Bool {
        guard let device = resolvePhysicalInputDevice(uid: preferredUID) else { return false }
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var id = device.id
        let status = AudioObjectSetPropertyData(
            AudioObjectID(kAudioObjectSystemObject), &address, 0, nil,
            UInt32(MemoryLayout<AudioDeviceID>.size), &id
        )
        return status == noErr
    }

    private func defaultInputDeviceID() -> AudioDeviceID? {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyDefaultInputDevice,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var id = AudioDeviceID()
        var size = UInt32(MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &id) == noErr else {
            return nil
        }
        return id
    }

    private func hasInputStreams(deviceID: AudioDeviceID) -> Bool {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioDevicePropertyStreamConfiguration,
            mScope: kAudioDevicePropertyScopeInput,
            mElement: kAudioObjectPropertyElementMain
        )
        var dataSize: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(deviceID, &address, 0, nil, &dataSize) == noErr else {
            return false
        }
        let bufferList = UnsafeMutableRawPointer.allocate(byteCount: Int(dataSize), alignment: MemoryLayout<AudioBufferList>.alignment)
        defer { bufferList.deallocate() }
        guard AudioObjectGetPropertyData(deviceID, &address, 0, nil, &dataSize, bufferList) == noErr else {
            return false
        }
        let audioBufferList = bufferList.assumingMemoryBound(to: AudioBufferList.self)
        let buffers = UnsafeMutableAudioBufferListPointer(audioBufferList)
        return buffers.contains { $0.mNumberChannels > 0 }
    }

    private func stringProperty(_ selector: AudioObjectPropertySelector, deviceID: AudioDeviceID) -> String? {
        var address = AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        var value: CFString = "" as CFString
        var size = UInt32(MemoryLayout<CFString>.size)
        let status = withUnsafeMutablePointer(to: &value) { pointer in
            AudioObjectGetPropertyData(deviceID, &address, 0, nil, &size, pointer)
        }
        guard status == noErr else { return nil }
        return value as String
    }
}
