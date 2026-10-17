import Foundation

public struct AppSettings: Codable, Equatable, Sendable {
    public var processingEnabled: Bool
    public var selectedInputDeviceUID: String?
    public var suppressionMode: SuppressionMode
    public var launchAtLogin: Bool
    public var bypassEnabled: Bool
    public var processorType: NoiseProcessorType
    public var automaticCallRecordingEnabled: Bool
    public var captureSystemAudioEnabled: Bool
    public var localRecordingEnabled: Bool
    /// Removes the far end's voice when you use speakers instead of headphones.
    public var echoCancellationEnabled: Bool
    /// How strongly quieter background voices are turned down.
    public var voiceIsolation: VoiceIsolationLevel
    /// The user has confirmed they are responsible for getting consent from
    /// everyone on a call before recording it. No recording starts until then.
    public var recordingConsentAcknowledged: Bool
    /// Version of the Terms of Use and Privacy Notice the user agreed to (0 = never).
    public var acceptedTermsVersion: Int

    public init(
        processingEnabled: Bool = false,
        selectedInputDeviceUID: String? = nil,
        suppressionMode: SuppressionMode = .balanced,
        launchAtLogin: Bool = false,
        bypassEnabled: Bool = false,
        processorType: NoiseProcessorType = .rnnoise,
        automaticCallRecordingEnabled: Bool = false,
        captureSystemAudioEnabled: Bool = true,
        localRecordingEnabled: Bool = true,
        echoCancellationEnabled: Bool = true,
        voiceIsolation: VoiceIsolationLevel = .balanced,
        recordingConsentAcknowledged: Bool = false,
        acceptedTermsVersion: Int = 0
    ) {
        self.processingEnabled = processingEnabled
        self.selectedInputDeviceUID = selectedInputDeviceUID
        self.suppressionMode = suppressionMode
        self.launchAtLogin = launchAtLogin
        self.bypassEnabled = bypassEnabled
        self.processorType = processorType
        self.automaticCallRecordingEnabled = automaticCallRecordingEnabled
        self.captureSystemAudioEnabled = captureSystemAudioEnabled
        self.localRecordingEnabled = localRecordingEnabled
        self.echoCancellationEnabled = echoCancellationEnabled
        self.voiceIsolation = voiceIsolation
        self.recordingConsentAcknowledged = recordingConsentAcknowledged
        self.acceptedTermsVersion = acceptedTermsVersion
    }

    private enum CodingKeys: String, CodingKey {
        case processingEnabled
        case selectedInputDeviceUID
        case suppressionMode
        case launchAtLogin
        case bypassEnabled
        case processorType
        case automaticCallRecordingEnabled
        case captureSystemAudioEnabled
        case localRecordingEnabled
        case echoCancellationEnabled
        case voiceIsolation
        case recordingConsentAcknowledged
        case acceptedTermsVersion
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        processingEnabled = try container.decodeIfPresent(Bool.self, forKey: .processingEnabled) ?? false
        selectedInputDeviceUID = try container.decodeIfPresent(String.self, forKey: .selectedInputDeviceUID)
        suppressionMode = try container.decodeIfPresent(SuppressionMode.self, forKey: .suppressionMode) ?? .balanced
        launchAtLogin = try container.decodeIfPresent(Bool.self, forKey: .launchAtLogin) ?? false
        bypassEnabled = try container.decodeIfPresent(Bool.self, forKey: .bypassEnabled) ?? false
        processorType = try container.decodeIfPresent(NoiseProcessorType.self, forKey: .processorType) ?? .rnnoise
        automaticCallRecordingEnabled = try container.decodeIfPresent(Bool.self, forKey: .automaticCallRecordingEnabled) ?? false
        captureSystemAudioEnabled = try container.decodeIfPresent(Bool.self, forKey: .captureSystemAudioEnabled) ?? true
        localRecordingEnabled = try container.decodeIfPresent(Bool.self, forKey: .localRecordingEnabled) ?? true
        echoCancellationEnabled = try container.decodeIfPresent(Bool.self, forKey: .echoCancellationEnabled) ?? true
        voiceIsolation = try container.decodeIfPresent(VoiceIsolationLevel.self, forKey: .voiceIsolation) ?? .balanced
        recordingConsentAcknowledged = try container.decodeIfPresent(Bool.self, forKey: .recordingConsentAcknowledged) ?? false
        acceptedTermsVersion = try container.decodeIfPresent(Int.self, forKey: .acceptedTermsVersion) ?? 0
    }
}

public final class SettingsStore {
    private let key: String
    private let defaults: UserDefaults

    public init(key: String = SettingsStore.defaultKey, defaults: UserDefaults = .standard) {
        self.key = key
        self.defaults = defaults
    }

    /// Preferences domain of builds published before the rename to Pabulong.
    static let legacyDomain = "com.noiseless.app"
    static let legacyKey = "com.noiseless.settings.v1"
    public static let defaultKey = "com.nextmatterpro.pabulong.settings.v1"

    public func load() -> AppSettings {
        if defaults.data(forKey: key) == nil, defaults === UserDefaults.standard,
           let legacy = UserDefaults(suiteName: Self.legacyDomain)?.data(forKey: Self.legacyKey) {
            defaults.set(legacy, forKey: key)
        }
        guard let data = defaults.data(forKey: key) else {
            return AppSettings()
        }
        do {
            let decoded = try JSONDecoder().decode(AppSettings.self, from: data)
            return migrated(decoded)
        } catch {
            return AppSettings()
        }
    }

    private func migrated(_ settings: AppSettings) -> AppSettings {
        settings
    }

    public func save(_ settings: AppSettings) throws {
        let data = try JSONEncoder().encode(settings)
        defaults.set(data, forKey: key)
    }
}
