// No console window in release builds (on Windows a GUI app otherwise opens one next to it).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    capricorn_lib::run();
}
