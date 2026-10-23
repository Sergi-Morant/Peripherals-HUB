//! Play/pause, stop and a seek bar, shared by every card that plays something on a device.

use eframe::egui::{self, RichText};

use super::Hub;
use crate::icons;
use crate::manager::Cmd;
use crate::theme::{self};

/// A playback the UI started on a device.
pub struct Player {
    /// Which card started it ("music" or "speaker"); only that card shows the controls.
    pub owner: &'static str,
    pub total_ms: u64,
    /// Playing through the device's speaker (position read from the audio control) rather than as
    /// vibration (position reported by the device thread).
    pub audio: bool,
    pub paused: bool,
    pub pos_ms: u64,
}

pub enum PlayerAction {
    Play,
    Pause,
    Resume,
    Stop,
    Seek(u64),
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

impl Hub {
    /// Current position of a device's playback.
    fn position_of(&self, p: &Player) -> u64 {
        if p.audio {
            self.manager.audio().position().min(p.total_ms)
        } else {
            p.pos_ms.min(p.total_ms)
        }
    }

    /// The controls for a card. `can_play` says whether a Play press would have something to play.
    pub(super) fn player_bar(&mut self, ui: &mut egui::Ui, id: &str, owner: &'static str, can_play: bool) -> Option<PlayerAction> {
        let mine = self.players.get(id).filter(|p| p.owner == owner);
        let busy_elsewhere = self.players.get(id).is_some_and(|p| p.owner != owner);
        let view = mine.map(|p| (p.paused, self.position_of(p), p.total_ms));
        let mut action = None;
        ui.horizontal(|ui| {
            match view {
                None => {
                    if theme::primary_button(ui, can_play && !busy_elsewhere, format!("{}  Play", icons::PLAY)).clicked() {
                        action = Some(PlayerAction::Play);
                    }
                }
                Some((true, ..)) => {
                    if theme::primary_button(ui, true, format!("{}  Resume", icons::PLAY)).clicked() {
                        action = Some(PlayerAction::Resume);
                    }
                }
                Some((false, ..)) => {
                    if ui.button(format!("{}  Pause", icons::PAUSE)).clicked() {
                        action = Some(PlayerAction::Pause);
                    }
                }
            }
            if view.is_some() && ui.button("Stop").clicked() {
                action = Some(PlayerAction::Stop);
            }
            let (pos, total) = view.map_or((0, 0), |(_, p, t)| (p, t));
            let mut value = pos as f32;
            ui.spacing_mut().slider_width = (ui.available_width() - 120.0).max(120.0);
            let r = ui.add_enabled(
                view.is_some(),
                egui::Slider::new(&mut value, 0.0..=total.max(1) as f32).show_value(false),
            );
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                action = Some(PlayerAction::Seek(value as u64));
            }
            ui.label(RichText::new(format!("{} / {}", clock(if r.dragged() { value as u64 } else { pos }), clock(total))).color(theme::TEXT_MUTED));
        });
        action
    }

    /// Start tracking a playback the card has just sent to the device.
    pub(super) fn start_player(&mut self, id: &str, owner: &'static str, total_ms: u64, audio: bool) {
        self.players.insert(id.to_string(), Player { owner, total_ms, audio, paused: false, pos_ms: 0 });
    }

    /// Pause, resume, stop or seek the playback on `id`.
    pub(super) fn player_do(&mut self, id: &str, action: PlayerAction) {
        let Some(p) = self.players.get_mut(id) else { return };
        match action {
            PlayerAction::Pause | PlayerAction::Resume => {
                p.paused = matches!(action, PlayerAction::Pause);
                if p.audio {
                    self.manager.audio().set_paused(p.paused);
                } else {
                    self.manager.send(Cmd::Pause { id: id.to_string(), paused: p.paused });
                }
            }
            PlayerAction::Seek(ms) => {
                p.pos_ms = ms;
                if p.audio {
                    self.manager.audio().request_seek(ms);
                } else {
                    self.manager.send(Cmd::Seek { id: id.to_string(), ms });
                }
            }
            PlayerAction::Stop => {
                if p.audio {
                    self.manager.audio().request_stop();
                }
                self.manager.send(Cmd::StopPlay { id: id.to_string() });
            }
            PlayerAction::Play => {}
        }
    }
}
