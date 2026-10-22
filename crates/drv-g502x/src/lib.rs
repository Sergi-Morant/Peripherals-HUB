//! Logitech G502 X (Plus) through a Lightspeed receiver, speaking HID++ 2.0.
//!
//! Long reports are 20 bytes: `11 <device index> <feature index> <function<<4 | software id>
//! <16 params>`. Feature indexes are looked up at runtime with `IRoot.GetFeature`. Battery uses
//! 0x1004; static colour uses RGB Effects 0x8071 (layout verified with `hub probe` on a real G502 X
//! Plus: one cluster, effect id 1 = fixed colour). DPI uses 0x2201, polling rate 0x8060 and the
//! onboard/host switch 0x8100; function numbers and the DPI list encoding follow Solaar's
//! `hidpp20.py`/`settings_templates.py`, effect parameter offsets its `LEDEffects` table.

use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice};
use hub_core::{
    transport_of, Battery, Capability, Driver, Effect, EffectSetting, Error, Info, Kind, Peripheral, Result, Rgb,
    Settings, Transport,
};

const VID: u16 = 0x046D;
const USAGE_PAGE: u16 = 0xFF00;
const USAGE_LONG: u16 = 0x0002;
const LONG: u8 = 0x11;
const REPORT_LEN: usize = 20;
const SW_ID: u8 = 0x07;
const TIMEOUT: Duration = Duration::from_millis(500);

const F_FEATURE_SET: u16 = 0x0001;
const F_DEVICE_NAME: u16 = 0x0005;
const F_BATTERY_LEVEL: u16 = 0x1000;
const F_UNIFIED_BATTERY: u16 = 0x1004;
const F_RGB_EFFECTS: u16 = 0x8071;
const F_ADJUSTABLE_DPI: u16 = 0x2201;
const F_REPORT_RATE: u16 = 0x8060;
const F_ONBOARD_PROFILES: u16 = 0x8100;

pub struct Hidpp {
    hid: HidDevice,
    index: u8,
    /// HID path of the interface; unique when several receivers are plugged in.
    path: String,
    /// How the mouse reaches the host: a receiver slot is 2.4 GHz, otherwise the interface's bus.
    transport: Transport,
    /// Attempts per request. 1 while probing receiver slots (empty ones never answer); raised for a
    /// mouse that was found, because an idle mouse needs a moment to wake up.
    tries: std::cell::Cell<u32>,
}

impl Hidpp {
    /// Call `function` of the feature at `feature_index`; returns the 16 parameter bytes of the reply.
    fn call(&self, feature_index: u8, function: u8, params: &[u8]) -> Result<[u8; 16]> {
        let mut last = Error::Protocol("no HID++ reply".into());
        for _ in 0..self.tries.get() {
            match self.call_once(feature_index, function, params) {
                Err(Error::Protocol(m)) if m == "no HID++ reply" => last = Error::Protocol(m),
                other => return other,
            }
        }
        Err(last)
    }

    fn call_once(&self, feature_index: u8, function: u8, params: &[u8]) -> Result<[u8; 16]> {
        let mut req = [0u8; REPORT_LEN];
        req[0] = LONG;
        req[1] = self.index;
        req[2] = feature_index;
        req[3] = function << 4 | SW_ID;
        req[4..4 + params.len()].copy_from_slice(params);
        self.hid.write(&req)?;
        let deadline = Instant::now() + TIMEOUT;
        let mut buf = [0u8; 64];
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let n = self.hid.read_timeout(&mut buf, left.as_millis().max(1) as i32)?;
            if n < REPORT_LEN || buf[0] != LONG || buf[1] != self.index {
                continue;
            }
            if buf[2] == 0xFF && buf[3] == feature_index {
                return Err(Error::Protocol(format!("HID++ error {:#04x}", buf[5])));
            }
            if buf[2] == feature_index && buf[3] == req[3] {
                return Ok(buf[4..20].try_into().unwrap());
            }
        }
        Err(Error::Protocol("no HID++ reply".into()))
    }

    /// Index of feature `id`, or `None` if the device does not have it.
    fn feature(&self, id: u16) -> Result<Option<u8>> {
        let r = self.call(0, 0, &id.to_be_bytes())?;
        Ok((r[0] != 0).then_some(r[0]))
    }

    fn name(&self) -> Result<String> {
        let Some(ix) = self.feature(F_DEVICE_NAME)? else { return Ok(String::new()) };
        let len = self.call(ix, 0, &[])?[0] as usize;
        let mut name = Vec::new();
        while name.len() < len {
            let r = self.call(ix, 1, &[name.len() as u8])?;
            name.extend(r.iter().take(len - name.len()));
        }
        Ok(String::from_utf8_lossy(&name).into_owned())
    }

    /// `(feature id, feature index)` for every feature the device exposes.
    fn feature_table(&self) -> Result<Vec<(u16, u8)>> {
        let Some(ix) = self.feature(F_FEATURE_SET)? else { return Ok(vec![]) };
        let count = self.call(ix, 0, &[])?[0];
        (1..=count)
            .map(|i| {
                let r = self.call(ix, 1, &[i])?;
                Ok((u16::from_be_bytes([r[0], r[1]]), i))
            })
            .collect()
    }
}

/// RGB Effects (0x8071) layout, big-endian, as implemented by OpenLogi's `rgb_effects.rs`.
/// Function 0 = getInfo `[cluster, effect, type]` (0xFF = all, type 0 = general), 1 =
/// setRgbClusterEffect, 5 = get/set software control.
impl Hidpp {
    /// Cluster count, then `(effect id, capabilities)` for every effect of every cluster.
    fn rgb_layout(&self, ix: u8) -> Result<Vec<Vec<(u16, u16)>>> {
        let clusters = self.call(ix, 0, &[0xFF, 0xFF, 0])?[2];
        (0..clusters)
            .map(|c| {
                let n = self.call(ix, 0, &[c, 0xFF, 0])?[4];
                (0..n)
                    .map(|e| {
                        let r = self.call(ix, 0, &[c, e, 0])?;
                        Ok((u16::from_be_bytes([r[2], r[3]]), u16::from_be_bytes([r[4], r[5]])))
                    })
                    .collect()
            })
            .collect()
    }
}

/// Open every HID++ long-report interface of Logitech devices and find paired devices on it.
fn open_all(api: &HidApi) -> Vec<(Hidpp, String)> {
    let mut found = Vec::new();
    for d in api.device_list().filter(|d| {
        // Bluetooth is skipped: it is untested for this mouse, and some non-Logitech Bluetooth
        // devices (e.g. the RK S85 keyboard) borrow Logitech's vendor id and must not get HID++.
        d.vendor_id() == VID
            && d.usage_page() == USAGE_PAGE
            && d.usage() == USAGE_LONG
            && transport_of(d) != Transport::Bluetooth
    }) {
        // Receivers hold up to 6 paired devices (index 1..=6); a directly connected device is index 0xFF.
        for index in [1, 2, 3, 4, 5, 6, 0xFF] {
            let Ok(hid) = d.open_device(api) else { break };
            let path = d.path().to_string_lossy().into_owned();
            // Index 0xFF is the device itself (cable or Bluetooth); 1..=6 are receiver slots.
            let transport = if index == 0xFF { transport_of(d) } else { Transport::Dongle };
            let h = Hidpp { hid, index, path, transport, tries: std::cell::Cell::new(1) };
            if let Ok(name) = h.name() {
                if !name.is_empty() {
                    h.tries.set(3);
                    found.push((h, name));
                }
            }
        }
    }
    found
}

/// Human-readable dump of every HID++ device and its features, for finding feature indexes.
pub fn probe(api: &HidApi) -> Vec<String> {
    open_all(api)
        .into_iter()
        .map(|(h, name)| {
            let table = h.feature_table().unwrap_or_default();
            let feats: Vec<String> =
                table.iter().map(|(id, ix)| format!("{id:#06x}@{ix}")).collect();
            let mut line = format!("{name} (index {}): {}", h.index, feats.join(" "));
            if let Ok(Some(ix)) = h.feature(F_RGB_EFFECTS) {
                match h.rgb_layout(ix) {
                    Ok(layout) => {
                        for (c, effects) in layout.iter().enumerate() {
                            let e: Vec<String> = effects
                                .iter()
                                .enumerate()
                                .map(|(i, (id, caps))| format!("#{i}=id {id:#06x} caps {caps:#06x}"))
                                .collect();
                            line.push_str(&format!("\n  rgb cluster {c}: {}", e.join(", ")));
                        }
                    }
                    Err(e) => line.push_str(&format!("\n  rgb layout error: {e}")),
                }
            }
            line
        })
        .collect()
}

pub struct G502xDriver;

impl Driver for G502xDriver {
    fn name(&self) -> &'static str {
        "Logitech G502 X"
    }

    fn discover(&self, api: &HidApi) -> Vec<Box<dyn Peripheral>> {
        open_all(api)
            .into_iter()
            .filter(|(_, name)| name.to_uppercase().contains("G502"))
            .map(|(h, name)| {
                let info = Info {
                    id: format!("g502x:{}:{}", h.path, h.index),
                    name,
                    kind: Kind::Mouse,
                    transport: h.transport,
                    serial: None,
                };
                Box::new(G502x::new(h, info)) as Box<dyn Peripheral>
            })
            .collect()
    }
}

/// Effect ids of 0x8071 and where their parameters sit in the 10-byte block (big-endian):
/// Static: colour@0..3. Breathe: colour@0..3, period@3..5, form@5, intensity@6.
/// Cycle: period@5..7, intensity@7.
fn effect_id(e: Effect) -> u16 {
    match e {
        Effect::Off => 0x00,
        Effect::Static => 0x01,
        Effect::Breathe => 0x0A,
        Effect::Cycle => 0x03,
    }
}

fn effect_params(s: &EffectSetting) -> [u8; 10] {
    let mut p = [0u8; 10];
    let period = s.period_ms.to_be_bytes();
    let intensity = s.brightness.clamp(1, 100);
    match s.effect {
        Effect::Off => {}
        Effect::Static => p[..3].copy_from_slice(&[s.color.0, s.color.1, s.color.2]),
        Effect::Breathe => {
            p[..3].copy_from_slice(&[s.color.0, s.color.1, s.color.2]);
            p[3..5].copy_from_slice(&period);
            p[6] = intensity;
        }
        Effect::Cycle => {
            p[5..7].copy_from_slice(&period);
            p[7] = intensity;
        }
    }
    p
}

/// DPI list of 0x2201: 16-bit values, where a value with the top three bits set (`0xE000 | step`)
/// means "from the previous value up to the next one in steps of `step`"; ends with 0.
fn parse_dpi_list(bytes: &[u8]) -> Vec<u16> {
    let mut list: Vec<u16> = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let v = u16::from_be_bytes([bytes[i], bytes[i + 1]]);
        if v == 0 {
            break;
        }
        if v >> 13 == 0b111 {
            let step = (v & 0x1FFF) as usize;
            let Some(&last) = list.last() else { break };
            let Some(end) = bytes.get(i + 2..i + 4).map(|b| u16::from_be_bytes([b[0], b[1]])) else { break };
            if step == 0 || end <= last {
                break;
            }
            list.extend((last as usize + step..=end as usize).step_by(step).map(|d| d as u16));
            i += 4;
        } else {
            list.push(v);
            i += 2;
        }
    }
    list
}

impl Hidpp {
    fn dpi_list(&self, ix: u8) -> Result<Vec<u16>> {
        let mut bytes = Vec::new();
        for page in 0..16u8 {
            // Function 1: sensor 0, direction 0, page; the reply starts with the sensor index.
            let r = self.call(ix, 1, &[0, 0, page])?;
            bytes.extend_from_slice(&r[1..]);
            if bytes.ends_with(&[0, 0]) {
                break;
            }
        }
        Ok(parse_dpi_list(&bytes))
    }

    fn dpi(&self, ix: u8) -> Result<u16> {
        // Function 2: sensor 0 -> [sensor, dpi, default dpi]; a zero dpi means "use the default".
        let r = self.call(ix, 2, &[0])?;
        let dpi = u16::from_be_bytes([r[1], r[2]]);
        Ok(if dpi == 0 { u16::from_be_bytes([r[3], r[4]]) } else { dpi })
    }

    /// Supported polling rates in Hz (bit i of the first reply byte = (i + 1) ms).
    fn rate_list(&self, ix: u8) -> Result<Vec<u16>> {
        let flags = self.call(ix, 0, &[])?[0];
        let mut hz: Vec<u16> = (0..8).filter(|i| flags >> i & 1 == 1).map(|i| 1000 / (i + 1)).collect();
        hz.sort_unstable();
        hz.dedup();
        Ok(hz)
    }

    fn rate(&self, ix: u8) -> Result<u16> {
        let ms = self.call(ix, 1, &[])?[0].max(1);
        Ok(1000 / ms as u16)
    }

    /// 1 = onboard profile active, 2 = host mode.
    fn onboard_mode(&self, ix: u8) -> Result<u8> {
        Ok(self.call(ix, 2, &[])?[0])
    }
}

pub struct G502x {
    h: Hidpp,
    info: Info,
    caps: Vec<Capability>,
    /// Effects the mouse reports as (zone 0 effect id -> index).
    effect_index: Vec<(u16, u8)>,
}

impl G502x {
    fn new(h: Hidpp, info: Info) -> G502x {
        let mut caps = vec![];
        if h.feature(F_UNIFIED_BATTERY).ok().flatten().is_some() || h.feature(F_BATTERY_LEVEL).ok().flatten().is_some() {
            caps.push(Capability::Battery);
        }
        let mut effect_index = vec![];
        if let Ok(Some(ix)) = h.feature(F_RGB_EFFECTS) {
            if let Ok(layout) = h.rgb_layout(ix) {
                effect_index = layout.first().map_or(vec![], |z| {
                    z.iter().enumerate().map(|(i, (id, _))| (*id, i as u8)).collect()
                });
            }
            caps.push(Capability::Rgb { zones: vec!["Lighting".into()] });
            let supported: Vec<Effect> = [Effect::Off, Effect::Static, Effect::Breathe, Effect::Cycle]
                .into_iter()
                .filter(|e| effect_index.iter().any(|(id, _)| *id == effect_id(*e)))
                .collect();
            caps.push(Capability::Effects(supported));
        }
        if let Ok(Some(ix)) = h.feature(F_ADJUSTABLE_DPI) {
            if let Ok(list) = h.dpi_list(ix) {
                if !list.is_empty() {
                    caps.push(Capability::Dpi(list));
                }
            }
        }
        if let Ok(Some(ix)) = h.feature(F_REPORT_RATE) {
            if let Ok(list) = h.rate_list(ix) {
                if !list.is_empty() {
                    caps.push(Capability::ReportRate(list));
                }
            }
        }
        if matches!(h.feature(F_ONBOARD_PROFILES), Ok(Some(_))) {
            caps.push(Capability::HostMode);
        }
        G502x { h, info, caps, effect_index }
    }

    fn rgb_ix(&self) -> Result<u8> {
        self.h.feature(F_RGB_EFFECTS)?.ok_or(Error::Unsupported("rgb"))
    }
}

impl Peripheral for G502x {
    fn info(&self) -> &Info {
        &self.info
    }

    fn capabilities(&self) -> Vec<Capability> {
        self.caps.clone()
    }

    fn set_rgb(&mut self, zone: usize, color: Rgb) -> Result<()> {
        self.set_effect(zone, EffectSetting { effect: Effect::Static, color, ..Default::default() })
    }

    fn set_effect(&mut self, zone: usize, setting: EffectSetting) -> Result<()> {
        if zone != 0 {
            return Err(Error::Unsupported("zone"));
        }
        let ix = self.rgb_ix()?;
        let id = effect_id(setting.effect);
        let index = self
            .effect_index
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, ix)| *ix)
            .ok_or(Error::Unsupported("this effect"))?;
        // Take control of all clusters (function 5: set, control flags, event flags).
        self.h.call(ix, 5, &[1, 1, 0])?;
        // Function 1: cluster, effect index, 10 effect params, flags (bit 0 = volatile).
        let mut args = [0u8; 13];
        args[1] = index;
        args[2..12].copy_from_slice(&effect_params(&setting));
        args[12] = 0x01;
        self.h.call(ix, 1, &args)?;
        Ok(())
    }

    fn settings(&mut self) -> Settings {
        let mut s = Settings::default();
        if let Ok(Some(ix)) = self.h.feature(F_ADJUSTABLE_DPI) {
            s.dpi = self.h.dpi(ix).ok();
        }
        if let Ok(Some(ix)) = self.h.feature(F_REPORT_RATE) {
            s.report_rate = self.h.rate(ix).ok();
        }
        if let Ok(Some(ix)) = self.h.feature(F_ONBOARD_PROFILES) {
            s.host_mode = self.h.onboard_mode(ix).ok().map(|m| m == 2);
        }
        s
    }

    fn set_dpi(&mut self, dpi: u16) -> Result<()> {
        let ix = self.h.feature(F_ADJUSTABLE_DPI)?.ok_or(Error::Unsupported("dpi"))?;
        let allowed = self.caps.iter().any(|c| matches!(c, Capability::Dpi(l) if l.contains(&dpi)));
        if !allowed {
            return Err(Error::Protocol(format!("{dpi} DPI is not a value this mouse accepts")));
        }
        let [hi, lo] = dpi.to_be_bytes();
        self.h.call(ix, 3, &[0, hi, lo])?;
        Ok(())
    }

    fn set_report_rate(&mut self, hz: u16) -> Result<()> {
        let ix = self.h.feature(F_REPORT_RATE)?.ok_or(Error::Unsupported("report rate"))?;
        let allowed = self.caps.iter().any(|c| matches!(c, Capability::ReportRate(l) if l.contains(&hz)));
        if !allowed || hz == 0 {
            return Err(Error::Protocol(format!("{hz} Hz is not a rate this mouse accepts")));
        }
        self.h.call(ix, 2, &[(1000 / hz) as u8])?;
        Ok(())
    }

    fn set_host_mode(&mut self, host: bool) -> Result<()> {
        let ix = self.h.feature(F_ONBOARD_PROFILES)?.ok_or(Error::Unsupported("host mode"))?;
        self.h.call(ix, 1, &[if host { 2 } else { 1 }])?;
        Ok(())
    }

    fn battery(&mut self) -> Result<Option<Battery>> {
        // Unified battery: GetStatus (function 1) -> [percent, level flags, charging status].
        // Battery level status: GetBatteryLevelStatus (function 0) -> [percent, next, status].
        let (r, charging_at) = if let Some(ix) = self.h.feature(F_UNIFIED_BATTERY)? {
            (self.h.call(ix, 1, &[])?, 2)
        } else if let Some(ix) = self.h.feature(F_BATTERY_LEVEL)? {
            (self.h.call(ix, 0, &[])?, 2)
        } else {
            return Ok(None);
        };
        // Status 1..=4 means charging in both features.
        Ok(Some(Battery { percent: Some(r[0].min(100)), charging: (1..=4).contains(&r[charging_at]), coarse: false }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_list_expands_step_ranges() {
        // 100, then "step 50" up to 400, then a single value 800, then the terminator.
        let bytes = [0x00, 0x64, 0xE0, 0x32, 0x01, 0x90, 0x03, 0x20, 0x00, 0x00];
        assert_eq!(parse_dpi_list(&bytes), vec![100, 150, 200, 250, 300, 350, 400, 800]);
    }

    #[test]
    fn dpi_list_stops_on_malformed_range() {
        // A range marker with no previous value must not panic.
        assert!(parse_dpi_list(&[0xE0, 0x32, 0x01, 0x90]).is_empty());
    }

    #[test]
    fn breathe_params_layout() {
        let p = effect_params(&EffectSetting {
            effect: Effect::Breathe,
            color: Rgb(1, 2, 3),
            period_ms: 0x1234,
            brightness: 50,
        });
        assert_eq!(p, [1, 2, 3, 0x12, 0x34, 0, 50, 0, 0, 0]);
    }

    #[test]
    fn cycle_params_layout() {
        let p = effect_params(&EffectSetting { effect: Effect::Cycle, period_ms: 5000, brightness: 100, ..Default::default() });
        assert_eq!(p, [0, 0, 0, 0, 0, 0x13, 0x88, 100, 0, 0]);
    }
}
