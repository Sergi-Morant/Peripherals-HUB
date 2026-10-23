//! Device model shared by every driver. The UI and CLI only talk to these traits, and render
//! controls from [`Capability`] instead of knowing about specific devices.

pub use hidapi::HidApi;

use std::sync::{Mutex, OnceLock};

/// The process-wide `HidApi`: hidapi allows only one instance, and the hub scanner and the
/// keyboard worker both need it. Lock it only for the duration of a refresh/open.
pub fn shared_api() -> Result<&'static Mutex<HidApi>> {
    static API: OnceLock<Mutex<HidApi>> = OnceLock::new();
    if let Some(a) = API.get() {
        return Ok(a);
    }
    let api = HidApi::new()?;
    Ok(API.get_or_init(|| Mutex::new(api)))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HID error: {0}")]
    Hid(#[from] hidapi::HidError),
    #[error("device does not support {0}")]
    Unsupported(&'static str),
    #[error("{0}")]
    Protocol(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse `RRGGBB` or `#RRGGBB`.
    pub fn from_hex(s: &str) -> Option<Rgb> {
        let s = s.trim_start_matches('#');
        if s.len() != 6 {
            return None;
        }
        let v = u32::from_str_radix(s, 16).ok()?;
        Some(Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// 0..=100, `None` when the device only reports charging state.
    pub percent: Option<u8>,
    pub charging: bool,
    /// The device reports only a few levels (Xbox: low/medium/high/full); `percent` is then a
    /// representative value for the level, not a measurement.
    pub coarse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keyboard,
    Mouse,
    Gamepad,
    Adapter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Bluetooth,
    /// Proprietary 2.4 GHz link through a dongle/receiver (Lightspeed, Xbox adapter, ...).
    Dongle,
}

impl Transport {
    pub fn label(self) -> &'static str {
        match self {
            Transport::Usb => "USB",
            Transport::Bluetooth => "Bluetooth",
            Transport::Dongle => "2.4 GHz",
        }
    }
}

/// How a HID interface is attached. Linux and macOS report the bus; on Windows older hidapi
/// builds report `Unknown`, where Bluetooth HID devices show the HID-over-Bluetooth service UUID
/// (`{00001124-...}`) or `BTHENUM`/`BTHLE` in their path.
pub fn transport_of(d: &hidapi::DeviceInfo) -> Transport {
    match d.bus_type() {
        hidapi::BusType::Bluetooth => Transport::Bluetooth,
        hidapi::BusType::Usb => Transport::Usb,
        _ => {
            let p = d.path().to_string_lossy().to_uppercase();
            if p.contains("00001124") || p.contains("BTHENUM") || p.contains("BTHLE") {
                Transport::Bluetooth
            } else {
                Transport::Usb
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Info {
    /// Stable within a scan, e.g. `ds4:<hid path>`.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub transport: Transport,
    /// Unique per physical device when the hardware has one (Bluetooth address, USB serial).
    /// Used to tell identical controllers apart and keep their order stable across replugs.
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    Off,
    Static,
    Breathe,
    Cycle,
}

impl Effect {
    pub fn label(self) -> &'static str {
        match self {
            Effect::Off => "Off",
            Effect::Static => "Static",
            Effect::Breathe => "Breathe",
            Effect::Cycle => "Colour cycle",
        }
    }

    pub fn uses_color(self) -> bool {
        matches!(self, Effect::Static | Effect::Breathe)
    }

    pub fn uses_period(self) -> bool {
        matches!(self, Effect::Breathe | Effect::Cycle)
    }
}

/// A lighting effect and its parameters. Fields an effect does not use are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectSetting {
    pub effect: Effect,
    pub color: Rgb,
    /// Length of one breathe/cycle, in milliseconds.
    pub period_ms: u16,
    /// Brightness 1..=100 (breathe and cycle only).
    pub brightness: u8,
}

impl Default for EffectSetting {
    fn default() -> Self {
        EffectSetting { effect: Effect::Static, color: Rgb(255, 255, 255), period_ms: 4000, brightness: 100 }
    }
}

/// A vibration motor. Strong and Weak are the two main motors in the grips; the triggers have their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motor {
    Strong,
    Weak,
    TriggerLeft,
    TriggerRight,
}

impl Motor {
    pub fn label(self) -> &'static str {
        match self {
            Motor::Strong => "Left grip (strong)",
            Motor::Weak => "Right grip (weak)",
            Motor::TriggerLeft => "Left trigger",
            Motor::TriggerRight => "Right trigger",
        }
    }
}

/// What a mouse button does, as stored in an onboard profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonAction {
    /// Sends nothing.
    Disabled,
    /// A mouse button, as a bit mask: 1 left, 2 right, 4 middle, 8 back, 0x10 forward, 0x20 button 6,
    /// 0x40/0x80 scroll left/right, 0x100.. buttons 9 and up.
    Mouse(u16),
    /// A keyboard key (USB HID usage id) with modifier bits (1 ctrl, 2 shift, 4 alt, 8 meta).
    Key { modifiers: u8, usage: u8 },
    /// A media key (USB HID consumer usage).
    Consumer(u16),
    /// A built-in function (DPI up/down/cycle/shift, profile switch...), see [`function_name`].
    Function { id: u8, data: u8 },
    /// Starts or stops a stored macro (the macro itself is not editable here).
    Macro { sector: u16, address: u16, stop: bool },
    /// Anything this crate does not understand; kept so it can be written back unchanged.
    Raw([u8; 4]),
}

pub fn function_name(id: u8) -> &'static str {
    match id {
        0x00 => "none",
        0x01 => "tilt left",
        0x02 => "tilt right",
        0x03 => "next DPI",
        0x04 => "previous DPI",
        0x05 => "cycle DPI",
        0x06 => "default DPI",
        0x07 => "DPI shift",
        0x08 => "next profile",
        0x09 => "previous profile",
        0x0A => "cycle profile",
        0x0B => "G-shift",
        0x0C => "battery status",
        0x0D => "select profile",
        0x0E => "switch mode",
        0x0F => "host button",
        0x10 => "scroll down",
        0x11 => "scroll up",
        _ => "unknown function",
    }
}

pub fn mouse_button_name(mask: u16) -> String {
    match mask {
        0x0001 => "left click".into(),
        0x0002 => "right click".into(),
        0x0004 => "middle click".into(),
        0x0008 => "back".into(),
        0x0010 => "forward".into(),
        0x0040 => "scroll left".into(),
        0x0080 => "scroll right".into(),
        m if m.count_ones() == 1 => format!("button {}", m.trailing_zeros() + 1),
        m => format!("buttons {m:#06x}"),
    }
}

/// Name of a USB HID keyboard usage (letters, digits, F-keys, navigation keys), if it is one of those.
pub fn key_name(usage: u8) -> Option<String> {
    Some(match usage {
        0x04..=0x1D => ((b'A' + (usage - 0x04)) as char).to_string(),
        0x1E..=0x26 => ((b'1' + (usage - 0x1E)) as char).to_string(),
        0x27 => "0".into(),
        0x28 => "Enter".into(),
        0x29 => "Esc".into(),
        0x2A => "Backspace".into(),
        0x2B => "Tab".into(),
        0x2C => "Space".into(),
        0x3A..=0x45 => format!("F{}", usage - 0x3A + 1),
        0x49 => "Insert".into(),
        0x4A => "Home".into(),
        0x4B => "Page up".into(),
        0x4C => "Delete".into(),
        0x4D => "End".into(),
        0x4E => "Page down".into(),
        0x4F => "Right".into(),
        0x50 => "Left".into(),
        0x51 => "Down".into(),
        0x52 => "Up".into(),
        _ => return None,
    })
}

/// Name of a USB HID consumer (media) usage, if known.
pub fn media_key_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0x00CD => "play / pause",
        0x00B5 => "next track",
        0x00B6 => "previous track",
        0x00B7 => "stop",
        0x00E2 => "mute",
        0x00E9 => "volume up",
        0x00EA => "volume down",
        _ => return None,
    })
}

impl ButtonAction {
    pub fn label(&self) -> String {
        match *self {
            ButtonAction::Disabled => "disabled".into(),
            ButtonAction::Mouse(m) => mouse_button_name(m),
            ButtonAction::Key { modifiers, usage } => {
                let mods: String = [(1, "Ctrl+"), (2, "Shift+"), (4, "Alt+"), (8, "Meta+")]
                    .iter()
                    .filter(|(bit, _)| modifiers & bit != 0)
                    .map(|(_, n)| *n)
                    .collect();
                format!("key {mods}{}", key_name(usage).unwrap_or_else(|| format!("{usage:#04x}")))
            }
            ButtonAction::Consumer(c) => media_key_name(c).map_or(format!("media key {c:#06x}"), |n| n.to_string()),
            ButtonAction::Function { id, data } if data != 0 => format!("{} ({data})", function_name(id)),
            ButtonAction::Function { id, .. } => function_name(id).into(),
            ButtonAction::Macro { stop: false, .. } => "run a macro".into(),
            ButtonAction::Macro { stop: true, .. } => "stop a macro".into(),
            ButtonAction::Raw(b) => format!("unknown {:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3]),
        }
    }
}

/// One onboard profile of a mouse: what it stores in its own memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// 1-based position in the profile list.
    pub index: usize,
    pub enabled: bool,
    pub name: String,
    /// Polling interval in milliseconds (1 = 1000 Hz).
    pub report_interval_ms: u8,
    /// The five DPI steps (0 = unused).
    pub dpi_levels: Vec<u16>,
    pub default_dpi_index: u8,
    pub shift_dpi_index: u8,
    pub color: Rgb,
    pub buttons: Vec<ButtonAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    Battery,
    /// Named lighting zones the device accepts a static colour for.
    Rgb { zones: Vec<String> },
    /// Lighting effects the device can run on every zone.
    Effects(Vec<Effect>),
    /// Sensor resolutions the device accepts, ascending.
    Dpi(Vec<u16>),
    /// Polling rates in Hz the device accepts, ascending.
    ReportRate(Vec<u16>),
    /// Vibration motors, each driven with a level of 0..=100.
    Rumble(Vec<Motor>),
    /// Actuators that can play a pitch (this many voices), as the Steam Controller's haptics can.
    Tones(u8),
    /// Remappable buttons and editable onboard profiles: this many profiles and buttons, and the
    /// names printed on the buttons when they are known (same order as the profile's buttons).
    Profiles { profiles: u8, buttons: u8, labels: Vec<String> },
    /// A built-in speaker that plays audio streamed to it.
    Speaker,
    /// The device can run its own stored profile (onboard) or take settings from the host.
    HostMode,
}

/// Current values of the adjustable settings; `None` when unknown or unsupported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    pub dpi: Option<u16>,
    pub report_rate: Option<u16>,
    /// True when the device takes DPI/rate from the host instead of its onboard profile.
    pub host_mode: Option<bool>,
}

pub trait Peripheral: Send {
    fn info(&self) -> &Info;
    /// Called every couple of seconds so streaming devices can drain pending reports and keep
    /// their cached state (battery, connection) fresh without blocking.
    fn tick(&mut self) {}
    /// Why the device is listed but cannot be controlled (e.g. missing permissions), if known.
    fn problem(&self) -> Option<String> {
        None
    }
    fn capabilities(&self) -> Vec<Capability>;
    fn battery(&mut self) -> Result<Option<Battery>> {
        Err(Error::Unsupported("battery"))
    }
    /// Set a static colour on `zone` (an index into the zones of [`Capability::Rgb`]).
    fn set_rgb(&mut self, _zone: usize, _color: Rgb) -> Result<()> {
        Err(Error::Unsupported("rgb"))
    }
    fn set_effect(&mut self, _zone: usize, _setting: EffectSetting) -> Result<()> {
        Err(Error::Unsupported("effects"))
    }
    /// Read the adjustable settings back from the device.
    fn settings(&mut self) -> Settings {
        Settings::default()
    }
    /// Set motor levels (0..=100); motors not listed are left alone. The vibration continues until
    /// the levels are set back to zero.
    fn rumble(&mut self, _levels: &[(Motor, u8)]) -> Result<()> {
        Err(Error::Unsupported("rumble"))
    }
    /// Play a tone on one voice (actuator) for `duration_ms`; `volume` is 0..=100. The device stops
    /// by itself when the duration is over.
    fn tone(&mut self, _voice: usize, _hz: u16, _duration_ms: u16, _volume: u8) -> Result<()> {
        Err(Error::Unsupported("tones"))
    }
    /// Play interleaved 16-bit PCM (`channels` 1 or 2, sample rate `rate`) on the built-in speaker
    /// and return when it has been sent; `volume` is 0..=100. `stop` is polled while playing and
    /// ends the playback early when it returns true.
    fn play_audio(
        &mut self,
        _samples: &[i16],
        _rate: u32,
        _channels: u16,
        _volume: u8,
        _stop: &dyn Fn() -> bool,
    ) -> Result<()> {
        Err(Error::Unsupported("speaker"))
    }
    /// Read the onboard profiles (read-only).
    fn profiles(&mut self) -> Result<Vec<Profile>> {
        Err(Error::Unsupported("profiles"))
    }
    /// Index (0-based) of the onboard profile the mouse is running, if known.
    fn active_profile(&mut self) -> Result<Option<usize>> {
        Err(Error::Unsupported("profiles"))
    }
    /// Run onboard profile `profile` (0-based); also switches the mouse to onboard mode.
    fn set_active_profile(&mut self, _profile: usize) -> Result<()> {
        Err(Error::Unsupported("profiles"))
    }
    /// Change what one button of one profile (`profile` and `button` are 0-based) does. Everything
    /// else in the profile is written back byte for byte.
    fn set_button(&mut self, _profile: usize, _button: usize, _action: ButtonAction) -> Result<()> {
        Err(Error::Unsupported("button remapping"))
    }
    /// A text backup of the whole onboard memory that [`Peripheral::import_profiles`] accepts.
    fn export_profiles(&mut self) -> Result<String> {
        Err(Error::Unsupported("profile backup"))
    }
    /// Write a backup made by [`Peripheral::export_profiles`] back to the device.
    fn import_profiles(&mut self, _backup: &str) -> Result<()> {
        Err(Error::Unsupported("profile backup"))
    }
    fn set_dpi(&mut self, _dpi: u16) -> Result<()> {
        Err(Error::Unsupported("dpi"))
    }
    fn set_report_rate(&mut self, _hz: u16) -> Result<()> {
        Err(Error::Unsupported("report rate"))
    }
    fn set_host_mode(&mut self, _host: bool) -> Result<()> {
        Err(Error::Unsupported("host mode"))
    }
}

pub trait Driver {
    fn name(&self) -> &'static str;
    /// Open every device of this kind that is currently connected.
    fn discover(&self, api: &HidApi) -> Vec<Box<dyn Peripheral>>;
}

/// All drivers, scanned together against one `HidApi`.
#[derive(Default)]
pub struct Hub {
    drivers: Vec<Box<dyn Driver>>,
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    pub fn register(&mut self, d: Box<dyn Driver>) {
        self.drivers.push(d);
    }

    /// Refresh the HID device list and open everything that is connected.
    pub fn scan(&self, api: &mut HidApi) -> Result<Vec<Box<dyn Peripheral>>> {
        api.refresh_devices()?;
        Ok(self.drivers.iter().flat_map(|d| d.discover(api)).collect())
    }
}

/// Battery levels the operating system's Bluetooth stack reports for connected devices through
/// the standard GATT Battery Service. Xbox controllers and many keyboards expose their battery
/// only this way, not in their HID reports.
pub mod bluetooth {
    /// Battery percentage of the connected device with this Bluetooth address, if known.
    /// Linux (BlueZ, via `bluetoothctl`) only; `None` elsewhere or when the device has no battery service.
    pub fn battery_percent(mac: &str) -> Option<u8> {
        #[cfg(target_os = "linux")]
        {
            // The address is passed to a program: accept nothing but a plain MAC.
            let ok = mac.len() == 17
                && mac.split(':').count() == 6
                && mac.chars().all(|c| c.is_ascii_hexdigit() || c == ':');
            if !ok {
                return None;
            }
            let out = linux::run_with_timeout("bluetoothctl", &["info", mac], std::time::Duration::from_secs(3))?;
            parse_battery(&out)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = mac;
            None
        }
    }

    /// Reads `Battery Percentage: 0x4b (75)` from `bluetoothctl info` output.
    pub fn parse_battery(info: &str) -> Option<u8> {
        let line = info.lines().find(|l| l.trim_start().starts_with("Battery Percentage:"))?;
        let open = line.rfind('(')?;
        let close = line.rfind(')')?;
        line.get(open + 1..close)?.trim().parse::<u8>().ok().map(|p| p.min(100))
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use std::io::Read;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        /// Run a program and return its stdout, giving up (and killing it) after `limit`.
        pub fn run_with_timeout(program: &str, args: &[&str], limit: Duration) -> Option<String> {
            let mut child =
                Command::new(program).args(args).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
            let end = Instant::now() + limit;
            loop {
                match child.try_wait().ok()? {
                    Some(_) => break,
                    None if Instant::now() >= end => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return None;
                    }
                    None => std::thread::sleep(Duration::from_millis(20)),
                }
            }
            let mut out = String::new();
            child.stdout.take()?.read_to_string(&mut out).ok()?;
            Some(out)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::parse_battery;

        #[test]
    fn key_and_media_labels() {
        assert_eq!(crate::ButtonAction::Key { modifiers: 1, usage: 0x04 }.label(), "key Ctrl+A");
        assert_eq!(crate::ButtonAction::Key { modifiers: 0, usage: 0x3A }.label(), "key F1");
        assert_eq!(crate::ButtonAction::Consumer(0xCD).label(), "play / pause");
    }

    #[test]
        fn parses_bluetoothctl_output() {
            let info = "Device AA:BB (public)\n\tConnected: yes\n\tBattery Percentage: 0x4b (75)\n";
            assert_eq!(parse_battery(info), Some(75));
            assert_eq!(parse_battery("Device AA:BB\n\tConnected: yes\n"), None);
        }
    }
}
