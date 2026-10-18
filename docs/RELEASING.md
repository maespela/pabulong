# Releasing Pabulong for Mac

The public download is one file, `Pabulong-<version>-arm64.pkg`. It installs
`/Applications/Pabulong.app` and the Pabulong Mic driver
(`/Library/Audio/Plug-Ins/HAL/PabulongHALPlugin.driver`), removes the old
`NoiselessHALPlugin.driver`, and shows a welcome page and the license before it
installs.

A download from a website must be signed with a **Developer ID** certificate and
**notarized** by Apple. Without that, macOS 15 and later refuse to open the
installer, and the person has to approve it in System Settings › Privacy &
Security, which most people won't do.

## Releasing before you have Developer ID

Manila Paper is published without Developer ID, and Pabulong can ship the same
way until the certificates exist. Sign the app and driver with the Apple
Development certificate, which keeps Pabulong's microphone permission across
updates and lets Core Audio load the driver:

```sh
./Scripts/build-rust-core.sh
SIGN_IDENTITY="Apple Development: mespela00@gmail.com (BV9Q58TJK8)" ./Scripts/package-offline-full-installer.sh
```

The package itself stays unsigned, because an Apple Development certificate
can't sign packages. The first time someone opens it, macOS refuses. They then
click **Open Anyway** in System Settings › Privacy & Security, once. The
download pages explain this step, using the same wording as the Manila Paper
page. Installer doesn't mark the installed app as downloaded, so the app opens
without a second warning.

Before you announce it, install the downloaded file on a second Mac that
never had Pabulong. Check that Pabulong Mic appears in System Settings › Sound
› Input. This Mac trusts your own certificate, so it can't confirm that
another Mac loads the driver.

When you have Developer ID, follow the rest of this guide and remove the
Open Anyway steps from both download pages.

## One-time setup

You do these steps yourself. They need your Apple Account password, and only
the Account Holder of the developer team can create Developer ID certificates.

### 1. Confirm the paid Apple Developer Program

Sign in at <https://developer.apple.com/account>. Under **Membership details**
the program must say **Apple Developer Program** with an expiry date (USD 99 a
year). A free account ("Personal Team") cannot create Developer ID certificates.

This Mac's current certificate belongs to team **K32XPPW99M** (organization
name "Marco Espela"). Two things to decide before you continue:

- **Who appears as the publisher.** Installer and Gatekeeper show the name on
  the Developer ID certificate. An individual membership shows
  "Marco Espela". To show "Next Matter Pro", the membership must be an
  organization enrollment, which needs a D-U-N-S number for the business.
- If you enroll a new organization team, use its team ID in the steps below
  instead of K32XPPW99M.

### 2. Create the two Developer ID certificates

You need both certificates:

- **Developer ID Application** signs the app and the driver.
- **Developer ID Installer** signs the `.pkg`.

**With Xcode** (if it's installed): open Xcode › Settings › Accounts, select
your Apple Account, select the team, and click **Manage Certificates…**. Click
**+** and choose **Developer ID Application**, then click **+** again and
choose **Developer ID Installer**.

**Without Xcode** (this Mac only has the Command Line Tools):

1. Open **Keychain Access** › menu **Keychain Access › Certificate Assistant ›
   Request a Certificate From a Certificate Authority…**. Enter your email,
   choose **Saved to disk**, and save the `.certSigningRequest` file.
2. Go to <https://developer.apple.com/account/resources/certificates/add>.
   Choose **Developer ID Application**, select the **G2 Sub-CA** profile type,
   upload the request file, and download the `.cer`.
3. Repeat step 2 for **Developer ID Installer** with the same request file.
4. Double-click both `.cer` files to add them to the **login** keychain.

Check that both certificates are installed:

```sh
security find-identity -v | grep "Developer ID"
```

You should see two lines, one `Developer ID Application: …` and one
`Developer ID Installer: …`. If a certificate is missing from the list, or
macOS says it is "not trusted", install the **Developer ID - G2** intermediate
certificate from <https://www.apple.com/certificateauthority/> and check again.

Back up both certificates with their private keys: in Keychain Access, select
them, choose **File › Export Items…**, and save a password-protected `.p12`
somewhere safe. You can't download the private key again later.

### 3. Store the notarization credentials

1. Create an app-specific password at <https://account.apple.com> › Sign-In and
   Security › App-Specific Passwords. Name it `pabulong-notary`.
2. Run this in Terminal with your Apple Account email. It asks for the
   app-specific password and saves it in the keychain as the profile
   `pabulong-notary`:

```sh
xcrun notarytool store-credentials pabulong-notary --apple-id YOUR-APPLE-ACCOUNT-EMAIL --team-id K32XPPW99M
```

Check the profile works:

```sh
xcrun notarytool history --keychain-profile pabulong-notary
```

## Each release

1. Set the version in the `VERSION` file, for example `0.1.1`. The app, the
   driver, the installer and the About screen all read it from there.
2. Build the signed, notarized and stapled installer:

```sh
./Scripts/release.sh
```

`release.sh` rebuilds the Rust core, finds both Developer ID certificates in
the keychain, and runs `Scripts/package-offline-full-installer.sh` with
`HARDENED=1 NOTARIZE=1 NOTARY_PROFILE=pabulong-notary`. That script:

- signs the helper, the app (with `Sources/NoiselessApp/Noiseless.entitlements`)
  and the driver with the hardened runtime and a secure timestamp;
- notarizes the app and the driver, and staples a ticket to each, so they pass
  Gatekeeper after install even when the Mac is offline;
- builds the `.pkg`, signs it with Developer ID Installer, notarizes it, and
  staples its ticket;
- writes `Pabulong-<version>-arm64.pkg.sha256` and `release.json` (size and
  SHA-256 for the download pages);
- runs `Scripts/verify-installer.sh` and stops if any check fails.

The output is in `outputs/offline-full/Pabulong-<version>-<timestamp>/`. If
notarization is rejected, Apple's log is saved there as `notary-*.json`.

3. Test it on a Mac (or a second user account) that never had Pabulong:
   download the `.pkg` from where you'll host it, open it, and install. The
   installer must open without any warning. After install, open Pabulong and
   check that Pabulong Mic appears in System Settings › Sound › Input.
4. Upload the `.pkg` and fill in the download pages' file name, size and
   SHA-256 from `release.json`.

## What Developer ID changes

A build signed with the Apple Development certificate (the only one on this
Mac now) is complete in every other respect: hardened runtime, timestamp,
entitlements, arm64 only, the Rust core linked, the license and welcome pages,
and non-relocatable bundles. `verify-installer.sh` reports these 8 checks
failing on that build. All of them pass once you complete the setup above:

| Check | Apple Development build | Developer ID release |
|---|---|---|
| `pkgutil --check-signature` | `Status: no signature` (an Apple Development certificate can't sign packages) | Signed by `Developer ID Installer: …`, "Notarization: trusted by the Apple notary service" |
| `spctl -a -vv -t install` on the pkg | `rejected source=no usable signature` | `accepted source=Notarized Developer ID` |
| `spctl -a -vv -t exec` on the app | `rejected origin=Apple Development: …` | `accepted source=Notarized Developer ID` |
| `codesign -dv` authority (app, driver) | `Apple Development: mespela00@gmail.com` | `Developer ID Application: …` |
| `stapler validate` (pkg, app, driver) | no ticket | ticket stapled |

`codesign --verify --deep --strict` already passes for the app and the driver,
and it still passes after the switch.

## Commands

| Command | What it does |
|---|---|
| `./Scripts/release.sh` | Builds the public release (Developer ID, notarized, stapled). |
| `./Scripts/package-offline-full-installer.sh` | Builds a QA installer. Ad-hoc signed by default; set `SIGN_IDENTITY` to sign with another certificate. |
| `./Scripts/verify-installer.sh <pkg>` | Checks a built installer without installing it. |
| `sudo /Applications/Pabulong.app/Contents/Resources/uninstall.sh` | Removes the app and driver. Keeps settings and recordings unless you add `--delete-data`. |

A QA build signed with the Apple Development certificate:

```sh
SIGN_IDENTITY="Apple Development: mespela00@gmail.com (BV9Q58TJK8)" ./Scripts/package-offline-full-installer.sh
```

Install it and test the uninstaller from the command line:

```sh
sudo installer -pkg outputs/offline-full/<folder>/Pabulong-<version>-arm64.pkg -target /
sudo /Applications/Pabulong.app/Contents/Resources/uninstall.sh
```
