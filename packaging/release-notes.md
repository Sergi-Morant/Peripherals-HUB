
### Downloads
| Platform | Installer | Plain archive |
|---|---|---|
| Windows x86-64 | `peripherals-hub-VERSION-windows-x86_64-setup.exe` | `peripherals-hub-VERSION-windows-x86_64.zip` |
| macOS (Apple Silicon + Intel) | `peripherals-hub-VERSION-macos-universal.dmg` | `peripherals-hub-VERSION-macos-universal.tar.gz` |
| Linux x86-64 (glibc ≥ 2.39, e.g. Ubuntu 24.04+) | `install.sh` inside the archive | `peripherals-hub-VERSION-linux-x86_64.tar.gz` |

The archives contain `peripherals-hub` (desktop app), `hub` (command-line tool), `s85` (keyboard
command-line tool), README, CHANGELOG and LICENSE; the Linux one also has the udev rules, the app
icon, a `.desktop` entry and `install.sh`.

### Notes
- **Only tested on Linux so far.** The Windows and macOS builds come straight from CI and have not
  been run; please report what works. See `docs/DEVICES.md` for what each device can do.
- The installers are **not code-signed**, so the first launch shows a warning:
  - **Windows:** "Windows protected your PC" → **More info** → **Run anyway**.
  - **macOS:** "Apple could not verify…" → **System Settings → Privacy & Security** → **Open Anyway**. Allow **Input Monitoring** if asked.
- **Linux:** install `70-hub.rules` (see README) so the app can reach your devices without root.
- Battery of Bluetooth Xbox controllers and keyboards is read through BlueZ, so it is **Linux only** for now.
- To configure the RK S85 keyboard, connect it **by USB cable** and close the official RK app first.

Not affiliated with or endorsed by Royal Kludge, Logitech, Sony, Microsoft, Valve or Nintendo.
