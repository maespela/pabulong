//! Lock-free single-producer / single-consumer frame ring buffer.
//!
//! Exactly one thread may call `write` and exactly one thread may call `read`.
//! (The cross-process shared-memory ring used by the macOS HAL driver is a
//! platform concern and lives outside this crate.)

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct RingBuffer {
    capacity: u32, // slots, one more than the usable frame capacity
    frame_length: usize,
    storage: UnsafeCell<Box<[f32]>>,
    read_index: AtomicU32,
    write_index: AtomicU32,
}

// SAFETY: the SPSC contract (one writer, one reader) plus the acquire/release
// ordering on the indices means the two sides never touch the same slot.
unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}

impl RingBuffer {
    pub fn new(frame_capacity: u32, frame_length: usize) -> Option<Self> {
        if frame_capacity == 0 || frame_length == 0 {
            return None;
        }
        let slots = frame_capacity + 1;
        Some(Self {
            capacity: slots,
            frame_length,
            storage: UnsafeCell::new(vec![0.0; slots as usize * frame_length].into_boxed_slice()),
            read_index: AtomicU32::new(0),
            write_index: AtomicU32::new(0),
        })
    }

    fn next(&self, index: u32) -> u32 {
        (index + 1) % self.capacity
    }

    /// Appends one frame. Returns false (without writing) when the ring is full
    /// or `frame` has the wrong length.
    pub fn write(&self, frame: &[f32]) -> bool {
        if frame.len() != self.frame_length {
            return false;
        }
        let write = self.write_index.load(Ordering::Relaxed);
        let next = self.next(write);
        if next == self.read_index.load(Ordering::Acquire) {
            return false;
        }
        unsafe {
            let base = (*self.storage.get())
                .as_mut_ptr()
                .add(write as usize * self.frame_length);
            std::ptr::copy_nonoverlapping(frame.as_ptr(), base, self.frame_length);
        }
        self.write_index.store(next, Ordering::Release);
        true
    }

    /// Pops one frame into `out`. When empty, fills `out` with silence and
    /// returns false.
    pub fn read(&self, out: &mut [f32]) -> bool {
        if out.len() != self.frame_length {
            return false;
        }
        let read = self.read_index.load(Ordering::Relaxed);
        if read == self.write_index.load(Ordering::Acquire) {
            out.fill(0.0);
            return false;
        }
        unsafe {
            let base = (*self.storage.get())
                .as_ptr()
                .add(read as usize * self.frame_length);
            std::ptr::copy_nonoverlapping(base, out.as_mut_ptr(), self.frame_length);
        }
        self.read_index.store(self.next(read), Ordering::Release);
        true
    }

    pub fn available_read(&self) -> u32 {
        let read = self.read_index.load(Ordering::Acquire);
        let write = self.write_index.load(Ordering::Acquire);
        if write >= read {
            write - read
        } else {
            self.capacity - read + write
        }
    }

    pub fn available_write(&self) -> u32 {
        self.capacity - 1 - self.available_read()
    }

    pub fn capacity(&self) -> u32 {
        self.capacity - 1
    }

    /// Resets to empty. Call only while neither side is mid-operation.
    pub fn clear(&self) {
        self.read_index.store(0, Ordering::Release);
        self.write_index.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_order_and_reports_full_and_empty() {
        let ring = RingBuffer::new(3, 4).unwrap();
        assert_eq!(ring.capacity(), 3);
        for v in 1..=3 {
            assert!(ring.write(&[v as f32; 4]));
        }
        assert!(!ring.write(&[9.0; 4]), "a full ring must refuse writes");
        assert_eq!(ring.available_read(), 3);
        assert_eq!(ring.available_write(), 0);

        let mut out = [0.0; 4];
        for v in 1..=3 {
            assert!(ring.read(&mut out));
            assert_eq!(out, [v as f32; 4]);
        }
        assert!(!ring.read(&mut out), "an empty ring must report underrun");
        assert_eq!(out, [0.0; 4], "underrun must produce silence");
    }

    #[test]
    fn rejects_wrong_frame_length_and_wraps_around() {
        let ring = RingBuffer::new(2, 3).unwrap();
        assert!(!ring.write(&[1.0; 2]));
        let mut out = [0.0; 3];
        for round in 0..10 {
            assert!(ring.write(&[round as f32; 3]));
            assert!(ring.read(&mut out));
            assert_eq!(out, [round as f32; 3]);
        }
    }

    #[test]
    fn works_across_threads() {
        use std::sync::Arc;
        let ring = Arc::new(RingBuffer::new(8, 16).unwrap());
        let producer = {
            let ring = ring.clone();
            std::thread::spawn(move || {
                let mut sent = 0u32;
                while sent < 2000 {
                    if ring.write(&[sent as f32; 16]) {
                        sent += 1;
                    }
                }
            })
        };
        let mut expected = 0u32;
        let mut out = [0.0; 16];
        while expected < 2000 {
            if ring.read(&mut out) {
                assert_eq!(out, [expected as f32; 16]);
                expected += 1;
            }
        }
        producer.join().unwrap();
    }
}
