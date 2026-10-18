# Pabulong

Pabulong is a free, open-source noise-cancelling microphone for Mac, made by
[Next Matter Pro](https://nextmatterpro.com/training/). It adds a virtual
microphone called **Pabulong Mic**: pick it in any call app that lets you choose
a microphone, and the people on the call hear your voice without the room.

- Removes background noise (RNNoise neural suppression).
- Cancels echo when you use your Mac's speakers instead of headphones.
- Turns down other people talking near you (Background Voices: Light, Balanced, Strong).
- Optional call recording, saved only on your Mac, with a consent reminder.

Everything runs on your Mac. The app makes no network connections and has no
account, analytics or subscription. See the
[Privacy Notice](https://nextmatterpro.com/training/pabulong-privacy.php) and
[Terms of Use](https://nextmatterpro.com/training/pabulong-terms.php). If you
record calls, you are responsible for following the recording laws where you
and everyone on the call are.

**Platforms:** macOS on Apple silicon. A Windows version that shares the
audio core in `rust/` is in development in [`windows/`](windows/README.md); it
is not released yet.

**Download:** [nextmatterpro.com/training](https://nextmatterpro.com/training/)
or [vastarterhub.com](https://vastarterhub.com/).

## Repository layout

- `Sources/NoiselessApp`: the Pabulong menu bar app (SwiftUI).
- `Sources/NoiselessAudioCore`: capture pipeline, settings, devices, call
  recording, and the bridge to the virtual mic.
- `Sources/NoiselessHALPlugin`: the Core Audio HAL plug-in that provides
  Pabulong Mic (shared-memory contract in `HAL_CONTRACT.md`).
- `Sources/NoiselessHelper`, `Sources/NoiselessCoreChecks`: diagnostics and checks.
- `rust/noiseless-core`: the shared voice-processing core (echo canceller,
  noise suppressor, background-voice gate) with a C ABI; `rust/noiseless-sim`
  is an offline acoustic simulator for it.

Internal module names still use the project's earlier working name; they are
not part of the product name.

## Noise suppression

The built-in suppressor is a real-time **spectral-subtraction** denoiser: a
512-point STFT (50% overlap, sqrt-Hann windows) tracks a per-frequency-bin noise
estimate during non-speech and applies a Wiener-style gain with temporal
smoothing. It substantially reduces **stationary and slowly-varying** noise —
fans, hum, hiss, steady room/traffic noise — and partially reduces broadband
music.

It does **not** fully remove **transient, speech-overlapping** noise such as
clapping, keyboard clicks, or foreground music: single-channel classical DSP
cannot separate those from speech. Removing that class of noise
requires a trained model.

### RNNoise neural suppressor

The repository includes a prebuilt macOS arm64 `librnnoise.a`. To rebuild it
from source: the build integrates [RNNoise](https://github.com/xiph/rnnoise) (BSD-licensed,
48 kHz / 480-sample frames — an exact match for this pipeline). It is optional
and fetched on demand:

```sh
brew install autoconf automake libtool   # one-time toolchain
./Scripts/vendor-rnnoise.sh              # clone + build + vendor the model
swift build                              # Package.swift auto-detects it
./Scripts/package-app.sh                 # rebuild the app with the neural engine
```

`vendor-rnnoise.sh` drops a static library under `Sources/CRNNoise/`, which
`Package.swift` detects to define `NL_HAS_RNNOISE` and link the neural backend.
When present, processors at 48 kHz/480 frames automatically use RNNoise (the
suppression modes become a wet/dry mix); otherwise they fall back to the
spectral suppressor. The active engine is shown in the menu bar app ("Engine")
and in `swift run NoiselessHelper`. Nothing in the pipeline or driver changes —
the swap is entirely behind the `NoiseProcessor` interface.

## Build and test

The echo canceller and background-voice gate live in the Rust core. Build and
vendor it first (needs [rustup](https://rustup.rs)); without it the app falls
back to noise suppression only:

```sh
./Scripts/build-rust-core.sh
swift build
swift run NoiselessCoreChecks
swift run NoiselessHelper --diagnostics
```

`swift test` is intentionally not used yet because this SwiftPM scaffold keeps its
checks in the `NoiselessCoreChecks` executable.

## Local call recording

Call recording is local-first and off by default. When enabled, Pabulong starts
a call session only while another app is actively pulling audio from
`Pabulong Mic`. It writes per-call audio files under:

```sh
~/Library/Application Support/Pabulong/Calls/
```

The other side of the call is captured with a Core Audio process tap (no Screen
Recording permission). If system-audio access is denied, Pabulong still records
the processed mic track and marks the call as mic-only.

Pabulong does not transcribe or upload recorded calls. Existing legacy
transcript files are left untouched when upgrading, but are not shown or used.

## Local app/driver build

```sh
./Scripts/package-hal-plugin.sh
./Scripts/package-app.sh
./Scripts/install-hal-plugin.sh
open outputs/Pabulong.app
```

After installation, select `Pabulong Mic` as the microphone in your meeting
app. The menu bar app captures the selected **physical** microphone, processes
frames, and feeds the HAL driver through the shared memory ring.

## Runtime model

- **One producer at a time.** The menu bar app is the producer; it holds a
  cross-process lock (`ProducerGuard`) so the app and the `NoiselessHelper --run`
  headless mode can never both capture the mic and feed the ring at once.
- **Never captures itself.** The pipeline binds an explicit physical input
  device and refuses to capture the `Pabulong Mic` virtual device, which would
  otherwise create an audio feedback loop that saturates the CPU.
- **Real-time safe.** The HAL IO callback makes no blocking syscalls; it only
  reads the shared ring (opened on the `StartIO` control path) and emits silence
  on underrun.

## Troubleshooting: "Pabulong Mic" doesn't appear

The virtual mic only shows up in System Settings / Meet / Zoom once the **HAL
plug-in is installed and loaded by `coreaudiod`**. Building the app is not
enough — the driver is a separate system component.

1. Confirm what CoreAudio currently sees:

   ```sh
   swift run NoiselessHelper
   ```

   It lists every input device and flags whether `Pabulong Mic` is detected.

2. If it's not detected, install the driver (needs admin; restarts audio):

   ```sh
   ./Scripts/install-hal-plugin.sh
   ```

   The script verifies the device after restarting `coreaudiod` and reports
   SUCCESS/FAILED.

3. If it still doesn't appear, it's almost always the **code signature**.
   `coreaudiod` on Apple Silicon routinely refuses ad-hoc-signed plug-ins. Sign
   with a Developer ID and reinstall:

   ```sh
   SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" ./Scripts/package-hal-plugin.sh
   ./Scripts/install-hal-plugin.sh
   ```

   Inspect why it was rejected:

   ```sh
   log show --last 2m --predicate 'process == "coreaudiod"' | grep -i pabulong
   ```

## Signing and notarization

The default build is **ad-hoc signed**, which gives the app a *different*
cryptographic identity every rebuild — so macOS re-prompts for microphone access
each time. There are three signing modes:

**Stable local signing (recommended for development).** Sign with any stable
identity you already have — an `Apple Development: …` certificate works. The
app's designated requirement becomes identifier + certificate based (not a
per-build `cdhash`), so the mic grant persists across rebuilds:

```sh
SIGN_IDENTITY="Apple Development: You (TEAMID)" ./Scripts/package-app.sh
```

List your identities with `security find-identity -v -p codesigning`.

**Distribution.** Sign with a Developer ID and a hardened runtime, then notarize:

```sh
HARDENED=1 SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" ./Scripts/package-hal-plugin.sh
HARDENED=1 SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)" ./Scripts/package-app.sh
# then zip and notarize the app + driver:
xcrun notarytool submit Pabulong.zip --keychain-profile <profile> --wait
xcrun stapler staple outputs/Pabulong.app
```

The app is signed with `Sources/NoiselessApp/Noiseless.entitlements`
(microphone entitlement) so capture works under a hardened runtime.

## Installer

The public download is `Pabulong-<version>-arm64.pkg`, built by
`Scripts/package-offline-full-installer.sh`. It installs the app and the
Pabulong Mic driver, shows a welcome page and the license, and includes an
uninstaller at `Pabulong.app/Contents/Resources/uninstall.sh`. The version comes
from the `VERSION` file.

```sh
./Scripts/package-offline-full-installer.sh       # QA build, ad-hoc signed
./Scripts/verify-installer.sh <path to .pkg>      # check signatures and contents
./Scripts/release.sh                              # Developer ID signed and notarized
```

[docs/RELEASING.md](docs/RELEASING.md) has the one-time Developer ID and
notarization setup and the release steps.

## License

Pabulong is open source under the [MIT License](LICENSE), © 2026 Next Matter Pro.
The Pabulong name, logo and artwork are not covered by the license. Third-party
components keep their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
