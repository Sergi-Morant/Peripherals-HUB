//! Sound helpers: loading MIDI/WAV files for the music card, and the Steam Controller's tone card.

use eframe::egui;

use super::Hub;
use crate::manager::{Audio, Cmd, DeviceView};
use crate::icons;
use crate::theme::{self, card, card_header};

/// A file the music card can play.
pub(super) enum Source {
    Midi(Vec<hub_midi::Note>),
    Wav(Audio),
}

/// Load a `.mid` or `.wav` file.
pub(super) fn load_source(path: &std::path::Path) -> Result<Source, String> {
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
        Ok(Source::Wav(Audio { samples, rate: spec.sample_rate, channels: spec.channels }))
    } else {
        Ok(Source::Midi(hub_midi::parse(&std::fs::read(path).map_err(|e| e.to_string())?)?))
    }
}

pub(super) fn seconds(a: &Audio) -> usize {
    seconds_ms(a) as usize / 1000
}

pub(super) fn seconds_ms(a: &Audio) -> u64 {
    a.samples.len() as u64 * 1000 / a.channels.max(1) as u64 / a.rate.max(1) as u64
}

impl Hub {
    /// Try single tones on the Steam Controller's haptic actuators.
    pub(super) fn tones_card(&mut self, ui: &mut egui::Ui, d: &DeviceView, voices: u8) {
        let (voice, hz, volume) = self.tone.entry(d.info.id.clone()).or_insert((0, 440, 75));
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
