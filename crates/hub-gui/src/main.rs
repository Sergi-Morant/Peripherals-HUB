#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod color_panel;
mod hub;
mod icons;
mod keyboard;
mod keyboard_link;
mod keyboard_view;
mod manager;
mod profile;
mod theme;
mod worker;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Peripherals HUB")
        .with_app_id("io.github.sergi-morant.peripherals-hub")
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([980.0, 640.0])
        .with_drag_and_drop(true);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icons/256x256.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Peripherals HUB", options, Box::new(|cc| Ok(Box::new(hub::Hub::new(cc)))))
}
