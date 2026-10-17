import Foundation

public final class CallSessionManager: @unchecked Sendable {
    private enum SessionOrigin {
        case automatic
        case manual
    }

    private let store: CallMetadataStore
    private let recorder: CallRecorder
    private let contextProvider: @Sendable () -> CallSessionContext?
    private var systemAudioCapture: SystemAudioCapturing?
    private let queue = DispatchQueue(label: "com.nextmatterpro.pabulong.call-session-manager", qos: .utility)
    private var activeSessionStorage: CallSession?
    private var activeSessionOrigin: SessionOrigin?

    public var activeSession: CallSession? {
        queue.sync { activeSessionStorage }
    }

    public init(
        store: CallMetadataStore,
        recorder: CallRecorder = CallRecorder(),
        contextProvider: @escaping @Sendable () -> CallSessionContext? = { nil }
    ) throws {
        self.store = store
        self.recorder = recorder
        self.contextProvider = contextProvider
    }

    public func updateVirtualMicConsumerCount(_ count: Int, settings: AppSettings) {
        guard count > 0 else {
            stopActiveSession()
            return
        }
        guard settings.automaticCallRecordingEnabled, settings.localRecordingEnabled, settings.recordingConsentAcknowledged else {
            return
        }
        startIfNeeded(settings: settings)
    }

    public func startIfNeeded(settings: AppSettings) {
        startSessionIfNeeded(settings: settings, origin: .automatic)
    }

    public func startManualSession(settings: AppSettings) {
        guard settings.localRecordingEnabled, settings.recordingConsentAcknowledged else { return }
        startSessionIfNeeded(settings: settings, origin: .manual)
    }

    private func startSessionIfNeeded(settings: AppSettings, origin: SessionOrigin) {
        queue.async { [weak self] in
            guard let self, self.activeSessionStorage == nil else { return }
            do {
                let context = self.contextProvider()
                let session = try self.store.createSession(title: context?.title, appName: context?.appName)
                try self.store.update(session)
                try self.recorder.start(session: session)
                self.activeSessionStorage = session
                self.activeSessionOrigin = origin
                self.refreshActiveSessionContext(after: 2.0)

                if settings.captureSystemAudioEnabled {
                    let capture = SystemAudioCaptureFactory.make()
                    capture.onAudioBuffer = { [weak self] buffer in
                        self?.recorder.recordRemoteBuffer(buffer)
                    }
                    self.systemAudioCapture = capture
                    Task { [weak self] in
                        do {
                            try await capture.start()
                        } catch {
                            self?.markRemoteAudioMissing()
                        }
                    }
                } else {
                    self.markRemoteAudioMissing()
                }

            } catch {
                self.activeSessionStorage = nil
                self.activeSessionOrigin = nil
            }
        }
    }

    public func recordLocalFrame(_ frame: AudioFrame) {
        recorder.recordLocalFrame(frame)
    }

    public func stopActiveSession() {
        queue.async { [weak self] in
            guard let self, var session = self.activeSessionStorage else { return }
            self.activeSessionStorage = nil
            self.activeSessionOrigin = nil
            self.recorder.stop()
            let capture = self.systemAudioCapture
            self.systemAudioCapture = nil
            Task { await capture?.stop() }

            session.endedAt = Date()
            session.status = .completed
            session.remoteAudioMissing = session.remoteAudioMissing || self.recorder.didLoseRemoteAudio
            try? self.store.update(session)
        }
    }

    public func listSessions() -> [CallSession] {
        (try? store.listSessions()) ?? []
    }

    public func refreshActiveSessionContext() {
        queue.async { [weak self] in
            self?.refreshActiveSessionContextUnlocked()
        }
    }

    public func deleteSession(id: UUID) {
        queue.async { [weak self] in
            guard let self else { return }
            if let session = self.activeSessionStorage, session.id == id {
                self.activeSessionStorage = nil
                self.activeSessionOrigin = nil
                self.recorder.stop()
                let capture = self.systemAudioCapture
                self.systemAudioCapture = nil
                Task { await capture?.stop() }
            }
            try? self.store.deleteSession(id: id)
        }
    }

    private func markRemoteAudioMissing() {
        queue.async { [weak self] in
            guard let self, var session = self.activeSessionStorage else { return }
            session.remoteAudioMissing = true
            self.activeSessionStorage = session
            try? self.store.update(session)
        }
    }

    private func refreshActiveSessionContext(after delay: TimeInterval) {
        queue.asyncAfter(deadline: .now() + delay) { [weak self] in
            self?.refreshActiveSessionContextUnlocked()
        }
    }

    private func refreshActiveSessionContextUnlocked() {
        guard var session = activeSessionStorage,
              let context = contextProvider() else {
            return
        }
        guard apply(context: context, to: &session) else { return }
        activeSessionStorage = session
        try? store.update(session)
    }

    private func apply(context: CallSessionContext, to session: inout CallSession) -> Bool {
        var changed = false
        if let appName = context.appName, session.appName != appName {
            session.appName = appName
            changed = true
        }
        if let title = context.title, shouldReplaceTitle(session.title, with: title, appName: session.appName) {
            session.title = title
            changed = true
        }
        return changed
    }


    private func shouldReplaceTitle(_ current: String, with candidate: String, appName: String?) -> Bool {
        guard current != candidate else { return false }
        if current.hasPrefix("Call ") { return true }
        if let appName, current.hasPrefix("\(appName) Call ") { return true }
        if let appName, current == appName, candidate.count > appName.count { return true }
        return false
    }
}
