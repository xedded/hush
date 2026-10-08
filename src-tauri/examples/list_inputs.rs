//! Dev probe: lists input devices with their ids and whether a default config can be read.
use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();
    if let Some(d) = host.default_input_device() {
        println!("default: {}", d.description().map(|x| x.name().to_string()).unwrap_or_default());
    }
    for d in host.input_devices().unwrap() {
        let name = d.description().map(|x| format!("{} [{:?}]", x.name(), x.interface_type())).unwrap_or_default();
        let id = d.id().map(|i| i.to_string()).unwrap_or_default();
        let cfg = d.default_input_config().map(|c| format!("{} Hz {} ch", c.sample_rate(), c.channels()));
        println!("{name}\n    {id}\n    {cfg:?}");
    }
}
