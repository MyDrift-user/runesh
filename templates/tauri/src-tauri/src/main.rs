// Prevents a console window on Windows in release builds
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Replace with your app's lib crate name
    YOUR_APP_desktop::run();
}
