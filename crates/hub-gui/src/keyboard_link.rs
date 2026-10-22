//! Lists the RK S85 when it is connected in a way the hub cannot configure: over Bluetooth, or
//! through its 2.4 GHz dongle. Nothing is sent to it (over Bluetooth only its battery is read from the
//! system's Bluetooth stack); open-s85's protocol only works over USB cable.

use std::collections::HashSet;

use hub_core::{transport_of, Battery, Capability, Driver, HidApi, Info, Kind, Peripheral, Result, Transport};

/// The 2.4 GHz dongle ("Gaming KB"). It stays enumerated while plugged in, whether or not the
/// keyboard is switched to it, so only the dongle's presence can be reported.
const DONGLE: (u16, u16) = (s85_protocol::VID, 0x0150);
const BLUETOOTH_NAME: &str = "RK-S85";

struct Listed(Info);

impl Peripheral for Listed {
    fn info(&self) -> &Info {
        &self.0
    }

    fn capabilities(&self) -> Vec<Capability> {
        // Over Bluetooth the keyboard's battery is available from the system's Bluetooth stack.
        if self.0.transport == Transport::Bluetooth && self.0.serial.is_some() {
            vec![Capability::Battery]
        } else {
            vec![]
        }
    }

    fn battery(&mut self) -> Result<Option<Battery>> {
        let p = self.0.serial.as_deref().and_then(hub_core::bluetooth::battery_percent);
        Ok(p.map(|p| Battery { percent: Some(p), charging: false, coarse: false }))
    }
}

pub struct KeyboardLinkDriver;

impl Driver for KeyboardLinkDriver {
    fn name(&self) -> &'static str {
        "RK S85 (wireless link)"
    }

    fn discover(&self, api: &HidApi) -> Vec<Box<dyn Peripheral>> {
        let mut seen = HashSet::new();
        let mut out: Vec<Box<dyn Peripheral>> = Vec::new();
        for d in api.device_list() {
            let transport = if (d.vendor_id(), d.product_id()) == DONGLE {
                Transport::Dongle
            } else if transport_of(d) == Transport::Bluetooth
                && d.product_string().is_some_and(|p| p.starts_with(BLUETOOTH_NAME))
            {
                Transport::Bluetooth
            } else {
                continue;
            };
            // Several HID interfaces belong to one keyboard.
            let key = match transport {
                Transport::Bluetooth => d.path().to_string_lossy().into_owned(),
                _ => format!("{:04x}:{:04x}", d.vendor_id(), d.product_id()),
            };
            if !seen.insert(key.clone()) {
                continue;
            }
            out.push(Box::new(Listed(Info {
                id: format!("kbd-link:{key}"),
                name: "RK S85".into(),
                kind: Kind::Keyboard,
                transport,
                serial: d.serial_number().filter(|s| !s.is_empty()).map(String::from),
            })));
        }
        out
    }
}
