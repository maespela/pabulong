import Foundation

/// Cross-process mutual exclusion for the single audio producer allowed to own
/// the shared virtual-mic ring at any time.
///
/// Both the menu bar app and the headless helper run the same capture pipeline;
/// if two of them produce into `/nlvmic` at once you get doubled microphone
/// capture, ring contention, and garbled audio. This guard makes ownership
/// exclusive.
///
/// It is backed by an advisory `flock`, which the kernel releases automatically
/// when the owning file descriptor is closed — including on process exit or
/// crash — so a stale lock can never wedge the device the way a flag in shared
/// memory could.
public final class ProducerGuard {
    /// Per-user lock path. Audio sessions are per-user on macOS, matching the
    /// single-producer scope we need for the common desktop case.
    public static var defaultPath: String {
        NSTemporaryDirectory() + "com.nextmatterpro.pabulong.producer.lock"
    }

    private let descriptor: Int32

    /// Attempts to become the sole producer. Returns `nil` if another process
    /// already holds the lock.
    public init?(path: String = ProducerGuard.defaultPath) {
        let fd = open(path, O_CREAT | O_RDWR, 0o644)
        guard fd >= 0 else { return nil }
        // Non-blocking exclusive lock: fail fast rather than wait if another
        // producer is already running.
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
            close(fd)
            return nil
        }
        descriptor = fd
    }

    deinit {
        // Closing the descriptor drops the advisory lock.
        flock(descriptor, LOCK_UN)
        close(descriptor)
    }
}
