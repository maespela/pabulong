import Combine
import Sparkle
import SwiftUI

/// In-app updates through Sparkle 2.
///
/// Pabulong contacts the update server only when the user chooses Check for
/// Updates, or after they turn on automatic checks (Sparkle asks on the second
/// launch, because Info.plist leaves SUEnableAutomaticChecks unset). No system
/// profile is sent. The feed and every installer are EdDSA-signed with the key
/// in Info.plist (SUPublicEDKey), and the installer is a pkg because it also
/// updates the Pabulong Mic driver, so macOS asks for an admin password.
@MainActor
final class AppUpdater: NSObject, ObservableObject, SPUUpdaterDelegate {
    @Published private(set) var canCheckForUpdates = false
    @Published private(set) var automaticallyChecksForUpdates = false

    /// False until the user has agreed to the current terms, so Sparkle's
    /// permission prompt never lands on top of the agreement.
    var mayAskPermission: () -> Bool = { true }

    private var controller: SPUStandardUpdaterController?

    /// False for a bare `swift run` build, which has no feed or key in its Info.plist.
    var isAvailable: Bool { controller != nil }

    override init() {
        super.init()
        guard Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") != nil else { return }
        let controller = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: self,
            userDriverDelegate: nil
        )
        self.controller = controller
        controller.updater.publisher(for: \.canCheckForUpdates)
            .assign(to: &$canCheckForUpdates)
        controller.updater.publisher(for: \.automaticallyChecksForUpdates)
            .assign(to: &$automaticallyChecksForUpdates)
    }

    func checkForUpdates() {
        controller?.checkForUpdates(nil)
    }

    func setAutomaticallyChecksForUpdates(_ on: Bool) {
        controller?.updater.automaticallyChecksForUpdates = on
    }

    // MARK: SPUUpdaterDelegate

    func updaterShouldPromptForPermissionToCheck(forUpdates updater: SPUUpdater) -> Bool {
        // Sparkle asks again on a later launch if we decline now.
        mayAskPermission()
    }
}

/// "Check for Updates…" in the app menu.
struct CheckForUpdatesCommand: View {
    @ObservedObject var updater: AppUpdater

    var body: some View {
        Button("Check for Updates…") { updater.checkForUpdates() }
            .disabled(!updater.canCheckForUpdates)
    }
}
