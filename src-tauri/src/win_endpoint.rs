//! Small helpers for reading Windows audio endpoint properties (Core Audio
//! property stores). cpal does not expose these.

use anyhow::{Context, Result};
use windows::core::{GUID, HSTRING};
use windows::Win32::Foundation::{PROPERTYKEY, RPC_E_CHANGED_MODE};
use windows::Win32::Media::Audio::{IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc, PROPVARIANT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

/// COM initialised for the current scope. Threads that already run COM in
/// another apartment (the UI thread is single-threaded) are used as they are.
pub struct Com {
    owned: bool,
}

impl Com {
    pub fn init() -> Result<Self> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr == RPC_E_CHANGED_MODE {
            return Ok(Com { owned: false });
        }
        hr.ok().context("COM init")?;
        Ok(Com { owned: true })
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.owned {
            unsafe { CoUninitialize() };
        }
    }
}

pub fn read_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    unsafe {
        let mut value: PROPVARIANT = store.GetValue(key).ok()?;
        let text = PropVariantToStringAlloc(&value).ok().map(|p| {
            let s = p.to_string().unwrap_or_default();
            CoTaskMemFree(Some(p.0 as _));
            s
        });
        let _ = PropVariantClear(&mut value);
        text
    }
}

pub fn enumerator() -> Result<IMMDeviceEnumerator> {
    Ok(unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)? })
}

/// cpal ids look like `wasapi:{0.0.1.00000000}.{guid}`; Core Audio wants the part after the host.
fn endpoint(cpal_id: &str) -> Result<IMMDevice> {
    let raw = cpal_id.split_once(':').map_or(cpal_id, |(_, rest)| rest);
    Ok(unsafe { enumerator()?.GetDevice(&HSTRING::from(raw))? })
}

/// Path of the device node an endpoint belongs to, e.g. `{1}.BTHENUM\{...}` for a
/// Bluetooth headset. Undocumented; the documented enumerator key only says
/// MMDEVAPI for endpoints.
const PKEY_ENDPOINT_DEVNODE_PATH: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xb3f8fa53_0004_438e_9003_51a46e139bfc), pid: 39 };

/// True for Bluetooth endpoints (hands-free and stereo profiles). cpal reports
/// these as an unknown interface type, so ask Windows which bus the device sits on.
pub fn is_bluetooth(cpal_id: &str) -> bool {
    let check = || -> Result<bool> {
        let _com = Com::init()?;
        let store = unsafe { endpoint(cpal_id)?.OpenPropertyStore(STGM_READ)? };
        Ok(read_string(&store, &PKEY_ENDPOINT_DEVNODE_PATH).is_some_and(|p| p.to_ascii_uppercase().contains("}.BTH")))
    };
    check().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual: dumps string properties of every capture endpoint.
    #[test]
    #[ignore]
    fn dump_capture_properties() {
        use windows::Win32::Media::Audio::{eCapture, DEVICE_STATE_ACTIVE};
        let _com = Com::init().unwrap();
        unsafe {
            let devices = enumerator().unwrap().EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE).unwrap();
            for i in 0..devices.GetCount().unwrap() {
                let store = devices.Item(i).unwrap().OpenPropertyStore(STGM_READ).unwrap();
                println!("--- endpoint {i}");
                for p in 0..store.GetCount().unwrap() {
                    let mut key = PROPERTYKEY::default();
                    store.GetAt(p, &mut key).unwrap();
                    if let Some(v) = read_string(&store, &key) {
                        if v.len() < 200 {
                            println!("{:?},{} = {v}", key.fmtid, key.pid);
                        }
                    }
                }
            }
        }
    }
}
