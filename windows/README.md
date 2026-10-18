# Pabulong for Windows

The Windows version of Pabulong: a tray app that cleans up your microphone
with the **same Rust core as the Mac app** (`rust/noiseless-core`: echo
canceller → RNNoise → background-voice gate) and feeds it to a virtual
microphone, **Pabulong Mic**, that Zoom, Teams, Meet, Discord and other call
apps can select. It makes no network connections.

> **Status: not released.** Everything here builds in CI, but the driver
> loads only on PCs in test-signing mode until it is signed by Microsoft. See
> [What blocks a public release](#what-blocks-a-public-release).

## How it works

```
 physical mic ──WASAPI (event, MMCSS)──► framer ─► shared core ─► ring ─► PabulongMic.sys ─► call app
                                                       ▲                   (virtual mic)
 default speakers ──WASAPI loopback──► echo reference ─┘
                                   └──► "remote" track of a local recording
```

- **Capture** (`crates/pabulong/src/win/wasapi.rs`): the chosen *physical*
  mic, never Pabulong Mic, in shared mode with WASAPI converting to 48 kHz mono
  float. Frames carry their QPC capture time.
- **Echo reference**: loopback of the default speakers, with QPC times, pushed
  to the core exactly as the Mac pushes its Core Audio tap. A silent render
  stream keeps loopback flowing while nothing plays, so the reference has no
  gaps; it follows the default device if you switch speakers.
- **Virtual mic** (`driver/`): a small WDM/PortCls WaveRT capture driver. It
  reads 10 ms frames from a shared ring the app writes, and plays silence if
  the app is not feeding it. Contract: [`driver/CONTRACT.md`](driver/CONTRACT.md),
  the Windows counterpart of `Sources/NoiselessHALPlugin/HAL_CONTRACT.md`.
- **When the mic is on**: only while a call app is recording from Pabulong
  Mic (the driver reports running streams), stopping 1 s after the call ends,
  exactly like the Mac. With Mic Processing off, a call that uses Pabulong Mic
  still gets your voice, unprocessed ("Pass Through").
- **Recording** (optional, off until you confirm consent once): `local-mic.wav`,
  `remote-system.wav` and `mixed-preview.wav`, 16 kHz 16-bit mono, in
  `%LOCALAPPDATA%\Pabulong\Calls\<id>\`, the same files the Mac writes.

## Feature parity with the Mac app

| | Windows |
|---|---|
| Tray icon + menu: Mic Processing switch, recording strip, "Pabulong Mic is your default input" warning and fix, Open Pabulong, Quit | ✓ |
| First-run Terms (I Agree / Read Terms / Quit, terms version 2, same links) | ✓ |
| Settings: Microphone, Suppression, Echo Cancellation, Background Voices (Off/Light/Balanced/Strong), Bypass, recording options, Launch at Login, About | ✓ |
| Diagnostics: Input, Output, Engine, Mic Clients, Latency, Virtual Mic, Echo Cancellation, Echo Path, Background Voices, Speaker Reference, Refresh, Export Diagnostics | ✓ (plus a driver-health line) |
| Local call recording with one-time consent | ✓ |
| Calls list with Reveal and Delete | ✓ (double-click opens the mixed WAV in your default player) |
| In-app call playback, call search, meeting-title detection | not yet |
| Run Speaker Echo Test | not yet |
| Brand: logo-kit mark (tray icons and app icon), navy/signal blue, Montserrat + DM Sans | ✓ |

Labels and captions are copied from `Sources/NoiselessApp/main.swift`
(`crates/pabulong/src/text.rs`). Three are changed for Windows and marked in
that file: "this Mac" → "this PC" (Terms and recording consent), and the
Capture System Audio caption names WASAPI loopback instead of Core Audio.
Windows lists the endpoint as **"Microphone (Pabulong Mic)"**.

## Layout

| Path | What |
|---|---|
| `crates/pabulong` | The tray app (Rust + windows-rs). `win/` is Windows-only; everything else is portable and unit-tested on any OS. |
| `crates/pabulong-ring` | The app ↔ driver ring (layout, producer, a Rust mirror of the driver's reader, tests). |
| `crates/pabulong-setup` | Creates/removes the `Root\PabulongMic` device node (what `devcon install` does); run by the MSI. |
| `driver/` | `PabulongMic.sys` source, INF, MSBuild project (WDK from NuGet), contract, user-mode ring test. |
| `installer/Pabulong.wxs` | The MSI (WiX v5). |
| `assets/` | Icons rendered from the logo kit's mark by `make-icons.py`; bundled fonts with their OFL licenses. |
| `scripts/` | RNNoise build (pinned), MSI build, off-Windows type-check. |
| `../.github/workflows/windows.yml` | CI: tests, x64 + ARM64 app, driver, MSI. |

The shared core in `../rust` is unchanged; this workspace depends on it by
path, so Windows and macOS process audio with identical code.

## Build

### In CI (recommended)

`.github/workflows/windows.yml` runs on GitHub's free runners for public
repositories. Per architecture (x64, ARM64) it uploads:

- `app-<arch>`: `Pabulong.exe`, `pabulong-setup.exe` (RNNoise linked)
- `driver-<arch>`: `PabulongMic.sys/.inf/.cat`, test-signed, plus
  `PabulongMic-TEST.cer` (a throwaway certificate made by that run)
- `Pabulong-0.1.0-<arch>-unsigned`: the MSI and the test certificate

It also runs the core's tests on Linux and on Windows (with RNNoise), the
portable app tests, and the driver's ring test under ASan/UBSan and TSan.

### On a Windows PC

Visual Studio 2022 or 2026 with "Desktop development with C++" (plus ARM64
tools for ARM64), the LLVM/clang component, Rust (`rustup`), NuGet, and for
the MSI the .NET SDK. From a *Developer PowerShell*:

```powershell
# RNNoise (pinned; checks the model's SHA-256)
./windows/scripts/build-rnnoise.ps1 -Arch x64 -Out C:\pabulong\rnnoise\x64

# App + setup helper
cd windows
$env:RNNOISE_LIB_DIR = 'C:\pabulong\rnnoise\x64'
cargo test --features pabulong/rnnoise
cargo build --release --features pabulong/rnnoise -p pabulong -p pabulong-setup

# Driver (WDK from NuGet)
cd driver
nuget restore packages.config -PackagesDirectory packages
msbuild PabulongMic.vcxproj -p:Configuration=Release -p:Platform=x64
# then Inf2Cat + signtool as in the workflow's "Catalog and test-sign" step

# MSI (WiX v5)
dotnet tool install --global wix --version 5.0.2
wix extension add -g WixToolset.Util.wixext/5.0.2 WixToolset.UI.wixext/5.0.2
../scripts/build-msi.ps1 -Arch x64 -AppDir ..\target\release -DriverDir <folder with .sys/.inf/.cat>
```

### Checks you can run on a Mac (no Windows toolchain)

```bash
cd windows && cargo test                       # portable app logic + ring
windows/scripts/typecheck-off-windows.sh       # type-checks win/ against the real windows-rs API
clang++ -std=c++17 -fsanitize=address,undefined -Iwindows/driver/src windows/driver/test/ring_test.cpp -o /tmp/ring_test && /tmp/ring_test
```

The type-check copies the windows-rs crates from the local cargo registry,
switches on their `cfg(windows)` gates in a temp dir and runs `cargo check`.
It proves the code matches the real API signatures; it does not compile the
C++ driver, link, or run anything.

## Install for testing (test-signed driver)

Only on a test PC. Test-signing mode lowers Windows' driver protection.

1. Download the MSI artifact for your PC (x64 or arm64) from a CI run.
2. In an administrator terminal:
   ```powershell
   bcdedit /set testsigning on          # Secure Boot must be off for this
   certutil -addstore Root PabulongMic-TEST.cer
   certutil -addstore TrustedPublisher PabulongMic-TEST.cer
   ```
   Restart.
3. Run the MSI. It installs to `C:\Program Files\Pabulong`, creates the
   Pabulong Mic device and adds Pabulong to the Start menu.
4. Start Pabulong, agree to the Terms, turn on **Mic Processing**, and pick
   **Microphone (Pabulong Mic)** in your call app.

Uninstall from Settings › Apps; it removes the device and the driver
package. Then `bcdedit /set testsigning off`, remove the certificate from
both stores, and restart.

## Choosing the virtual-mic driver

Windows has no user-mode API that creates a microphone every app can see
(macOS has the HAL plug-in API; Windows 11's virtual *camera* API has no
audio equivalent). Every option is a kernel driver:

| Option | Licence | Notes |
|---|---|---|
| Microsoft SysVAD / SimpleAudioSample | **MS-PL** (not MIT: the Windows-driver-samples repository is MS-PL) | Full-featured, large; derived files would have to stay MS-PL. |
| VirtualDrivers/Virtual-Audio-Driver | MIT + MS-PL (SysVAD-derived parts) | Beta, test-signed only; no documented way for an app to feed its mic. |
| VB-Cable, Thesycon TVirtAudio | Proprietary | Licensing/redistribution cost; not open source. |
| **Our own PortCls WaveRT driver** | **MIT** | About 1,300 lines, written from the WDK documentation, one capture endpoint, a hardened shared-memory ring. **Chosen.** |

## Signing: what you need to do

Neither step can be done for you: both need accounts and credentials in
Next Matter Pro's name.

### 1. Microsoft signature for the driver (required)

64-bit Windows 10/11 only loads kernel drivers signed by Microsoft. For
distribution outside Windows Update, **attestation signing** is enough for
Windows 10/11 desktop:

1. Buy an **EV code-signing certificate** in Next Matter Pro's legal name
   (DigiCert, Sectigo, SSL.com, GlobalSign; typically delivered on a hardware
   token or cloud HSM, with business identity checks).
2. Register for the **Windows Hardware Developer Program** in Partner Center
   (https://partner.microsoft.com/dashboard/hardware) using an Entra ID
   (Azure AD) tenant. Registration has required signing a file with an EV
   certificate; check the current requirement there. Accept the agreements.
3. For each architecture, make a CAB containing `PabulongMic.inf` and
   `PabulongMic.sys` from the CI `driver-<arch>` artifact (`makecab /f` with a
   .ddf), and sign the CAB with the EV certificate (`signtool sign /fd sha256
   /tr <timestamp-url> /td sha256 ...`).
4. In Partner Center: *Hardware → Submit new hardware*, upload the CAB, choose
   **Attestation**, tick Windows 10/11 client x64 (and ARM64 in a separate
   submission or the same if supported), submit.
5. Download the signed package (Microsoft adds its signature and a new .cat)
   and use those files as the MSI's `driver\` payload.

Attestation-signed drivers work on Windows 10/11 client, not Windows Server,
and are not distributed by Windows Update. (WHQL through the Hardware Lab Kit
is the further step if that is ever wanted.)

### 2. Code signing for Pabulong.exe, pabulong-setup.exe and the MSI (strongly recommended)

Without it, SmartScreen warns on download and some antivirus tools flag the
installer. The EV certificate above can sign these too (`signtool sign` each
.exe before packaging, then the MSI). An OV certificate also works; neither
gives instant SmartScreen reputation any more, which builds with downloads.

## What blocks a public release

1. **Driver signing (Microsoft attestation)**: needs the EV certificate and
   Partner Center account above. Until then the driver loads only in
   test-signing mode.
2. **Code-signing certificate** for the app, the setup helper and the MSI.
3. **First CI run**: the branch has not been pushed, so nothing has been
   built on Windows yet. The Rust code type-checks against the real
   windows-rs API and the ring/conversion code is tested, but the C++ driver,
   the MSBuild/WDK project, the RNNoise MSVC build and the WiX build have
   not been compiled. Expect a round of fixes.
4. **Testing on real hardware**, x64 and ARM64, Windows 10 2004+ and 11:
   - Zoom, Teams (new), Google Meet and Slack in Chrome/Edge, Discord
   - laptop speakers (echo cancellation), wired/USB headsets, Bluetooth
     headsets (16 kHz hands-free), USB mics
   - start/stop of calls, switching mics and speakers mid-call, sleep/resume,
     unplugging the mic, signing out, fast user switching
   - the driver under Driver Verifier (`verifier /standard /driver PabulongMic.sys`)
     and, ideally, the HLK audio tests
   - long calls (drift between the mic clock and the driver's QPC clock is
     absorbed by skipping or padding 10 ms frames; check it is inaudible)
   - high-DPI monitors and per-monitor DPI changes (fonts are created at the
     system DPI), keyboard navigation, Narrator
5. **Not yet ported**: in-app call playback and search, meeting-title
   detection, Run Speaker Echo Test.
6. **Legal pages**: the Terms of Use and Privacy Notice describe the Mac app;
   they should cover Windows (the driver, recordings in `%LOCALAPPDATA%`).
7. **RNNoise pin on macOS**: Windows builds RNNoise at
   `xiph/rnnoise@70f1d256` (the newest commit; the Mac library was built from
   an unpinned clone, so almost certainly the same). Pin
   `Scripts/vendor-rnnoise.sh` to it too so both platforms are provably
   identical.
