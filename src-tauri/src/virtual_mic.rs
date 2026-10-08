//! Gives the VB-Cable recording endpoint the name meeting apps show, so users
//! pick "Hush Microphone" instead of "CABLE Output". Writing endpoint
//! properties needs administrator rights, so the installer runs this once
//! elevated (`hush.exe --name-virtual-mic`, and `--restore-virtual-mic` on
//! uninstall). The app itself only reads; when VB-Cable was installed after
//! Hush it relaunches itself elevated with the same flag.
//!
//! VB-Cable is not bundled: its terms do not allow embedding it in another
//! installer without the author's agreement, so users install it themselves.

pub const HUSH_MIC_NAME: &str = "Hush Microphone";
const CABLE_CAPTURE_NAME: &str = "CABLE Output";
#[cfg_attr(not(windows), allow(dead_code))]
const VIRTUAL_INTERFACE_MARKER: &str = "VB-Audio";

/// Where users download the virtual cable themselves.
#[cfg(not(target_os = "macos"))]
pub const DOWNLOAD_URL: &str = "https://vb-audio.com/Cable/";
#[cfg(target_os = "macos")]
pub const DOWNLOAD_URL: &str = "https://existential.audio/blackhole/";

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// The virtual cable is not installed (or its endpoint is not active yet).
    Missing,
    /// Windows only: installed but still called "CABLE Output" in meeting apps.
    Rename,
    /// Installed and ready to be picked in meeting apps.
    Ready,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Renamed,
    AlreadyNamed,
    NotInstalled,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{anyhow, Context, Result};
    use windows::core::PWSTR;
    use windows::Win32::Devices::FunctionDiscovery::{PKEY_DeviceInterface_FriendlyName, PKEY_Device_DeviceDesc};
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::Media::Audio::{eCapture, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE};
    use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc, PROPVARIANT};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
        STGM_READWRITE,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

    struct Com;
    impl Com {
        fn init() -> Result<Self> {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().context("COM init")?;
            Ok(Com)
        }
    }
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn read_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
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

    fn write_string(store: &IPropertyStore, key: &PROPERTYKEY, text: &str) -> Result<()> {
        let mut wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // The store copies the value, so pointing at our own buffer is fine; it is not cleared.
        let mut value = PROPVARIANT::default();
        unsafe {
            let inner = &mut *value.Anonymous.Anonymous;
            inner.vt = VT_LPWSTR;
            inner.Anonymous.pwszVal = PWSTR(wide.as_mut_ptr());
            store.SetValue(key, &value)?;
            store.Commit()?;
        }
        Ok(())
    }

    /// The VB-Cable recording endpoint, whether or not it was renamed already.
    fn find_cable_capture() -> Result<Option<IMMDevice>> {
        unsafe {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let devices = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE)?;
            for i in 0..devices.GetCount()? {
                let device = devices.Item(i)?;
                let store = device.OpenPropertyStore(STGM_READ)?;
                let iface = read_string(&store, &PKEY_DeviceInterface_FriendlyName).unwrap_or_default();
                let desc = read_string(&store, &PKEY_Device_DeviceDesc).unwrap_or_default();
                if iface.contains(VIRTUAL_INTERFACE_MARKER) && (desc == CABLE_CAPTURE_NAME || desc == HUSH_MIC_NAME) {
                    return Ok(Some(device));
                }
            }
            Ok(None)
        }
    }

    pub fn status() -> Result<Status> {
        let _com = Com::init()?;
        let Some(device) = find_cable_capture()? else { return Ok(Status::Missing) };
        let name = unsafe { read_string(&device.OpenPropertyStore(STGM_READ)?, &PKEY_Device_DeviceDesc) };
        Ok(if name.as_deref() == Some(HUSH_MIC_NAME) { Status::Ready } else { Status::Rename })
    }

    /// Give the VB-Cable recording endpoint `name` (one of the two known names).
    pub fn set_name(name: &str) -> Result<Outcome> {
        let _com = Com::init()?;
        let Some(device) = find_cable_capture()? else { return Ok(Outcome::NotInstalled) };
        unsafe {
            let current = read_string(&device.OpenPropertyStore(STGM_READ)?, &PKEY_Device_DeviceDesc);
            if current.as_deref() == Some(name) {
                return Ok(Outcome::AlreadyNamed);
            }
            let store = device
                .OpenPropertyStore(STGM_READWRITE)
                .map_err(|e| anyhow!("needs administrator rights to rename the microphone: {e}"))?;
            write_string(&store, &PKEY_Device_DeviceDesc, name)?;
        }
        Ok(Outcome::Renamed)
    }
}

#[cfg(windows)]
use imp::set_name;

#[cfg(not(windows))]
fn set_name(_name: &str) -> anyhow::Result<Outcome> {
    Ok(Outcome::NotInstalled)
}

/// Read-only check used by the UI; failures count as missing.
pub fn status() -> Status {
    #[cfg(windows)]
    {
        imp::status().unwrap_or(Status::Missing)
    }
    #[cfg(not(windows))]
    {
        if crate::audio::devices::find_virtual_sink().is_some() { Status::Ready } else { Status::Missing }
    }
}

/// Installer step: show the virtual microphone as "Hush Microphone".
pub fn ensure_named() -> anyhow::Result<Outcome> {
    set_name(HUSH_MIC_NAME)
}

/// Uninstaller step: give VB-Cable its own name back, since it stays installed.
pub fn restore_name() -> anyhow::Result<Outcome> {
    set_name(CABLE_CAPTURE_NAME)
}

#[cfg(windows)]
mod shell {
    use anyhow::{anyhow, Context, Result};
    use windows::core::{w, HSTRING};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, ShellExecuteW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

    pub fn open_download_page() -> Result<()> {
        let url = HSTRING::from(super::DOWNLOAD_URL);
        let r = unsafe { ShellExecuteW(None, w!("open"), &url, None, None, SW_SHOWNORMAL) };
        // ShellExecute reports success with a value above 32.
        if r.0 as usize > 32 { Ok(()) } else { Err(anyhow!("could not open browser")) }
    }

    /// Relaunch this executable elevated with `--name-virtual-mic` and wait for it.
    /// Windows shows the UAC prompt; declining it returns an error.
    pub fn rename_elevated() -> Result<()> {
        let exe = HSTRING::from(std::env::current_exe().context("own path")?.as_os_str());
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: w!("runas"),
            lpFile: windows::core::PCWSTR(exe.as_ptr()),
            lpParameters: w!("--name-virtual-mic"),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        unsafe {
            ShellExecuteExW(&mut info).context("administratörsbehörighet nekades")?;
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 1u32;
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
            let _ = CloseHandle(info.hProcess);
            if code == 0 { Ok(()) } else { Err(anyhow!("renaming failed with code {code}")) }
        }
    }
}

#[cfg(windows)]
pub use shell::{open_download_page, rename_elevated};

#[cfg(not(windows))]
pub fn open_download_page() -> anyhow::Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener).arg(DOWNLOAD_URL).spawn()?;
    Ok(())
}

#[cfg(not(windows))]
pub fn rename_elevated() -> anyhow::Result<()> {
    Ok(())
}
