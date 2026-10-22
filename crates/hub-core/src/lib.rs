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
        fn parses_bluetoothctl_output() {
            let info = "Device AA:BB (public)\n\tConnected: yes\n\tBattery Percentage: 0x4b (75)\n";
            assert_eq!(parse_battery(info), Some(75));
            assert_eq!(parse_battery("Device AA:BB\n\tConnected: yes\n"), None);
        }
    }
}
