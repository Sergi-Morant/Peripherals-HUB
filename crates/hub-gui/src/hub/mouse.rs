//! The "Buttons and profiles" card of mice with onboard profiles: pick which profile runs, change
//! what each button does, back up and restore the mouse's memory.

use eframe::egui::{self, RichText};
use hub_core::{ButtonAction, Capability, Profile};

use super::Hub;
use crate::manager::{Cmd, DeviceView};
use crate::theme::{self, card, card_header, hint, segmented};
use crate::icons;

/// What the UI knows about a mouse's profiles.
pub struct ProfileState {
    pub profiles: Vec<Profile>,
    /// Index of the profile the mouse runs.
    pub active: Option<usize>,
    /// Profile shown in the editor.
    pub shown: usize,
}

/// Actions offered in the drop-down of every button.
const CHOICES: &[(&str, ButtonAction)] = &[
    ("Left click", ButtonAction::Mouse(0x0001)),
    ("Right click", ButtonAction::Mouse(0x0002)),
    ("Middle click", ButtonAction::Mouse(0x0004)),
    ("Back", ButtonAction::Mouse(0x0008)),
    ("Forward", ButtonAction::Mouse(0x0010)),
    ("Scroll left", ButtonAction::Mouse(0x0040)),
    ("Scroll right", ButtonAction::Mouse(0x0080)),
    ("DPI shift (hold)", ButtonAction::Function { id: 0x07, data: 0 }),
    ("Next DPI", ButtonAction::Function { id: 0x03, data: 0 }),
    ("Previous DPI", ButtonAction::Function { id: 0x04, data: 0 }),
    ("Cycle DPI", ButtonAction::Function { id: 0x05, data: 0 }),
    ("Default DPI", ButtonAction::Function { id: 0x06, data: 0 }),
    ("Next profile", ButtonAction::Function { id: 0x08, data: 0 }),
    ("Previous profile", ButtonAction::Function { id: 0x09, data: 0 }),
    ("Cycle profile", ButtonAction::Function { id: 0x0A, data: 0 }),
    ("Play / pause", ButtonAction::Consumer(0x00CD)),
    ("Next track", ButtonAction::Consumer(0x00B5)),
    ("Previous track", ButtonAction::Consumer(0x00B6)),
    ("Volume up", ButtonAction::Consumer(0x00E9)),
    ("Volume down", ButtonAction::Consumer(0x00EA)),
    ("Mute", ButtonAction::Consumer(0x00E2)),
    ("Disabled", ButtonAction::Disabled),
];

/// USB HID usage of a pressed key, for the "Keyboard key" choice.
fn hid_usage(key: egui::Key) -> Option<u8> {
    use egui::Key::*;
    let letters = [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z];
    if let Some(i) = letters.iter().position(|k| *k == key) {
        return Some(0x04 + i as u8);
    }
    let digits = [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0];
    if let Some(i) = digits.iter().position(|k| *k == key) {
        return Some(0x1E + i as u8);
    }
    let fkeys = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];
    if let Some(i) = fkeys.iter().position(|k| *k == key) {
        return Some(0x3A + i as u8);
    }
    Some(match key {
        Enter => 0x28,
        Escape => 0x29,
        Backspace => 0x2A,
        Tab => 0x2B,
        Space => 0x2C,
        Insert => 0x49,
        Home => 0x4A,
        PageUp => 0x4B,
        Delete => 0x4C,
        End => 0x4D,
        PageDown => 0x4E,
        ArrowRight => 0x4F,
        ArrowLeft => 0x50,
        ArrowDown => 0x51,
        ArrowUp => 0x52,
        _ => return None,
    })
}

impl Hub {
    pub(super) fn profiles_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, labels: &[String]) {
        let id = d.info.id.clone();
        // Ask the mouse once; it takes a moment, so this is not done on every scan.
        if !self.profiles_asked.contains(&id) {
            self.profiles_asked.insert(id.clone());
            self.manager.send(Cmd::LoadProfiles { id: id.clone() });
        }
        // A key pressed while a button waits for one becomes its action.
        let mut set: Option<(usize, usize, ButtonAction)> = None;
        if let Some((cid, profile, button)) = self.key_capture.clone().filter(|(cid, _, _)| *cid == id) {
            let pressed = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => hid_usage(*key).map(|usage| {
                        let m = u8::from(modifiers.ctrl) | u8::from(modifiers.shift) << 1 | u8::from(modifiers.alt) << 2
                            | u8::from(modifiers.mac_cmd) << 3;
                        ButtonAction::Key { modifiers: m, usage }
                    }),
                    _ => None,
                })
            });
            if let Some(a) = pressed {
                set = Some((profile, button, a));
                self.key_capture = None;
                let _ = cid;
            }
        }

        let mut load = false;
        let mut run: Option<usize> = None;
        let mut start_capture: Option<(usize, usize)> = None;
        let mut cancel_capture = false;
        let mut backup_save = false;
        let mut backup_restore = false;
        let mut shown_change: Option<usize> = None;
        let capturing = self.key_capture.clone();

        let state = self.profile_data.get(&id);
        card(ui, |ui| {
            card_header(
                ui,
                icons::CURSOR_CLICK,
                "Buttons and profiles",
                "Stored on the mouse itself, so they work in every program and on every computer",
            );
            let Some(st) = state else {
                ui.label(RichText::new("Reading the profiles from the mouse...").color(theme::TEXT_MUTED));
                if ui.button("Try again").clicked() {
                    load = true;
                }
                return;
            };
            if st.profiles.is_empty() {
                hint(ui, "The mouse reported no profiles.");
                return;
            }
            let names: Vec<String> = st
                .profiles
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let base = if p.name.is_empty() { format!("Profile {}", p.index) } else { p.name.clone() };
                    if st.active == Some(i) { format!("{base}  ·  running") } else { base }
                })
                .collect();
            let items: Vec<(&str, &str)> = names.iter().map(|n| ("", n.as_str())).collect();
            if let Some(i) = segmented(ui, &items, st.shown.min(st.profiles.len() - 1)) {
                shown_change = Some(i);
            }
            let shown = shown_change.unwrap_or(st.shown).min(st.profiles.len() - 1);
            let p = &st.profiles[shown];
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let hz = if p.report_interval_ms > 0 { 1000 / p.report_interval_ms as u32 } else { 0 };
                let dpis: Vec<String> = p.dpi_levels.iter().filter(|v| **v > 0).map(|v| v.to_string()).collect();
                ui.label(RichText::new(format!("{hz} Hz  ·  DPI steps {}", dpis.join(", "))).color(theme::TEXT_MUTED));
                if st.active != Some(shown) && theme::primary_button(ui, true, format!("{}  Run this profile", icons::PLAY)).clicked() {
                    run = Some(shown);
                }
            });
            ui.add_space(6.0);
            egui::Grid::new(("buttons", id.clone())).num_columns(3).spacing([14.0, 6.0]).show(ui, |ui| {
                for (b, action) in p.buttons.iter().enumerate() {
                    let name = labels.get(b).cloned().unwrap_or_else(|| format!("Button {}", b + 1));
                    ui.label(RichText::new(name).strong());
                    let waiting = capturing.as_ref().is_some_and(|(_, cp, cb)| *cp == shown && *cb == b);
                    if waiting {
                        ui.label(RichText::new("Press a key on your keyboard...").color(theme::ACCENT));
                        if ui.button("Cancel").clicked() {
                            cancel_capture = true;
                        }
                    } else {
                        let current = action.label();
                        egui::ComboBox::from_id_salt(("action", shown, b))
                            .width(220.0)
                            .selected_text(capitalise(&current))
                            .show_ui(ui, |ui| {
                                for (name, choice) in CHOICES {
                                    if ui.selectable_label(*choice == *action, *name).clicked() {
                                        set = Some((shown, b, *choice));
                                    }
                                }
                                ui.separator();
                                if ui.selectable_label(false, "Keyboard key...").clicked() {
                                    start_capture = Some((shown, b));
                                }
                            });
                        ui.label("");
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Save a backup...", icons::FLOPPY)).clicked() {
                    backup_save = true;
                }
                if ui.button(format!("{}  Restore a backup...", icons::RESTORE)).clicked() {
                    backup_restore = true;
                }
            });
            hint(
                ui,
                "Changes are written to the mouse's memory right away. Save a backup first: restoring a backup \
                 of a mouse that was still on its factory profiles returns it to them.",
            );
        });

        if let Some(i) = shown_change {
            if let Some(st) = self.profile_data.get_mut(&id) {
                st.shown = i;
            }
        }
        if load {
            self.manager.send(Cmd::LoadProfiles { id: id.clone() });
        }
        if let Some(i) = run {
            self.manager.send(Cmd::SetActiveProfile { id: id.clone(), index: i });
        }
        if let Some((profile, button)) = start_capture {
            self.key_capture = Some((id.clone(), profile, button));
        }
        if cancel_capture {
            self.key_capture = None;
        }
        if let Some((profile, button, action)) = set {
            self.manager.send(Cmd::SetButton { id: id.clone(), profile, button, action });
        }
        if backup_save {
            let file = format!("{}-profiles.txt", d.info.name.replace(' ', "-").to_lowercase());
            if let Some(path) = rfd::FileDialog::new().set_file_name(file).save_file() {
                self.manager.send(Cmd::ExportProfiles { id: id.clone(), path });
            }
        }
        if backup_restore {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                self.manager.send(Cmd::ImportProfiles { id, path });
            }
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// Labels printed on the buttons (empty when unknown) from a device's capabilities.
pub(super) fn button_labels(d: &DeviceView) -> Option<Vec<String>> {
    d.caps.iter().find_map(|c| if let Capability::Profiles { labels, .. } = c { Some(labels.clone()) } else { None })
}
