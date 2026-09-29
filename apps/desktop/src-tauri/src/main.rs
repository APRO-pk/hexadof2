// Prevent a second console window from appearing on Windows in release builds,
// while keeping the console in a debug build so a startup problem stays visible.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! HexaDOF desktop entry point.

fn main() {
    hexadof_desktop_lib::run();
}
