# Devices

For each device: how it is reached, what the hub does, **what was verified on real hardware**, and
what is not known. All verification so far was done on one Linux machine (Arch, kernel 6.18, BlueZ).
Nothing has been run on Windows or macOS.

Legend: **verified** = observed working on hardware; **sent** = the device accepted the command
without error, but the effect was not confirmed independently; **untested** = implemented from
documentation only.

## Logitech G502 X Plus (Lightspeed receiver `046d:c547`)

HID++ 2.0 long reports (`0x11`, 20 bytes) on the receiver's vendor interface (usage page
`0xFF00`, usage `0x02`), device index 1..6 for paired mice. Feature indexes are looked up at runtime
(`IRoot.GetFeature`). The mouse sleeps when idle, so requests are retried and the mouse may only
appear once it is moved.

| Feature | HID++ feature | Status |
|---|---|---|
| Battery | `0x1004` unified battery, fallback `0x1000` | verified |
| DPI | `0x2201`, list in function 1 (16-bit values, `0xE000|step` = range), get 2, set 3 | verified (set 1600, read back, restored) |
| Polling rate | `0x8060`, list 0, get 1, set 2 (ms) | verified, **only in host mode** |
| Onboard/host switch | `0x8100`, get mode 2, set mode 1 (1 = onboard, 2 = host) | verified (toggled and restored) |
| Lighting | `0x8071` RGB Effects: static (id 1), breathe (`0x0A`), cycle (3), off (0). Takes software control first (function 5), then function 1 with the effect index and 10 parameter bytes, volatile | static colour confirmed by the user; the other effects were accepted (sent) |

Notes and caveats:

* While the mouse runs its own stored profile (onboard mode, the default) it refuses polling-rate
  changes (HID++ error `0x02`). DPI changes are accepted in both modes. Host mode pauses the stored
  profile, so button actions defined by the official app may stop working while it is on: not
  checked.
* The current lighting effect cannot be read back, so the UI starts from "static white".
* Lighting is volatile; it may revert when the mouse sleeps or is power-cycled.
* Bluetooth and USB-cable modes of this mouse are untested (Bluetooth devices are skipped, see
  below), and button remapping is not implemented.
* Some non-Logitech Bluetooth devices reuse Logitech's vendor id (the RK S85 keyboard does), so the
  driver never sends HID++ to Bluetooth devices.

## Sony DualShock 4 (`054c:05c4`, `054c:09cc`)

Raw HID. USB: output report `0x05` (32 bytes). Bluetooth: output report `0x11` (78 bytes) with a
CRC32 (seed `0xA2`) in the last four bytes; the first read of feature report `0x02` switches the pad
to full input reports. Rumble and lightbar are in the same report, so the driver keeps both states.

| Feature | Status |
|---|---|
| Lightbar colour (Bluetooth) | verified (confirmed by the user) |
| Battery (Bluetooth, input report `0x11` byte 32) | verified (matches the kernel's reading) |
| Rumble (strong/weak) | sent; the user answered "so far so good" after trying it, not confirmed in detail |
| USB connection, Sony USB wireless adapter (`0x0BA0`, shown as 2.4 GHz) | untested |

## Xbox One / Series controller (Bluetooth, `045e:0b13` tested)

Output report `0x03` (9 bytes: enable mask, left trigger, right trigger, strong, weak, sustain,
release, loop), layout from xpadneo. The tested controller's HID descriptor has **no battery
report**; the battery comes from the Bluetooth Battery Service (GATT) through the OS: BlueZ
(`bluetoothctl info`) on Linux. A HID battery report (`0x04`) is decoded as a fallback for
controllers that send one (untested).

| Feature | Status |
|---|---|
| Battery | verified on Linux (75%, same as `bluetoothctl`); **not available on Windows/macOS yet** |
| Rumble: left grip, right grip, both triggers | sent; the user answered "so far so good" after trying it, the motor-by-motor mapping was not confirmed individually |
| USB cable | not covered: the OS driver handles it and exposes no HID node |
| Xbox Wireless Adapter (2.4 GHz) | not covered |

Needs read/write access to the hidraw node; without the udev rules the pad is listed with a
"Cannot control this device" note.

## Steam Controller (2026) through the Steam Controller Puck (`28de:1304`)

The Puck exposes four controller slots as HID interfaces 2..5 and a vendor interface 6. A connected
controller streams reports on its slot's interface (layout from SDL):

* `0x42`/`0x45`/`0x47`: controller state. A slot is considered occupied when these arrive.
* `0x43`: battery `[charge state, level %]` (charge state 1 = discharging, 2 = charging). It is sent
  only occasionally, so the first reading can take a few seconds.
* Output `0x83`: tone `[side, gain dB, frequency Hz, duration ms, LFO frequency, LFO depth]`.
  Output `0x80`: plain rumble (not used yet; the hardware stops it after ~50 ms without refresh).

| Feature | Status |
|---|---|
| Detection of the controller in a slot | verified (slot 1) |
| Battery | verified (96%) |
| Haptic tones, two voices | verified: the user felt both; which voice is left/right is **not known** |
| MIDI playback | sent; works through the tone report |
| Wired (`28de:1302`) and Bluetooth (`28de:1303`) controllers; other slots | untested / not implemented |

Steam itself claims the controller when it runs; behaviour with Steam open was not checked.

## RK S85 keyboard

* **USB cable** (`258a:01fa`, `258a:0193`): the full open-s85 feature set (key mapping, lighting,
  screen/GIF upload, clock, profiles) in embedded pages. Unchanged from open-s85; not re-tested
  inside the hub on this machine because the keyboard was not wired.
* **2.4 GHz dongle** (`258a:0150`, "Gaming KB"): only listed. A read-only configuration request
  through the dongle was rejected (`Broken pipe`), so it does not forward the configuration
  protocol. The dongle stays enumerated when plugged in, so the hub cannot tell whether the
  keyboard is actually using it.
* **Bluetooth** (shows as `RK-S85`, vendor id `046d`): listed; battery from the Bluetooth stack
  (Linux). Nothing is sent to it.

## Other HID gamepads and the GameCube adapter

Any device that announces the gamepad/joystick usage (page `0x01`, usage `0x04`/`0x05`) is listed
with its name, serial and connection type. The GameCube USB adapter (`057e:0337`) is listed as an
adapter; its four ports are not read. No battery, lighting or rumble. Untested except that Xbox
Series pads were correctly listed before their own driver existed.

## Music through haptics

`hub-midi` reads a Standard MIDI file (tempo changes handled, percussion channel skipped) and plans:

* **Tone devices** (Steam Controller): each note becomes a tone on one of two voices. Frequencies are
  folded by octaves into 100-600 Hz. A third simultaneous note cuts off the voice that started
  earliest.
* **Motor devices** (DualShock 4, Xbox): the song's pitch range is split into one band per motor;
  each note switches its motor on at a strength set by its velocity. This is a rhythm, not a melody.

Timing is done by the manager thread; hardware scans are paused during playback.

## Known problems

* First scan is slow: the Steam Controller waits up to ~4 s for its first battery report and the
  Logitech probe waits for the mouse to wake. The CLI `hub battery` can take ~12 s.
* A controller that goes quiet for 10 s is treated as gone (Steam) and rescanned.
* Windows and macOS: untested, and several parts are Linux-only for now (Bluetooth battery,
  udev rules).
