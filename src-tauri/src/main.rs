// Evita que se abra una consola en Windows en modo release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    claudebar_lib::run();
}
