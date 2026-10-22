//! The hub window: a sidebar of whatever is connected right now, and a page for the selected device.

use std::collections::HashMap;

use eframe::egui::{self, Frame, Margin, RichText};
use hub_core::{Battery, Capability, EffectSetting, Kind, Rgb, Transport};

mod cards;

use crate::manager::{Cmd, DeviceView, Manager, Update};
use crate::theme::{self, card, card_header, chip, page_header};
use crate::{color_panel, icons, keyboard};

/// Sidebar id of the RK S85, which has its own worker and pages (see `keyboard.rs`).
const KEYBOARD_ID: &str = "keyboard";

pub struct Hub {
    manager: Manager,
    keyboard: keyboard::App,
    keyboard_present: bool,
    devices: Vec<DeviceView>,
    /// Unique sidebar name per device id: identical devices get " 1", " 2", ... suffixes.
    labels: HashMap<String, String>,
    selected: Option<String>,
    /// Colour last chosen per (device, zone), shown in the colour panel.
    colors: HashMap<(String, usize), [u8; 3]>,
    effects: HashMap<(String, usize), EffectSetting>,
    /// Values just sent and not yet confirmed by the device, so sliders don't jump back mid-drag.
    dpi_edit: HashMap<String, u16>,
    rate_edit: HashMap<String, u16>,
    /// Rumble test levels per device and motor (0..=100).
    rumble: HashMap<(String, hub_core::Motor), u8>,
    /// The loaded MIDI file: display name and notes.
    midi: Option<(String, Vec<hub_midi::Note>)>,
    /// Devices currently playing music.
    playing: std::collections::HashSet<String>,
    status: String,
    status_is_error: bool,
}

impl Hub {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Hub {
        icons::install(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        Hub {
            manager: Manager::spawn(cc.egui_ctx.clone()),
            keyboard: keyboard::App::new(&cc.egui_ctx),
            keyboard_present: false,
            devices: vec![],
            labels: HashMap::new(),
            selected: None,
            colors: HashMap::new(),
            effects: HashMap::new(),
            dpi_edit: HashMap::new(),
            rate_edit: HashMap::new(),
            rumble: HashMap::new(),
            midi: None,
            playing: Default::default(),
            status: "Looking for devices...".into(),
            status_is_error: false,
        }
    }

    fn poll(&mut self) {
        while let Ok(u) = self.manager.rx.try_recv() {
            match u {
                Update::Devices(d) => {
                    self.status = match d.len() + self.keyboard_present as usize {
                        0 => "No devices connected".into(),
                        1 => "1 device connected".into(),
                        n => format!("{n} devices connected"),
                    };
                    self.status_is_error = false;
                    self.labels = unique_labels(&d);
                    cards::settled(&mut self.dpi_edit, &d, |s| s.dpi);
                    cards::settled(&mut self.rate_edit, &d, |s| s.report_rate);
                    self.devices = d;
                }
                Update::Keyboard(present) => {
                    if present {
                        self.keyboard.connect();
                    } else {
                        self.keyboard.disconnect();
                    }
                    self.keyboard_present = present;
                }
                Update::PlaybackEnded(id) => {
                    self.playing.remove(&id);
                }
                Update::Error(e) => {
                    self.dpi_edit.clear();
                    self.rate_edit.clear();
                    self.status = e;
                    self.status_is_error = true;
                }
            }
        }
        // Keep the selection valid as devices come and go.
        let still_there = |id: &str, s: &Hub| {
            (id == KEYBOARD_ID && s.keyboard_present) || s.devices.iter().any(|d| d.info.id == id)
        };
        if !self.selected.as_deref().is_some_and(|id| still_there(id, self)) {
            self.selected = if self.keyboard_present {
                Some(KEYBOARD_ID.into())
            } else {
                self.devices.first().map(|d| d.info.id.clone())
            };
        }
    }
}

fn kind_icon(k: Kind) -> &'static str {
    match k {
        Kind::Keyboard => icons::KEYBOARD,
        Kind::Mouse => icons::MOUSE,
        Kind::Gamepad => icons::GAMEPAD,
        Kind::Adapter => icons::PLUGS_CONNECTED,
    }
}

fn transport_icon(t: Transport) -> &'static str {
    match t {
        Transport::Usb => icons::PLUGS_CONNECTED,
        Transport::Bluetooth => icons::BLUETOOTH,
        Transport::Dongle => icons::BROADCAST,
    }
}

/// Name each device uniquely. Devices sharing a name are numbered in a stable order (by serial /
/// Bluetooth address when known, else by id), so "DualShock 4 1" stays the same pad across rescans.
fn unique_labels(devices: &[DeviceView]) -> HashMap<String, String> {
    let mut by_name: HashMap<&str, Vec<&DeviceView>> = HashMap::new();
    for d in devices {
        by_name.entry(d.info.name.as_str()).or_default().push(d);
    }
    let mut out = HashMap::new();
    for (name, mut group) in by_name {
        group.sort_by_key(|d| d.info.serial.clone().unwrap_or_else(|| d.info.id.clone()));
        // Same device over different links (e.g. keyboard on Bluetooth and dongle): name the link.
        let distinct_links =
            group.iter().map(|d| d.info.transport.label()).collect::<std::collections::HashSet<_>>().len() == group.len();
        for (i, d) in group.iter().enumerate() {
            let label = match (group.len() > 1, distinct_links) {
                (false, _) => name.to_string(),
                (true, true) => format!("{name} ({})", d.info.transport.label()),
                (true, false) => format!("{name} {}", i + 1),
            };
            out.insert(d.info.id.clone(), label);
        }
    }
    out
}

fn battery_icon(b: Battery) -> &'static str {
    match (b.charging, b.percent) {
        (true, _) => icons::BATTERY_CHARGING,
        (_, Some(p)) if p >= 75 => icons::BATTERY_FULL,
        (_, Some(p)) if p >= 25 => icons::BATTERY_MEDIUM,
        _ => icons::BATTERY_EMPTY,
    }
}

fn battery_text(b: Battery) -> String {
    let pct = match (b.percent, b.coarse) {
        (Some(p), true) => match p {
            0..=10 => "Low",
            11..=40 => "Medium",
            41..=70 => "High",
            _ => "Full",
        }
        .to_string(),
        (Some(p), false) => format!("{p}%"),
        _ => "?".to_string(),
    };
    if b.charging {
        format!("{pct}, charging")
    } else {
        pct
    }
}

impl eframe::App for Hub {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();

        egui::Panel::left("devices")
            .exact_size(250.0)
            .resizable(false)
            .frame(Frame::new().fill(theme::SIDEBAR).inner_margin(Margin::same(12)))
            .show(root, |ui| self.sidebar(ui));

        egui::Panel::bottom("hub_status")
            .frame(Frame::new().fill(theme::SIDEBAR).inner_margin(Margin::symmetric(16, 6)))
            .show(root, |ui| {
                let (icon, c) = if self.status_is_error {
                    (icons::WARNING, theme::DANGER)
                } else {
                    (icons::INFO, theme::TEXT_MUTED)
                };
                ui.label(RichText::new(format!("{icon}  {}", self.status)).size(12.5).color(c));
            });

        egui::CentralPanel::default().frame(Frame::new().fill(theme::BG)).show(root, |ui| {
            match self.selected.clone().as_deref() {
                Some(KEYBOARD_ID) => self.keyboard.show(ui),
                Some(id) => match self.devices.iter().find(|d| d.info.id == id).cloned() {
                    Some(d) => self.device_page(ui, &d),
                    None => self.empty_page(ui),
                },
                None => self.empty_page(ui),
            }
        });
    }
}

impl Hub {
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("DEVICES").size(11.5).strong().color(theme::TEXT_MUTED));
        ui.add_space(6.0);
        if self.keyboard_present {
            let active = self.selected.as_deref() == Some(KEYBOARD_ID);
            if theme::nav_item(ui, icons::KEYBOARD, "RK S85", "Keyboard · USB", active).clicked() {
                self.selected = Some(KEYBOARD_ID.into());
            }
        }
        let mut order: Vec<&DeviceView> = self.devices.iter().collect();
        order.sort_by_key(|d| self.labels.get(&d.info.id).cloned().unwrap_or_default());
        for d in order {
            let active = self.selected.as_deref() == Some(d.info.id.as_str());
            let mut sub = format!("{} · {}", kind_label(d.info.kind), d.info.transport.label());
            if let Some(b) = d.battery {
                sub.push_str(&format!(" · {}", battery_text(b).split(',').next().unwrap_or("")));
            }
            let label = self.labels.get(&d.info.id).map_or(d.info.name.as_str(), String::as_str);
            if theme::nav_item(ui, kind_icon(d.info.kind), label, &sub, active).clicked() {
                self.selected = Some(d.info.id.clone());
            }
        }
        if !self.keyboard_present && self.devices.is_empty() {
            ui.label(RichText::new("Nothing connected").color(theme::TEXT_MUTED));
        }
    }

    fn empty_page(&self, ui: &mut egui::Ui) {
        ui.add_space(80.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(460.0);
            card(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(icons::PLUGS).size(42.0).color(theme::TEXT_MUTED));
                    ui.label(RichText::new("No device selected").size(17.0).strong());
                    ui.label(
                        RichText::new(
                            "Plug in a keyboard, mouse receiver or controller, or pair one over Bluetooth. \
                             It shows up in the list on its own.",
                        )
                        .color(theme::TEXT_MUTED),
                    );
                });
            });
        });
    }

    fn device_page(&mut self, ui: &mut egui::Ui, d: &DeviceView) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.set_width(ui.available_width());
            Frame::new().inner_margin(Margin::symmetric(22, 0)).show(ui, |ui| {
                let label = self.labels.get(&d.info.id).map_or(d.info.name.as_str(), String::as_str);
                page_header(ui, kind_icon(d.info.kind), label, kind_label(d.info.kind));
                ui.horizontal(|ui| {
                    chip(ui, transport_icon(d.info.transport), d.info.transport.label());
                    if let Some(serial) = &d.info.serial {
                        chip(ui, icons::INFO, serial.clone());
                    }
                    if let Some(b) = d.battery {
                        chip(ui, battery_icon(b), battery_text(b));
                    }
                });
                ui.add_space(12.0);
                if let Some(problem) = &d.problem {
                    card(ui, |ui| card_header(ui, icons::WARNING, "Cannot control this device", problem));
                    ui.add_space(12.0);
                } else if d.caps.is_empty() {
                    card(ui, |ui| match (d.info.kind, d.info.transport) {
                        (Kind::Keyboard, Transport::Bluetooth) => card_header(
                            ui,
                            icons::BLUETOOTH,
                            "Connected over Bluetooth",
                            "That is all the hub can tell. Battery, lighting and key mapping are only available \
                             with the USB cable plugged in.",
                        ),
                        (Kind::Keyboard, _) => card_header(
                            ui,
                            icons::BROADCAST,
                            "2.4 GHz dongle plugged in",
                            "The hub can see the dongle but not whether the keyboard is using it. Battery, \
                             lighting and key mapping are only available with the USB cable plugged in.",
                        ),
                        _ => card_header(
                            ui,
                            icons::INFO,
                            "Detected",
                            "The hub can see this device and how it is connected, but controls for it are not built yet.",
                        ),
                    });
                    ui.add_space(12.0);
                }
                if let Some(b) = d.battery {
                    card(ui, |ui| {
                        card_header(ui, battery_icon(b), "Battery", "Refreshed every 30 seconds");
                        let frac = b.percent.map_or(0.0, |p| p as f32 / 100.0);
                        ui.add(egui::ProgressBar::new(frac).text(battery_text(b)).fill(theme::OK));
                    });
                    ui.add_space(12.0);
                }
                if d.caps.iter().any(|c| matches!(c, Capability::Tones(_) | Capability::Rumble(_))) {
                    self.music_card(ui, d);
                    ui.add_space(12.0);
                }
                if d.caps.contains(&Capability::HostMode) {
                    self.host_mode_card(ui, d);
                    ui.add_space(12.0);
                }
                for cap in &d.caps {
                    match cap {
                        Capability::Dpi(values) => {
                            self.dpi_card(ui, d, values);
                            ui.add_space(12.0);
                        }
                        Capability::Rumble(motors) => {
                            self.rumble_card(ui, d, motors);
                            ui.add_space(12.0);
                        }
                        Capability::Tones(_) => {}
                        Capability::ReportRate(rates) => {
                            self.rate_card(ui, d, rates);
                            ui.add_space(12.0);
                        }
                        Capability::Effects(effects) if !effects.is_empty() => {
                            self.effects_card(ui, d, effects);
                            ui.add_space(12.0);
                        }
                        // Devices with effects get their colour from the effects card.
                        Capability::Rgb { zones } if !d.caps.iter().any(|c| matches!(c, Capability::Effects(e) if !e.is_empty())) => {
                            for (z, zone) in zones.iter().enumerate() {
                                self.color_card(ui, d, z, zone);
                                ui.add_space(12.0);
                            }
                        }
                        _ => {}
                    }
                }
            });
        });
    }

    fn color_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, zone: usize, name: &str) {
        let key = (d.info.id.clone(), zone);
        let rgb = *self.colors.get(&key).unwrap_or(&[255, 255, 255]);
        card(ui, |ui| {
            card_header(ui, icons::PALETTE, name, "Static colour");
            if let Some(c) = color_panel::show(ui, rgb) {
                self.colors.insert(key, c);
                self.manager.send(Cmd::SetRgb { id: d.info.id.clone(), zone, color: Rgb(c[0], c[1], c[2]) });
            }
        });
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Keyboard => "Keyboard",
        Kind::Mouse => "Mouse",
        Kind::Gamepad => "Game controller",
        Kind::Adapter => "Adapter",
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use hub_core::Info;

    fn pad(id: &str, serial: Option<&str>) -> DeviceView {
        DeviceView {
            info: Info {
                id: id.into(),
                name: "Wireless Controller".into(),
                kind: Kind::Gamepad,
                transport: Transport::Bluetooth,
                serial: serial.map(String::from),
            },
            caps: vec![],
            battery: None,
            settings: Default::default(),
            problem: None,
        }
    }

    #[test]
    fn identical_pads_are_numbered_by_serial_not_path() {
        // The pad with the lower address stays "1" even if its hidraw node sorts later.
        let l = unique_labels(&[pad("/dev/hidraw3", Some("aa:bb")), pad("/dev/hidraw9", Some("00:11"))]);
        assert_eq!(l["/dev/hidraw9"], "Wireless Controller 1");
        assert_eq!(l["/dev/hidraw3"], "Wireless Controller 2");
    }

    #[test]
    fn single_device_keeps_plain_name() {
        let l = unique_labels(&[pad("a", None)]);
        assert_eq!(l["a"], "Wireless Controller");
    }

    #[test]
    fn same_device_on_two_links_is_named_by_link() {
        let mut bt = pad("a", Some("00:11"));
        bt.info.name = "RK S85".into();
        let mut dongle = bt.clone();
        dongle.info.id = "b".into();
        dongle.info.transport = Transport::Dongle;
        let l = unique_labels(&[bt, dongle]);
        assert_eq!(l["a"], "RK S85 (Bluetooth)");
        assert_eq!(l["b"], "RK S85 (2.4 GHz)");
    }
}
