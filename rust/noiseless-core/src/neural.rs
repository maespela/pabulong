//! RNNoise neural backend. Compiled only with the `rnnoise` feature, which
//! links the vendored static library (see `build.rs`).

use std::os::raw::c_void;

#[repr(C)]
struct DenoiseState {
    _private: [u8; 0],
}

extern "C" {
    fn rnnoise_create(model: *mut c_void) -> *mut DenoiseState;
    fn rnnoise_destroy(state: *mut DenoiseState);
    fn rnnoise_process_frame(state: *mut DenoiseState, out: *mut f32, input: *const f32) -> f32;
}

/// Owning wrapper around one RNNoise state (one audio stream).
pub struct Rnnoise {
    state: *mut DenoiseState,
}

// The state is only ever touched through `&mut self`, so moving it to another
// thread is sound.
unsafe impl Send for Rnnoise {}

impl Rnnoise {
    pub fn new() -> Option<Self> {
        let state = unsafe { rnnoise_create(std::ptr::null_mut()) };
        if state.is_null() {
            None
        } else {
            Some(Self { state })
        }
    }

    /// Processes one 480-sample frame in int16 range. Returns the voice
    /// probability in 0..1.
    pub fn process_frame(&mut self, input: &[f32], output: &mut [f32]) -> f32 {
        assert_eq!(input.len(), 480);
        assert_eq!(output.len(), 480);
        unsafe { rnnoise_process_frame(self.state, output.as_mut_ptr(), input.as_ptr()) }
    }
}

impl Drop for Rnnoise {
    fn drop(&mut self) {
        unsafe { rnnoise_destroy(self.state) };
    }
}
