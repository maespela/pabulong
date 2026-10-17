import Foundation

public struct DiagnosticsSnapshot: Codable, Equatable, Sendable {
    public var appVersion: String
    public var virtualDeviceName: String
    public var settings: AppSettings
    public var inputDevices: [String]
    public var bridge: BridgeHealth
    public var processor: ProcessorMetrics

    public init(
        appVersion: String = "0.1.0-dev",
        virtualDeviceName: String = "Pabulong Mic",
        settings: AppSettings,
        inputDevices: [String],
        bridge: BridgeHealth,
        processor: ProcessorMetrics
    ) {
        self.appVersion = appVersion
        self.virtualDeviceName = virtualDeviceName
        self.settings = settings
        self.inputDevices = inputDevices
        self.bridge = bridge
        self.processor = processor
    }

    public func jsonData() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return try encoder.encode(self)
    }
}
