//! Microsoft Xbox One / Series controllers over Bluetooth: battery and rumble.
//!
//! The battery comes from the system's Bluetooth stack (GATT Battery Service): this pad's HID
//! descriptor has no battery report. Controllers that do send the HID battery report (older
//! firmware) are decoded as a fallback.
//!
//! Report layouts follow the xpadneo driver (`power.c`, `rumble.c`, `xpadneo.h`):
//! * input report `0x04`, two bytes: `[id, flags]`. Flags: bit 7 online, bits 2..3 power source
//!   (0 = USB cable, 1 = battery, 2 = play-and-charge), bit 4 charging, bits 0..1 level 0..=3.
//! * output report `0x03`, nine bytes: `[id, enable, left trigger, right trigger, strong, weak,
//!   sustain (10 ms), release (10 ms), loop count]`, motor levels 0..=100, enable bits
//!   weak 1 / strong 2 / right trigger 4 / left trigger 8.
//!
//! USB-connected controllers are handled by the operating system's own driver and expose no HID
//! node, so only Bluetooth is covered here.

use hidapi::{HidApi, HidDevice};
use hub_core::{
    transport_of, Battery, Capability, Driver, Error, Info, Kind, Motor, Peripheral, Result, Transport,
};

pub const VID: u16 = 0x045E;
const USAGE_PAGE_DESKTOP: u16 = 0x01;
const USAGE_GAMEPAD: u16 = 0x05;

const REPORT_BATTERY: u8 = 0x04;
const REPORT_RUMBLE: u8 = 0x03;
const MOTORS: [Motor; 4] = [Motor::Strong, Motor::Weak, Motor::TriggerLeft, Motor::TriggerRight];

pub struct Xbox {
    /// `None` when the node could not be opened; `problem` then says why.
    hid: Option<HidDevice>,
    problem: Option<String>,
    info: Info,
    battery: Option<Battery>,
    /// Levels last sent, in `MOTORS` order.
    levels: [u8; 4],
}

pub struct XboxDriver;

impl Driver for XboxDriver {
    fn name(&self) -> &'static str {
        "Xbox controllers"
    }

    fn discover(&self, api: &HidApi) -> Vec<Box<dyn Peripheral>> {
        api.device_list()
            .filter(|d| {
                d.vendor_id() == VID
                    && d.usage_page() == USAGE_PAGE_DESKTOP
                    && d.usage() == USAGE_GAMEPAD
                    && transport_of(d) == Transport::Bluetooth
            })
            .map(|d| {
                let (hid, problem) = match d.open_device(api) {
                    Ok(h) => (Some(h), None),
                    Err(e) => (None, Some(format!("Cannot open the controller: {e}. On Linux install packaging/linux/70-hub.rules and reconnect it."))),
                };
                let name = d.product_string().filter(|n| !n.is_empty()).unwrap_or("Xbox controller");
                Box::new(Xbox {
                    hid,
                    problem,
                    info: Info {
                        id: format!("xbox:{}", d.path().to_string_lossy()),
                        name: name.into(),
                        kind: Kind::Gamepad,
                        transport: Transport::Bluetooth,
                        serial: d.serial_number().filter(|s| !s.is_empty()).map(String::from),
                    },
                    battery: None,
                    levels: [0; 4],
                }) as Box<dyn Peripheral>
            })
            .collect()
    }
}

/// Decode the flags byte of report `0x04`. `None` while the pad is powered from a cable.
fn battery_from_flags(flags: u8) -> Option<Battery> {
    let online = flags & 0x80 != 0;
    let source = (flags >> 2) & 0x03;
    if !online || source == 0 {
        return None;
    }
    // Four levels: low / medium / high / full.
    let percent = [10, 40, 70, 100][(flags & 0x03) as usize];
    Some(Battery { percent: Some(percent), charging: flags & 0x10 != 0, coarse: true })
}

fn rumble_report(old: &[u8; 4], new: &[u8; 4]) -> [u8; 9] {
    let mut r = [0u8; 9];
    r[0] = REPORT_RUMBLE;
    // Only reprogram motors whose level changed (xpadneo does the same to avoid firmware hiccups).
    let bit = |m: Motor| match m {
        Motor::Weak => 0x01,
        Motor::Strong => 0x02,
        Motor::TriggerRight => 0x04,
        Motor::TriggerLeft => 0x08,
    };
    for (i, m) in MOTORS.iter().enumerate() {
        if old[i] != new[i] {
            r[1] |= bit(*m);
        }
    }
    // Wire order: left trigger, right trigger, strong, weak.
    r[2] = new[2];
    r[3] = new[3];
    r[4] = new[0];
    r[5] = new[1];
    // Pulse for about an hour (as the Windows driver does); stopping is "level 0".
    r[6] = 0xFF;
    r[8] = 0xEB;
    r
}

impl Xbox {
    fn drain(&mut self) {
        let Some(hid) = &self.hid else { return };
        let mut buf = [0u8; 64];
        while let Ok(n) = hid.read_timeout(&mut buf, 0) {
            if n == 0 {
                break;
            }
            if buf[0] == REPORT_BATTERY && n == 2 {
                self.battery = battery_from_flags(buf[1]);
            }
        }
    }
}

impl Peripheral for Xbox {
    fn info(&self) -> &Info {
        &self.info
    }

    fn capabilities(&self) -> Vec<Capability> {
        if self.hid.is_none() {
            return vec![];
        }
        vec![Capability::Battery, Capability::Rumble(MOTORS.to_vec())]
    }

    fn problem(&self) -> Option<String> {
        self.problem.clone()
    }

    fn tick(&mut self) {
        self.drain();
    }

    fn battery(&mut self) -> Result<Option<Battery>> {
        self.drain();
        // The Battery Service gives a real percentage; it does not say whether the pad is charging.
        let from_stack = self.info.serial.as_deref().and_then(hub_core::bluetooth::battery_percent);
        Ok(match from_stack {
            Some(p) => Some(Battery { percent: Some(p), charging: self.battery.is_some_and(|b| b.charging), coarse: false }),
            None => self.battery,
        })
    }

    fn rumble(&mut self, levels: &[(Motor, u8)]) -> Result<()> {
        let mut new = self.levels;
        for &(m, l) in levels {
            if let Some(i) = MOTORS.iter().position(|x| *x == m) {
                new[i] = l.min(100);
            }
        }
        let Some(hid) = &self.hid else { return Err(Error::Unsupported("rumble (controller not opened)")) };
        let report = rumble_report(&self.levels, &new);
        if report[1] == 0 {
            return Ok(());
        }
        hid.write(&report).map_err(Error::from)?;
        self.levels = new;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_flags() {
        // online, on battery, level 2 -> "high", not charging.
        let b = battery_from_flags(0x80 | (1 << 2) | 2).unwrap();
        assert_eq!((b.percent, b.charging, b.coarse), (Some(70), false, true));
        // charging bit.
        assert!(battery_from_flags(0x80 | (1 << 2) | 0x10 | 1).unwrap().charging);
        // cable-powered or offline: no battery.
        assert!(battery_from_flags(0x80).is_none());
        assert!(battery_from_flags(0x04).is_none());
    }

    #[test]
    fn rumble_only_changed_motors_and_wire_order() {
        let old = [0, 0, 0, 0];
        let new = [30, 0, 0, 80]; // strong, weak, left trigger, right trigger
        let r = rumble_report(&old, &new);
        assert_eq!(r[0], 0x03);
        assert_eq!(r[1], 0x02 | 0x04); // strong + right trigger
        assert_eq!((r[2], r[3], r[4], r[5]), (0, 80, 30, 0)); // lt, rt, strong, weak
    }
}
