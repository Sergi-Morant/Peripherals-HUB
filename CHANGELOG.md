# Changelog

## Unreleased

* **G502 X Plus onboard profiles** (command line): `hub profiles`, `hub button`, `hub profile`,
  `hub backup`, `hub restore`. One button was remapped on the real mouse and worked; the factory
  ROM profiles are copied to flash on the first edit (see `docs/DEVICES.md`).
* **DualShock 4 speaker** (Bluetooth): `hub speaker ds4 file.wav` / `--tone HZ` and
  `hub play ds4 song.mid --speaker`, with a built-in SBC encoder. Test tone verified on hardware.
* Example MIDI replaced by a public-domain tune (`examples/ode-to-joy.mid`); the README explains how
  far the Linux udev rules reach.

## 0.1.0 — 2026-10-22

First version of the Peripherals HUB: one app for several devices, built on top of
[open-s85](https://github.com/Sergi-Morant/open-s85). **Early and only partly tested**: see
[docs/DEVICES.md](docs/DEVICES.md) for exactly what was checked on real hardware (Linux only so
far; the Windows and macOS builds have not been run yet).

* **Hub window** with a sidebar of whatever is connected right now. Devices appear and disappear
  on their own; any number of identical controllers can be connected at once and are told apart by
  their Bluetooth address / serial. Each device shows how it is connected: USB, Bluetooth or
  2.4 GHz.
* **Logitech G502 X Plus** (through the Lightspeed receiver): battery, DPI, polling rate, lighting
  effects (static, breathe, colour cycle, off) and the onboard/host switch.
* **DualShock 4** (Bluetooth): battery, lightbar colour, rumble.
* **Xbox Series controller** (Bluetooth): battery (from the Bluetooth stack), rumble including the
  triggers.
* **Steam Controller** (through the Steam Controller Puck): battery, haptic tones.
* **MIDI playback** through the haptics: real notes on the Steam Controller, a rhythm on the DS4
  and Xbox motors.
* **RK S85 keyboard**: the open-s85 pages are embedded unchanged for the wired keyboard. Over
  Bluetooth or the 2.4 GHz dongle the keyboard is only listed (Bluetooth also shows its battery);
  configuring it needs the USB cable.
* Other HID gamepads and the GameCube adapter are listed with their connection type; nothing is
  sent to them yet.
* New command-line tool `hub` (list, battery, colour, effects, DPI, rate, rumble, tone, MIDI play,
  and a few development helpers).
* One udev rules file for Linux (`70-hub.rules`) covering all supported devices.
* Builds for Linux, Windows and macOS are produced by CI; the Windows and macOS ones are untested.

## Before the hub: open-s85

The history below is from the standalone keyboard app this project started from.

## 0.2.2 — 2026-10-21

Linux fixes. The app is otherwise unchanged.

* **Linux:** the udev rule is now `70-rk-s85.rules` (was `99-rk-s85.rules`). systemd grants
  `uaccess` permissions in `73-seat-late.rules`, so the old file's tag came too late and the app
  failed with "Permission denied" unless run as root. If you installed the old rule, delete it
  and install the new one (see README).
* **Linux:** the app icon now appears in the applications menu, taskbar and window title, like on
  Windows and macOS. The download includes the icon, a `.desktop` entry and an `install.sh` that
  sets them up for the current user. The window now reports the app ID
  `io.github.sergi-morant.open-s85`, which Wayland desktops use to find the icon.

## 0.2.1 — 2026-10-21

Installers and an app icon. The app itself works the same as 0.2.0.

* **App icon** based on the S85: cream case, lavender and navy keys, the clock screen, the silver
  knob and the RGB glow. Shown in the window, the taskbar/Dock and on the Windows `.exe`.
* **macOS:** `Open S85.app` in a `.dmg` (drag it to Applications). Universal build that runs on
  Apple Silicon and Intel Macs, as is the `.tar.gz` download.
* **Windows:** a setup `.exe` that installs for the current user (no administrator rights needed),
  adds Start menu and desktop shortcuts, and appears in "Add or remove programs" with an uninstaller.
* The `.zip`/`.tar.gz` downloads are still available for those who prefer no installer.

## 0.2.0 — 2026-10-21

A redesigned desktop app. The keyboard protocol and the command-line tool are unchanged.

### New look
* Dark theme with icons ([Phosphor](https://phosphoricons.com), MIT) throughout.
* Every page and setting has a title, an icon and a one-line explanation; buttons have tooltips.
* Sidebar with a subtitle per page and a connection card (status, device name, Reconnect).
* Status bar shows the last action or error and the active profile.

### Keys
* Layers (Default, FN1, FN2, Tap) are tabs, each with a short explanation.
* Legend for selected, customised and empty keys.
* Card for the selected key with "Factory" (restore that key) and deselect buttons.
* Search box for the function picker (e.g. "volume", "F5").

### Lighting
* Effects are tiles with an icon and a description each.
* Brightness, speed, colour and wireless sleep each have their own card.
* Colour panel like the official app: hue ring (click or drag), R/G/B sliders with number boxes,
  an editable hex code, and eight presets (red, orange, yellow, green, blue, cyan, magenta, white).
* Key legends in the preview stay readable on bright colours.

### Screen
* Drag and drop an image or GIF onto the window, or click the preview to browse.
* Animated GIF preview inside a mock-up of the screen, with play/pause.
* Shows frame count, delay per frame and estimated upload time.
* Fit options (Fill, Fit, Stretch) are tiles explaining what each one does.
* Frame delay override is a slider (10–1000 ms).

### Settings
* "Restore factory key mapping" now asks for confirmation.
* Shows where profiles are stored, and links to the project on GitHub.

## 0.1.0 — 2026-10-20

First release: key mapping on all four layers, the 17 lighting effects with brightness, speed,
colour and colour mixing, wireless sleep time, screen upload of images and GIFs, clock sync,
local profiles, and the `s85` command-line tool.
