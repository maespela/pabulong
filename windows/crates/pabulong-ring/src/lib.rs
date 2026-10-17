//! The shared-memory ring between the Pabulong app (producer) and the
//! Pabulong Mic kernel driver (consumer). Windows counterpart of the macOS
//! `/nlvmic` ring described in `Sources/NoiselessHALPlugin/HAL_CONTRACT.md`.
//!
//! The driver owns the memory: it allocates whole non-paged pages, writes the
//! header, and maps the pages into the one process that opens its control
//! device and asks for them (`IOCTL_PABULONG_MAP_RING`). Everything here is
//! plain memory access, so this crate builds and tests on any platform; the
//! C mirror of the layout is `windows/driver/src/pabulong_ring.h` and the full
//! contract is `windows/driver/CONTRACT.md`.
//!
//! Ownership of each field is strict, so no field ever has two writers:
//!
//! | field                    | written by |
//! |--------------------------|------------|
//! | geometry (magic … max)   | driver, once, before mapping |
//! | `write_count`            | producer   |
//! | `overrun_count`          | producer   |
//! | `producer_active`        | producer   |
//! | `read_count`             | driver     |
//! | `underrun_count`         | driver     |
//! | `dropped_count`          | driver     |
//! | `producer_attached`      | driver     |
//! | `consumer_running_count` | driver     |
//! | `driver_version`         | driver     |
//!
//! Counters are monotonic 64-bit frame counts; slot = count % capacity.

#![cfg_attr(not(test), no_std)]

use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const MAGIC: u32 = u32::from_le_bytes(*b"PBMR");
pub const VERSION: u32 = 1;
pub const HEADER_BYTES: usize = 128;
pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u32 = 1;
/// 10 ms at 48 kHz, the core's frame size.
pub const FRAME_LENGTH: u32 = 480;
/// Same as `SharedVirtualMicBridge.defaultFrameCapacity` on macOS.
pub const CAPACITY_FRAMES: u32 = 32;
/// The driver never lets more than this many frames queue up (30 ms), the
/// same bound as `kNoiselessMaxBufferedFrames` in the macOS HAL plug-in.
pub const MAX_BUFFERED_FRAMES: u32 = 3;

/// Byte offsets of the header fields (see `pabulong_ring.h`).
pub mod offset {
    pub const MAGIC: usize = 0;
    pub const VERSION: usize = 4;
    pub const HEADER_BYTES: usize = 8;
    pub const SAMPLE_RATE: usize = 12;
    pub const CHANNELS: usize = 16;
    pub const FRAME_LENGTH: usize = 20;
    pub const CAPACITY_FRAMES: usize = 24;
    pub const MAX_BUFFERED_FRAMES: usize = 28;
    pub const WRITE_COUNT: usize = 32;
    pub const READ_COUNT: usize = 40;
    pub const UNDERRUN_COUNT: usize = 48;
    pub const DROPPED_COUNT: usize = 56;
    pub const OVERRUN_COUNT: usize = 64;
    pub const PRODUCER_ATTACHED: usize = 72;
    pub const CONSUMER_RUNNING_COUNT: usize = 76;
    pub const PRODUCER_ACTIVE: usize = 80;
    pub const DRIVER_VERSION: usize = 84;
    /// 88..128 reserved (zero).
    pub const SAMPLES: usize = super::HEADER_BYTES;
}

/// Bytes the ring needs for a given geometry (before rounding up to pages).
pub const fn ring_bytes(capacity_frames: u32, frame_length: u32) -> usize {
    HEADER_BYTES + capacity_frames as usize * frame_length as usize * 4
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingError {
    Null,
    Misaligned,
    TooSmall,
    BadMagic,
    UnsupportedVersion(u32),
    UnsupportedGeometry,
}

/// Snapshot of the ring's counters, for the Diagnostics view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RingHealth {
    pub available_read_frames: u32,
    pub available_write_frames: u32,
    pub underrun_count: u64,
    pub overrun_count: u64,
    pub dropped_count: u64,
    pub producer_attached: bool,
    pub producer_active: bool,
    pub consumer_running_count: u32,
    pub driver_version: u32,
}

/// A view of a mapped ring. Cheap to copy; does not own the memory.
#[derive(Clone, Copy, Debug)]
pub struct Ring {
    base: NonNull<u8>,
    capacity: u32,
    frame_length: u32,
}

// SAFETY: every shared field is accessed atomically and the sample slots are
// handed over through release/acquire on the counters (SPSC).
unsafe impl Send for Ring {}
unsafe impl Sync for Ring {}

impl Ring {
    /// Writes a fresh header into `len` bytes at `base`. The driver does this
    /// in production; the app only calls it in tests and the simulator.
    ///
    /// # Safety
    /// `base` must be valid for `len` writable bytes, 8-byte aligned, and not
    /// in use by anyone else.
    pub unsafe fn initialize(base: *mut u8, len: usize, driver_version: u32) -> Result<Ring, RingError> {
        let base = NonNull::new(base).ok_or(RingError::Null)?;
        if base.as_ptr() as usize % 8 != 0 {
            return Err(RingError::Misaligned);
        }
        if len < ring_bytes(CAPACITY_FRAMES, FRAME_LENGTH) {
            return Err(RingError::TooSmall);
        }
        core::ptr::write_bytes(base.as_ptr(), 0, len);
        let put = |off: usize, v: u32| (base.as_ptr().add(off) as *mut u32).write(v);
        put(offset::VERSION, VERSION);
        put(offset::HEADER_BYTES, HEADER_BYTES as u32);
        put(offset::SAMPLE_RATE, SAMPLE_RATE);
        put(offset::CHANNELS, CHANNELS);
        put(offset::FRAME_LENGTH, FRAME_LENGTH);
        put(offset::CAPACITY_FRAMES, CAPACITY_FRAMES);
        put(offset::MAX_BUFFERED_FRAMES, MAX_BUFFERED_FRAMES);
        put(offset::DRIVER_VERSION, driver_version);
        // Magic last: a reader that sees it sees a complete header.
        (*(base.as_ptr().add(offset::MAGIC) as *const AtomicU32)).store(MAGIC, Ordering::Release);
        Ring::attach(base.as_ptr(), len)
    }

    /// Validates the header of a mapped ring.
    ///
    /// # Safety
    /// `base` must stay valid for `len` bytes for as long as the returned
    /// `Ring` (or any copy of it) is used.
    pub unsafe fn attach(base: *mut u8, len: usize) -> Result<Ring, RingError> {
        let base = NonNull::new(base).ok_or(RingError::Null)?;
        if base.as_ptr() as usize % 8 != 0 {
            return Err(RingError::Misaligned);
        }
        if len < HEADER_BYTES {
            return Err(RingError::TooSmall);
        }
        let get = |off: usize| (*(base.as_ptr().add(off) as *const AtomicU32)).load(Ordering::Acquire);
        if get(offset::MAGIC) != MAGIC {
            return Err(RingError::BadMagic);
        }
        let version = get(offset::VERSION);
        if version != VERSION {
            return Err(RingError::UnsupportedVersion(version));
        }
        let capacity = get(offset::CAPACITY_FRAMES);
        let frame_length = get(offset::FRAME_LENGTH);
        if get(offset::HEADER_BYTES) as usize != HEADER_BYTES
            || get(offset::SAMPLE_RATE) != SAMPLE_RATE
            || get(offset::CHANNELS) != CHANNELS
            || frame_length != FRAME_LENGTH
            || capacity < 2
        {
            return Err(RingError::UnsupportedGeometry);
        }
        if len < ring_bytes(capacity, frame_length) {
            return Err(RingError::TooSmall);
        }
        Ok(Ring { base, capacity, frame_length })
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn frame_length(&self) -> usize {
        self.frame_length as usize
    }

    fn u32_at(&self, off: usize) -> &AtomicU32 {
        unsafe { &*(self.base.as_ptr().add(off) as *const AtomicU32) }
    }

    fn u64_at(&self, off: usize) -> &AtomicU64 {
        unsafe { &*(self.base.as_ptr().add(off) as *const AtomicU64) }
    }

    fn slot(&self, count: u64) -> *mut f32 {
        let index = (count % u64::from(self.capacity)) as usize;
        unsafe { (self.base.as_ptr().add(offset::SAMPLES) as *mut f32).add(index * self.frame_length()) }
    }

    fn max_buffered(&self) -> u64 {
        u64::from(self.u32_at(offset::MAX_BUFFERED_FRAMES).load(Ordering::Relaxed).clamp(1, self.capacity))
    }

    // ------------------------------------------------------------ producer

    /// Appends one frame. Returns false (and counts an overrun) when the ring
    /// is full, which only happens while no call app is reading Pabulong Mic.
    pub fn write_frame(&self, frame: &[f32]) -> bool {
        if frame.len() != self.frame_length() {
            return false;
        }
        let write = self.u64_at(offset::WRITE_COUNT).load(Ordering::Relaxed);
        let read = self.u64_at(offset::READ_COUNT).load(Ordering::Acquire);
        if write.wrapping_sub(read) >= u64::from(self.capacity) {
            self.u64_at(offset::OVERRUN_COUNT).fetch_add(1, Ordering::Relaxed);
            return false;
        }
        unsafe { core::ptr::copy_nonoverlapping(frame.as_ptr(), self.slot(write), frame.len()) };
        self.u64_at(offset::WRITE_COUNT).store(write.wrapping_add(1), Ordering::Release);
        true
    }

    /// Producer is feeding (true) or has paused (false). While paused the
    /// driver outputs silence without counting underruns.
    pub fn set_producer_active(&self, active: bool) {
        self.u32_at(offset::PRODUCER_ACTIVE).store(u32::from(active), Ordering::Release);
    }

    // ------------------------------------------------------ driver (mirror)

    /// The driver's read, ported line for line from `PabulongRingReadLatest`
    /// in `windows/driver/src/ring.cpp`, so the protocol is tested here.
    /// Fills `out` with the next frame, or silence. Returns true on real audio.
    pub fn read_frame_latest(&self, out: &mut [f32]) -> bool {
        if out.len() != self.frame_length() {
            return false;
        }
        let write = self.u64_at(offset::WRITE_COUNT).load(Ordering::Acquire);
        let mut read = self.u64_at(offset::READ_COUNT).load(Ordering::Relaxed);
        let mut available = write.wrapping_sub(read);
        if available > u64::from(self.capacity) {
            // Producer restarted its counter (new mapping): resynchronise.
            read = write;
            available = 0;
        }
        let max = self.max_buffered();
        if available > max {
            self.u64_at(offset::DROPPED_COUNT).fetch_add(available - max, Ordering::Relaxed);
            read = write - max;
            available = max;
        }
        if available == 0 || self.u32_at(offset::PRODUCER_ACTIVE).load(Ordering::Acquire) == 0 {
            out.fill(0.0);
            if available == 0 && self.u32_at(offset::PRODUCER_ACTIVE).load(Ordering::Relaxed) != 0 {
                self.u64_at(offset::UNDERRUN_COUNT).fetch_add(1, Ordering::Relaxed);
            }
            self.u64_at(offset::READ_COUNT).store(read, Ordering::Release);
            return false;
        }
        unsafe { core::ptr::copy_nonoverlapping(self.slot(read), out.as_mut_ptr(), out.len()) };
        self.u64_at(offset::READ_COUNT).store(read + 1, Ordering::Release);
        true
    }

    /// Driver-owned fields; only tests and the simulator call these.
    pub fn driver_set_producer_attached(&self, attached: bool) {
        self.u32_at(offset::PRODUCER_ATTACHED).store(u32::from(attached), Ordering::Release);
    }

    pub fn driver_set_consumer_running_count(&self, count: u32) {
        self.u32_at(offset::CONSUMER_RUNNING_COUNT).store(count, Ordering::Release);
    }

    // ---------------------------------------------------------------- both

    pub fn consumer_running_count(&self) -> u32 {
        self.u32_at(offset::CONSUMER_RUNNING_COUNT).load(Ordering::Acquire)
    }

    pub fn health(&self) -> RingHealth {
        let write = self.u64_at(offset::WRITE_COUNT).load(Ordering::Acquire);
        let read = self.u64_at(offset::READ_COUNT).load(Ordering::Acquire);
        let available = write.wrapping_sub(read).min(u64::from(self.capacity)) as u32;
        RingHealth {
            available_read_frames: available,
            available_write_frames: self.capacity - available,
            underrun_count: self.u64_at(offset::UNDERRUN_COUNT).load(Ordering::Relaxed),
            overrun_count: self.u64_at(offset::OVERRUN_COUNT).load(Ordering::Relaxed),
            dropped_count: self.u64_at(offset::DROPPED_COUNT).load(Ordering::Relaxed),
            producer_attached: self.u32_at(offset::PRODUCER_ATTACHED).load(Ordering::Acquire) != 0,
            producer_active: self.u32_at(offset::PRODUCER_ACTIVE).load(Ordering::Acquire) != 0,
            consumer_running_count: self.consumer_running_count(),
            driver_version: self.u32_at(offset::DRIVER_VERSION).load(Ordering::Relaxed),
        }
    }
}

// ------------------------------------------------------------------ IOCTLs

/// Device type for the control device (vendor range, >= 0x8000).
pub const DEVICE_TYPE: u32 = 0x9A5C;
pub const METHOD_BUFFERED: u32 = 0;
pub const FILE_READ_DATA: u32 = 1;
pub const FILE_WRITE_DATA: u32 = 2;

pub const fn ctl_code(device_type: u32, function: u32, method: u32, access: u32) -> u32 {
    (device_type << 16) | (access << 14) | (function << 2) | method
}

/// Maps the ring into the calling process. Output: [`MapRingOutput`].
/// Fails with ERROR_BUSY when another handle already holds the mapping, which
/// makes the driver the single-producer lock (`ProducerGuard` on macOS).
pub const IOCTL_MAP_RING: u32 = ctl_code(DEVICE_TYPE, 0x801, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
/// Unmaps it again (also happens automatically when the handle is closed).
pub const IOCTL_UNMAP_RING: u32 = ctl_code(DEVICE_TYPE, 0x802, METHOD_BUFFERED, FILE_READ_DATA | FILE_WRITE_DATA);
/// Output: [`DriverInfo`]. Needs no mapping; any reader may ask.
pub const IOCTL_GET_INFO: u32 = ctl_code(DEVICE_TYPE, 0x803, METHOD_BUFFERED, FILE_READ_DATA);

/// User-mode path of the control device.
pub const CONTROL_DEVICE_PATH: &str = r"\\.\PabulongMic";

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct MapRingOutput {
    pub user_address: u64,
    pub bytes: u32,
    pub version: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct DriverInfo {
    pub version: u32,
    pub driver_version: u32,
    pub consumer_running_count: u32,
    pub producer_attached: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Mem(Vec<u64>);

    impl Mem {
        fn new() -> Self {
            Mem(vec![0; ring_bytes(CAPACITY_FRAMES, FRAME_LENGTH).div_ceil(4096) * 4096 / 8])
        }
        fn ring(&mut self) -> Ring {
            let len = self.0.len() * 8;
            unsafe { Ring::initialize(self.0.as_mut_ptr() as *mut u8, len, 7).unwrap() }
        }
    }

    fn frame(v: f32) -> Vec<f32> {
        vec![v; FRAME_LENGTH as usize]
    }

    #[test]
    fn layout_matches_the_c_header() {
        assert_eq!(MAGIC, 0x524D_4250);
        assert_eq!(offset::DRIVER_VERSION + 4, 88);
        assert_eq!(ring_bytes(CAPACITY_FRAMES, FRAME_LENGTH), 128 + 32 * 480 * 4);
        assert_eq!(IOCTL_MAP_RING, 0x9A5C_E004);
        assert_eq!(IOCTL_UNMAP_RING, 0x9A5C_E008);
        assert_eq!(IOCTL_GET_INFO, 0x9A5C_600C);
        assert_eq!(core::mem::size_of::<MapRingOutput>(), 16);
        assert_eq!(core::mem::size_of::<DriverInfo>(), 16);
    }

    #[test]
    fn attach_validates_the_header() {
        let mut mem = Mem::new();
        let len = mem.0.len() * 8;
        let ptr = mem.0.as_mut_ptr() as *mut u8;
        assert_eq!(unsafe { Ring::attach(ptr, len) }.unwrap_err(), RingError::BadMagic);
        mem.ring();
        let ring = unsafe { Ring::attach(ptr, len) }.unwrap();
        assert_eq!(ring.capacity(), CAPACITY_FRAMES);
        assert_eq!(ring.health().driver_version, 7);
        assert_eq!(unsafe { Ring::attach(ptr, 1000) }.unwrap_err(), RingError::TooSmall);
    }

    #[test]
    fn frames_pass_through_in_order() {
        let mut mem = Mem::new();
        let ring = mem.ring();
        ring.set_producer_active(true);
        let mut out = frame(9.0);
        for i in 0..100 {
            assert!(ring.write_frame(&frame(i as f32)));
            assert!(ring.read_frame_latest(&mut out));
            assert_eq!(out[0], i as f32);
            assert_eq!(out[479], i as f32);
        }
        let h = ring.health();
        assert_eq!((h.underrun_count, h.overrun_count, h.dropped_count), (0, 0, 0));
    }

    #[test]
    fn empty_ring_reads_silence_and_counts_an_underrun_only_while_active() {
        let mut mem = Mem::new();
        let ring = mem.ring();
        let mut out = frame(1.0);
        assert!(!ring.read_frame_latest(&mut out));
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(ring.health().underrun_count, 0, "an idle producer is not an underrun");
        ring.set_producer_active(true);
        assert!(!ring.read_frame_latest(&mut out));
        assert_eq!(ring.health().underrun_count, 1);
    }

    #[test]
    fn reader_keeps_latency_bounded() {
        let mut mem = Mem::new();
        let ring = mem.ring();
        ring.set_producer_active(true);
        for i in 0..10 {
            ring.write_frame(&frame(i as f32));
        }
        let mut out = frame(0.0);
        assert!(ring.read_frame_latest(&mut out));
        // Only the newest MAX_BUFFERED_FRAMES survive: 7, 8, 9.
        assert_eq!(out[0], 7.0);
        assert_eq!(ring.health().dropped_count, 7);
        assert_eq!(ring.health().available_read_frames, 2);
    }

    #[test]
    fn full_ring_rejects_writes_without_touching_the_reader() {
        let mut mem = Mem::new();
        let ring = mem.ring();
        for i in 0..CAPACITY_FRAMES {
            assert!(ring.write_frame(&frame(i as f32)));
        }
        assert!(!ring.write_frame(&frame(99.0)));
        let h = ring.health();
        assert_eq!(h.overrun_count, 1);
        assert_eq!(h.available_write_frames, 0);
    }

    #[test]
    fn inactive_producer_is_silence_even_with_queued_frames() {
        let mut mem = Mem::new();
        let ring = mem.ring();
        ring.write_frame(&frame(0.5));
        let mut out = frame(1.0);
        assert!(!ring.read_frame_latest(&mut out));
        assert!(out.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn concurrent_producer_and_reader_never_tear_a_frame() {
        let mut mem = Mem::new();
        let ring = mem.ring();
        ring.set_producer_active(true);
        let producer = std::thread::spawn(move || {
            let mut written = 0u32;
            while written < 20_000 {
                if ring.write_frame(&frame(written as f32)) {
                    written += 1;
                } else {
                    std::thread::yield_now();
                }
            }
        });
        let mut out = frame(0.0);
        let mut last = -1.0f32;
        let mut seen = 0;
        while !producer.is_finished() || ring.health().available_read_frames > 0 {
            if ring.read_frame_latest(&mut out) {
                assert!(out.iter().all(|s| *s == out[0]), "torn frame");
                assert!(out[0] > last, "frames out of order");
                last = out[0];
                seen += 1;
            }
        }
        producer.join().unwrap();
        assert!(seen > 0);
    }
}
