//! The byte layout of HID++ onboard profiles (feature 0x8100) as used by Logitech gaming mice, from
//! Solaar's `hidpp20.py` (`OnboardProfile`, `Button`). A profile is one flash sector: settings,
//! 4-byte button entries, a UTF-16 name and lighting, with a CRC-16 in the last two bytes. Sectors
//! are kept as raw bytes and edited in place, so fields that are not understood survive a write.

use hub_core::{ButtonAction, Profile, Rgb};

/// CRC-16/CCITT-FALSE (polynomial 0x1021, start value 0xFFFF), the checksum at the end of a sector.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// Answer of GetDescription (function 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Description {
    pub memory_model: u8,
    pub profile_format: u8,
    pub macro_format: u8,
    pub profile_count: u8,
    pub button_count: u8,
    pub sector_count: u8,
    pub sector_size: u16,
    pub shift: u8,
}

impl Description {
    pub fn parse(r: &[u8; 16]) -> Description {
        Description {
            memory_model: r[0],
            profile_format: r[1],
            macro_format: r[2],
            profile_count: r[3],
            button_count: r[5],
            sector_count: r[6],
            sector_size: u16::from_be_bytes([r[7], r[8]]),
            shift: r[9],
        }
    }

    /// Whether this crate knows the memory layout (Solaar only handles these as well).
    pub fn supported(&self) -> bool {
        self.memory_model == 1 && self.profile_format <= 5 && self.sector_size >= 224
    }
}

pub fn decode_button(b: &[u8]) -> ButtonAction {
    let raw = [b[0], b[1], b[2], b[3]];
    match b[0] >> 4 {
        0x0 | 0x1 => ButtonAction::Macro {
            sector: (u16::from(b[0] & 0x0F) << 8) | u16::from(b[1]),
            address: u16::from_be_bytes([b[2], b[3]]),
            stop: b[0] >> 4 == 1,
        },
        0x8 => match b[1] {
            0 => ButtonAction::Disabled,
            1 => ButtonAction::Mouse(u16::from_be_bytes([b[2], b[3]])),
            2 => ButtonAction::Key { modifiers: b[2], usage: b[3] },
            3 => ButtonAction::Consumer(u16::from_be_bytes([b[2], b[3]])),
            _ => ButtonAction::Raw(raw),
        },
        0x9 => ButtonAction::Function { id: b[1], data: b[3] },
        _ => ButtonAction::Raw(raw),
    }
}

pub fn encode_button(a: ButtonAction) -> [u8; 4] {
    match a {
        ButtonAction::Disabled => [0x80, 0x00, 0xFF, 0xFF],
        ButtonAction::Mouse(m) => [0x80, 0x01, (m >> 8) as u8, m as u8],
        ButtonAction::Key { modifiers, usage } => [0x80, 0x02, modifiers, usage],
        ButtonAction::Consumer(c) => [0x80, 0x03, (c >> 8) as u8, c as u8],
        ButtonAction::Function { id, data } => [0x90, id, 0x00, data], // the mouse stores 00 here
        ButtonAction::Macro { sector, address, stop } => {
            [(u8::from(stop) << 4) | (sector >> 8) as u8 & 0x0F, sector as u8, (address >> 8) as u8, address as u8]
        }
        ButtonAction::Raw(b) => b,
    }
}

const BUTTONS_AT: usize = 32;
const NAME_AT: usize = 160;

/// One profile sector, as read from the mouse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sector {
    pub number: u16,
    pub enabled: bool,
    pub raw: Vec<u8>,
}

impl Sector {
    pub fn crc_ok(&self) -> bool {
        let n = self.raw.len();
        n > 2 && crc16(&self.raw[..n - 2]) == u16::from_be_bytes([self.raw[n - 2], self.raw[n - 1]])
    }

    pub fn fix_crc(&mut self) {
        let n = self.raw.len();
        let crc = crc16(&self.raw[..n - 2]);
        self.raw[n - 2..].copy_from_slice(&crc.to_be_bytes());
    }

    pub fn button(&self, i: usize) -> ButtonAction {
        decode_button(&self.raw[BUTTONS_AT + i * 4..][..4])
    }

    pub fn set_button(&mut self, i: usize, a: ButtonAction) {
        self.raw[BUTTONS_AT + i * 4..][..4].copy_from_slice(&encode_button(a));
        self.fix_crc();
    }

    pub fn to_profile(&self, index: usize, buttons: usize) -> Profile {
        let d = &self.raw;
        let units: Vec<u16> = d[NAME_AT..NAME_AT + 48].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let end = units.iter().position(|&u| u == 0 || u == 0xFFFF).unwrap_or(units.len());
        Profile {
            index,
            enabled: self.enabled,
            name: String::from_utf16_lossy(&units[..end]),
            report_interval_ms: d[0],
            dpi_levels: (0..5).map(|i| u16::from_le_bytes([d[3 + i * 2], d[4 + i * 2]])).collect(),
            default_dpi_index: d[1],
            shift_dpi_index: d[2],
            color: Rgb(d[13], d[14], d[15]),
            buttons: (0..buttons).map(|i| self.button(i)).collect(),
        }
    }
}

/// Parse the directory sector: `(sector, enabled)` for every profile, ending at `ff ff`.
pub fn parse_directory(raw: &[u8]) -> Vec<(u16, bool)> {
    raw.chunks(4).filter(|c| c.len() == 4)
        .take_while(|c| c[0..2] != [0xFF, 0xFF])
        .map(|c| (u16::from_be_bytes([c[0], c[1]]), c[2] != 0))
        .collect()
}

/// Build the flash directory sector for these `(sector, enabled)` profiles: 4-byte entries, the
/// `ff ff 00 00` end marker, `ff` padding and the CRC-16 in the last two bytes.
pub fn build_directory(entries: &[(u16, bool)], size: usize) -> Vec<u8> {
    let mut raw = Vec::with_capacity(size);
    for &(sector, enabled) in entries {
        raw.extend_from_slice(&sector.to_be_bytes());
        raw.extend_from_slice(&[u8::from(enabled), 0]);
    }
    raw.extend_from_slice(&[0xFF, 0xFF, 0, 0]);
    raw.resize(size - 2, 0xFF);
    let crc = crc16(&raw);
    raw.extend_from_slice(&crc.to_be_bytes());
    raw
}

/// `# ...` header, then one `sector <number> <enabled> <hex>` line per sector (directory first).
pub fn backup_text(device: &str, directory_sector: u16, directory: &[u8], profiles: &[Sector]) -> String {
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let mut out = format!("# Peripherals HUB onboard profile backup v1\n# {device}\n");
    out += &format!("directory {directory_sector} {}\n", hex(directory));
    for s in profiles {
        out += &format!("sector {} {} {}\n", s.number, u8::from(s.enabled), hex(&s.raw));
    }
    out
}

pub struct Backup {
    pub directory_sector: u16,
    pub directory: Vec<u8>,
    pub sectors: Vec<Sector>,
}

pub fn parse_backup(text: &str) -> Result<Backup, String> {
    let unhex = |s: &str| -> Result<Vec<u8>, String> {
        if s.len() & 1 != 0 {
            return Err("odd hex length".into());
        }
        (0..s.len() / 2).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())).collect()
    };
    let (mut directory, mut sectors) = (None, vec![]);
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["directory", n, hex] => directory = Some((n.parse::<u16>().map_err(|e| e.to_string())?, unhex(hex)?)),
            ["sector", n, en, hex] => sectors.push(Sector {
                number: n.parse().map_err(|e: std::num::ParseIntError| e.to_string())?,
                enabled: *en != "0",
                raw: unhex(hex)?,
            }),
            _ => return Err(format!("cannot read line: {line}")),
        }
    }
    let (directory_sector, directory) = directory.ok_or("no directory line")?;
    if sectors.is_empty() {
        return Err("no sectors in the backup".into());
    }
    if let Some(bad) = sectors.iter().find(|s| !s.crc_ok()) {
        return Err(format!("sector {} has a bad checksum; the file is damaged", bad.number));
    }
    Ok(Backup { directory_sector, directory, sectors })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_matches_the_standard_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn button_entries_round_trip() {
        for a in [
            ButtonAction::Disabled,
            ButtonAction::Mouse(0x0008),
            ButtonAction::Key { modifiers: 0x02, usage: 0x04 },
            ButtonAction::Consumer(0x00CD),
            ButtonAction::Function { id: 0x07, data: 0 },
            ButtonAction::Macro { sector: 0x123, address: 0x0456, stop: false },
        ] {
            assert_eq!(decode_button(&encode_button(a)), a);
        }
        // what the G502 X Plus really stores for "DPI shift"
        assert_eq!(decode_button(&[0x90, 0x07, 0, 0]), ButtonAction::Function { id: 7, data: 0 });
        assert_eq!(encode_button(ButtonAction::Function { id: 7, data: 0 }), [0x90, 0x07, 0, 0]);
        // unknown entries are kept as they are
        let raw = [0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(encode_button(decode_button(&raw)), raw);
    }

    fn sample() -> Sector {
        let mut raw = vec![0xFFu8; 256];
        raw[0] = 1; // 1 ms
        raw[3..5].copy_from_slice(&800u16.to_le_bytes());
        raw[BUTTONS_AT..BUTTONS_AT + 4].copy_from_slice(&[0x80, 0x01, 0x00, 0x01]);
        for (i, u) in "Prof".encode_utf16().enumerate() {
            raw[NAME_AT + i * 2..][..2].copy_from_slice(&u.to_le_bytes());
        }
        raw[NAME_AT + 8..NAME_AT + 10].copy_from_slice(&[0, 0]);
        let mut s = Sector { number: 2, enabled: true, raw };
        s.fix_crc();
        s
    }

    #[test]
    fn sector_parses_and_edits_only_one_button() {
        let mut s = sample();
        assert!(s.crc_ok());
        let p = s.to_profile(1, 3);
        assert_eq!((p.name.as_str(), p.report_interval_ms, p.dpi_levels[0]), ("Prof", 1, 800));
        assert_eq!(p.buttons[0], ButtonAction::Mouse(1));
        let before = s.raw.clone();
        s.set_button(1, ButtonAction::Mouse(0x0010));
        assert!(s.crc_ok());
        let changed: Vec<usize> = (0..256).filter(|&i| before[i] != s.raw[i]).collect();
        // the new button entry (4 bytes at most) and the checksum, nothing else
        assert!(changed.iter().all(|&i| (BUTTONS_AT + 4..BUTTONS_AT + 8).contains(&i) || i >= 254), "{changed:?}");
        assert_eq!(s.button(1), ButtonAction::Mouse(0x0010));
    }

    #[test]
    fn directory_ends_at_the_marker() {
        let raw = [0, 2, 1, 0, 0, 3, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(parse_directory(&raw), vec![(2, true), (3, false)]);
    }

    #[test]
    fn built_directory_parses_back_and_has_a_valid_checksum() {
        let raw = build_directory(&[(1, true), (2, false)], 255);
        assert_eq!(raw.len(), 255);
        assert_eq!(parse_directory(&raw), vec![(1, true), (2, false)]);
        assert_eq!(crc16(&raw[..253]), u16::from_be_bytes([raw[253], raw[254]]));
    }

    #[test]
    fn backup_round_trips_and_detects_damage() {
        let s = sample();
        let text = backup_text("G502", 0, &[0, 2, 1, 0, 0xFF, 0xFF, 0, 0], std::slice::from_ref(&s));
        let b = parse_backup(&text).unwrap();
        assert_eq!(b.sectors, vec![s]);
        assert_eq!(b.directory_sector, 0);
        let damaged = text.replace("sector 2 1 01", "sector 2 1 02");
        assert!(parse_backup(&damaged).is_err());
    }
}
