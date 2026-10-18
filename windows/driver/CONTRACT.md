# Pabulong Mic driver contract (Windows)

The Windows counterpart of `Sources/NoiselessHALPlugin/HAL_CONTRACT.md`.

The driver (`PabulongMic.sys`) publishes one virtual **capture** endpoint
named *Pabulong Mic*. Call apps select it like any microphone. Its audio comes
from the Pabulong app through a shared-memory ring.

## Format

The ring carries **48 kHz mono 32-bit float** in 480-sample (10 ms) frames,
exactly what the shared Rust core produces. The endpoint offers that format
and 16-bit PCM (converted in the driver with integer arithmetic). As on
macOS, a mono microphone is the most compatible shape; no runtime format
negotiation beyond those two.

## The ring

Owned by the driver: whole non-paged pages allocated in `DriverEntry`, mapped
into system space for the WaveRT timer, and mapped into **one** user process
on request.

| offset | field                    | written by |
|-------:|--------------------------|------------|
|      0 | magic `"PBMR"`           | driver (once) |
|      4 | version = 1              | driver (once) |
|      8 | header bytes = 128       | driver (once) |
|     12 | sample rate = 48000      | driver (once) |
|     16 | channels = 1             | driver (once) |
|     20 | frame length = 480       | driver (once) |
|     24 | capacity frames = 32     | driver (once) |
|     28 | max buffered frames = 3  | driver (once) |
|     32 | `write_count` (u64)      | producer |
|     40 | `read_count` (u64)       | driver |
|     48 | `underrun_count` (u64)   | driver |
|     56 | `dropped_count` (u64)    | driver |
|     64 | `overrun_count` (u64)    | producer |
|     72 | `producer_attached`      | driver |
|     76 | `consumer_running_count` | driver |
|     80 | `producer_active`        | producer (driver clears it on detach) |
|     84 | `driver_version`         | driver (once) |
|    128 | `float samples[32 * 480]`| producer |

Counters are monotonic frame counts; a frame's slot is `count % 32`. No field
has two concurrent writers. The producer publishes a frame with a release
store of `write_count` after filling its slot; the driver reads it after an
acquire load and publishes `read_count + 1` with a release store.

Definitions: `windows/crates/pabulong-ring` (Rust, with tests) and
`windows/driver/src/pabulong_ring.h` + `ring_read.h` (C, tested in user mode
by `windows/driver/test/ring_test.cpp`). Both pin the same offsets and IOCTL
codes.

## Control device `\\.\PabulongMic`

| IOCTL | code | |
|---|---|---|
| `MAP_RING`   | `0x9A5CE004` | maps the ring into the caller; returns `{u64 address, u32 bytes, u32 version}`. `STATUS_DEVICE_BUSY` if another handle holds it. |
| `UNMAP_RING` | `0x9A5CE008` | unmaps (also automatic on handle close / process exit). |
| `GET_INFO`   | `0x9A5C600C` | `{version, driver_version, consumer_running_count, producer_attached}`. |

Access: SYSTEM and Administrators full, interactive users read/write
(`D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)`).

## The driver (consumer) must

- read 480-sample frames from the ring in its WaveRT timer callback
  (1 ms, `DISPATCH_LEVEL`) and in `GetPosition`;
- keep at most 3 frames (30 ms) queued: when more are waiting, skip to the
  newest 3 and add the skipped count to `dropped_count` (macOS:
  `nl_shared_ring_read_latest` with `kNoiselessMaxBufferedFrames = 3`);
- output silence when the ring is empty or `producer_active == 0`, counting
  an underrun only when the producer is active;
- never wait for the app, never block, never touch pageable memory at
  `DISPATCH_LEVEL`;
- **never trust the shared header**: geometry comes from compile-time
  constants; producer-written counters only change what is played, never
  where the driver reads; non-finite samples become silence;
- set `consumer_running_count` to the number of capture streams in
  `KSSTATE_RUN` (the audio engine runs the stream only while some app
  records from Pabulong Mic);
- use no floating-point arithmetic in the timer callback.

## The app (producer) must

- capture an explicit **physical** microphone, never Pabulong Mic itself;
- hold the mapping for as long as it runs; only one producer can (the
  driver enforces it);
- set `producer_active = 1` while feeding, `0` when it stops;
- write processed frames with `write_frame`; when the ring is full (no call
  app reading), drop the frame and count an overrun;
- start capturing the mic only while `consumer_running_count > 0`, and stop
  one second after it returns to 0 (as `MenuBarModel` does on macOS).

## Clocks

There is no hardware clock. The driver's stream runs on the QPC; the app's
frames arrive on the physical mic's clock. Small drift is absorbed by the
3-frame bound (skip) and silence on underrun, the same strategy as the macOS
HAL plug-in.
