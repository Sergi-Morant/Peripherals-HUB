# Peripherals HUB

One desktop app for the peripherals on your desk: keyboard, mouse and game controllers. Instead
of one vendor app per device, you get a single window with a sidebar of whatever is connected right
now: plug something in or pair it and it shows up, unplug it and it goes away. Controllers of the
same kind can be connected in any number, and each device shows how it is connected (USB,
Bluetooth or 2.4 GHz).

> **Status: early.** It was developed and tested on one Linux machine with the devices listed
> below. The Windows and macOS builds are produced by CI but have **not been run yet**. Nothing here
> is affiliated with or endorsed by any hardware vendor, and it talks to hardware directly, so use it
> at your own risk.

It grew out of [open-s85](https://github.com/Sergi-Morant/open-s85), a configuration app for the
Royal Kludge RK S85 keyboard, whose code is embedded here unchanged.

## What works

"Verified" means it was run against real hardware on Linux. Details, protocols and caveats are in
[docs/DEVICES.md](docs/DEVICES.md).

| Device | Connection | Battery | Lighting | Other |
|---|---|---|---|---|
| Logitech G502 X Plus | 2.4 GHz (Lightspeed receiver) | verified | effects: static, breathe, colour cycle, off | DPI, polling rate, onboard/host switch, button remapping in the onboard profiles (command line; verified) |
| Sony DualShock 4 | Bluetooth | verified | lightbar colour (verified) | rumble, built-in speaker (test tone verified) |
| Xbox Series controller | Bluetooth | verified (via the OS Bluetooth stack, Linux only) | – | rumble incl. triggers |
| Steam Controller | 2.4 GHz (Steam Controller Puck) | verified | – | haptic tones (verified), MIDI playback |
| RK S85 keyboard | USB cable | – | everything open-s85 does | key mapping, screen, profiles |
| RK S85 keyboard | Bluetooth / 2.4 GHz dongle | Bluetooth: Linux only | – | listed only: configuration needs the USB cable |
| Other HID gamepads, GameCube adapter | any | – | – | listed with their connection type only |

Music: pick a `.mid` file and the haptics play it. The Steam Controller plays real notes on two
voices; DualShock 4 and Xbox motors cannot change pitch, so the song becomes a rhythm there.

Not done yet (see [docs/ROADMAP.md](docs/ROADMAP.md)): a GUI for mouse button remapping (the command line works),
controller button remapping, Steam Controller rumble and wired/Bluetooth modes, Xbox over USB or
the 2.4 GHz adapter, GameCube adapter ports, remembered colours, Windows/macOS battery sources.

## Install

Builds for every push are made by GitHub Actions (Actions tab → latest run → Artifacts), and tagged
versions appear under Releases:

| Platform | Download |
|---|---|
| Windows | `…-windows-x86_64-setup.exe` (installer) or `…-windows-x86_64.zip` |
| macOS (Apple Silicon and Intel) | `…-macos-universal.dmg` or `…-macos-universal.tar.gz` |
| Linux | `…-linux-x86_64.tar.gz` (contains `install.sh`) |

The installers are not code-signed, so the first launch shows a warning:

* **Windows:** "Windows protected your PC" → **More info** → **Run anyway**.
* **macOS:** "Apple could not verify…" → **System Settings → Privacy & Security** → **Open Anyway**,
  and allow **Input Monitoring** if asked.

### Linux

Run `./install.sh` from the extracted archive (adds the app to your menu), then give your user
access to the devices once and reconnect them:

```sh
sudo cp packaging/linux/70-hub.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

Without these rules the devices are listed but cannot be opened ("Permission denied").

**What the rules allow:** they give the logged-in desktop user raw (`hidraw`) access to every HID
device made by Logitech, Sony, Microsoft, Valve, Nintendo and SINOWEALTH (RK), which includes their
keyboards and mice, not only the supported models. That is the usual approach for tools like this,
but any program you run could then read or send raw reports to those devices. If you do not want
that, edit the copy in `/etc/udev/rules.d/` and replace the vendor match with `ATTRS{idProduct}`
lines for just the devices you own (`hub list` shows the ids). Bluetooth
battery levels of Xbox controllers and the RK S85 keyboard are read with `bluetoothctl`
(BlueZ). For the keyboard's configuration pages connect it by USB cable and close the official RK
app.

## Build from source

```sh
cargo build --release
# GUI:  target/release/peripherals-hub
# CLI:  target/release/hub        (keyboard CLI: target/release/s85)
```

Linux build dependencies (Debian/Ubuntu): `libudev-dev libgtk-3-dev libxkbcommon-dev libwayland-dev`.
Tests and lints: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
Installers: see `.github/workflows/ci.yml` (uses [cargo-packager](https://github.com/crabnebula-dev/cargo-packager)).

## Command line

```sh
hub list                                   # devices, connection type, capabilities
hub battery
hub color ds4 ff0000                       # lightbar / static colour (--all for every match)
hub effect g502 breathe 0000ff --period 3000
hub dpi g502 1600                          # show with no value
hub rate g502 500                          # polling rate in Hz (needs host mode, see DEVICES.md)
hub mode g502 host|onboard|show
hub backup g502 mouse.profiles             # always do this first
hub profiles g502                          # profiles and what every button does
hub button g502 1 6 mouse:back             # profile 1, button 6 (G5); also key:04+ctrl, media:CD, fn:cycle-dpi, none
hub profile g502 1                         # run profile 1
hub restore g502 mouse.profiles
hub rumble xbox --strong 60 --right 40 --ms 600
hub tone steam --voice 0 --hz 220
hub play steam examples/ode-to-joy.mid
hub play ds4 examples/ode-to-joy.mid --speaker   # as sound on the DualShock 4 speaker
hub speaker ds4 song.wav --volume 80         # or --tone 1500
hub raw | hub probe | hub steam | hub sniff 045e:0b13   # development helpers
```

A device is selected by a case-insensitive part of its name or id.

## Documentation

* [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): crates, traits, threads, how to add a device
* [docs/DEVICES.md](docs/DEVICES.md): per-device protocols, what was verified, known problems
* [docs/ROADMAP.md](docs/ROADMAP.md): what is next
* [docs/rk-s85/](docs/rk-s85/): the keyboard protocol notes inherited from open-s85
* [CHANGELOG.md](CHANGELOG.md)

## Credits and licence

Licensed under the GNU GPL v3 (see [LICENSE](LICENSE)). The keyboard code inherited from open-s85
was published there under the MIT licence.

Protocol knowledge comes from reading open-source projects, used as references rather than copied:
[Solaar](https://github.com/pwr-Solaar/Solaar) and [OpenLogi](https://github.com/AprilNEA/OpenLogi)
(Logitech HID++ 2.0), [xpadneo](https://github.com/atar-axis/xpadneo) (Xbox over Bluetooth),
[SDL](https://github.com/libsdl-org/SDL) (Steam Controller reports and haptics), and the Linux
`hid-sony`/`hid-playstation` drivers (DualShock 4). [SteamlessController](https://github.com/ddeverill/SteamlessController)
is a pointer for the Windows side of the Steam Controller. The icon font is
[Phosphor](https://phosphoricons.com) (MIT).
