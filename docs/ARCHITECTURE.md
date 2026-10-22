# Architecture

A Cargo workspace. Drivers know how to talk to one kind of hardware; the hub core defines what every
device looks like; the GUI and CLI only use that common shape.

```
crates/
  hub-core      device model and driver traits (no UI)
  hub-midi      MIDI file -> vibration schedule
  drv-ds4       DualShock 4 (USB / Bluetooth)
  drv-xbox      Xbox One / Series over Bluetooth
  drv-steam     Steam Controller behind a Steam Controller Puck
  drv-g502x     Logitech G502 X (HID++ 2.0 through a Lightspeed receiver)
  drv-gamepads  detection-only listing of other HID gamepads and the GameCube adapter
  s85-protocol  RK S85 keyboard protocol (from open-s85, unchanged)
  s85-cli       RK S85 command-line tool (from open-s85, unchanged)
  hub-cli       the `hub` command-line tool
  hub-gui       the desktop app (eframe/egui)
```

## `hub-core`

* `Peripheral`: one connected device. `info()` (name, kind, transport, serial), `capabilities()`,
  and optional operations that default to "unsupported": `battery`, `set_rgb`, `set_effect`,
  `settings`/`set_dpi`/`set_report_rate`/`set_host_mode`, `rumble`, `tone`, plus `tick()` (drain
  streaming reports) and `problem()` (listed but not controllable, e.g. permissions).
* `Capability`: what the device can do (`Battery`, `Rgb`, `Effects`, `Dpi`, `ReportRate`,
  `HostMode`, `Rumble`, `Tones`). **The UI draws its cards from the capability list**, so adding a
  device does not need device-specific UI code.
* `Driver::discover(&HidApi) -> Vec<Box<dyn Peripheral>>`: open every device of this kind that is
  connected. `Hub` holds the registered drivers and scans them together.
* `Transport` (`Usb`, `Bluetooth`, `Dongle` = 2.4 GHz) and `transport_of(&DeviceInfo)`: the
  connection type from the HID bus, with a path heuristic on Windows.
* `shared_api()`: the process-wide `HidApi`. hidapi allows only one instance per process, so the
  scanner and the keyboard worker share this one behind a mutex.
* `bluetooth::battery_percent(mac)`: battery from the OS Bluetooth stack (BlueZ via `bluetoothctl`
  on Linux, nothing elsewhere yet).

## Threads in the GUI

* **UI thread** (`hub-gui/src/hub.rs`, `hub/cards.rs`): draws the sidebar and the page of the
  selected device. It never talks to hardware.
* **Manager thread** (`manager.rs`): owns every non-keyboard `Peripheral`. It rescans when the set of
  HID nodes of the watched vendors changes (checked every 2 s), polls batteries every 30 s,
  applies commands from the UI, runs timed rumble stops and MIDI playback, and sends `Update`
  messages back. Queued commands for the same control are collapsed (dragging a slider only
  sends the last value). Hardware scans are paused while music plays so they cannot disturb the
  timing.
* **Keyboard worker** (`worker.rs`, from open-s85): owns the wired RK S85. The manager only
  detects that the keyboard appeared or disappeared and tells it to (re)connect.

Identical devices: each gets an id from its HID path; the sidebar names are made unique with the
serial (a Bluetooth address, for pads) so "DualShock 4 1" stays the same pad across replugs.

## Adding a device

1. New crate `drv-<name>` implementing `Driver` and `Peripheral`; fill `capabilities()` honestly
   with only what was verified.
2. Register it in `hub-gui/src/manager.rs` and `hub-cli/src/main.rs`, and add its vendor id to
   `WATCHED_VIDS` in the manager so hotplug triggers a rescan.
3. Add its VID/PID to `packaging/linux/70-hub.rules`.
4. If it needs a control the UI does not have yet, add a `Capability` variant, a `Peripheral`
   method and a card in `hub-gui/src/hub/cards.rs`.
5. Add unit tests for report encoding/decoding (see `drv-ds4`, `drv-xbox`, `drv-g502x`).
6. Record what was verified on real hardware in `docs/DEVICES.md`.

A useful workflow for unknown hardware: `hub raw` (HID interfaces), `hub sniff VID:PID` (report ids
and samples), then read the open-source driver for that device.
