//! Dev probe: plays a 440 Hz tone at -20 dBFS into "CABLE Input" for four seconds.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

fn main() {
    let host = cpal::default_host();
    for d in host.output_devices().unwrap() {
        println!("output: {:?}", d.description().map(|x| x.name().to_string()));
    }
    let dev = host
        .output_devices()
        .unwrap()
        .find(|d| d.description().map(|x| x.name().contains("CABLE Input")).unwrap_or(false))
        .expect("CABLE Input not found");
    let def = dev.default_output_config().unwrap();
    println!("CABLE Input default: {} Hz, {} ch, {:?}", def.sample_rate(), def.channels(), def.sample_format());
    let ch = def.channels() as usize;
    let cfg = cpal::StreamConfig { channels: def.channels(), sample_rate: 48_000, buffer_size: cpal::BufferSize::Default };
    let mut phase = 0f32;
    let stream = dev
        .build_output_stream::<f32, _, _>(
            cfg,
            move |data: &mut [f32], _| {
                for f in data.chunks_exact_mut(ch) {
                    phase = (phase + 440.0 / 48_000.0) % 1.0;
                    f.fill(0.1 * (phase * std::f32::consts::TAU).sin());
                }
            },
            |e| eprintln!("stream error: {e}"),
            None,
        )
        .unwrap();
    stream.play().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(4));
}
