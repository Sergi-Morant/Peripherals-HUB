//! Lists every game controller that identifies itself as a HID gamepad (usage page 0x01, usage
//! 0x04/0x05) and the Nintendo GameCube USB adapter, with how each one is connected. Nothing is
//! sent to these devices yet: no battery, no lighting. Sony DualShock 4 pads are left to
//! `drv-ds4`.
//!
//! One entry per HID node, so any number of identical controllers can be connected at once; the
//! Bluetooth address / USB serial is kept to tell them apart.

use hidapi::{DeviceInfo, HidApi};
use hub_core::{transport_of, Capability, Driver, Info, Kind, Peripheral, Transport};

const SONY: u16 = 0x054C;
const MICROSOFT: u16 = 0x045E;
const NINTENDO: u16 = 0x057E;
const VALVE: u16 = 0x28DE;
const GAMECUBE_ADAPTER: (u16, u16) = (NINTENDO, 0x0337);
/// DualShock 4 PIDs, handled by `drv-ds4`.
const DS4_PIDS: [u16; 3] = [0x05C4, 0x09CC, 0x0BA0];

const PAGE_GENERIC_DESKTOP: u16 = 0x01;
const USAGE_JOYSTICK: u16 = 0x04;
const USAGE_GAMEPAD: u16 = 0x05;

/// A device that is only listed.
pub struct Listed {
    info: Info,
}

impl Peripheral for Listed {
    fn info(&self) -> &Info {
        &self.info
    }

    fn capabilities(&self) -> Vec<Capability> {
        vec![]
    }
}

pub struct GamepadDriver;

fn friendly_name(d: &DeviceInfo) -> String {
    let product = d.product_string().unwrap_or("").trim();
    match (d.vendor_id(), product) {
        (MICROSOFT, p) if p.is_empty() || p.eq_ignore_ascii_case("controller") => "Xbox controller".into(),
        (VALVE, "") => "Steam Controller".into(),
        // Generic Bluetooth name used by Sony, 8BitDo, ... when nothing better is reported.
        (_, "") => format!("Game controller {:04x}:{:04x}", d.vendor_id(), d.product_id()),
        (_, p) => p.into(),
    }
}

impl Driver for GamepadDriver {
    fn name(&self) -> &'static str {
        "Game controllers"
    }

    fn discover(&self, api: &HidApi) -> Vec<Box<dyn Peripheral>> {
        let mut out: Vec<Box<dyn Peripheral>> = Vec::new();
        for d in api.device_list() {
            let (vid, pid) = (d.vendor_id(), d.product_id());
            let id_base = format!("pad:{vid:04x}:{pid:04x}:{}", d.path().to_string_lossy());
            let serial = d.serial_number().filter(|s| !s.is_empty()).map(String::from);
            if (vid, pid) == GAMECUBE_ADAPTER {
                // The adapter exposes one interface; its four ports are not read yet.
                if d.interface_number() <= 0 {
                    out.push(Box::new(Listed {
                        info: Info {
                            id: id_base,
                            name: "GameCube controller adapter".into(),
                            kind: Kind::Adapter,
                            transport: Transport::Usb,
                            serial,
                        },
                    }));
                }
                continue;
            }
            let is_pad = d.usage_page() == PAGE_GENERIC_DESKTOP
                && (d.usage() == USAGE_GAMEPAD || d.usage() == USAGE_JOYSTICK);
            // Bluetooth Xbox pads have their own driver (drv-xbox).
            let xbox_bt = vid == MICROSOFT && transport_of(d) == Transport::Bluetooth;
            if !is_pad || xbox_bt || (vid == SONY && DS4_PIDS.contains(&pid)) {
                continue;
            }
            out.push(Box::new(Listed {
                info: Info { id: id_base, name: friendly_name(d), kind: Kind::Gamepad, transport: transport_of(d), serial },
            }));
        }
        out
    }
}
