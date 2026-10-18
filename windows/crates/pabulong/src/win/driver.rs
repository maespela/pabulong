//! Connection to the Pabulong Mic driver's control device. Holding the
//! mapping makes this process the single producer (the driver refuses a
//! second one with ERROR_BUSY), like `ProducerGuard` on macOS.

use crate::controller::RingOpenError;
use pabulong_ring::{DriverInfo, MapRingOutput, Ring, CONTROL_DEVICE_PATH, IOCTL_GET_INFO, IOCTL_MAP_RING, IOCTL_UNMAP_RING};
use std::ffi::c_void;
use windows::Win32::Foundation::{CloseHandle, ERROR_BUSY, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;

pub struct DriverConnection {
    handle: HANDLE,
    ring: Ring,
}

// SAFETY: the handle is only used for DeviceIoControl/CloseHandle, which are
// thread-safe; `Ring` is Send + Sync.
unsafe impl Send for DriverConnection {}
unsafe impl Sync for DriverConnection {}

impl DriverConnection {
    pub fn open() -> Result<Self, RingOpenError> {
        unsafe {
            let handle = CreateFileW(
                &super::h(CONTROL_DEVICE_PATH),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
            .map_err(|_| RingOpenError::DriverMissing)?;
            let mut out = MapRingOutput::default();
            let mut returned = 0u32;
            let mapped = DeviceIoControl(
                handle,
                IOCTL_MAP_RING,
                None,
                0,
                Some(&mut out as *mut _ as *mut c_void),
                size_of::<MapRingOutput>() as u32,
                Some(&mut returned),
                None,
            );
            if let Err(e) = mapped {
                let _ = CloseHandle(handle);
                return Err(if e.code() == windows::core::HRESULT::from_win32(ERROR_BUSY.0) { RingOpenError::Busy } else { RingOpenError::DriverMissing });
            }
            if returned as usize != size_of::<MapRingOutput>() || out.user_address == 0 {
                let _ = CloseHandle(handle);
                return Err(RingOpenError::DriverMissing);
            }
            match Ring::attach(out.user_address as *mut u8, out.bytes as usize) {
                Ok(ring) => Ok(Self { handle, ring }),
                Err(_) => {
                    let _ = CloseHandle(handle);
                    Err(RingOpenError::DriverMissing)
                }
            }
        }
    }

    pub fn ring(&self) -> Ring {
        self.ring
    }

    /// Asks the driver directly (works even if the mapping is stale).
    pub fn info(&self) -> Option<DriverInfo> {
        let mut info = DriverInfo::default();
        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                self.handle,
                IOCTL_GET_INFO,
                None,
                0,
                Some(&mut info as *mut _ as *mut c_void),
                size_of::<DriverInfo>() as u32,
                Some(&mut returned),
                None,
            )
            .ok()?;
        }
        (returned as usize == size_of::<DriverInfo>()).then_some(info)
    }
}

impl Drop for DriverConnection {
    fn drop(&mut self) {
        self.ring.set_producer_active(false);
        unsafe {
            let mut returned = 0u32;
            let _ = DeviceIoControl(self.handle, IOCTL_UNMAP_RING, None, 0, None, 0, Some(&mut returned), None);
            let _ = CloseHandle(self.handle);
        }
    }
}
