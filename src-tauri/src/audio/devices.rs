//! Device discovery. The virtual cable (VB-Cable on Windows, BlackHole on
//! macOS) is where Hush sends processed audio; its endpoints never show up
//! as selectable microphones.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, DeviceId, InterfaceType};

/// Output endpoint Hush writes processed audio into.
#[cfg(not(target_os = "macos"))]
const VIRTUAL_SINK: &str = "CABLE Input";
#[cfg(target_os = "macos")]
const VIRTUAL_SINK: &str = "BlackHole";

/// Name the meeting apps show for the virtual microphone. On Windows Hush renames
/// the VB-Cable endpoint; BlackHole on macOS cannot be renamed.
#[cfg(not(target_os = "macos"))]
pub const VIRTUAL_MIC_NAME: &str = "Hush Microphone";
#[cfg(target_os = "macos")]
pub const VIRTUAL_MIC_NAME: &str = "BlackHole 2ch";

#[cfg(not(target_os = "macos"))]
pub const MISSING_SINK_MESSAGE: &str = "VB-CABLE saknas. Installera den från vb-audio.com och starta om datorn.";
#[cfg(target_os = "macos")]
pub const MISSING_SINK_MESSAGE: &str = "BlackHole saknas. Installera BlackHole 2ch från existential.audio.";

/// Any endpoint containing one of these is a virtual cable, never a real microphone.
const VIRTUAL_MARKERS: [&str; 3] = ["VB-Audio", "CABLE ", "BlackHole"];

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InputInfo {
    pub id: String,
    pub name: String,
    pub bluetooth: bool,
    pub is_default: bool,
}

fn is_virtual(name: &str) -> bool {
    VIRTUAL_MARKERS.iter().any(|m| name.contains(m))
}

/// Strip the driver suffix Windows appends, "Mikrofon (Jabra Link 390)" -> "Jabra Link 390".
pub fn display_name(raw: &str) -> String {
    match (raw.find('('), raw.rfind(')')) {
        (Some(open), Some(close)) if close > open + 1 && close == raw.len() - 1 => {
            let inner = raw[open + 1..close].trim();
            let outer = raw[..open].trim();
            // Keep the outer part when the inner one is just a driver brand.
            if inner.is_empty() || outer.is_empty() {
                raw.trim().to_string()
            } else {
                inner.to_string()
            }
        }
        _ => raw.trim().to_string(),
    }
}

fn device_name(d: &Device) -> String {
    d.description().map(|desc| desc.name().to_string()).unwrap_or_else(|_| d.to_string())
}

pub fn list_inputs() -> Vec<InputInfo> {
    let host = cpal::default_host();
    let default_id = host.default_input_device().and_then(|d| d.id().ok());
    let Ok(devices) = host.input_devices() else { return Vec::new() };
    let mut out: Vec<InputInfo> = devices
        .filter_map(|d| {
            let name = device_name(&d);
            if is_virtual(&name) {
                return None;
            }
            let id = d.id().ok()?;
            let id_text = id.to_string();
            Some(InputInfo {
                is_default: default_id.as_ref() == Some(&id),
                bluetooth: is_bluetooth(&d, &id_text),
                id: id_text,
                name: display_name(&name),
            })
        })
        .collect();
    out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    out
}

/// Resolve a saved input id, falling back to the system default microphone
/// (unless that default is itself the virtual cable).
pub fn find_input(id: Option<&str>) -> Option<Device> {
    let host = cpal::default_host();
    if let Some(dev) = id
        .and_then(|s| s.parse::<DeviceId>().ok())
        .and_then(|id| host.device_by_id(&id))
    {
        return Some(dev);
    }
    if let Some(d) = host.default_input_device() {
        if !is_virtual(&device_name(&d)) {
            return Some(d);
        }
    }
    host.input_devices().ok()?.find(|d| !is_virtual(&device_name(d)))
}

pub fn find_virtual_sink() -> Option<Device> {
    let host = cpal::default_host();
    host.output_devices().ok()?.find(|d| device_name(d).contains(VIRTUAL_SINK))
}

/// Where "listen to yourself" plays: the system's default playback device.
/// Never the virtual cable, which would feed the sound straight back into the meeting.
pub fn monitor_output() -> Result<Device, &'static str> {
    let dev = cpal::default_host().default_output_device().ok_or("Ingen uppspelningsenhet hittades.")?;
    if is_virtual(&device_name(&dev)) {
        return Err("Standardutgången är den virtuella kabeln. Välj dina hörlurar som uppspelningsenhet i datorns ljudinställningar.");
    }
    Ok(dev)
}

pub fn input_display_name(d: &Device) -> String {
    display_name(&device_name(d))
}

/// Bluetooth microphones switch the headset to the hands-free profile: telephone
/// quality for both directions. cpal often reports their interface as unknown on
/// Windows, so Windows is asked as well.
pub fn is_bluetooth(d: &Device, id: &str) -> bool {
    let by_cpal = d.description().map(|desc| desc.interface_type() == InterfaceType::Bluetooth).unwrap_or(false);
    #[cfg(windows)]
    {
        by_cpal || crate::win_endpoint::is_bluetooth(id)
    }
    #[cfg(not(windows))]
    {
        let _ = id;
        by_cpal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_unwraps_windows_endpoint_names() {
        assert_eq!(display_name("Mikrofon (Jabra Link 390)"), "Jabra Link 390");
        assert_eq!(display_name("Headset Microphone (Jabra Evolve2 65)"), "Jabra Evolve2 65");
        assert_eq!(display_name("Plain Mic"), "Plain Mic");
        assert_eq!(display_name("(Odd)"), "(Odd)");
    }

    /// Manual check against the real devices: `cargo test --lib print_inputs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_inputs() {
        for i in list_inputs() {
            println!("{:<50} bluetooth={} default={}", i.name, i.bluetooth, i.is_default);
        }
    }

    #[test]
    fn virtual_cable_endpoints_are_hidden() {
        assert!(is_virtual("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(is_virtual("CABLE In 16ch (VB-Audio Virtual Cable)"));
        assert!(is_virtual("BlackHole 2ch"));
        assert!(!is_virtual("Mikrofon (Jabra Link 390)"));
        assert!(!is_virtual("MacBook Pro Microphone"));
    }
}
