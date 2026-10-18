import AVFoundation
import CoreGraphics
import NoiselessAudioCore
import ServiceManagement
import SwiftUI

@main
struct NoiselessApp: App {
    @StateObject private var model = MenuBarModel()

    init() {
        BrandFonts.register()
    }

    var body: some Scene {
        // The window comes first so a launch (Pabulong is a Dock app) opens it.
        WindowGroup("Pabulong", id: "main") {
            NoiselessMainWindow(model: model)
                .frame(minWidth: 980, minHeight: 640)
                .tint(Brand.signal)
                // Pabulong keeps its light glass look in dark mode too.
                .preferredColorScheme(.light)
                .onAppear {
                    model.refreshDevices()
                    model.refreshCalls()
                    model.startMonitoringIfNeeded()
                }
                .onChange(of: model.settings) { _, _ in
                    model.persistSettings()
                    model.reconfigurePipeline()
                }
        }
        .windowResizability(.contentMinSize)
        .commands {
            CommandGroup(after: .appInfo) {
                CheckForUpdatesCommand(updater: model.updater)
            }
        }

        MenuBarExtra {
            MenuBarPopover(model: model)
                .tint(Brand.signal)
                .preferredColorScheme(.light)
        } label: {
            MenuBarStatusMark(enabled: model.enabled)
                .accessibilityLabel("Pabulong")
                .task {
                    // The menu bar icon renders at launch, so this begins call
                    // monitoring in the background without needing the user to
                    // open the menu first (when processing is enabled).
                    model.refreshDevices()
                    model.startMonitoringIfNeeded()
                }
        }
        .menuBarExtraStyle(.window)
    }
}

struct MenuBarPopover: View {
    @ObservedObject var model: MenuBarModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                PabulongAppMark(enabled: model.enabled, size: 32)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Pabulong")
                        .font(BrandFonts.body(16, .semibold))
                        .foregroundStyle(Brand.text)
                    Text(model.enabled || model.isVirtualMicInUse ? model.virtualDeviceStatus : "Off")
                        .font(BrandFonts.body(12))
                        .foregroundStyle(Brand.secondaryText)
                }
                Spacer()
                StatusDot(active: model.enabled || model.bridgeHealth.producerAttached)
            }
            .padding(.bottom, 10)

            HStack {
                Text("Mic Processing")
                    .font(BrandFonts.body(14.5))
                    .foregroundStyle(Brand.text)
                Spacer()
                BrandSwitch(title: "Mic Processing", isOn: Binding(
                    get: { model.enabled },
                    set: { model.setProcessingEnabled($0) }
                ))
            }
            .frame(height: 32)

            if model.isVirtualMicInUse || model.activeCall != nil {
                RecordingControlStrip(model: model, compact: true)
                    .padding(.top, 8)
            }

            if model.enabled && model.micAccessDenied {
                Button {
                    model.openMicrophoneSettings()
                } label: {
                    Label("Microphone Settings", systemImage: "exclamationmark.triangle.fill")
                }
                .buttonStyle(BrandButtonStyle(kind: .secondary, fullWidth: true))
                .padding(.top, 8)
            }

            if model.virtualMicIsDefaultInput {
                VStack(alignment: .leading, spacing: 6) {
                    Label("Pabulong Mic is your default input", systemImage: "exclamationmark.triangle.fill")
                        .font(BrandFonts.body(12, .semibold))
                        .foregroundStyle(.orange)
                    Text("That keeps your mic active all the time. Use Pabulong Mic only inside your call app.")
                        .font(BrandFonts.body(11))
                        .foregroundStyle(Brand.secondaryText)
                        .fixedSize(horizontal: false, vertical: true)
                    Button("Fix — switch default to a real mic") {
                        model.ensureVirtualMicIsNotDefaultInput()
                    }
                    .buttonStyle(BrandButtonStyle(kind: .primary, height: 26))
                }
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.orange.opacity(0.12), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .padding(.top, 8)
            }

            Rectangle()
                .fill(Brand.separator)
                .frame(height: 1)
                .padding(.top, 11)
                .padding(.bottom, 5)

            PopoverLinkButton(title: "Open Pabulong", icon: .window) {
                openWindow(id: "main")
                NSApp.activate(ignoringOtherApps: true)
            }
            .keyboardShortcut("o")

            PopoverLinkButton(title: "Quit", icon: .power) {
                NSApplication.shared.terminate(nil)
            }
            .keyboardShortcut("q")
        }
        .padding(15)
        .frame(width: 300)
        .background(Brand.popoverGlass)
        .onAppear {
            model.refreshDevices()
            model.refreshCalls()
            model.startMonitoringIfNeeded()
        }
        .onChange(of: model.settings) { _, _ in
            model.persistSettings()
            model.reconfigurePipeline()
        }
    }
}

/// "Open Pabulong" / "Quit" rows: line icon and label, full-width hover wash.
struct PopoverLinkButton: View {
    var title: String
    var icon: BrandIcon
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                BrandIconView(icon: icon, size: 16)
                Text(title)
                    .font(BrandFonts.body(14))
                Spacer(minLength: 0)
            }
            .foregroundStyle(Brand.text)
            .padding(.horizontal, 5)
            .frame(maxWidth: .infinity, minHeight: 30, alignment: .leading)
        }
        .buttonStyle(BrandPlainButtonStyle(cornerRadius: 6))
        .padding(.horizontal, -5)
    }
}

@MainActor
final class MenuBarModel: ObservableObject {
    @Published var enabled: Bool
    @Published var settings: AppSettings
    @Published var selectedDeviceUID: String?
    @Published var devices: [AudioInputDevice] = []
    @Published var processorMetrics = ProcessorMetrics()
    @Published var echoMetrics = EchoMetrics()
    /// True while the speaker reference (system output tap) is feeding the echo canceller.
    @Published var echoReferenceActive = false
    /// Bump when the Terms of Use or Privacy Notice change materially, so users
    /// are asked to agree again.
    static let currentTermsVersion = 3

    let updater = AppUpdater()

    /// First-launch agreement (click-wrap). The app does not run until the user
    /// agrees; choosing Quit exits.
    func presentTermsAgreementIfNeeded() {
        guard settings.acceptedTermsVersion < Self.currentTermsVersion else { return }
        NSApp.activate(ignoringOtherApps: true)
        while true {
            let choice = BrandAlert.run(
                title: "Welcome to Pabulong",
                message: """
                Please read and agree to the Pabulong Terms of Use and Privacy Notice before you continue.

                • Pabulong works on this Mac. It does not send your audio or recordings to anyone.
                • Pabulong checks for updates only if you ask it to. A check sends your IP address, the Pabulong version and your language setting to our server.
                • If you record calls, you are bound by the recording laws where you and everyone on the call are, and you must know and follow them. Next Matter Pro does not control your recordings and is not liable for them.
                """,
                messageAlignment: .leading,
                choices: [
                    .init(title: "I Agree", kind: .primary),
                    .init(title: "Read Terms", kind: .secondary),
                    .init(title: "Quit", kind: .secondary, isCancel: true)
                ]
            )
            switch choice {
            case 0:
                settings.acceptedTermsVersion = Self.currentTermsVersion
                persistSettings()
                return
            case 1:
                NSWorkspace.shared.open(LegalLinks.terms)
                NSWorkspace.shared.open(LegalLinks.privacy)
            default:
                NSApp.terminate(nil)
                return
            }
        }
    }

    @Published var showingThirdPartyNotices = false
    private var afterRecordingConsent: (() -> Void)?
    private var isPresentingRecordingConsent = false

    /// Runs `action` once the user has confirmed they will get consent from
    /// everyone on the call. Asks the first time only.
    func withRecordingConsent(_ action: @escaping () -> Void) {
        if settings.recordingConsentAcknowledged {
            action()
        } else {
            afterRecordingConsent = action
            presentRecordingConsent()
        }
    }

    private func presentRecordingConsent() {
        guard !isPresentingRecordingConsent else { return }
        isPresentingRecordingConsent = true
        // Leave the current event (a click in the popover or a switch) first.
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            let choice = BrandAlert.run(
                title: "Recording laws are your responsibility",
                message: "When you record, you are bound by the recording laws where you are and where everyone on the call is, and you need to know them. Many places, including the Philippines and several U.S. states such as California and Florida, require everyone's consent. Follow your client's or law firm's confidentiality rules too. Pabulong does not control or review your recordings, and Next Matter Pro is not liable for them. Recordings stay on this Mac.",
                choices: [
                    .init(title: "I Understand", kind: .primary),
                    .init(title: "Cancel", kind: .secondary, isCancel: true)
                ]
            )
            self.isPresentingRecordingConsent = false
            if choice == 0 {
                self.acceptRecordingConsent()
            } else {
                self.declineRecordingConsent()
            }
        }
    }

    func acceptRecordingConsent() {
        settings.recordingConsentAcknowledged = true
        persistSettings()
        let action = afterRecordingConsent
        afterRecordingConsent = nil
        action?()
    }

    func declineRecordingConsent() {
        afterRecordingConsent = nil
    }

    @Published var echoTestRunning = false
    @Published var echoTestStatus = ""

    /// Plays speech through the speakers and measures how much echo the chain
    /// removes on this Mac's real hardware.
    func runSpeakerEchoTest() {
        guard !echoTestRunning else { return }
        guard pipeline == nil else {
            echoTestStatus = "End the call first — the test needs the mic to itself."
            return
        }
        guard #available(macOS 14.2, *), NoiseProcessorFactory.usesSharedCore else {
            echoTestStatus = "Echo cancellation needs macOS 14.2 or later."
            return
        }
        echoTestRunning = true
        echoTestStatus = "Playing a test sentence through your speakers — please stay quiet…"
        let inputUID = settings.selectedInputDeviceUID
        Task { @MainActor [weak self] in
            #if NL_HAS_RUST_CORE
            do {
                let result = try await SpeakerEchoTest.run(preferredInputUID: inputUID)
                self?.echoTestStatus = result.summary
            } catch {
                self?.echoTestStatus = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            }
            #endif
            self?.echoTestRunning = false
        }
    }
    @Published var bridgeHealth = BridgeHealth()
    @Published var pipelineStatus = "Stopped"
    @Published var micAccessDenied = false
    @Published var calls: [CallSession] = []
    @Published var activeCall: CallSession?
    @Published var virtualMicIsDefaultInput = false
    @Published var callStatusMessage = ""
    @Published var playbackCallID: UUID?
    @Published var playbackElapsed: TimeInterval = 0
    @Published var playbackDuration: TimeInterval = 0
    @Published var playbackIsPlaying = false

    var virtualDeviceStatus: String {
        bridgeHealth.producerAttached ? (enabled ? "Feeding" : "Pass Through") : pipelineStatus
    }
    /// True when an app is actively pulling the virtual mic — i.e. a call app has
    /// selected and is using Noiseless Mic. The physical mic is only captured in
    /// this state, which is why Noiseless Mic must not be the *default* input
    /// (that would make something consume it continuously).
    var isCallActive: Bool {
        bridgeHealth.consumerRunningCount > 0
    }
    var isVirtualMicInUse: Bool {
        isCallActive || pendingPipelineStop != nil || pipeline?.isRunning == true
    }
    var canStartManualRecording: Bool {
        enabled && isVirtualMicInUse && activeCall == nil && settings.localRecordingEnabled
    }
    var recordingControlTitle: String {
        if activeCall != nil { return "Recording" }
        if isVirtualMicInUse { return "Meeting Detected" }
        return "No Active Meeting"
    }
    var recordingControlSubtitle: String {
        if let activeCall {
            return "\(callDurationText(activeCall)) · Recording locally"
        }
        if isVirtualMicInUse {
            return settings.localRecordingEnabled ? "Ready to record" : "Local recording is off"
        }
        return "Select Pabulong Mic in a call"
    }
    var engineName: String {
        processor?.engineName ?? NoiseProcessorFactory.engineDescription
    }
    let latencyLabel = "< 40 ms target"

    var echoStatusText: String {
        guard settings.echoCancellationEnabled else { return "Off" }
        guard echoReferenceActive else { return "Waiting for speaker audio" }
        let m = echoMetrics
        return m.echoReductionDB > 0.5 ? String(format: "%.0f dB removed", m.echoReductionDB) : (m.farEndActive ? "Learning…" : "Ready")
    }

    var echoPathText: String {
        let m = echoMetrics
        guard echoReferenceActive else { return "—" }
        return String(format: "%.0f ms delay · ref −%.0f ms", m.echoDelayMS, m.referenceLookbackMS)
    }

    var voiceGateText: String {
        guard settings.voiceIsolation != .off else { return "Off" }
        let gain = echoMetrics.voiceGateGain
        return gain < 0.9 ? String(format: "Ducking (%.0f dB)", 20 * log10(max(gain, 0.001))) : "\(settings.voiceIsolation.displayName) · open"
    }

    private let settingsStore = SettingsStore()
    private let deviceManager = DeviceManager()
    private var bridge: SharedVirtualMicBridge?
    private var processor: NoiseProcessor?
    private var echoFeed: AnyObject? // EchoReferenceFeed (macOS 14.2+)
    private var pipeline: AudioPipeline?
    private var metricsTimer: Timer?
    private var pendingPipelineStop: DispatchWorkItem?
    private var lastSlowRefreshAt = Date.distantPast
    private var lastMeetingContextRefreshAt = Date.distantPast
    private var lastReportedCallConsumerCount: Int?
    private var lastReportedCallRecordingEnabled: Bool?
    private var isRequestingMicAccess = false
    private var audioPlayer: AVAudioPlayer?
    private var playbackTimer: Timer?
    private var activeDeviceUID: String?
    private let callManager: CallSessionManager?
    private let realtimeWatchdogInterval: TimeInterval = 0.05
    private let slowRefreshInterval: TimeInterval = 0.5
    private let meetingContextRefreshInterval: TimeInterval = 2.0
    private let pipelineStopDebounceInterval: TimeInterval = 1.0

    init() {
        let loaded = settingsStore.load()
        settings = loaded
        enabled = loaded.processingEnabled
        selectedDeviceUID = loaded.selectedInputDeviceUID
        if let store = try? CallMetadataStore(),
           let manager = try? CallSessionManager(store: store, contextProvider: MeetingContextDetector.detect) {
            callManager = manager
            calls = manager.listSessions()
        } else {
            callManager = nil
            callStatusMessage = "Call storage unavailable"
        }
        updater.mayAskPermission = { [weak self] in
            guard let self else { return false }
            return self.settings.acceptedTermsVersion >= Self.currentTermsVersion
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6) { [weak self] in
            self?.presentTermsAgreementIfNeeded()
        }
    }

    func refreshDevices() {
        devices = deviceManager.listInputDevices()
    }

    func setProcessingEnabled(_ isEnabled: Bool) {
        enabled = isEnabled
        settings.processingEnabled = isEnabled
        if isEnabled {
            startMonitoringIfNeeded()
            pipeline?.setBypass(settings.bypassEnabled)
            if isCallActive {
                startPipelineIfNeeded(requestPermission: true)
            }
        } else {
            pipeline?.setBypass(true)
            startMonitoringIfNeeded()
            if isCallActive {
                startPipelineIfNeeded(requestPermission: true)
            } else {
                schedulePipelineStopIfIdle()
            }
        }
    }

    func persistSettings() {
        try? settingsStore.save(settings)
        applyLaunchAtLogin(settings.launchAtLogin)
    }

    /// Registers or unregisters the app as a macOS login item. No-op when the
    /// binary is not running from a proper .app bundle (e.g. `swift run`).
    func applyLaunchAtLogin(_ enabled: Bool) {
        let service = SMAppService.mainApp
        do {
            if enabled {
                if service.status != .enabled {
                    try service.register()
                }
            } else {
                if service.status == .enabled {
                    try service.unregister()
                }
            }
        } catch {
            // Not fatal: unbundled runs and unsigned builds can't manage login items.
            pipelineStatus = pipelineStatus == "Running" ? pipelineStatus : "Login item unavailable"
        }
    }

    func startMonitoringIfNeeded() {
        // Never let Noiseless Mic remain the system default input (that would keep
        // the physical mic captured constantly). Only capture when a call app
        // explicitly selects Noiseless Mic.
        ensureVirtualMicIsNotDefaultInput()
        if bridge == nil {
            do {
                bridge = try SharedVirtualMicBridge()
                pipelineStatus = enabled ? "Waiting for call" : "Off"
            } catch {
                pipelineStatus = "Virtual mic unavailable"
                return
            }
        } else if pipeline?.isRunning != true {
            pipelineStatus = enabled ? "Waiting for call" : "Off"
        }
        startMetricsTimer()
    }

    private func startPipelineIfNeeded(requestPermission: Bool = false) {
        guard enabled || isCallActive || pendingPipelineStop != nil else { return }
        cancelPendingPipelineStop()
        guard pipeline?.isRunning != true else { return }
        startMonitoringIfNeeded()

        // Do not trigger TCC just because Noiseless is enabled. The permission
        // prompt is reserved for the moment an app actively uses Noiseless Mic.
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            isRequestingMicAccess = false
            micAccessDenied = false
        case .notDetermined:
            guard requestPermission else {
                micAccessDenied = false
                pipelineStatus = "Mic permission needed"
                return
            }
            guard !isRequestingMicAccess else { return }
            isRequestingMicAccess = true
            AVCaptureDevice.requestAccess(for: .audio) { [weak self] granted in
                Task { @MainActor in
                    guard let self else { return }
                    self.isRequestingMicAccess = false
                    self.micAccessDenied = !granted
                    if granted { self.startPipelineIfNeeded(requestPermission: false) }
                }
            }
            return
        case .denied, .restricted:
            isRequestingMicAccess = false
            micAccessDenied = true
            pipelineStatus = "Microphone access denied"
            return
        @unknown default:
            isRequestingMicAccess = false
            micAccessDenied = true
            return
        }

        do {
            guard let bridge else {
                pipelineStatus = "Virtual mic unavailable"
                return
            }
            let processor = NoiseProcessorFactory.make(settings: settings)
            let pipeline = AudioPipeline(
                processor: processor,
                bridge: bridge,
                deviceManager: deviceManager,
                preferredInputUID: settings.selectedInputDeviceUID
            )
            let callManager = self.callManager
            pipeline.processedFrameHandler = { frame in
                callManager?.recordLocalFrame(frame)
            }
            try pipeline.start()
            pipeline.setBypass(settings.bypassEnabled || !enabled)
            self.processor = processor
            self.pipeline = pipeline
            updateEchoReference()
            activeDeviceUID = settings.selectedInputDeviceUID
            pipelineStatus = enabled ? "Running" : "Pass Through"
            startMetricsTimer()
        } catch AudioPipelineError.wouldCaptureVirtualDevice {
            stopPipeline(keepMonitoring: true, statusAfterStop: "Pick a physical mic")
        } catch AudioPipelineError.inputDeviceUnavailable {
            stopPipeline(keepMonitoring: true, statusAfterStop: "No input device")
        } catch AudioPipelineError.producerAlreadyRunning {
            stopPipeline(keepMonitoring: true, statusAfterStop: "Already running elsewhere")
        } catch {
            stopPipeline(keepMonitoring: true, statusAfterStop: "Error")
        }
    }

    private func stopPipeline(keepMonitoring: Bool = false, statusAfterStop: String? = nil) {
        cancelPendingPipelineStop()
        pipeline?.stop()
        pipeline = nil
        processor = nil
        stopEchoReference()
        callManager?.stopActiveSession()
        lastReportedCallConsumerCount = nil
        lastReportedCallRecordingEnabled = nil
        processorMetrics = ProcessorMetrics()
        echoMetrics = EchoMetrics()
        micAccessDenied = false
        isRequestingMicAccess = false
        activeDeviceUID = nil
        if keepMonitoring {
            if let bridge {
                bridgeHealth = bridge.health
            }
        } else {
            bridge?.detachProducer()
            bridge = nil
            metricsTimer?.invalidate()
            metricsTimer = nil
            bridgeHealth = BridgeHealth()
        }
        pipelineStatus = statusAfterStop ?? (keepMonitoring ? (enabled ? "Waiting for call" : "Off") : "Stopped")
    }

    private func schedulePipelineStopIfIdle() {
        guard pipeline?.isRunning == true else {
            let idleStatus = enabled ? "Waiting for call" : "Off"
            if pipelineStatus != idleStatus {
                pipelineStatus = idleStatus
            }
            return
        }
        guard pendingPipelineStop == nil else { return }
        pipelineStatus = enabled ? "Waiting for call" : "Off"
        let workItem = DispatchWorkItem { [weak self] in
            Task { @MainActor in
                guard let self else { return }
                self.pendingPipelineStop = nil
                if let bridge = self.bridge {
                    self.bridgeHealth = bridge.health
                }
                if !self.isCallActive {
                    self.stopPipeline(keepMonitoring: true)
                }
            }
        }
        pendingPipelineStop = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + pipelineStopDebounceInterval, execute: workItem)
    }

    private func cancelPendingPipelineStop() {
        pendingPipelineStop?.cancel()
        pendingPipelineStop = nil
    }

    func stopMonitoring() {
        stopPipeline(keepMonitoring: false)
    }

    func openMicrophoneSettings() {
        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") {
            NSWorkspace.shared.open(url)
        }
    }

    func reconfigurePipeline() {
        processor?.mode = settings.suppressionMode
        if let chain = processor as? EchoCancellingProcessor {
            if chain.voiceIsolation != settings.voiceIsolation { chain.voiceIsolation = settings.voiceIsolation }
            if chain.echoCancellationEnabled != settings.echoCancellationEnabled { chain.echoCancellationEnabled = settings.echoCancellationEnabled }
        }
        updateEchoReference()
        // Bypass applies live without a restart.
        pipeline?.setBypass(settings.bypassEnabled || !enabled)
        // The processor type and the capture device are both fixed at pipeline
        // construction, so either change requires a restart.
        let deviceChanged = settings.selectedInputDeviceUID != activeDeviceUID
        if pipeline?.isRunning == true, processor?.type != settings.processorType || deviceChanged {
            stopPipeline(keepMonitoring: true)
            if isCallActive {
                startPipelineIfNeeded(requestPermission: false)
            }
        } else if enabled {
            startMonitoringIfNeeded()
        }
    }

    /// Runs the speaker-output tap that feeds the echo canceller whenever the
    /// pipeline is running with echo cancellation on (same design as Windows,
    /// where WASAPI loopback provides the reference).
    private func updateEchoReference() {
        let wanted = pipeline != nil && settings.echoCancellationEnabled && processor is EchoCancellingProcessor
        if wanted, echoFeed == nil {
            guard #available(macOS 14.2, *), let chain = processor as? EchoCancellingProcessor else { return }
            let feed = EchoReferenceFeed(target: chain)
            echoFeed = feed
            Task { [weak self] in
                do {
                    try await feed.start()
                    await MainActor.run { self?.echoReferenceActive = true }
                } catch {
                    await MainActor.run {
                        guard let self, self.echoFeed === feed else { return }
                        self.echoFeed = nil
                        self.echoReferenceActive = false
                    }
                }
            }
        } else if !wanted {
            stopEchoReference()
        }
    }

    private func stopEchoReference() {
        guard let feed = echoFeed else { return }
        echoFeed = nil
        echoReferenceActive = false
        if #available(macOS 14.2, *), let feed = feed as? EchoReferenceFeed {
            Task { await feed.stop() }
        }
    }

    func exportDiagnostics() {
        if let processor {
            processorMetrics = processor.metrics
        }
        if let bridge {
            bridgeHealth = bridge.health
        }
        let snapshot = DiagnosticsSnapshot(
            settings: settings,
            inputDevices: devices.map { "\($0.name) [\($0.uid)]" },
            bridge: bridgeHealth,
            processor: processorMetrics
        )
        guard let data = try? snapshot.jsonData() else { return }
        let url = FileManager.default.temporaryDirectory.appending(path: "PabulongDiagnostics.json")
        try? data.write(to: url, options: .atomic)
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    private func startMetricsTimer() {
        guard metricsTimer == nil else { return }
        let timer = Timer(timeInterval: realtimeWatchdogInterval, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self else { return }
                if let processor = self.processor {
                    self.processorMetrics = processor.metrics
                    if let chain = processor as? EchoCancellingProcessor {
                        self.echoMetrics = chain.echoMetrics
                    }
                }
                if let bridge = self.bridge {
                    self.bridgeHealth = bridge.health
                    if self.isCallActive {
                        self.startPipelineIfNeeded(requestPermission: true)
                    } else if self.pipeline?.isRunning == true || self.pendingPipelineStop != nil {
                        self.schedulePipelineStopIfIdle()
                    } else {
                        self.pipelineStatus = self.enabled ? "Waiting for call" : "Off"
                    }
                    let effectiveCallConsumerCount =
                        self.isCallActive || self.pendingPipelineStop != nil ? 1 : 0
                    self.reportCallConsumerCountIfNeeded(effectiveCallConsumerCount)
                    self.refreshSlowStateIfNeeded()
                }
            }
        }
        timer.tolerance = 0.01
        metricsTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    private func reportCallConsumerCountIfNeeded(_ count: Int) {
        let recordingEnabled = settings.automaticCallRecordingEnabled && settings.localRecordingEnabled && settings.recordingConsentAcknowledged
        guard count != lastReportedCallConsumerCount || recordingEnabled != lastReportedCallRecordingEnabled else {
            return
        }
        lastReportedCallConsumerCount = count
        lastReportedCallRecordingEnabled = recordingEnabled
        callManager?.updateVirtualMicConsumerCount(count, settings: settings)
    }

    private func refreshSlowStateIfNeeded() {
        let now = Date()
        guard now.timeIntervalSince(lastSlowRefreshAt) >= slowRefreshInterval else { return }
        lastSlowRefreshAt = now
        // Surface (but don't auto-change mid-session) if Noiseless Mic became the
        // default input again — the UI offers a one-click fix. Auto-correction
        // happens on launch to avoid switching a device out from under a call.
        virtualMicIsDefaultInput = deviceManager.isVirtualDeviceDefaultInput()
        refreshMeetingContextIfNeeded(now: now)
        refreshCalls()
    }

    /// Noiseless Mic must not be the *system default* input: as the default it is
    /// consumed continuously by macOS/background apps, which keeps the physical
    /// mic captured 24/7. If it is the default, move the default to a real input
    /// so the mic is only captured when a call app explicitly selects Noiseless
    /// Mic. Returns true if it made a change.
    @discardableResult
    func ensureVirtualMicIsNotDefaultInput() -> Bool {
        guard deviceManager.isVirtualDeviceDefaultInput() else {
            virtualMicIsDefaultInput = false
            return false
        }
        let target = settings.selectedInputDeviceUID
        let moved = deviceManager.moveDefaultInputToPhysical(preferredUID: target)
        virtualMicIsDefaultInput = !moved
        return moved
    }

    private func refreshMeetingContextIfNeeded(now: Date) {
        guard activeCall != nil || bridgeHealth.consumerRunningCount > 0 else { return }
        guard now.timeIntervalSince(lastMeetingContextRefreshAt) >= meetingContextRefreshInterval else { return }
        lastMeetingContextRefreshAt = now
        callManager?.refreshActiveSessionContext()
    }

    func refreshCalls() {
        activeCall = callManager?.activeSession
        calls = callManager?.listSessions() ?? []
    }

    func stopActiveCall() {
        callManager?.stopActiveSession()
        refreshCalls()
    }

    func startManualRecording() {
        guard settings.recordingConsentAcknowledged else {
            withRecordingConsent { [weak self] in self?.startManualRecording() }
            return
        }
        guard enabled else {
            callStatusMessage = "Turn on Mic Processing first"
            return
        }
        guard isVirtualMicInUse else {
            callStatusMessage = "No active meeting detected"
            return
        }
        guard settings.localRecordingEnabled else {
            callStatusMessage = "Local recording is off"
            return
        }
        startPipelineIfNeeded(requestPermission: true)
        callManager?.startManualSession(settings: settings)
        callStatusMessage = ""
        refreshCalls()
    }

    func deleteCall(_ call: CallSession) {
        if playbackCallID == call.id {
            stopPlayback()
        }
        callManager?.deleteSession(id: call.id)
        refreshCalls()
    }

    func revealCall(_ call: CallSession) {
        let base = try? CallDataPaths.defaultPaths()
        let url = base?.directory(for: call.id) ?? URL(fileURLWithPath: call.localMicAudioPath ?? "")
        NSWorkspace.shared.activateFileViewerSelecting([url])
    }

    func revealCallsFolder() {
        guard let base = try? CallDataPaths.defaultPaths() else { return }
        NSWorkspace.shared.activateFileViewerSelecting([base.callsDirectory])
    }

    func playCall(_ call: CallSession) {
        if playbackCallID == call.id, let player = audioPlayer {
            if player.isPlaying {
                player.pause()
                playbackIsPlaying = false
                refreshPlaybackProgress()
            } else {
                player.play()
                playbackIsPlaying = true
                startPlaybackTimer()
            }
            return
        }

        stopPlayback()
        let paths = playableAudioPaths(for: call)
        guard !paths.isEmpty else {
            callStatusMessage = "Recording unavailable"
            return
        }

        for path in paths {
            guard let player = try? AVAudioPlayer(contentsOf: URL(fileURLWithPath: path)),
                  player.duration > 0 else {
                continue
            }
            player.prepareToPlay()
            audioPlayer = player
            playbackCallID = call.id
            playbackDuration = player.duration
            playbackElapsed = 0
            playbackIsPlaying = player.play()
            callStatusMessage = playbackIsPlaying ? "" : "Playback unavailable"
            startPlaybackTimer()
            return
        }
        callStatusMessage = "Playback unavailable"
        stopPlayback()
    }

    func stopPlayback() {
        playbackTimer?.invalidate()
        playbackTimer = nil
        audioPlayer?.stop()
        audioPlayer = nil
        playbackCallID = nil
        playbackElapsed = 0
        playbackDuration = 0
        playbackIsPlaying = false
    }

    func isPlaybackActive(for call: CallSession) -> Bool {
        playbackCallID == call.id
    }

    func playableAudioPath(for call: CallSession) -> String? {
        playableAudioPaths(for: call).first
    }

    private func playableAudioPaths(for call: CallSession) -> [String] {
        [call.mixedPreviewAudioPath, call.localMicAudioPath, call.remoteSystemAudioPath]
            .compactMap { $0 }
            .filter { isPlayableAudioPath($0) }
    }

    private func isPlayableAudioPath(_ path: String) -> Bool {
        guard FileManager.default.fileExists(atPath: path),
              ((try? FileManager.default.attributesOfItem(atPath: path)[.size] as? UInt64) ?? 0) > 44,
              let file = try? AVAudioFile(forReading: URL(fileURLWithPath: path)) else {
            return false
        }
        return file.length > 0
    }

    func playbackTimeText() -> String {
        "\(formatPlaybackTime(playbackElapsed)) / \(formatPlaybackTime(playbackDuration))"
    }

    func seekPlayback(to progress: Double) {
        guard let player = audioPlayer, player.duration > 0 else { return }
        let clampedProgress = min(max(progress, 0), 1)
        player.currentTime = player.duration * clampedProgress
        refreshPlaybackProgress()
    }

    private func startPlaybackTimer() {
        playbackTimer?.invalidate()
        let timer = Timer(timeInterval: 0.2, repeats: true) { [weak self] _ in
            Task { @MainActor in
                self?.refreshPlaybackProgress()
            }
        }
        timer.tolerance = 0.05
        playbackTimer = timer
        RunLoop.main.add(timer, forMode: .common)
    }

    private func refreshPlaybackProgress() {
        guard let player = audioPlayer else {
            stopPlayback()
            return
        }
        playbackElapsed = player.currentTime
        playbackDuration = player.duration
        playbackIsPlaying = player.isPlaying
        if !player.isPlaying, player.currentTime >= max(0, player.duration - 0.05) {
            stopPlayback()
        }
    }

    private func formatPlaybackTime(_ seconds: TimeInterval) -> String {
        guard seconds.isFinite, seconds > 0 else { return "0:00" }
        let rounded = Int(seconds.rounded())
        return String(format: "%d:%02d", rounded / 60, rounded % 60)
    }

    func callDurationText(_ call: CallSession) -> String {
        let end = call.endedAt ?? Date()
        let seconds = max(0, Int(end.timeIntervalSince(call.startedAt)))
        let minutes = seconds / 60
        let remainder = seconds % 60
        if minutes >= 60 {
            return "\(minutes / 60)h \(minutes % 60)m"
        }
        return "\(minutes)m \(remainder)s"
    }

    func callDateText(_ call: CallSession) -> String {
        Self.callDateFormatter.string(from: call.startedAt)
    }

    func callTimeText(_ call: CallSession) -> String {
        Self.callTimeFormatter.string(from: call.startedAt)
    }

    func callFileSizeText(path: String?) -> String {
        guard let path,
              let size = (try? FileManager.default.attributesOfItem(atPath: path))?[.size] as? UInt64 else {
            return "Missing"
        }
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        return formatter.string(fromByteCount: Int64(size))
    }

    private static let callDateFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .none
        return formatter
    }()

    private static let callTimeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateStyle = .none
        formatter.timeStyle = .short
        return formatter
    }()
}

private enum MeetingContextDetector {
    private struct Candidate {
        var title: String?
        var appName: String
        var score: Int
    }

    static func detect() -> CallSessionContext? {
        guard let windows = CGWindowListCopyWindowInfo(
            [.optionOnScreenOnly, .excludeDesktopElements],
            kCGNullWindowID
        ) as? [[String: Any]] else {
            return nil
        }

        let ownPID = Int32(ProcessInfo.processInfo.processIdentifier)
        var best: Candidate?

        for (index, window) in windows.enumerated() {
            let layer = (window[kCGWindowLayer as String] as? NSNumber)?.intValue ?? 0
            guard layer == 0 else { continue }

            let ownerPID = (window[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value ?? 0
            guard ownerPID != ownPID else { continue }

            let owner = clean(window[kCGWindowOwnerName as String] as? String)
            let windowTitle = clean(window[kCGWindowName as String] as? String)
            guard let recognition = recognize(owner: owner, windowTitle: windowTitle) else { continue }

            let title = meetingTitle(from: windowTitle, appName: recognition.appName)
            let score = recognition.score + (title == nil ? 0 : 25) - min(index, 30)
            let candidate = Candidate(title: title, appName: recognition.appName, score: score)
            if best == nil || candidate.score > best!.score {
                best = candidate
            }
        }

        guard let best else { return nil }
        return CallSessionContext(title: best.title, appName: best.appName)
    }

    private static func recognize(owner: String?, windowTitle: String?) -> (appName: String, score: Int)? {
        let ownerLower = owner?.lowercased() ?? ""
        let titleLower = windowTitle?.lowercased() ?? ""

        if ownerLower.contains("microsoft teams") || ownerLower == "teams" {
            return ("Microsoft Teams", 120)
        }
        if isGoogleMeetTitle(titleLower) {
            return ("Google Meet", 115)
        }
        if ownerLower.contains("facetime") {
            return ("FaceTime", 105)
        }
        if ownerLower == "calls" || ownerLower.contains("phone") {
            return ("Calls", 100)
        }
        if ownerLower.contains("zoom") {
            return ("Zoom", 100)
        }
        if ownerLower.contains("webex") {
            return ("Webex", 95)
        }
        if ownerLower.contains("slack"), titleLower.contains("huddle") || titleLower.contains("call") {
            return ("Slack", 85)
        }
        if ownerLower.contains("discord"), titleLower.contains("voice") || titleLower.contains("call") {
            return ("Discord", 80)
        }
        return nil
    }

    private static func isGoogleMeetTitle(_ lowercasedTitle: String) -> Bool {
        lowercasedTitle.contains("google meet")
            || lowercasedTitle.hasPrefix("meet -")
            || lowercasedTitle.contains("meet.google.com")
    }

    private static func meetingTitle(from rawTitle: String?, appName: String) -> String? {
        guard var title = clean(rawTitle) else { return nil }

        var changed = true
        while changed {
            changed = false
            for suffix in appSuffixes {
                if removeCaseInsensitiveSuffix(suffix, from: &title) {
                    changed = true
                }
            }
        }

        if appName == "Google Meet" {
            if removeCaseInsensitivePrefix("Meet - ", from: &title), looksLikeGoogleMeetCode(title) {
                return nil
            }
            _ = removeCaseInsensitivePrefix("Google Meet - ", from: &title)
        }

        title = clean(title) ?? ""
        guard !isGeneric(title, appName: appName) else { return nil }
        return title
    }

    private static func isGeneric(_ title: String, appName: String) -> Bool {
        let lower = title.lowercased()
        if lower.isEmpty || lower == appName.lowercased() { return true }
        if genericTitles.contains(lower) { return true }
        if lower.hasPrefix("meet -") { return true }
        if looksLikeGoogleMeetCode(lower) { return true }
        return false
    }

    private static func clean(_ value: String?) -> String? {
        guard let value else { return nil }
        let collapsed = value
            .components(separatedBy: .whitespacesAndNewlines)
            .filter { !$0.isEmpty }
            .joined(separator: " ")
        return collapsed.isEmpty ? nil : collapsed
    }

    private static func removeCaseInsensitiveSuffix(_ suffix: String, from title: inout String) -> Bool {
        guard let range = title.range(of: suffix, options: [.caseInsensitive, .backwards]),
              range.upperBound == title.endIndex else {
            return false
        }
        title.removeSubrange(range)
        title = clean(title) ?? ""
        return true
    }

    private static func removeCaseInsensitivePrefix(_ prefix: String, from title: inout String) -> Bool {
        guard let range = title.range(of: prefix, options: [.caseInsensitive, .anchored]) else {
            return false
        }
        title.removeSubrange(range)
        title = clean(title) ?? ""
        return true
    }

    private static func looksLikeGoogleMeetCode(_ value: String) -> Bool {
        let parts = value.lowercased().split(separator: "-")
        guard parts.count == 3,
              parts[0].count == 3,
              parts[1].count == 4,
              parts[2].count == 3 else {
            return false
        }
        return parts.allSatisfy { part in
            part.allSatisfy { character in
                character >= "a" && character <= "z"
            }
        }
    }

    private static let appSuffixes = [
        " | Microsoft Teams",
        " - Microsoft Teams",
        " – Microsoft Teams",
        " — Microsoft Teams",
        " | Teams",
        " - Teams",
        " - Google Meet",
        " | Google Meet",
        " – Google Meet",
        " — Google Meet",
        " - Google Chrome",
        " | Google Chrome",
        " - Microsoft Edge",
        " | Microsoft Edge",
        " - Safari",
        " | Safari",
        " - Arc",
        " | Arc",
        " - Brave Browser",
        " | Brave Browser",
        " - Firefox",
        " — Mozilla Firefox",
        " - Zoom",
        " | Zoom",
        " - Zoom Workplace",
        " | Zoom Workplace",
        " - Slack",
        " | Slack",
        " - Discord",
        " | Discord"
    ]

    private static let genericTitles: Set<String> = [
        "activity",
        "calendar",
        "call",
        "calls",
        "chat",
        "discord",
        "facetime",
        "google meet",
        "meet",
        "meeting",
        "microsoft teams",
        "slack",
        "teams",
        "webex",
        "zoom",
        "zoom workplace"
    ]
}

enum MainAppSection: String, CaseIterable, Identifiable {
    case calls
    case settings
    case diagnostics

    var id: String { rawValue }

    var title: String {
        switch self {
        case .calls: "Calls"
        case .settings: "Settings"
        case .diagnostics: "Diagnostics"
        }
    }

    var icon: BrandIcon {
        switch self {
        case .calls: .calls
        case .settings: .settings
        case .diagnostics: .diagnostics
        }
    }
}

// View-local UI state. Held in an ObservableObject rather than `@State` because
// on the macOS 27 SDK `@State` is a macro whose plugin ships only with full
// Xcode; `@StateObject` is still a plain property wrapper, so the app builds
// with just the Command Line Tools.
@MainActor
final class MainWindowUIState: ObservableObject {
    @Published var selectedSection: MainAppSection = .calls
    @Published var columnVisibility: NavigationSplitViewVisibility = .all
}

@MainActor
final class CallsLibraryUIState: ObservableObject {
    @Published var selectedCallID: UUID?
    @Published var searchText = ""
}

struct NoiselessMainWindow: View {
    @ObservedObject var model: MenuBarModel
    @StateObject private var ui = MainWindowUIState()

    var body: some View {
        // A native split view: on current macOS the sidebar floats as a glass
        // panel and the system adds the sidebar toggle to the toolbar.
        NavigationSplitView(columnVisibility: $ui.columnVisibility) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 9) {
                    PabulongAppMark(enabled: model.enabled, size: 24)
                    VStack(alignment: .leading, spacing: 0) {
                        Text("Pabulong")
                            .font(BrandFonts.body(14, .semibold))
                            .foregroundStyle(Brand.text)
                        Text(model.enabled ? model.virtualDeviceStatus : "Off")
                            .font(BrandFonts.body(11.5))
                            .foregroundStyle(Brand.secondaryText)
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 6)
                .padding(.top, 4)
                .padding(.bottom, 12)

                ForEach(MainAppSection.allCases) { section in
                    SidebarButton(
                        title: section.title,
                        icon: section.icon,
                        isSelected: ui.selectedSection == section
                    ) {
                        ui.selectedSection = section
                    }
                }

                Spacer()
            }
            .padding(.horizontal, 10)
            .navigationSplitViewColumnWidth(min: 190, ideal: 210, max: 260)
        } detail: {
            Group {
                switch ui.selectedSection {
                case .calls:
                    CallsLibraryView(model: model)
                case .settings:
                    AppSettingsView(model: model, updater: model.updater)
                case .diagnostics:
                    DiagnosticsView(model: model)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Brand.windowGlass.ignoresSafeArea())
        }
        .modifier(GlassWindowBackground())
        .foregroundStyle(Brand.text)
        .toolbar {
            ToolbarItemGroup {
                Toggle(isOn: Binding(
                    get: { model.enabled },
                    set: { model.setProcessingEnabled($0) }
                )) {
                    Label("Mic Processing", systemImage: model.enabled ? "mic.fill" : "mic.slash.fill")
                }
                .toggleStyle(.button)

                Button {
                    model.refreshDevices()
                    model.refreshCalls()
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
            }
        }
    }
}

struct SidebarButton: View {
    var title: String
    var icon: BrandIcon
    var isSelected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 9) {
                BrandIconView(icon: icon, size: 15)
                Text(title)
                    .font(BrandFonts.body(13.5, isSelected ? .medium : .regular))
                Spacer(minLength: 0)
            }
            .foregroundStyle(Brand.text)
            .frame(maxWidth: .infinity, minHeight: 30, alignment: .leading)
            .padding(.horizontal, 10)
            .background(isSelected ? Brand.sidebarSelection : Color.clear, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .contentShape(Rectangle())
        }
        .buttonStyle(BrandPlainButtonStyle(cornerRadius: 8))
        .help(title)
        .accessibilityLabel(title)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

struct CallsLibraryView: View {
    @ObservedObject var model: MenuBarModel
    @StateObject private var ui = CallsLibraryUIState()

    private var filteredCalls: [CallSession] {
        let query = ui.searchText.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard !query.isEmpty else { return model.calls }
        return model.calls.filter { call in
            call.title.lowercased().contains(query)
            || call.status.rawValue.lowercased().contains(query)
        }
    }

    private var selectedCall: CallSession? {
        guard let selectedCallID = ui.selectedCallID else { return model.activeCall ?? filteredCalls.first }
        if model.activeCall?.id == selectedCallID {
            return model.activeCall
        }
        return model.calls.first { $0.id == selectedCallID } ?? filteredCalls.first
    }

    private var visibleCalls: [(call: CallSession, isActive: Bool)] {
        var rows: [(CallSession, Bool)] = []
        if let activeCall = model.activeCall {
            rows.append((activeCall, true))
        }
        rows += filteredCalls.filter { $0.id != model.activeCall?.id }.map { ($0, false) }
        return rows
    }

    var body: some View {
        HStack(spacing: 0) {
            VStack(spacing: 0) {
                LibraryHeader(
                    title: "Calls",
                    subtitle: "\(model.calls.count) saved",
                    icon: .calls
                )

                BrandSearchField(placeholder: "Search calls", text: $ui.searchText)
                    .padding(.horizontal, 14)
                    .padding(.bottom, model.isVirtualMicInUse || model.activeCall != nil ? 10 : 12)

                if model.isVirtualMicInUse || model.activeCall != nil {
                    RecordingControlStrip(model: model, compact: false)
                        .padding(.horizontal, 14)
                        .padding(.bottom, 12)
                }

                if model.activeCall == nil && filteredCalls.isEmpty {
                    EmptyCallList()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        LazyVStack(spacing: 2) {
                            ForEach(visibleCalls, id: \.call.id) { row in
                                Button {
                                    ui.selectedCallID = row.call.id
                                } label: {
                                    CallRow(call: row.call, model: model, isActive: row.isActive)
                                        .padding(.horizontal, 10)
                                        .background(
                                            selectedCall?.id == row.call.id ? Brand.sidebarSelection : Color.clear,
                                            in: RoundedRectangle(cornerRadius: 8, style: .continuous)
                                        )
                                }
                                .buttonStyle(BrandPlainButtonStyle(cornerRadius: 8))
                                .accessibilityAddTraits(selectedCall?.id == row.call.id ? .isSelected : [])
                            }
                        }
                        .padding(.horizontal, 8)
                        .padding(.bottom, 10)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(width: 340)

            Rectangle().fill(Brand.separator).frame(width: 1)

            CallDetailView(
                model: model,
                call: selectedCall,
                onDelete: confirmDelete
            )
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onAppear {
            model.refreshCalls()
            ui.selectedCallID = ui.selectedCallID ?? model.activeCall?.id ?? model.calls.first?.id
        }
        .onChange(of: model.calls) { _, calls in
            if ui.selectedCallID == nil || !calls.contains(where: { $0.id == ui.selectedCallID }) {
                ui.selectedCallID = model.activeCall?.id ?? calls.first?.id
            }
        }
        .onChange(of: model.activeCall) { _, activeCall in
            if let activeCall {
                ui.selectedCallID = activeCall.id
            }
        }
    }

    private func confirmDelete(_ call: CallSession) {
        DispatchQueue.main.async {
            let choice = BrandAlert.run(
                title: "Delete call?",
                message: nil,
                choices: [
                    .init(title: "Delete", kind: .destructive),
                    .init(title: "Cancel", kind: .secondary, isCancel: true)
                ]
            )
            if choice == 0 {
                model.deleteCall(call)
                ui.selectedCallID = nil
            }
        }
    }
}

struct BrandSearchField: View {
    var placeholder: String
    @Binding var text: String

    var body: some View {
        HStack(spacing: 7) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(Brand.secondaryText)
            TextField(placeholder, text: $text)
                .textFieldStyle(.plain)
                .font(BrandFonts.body(13))
                .foregroundStyle(Brand.text)
        }
        .padding(.horizontal, 10)
        .frame(height: 30)
        .brandCard(cornerRadius: 10)
    }
}

struct CallDetailView: View {
    @ObservedObject var model: MenuBarModel
    var call: CallSession?
    var onDelete: (CallSession) -> Void

    var body: some View {
        Group {
            if let call {
                ScrollView {
                    VStack(alignment: .leading, spacing: 22) {
                        CallDetailHeader(model: model, call: call, onDelete: onDelete)
                        FilesPane(model: model, call: call)
                    }
                    .padding(24)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            } else {
                NoCallDetailPlaceholder()
            }
        }
    }
}

struct EmptyCallList: View {
    var body: some View {
        VStack(spacing: 10) {
            BrandIconView(icon: .calls, size: 30)
                .foregroundStyle(Brand.secondaryText)
            Text("No Calls")
                .font(BrandFonts.display(15, .medium))
                .foregroundStyle(Brand.text)
            Text("Recordings will appear here.")
                .font(BrandFonts.body(12))
                .foregroundStyle(Brand.secondaryText)
        }
        .multilineTextAlignment(.center)
        .padding(24)
    }
}

struct NoCallDetailPlaceholder: View {
    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "waveform")
                .font(.system(size: 40, weight: .light))
                .foregroundStyle(Brand.secondaryText)
            Text("Select a Call")
                .font(BrandFonts.display(18, .medium))
                .foregroundStyle(Brand.text)
            Text("Recordings and audio files open here.")
                .font(BrandFonts.body(13))
                .foregroundStyle(Brand.secondaryText)
        }
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// The rounded recording strip: blue ring icon, title and status, and the
/// record button (red stop while recording).
struct RecordingControlStrip: View {
    @ObservedObject var model: MenuBarModel
    var compact: Bool

    private var isRecording: Bool {
        model.activeCall != nil
    }

    var body: some View {
        HStack(spacing: compact ? 9 : 11) {
            BrandIconView(icon: .record, size: compact ? 22 : 26)
                .foregroundStyle(isRecording ? Color(nsColor: .systemRed) : Brand.accentInk)

            VStack(alignment: .leading, spacing: 1) {
                Text(model.recordingControlTitle)
                    .font(BrandFonts.body(compact ? 12.5 : 13.5, .semibold))
                    .foregroundStyle(Brand.text)
                    .lineLimit(1)
                Text(model.recordingControlSubtitle)
                    .font(BrandFonts.body(compact ? 11 : 11.5))
                    .foregroundStyle(Brand.secondaryText)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            Button {
                if isRecording {
                    model.stopActiveCall()
                } else {
                    model.startManualRecording()
                }
            } label: {
                if compact {
                    Group {
                        if isRecording {
                            Image(systemName: "stop.fill").font(.system(size: 11, weight: .bold))
                        } else {
                            BrandIconView(icon: .record, size: 17)
                        }
                    }
                    .frame(width: 38, height: 29)
                } else {
                    HStack(spacing: 6) {
                        if isRecording {
                            Image(systemName: "stop.fill").font(.system(size: 10, weight: .bold))
                        } else {
                            BrandIconView(icon: .record, size: 15)
                        }
                        Text(isRecording ? "Stop" : "Record")
                    }
                    .frame(minWidth: 64)
                }
            }
            .buttonStyle(BrandButtonStyle(kind: isRecording ? .destructive : .primary, height: compact ? 29 : 30))
            .disabled(!isRecording && !model.canStartManualRecording)
            .help(isRecording ? "Stop recording" : "Start recording")
            .accessibilityLabel(isRecording ? "Stop recording" : "Start recording")
        }
        .padding(.horizontal, compact ? 9 : 11)
        .padding(.vertical, compact ? 7 : 9)
        .background(Brand.strip, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

struct CallDetailHeader: View {
    @ObservedObject var model: MenuBarModel
    var call: CallSession
    var onDelete: (CallSession) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 14) {
                Group {
                    if call.status == .recording {
                        BrandIconView(icon: .record, size: 24)
                            .foregroundStyle(Color(nsColor: .systemRed))
                    } else {
                        Image(systemName: "waveform")
                            .font(.system(size: 18, weight: .medium))
                            .foregroundStyle(Brand.accentInk)
                    }
                }
                .frame(width: 44, height: 44)
                .brandCard(cornerRadius: 12)

                VStack(alignment: .leading, spacing: 5) {
                    Text(call.title)
                        .font(BrandFonts.display(20, .medium))
                        .foregroundStyle(Brand.text)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 12) {
                        StatusPill(
                            text: call.status.rawValue.capitalized,
                            systemImage: "circle.fill",
                            tint: call.status == .recording ? Color(nsColor: .systemRed) : Brand.secondaryText
                        )
                        if call.remoteAudioMissing {
                            StatusPill(text: "Mic Only", systemImage: "mic.fill")
                        }
                    }
                    Text("\(model.callDateText(call)) at \(model.callTimeText(call)) · \(model.callDurationText(call))")
                        .font(BrandFonts.body(13))
                        .foregroundStyle(Brand.secondaryText)
                }

                Spacer()

                HStack(spacing: 4) {
                    if call.status == .recording {
                        HeaderIconButton(title: "Stop Recording", systemImage: "stop.circle.fill", role: .destructive) {
                            model.stopActiveCall()
                        }
                    }
                    HeaderIconButton(
                        title: model.isPlaybackActive(for: call) && model.playbackIsPlaying ? "Pause" : "Play",
                        systemImage: model.isPlaybackActive(for: call) && model.playbackIsPlaying ? "pause.fill" : "play.fill",
                        isDisabled: model.playableAudioPath(for: call) == nil
                    ) {
                        model.playCall(call)
                    }
                    if model.isPlaybackActive(for: call) {
                        HeaderIconButton(title: "Stop", systemImage: "stop.fill") {
                            model.stopPlayback()
                        }
                    }
                    HeaderIconButton(title: "Reveal", systemImage: "folder") {
                        model.revealCall(call)
                    }
                    HeaderIconButton(title: "Delete", systemImage: "trash", role: .destructive) {
                        onDelete(call)
                    }
                }
                .fixedSize(horizontal: true, vertical: false)
            }

            if model.isPlaybackActive(for: call) {
                PlaybackStrip(model: model)
            }

            if !model.callStatusMessage.isEmpty {
                Text(model.callStatusMessage)
                    .font(BrandFonts.body(12))
                    .foregroundStyle(Brand.secondaryText)
            }
        }
    }
}

struct PlaybackStrip: View {
    @ObservedObject var model: MenuBarModel

    private var progress: Double {
        guard model.playbackDuration > 0 else { return 0 }
        return min(max(model.playbackElapsed / model.playbackDuration, 0), 1)
    }

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: model.playbackIsPlaying ? "speaker.wave.2.fill" : "pause.circle")
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(Brand.secondaryText)
                .frame(width: 18)
            SeekablePlaybackBar(progress: progress) { newProgress in
                model.seekPlayback(to: newProgress)
            }
            .frame(maxWidth: 280)
            Text(model.playbackTimeText())
                .font(BrandFonts.body(12).monospacedDigit())
                .foregroundStyle(Brand.secondaryText)
                .frame(width: 82, alignment: .trailing)
        }
        .padding(.horizontal, 11)
        .padding(.vertical, 9)
        .background(Brand.strip, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Playback \(model.playbackTimeText())")
    }
}

struct SeekablePlaybackBar: View {
    let progress: Double
    let onSeek: (Double) -> Void

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(Brand.meterTrack)
                Capsule()
                    .fill(Brand.signal)
                    .frame(width: max(5, proxy.size.width * progress))
            }
            .frame(height: 5)
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onEnded { value in
                        let width = max(proxy.size.width, 1)
                        onSeek(Double(value.location.x / width))
                    }
            )
        }
        .frame(height: 14)
    }
}

struct FilesPane: View {
    @ObservedObject var model: MenuBarModel
    var call: CallSession

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            BrandSection(title: "Audio") {
                FilePathRow(title: "Local Mic", path: call.localMicAudioPath, size: model.callFileSizeText(path: call.localMicAudioPath))
                FilePathRow(title: "Remote Audio", path: call.remoteSystemAudioPath, size: model.callFileSizeText(path: call.remoteSystemAudioPath))
                FilePathRow(title: "Mixed Preview", path: call.mixedPreviewAudioPath, size: model.callFileSizeText(path: call.mixedPreviewAudioPath), showsDivider: false)
            }
            Button {
                model.revealCall(call)
            } label: {
                Label("Reveal in Finder", systemImage: "folder")
            }
            .buttonStyle(BrandButtonStyle(kind: .secondary))
        }
        .frame(maxWidth: 720, alignment: .leading)
    }
}

enum LegalLinks {
    static let privacy = URL(string: "https://nextmatterpro.com/training/pabulong-privacy.php")!
    static let terms = URL(string: "https://nextmatterpro.com/training/pabulong-terms.php")!
}

/// Bundled third-party license notices (required by RNNoise's BSD license).
struct ThirdPartyNoticesView: View {
    @Environment(\.dismiss) private var dismiss

    private var text: String {
        if let url = Bundle.main.url(forResource: "ThirdPartyNotices", withExtension: "md"),
           let notices = try? String(contentsOf: url, encoding: .utf8) {
            return notices
        }
        return "Third-party notices are included with the app in Contents/Resources/ThirdPartyNotices.md."
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Third-Party Notices")
                .font(BrandFonts.display(18, .medium))
                .foregroundStyle(Brand.text)
            ScrollView {
                Text(text)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(Brand.text)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(12)
            }
            .brandCard(cornerRadius: 12)
            HStack {
                Spacer()
                Button("Done") { dismiss() }
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(BrandButtonStyle(kind: .primary))
            }
        }
        .padding(20)
        .frame(width: 560, height: 520)
        .background(Brand.window)
    }
}

struct AppSettingsView: View {
    @ObservedObject var model: MenuBarModel
    @ObservedObject var updater: AppUpdater

    private var microphoneLabel: String {
        guard let uid = model.settings.selectedInputDeviceUID else { return "System Default" }
        guard let device = model.devices.first(where: { $0.uid == uid }) else { return "" }
        return device.isDefault ? "\(device.name) (Default)" : device.name
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                BrandSection(title: "Audio") {
                    BrandRow(title: "Mic Processing") {
                        BrandSwitch(title: "Mic Processing", isOn: Binding(
                            get: { model.enabled },
                            set: { model.setProcessingEnabled($0) }
                        ))
                    }
                    BrandRow(title: "Microphone") {
                        BrandPopUp(title: "Microphone", selection: $model.settings.selectedInputDeviceUID, valueLabel: microphoneLabel) {
                            Text("System Default").tag(String?.none)
                            ForEach(model.devices) { device in
                                Text(device.isDefault ? "\(device.name) (Default)" : device.name)
                                    .tag(Optional(device.uid))
                            }
                        }
                    }
                    BrandRow(title: "Suppression") {
                        BrandPopUp(title: "Suppression", selection: $model.settings.suppressionMode, valueLabel: model.settings.suppressionMode.displayName) {
                            ForEach(SuppressionMode.allCases, id: \.self) { mode in
                                Text(mode.displayName).tag(mode)
                            }
                        }
                    }
                    if NoiseProcessorFactory.usesSharedCore {
                        BrandRow(
                            title: "Echo Cancellation",
                            caption: "Removes the other person's voice from your mic when you use speakers instead of headphones."
                        ) {
                            BrandSwitch(title: "Echo Cancellation", isOn: $model.settings.echoCancellationEnabled)
                        }
                        BrandRow(
                            title: "Background Voices",
                            caption: "Turns down people talking near you who aren't on the call. Strong also trims your own softest speech."
                        ) {
                            BrandPopUp(title: "Background Voices", selection: $model.settings.voiceIsolation, valueLabel: model.settings.voiceIsolation.displayName) {
                                ForEach(VoiceIsolationLevel.allCases, id: \.self) { level in
                                    Text(level.displayName).tag(level)
                                }
                            }
                        }
                    }
                    BrandRow(title: "Bypass", showsDivider: false) {
                        BrandSwitch(title: "Bypass", isOn: $model.settings.bypassEnabled)
                    }
                }

                BrandSection(title: "Recording") {
                    BrandRow(title: "Automatic Call Recording") {
                        BrandSwitch(title: "Automatic Call Recording", isOn: Binding(
                            get: { model.settings.automaticCallRecordingEnabled },
                            set: { on in
                                if on {
                                    model.withRecordingConsent { model.settings.automaticCallRecordingEnabled = true }
                                } else {
                                    model.settings.automaticCallRecordingEnabled = false
                                }
                            }
                        ))
                    }
                    if model.settings.automaticCallRecordingEnabled && !model.settings.recordingConsentAcknowledged {
                        BrandCustomRow {
                            HStack {
                                Text("Paused until you confirm recording consent.")
                                    .font(BrandFonts.body(12))
                                    .foregroundStyle(.orange)
                                Spacer()
                                Button("Review") { model.withRecordingConsent {} }
                                    .buttonStyle(BrandButtonStyle(kind: .secondary, height: 26))
                            }
                        }
                    }
                    BrandRow(title: "Local Recording") {
                        BrandSwitch(title: "Local Recording", isOn: $model.settings.localRecordingEnabled)
                    }
                    BrandRow(title: "Capture System Audio") {
                        BrandSwitch(title: "Capture System Audio", isOn: $model.settings.captureSystemAudioEnabled)
                    }
                    if model.settings.captureSystemAudioEnabled {
                        BrandCustomRow {
                            HStack(alignment: .firstTextBaseline, spacing: 7) {
                                Image(systemName: "checkmark.circle.fill")
                                    .font(.system(size: 12))
                                    .foregroundStyle(Brand.live)
                                Text("Captures the other side of the call via Core Audio — no Screen Recording permission needed.")
                                    .font(BrandFonts.body(11.5))
                                    .foregroundStyle(Brand.secondaryText)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    BrandCustomRow(showsDivider: false) {
                        Button {
                            model.revealCallsFolder()
                        } label: {
                            Label("Open Call Storage", systemImage: "folder")
                                .font(BrandFonts.body(13.5, .medium))
                                .foregroundStyle(Brand.accentInk)
                        }
                        .buttonStyle(.plain)
                    }
                }

                BrandSection(title: "System") {
                    BrandRow(title: "Launch at Login", showsDivider: false) {
                        BrandSwitch(title: "Launch at Login", isOn: $model.settings.launchAtLogin)
                    }
                }

                BrandSection(title: "About") {
                    BrandRow(title: "Version") {
                        HStack(spacing: 14) {
                            Text(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev")
                                .font(BrandFonts.body(13.5))
                                .foregroundStyle(Brand.secondaryText)
                            if updater.isAvailable {
                                Button("Check for Updates…") { updater.checkForUpdates() }
                                    .buttonStyle(BrandButtonStyle(kind: .secondary, height: 26))
                                    .disabled(!updater.canCheckForUpdates)
                            }
                        }
                    }
                    if updater.isAvailable {
                        BrandRow(
                            title: "Automatically Check for Updates",
                            caption: "Once a day. A check sends your IP address, the Pabulong version and your language setting to vastarterhub.com. Installing an update asks for your Mac password."
                        ) {
                            BrandSwitch(title: "Automatically Check for Updates", isOn: Binding(
                                get: { updater.automaticallyChecksForUpdates },
                                set: { updater.setAutomaticallyChecksForUpdates($0) }
                            ))
                        }
                    }
                    BrandCustomRow {
                        Text("© 2026 Next Matter Pro. Open source under the MIT License.")
                            .font(BrandFonts.body(12))
                            .foregroundStyle(Brand.secondaryText)
                    }
                    BrandCustomRow(showsDivider: false) {
                        HStack(spacing: 18) {
                            Link("Privacy Notice", destination: LegalLinks.privacy)
                            Link("Terms of Use", destination: LegalLinks.terms)
                            Button("Third-Party Notices") { model.showingThirdPartyNotices = true }
                                .buttonStyle(.plain)
                        }
                        .font(BrandFonts.body(13, .medium))
                        .foregroundStyle(Brand.accentInk)
                        .tint(Brand.accentInk)
                    }
                }
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .frame(maxWidth: 760, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .navigationTitle("Settings")
        .sheet(isPresented: $model.showingThirdPartyNotices) { ThirdPartyNoticesView().preferredColorScheme(.light) }
    }
}

struct DiagnosticsView: View {
    @ObservedObject var model: MenuBarModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                LibraryHeader(title: "Diagnostics", subtitle: model.virtualDeviceStatus, icon: .diagnostics)
                    .padding(.horizontal, -14)

                Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 14) {
                    GridRow {
                        MetricTile(title: "Input", value: model.processorMetrics.inputRMS)
                        MetricTile(title: "Output", value: model.processorMetrics.outputRMS)
                    }
                    GridRow {
                        InfoTile(title: "Engine", value: model.engineName, systemImage: "cpu")
                        InfoTile(title: "Mic Clients", value: "\(model.bridgeHealth.consumerRunningCount)", systemImage: "person.wave.2")
                    }
                    GridRow {
                        InfoTile(title: "Latency", value: model.latencyLabel, systemImage: "timer")
                        InfoTile(title: "Virtual Mic", value: model.virtualDeviceStatus, systemImage: "dot.radiowaves.left.and.right")
                    }
                    if NoiseProcessorFactory.usesSharedCore {
                        GridRow {
                            InfoTile(title: "Echo Cancellation", value: model.echoStatusText, systemImage: "speaker.wave.2")
                            InfoTile(title: "Echo Path", value: model.echoPathText, systemImage: "arrow.triangle.2.circlepath")
                        }
                        GridRow {
                            InfoTile(title: "Background Voices", value: model.voiceGateText, systemImage: "person.2.wave.2")
                            InfoTile(title: "Speaker Reference", value: model.echoReferenceActive ? "Tapping system output" : "Not running", systemImage: "waveform")
                        }
                    }
                }

                if NoiseProcessorFactory.usesSharedCore {
                    VStack(alignment: .leading, spacing: 8) {
                        Button {
                            model.runSpeakerEchoTest()
                        } label: {
                            Label(model.echoTestRunning ? "Testing…" : "Run Speaker Echo Test", systemImage: "speaker.wave.3")
                        }
                        .buttonStyle(BrandButtonStyle(kind: .primary))
                        .disabled(model.echoTestRunning)
                        Text(model.echoTestStatus.isEmpty
                             ? "Plays a sentence through your speakers (no headphones) and measures how much of it Pabulong removes from your mic. Not available during a call."
                             : model.echoTestStatus)
                            .font(BrandFonts.body(11.5))
                            .foregroundStyle(Brand.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: 674, alignment: .leading)
                }

                HStack(spacing: 10) {
                    Button {
                        model.refreshDevices()
                        model.refreshCalls()
                    } label: {
                        Label("Refresh", systemImage: "arrow.clockwise")
                    }
                    Button {
                        model.exportDiagnostics()
                    } label: {
                        Label("Export Diagnostics", systemImage: "square.and.arrow.up")
                    }
                }
                .buttonStyle(BrandButtonStyle(kind: .secondary))
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 12)
            .frame(maxWidth: 760, alignment: .leading)
        }
        .navigationTitle("Diagnostics")
    }
}

struct LibraryHeader: View {
    var title: String
    var subtitle: String
    var icon: BrandIcon

    var body: some View {
        HStack(spacing: 11) {
            BrandIconView(icon: icon, size: 18)
                .foregroundStyle(Brand.accentInk)
                .frame(width: 34, height: 34)
                .brandCard(cornerRadius: 10)
            VStack(alignment: .leading, spacing: 1) {
                Text(title)
                    .font(BrandFonts.display(17, .medium))
                    .foregroundStyle(Brand.text)
                Text(subtitle)
                    .font(BrandFonts.body(12))
                    .foregroundStyle(Brand.secondaryText)
            }
            Spacer()
        }
        .padding(14)
    }
}

struct CallRow: View {
    var call: CallSession
    @ObservedObject var model: MenuBarModel
    var isActive: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 9) {
            Group {
                if isActive {
                    BrandIconView(icon: .record, size: 15)
                        .foregroundStyle(Color(nsColor: .systemRed))
                } else {
                    Image(systemName: "waveform")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Brand.secondaryText)
                }
            }
            .frame(width: 18, height: 18)
            VStack(alignment: .leading, spacing: 3) {
                Text(call.title)
                    .font(BrandFonts.body(13.5, .medium))
                    .foregroundStyle(Brand.text)
                    .lineLimit(1)
                Text("\(model.callDateText(call)) · \(model.callDurationText(call))")
                    .font(BrandFonts.body(12))
                    .foregroundStyle(Brand.secondaryText)
                    .lineLimit(1)
            }
        }
        .padding(.vertical, 9)
        .frame(maxWidth: .infinity, minHeight: 52, alignment: .leading)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

struct HeaderIconButton: View {
    var title: String
    var systemImage: String
    var role: ButtonRole?
    var isDisabled: Bool
    var action: () -> Void

    init(
        title: String,
        systemImage: String,
        role: ButtonRole? = nil,
        isDisabled: Bool = false,
        action: @escaping () -> Void
    ) {
        self.title = title
        self.systemImage = systemImage
        self.role = role
        self.isDisabled = isDisabled
        self.action = action
    }

    var body: some View {
        Button(role: role, action: action) {
            Image(systemName: systemImage)
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(role == .destructive ? Color(nsColor: .systemRed) : Brand.text)
                .frame(width: 32, height: 32)
                .contentShape(Rectangle())
        }
        .buttonStyle(BrandPlainButtonStyle())
        .disabled(isDisabled)
        .help(title)
        .accessibilityLabel(title)
    }
}

struct FilePathRow: View {
    var title: String
    var path: String?
    var size: String
    var showsDivider = true

    var body: some View {
        BrandRow(title: title, showsDivider: showsDivider) {
            VStack(alignment: .trailing, spacing: 3) {
                Text(size)
                    .font(BrandFonts.body(13))
                    .foregroundStyle(Brand.secondaryText)
                Text(path ?? "Missing")
                    .font(.system(size: 10.5, design: .monospaced))
                    .foregroundStyle(Brand.secondaryText)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
            }
            .frame(maxWidth: 380, alignment: .trailing)
        }
    }
}

struct MetricTile: View {
    var title: String
    var value: Float

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(BrandFonts.body(14, .medium))
                .foregroundStyle(Brand.text)
            LevelMeter(value: value)
                .frame(width: 240)
            Text(String(format: "%.3f", value))
                .font(BrandFonts.body(11.5).monospacedDigit())
                .foregroundStyle(Brand.secondaryText)
        }
        .padding(14)
        .frame(width: 330, alignment: .leading)
        .brandCard(cornerRadius: 12)
    }
}

struct InfoTile: View {
    var title: String
    var value: String
    var systemImage: String

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: systemImage)
                .font(.system(size: 17, weight: .regular))
                .foregroundStyle(Brand.accentInk)
                .frame(width: 30, height: 30)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(BrandFonts.body(11.5))
                    .foregroundStyle(Brand.secondaryText)
                Text(value)
                    .font(BrandFonts.body(14, .medium))
                    .foregroundStyle(Brand.text)
                    .lineLimit(1)
            }
            Spacer()
        }
        .padding(14)
        .frame(width: 330, alignment: .leading)
        .brandCard(cornerRadius: 12)
    }
}

/// Small status label (dot or glyph plus text), no capsule.
struct StatusPill: View {
    var text: String
    var systemImage: String
    var tint: Color = Brand.secondaryText

    var body: some View {
        HStack(spacing: 5) {
            Image(systemName: systemImage)
                .font(.system(size: systemImage == "circle.fill" ? 6 : 10, weight: .bold))
                .foregroundStyle(tint)
            Text(text)
                .lineLimit(1)
                .truncationMode(.tail)
        }
        .font(BrandFonts.body(12, .semibold))
        .foregroundStyle(Brand.secondaryText)
        .fixedSize(horizontal: false, vertical: true)
    }
}

struct StatusDot: View {
    var active: Bool

    var body: some View {
        Circle()
            .fill(active ? Brand.live : Brand.secondaryText.opacity(0.45))
            .frame(width: 9, height: 9)
            .accessibilityLabel(active ? "Active" : "Inactive")
    }
}

struct LevelMeter: View {
    var value: Float

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(Brand.meterTrack)
                Capsule()
                    .fill(value > 0.04 ? Brand.signal : Brand.secondaryText.opacity(0.6))
                    .frame(width: max(4, proxy.size.width * CGFloat(min(value * 12, 1))))
            }
        }
        .frame(height: 8)
        .accessibilityLabel("Level")
    }
}
