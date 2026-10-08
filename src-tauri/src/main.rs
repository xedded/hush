// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--name-virtual-mic" => std::process::exit(hush_lib::name_virtual_mic(false)),
            "--restore-virtual-mic" => std::process::exit(hush_lib::name_virtual_mic(true)),
            _ => {}
        }
    }
    hush_lib::run()
}
