import Foundation

public enum AudioTrackRole: String, Codable, CaseIterable, Sendable {
    case localMic
    case remoteSystem
    case mixedPreview

    public var displayName: String {
        switch self {
        case .localMic: "You"
        case .remoteSystem: "Call Audio"
        case .mixedPreview: "Mixed"
        }
    }
}

public enum CallSessionStatus: String, Codable, Sendable {
    case recording
    case finalizing
    case completed
    case failed
    case deleted
}

public struct CallSession: Codable, Equatable, Identifiable, Sendable {
    public var id: UUID
    public var title: String
    public var startedAt: Date
    public var endedAt: Date?
    public var status: CallSessionStatus
    public var appName: String?
    public var localMicAudioPath: String?
    public var remoteSystemAudioPath: String?
    public var mixedPreviewAudioPath: String?
    public var remoteAudioMissing: Bool

    public init(
        id: UUID = UUID(),
        title: String,
        startedAt: Date = Date(),
        endedAt: Date? = nil,
        status: CallSessionStatus = .recording,
        appName: String? = nil,
        localMicAudioPath: String? = nil,
        remoteSystemAudioPath: String? = nil,
        mixedPreviewAudioPath: String? = nil,
        remoteAudioMissing: Bool = false
    ) {
        self.id = id
        self.title = title
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.status = status
        self.appName = appName
        self.localMicAudioPath = localMicAudioPath
        self.remoteSystemAudioPath = remoteSystemAudioPath
        self.mixedPreviewAudioPath = mixedPreviewAudioPath
        self.remoteAudioMissing = remoteAudioMissing
    }

    private enum CodingKeys: String, CodingKey {
        case id, title, startedAt, endedAt, status, appName, localMicAudioPath
        case remoteSystemAudioPath, mixedPreviewAudioPath, remoteAudioMissing
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UUID.self, forKey: .id)
        title = try c.decode(String.self, forKey: .title)
        startedAt = try c.decode(Date.self, forKey: .startedAt)
        endedAt = try c.decodeIfPresent(Date.self, forKey: .endedAt)
        status = try c.decode(CallSessionStatus.self, forKey: .status)
        appName = try c.decodeIfPresent(String.self, forKey: .appName)
        localMicAudioPath = try c.decodeIfPresent(String.self, forKey: .localMicAudioPath)
        remoteSystemAudioPath = try c.decodeIfPresent(String.self, forKey: .remoteSystemAudioPath)
        mixedPreviewAudioPath = try c.decodeIfPresent(String.self, forKey: .mixedPreviewAudioPath)
        remoteAudioMissing = try c.decodeIfPresent(Bool.self, forKey: .remoteAudioMissing) ?? false
    }
}

public struct CallSessionContext: Equatable, Sendable {
    public var title: String?
    public var appName: String?

    public init(title: String? = nil, appName: String? = nil) {
        self.title = Self.clean(title)
        self.appName = Self.clean(appName)
    }

    private static func clean(_ value: String?) -> String? {
        guard let value else { return nil }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}

public struct CallDataPaths: Sendable {
    public let rootDirectory: URL
    public let callsDirectory: URL
    public let databaseURL: URL

    public init(rootDirectory: URL) {
        self.rootDirectory = rootDirectory
        callsDirectory = rootDirectory.appending(path: "Calls", directoryHint: .isDirectory)
        databaseURL = rootDirectory.appending(path: "Calls.sqlite")
    }

    /// App data folder name (the app was renamed from Noiseless to Pabulong).
    public static let folderName = "Pabulong"
    static let legacyFolderName = "Noiseless"

    public static func defaultPaths() throws -> CallDataPaths {
        let base = try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        return CallDataPaths(rootDirectory: base.appending(path: folderName, directoryHint: .isDirectory))
    }

    /// Moves data written by builds named "Noiseless" into the Pabulong folder.
    /// Returns the old root when a move happened, so stored paths can be rewritten.
    static func migrateLegacyFolderIfNeeded() -> URL? {
        guard let base = try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true) else {
            return nil
        }
        let legacy = base.appending(path: legacyFolderName, directoryHint: .isDirectory)
        let current = base.appending(path: folderName, directoryHint: .isDirectory)
        let fm = FileManager.default
        guard fm.fileExists(atPath: legacy.path), !fm.fileExists(atPath: current.path) else { return nil }
        do {
            try fm.moveItem(at: legacy, to: current)
            return legacy
        } catch {
            return nil
        }
    }

    public func prepareDirectories() throws {
        try FileManager.default.createDirectory(at: rootDirectory, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: callsDirectory, withIntermediateDirectories: true)
    }

    public func directory(for callID: UUID) -> URL {
        callsDirectory.appending(path: callID.uuidString, directoryHint: .isDirectory)
    }
}
