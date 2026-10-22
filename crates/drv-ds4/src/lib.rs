//! Sony DualShock 4 over USB or Bluetooth: lightbar colour and battery.
//!
//! USB: output report `0x05` (32 bytes). Bluetooth: output report `0x11` (78 bytes) whose last
//! four bytes are a CRC32 over `0xA2` followed by the first 74 bytes; without it the pad ignores
//! the report. Reading feature report `0x02` once switches a Bluetooth pad from the simple input
//! report to the full `0x11` one, which carries the battery status.

use hidapi::{HidApi, HidDevice};
use hub_core::{
    transport_of, Battery, Capability, Driver, Error, Info, Kind, Motor, Peripheral, Result, Rgb, Transport,
};

const VID: u16 = 0x054C;
const PIDS: &[(u16, &str)] = &[(0x05C4, "DualShock 4"), (0x09CC, "DualShock 4 (v2)"), (0x0BA0, "DualShock 4 USB adapter")];
/// The Sony USB wireless adapter: the pad behind it speaks the USB protocol over a 2.4 GHz link.
const PID_WIRELESS_ADAPTER: u16 = 0x0BA0;

const BT_LEN: usize = 78;
const USB_LEN: usize = 32;

pub struct Ds4 {
    hid: HidDevice,
    info: Info,
    color: Rgb,
    /// Motor levels 0..=255 (strong/left, weak/right). Every output report carries these together
    /// with the lightbar, so a colour change must not cancel a running vibration.
    strong: u8,
    weak: u8,
}

pub struct Ds4Driver;

impl Driver for Ds4Driver {
    fn name(&self) -> &'static str {
        "DualShock 4"
    }

    fn discover(&self, api: &HidApi) -> Vec<Box<dyn Peripheral>> {
        api.device_list()
            .filter(|d| d.vendor_id() == VID)
            .filter_map(|d| {
                let name = PIDS.iter().find(|(p, _)| *p == d.product_id())?.1;
                let transport = if d.product_id() == PID_WIRELESS_ADAPTER {
                    Transport::Dongle
                } else {
                    transport_of(d)
                };
                let bt = transport == Transport::Bluetooth;
                let hid = d.open_device(api).ok()?;
                if bt {
                    // Switch to full input reports; the reply itself is not needed.
                    let mut buf = [0u8; 37];
                    buf[0] = 0x02;
                    let _ = hid.get_feature_report(&mut buf);
                }
                let info = Info {
                    id: format!("ds4:{}", d.path().to_string_lossy()),
                    name: name.into(),
                    kind: Kind::Gamepad,
                    transport,
                    serial: d.serial_number().filter(|s| !s.is_empty()).map(String::from),
                };
                Some(Box::new(Ds4 { hid, info, color: Rgb(0, 0, 64), strong: 0, weak: 0 }) as Box<dyn Peripheral>)
            })
            .collect()
    }
}

/// CRC32 (reflected, poly 0xEDB88320) as used by the pad's Bluetooth reports.
fn crc32(seed_byte: u8, data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in std::iter::once(&seed_byte).chain(data) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn output_report(bluetooth: bool, c: Rgb, strong: u8, weak: u8) -> Vec<u8> {
    if bluetooth {
        let mut r = vec![0u8; BT_LEN];
        r[0] = 0x11;
        r[1] = 0xC0; // HID output + CRC flags, 1 ms poll interval
        r[3] = 0x07; // enable rumble, lightbar, flash
        r[6] = weak;
        r[7] = strong;
        r[8..11].copy_from_slice(&[c.0, c.1, c.2]);
        let crc = crc32(0xA2, &r[..BT_LEN - 4]);
        r[BT_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
        r
    } else {
        let mut r = vec![0u8; USB_LEN];
        r[0] = 0x05;
        r[1] = 0x07;
        r[4] = weak;
        r[5] = strong;
        r[6..9].copy_from_slice(&[c.0, c.1, c.2]);
        r
    }
}

/// Decode the status byte (`low nibble` level 0..=10, bit 4 = cable connected).
fn battery_from_status(status: u8) -> Battery {
    let level = (status & 0x0F) as u32;
    let cable = status & 0x10 != 0;
    let percent = if cable { (level * 10 + 5).min(100) } else { (level.min(9) * 10 + 5).min(100) };
    Battery { percent: Some(percent as u8), charging: cable && level < 11, coarse: false }
}

impl Ds4 {
    fn send(&mut self) -> Result<()> {
        let report = output_report(self.info.transport == Transport::Bluetooth, self.color, self.strong, self.weak);
        self.hid.write(&report)?;
        Ok(())
    }
}

impl Peripheral for Ds4 {
    fn info(&self) -> &Info {
        &self.info
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![
            Capability::Battery,
            Capability::Rgb { zones: vec!["Lightbar".into()] },
            Capability::Rumble(vec![Motor::Strong, Motor::Weak]),
        ]
    }

    fn battery(&mut self) -> Result<Option<Battery>> {
        let bt = self.info.transport == Transport::Bluetooth;
        // Offset of the status byte in the input report, including the report ID.
        let (id, off) = if bt { (0x11, 32) } else { (0x01, 30) };
        let mut buf = [0u8; 96];
        // The pad streams reports; skip until the full one shows up.
        for _ in 0..8 {
            let n = self.hid.read_timeout(&mut buf, 100)?;
            if n > off && buf[0] == id {
                return Ok(Some(battery_from_status(buf[off])));
            }
        }
        Err(Error::Protocol("no full input report from the controller".into()))
    }

    fn set_rgb(&mut self, zone: usize, color: Rgb) -> Result<()> {
        if zone != 0 {
            return Err(Error::Unsupported("zone"));
        }
        self.color = color;
        self.send()
    }

    fn rumble(&mut self, levels: &[(Motor, u8)]) -> Result<()> {
        for &(motor, level) in levels {
            let v = (level.min(100) as u16 * 255 / 100) as u8;
            match motor {
                Motor::Strong => self.strong = v,
                Motor::Weak => self.weak = v,
                _ => return Err(Error::Unsupported("trigger motors")),
            }
        }
        self.send()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_reference() {
        // Standard CRC-32 check value for "123456789" (seed byte folded in as first byte).
        assert_eq!(crc32(b'1', b"23456789"), 0xCBF4_3926);
    }

    #[test]
    fn bt_report_has_valid_crc() {
        let r = output_report(true, Rgb(1, 2, 3), 10, 20);
        assert_eq!(r.len(), BT_LEN);
        assert_eq!(&r[8..11], &[1, 2, 3]);
        assert_eq!((r[6], r[7]), (20, 10)); // weak/right, strong/left
        let crc = u32::from_le_bytes(r[BT_LEN - 4..].try_into().unwrap());
        assert_eq!(crc, crc32(0xA2, &r[..BT_LEN - 4]));
    }

    #[test]
    fn battery_decode() {
        assert_eq!(battery_from_status(0x05).percent, Some(55));
        assert!(battery_from_status(0x1A).charging);
        assert!(!battery_from_status(0x1B).charging); // cable + level 11 = full
    }
}
