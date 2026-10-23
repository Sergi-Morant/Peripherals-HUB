# Handoff: continuing on macOS

State on 2026-10-23, written to pick the work up on another machine. Everything below was developed
and tested on Linux (Arch) with the author's own devices; nothing has been run on macOS or Windows
by the code's author yet except the first impressions listed under "Reported on macOS".

## Build and run

```sh
cargo build --release                  # needs Rust stable; on macOS no extra libraries
target/release/peripherals-hub         # the GUI
target/release/hub list                # devices, connection, capabilities
target/release/hub raw                 # EVERY HID interface of every device: vid:pid, interface number,
                                       # usage page/usage, path. Paste this when reporting a problem.
target/release/hub sniff 28de:1304     # which report ids each Steam Puck slot sends
cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
```

macOS asks for **Input Monitoring** permission the first time (System Settings, Privacy & Security).
Read `README.md`, `docs/ARCHITECTURE.md` (crates, traits, how to add a driver), `docs/DEVICES.md`
(protocols and what was verified on hardware) and `docs/ROADMAP.md` first.

## Reported on macOS

1. **Steam Controller via the Puck: "completely overwrites the trackpad, and it is detected as a
   keyboard".** Explained, **not a hub bug**: the Puck announces itself to macOS as a mouse and a
   keyboard (its desktop emulation, see "Findings on macOS" below), and macOS then switches off the
   laptop's built-in trackpad. It happens with the hub closed, and with `hub sniff` running it made
   no difference. The Steam Controller's own trackpad does move the cursor. To check: System
   Settings, Accessibility, Pointer Control, Alternative control methods, "Ignore built-in trackpad
   when mouse or wireless trackpad is present" (if it is on, turn it off). Whether that setting is
   the cause was **not confirmed yet**. Left in place, harmless: `hub_core::new_api()` opens
   devices non-exclusively on macOS.
   * Open defect: the `drv-steam` filter that skips the keyboard and mouse usages does not work on
     macOS, because all collections of an interface share one path (hidapi lists the same path
     four times, once per usage), so the interface is opened anyway.
   * `hub sniff` fails on macOS with `IOHIDDeviceGetReport failed` for report `0x04`/`0x01`
     (development helper only).
2. **DualShock 4: the MIDI player used only the rumble motors.** Fixed on Linux. On macOS the
   speaker stuttered because Bluetooth writes block for about 30 ms; see "Findings on macOS" and
   the "macOS" paragraph in `docs/DEVICES.md`. It was not report padding.

## Findings on macOS (2026-10-23, Steam Controller Puck + DualShock 4 v2 over Bluetooth)

* `hub list`, `hub battery`, lightbar colour and the Steam tone work. The Puck's interfaces report
  `interface_number()` 2..=5 correctly (it is not -1), and each exposes usages `0001/0002`,
  `0001/0001`, `0001/0006` and `ff00/0001`: that is the controller's own keyboard/mouse emulation
  ("lizard mode"), which is what macOS sees as a keyboard. The hub does not list it as one. The
  tone did not move the cursor. The GUI lists the controller, its tones work, and it does not
  change anything about the trackpad compared with the hub closed.
* DS4 speaker: the stutter was throughput, not report padding (see "macOS" in `docs/DEVICES.md`).
  Fixed in part with `0x17` reports; still not as clean as on Linux. `--both` is untested on macOS.

## Changes since 0.1.0 (all local to this branch, none released)

* DS4 speaker over Bluetooth (`crates/drv-ds4/src/sbc.rs` is a small SBC encoder), speaker card and
  haptic-tone card in the GUI, G502 X Plus onboard profiles and button remapping (CLI and GUI card),
  public-domain example MIDI, udev note.
* **MIDI on the DualShock 4 can play through the speaker, the motors or both**: the "Play music"
  card has "Speaker" and "Vibration" ticks (vibration off by default because it makes the speaker
  crackle), `hub play ds4 song.mid --speaker|--both`. There is one music card per device with Play,
  Pause/Resume, Stop and a seek bar (pause and seek on the DS4 are done inside the audio stream:
  `AudioControl` in `hub-core`).
  The rumble levels travel inside the audio reports (`Peripheral::play_audio` takes
  `AudioOptions` with a `rumble(ms)` callback), so the two stay in step. The speaker part was
  confirmed by the author on hardware; **speaker plus motors together: see the status in
  `docs/DEVICES.md`**.
* `CHANGELOG.md` has a `0.1.1` section and the workspace version is 0.1.1, but **`v0.1.1` is not
  published**. A tag `v0.1.1` may exist locally on the Linux machine only; before releasing, put the
  tag on the final commit (pushing a `v*` tag builds and publishes a GitHub pre-release).

## Things to keep in mind

* The author does not want any tool or assistant credited in commits, tags or release notes: no
  `Co-Authored-By` trailers, no "generated with" lines. Commit identity is the repo-local one.
* Never write to a mouse's memory without a backup (`hub backup g502 file`); restoring a backup that
  was taken in the factory state erases the flash profile directory (see `docs/DEVICES.md`).
* Verified means run on hardware; say "sent" when it was only sent. Windows and macOS builds are
  produced by CI (`.github/workflows/ci.yml`); download the artifacts from the Actions tab to test.
