//! Sony DualShock 4 over USB or Bluetooth: lightbar colour, rumble, battery and (Bluetooth only)
//! the built-in speaker.
//!
//! USB: output report `0x05` (32 bytes). Bluetooth: output report `0x11` (78 bytes) whose last
//! four bytes are a CRC32 over `0xA2` followed by the first 74 bytes; without it the pad ignores
//! the report. Reading feature report `0x02` once switches a Bluetooth pad from the simple input
//! report to the full `0x11` one, which carries the battery status.

pub mod sbc;

use std::time::{Duration, Instant};

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

/// Bluetooth output report `0x11` with the audio path enabled (`audio_volume` is 0..=100).
fn audio_control_report(c: Rgb, audio_volume: u8, rumble: u8) -> Vec<u8> {
    let mut r = vec![0u8; BT_LEN];
    r[0] = 0x11;
    r[1] = 0xC0;
    r[2] = 0x20; // audio control
    r[3] = 0xF3;
    r[4] = 0x04;
    r[6] = rumble;
    r[7] = rumble;
    r[8..11].copy_from_slice(&[c.0, c.1, c.2]);
    r[21..26].copy_from_slice(&[0x43, 0x43, 0x00, audio_volume, 0x85]);
    let crc = crc32(0xA2, &r[..BT_LEN - 4]);
    r[BT_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
    r
}

/// Report `0x14` (two SBC frames) or `0x15` (the same, with the control block in front, which
/// keeps the volume alive), `counter` counts SBC frames.
fn audio_report(with_control: bool, control: &[u8], counter: u16, frames: &[u8]) -> Vec<u8> {
    debug_assert_eq!(frames.len(), 2 * sbc::FRAME_BYTES);
    let (mut r, at) = if with_control {
        let mut r = vec![0u8; 334];
        r[0] = 0x15;
        r[1] = 0xC0;
        r[3..78].copy_from_slice(&control[3..78]);
        (r, 78)
    } else {
        let mut r = vec![0u8; 270];
        r[0] = 0x14;
        r[1] = 0x40;
        (r, 3)
    };
    r[2] = 0xA0;
    r[at..at + 2].copy_from_slice(&counter.to_le_bytes());
    r[at + 2] = 0x02; // built-in speaker (0x24 would be the headphone jack)
    r[at + 3..at + 3 + frames.len()].copy_from_slice(frames);
    let n = r.len();
    let crc = crc32(0xA2, &r[..n - 4]);
    r[n - 4..].copy_from_slice(&crc.to_le_bytes());
    r
}

/// Convert to 32 kHz stereo by linear interpolation.
fn to_32k_stereo(samples: &[i16], rate: u32, channels: u16) -> Vec<i16> {
    let ch = channels.max(1) as usize;
    let frames = samples.len() / ch;
    let out_frames = (frames as u64 * 32_000 / rate.max(1) as u64) as usize;
    let at = |f: usize, c: usize| samples[f.min(frames - 1) * ch + c.min(ch - 1)] as f64;
    let mut out = Vec::with_capacity(out_frames * 2);
    for i in 0..out_frames {
        let pos = i as f64 * rate as f64 / 32_000.0;
        let (a, frac) = (pos as usize, pos.fract());
        for c in 0..2 {
            out.push((at(a, c) * (1.0 - frac) + at(a + 1, c) * frac).round() as i16);
        }
    }
    out
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
        let mut caps = vec![
            Capability::Battery,
            Capability::Rgb { zones: vec!["Lightbar".into()] },
            Capability::Rumble(vec![Motor::Strong, Motor::Weak]),
        ];
        if self.info.transport == Transport::Bluetooth {
            caps.push(Capability::Speaker);
        }
        caps
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

    fn play_audio(&mut self, samples: &[i16], rate: u32, channels: u16, volume: u8, stop: &dyn Fn() -> bool) -> Result<()> {
        if self.info.transport != Transport::Bluetooth {
            return Err(Error::Unsupported("speaker (only over Bluetooth)"));
        }
        if samples.is_empty() {
            return Ok(());
        }
        let mut pcm = to_32k_stereo(samples, rate, channels);
        // Whole SBC frames only; pad the tail with silence up to a pair of frames.
        let pair = sbc::FRAME_SAMPLES * 2 * 2;
        pcm.resize(pcm.len().div_ceil(pair) * pair, 0);
        let volume = volume.min(100);
        let control = audio_control_report(self.color, volume, 0);
        self.hid.write(&audio_control_report(self.color, volume, 80))?; // the pad buzzes once to confirm
        std::thread::sleep(Duration::from_millis(200));
        self.hid.write(&control)?;
        std::thread::sleep(Duration::from_millis(50));

        let mut enc = sbc::Encoder::default();
        let start = Instant::now();
        // Each report carries two SBC frames = 256 samples per channel = 8 ms of sound; stay about
        // 96 ms ahead of real time so Bluetooth hiccups do not make it stutter.
        let step = Duration::from_millis(8);
        let lead = Duration::from_millis(96);
        let mut counter = 0u16;
        for (n, chunk) in pcm.chunks_exact(pair).enumerate() {
            if stop() {
                break;
            }
            let a = enc.encode(&chunk[..pair / 2]);
            let b = enc.encode(&chunk[pair / 2..]);
            let mut frames = Vec::with_capacity(2 * sbc::FRAME_BYTES);
            frames.extend_from_slice(&a);
            frames.extend_from_slice(&b);
            self.hid.write(&audio_report(n % 3 == 0, &control, counter, &frames))?;
            counter = counter.wrapping_add(2);
            if let Some(wait) = (step * (n as u32 + 1)).checked_sub(lead + start.elapsed()) {
                std::thread::sleep(wait);
            }
        }
        if !stop() {
            std::thread::sleep(lead); // let what is already buffered on the pad play out
        }
        // Back to a plain output report: lightbar colour, no rumble, audio path off.
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
    fn audio_reports_have_valid_crc() {
        let control = audio_control_report(Rgb(0, 0, 64), 80, 0);
        assert_eq!(control.len(), BT_LEN);
        assert_eq!(control[24], 80);
        let frames = vec![0u8; 2 * sbc::FRAME_BYTES];
        for (with_control, len) in [(false, 270), (true, 334)] {
            let r = audio_report(with_control, &control, 6, &frames);
            assert_eq!(r.len(), len);
            let crc = u32::from_le_bytes(r[len - 4..].try_into().unwrap());
            assert_eq!(crc, crc32(0xA2, &r[..len - 4]));
        }
    }

    #[test]
    fn resamples_mono_to_32k_stereo() {
        let out = to_32k_stereo(&[100; 160], 16_000, 1);
        assert_eq!(out.len(), 320 * 2);
        assert!(out.iter().all(|&v| v == 100));
    }

    #[test]
    fn battery_decode() {
        assert_eq!(battery_from_status(0x05).percent, Some(55));
        assert!(battery_from_status(0x1A).charging);
        assert!(!battery_from_status(0x1B).charging); // cable + level 11 = full
    }
}
