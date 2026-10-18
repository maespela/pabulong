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

## In-app updates

Pabulong checks for updates with [Sparkle 2](https://sparkle-project.org)
(pinned in `Package.swift`, embedded in `Contents/Frameworks`, and signed with
the app's certificate by the packaging scripts).

- **Feed:** `https://vastarterhub.com/downloads/pabulong/appcast.xml`. The
  master copy is `release/appcast.xml` in this repository. It lists the newest
  few releases, each with its version, size, release notes and EdDSA signature,
  and the feed itself is signed too.
- **Update file:** the same `Pabulong-<version>-arm64.pkg` people download, in
  the same folder. Sparkle installs it with macOS Installer, which asks for an
  administrator password because the pkg also replaces the Pabulong Mic driver.
  The pkg's scripts quit Pabulong, remove the old Noiseless driver and restart
  `coreaudiod`, and Sparkle then reopens the app.
- **Privacy:** the app checks only when the user chooses Check for Updates, or
  once a day after they agree to automatic checks. Sparkle asks on the second
  launch. `SUEnableAutomaticChecks` is deliberately not in Info.plist, and no
  system profile is sent. A check sends the IP address, a user agent such as
  `Pabulong/0.1.3 Sparkle/2.10.0`, and the Mac's preferred languages
  (`Accept-Language`). If that changes, the Privacy Notice must change first.
- **Trust:** Info.plist carries the public key (`SUPublicEDKey`, also in
  `Scripts/common.sh`). Sparkle refuses an update or feed that isn't signed
  with the matching private key, so a hacked web server alone can't push an
  update.

### The update signing key

The private key is in the login keychain of the Mac that makes releases. It was
created with Sparkle's `generate_keys --account pabulong` and is shown in
Keychain Access as "Private key for signing Sparkle updates" (account
`pabulong`). The public key is
`1wHGLZ3n8UcZQ42SKogDMabWBKTIRZZg9k6yTj/aYy8=`.

**Back it up now, and keep two copies.** If the key is lost, every installed
copy of Pabulong stops accepting updates, and each user has to download and
install the next version by hand. If it leaks, someone who also gets into the
web server could ship an update that installs as root.

```sh
swift package resolve
.build/artifacts/sparkle/Sparkle/bin/generate_keys --account pabulong -x ~/Desktop/pabulong-update-key.txt
```

The file holds the private key in plain text. Store it in your password manager
(as a secure note or attachment) and in one offline place, such as an encrypted
USB drive or an encrypted disk image. Then delete the file from the Desktop and
empty the Trash. Never commit it, email it, or put it in a chat.

To restore it on a new Mac:

```sh
.build/artifacts/sparkle/Sparkle/bin/generate_keys --account pabulong -f pabulong-update-key.txt
.build/artifacts/sparkle/Sparkle/bin/generate_keys --account pabulong -p
```

The second command must print the public key above. `release.sh` checks this
before it builds.

To change the key (for example after a leak), sign one more release with the
old key while its Info.plist already carries the new public key. Every later
release is then signed with the new key only. Without the old key that's not
possible, which is why the backup matters.

### The server

`appcast.xml` keeps its name and changes with each release, so it must not be
cached like the installers. `php-hosting/downloads/pabulong/.htaccess` in the
VA Starter Hub repository sends `Cache-Control: no-cache, max-age=0` for it,
while each `.pkg` stays `immutable` for a year. That is also why every release
needs a new version number: an installer name is never reused. After uploading,
check both:

```sh
curl -sI https://vastarterhub.com/downloads/pabulong/appcast.xml | grep -i cache-control
curl -sI https://vastarterhub.com/downloads/pabulong/Pabulong-<version>-arm64.pkg | grep -i -E "cache-control|content-length"
```

Upload the pkg first and the appcast second, so the feed never points at a
file that isn't there yet.

Pabulong 0.1.2 and earlier don't include the updater. People on those versions
need to download 0.1.3 (or later) from the website once; after that, updates
come through the app.

## Releasing before you have Developer ID

Manila Paper is published without Developer ID, and Pabulong can ship the same
way until the certificates exist. Sign the app and driver with the Apple
Development certificate, which keeps Pabulong's microphone permission across
updates and lets Core Audio load the driver. Write the release notes first
(see [Each release](#each-release)), then:

```sh
./Scripts/release.sh --dev-signed 0.1.3 release/notes/0.1.3.txt
```

This bumps `VERSION`, rebuilds the Rust core, builds the installer with the
Apple Development certificate (found in the keychain), signs it for in-app
updates, and adds it to `release/appcast.xml`. In-app updates work the same as
for a Developer ID release: Sparkle checks the EdDSA signature, not Gatekeeper,
and Installer doesn't quarantine the files it installs.

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

1. Write the release notes in `release/notes/<version>.txt`: one short, plain
   sentence per line, about what changed for the person using the app. Each
   line becomes a bullet in the update window. For example:

   ```text
   Pabulong can now check for updates and install them for you.
   Echo cancellation recovers faster when you unplug headphones.
   ```

2. Build the signed, notarized and stapled installer and add it to the feed:

```sh
./Scripts/release.sh 0.1.3 release/notes/0.1.3.txt
```

`release.sh` refuses a version that is already in the feed or older than the
newest one, and checks that the update signing key is in the keychain. It then
writes the version to `VERSION` (the app, the driver, the installer and the
About screen all read it from there), rebuilds the Rust core, finds both
Developer ID certificates in the keychain, and runs
`Scripts/package-offline-full-installer.sh` with
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

Last, `release.sh` runs `Scripts/update-appcast.sh`, which signs the pkg with
the EdDSA key (`sign_update`), puts the new release at the top of
`release/appcast.xml` with its version, size, signature and notes, signs the
feed, and verifies both signatures.

The output is in `outputs/offline-full/Pabulong-<version>-<timestamp>/`,
including a copy of `appcast.xml`. If notarization is rejected, Apple's log is
saved there as `notary-*.json`.

3. Test it on a Mac (or a second user account) that never had Pabulong:
   download the `.pkg` from where you'll host it, open it, and install. The
   installer must open without any warning. After install, open Pabulong and
   check that Pabulong Mic appears in System Settings › Sound › Input.
4. Test the in-app update: on a Mac with the previous version, choose
   Pabulong › Check for Updates…, install, and check that the app reopens as
   the new version and Pabulong Mic still appears. You can do this before
   uploading by serving the folder locally (see
   [Testing an update locally](#testing-an-update-locally)).
5. Upload the `.pkg` to `https://vastarterhub.com/downloads/pabulong/`, then
   `appcast.xml`, and check the headers as described in [The server](#the-server).
6. Fill in the download page's file name, size and SHA-256 from `release.json`.
7. Commit `VERSION`, `release/appcast.xml` and the release notes.

### Testing an update locally

Build two versions that point at a feed on this Mac. A plain-HTTP feed on
`127.0.0.1` works for testing; Sparkle warns that it isn't HTTPS, and
`verify-installer.sh` fails its HTTPS check for these builds on purpose.

```sh
export UPDATE_FEED_URL=http://127.0.0.1:8765/appcast.xml UPDATE_DOWNLOAD_BASE=http://127.0.0.1:8765
SIGN_IDENTITY="Apple Development: Your Name (TEAMID)" VERSION=0.1.2 BUILD_ID=test-old ./Scripts/package-offline-full-installer.sh
SIGN_IDENTITY="Apple Development: Your Name (TEAMID)" VERSION=0.1.3 BUILD_ID=test-new ./Scripts/package-offline-full-installer.sh
mkdir -p /tmp/pabulong-feed
cp outputs/offline-full/Pabulong-0.1.3-test-new/Pabulong-0.1.3-arm64.pkg /tmp/pabulong-feed/
APPCAST=/tmp/pabulong-feed/appcast.xml ./Scripts/update-appcast.sh /tmp/pabulong-feed/Pabulong-0.1.3-arm64.pkg release/notes/0.1.3.txt
python3 -m http.server 8765 --bind 127.0.0.1 --directory /tmp/pabulong-feed
```

Install the 0.1.2 test pkg, open Pabulong, choose Check for Updates…, and install.
Afterwards, reinstall a normal build so the app points at the real feed again.

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
| `codesign -dv` authority (app, driver) | `Apple Development: Your Name` | `Developer ID Application: …` |
| `stapler validate` (pkg, app, driver) | no ticket | ticket stapled |

`codesign --verify --deep --strict` already passes for the app and the driver,
and it still passes after the switch.

## Commands

| Command | What it does |
|---|---|
| `./Scripts/release.sh <version> <notes>` | Builds the public release (Developer ID, notarized, stapled) and adds it to the update feed. |
| `./Scripts/release.sh --dev-signed <version> <notes>` | The same with the Apple Development certificate, until Developer ID exists. |
| `./Scripts/update-appcast.sh <pkg> <notes>` | Signs a built pkg for in-app updates and adds it to `release/appcast.xml`. |
| `./Scripts/package-offline-full-installer.sh` | Builds a QA installer. Ad-hoc signed by default; set `SIGN_IDENTITY` to sign with another certificate. |
| `./Scripts/verify-installer.sh <pkg>` | Checks a built installer without installing it. |
| `sudo /Applications/Pabulong.app/Contents/Resources/uninstall.sh` | Removes the app and driver. Keeps settings and recordings unless you add `--delete-data`. |

A QA build signed with the Apple Development certificate:

```sh
SIGN_IDENTITY="Apple Development: Your Name (TEAMID)" ./Scripts/package-offline-full-installer.sh
```

Install it and test the uninstaller from the command line:

```sh
sudo installer -pkg outputs/offline-full/<folder>/Pabulong-<version>-arm64.pkg -target /
sudo /Applications/Pabulong.app/Contents/Resources/uninstall.sh
```
