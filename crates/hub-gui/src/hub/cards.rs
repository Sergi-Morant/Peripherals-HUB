//! Cards for adjustable device settings: lighting effects, DPI, polling rate and host mode.

use eframe::egui::{self, RichText};
use hub_core::{Capability, Effect, EffectSetting, Motor, Rgb, Settings};

use super::Hub;
use crate::manager::{Cmd, DeviceView};
use crate::theme::{self, card, card_header, hint, segmented};
use crate::{color_panel, icons};

const PRESET_DPI: [u16; 5] = [400, 800, 1200, 1600, 3200];

impl Hub {
    pub(super) fn effects_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, effects: &[Effect]) {
        let key = (d.info.id.clone(), 0usize);
        let default = EffectSetting {
            effect: if effects.contains(&Effect::Static) { Effect::Static } else { effects[0] },
            ..Default::default()
        };
        let mut cur = *self.effects.get(&key).unwrap_or(&default);
        let mut changed = false;
        card(ui, |ui| {
            card_header(ui, icons::PALETTE, "Lighting", "Effect and colour. The mouse cannot report its current effect.");
            let items: Vec<(&str, &str)> = effects.iter().map(|e| ("", e.label())).collect();
            let selected = effects.iter().position(|e| *e == cur.effect).unwrap_or(0);
            if let Some(i) = segmented(ui, &items, selected) {
                cur.effect = effects[i];
                changed = true;
            }
            if cur.effect.uses_color() {
                ui.add_space(8.0);
                if let Some(c) = color_panel::show(ui, [cur.color.0, cur.color.1, cur.color.2]) {
                    cur.color = Rgb(c[0], c[1], c[2]);
                    changed = true;
                }
            }
            if cur.effect.uses_period() {
                ui.add_space(8.0);
                let mut secs = cur.period_ms as f32 / 1000.0;
                ui.horizontal(|ui| {
                    ui.label("Speed (seconds per cycle)");
                    if ui.add(egui::Slider::new(&mut secs, 0.5..=10.0).step_by(0.1)).changed() {
                        cur.period_ms = (secs * 1000.0) as u16;
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Brightness");
                    let mut b = cur.brightness as f32;
                    if ui.add(egui::Slider::new(&mut b, 1.0..=100.0).step_by(1.0)).changed() {
                        cur.brightness = b as u8;
                        changed = true;
                    }
                });
            }
        });
        if changed {
            self.effects.insert(key, cur);
            self.manager.send(Cmd::SetEffect { id: d.info.id.clone(), zone: 0, setting: cur });
        }
    }

    pub(super) fn dpi_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, values: &[u16]) {
        let current = self.dpi_edit.get(&d.info.id).copied().or(d.settings.dpi);
        let mut new = None;
        card(ui, |ui| {
            card_header(ui, icons::GAUGE, "Sensitivity (DPI)", "Applies immediately");
            let Some(cur) = current else {
                hint(ui, "The mouse did not report its DPI (it may be asleep: move it).");
                return;
            };
            let mut idx = values.iter().position(|v| *v >= cur).unwrap_or(values.len() - 1) as f64;
            ui.label(RichText::new(format!("{cur} DPI")).size(22.0).strong());
            let slider = egui::Slider::new(&mut idx, 0.0..=(values.len() - 1) as f64)
                .step_by(1.0)
                .show_value(false)
                .smart_aim(false);
            ui.spacing_mut().slider_width = 420.0;
            if ui.add(slider).changed() {
                new = Some(values[idx as usize]);
            }
            ui.horizontal(|ui| {
                for p in PRESET_DPI.into_iter().filter(|p| values.contains(p)) {
                    if ui.selectable_label(cur == p, p.to_string()).clicked() {
                        new = Some(p);
                    }
                }
            });
        });
        if let Some(dpi) = new.filter(|v| Some(*v) != current) {
            self.dpi_edit.insert(d.info.id.clone(), dpi);
            self.manager.send(Cmd::SetDpi { id: d.info.id.clone(), dpi });
        }
    }

    pub(super) fn rate_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, rates: &[u16]) {
        let current = self.rate_edit.get(&d.info.id).copied().or(d.settings.report_rate);
        // While the mouse runs its own stored profile it refuses polling-rate changes (HID++ error).
        let editable = d.settings.host_mode != Some(false);
        let mut new = None;
        card(ui, |ui| {
            card_header(ui, icons::TIMER, "Polling rate", "How often the mouse reports movement");
            let labels: Vec<String> = rates.iter().map(|r| format!("{r} Hz")).collect();
            let items: Vec<(&str, &str)> = labels.iter().map(|l| ("", l.as_str())).collect();
            let selected = current.and_then(|c| rates.iter().position(|r| *r == c)).unwrap_or(usize::MAX);
            let clicked = ui.add_enabled_ui(editable, |ui| segmented(ui, &items, selected)).inner;
            if editable {
                new = clicked.map(|i| rates[i]);
            } else {
                hint(ui, "Turn on host mode (above) to change the polling rate.");
            }
        });
        if let Some(hz) = new.filter(|v| Some(*v) != current) {
            self.rate_edit.insert(d.info.id.clone(), hz);
            self.manager.send(Cmd::SetReportRate { id: d.info.id.clone(), hz });
        }
    }

    pub(super) fn rumble_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, motors: &[Motor]) {
        let mut send: Option<Option<u64>> = None; // Some(Some(ms)) = test, Some(None) = stop
        card(ui, |ui| {
            card_header(ui, icons::LIGHTNING, "Vibration", "Set the strength of each motor, then try it");
            for &m in motors {
                let level = self.rumble.entry((d.info.id.clone(), m)).or_insert(50);
                ui.horizontal(|ui| {
                    ui.add_sized([150.0, 20.0], egui::Label::new(m.label()));
                    let mut v = *level as f32;
                    if ui.add(egui::Slider::new(&mut v, 0.0..=100.0).step_by(1.0)).changed() {
                        *level = v as u8;
                    }
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if theme::primary_button(ui, true, format!("{}  Try it (1 s)", icons::PLAY)).clicked() {
                    send = Some(Some(1000));
                }
                if ui.button("Stop").clicked() {
                    send = Some(None);
                }
            });
        });
        if let Some(for_ms) = send {
            let levels: Vec<(Motor, u8)> = motors
                .iter()
                .map(|&m| (m, if for_ms.is_some() { *self.rumble.get(&(d.info.id.clone(), m)).unwrap_or(&0) } else { 0 }))
                .collect();
            self.manager.send(Cmd::Rumble { id: d.info.id.clone(), levels, for_ms });
        }
    }

    /// Play a MIDI file through the device: real notes on tone-capable actuators, a rhythm otherwise.
    pub(super) fn music_card(&mut self, ui: &mut egui::Ui, d: &DeviceView) {
        let tones = d.caps.iter().find_map(|c| if let Capability::Tones(n) = c { Some(*n as usize) } else { None });
        let motors = d.caps.iter().find_map(|c| if let Capability::Rumble(m) = c { Some(m.clone()) } else { None });
        let playing = self.playing.contains(&d.info.id);
        let has_speaker = d.caps.contains(&Capability::Speaker);
        // Pads with a speaker and motors can use either or both; the others only vibrate.
        let (mut use_speaker, mut use_rumble) = *self.music_output.get(&d.info.id).unwrap_or(&(true, true));
        let mut action: Option<bool> = None; // Some(true) = play, Some(false) = stop
        card(ui, |ui| {
            card_header(
                ui,
                icons::MUSIC_NOTES,
                "Play music",
                if tones.is_some() {
                    "Plays the notes of a MIDI file through the haptic actuators"
                } else {
                    "These motors cannot change pitch, so a MIDI file becomes a rhythm: low notes on one motor, high notes on the other"
                },
            );
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Choose MIDI file", icons::FILE_IMAGE)).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi"]).pick_file() {
                        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| hub_midi::parse(&b)) {
                            Ok(notes) => {
                                let name = path.file_name().map_or("MIDI file".into(), |n| n.to_string_lossy().into_owned());
                                self.midi = Some((name, notes));
                            }
                            Err(e) => {
                                self.midi = None;
                                self.status = format!("MIDI: {e}");
                                self.status_is_error = true;
                            }
                        }
                    }
                }
                if let Some((name, notes)) = &self.midi {
                    ui.label(
                        RichText::new(format!("{name}  ·  {} notes, {} s", notes.len(), hub_midi::total_ms(notes) / 1000))
                            .color(theme::TEXT_MUTED),
                    );
                }
            });
            if has_speaker {
                ui.horizontal(|ui| {
                    ui.label("Play through");
                    ui.checkbox(&mut use_speaker, "Speaker");
                    ui.checkbox(&mut use_rumble, "Vibration");
                });
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let ready = self.midi.is_some() && (!has_speaker || use_speaker || use_rumble);
                if playing {
                    if ui.button(format!("{}  Stop", icons::PAUSE)).clicked() {
                        action = Some(false);
                    }
                } else if theme::primary_button(ui, ready, format!("{}  Play", icons::PLAY)).clicked() {
                    action = Some(true);
                }
            });
        });
        if has_speaker {
            self.music_output.insert(d.info.id.clone(), (use_speaker, use_rumble));
        }
        match action {
            Some(true) => {
                let Some((_, notes)) = &self.midi else { return };
                self.playing.insert(d.info.id.clone());
                if has_speaker && use_speaker {
                    // Sound from the speaker, with the vibration rhythm riding along if wanted.
                    let states = match (&motors, use_rumble) {
                        (Some(m), true) => hub_midi::rumble_states(&hub_midi::plan_rhythm(notes, m)),
                        _ => vec![],
                    };
                    let audio = crate::manager::Audio {
                        samples: hub_midi::render_pcm(notes, 32_000),
                        rate: 32_000,
                        channels: 1,
                    };
                    let volume = *self.volume.get(&d.info.id).unwrap_or(&60);
                    self.manager.send(Cmd::PlayAudio {
                        id: d.info.id.clone(),
                        audio: std::sync::Arc::new(audio),
                        volume,
                        rumble: std::sync::Arc::new(states),
                    });
                } else {
                    let events = match (tones, motors) {
                        (Some(voices), _) => hub_midi::plan_tones(notes, voices),
                        (None, Some(m)) => hub_midi::plan_rhythm(notes, &m),
                        _ => vec![],
                    };
                    self.manager.send(Cmd::Play { id: d.info.id.clone(), events });
                }
            }
            Some(false) => {
                self.manager.stop_audio();
                self.manager.send(Cmd::StopPlay { id: d.info.id.clone() });
            }
            None => {}
        }
    }

    pub(super) fn host_mode_card(&mut self, ui: &mut egui::Ui, d: &DeviceView) {
        let Some(mut host) = d.settings.host_mode else { return };
        card(ui, |ui| {
            card_header(
                ui,
                icons::GEAR,
                "Who controls the mouse",
                "Its own stored profile (as set by the official app) or this hub",
            );
            if ui.checkbox(&mut host, "Hub controls polling rate (host mode)").changed() {
                self.manager.send(Cmd::SetHostMode { id: d.info.id.clone(), host });
            }
            hint(
                ui,
                "Off: the mouse uses the profile stored on it. On: the hub can change the polling rate, and the \
                 stored profile is paused, so its button actions may stop working until you turn this off again.",
            );
        });
    }
}

/// Forget optimistic edits the device has confirmed.
pub(super) fn settled(edit: &mut std::collections::HashMap<String, u16>, devices: &[DeviceView], pick: fn(&Settings) -> Option<u16>) {
    edit.retain(|id, v| devices.iter().find(|d| &d.info.id == id).is_some_and(|d| pick(&d.settings) != Some(*v)));
}
