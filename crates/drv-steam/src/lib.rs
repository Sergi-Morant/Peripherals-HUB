//! Valve Steam Controller behind a Steam Controller Puck (2.4 GHz). Each Puck exposes up to four
//! controller slots as HID interfaces 2..=5; a connected controller streams reports on its slot
//! (layout from SDL's `SDL_hidapi_steam_triton.c`, confirmed against a real Puck):
//! `0x42`/`0x45`/`0x47` controller state, `0x43` battery `[charge state, level %, ...]`
//! (charge state 1 = discharging, 2 = charging, 4 = charged), `0x79`/`0x46` wireless status.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use hub_core::{Battery, Capability, Driver, Error, Info, Kind, Peripheral, Result, Transport};

pub const VID: u16 = 0x28DE;
/// Puck ("Proteus") and its sibling dongle ("Nereid"), see SDL's `usb_ids.h`.
pub const PUCK_PIDS: [u16; 2] = [0x1304, 0x1305];
pub const SLOT_INTERFACES: std::ops::RangeInclusive<i32> = 2..=5;

/// For each puck slot, listen for `listen` and describe which report ids arrived (development aid).
pub fn sniff(api: &HidApi, listen: Duration) -> Vec<String> {
    let mut seen: BTreeMap<String, i32> = BTreeMap::new();
    for d in api.device_list() {
        if d.vendor_id() == VID && PUCK_PIDS.contains(&d.product_id()) && SLOT_INTERFACES.contains(&d.interface_number()) {
            seen.insert(d.path().to_string_lossy().into_owned(), d.interface_number());
        }
    }
    seen.into_iter()
        .map(|(path, iface)| {
            let Some(dev) = api.device_list().find(|d| d.path().to_string_lossy() == path).and_then(|d| d.open_device(api).ok())
            else {
                return format!("interface {iface} ({path}): cannot open");
            };
            let mut ids: BTreeMap<u8, (u32, Vec<u8>)> = BTreeMap::new();
            let end = Instant::now() + listen;
            let mut buf = [0u8; 64];
            while Instant::now() < end {
                match dev.read_timeout(&mut buf, 100) {
                    Ok(n) if n > 0 => {
                        let e = ids.entry(buf[0]).or_insert((0, vec![]));
                        e.0 += 1;
                        e.1 = buf[..n.min(12)].to_vec();
                    }
                    Ok(_) => {}
                    Err(e) => return format!("interface {iface} ({path}): read error {e}"),
                }
            }
            let summary: Vec<String> = ids
                .iter()
                .map(|(id, (count, sample))| format!("id {id:#04x} x{count} e.g. {}", sample.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")))
                .collect();
            format!("interface {iface} ({path}): {}", if summary.is_empty() { "silent".into() } else { summary.join("; ") })
        })
        .collect()
}

const ID_STATE: [u8; 3] = [0x42, 0x45, 0x47];
const ID_BATTERY: u8 = 0x43;
/// A controller that streams nothing for this long has gone to sleep or left.
const STALE_AFTER: Duration = Duration::from_secs(10);

pub struct SteamController {
    hid: HidDevice,
    info: Info,
    battery: Option<Battery>,
    last_seen: Instant,
    /// The battery report is sent only now and then; wait for it once before giving up.
    waited_for_battery: bool,
}

impl SteamController {
    /// Drain whatever the controller has sent; true if any report arrived.
    fn drain(&mut self, wait_ms: i32) -> bool {
        let mut buf = [0u8; 64];
        let mut got = false;
        while let Ok(n) = self.hid.read_timeout(&mut buf, if got { 0 } else { wait_ms }) {
            if n == 0 {
                break;
            }
            got = true;
            self.last_seen = Instant::now();
            if buf[0] == ID_BATTERY && n >= 3 {
                self.battery = Some(Battery { percent: Some(buf[2].min(100)), charging: buf[1] == 2, coarse: false });
            }
        }
        got
    }
}

impl Peripheral for SteamController {
    fn info(&self) -> &Info {
        &self.info
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![Capability::Battery, Capability::Tones(2)]
    }

    fn tone(&mut self, voice: usize, hz: u16, duration_ms: u16, volume: u8) -> Result<()> {
        if voice > 1 {
            return Err(Error::Unsupported("voice"));
        }
        self.hid.write(&tone_report(voice as u8, hz, duration_ms, volume)).map_err(Error::from)?;
        Ok(())
    }

    fn tick(&mut self) {
        self.drain(0);
    }

    fn battery(&mut self) -> Result<Option<Battery>> {
        if self.last_seen.elapsed() > STALE_AFTER {
            return Err(Error::Protocol("controller is not sending (asleep or out of range)".into()));
        }
        if self.battery.is_none() && !self.waited_for_battery {
            self.waited_for_battery = true;
            let end = Instant::now() + Duration::from_secs(4);
            while self.battery.is_none() && Instant::now() < end {
                self.drain(200);
            }
        }
        Ok(self.battery)
    }
}

pub struct SteamDriver;

impl Driver for SteamDriver {
    fn name(&self) -> &'static str {
        "Steam Controller"
    }

    fn discover(&self, api: &HidApi) -> Vec<Box<dyn Peripheral>> {
        let mut slots: BTreeMap<String, (i32, Option<String>)> = BTreeMap::new();
        for d in api.device_list() {
            if d.vendor_id() == VID && PUCK_PIDS.contains(&d.product_id()) && SLOT_INTERFACES.contains(&d.interface_number()) {
                slots.insert(d.path().to_string_lossy().into_owned(), (d.interface_number(), d.serial_number().map(String::from)));
            }
        }
        let mut out: Vec<Box<dyn Peripheral>> = Vec::new();
        for (path, (iface, puck_serial)) in slots {
            let Some(hid) = api.device_list().find(|d| d.path().to_string_lossy() == path).and_then(|d| d.open_device(api).ok())
            else {
                continue;
            };
            let mut c = SteamController {
                hid,
                info: Info {
                    id: format!("steam:{path}"),
                    name: "Steam Controller".into(),
                    kind: Kind::Gamepad,
                    transport: Transport::Dongle,
                    serial: puck_serial.map(|s| format!("{s} slot {}", iface - 1)),
                },
                battery: None,
                last_seen: Instant::now(),
                waited_for_battery: false,
            };
            // An empty slot is silent; a connected controller streams state many times a second.
            let end = Instant::now() + Duration::from_millis(1500);
            let mut connected = false;
            while Instant::now() < end && !(connected && c.battery.is_some()) {
                let mut buf = [0u8; 64];
                match c.hid.read_timeout(&mut buf, 100) {
                    Ok(n) if n > 0 => {
                        connected |= ID_STATE.contains(&buf[0]) || buf[0] == ID_BATTERY;
                        if buf[0] == ID_BATTERY && n >= 3 {
                            c.battery = Some(Battery { percent: Some(buf[2].min(100)), charging: buf[1] == 2, coarse: false });
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            if connected {
                c.last_seen = Instant::now();
                out.push(Box::new(c));
            }
        }
        out
    }
}

/// Output report `0x83` (LFO tone, 10 bytes, little-endian): side, gain in dB, frequency in Hz,
/// duration in ms, LFO frequency, LFO depth. Layout from SDL's `controller_structs.h`.
fn tone_report(side: u8, hz: u16, duration_ms: u16, volume: u8) -> [u8; 10] {
    // Volume 100 = 0 dB (full), then 0.5 dB steps down to -40 dB at volume 0.
    let gain_db = -(((100 - volume.min(100)) as i16 * 40) / 100) as i8;
    let mut r = [0u8; 10];
    r[0] = 0x83;
    r[1] = side;
    r[2] = gain_db as u8;
    r[3..5].copy_from_slice(&hz.to_le_bytes());
    r[5..7].copy_from_slice(&duration_ms.to_le_bytes());
    // LFO off: bytes 7..10 stay zero.
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_report_layout() {
        let r = tone_report(1, 440, 500, 100);
        assert_eq!(r, [0x83, 1, 0, 0xB8, 0x01, 0xF4, 0x01, 0, 0, 0]);
        // Quieter volume gives a negative gain in dB.
        assert!((tone_report(0, 440, 500, 50)[2] as i8) < 0);
    }
}
