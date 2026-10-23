//! Cards for sound: the DualShock 4's speaker and the Steam Controller's haptic tones.

use std::sync::Arc;

use eframe::egui::{self, RichText};
use hub_core::Capability;

use super::Hub;
use crate::manager::{Audio, Cmd, DeviceView};
use crate::theme::{self, card, card_header, hint};
use crate::icons;

/// Sample rate the MIDI synth renders at (the speaker path resamples to 32 kHz anyway).
const SYNTH_RATE: u32 = 32_000;

/// Load a `.mid` (synthesised) or `.wav` file as audio.
fn load_audio(path: &std::path::Path) -> Result<Audio, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ext == "wav" {
        let mut r = hound::WavReader::open(path).map_err(|e| e.to_string())?;
        let spec = r.spec();
        let samples: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
            (hound::SampleFormat::Int, 16) => r.samples::<i16>().collect::<Result<_, _>>(),
            (hound::SampleFormat::Int, bits) if bits <= 32 => {
                let shift = bits - 16;
                r.samples::<i32>().map(|s| s.map(|v| (v >> shift) as i16)).collect()
            }
            (hound::SampleFormat::Float, _) => r.samples::<f32>().map(|s| s.map(|v| (v * 32767.0) as i16)).collect(),
            _ => return Err("unsupported WAV format".into()),
        }
        .map_err(|e| e.to_string())?;
        Ok(Audio { samples, rate: spec.sample_rate, channels: spec.channels })
    } else {
        let notes = hub_midi::parse(&std::fs::read(path).map_err(|e| e.to_string())?)?;
        Ok(Audio { samples: hub_midi::render_pcm(&notes, SYNTH_RATE), rate: SYNTH_RATE, channels: 1 })
    }
}

fn seconds(a: &Audio) -> usize {
    a.samples.len() / a.channels.max(1) as usize / a.rate.max(1) as usize
}

impl Hub {
    /// Play a MIDI or WAV file, or a test tone, on a built-in speaker.
    pub(super) fn speaker_card(&mut self, ui: &mut egui::Ui, d: &DeviceView) {
        let playing = self.playing.contains(&d.info.id);
        let mut play: Option<Arc<Audio>> = None;
        let mut stop = false;
        card(ui, |ui| {
            card_header(
                ui,
                icons::MUSIC_NOTES,
                "Speaker",
                "Plays sound through the controller's built-in speaker (over Bluetooth)",
            );
            ui.horizontal(|ui| {
                if ui.button(format!("{}  Choose a MIDI or WAV file", icons::FILE_IMAGE)).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Audio", &["mid", "midi", "wav"]).pick_file() {
                        match load_audio(&path) {
                            Ok(a) => {
                                let name = path.file_name().map_or("audio".into(), |n| n.to_string_lossy().into_owned());
                                self.audio = Some((name, Arc::new(a)));
                            }
                            Err(e) => {
                                self.status = format!("Audio: {e}");
                                self.status_is_error = true;
                            }
                        }
                    }
                }
                if let Some((name, a)) = &self.audio {
                    ui.label(RichText::new(format!("{name}  ·  {} s", seconds(a))).color(theme::TEXT_MUTED));
                }
            });
            let volume = self.volume.entry(d.info.id.clone()).or_insert(60);
            ui.horizontal(|ui| {
                ui.label("Volume");
                let mut v = *volume as f32;
                if ui.add(egui::Slider::new(&mut v, 0.0..=100.0).step_by(1.0)).changed() {
                    *volume = v as u8;
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if playing {
                    if ui.button(format!("{}  Stop", icons::PAUSE)).clicked() {
                        stop = true;
                    }
                } else {
                    if theme::primary_button(ui, self.audio.is_some(), format!("{}  Play", icons::PLAY)).clicked() {
                        play = self.audio.as_ref().map(|(_, a)| a.clone());
                    }
                    if ui.button("Test tone").clicked() {
                        let tone: Vec<i16> = (0..SYNTH_RATE * 2)
                            .map(|i| (20_000.0 * (std::f32::consts::TAU * 1500.0 * i as f32 / SYNTH_RATE as f32).sin()) as i16)
                            .collect();
                        play = Some(Arc::new(Audio { samples: tone, rate: SYNTH_RATE, channels: 1 }));
                    }
                }
            });
            hint(
                ui,
                "The controller buzzes once when playback starts. While a sound plays the hub cannot change \
                 anything else on this controller.",
            );
        });
        if let Some(audio) = play {
            let volume = *self.volume.get(&d.info.id).unwrap_or(&60);
            self.playing.insert(d.info.id.clone());
            self.manager.send(Cmd::PlayAudio { id: d.info.id.clone(), audio, volume, rumble: Default::default() });
        }
        if stop {
            self.manager.stop_audio();
        }
    }

    /// Try single tones on the Steam Controller's haptic actuators.
    pub(super) fn tones_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, voices: u8) {
        let (voice, hz, volume) = self.tone.entry(d.info.id.clone()).or_insert((0, 440, 60));
        let mut play = false;
        card(ui, |ui| {
            card_header(ui, icons::PULSE, "Haptic tones", "The controller's actuators can play a pitch");
            ui.horizontal(|ui| {
                ui.label("Actuator");
                for v in 0..voices as usize {
                    ui.selectable_value(voice, v, format!("{}", v + 1));
                }
            });
            ui.horizontal(|ui| {
                ui.label("Pitch");
                let mut f = *hz as f32;
                if ui.add(egui::Slider::new(&mut f, 40.0..=1500.0).logarithmic(true).suffix(" Hz")).changed() {
                    *hz = f as u16;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Strength");
                let mut v = *volume as f32;
                if ui.add(egui::Slider::new(&mut v, 0.0..=100.0).step_by(1.0)).changed() {
                    *volume = v as u8;
                }
            });
            ui.add_space(6.0);
            play = theme::primary_button(ui, true, format!("{}  Play for 1 s", icons::PLAY)).clicked();
        });
        if play {
            let (voice, hz, volume) = (*voice, *hz, *volume);
            self.manager.send(Cmd::Tone { id: d.info.id.clone(), voice, hz, ms: 1000, volume });
        }
    }
}

/// Whether the device has a speaker or tone actuators (used to place the cards).
pub(super) fn has_speaker(d: &DeviceView) -> bool {
    d.caps.contains(&Capability::Speaker)
}
