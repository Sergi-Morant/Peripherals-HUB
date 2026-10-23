//! Background thread that owns every non-keyboard device: it watches for devices appearing and
//! disappearing, polls batteries and applies colour changes, so the UI never blocks on USB.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use hub_core::{Battery, ButtonAction, Capability, EffectSetting, Hub, Info, Motor, Peripheral, Profile, Rgb, Settings};

/// Vendors the hub watches; a change in their HID nodes triggers a rescan.
const WATCHED_VIDS: [u16; 6] = [s85_protocol::VID, 0x046D, 0x054C, 0x045E, 0x057E, 0x28DE];
const LOGITECH_VID: u16 = 0x046D;
const TICK: Duration = Duration::from_secs(2);
const BATTERY_EVERY: Duration = Duration::from_secs(30);
/// A receiver with no mouse behind it is probed again this often (the mouse may just be asleep).
const REPROBE_EVERY: Duration = Duration::from_secs(10);

/// Sound to play on a device's speaker: interleaved 16-bit PCM.
pub struct Audio {
    pub samples: Vec<i16>,
    pub rate: u32,
    pub channels: u16,
}

#[derive(Clone)]
pub struct DeviceView {
    pub info: Info,
    pub caps: Vec<Capability>,
    pub battery: Option<Battery>,
    pub settings: Settings,
    pub problem: Option<String>,
}

pub enum Cmd {
    SetRgb { id: String, zone: usize, color: Rgb },
    SetEffect { id: String, zone: usize, setting: EffectSetting },
    SetDpi { id: String, dpi: u16 },
    SetReportRate { id: String, hz: u16 },
    SetHostMode { id: String, host: bool },
    /// Vibrate at these levels; `for_ms` stops it automatically afterwards.
    Rumble { id: String, levels: Vec<(Motor, u8)>, for_ms: Option<u64> },
    /// Play a planned MIDI schedule on the device (replaces any playback already running on it).
    Play { id: String, events: Vec<hub_midi::Event> },
    StopPlay { id: String },
    /// Stream sound to the device's speaker. Blocks the device thread until it is over or stopped
    /// through [`Manager::stop_audio`].
    PlayAudio { id: String, audio: Arc<Audio>, volume: u8 },
    Tone { id: String, voice: usize, hz: u16, ms: u16, volume: u8 },
    /// Read the onboard profiles and which one runs; answered with [`Update::Profiles`].
    LoadProfiles { id: String },
    SetActiveProfile { id: String, index: usize },
    SetButton { id: String, profile: usize, button: usize, action: ButtonAction },
    ExportProfiles { id: String, path: PathBuf },
    ImportProfiles { id: String, path: PathBuf },
}

impl Cmd {
    /// Commands with the same key replace each other while queued (dragging a slider floods the queue).
    fn key(&self) -> (u8, &str, usize) {
        match self {
            Cmd::SetRgb { id, zone, .. } => (0, id, *zone),
            Cmd::SetEffect { id, zone, .. } => (1, id, *zone),
            Cmd::SetDpi { id, .. } => (2, id, 0),
            Cmd::SetReportRate { id, .. } => (3, id, 0),
            Cmd::SetHostMode { id, .. } => (4, id, 0),
            Cmd::Rumble { id, .. } => (5, id, 0),
            Cmd::Play { id, .. } => (6, id, 0),
            Cmd::StopPlay { id } => (7, id, 0),
            Cmd::PlayAudio { id, .. } => (8, id, 0),
            Cmd::Tone { id, voice, .. } => (9, id, *voice),
            Cmd::LoadProfiles { id } => (10, id, 0),
            Cmd::SetActiveProfile { id, .. } => (11, id, 0),
            Cmd::SetButton { id, profile, button, .. } => (12, id, profile * 64 + button),
            Cmd::ExportProfiles { id, .. } => (13, id, 0),
            Cmd::ImportProfiles { id, .. } => (14, id, 0),
        }
    }

    fn id(&self) -> &str {
        self.key().1
    }
}

pub enum Update {
    Devices(Vec<DeviceView>),
    /// Whether the RK S85 is plugged in (handled by the keyboard worker, not here).
    Keyboard(bool),
    Error(String),
    /// A MIDI playback finished or was stopped.
    PlaybackEnded(String),
    /// The onboard profiles of a device and the index of the running one.
    Profiles { id: String, profiles: Vec<Profile>, active: Option<usize> },
    /// A message that is not an error (a backup was saved...).
    Notice(String),
}

pub struct Manager {
    tx: Sender<Cmd>,
    pub rx: Receiver<Update>,
    /// Set to end a running [`Cmd::PlayAudio`]; the device thread is busy with it and cannot read
    /// the command queue meanwhile.
    stop_audio: Arc<AtomicBool>,
}

impl Manager {
    pub fn spawn(ctx: eframe::egui::Context) -> Manager {
        let (tx, cmd_rx) = channel();
        let (up_tx, rx) = channel();
        let stop_audio = Arc::new(AtomicBool::new(false));
        let stop = stop_audio.clone();
        thread::spawn(move || run(cmd_rx, up_tx, ctx, stop));
        Manager { tx, rx, stop_audio }
    }

    pub fn stop_audio(&self) {
        self.stop_audio.store(true, Ordering::Relaxed);
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

struct State {
    hub: Hub,
    devices: Vec<Box<dyn Peripheral>>,
    batteries: HashMap<String, Battery>,
    settings: HashMap<String, Settings>,
    fingerprint: Vec<String>,
    keyboard: bool,
    scanned: Instant,
    polled: Instant,
}

fn views(s: &State) -> Vec<DeviceView> {
    s.devices
        .iter()
        .map(|d| DeviceView {
            info: d.info().clone(),
            caps: d.capabilities(),
            battery: s.batteries.get(&d.info().id).copied(),
            settings: s.settings.get(&d.info().id).copied().unwrap_or_default(),
            problem: d.problem(),
        })
        .collect()
}

/// Cheap list of the HID nodes of watched vendors, plus whether the keyboard is among them.
fn fingerprint() -> Option<(Vec<String>, bool, bool)> {
    let api = hub_core::shared_api().ok()?;
    let mut api = api.lock().unwrap_or_else(|e| e.into_inner());
    api.refresh_devices().ok()?;
    let mut keyboard = false;
    let mut receiver = false;
    let mut fp: Vec<String> = api
        .device_list()
        .filter(|d| WATCHED_VIDS.contains(&d.vendor_id()))
        .map(|d| {
            keyboard |= d.vendor_id() == s85_protocol::VID
                && s85_protocol::PIDS.iter().any(|(p, _)| *p == d.product_id());
            receiver |= d.vendor_id() == LOGITECH_VID;
            format!("{:04x}:{:04x}:{}", d.vendor_id(), d.product_id(), d.path().to_string_lossy())
        })
        .collect();
    fp.sort();
    Some((fp, keyboard, receiver))
}

fn rescan(s: &mut State) -> Result<(), String> {
    // Close old handles first so reopening does not fight over the same node.
    s.devices.clear();
    let api = hub_core::shared_api().map_err(|e| e.to_string())?;
    let mut api = api.lock().unwrap_or_else(|e| e.into_inner());
    s.devices = s.hub.scan(&mut api).map_err(|e| e.to_string())?;
    s.scanned = Instant::now();
    s.settings = s.devices.iter_mut().map(|d| (d.info().id.clone(), d.settings())).collect();
    poll_batteries(s);
    Ok(())
}

/// Refresh batteries; true if some device that should report one failed (it probably went away).
fn poll_batteries(s: &mut State) -> bool {
    s.batteries.clear();
    let mut failed = false;
    for d in s.devices.iter_mut() {
        if d.capabilities().contains(&Capability::Battery) {
            match d.battery() {
                Ok(Some(b)) => {
                    s.batteries.insert(d.info().id.clone(), b);
                }
                Ok(None) => {}
                Err(_) => failed = true,
            }
        }
    }
    s.polled = Instant::now();
    failed
}

fn run(cmds: Receiver<Cmd>, updates: Sender<Update>, ctx: eframe::egui::Context, stop_audio: Arc<AtomicBool>) {
    let emit = |u: Update| {
        let _ = updates.send(u);
        ctx.request_repaint();
    };
    let mut hub = Hub::new();
    hub.register(Box::new(drv_ds4::Ds4Driver));
    hub.register(Box::new(drv_g502x::G502xDriver));
    hub.register(Box::new(drv_gamepads::GamepadDriver));
    hub.register(Box::new(drv_steam::SteamDriver));
    hub.register(Box::new(drv_xbox::XboxDriver));
    hub.register(Box::new(crate::keyboard_link::KeyboardLinkDriver));
    let long_ago = Instant::now() - Duration::from_secs(3600);
    let mut s = State {
        hub,
        devices: vec![],
        batteries: HashMap::new(),
        settings: HashMap::new(),
        fingerprint: vec!["<unscanned>".into()],
        keyboard: false,
        scanned: long_ago,
        polled: long_ago,
    };
    // Rumble that must be switched off at a given time.
    let mut stops: Vec<(Instant, String)> = Vec::new();
    // MIDI playbacks in progress, per device id.
    let mut playing: HashMap<String, Playback> = HashMap::new();
    let mut checked = long_ago;
    loop {
        let now = Instant::now();
        let mut wait = if playing.is_empty() { TICK.saturating_sub(checked.elapsed()) } else { TICK };
        for (t, _) in &stops {
            wait = wait.min(t.saturating_duration_since(now));
        }
        for p in playing.values() {
            wait = wait.min(p.next_due().saturating_duration_since(now));
        }
        // Collapse queued changes (dragging a slider floods the queue): the last one per control wins.
        let mut pending: Vec<Cmd> = Vec::new();
        match cmds.recv_timeout(wait) {
            Ok(c) => {
                pending.push(c);
                pending.extend(cmds.try_iter());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let mut latest: Vec<Cmd> = Vec::new();
        for c in pending {
            latest.retain(|o| o.key() != c.key());
            latest.push(c);
        }
        let touched = !latest.is_empty();
        for cmd in latest {
            let id = cmd.id().to_string();
            let Some(d) = s.devices.iter_mut().find(|d| d.info().id == id) else { continue };
            let result = match cmd {
                Cmd::SetRgb { zone, color, .. } => d.set_rgb(zone, color),
                Cmd::SetEffect { zone, setting, .. } => d.set_effect(zone, setting),
                Cmd::SetDpi { dpi, .. } => d.set_dpi(dpi),
                Cmd::SetReportRate { hz, .. } => d.set_report_rate(hz),
                Cmd::SetHostMode { host, .. } => d.set_host_mode(host),
                Cmd::Rumble { levels, for_ms, .. } => {
                    stops.retain(|(_, i)| *i != id);
                    if let Some(ms) = for_ms {
                        stops.push((Instant::now() + Duration::from_millis(ms), id.clone()));
                    }
                    d.rumble(&levels)
                }
                Cmd::Play { events, .. } => {
                    if playing.remove(&id).is_some() {
                        silence(d.as_mut());
                    }
                    playing.insert(id.clone(), Playback::new(events));
                    Ok(())
                }
                Cmd::StopPlay { .. } => {
                    if playing.remove(&id).is_some() {
                        silence(d.as_mut());
                        emit(Update::PlaybackEnded(id.clone()));
                    }
                    Ok(())
                }
                Cmd::PlayAudio { audio, volume, .. } => {
                    stop_audio.store(false, Ordering::Relaxed);
                    let r = d.play_audio(&audio.samples, audio.rate, audio.channels, volume, &|| {
                        stop_audio.load(Ordering::Relaxed)
                    });
                    emit(Update::PlaybackEnded(id.clone()));
                    r
                }
                Cmd::Tone { voice, hz, ms, volume, .. } => d.tone(voice, hz, ms, volume),
                Cmd::LoadProfiles { .. } => {
                    emit_profiles(d.as_mut(), &emit);
                    Ok(())
                }
                Cmd::SetActiveProfile { index, .. } => {
                    let r = d.set_active_profile(index);
                    emit_profiles(d.as_mut(), &emit);
                    r
                }
                Cmd::SetButton { profile, button, action, .. } => {
                    let r = d.set_button(profile, button, action);
                    emit_profiles(d.as_mut(), &emit);
                    r
                }
                Cmd::ExportProfiles { path, .. } => d
                    .export_profiles()
                    .and_then(|text| std::fs::write(&path, text).map_err(|e| hub_core::Error::Protocol(e.to_string())))
                    .map(|()| emit(Update::Notice(format!("Backup saved to {}", path.display())))),
                Cmd::ImportProfiles { path, .. } => {
                    let r = std::fs::read_to_string(&path)
                        .map_err(|e| hub_core::Error::Protocol(e.to_string()))
                        .and_then(|text| d.import_profiles(&text))
                        .map(|()| emit(Update::Notice("Backup restored".into())));
                    emit_profiles(d.as_mut(), &emit);
                    r
                }
            };
            if let Err(e) = result {
                emit(Update::Error(format!("{}: {e}", d.info().name)));
                if matches!(e, hub_core::Error::Hid(_)) {
                    s.scanned = long_ago; // the device may be gone: rescan below
                }
            }
            // Read the device back so the UI shows what it really accepted.
            let settings = d.settings();
            s.settings.insert(id, settings);
        }
        if touched {
            emit(Update::Devices(views(&s)));
        }

        // Switch off vibrations whose time is up.
        let now = Instant::now();
        let (due, later): (Vec<_>, Vec<_>) = stops.into_iter().partition(|(t, _)| *t <= now);
        stops = later;
        for (_, id) in due {
            if let Some(d) = s.devices.iter_mut().find(|d| d.info().id == id) {
                silence(d.as_mut());
            }
        }

        // Run due MIDI events.
        let mut finished = Vec::new();
        for (id, p) in playing.iter_mut() {
            let Some(d) = s.devices.iter_mut().find(|d| d.info().id == *id) else {
                finished.push(id.clone()); // device went away
                continue;
            };
            while let Some(action) = p.take_due() {
                let r = match action {
                    hub_midi::Action::Tone { voice, hz, duration_ms, volume } => d.tone(voice, hz, duration_ms, volume),
                    hub_midi::Action::Rumble(levels) => d.rumble(&levels),
                };
                if let Err(e) = r {
                    emit(Update::Error(format!("{}: {e}", d.info().name)));
                    finished.push(id.clone());
                    break;
                }
            }
            if p.done() {
                finished.push(id.clone());
            }
        }
        for id in finished {
            if playing.remove(&id).is_some() {
                emit(Update::PlaybackEnded(id));
            }
        }

        // Housekeeping: hotplug scan, battery polling. Skipped while music plays so scans (which
        // can take seconds) do not disturb the timing.
        if !playing.is_empty() || checked.elapsed() < TICK {
            continue;
        }
        checked = Instant::now();
        for d in s.devices.iter_mut() {
            d.tick();
        }
        let Some((fp, keyboard, receiver)) = fingerprint() else { continue };
        if keyboard != s.keyboard {
            s.keyboard = keyboard;
            emit(Update::Keyboard(keyboard));
        }
        let have_mouse = s.devices.iter().any(|d| d.info().kind == hub_core::Kind::Mouse);
        let changed = fp != s.fingerprint;
        let reprobe = receiver && !have_mouse && s.scanned.elapsed() > REPROBE_EVERY;
        if changed || reprobe || s.scanned.elapsed() > Duration::from_secs(3000) {
            s.fingerprint = fp;
            let before = views(&s).len();
            if let Err(e) = rescan(&mut s) {
                emit(Update::Error(e));
            }
            if changed || before != s.devices.len() || reprobe {
                emit(Update::Devices(views(&s)));
            }
        } else if s.polled.elapsed() > BATTERY_EVERY {
            if poll_batteries(&mut s) {
                s.scanned = long_ago; // a device stopped answering: rescan on the next tick
            }
            emit(Update::Devices(views(&s)));
        }
    }
}

/// Read the profiles and the running one from a device and tell the UI; a device without profiles
/// or one that does not answer simply sends nothing.
fn emit_profiles(d: &mut dyn Peripheral, emit: &dyn Fn(Update)) {
    if let Ok(profiles) = d.profiles() {
        let active = d.active_profile().ok().flatten();
        emit(Update::Profiles { id: d.info().id.clone(), profiles, active });
    }
}

/// A MIDI schedule being played: events are fired as their time comes.
struct Playback {
    events: Vec<hub_midi::Event>,
    next: usize,
    started: Instant,
}

impl Playback {
    fn new(events: Vec<hub_midi::Event>) -> Playback {
        Playback { events, next: 0, started: Instant::now() }
    }

    fn next_due(&self) -> Instant {
        self.events.get(self.next).map_or(Instant::now() + TICK, |e| self.started + Duration::from_millis(e.at_ms))
    }

    /// The next event whose time has come, if any.
    fn take_due(&mut self) -> Option<hub_midi::Action> {
        let e = self.events.get(self.next)?;
        if self.started + Duration::from_millis(e.at_ms) > Instant::now() {
            return None;
        }
        self.next += 1;
        Some(e.action.clone())
    }

    fn done(&self) -> bool {
        self.next >= self.events.len()
    }
}

/// Stop whatever the device is vibrating or playing.
fn silence(d: &mut dyn Peripheral) {
    let off: Vec<(Motor, u8)> = [Motor::Strong, Motor::Weak, Motor::TriggerLeft, Motor::TriggerRight]
        .into_iter()
        .map(|m| (m, 0))
        .collect();
    let _ = d.rumble(&off);
    // Tone voices stop by themselves; cut a long one short with a silent 1 ms tone on each voice.
    for voice in 0..2 {
        let _ = d.tone(voice, 100, 1, 0);
    }
}
