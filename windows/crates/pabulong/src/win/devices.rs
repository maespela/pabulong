//! Audio endpoint discovery (the Windows `DeviceManager`).

use crate::text;
use std::ffi::c_void;
use windows::core::{Interface, GUID, HRESULT, IUnknown, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::{PKEY_DeviceInterface_FriendlyName, PKEY_Device_FriendlyName};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, eMultimedia, eRender, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputDevice {
    /// WASAPI endpoint id; stored as `selectedInputDeviceUID`.
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
}

pub fn device_id(device: &IMMDevice) -> Option<String> {
    unsafe {
        let p = device.GetId().ok()?;
        let id = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        id
    }
}

fn property(device: &IMMDevice, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let mut value = store.GetValue(key).ok()?;
        let text = PropVariantToStringAlloc(&value).ok().and_then(|p| {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            s
        });
        let _ = PropVariantClear(&mut value);
        text
    }
}

pub fn friendly_name(device: &IMMDevice) -> String {
    property(device, &PKEY_Device_FriendlyName).unwrap_or_else(|| "Microphone".into())
}

/// True for our own virtual mic, which must never be captured (feedback).
pub fn is_pabulong(device: &IMMDevice) -> bool {
    let adapter = property(device, &PKEY_DeviceInterface_FriendlyName).unwrap_or_default();
    adapter.contains(text::VIRTUAL_MIC_NAME) || friendly_name(device).contains(text::VIRTUAL_MIC_NAME)
}

fn default_capture_id(e: &IMMDeviceEnumerator) -> Option<String> {
    unsafe { e.GetDefaultAudioEndpoint(eCapture, eConsole).ok().as_ref().and_then(device_id) }
}

pub fn default_render_id(e: &IMMDeviceEnumerator) -> Option<String> {
    unsafe { e.GetDefaultAudioEndpoint(eRender, eConsole).ok().as_ref().and_then(device_id) }
}

/// Physical microphones (Pabulong Mic excluded), for the Microphone picker.
pub fn list_inputs() -> Vec<InputDevice> {
    let Ok(e) = enumerator() else { return Vec::new() };
    let default = default_capture_id(&e);
    let mut out = Vec::new();
    unsafe {
        let Ok(collection) = e.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else { return out };
        let count = collection.GetCount().unwrap_or(0);
        for i in 0..count {
            let Ok(device) = collection.Item(i) else { continue };
            if is_pabulong(&device) {
                continue;
            }
            let Some(id) = device_id(&device) else { continue };
            out.push(InputDevice { is_default: default.as_deref() == Some(id.as_str()), name: friendly_name(&device), id });
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    NoDevice,
    IsVirtual,
}

/// The physical mic to capture: the chosen one if present, else the system
/// default if it is physical, else the first physical mic.
pub fn resolve_physical_input(preferred: Option<&str>) -> Result<IMMDevice, ResolveError> {
    let e = enumerator().map_err(|_| ResolveError::NoDevice)?;
    unsafe {
        if let Some(id) = preferred {
            if let Ok(device) = e.GetDevice(&super::h(id)) {
                if is_pabulong(&device) {
                    return Err(ResolveError::IsVirtual);
                }
                if device.GetState().is_ok_and(|s| s == DEVICE_STATE_ACTIVE) {
                    return Ok(device);
                }
            }
        }
        if let Ok(device) = e.GetDefaultAudioEndpoint(eCapture, eConsole) {
            if !is_pabulong(&device) {
                return Ok(device);
            }
        }
        let collection = e.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).map_err(|_| ResolveError::NoDevice)?;
        for i in 0..collection.GetCount().unwrap_or(0) {
            if let Ok(device) = collection.Item(i) {
                if !is_pabulong(&device) {
                    return Ok(device);
                }
            }
        }
    }
    Err(ResolveError::NoDevice)
}

/// Pabulong Mic is the system default input (`isVirtualDeviceDefaultInput`).
pub fn virtual_mic_is_default_input() -> bool {
    let Ok(e) = enumerator() else { return false };
    unsafe {
        [eConsole, eCommunications]
            .into_iter()
            .any(|role| e.GetDefaultAudioEndpoint(eCapture, role).is_ok_and(|d| is_pabulong(&d)))
    }
}

// Windows has no public API for changing the default device. Every audio
// switcher uses the IPolicyConfig interface that the Sound control panel
// itself uses; it has been stable since Windows 7.
const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
const IID_IPOLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
/// IUnknown (3) + 10 methods before SetDefaultEndpoint.
const SET_DEFAULT_ENDPOINT_SLOT: usize = 13;

fn set_default_capture(id: &str) -> bool {
    unsafe {
        let Ok(unknown) = CoCreateInstance::<_, IUnknown>(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) else { return false };
        let mut raw: *mut c_void = std::ptr::null_mut();
        if unknown.query(&IID_IPOLICY_CONFIG, &mut raw).is_err() || raw.is_null() {
            return false;
        }
        let policy = IUnknown::from_raw(raw); // released on drop
        let vtable = *(raw as *const *const usize);
        let set: unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT = std::mem::transmute(*vtable.add(SET_DEFAULT_ENDPOINT_SLOT));
        let wide = super::wide(id);
        let ok = [eConsole, eMultimedia, eCommunications].into_iter().all(|role| set(policy.as_raw(), PCWSTR(wide.as_ptr()), role.0).is_ok());
        drop(policy);
        ok
    }
}

/// `moveDefaultInputToPhysical`: makes a real mic the default input again.
/// Returns true if the default changed.
pub fn move_default_input_to_physical(preferred: Option<&str>) -> bool {
    if !virtual_mic_is_default_input() {
        return false;
    }
    let target = resolve_physical_input(preferred).ok().and_then(|d| device_id(&d));
    match target {
        Some(id) => set_default_capture(&id),
        None => false,
    }
}
