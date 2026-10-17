# Noiseless HAL Plug-in Contract

The production HAL bundle publishes one virtual input device named
`Noiseless Mic`.

The device publishes a single **48 kHz mono** float input stream. A mono
microphone is the correct and maximally compatible shape for this product:
physical microphones are mono, and every conferencing app accepts a mono input
device. Runtime stereo format negotiation is intentionally out of scope — it
adds host-config-change complexity with no compatibility benefit for a mic.

The HAL audio callback must:

- read 48 kHz mono float frames from shared memory,
- output silence when no producer is attached or the ring buffer underruns,
- never wait for the helper process, and never make blocking syscalls on the
  real-time IO thread (the shared ring is opened on the `StartIO` control path),
- increment shared underrun counters for diagnostics.

The producer (the menu bar app, or the `NoiselessHelper --run` headless
equivalent) must:

- own physical microphone capture, binding an explicit *physical* input device
  and never the Noiseless virtual mic (that would form an audio feedback loop),
- hold the single-producer lock (`ProducerGuard`) so only one process feeds the
  ring at a time,
- write processed frames into the shared ring buffer,
- update shared health metadata,
- clear the ring buffer during device switches.

Exactly one producer runs at a time. The shipping app is the producer; the
helper's `--run` mode exists for headless/diagnostic use and refuses to start if
the app already holds the lock.

The current implementation provides a real Core Audio
`AudioServerPlugInDriverInterface` with one 48 kHz mono input stream. It reads
processed frames from the shared memory ring named
`/nlvmic` and emits silence when the helper/app producer
is absent or the ring underruns.

Use `Scripts/package-hal-plugin.sh` to create the `.driver` bundle and
`Scripts/install-hal-plugin.sh` to install it into `/Library/Audio/Plug-Ins/HAL`.
