// Prevents an additional console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    auto_tagger_lib::run();
}
