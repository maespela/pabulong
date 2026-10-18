import Foundation

/// The release version, read from Pabulong.app's Info.plist. The packaging
/// scripts write that plist from the repo's VERSION file, so the number is set
/// in one place. Unbundled runs (`swift run`) report "dev".
public enum AppVersion {
    public static let current: String = {
        if let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String {
            return version
        }
        // PabulongHelper ships at Pabulong.app/Contents/Resources/PabulongHelper,
        // where Bundle.main is not the app. Read the enclosing app's plist.
        let contents = Bundle.main.executableURL?
            .resolvingSymlinksInPath()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        if let plist = contents?.appendingPathComponent("Info.plist"),
           let info = NSDictionary(contentsOf: plist),
           let version = info["CFBundleShortVersionString"] as? String {
            return version
        }
        return "dev"
    }()
}
