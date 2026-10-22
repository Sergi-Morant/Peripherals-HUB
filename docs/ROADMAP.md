# Roadmap

Roughly in the order I intend to work on them. Items marked (needs hardware) cannot be finished
without the device being connected.

## Next

1. **Mouse buttons and profiles (G502 X Plus).** The official app lets you remap every button and
   edit onboard profiles. Implement HID++ `0x8100` onboard profiles (read/write the profile memory
   sectors) and/or `0x1B04` reprogrammable controls; read and compare before ever writing; export a
   backup of the profile first.
2. **Windows and macOS first run.** Test the CI builds, fix what breaks, add the missing battery
   sources (Xbox via XInput/WinRT BLE battery, macOS IOKit) and Windows permissions notes.
3. **Remembered settings.** Store each device's colour/effect/DPI per serial and re-apply it when
   the device reconnects (DualShock 4 lightbars reset on reconnect, the mouse lighting is volatile).
4. **Steam Controller:** plain rumble (needs a refresh every ~40 ms), wired (`0x1302`) and Bluetooth
   (`0x1303`) modes, multiple controllers on one Puck (needs hardware), find out which voice is
   which side.

## Later

* **Controller button remapping.** The pads cannot store remaps, so this means a virtual gamepad:
  Linux uinput first, Windows through ViGEmBus (see
  [SteamlessController](https://github.com/ddeverill/SteamlessController) for a working Windows
  approach), macOS TBD. Per-game profiles.
* **Xbox over USB and the Xbox Wireless Adapter**, **GameCube adapter** ports (init `0x13`, read
  input, rumble `0x11`; needs the adapter), Switch pads.
* **More mouse features:** Bluetooth and cable modes, per-profile lighting, battery warnings.
* **DualShock 4 over USB** and the Sony wireless adapter (needs hardware), DualSense.
* **Keyboard:** the open-s85 protocol does not work through the 2.4 GHz dongle or Bluetooth;
  investigate whether those modes expose any configuration channel.
* **Music:** better voice allocation, a trigger-motor band for Xbox, a progress bar, loop.
* Event-based hotplug instead of 2-second polling; smaller first-scan latency.
* Proper icon and app name artwork (the icon is still open-s85's), code signing.
* CI: formatting check, integration tests with fake devices.
