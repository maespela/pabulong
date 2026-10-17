import Foundation
import SQLite3

public enum CallMetadataStoreError: Error {
    case databaseOpenFailed(String)
    case databaseStepFailed(String)
}

public final class CallMetadataStore: @unchecked Sendable {
    private let paths: CallDataPaths
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private let queue = DispatchQueue(label: "com.nextmatterpro.pabulong.call-metadata")
    private var database: OpaquePointer?

    public init(paths: CallDataPaths? = nil) throws {
        let movedFrom = paths == nil ? CallDataPaths.migrateLegacyFolderIfNeeded() : nil
        self.paths = try paths ?? CallDataPaths.defaultPaths()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        encoder.dateEncodingStrategy = .iso8601
        decoder.dateDecodingStrategy = .iso8601

        try self.paths.prepareDirectories()
        var db: OpaquePointer?
        if sqlite3_open(self.paths.databaseURL.path, &db) != SQLITE_OK {
            let message = db.flatMap { sqlite3_errmsg($0) }.map { String(cString: $0) } ?? "unknown"
            throw CallMetadataStoreError.databaseOpenFailed(message)
        }
        database = db
        try execute("""
        CREATE TABLE IF NOT EXISTS calls (
            id TEXT PRIMARY KEY NOT NULL,
            payload TEXT NOT NULL,
            started_at REAL NOT NULL,
            ended_at REAL,
            status TEXT NOT NULL,
            deleted INTEGER NOT NULL DEFAULT 0
        );
        """)
        try execute("CREATE INDEX IF NOT EXISTS calls_started_at_idx ON calls(started_at DESC);")
        if let movedFrom {
            try rewriteStoredPaths(from: movedFrom.path, to: self.paths.rootDirectory.path)
        }
    }

    /// Calls store absolute file paths; after the data folder moves, point them
    /// at the new location.
    func rewriteStoredPaths(from oldRoot: String, to newRoot: String) throws {
        func moved(_ path: String?) -> String? {
            guard let path, path.hasPrefix(oldRoot) else { return path }
            return newRoot + path.dropFirst(oldRoot.count)
        }
        for var session in try listSessions(includeDeleted: true) {
            session.localMicAudioPath = moved(session.localMicAudioPath)
            session.remoteSystemAudioPath = moved(session.remoteSystemAudioPath)
            session.mixedPreviewAudioPath = moved(session.mixedPreviewAudioPath)
            try update(session)
        }
    }

    deinit {
        if let database {
            sqlite3_close(database)
        }
    }

    public func createSession(title providedTitle: String? = nil, appName: String? = nil) throws -> CallSession {
        try queue.sync {
            let id = UUID()
            let directory = paths.directory(for: id)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let title = Self.cleanTitle(providedTitle) ?? Self.fallbackTitle(appName: appName, date: Date())
            var session = CallSession(id: id, title: title, appName: appName)
            session.localMicAudioPath = directory.appending(path: "local-mic.wav").path
            session.remoteSystemAudioPath = directory.appending(path: "remote-system.wav").path
            session.mixedPreviewAudioPath = directory.appending(path: "mixed-preview.wav").path
            try upsertUnlocked(session)
            return session
        }
    }

    public func update(_ session: CallSession) throws {
        try queue.sync {
            try upsertUnlocked(session)
        }
    }

    public func listSessions(includeDeleted: Bool = false) throws -> [CallSession] {
        try queue.sync {
            let sql = includeDeleted
                ? "SELECT payload FROM calls ORDER BY started_at DESC;"
                : "SELECT payload FROM calls WHERE deleted = 0 ORDER BY started_at DESC;"
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else {
                throw CallMetadataStoreError.databaseStepFailed(lastError)
            }
            defer { sqlite3_finalize(statement) }

            var sessions: [CallSession] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                guard let text = sqlite3_column_text(statement, 0) else { continue }
                let payload = String(cString: text)
                if let data = payload.data(using: .utf8),
                   let session = try? decoder.decode(CallSession.self, from: data) {
                    sessions.append(session)
                }
            }
            return sessions
        }
    }

    public func deleteSession(id: UUID) throws {
        try queue.sync {
            if let session = try sessionUnlocked(id: id) {
                try? FileManager.default.removeItem(at: paths.directory(for: session.id))
            }
            var statement: OpaquePointer?
            guard sqlite3_prepare_v2(database, "DELETE FROM calls WHERE id = ?;", -1, &statement, nil) == SQLITE_OK else {
                throw CallMetadataStoreError.databaseStepFailed(lastError)
            }
            defer { sqlite3_finalize(statement) }
            sqlite3_bind_text(statement, 1, id.uuidString, -1, SQLITE_TRANSIENT)
            guard sqlite3_step(statement) == SQLITE_DONE else {
                throw CallMetadataStoreError.databaseStepFailed(lastError)
            }
        }
    }

    public var rootDirectory: URL { paths.rootDirectory }

    private func execute(_ sql: String) throws {
        guard sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK else {
            throw CallMetadataStoreError.databaseStepFailed(lastError)
        }
    }

    private func upsertUnlocked(_ session: CallSession) throws {
        let data = try encoder.encode(session)
        guard let payload = String(data: data, encoding: .utf8) else {
            throw CallMetadataStoreError.databaseStepFailed("Unable to encode call payload")
        }

        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, """
        INSERT INTO calls (id, payload, started_at, ended_at, status, deleted)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            payload = excluded.payload,
            started_at = excluded.started_at,
            ended_at = excluded.ended_at,
            status = excluded.status,
            deleted = excluded.deleted;
        """, -1, &statement, nil) == SQLITE_OK else {
            throw CallMetadataStoreError.databaseStepFailed(lastError)
        }
        defer { sqlite3_finalize(statement) }

        sqlite3_bind_text(statement, 1, session.id.uuidString, -1, SQLITE_TRANSIENT)
        sqlite3_bind_text(statement, 2, payload, -1, SQLITE_TRANSIENT)
        sqlite3_bind_double(statement, 3, session.startedAt.timeIntervalSince1970)
        if let endedAt = session.endedAt {
            sqlite3_bind_double(statement, 4, endedAt.timeIntervalSince1970)
        } else {
            sqlite3_bind_null(statement, 4)
        }
        sqlite3_bind_text(statement, 5, session.status.rawValue, -1, SQLITE_TRANSIENT)
        sqlite3_bind_int(statement, 6, session.status == .deleted ? 1 : 0)

        guard sqlite3_step(statement) == SQLITE_DONE else {
            throw CallMetadataStoreError.databaseStepFailed(lastError)
        }
    }

    private func sessionUnlocked(id: UUID) throws -> CallSession? {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, "SELECT payload FROM calls WHERE id = ?;", -1, &statement, nil) == SQLITE_OK else {
            throw CallMetadataStoreError.databaseStepFailed(lastError)
        }
        defer { sqlite3_finalize(statement) }
        sqlite3_bind_text(statement, 1, id.uuidString, -1, SQLITE_TRANSIENT)
        guard sqlite3_step(statement) == SQLITE_ROW, let text = sqlite3_column_text(statement, 0) else {
            return nil
        }
        let payload = String(cString: text)
        guard let data = payload.data(using: .utf8) else { return nil }
        return try decoder.decode(CallSession.self, from: data)
    }

    private var lastError: String {
        guard let database, let message = sqlite3_errmsg(database) else { return "unknown sqlite error" }
        return String(cString: message)
    }

    public static func fallbackTitle(appName: String?, date: Date = Date()) -> String {
        let timestamp = titleDateFormatter.string(from: date)
        guard let appName = cleanTitle(appName) else {
            return "Call \(timestamp)"
        }
        return "\(appName) Call \(timestamp)"
    }

    public static func cleanTitle(_ title: String?) -> String? {
        guard let title else { return nil }
        let collapsed = title
            .components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        return collapsed.isEmpty ? nil : collapsed
    }

    private static let titleDateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        return formatter
    }()
}

private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
