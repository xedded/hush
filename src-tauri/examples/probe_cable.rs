//! Dev probe: reads a recording device by name, by default the VB-Cable endpoint
//! meeting apps use ("Hush Microphone", formerly "CABLE Output"),
//! for a few seconds and prints the peak level per half second.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

fn main() {
    let host = cpal::default_host();
    let target = std::env::args().nth(1).unwrap_or_else(|| "Hush Microphone".into());
    let dev = host
        .input_devices()
        .unwrap()
        .find(|d| d.description().map(|x| x.name().contains(target.as_str())).unwrap_or(false))
        .expect("device not found");
    let cfg = dev.default_input_config().unwrap();
    println!("{target}: {} Hz, {} ch, {:?}", cfg.sample_rate(), cfg.channels(), cfg.sample_format());
    let peak = Arc::new(Mutex::new(0f32));
    let p = peak.clone();
    let stream = dev
        .build_input_stream::<f32, _, _>(
            cfg.config(),
            move |data: &[f32], _| {
                let m = data.iter().fold(0f32, |a, v| a.max(v.abs()));
                let mut g = p.lock().unwrap();
                *g = g.max(m);
            },
            |e| eprintln!("stream error: {e}"),
            None,
        )
        .unwrap();
    stream.play().unwrap();
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut g = peak.lock().unwrap();
        let db = if *g > 1e-6 { 20.0 * g.log10() } else { f32::NEG_INFINITY };
        println!("peak {db:.1} dBFS");
        *g = 0.0;
    }
}
