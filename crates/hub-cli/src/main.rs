use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use hub_core::{ButtonAction, Capability, Effect, EffectSetting, HidApi, Hub, Motor, Peripheral, Rgb};

#[derive(Parser)]
#[command(name = "hub", version, about = "Control tool for the Peripherals HUB")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List connected devices.
    List,
    /// Listen to the Steam Puck's controller slots for 3 seconds (for driver development).
    Steam,
    /// Listen to one HID device for 4 seconds and summarise the report ids: `hub sniff 045e:0b13`.
    Sniff { id: String },
    /// Dump every HID interface hidapi sees (for driver development).
    Raw,
    /// Dump HID++ features of Logitech devices (for driver development).
    Probe,
    /// Print the battery of every device that reports one.
    Battery,
    /// Vibrate for a moment: `hub rumble xbox --strong 60 --ms 600`.
    Rumble {
        device: String,
        #[arg(long, default_value_t = 0)]
        strong: u8,
        #[arg(long, default_value_t = 0)]
        weak: u8,
        #[arg(long, default_value_t = 0)]
        left: u8,
        #[arg(long, default_value_t = 0)]
        right: u8,
        #[arg(long, default_value_t = 600)]
        ms: u64,
    },
    /// Play a MIDI file through a device: `hub play steam examples/ode-to-joy.mid`.
    Play {
        device: String,
        file: std::path::PathBuf,
        /// Play it as sound on the built-in speaker instead of as vibration.
        #[arg(long)]
        speaker: bool,
    },
    /// Play a WAV file (or a test tone with `--tone HZ`) on the built-in speaker:
    /// `hub speaker ds4 song.wav --volume 80`.
    Speaker {
        device: String,
        file: Option<std::path::PathBuf>,
        #[arg(long)]
        tone: Option<u32>,
        #[arg(long, default_value_t = 60)]
        volume: u8,
    },
    /// Play a tone on a voice (Steam Controller haptics): `hub tone steam --voice 0 --hz 220`.
    Tone {
        device: String,
        #[arg(long, default_value_t = 0)]
        voice: usize,
        #[arg(long, default_value_t = 220)]
        hz: u16,
        #[arg(long, default_value_t = 600)]
        ms: u16,
        #[arg(long, default_value_t = 60)]
        volume: u8,
    },
    /// Show the onboard profiles and what each button does: `hub profiles g502`.
    Profiles { device: String },
    /// Show which onboard profile is running, or switch: `hub profile g502 1`.
    Profile { device: String, number: Option<usize> },
    /// Save all onboard profiles to a file: `hub backup g502 g502.profiles`.
    Backup { device: String, file: std::path::PathBuf },
    /// Write a backup file back to the mouse: `hub restore g502 g502.profiles`.
    Restore { device: String, file: std::path::PathBuf },
    /// Remap a button of a profile (numbers as in `hub profiles`): `hub button g502 1 4 mouse:forward`.
    /// ACTION is mouse:left|right|middle|back|forward|N, key:HEXUSAGE[+ctrl+shift+alt+meta],
    /// media:HEXUSAGE, fn:NAME (e.g. fn:cycle-dpi), or none.
    Button { device: String, profile: usize, button: usize, action: String },
    /// Show or set the DPI: `hub dpi g502 1600`.
    Dpi { device: String, value: Option<u16> },
    /// Show or set the polling rate in Hz: `hub rate g502 500`.
    Rate { device: String, hz: Option<u16> },
    /// Switch between onboard profile and host control: `hub mode g502 host|onboard`.
    Mode { device: String, mode: String },
    /// Run a lighting effect: `hub effect g502 breathe ff0000 --period 3000`.
    Effect {
        device: String,
        /// off | static | breathe | cycle
        effect: String,
        /// RRGGBB (static and breathe)
        rgb: Option<String>,
        #[arg(long, default_value_t = 4000)]
        period: u16,
        #[arg(long, default_value_t = 100)]
        brightness: u8,
    },
    /// Set a static colour: `hub color ds4 ff0000`.
    Color {
        /// Substring of the device name or id (case-insensitive).
        device: String,
        /// RRGGBB
        rgb: String,
        /// Lighting zone index.
        #[arg(long, default_value_t = 0)]
        zone: usize,
        /// Apply to every matching device instead of just the first.
        #[arg(long)]
        all: bool,
    },
}

fn coarse_label(p: u8) -> &'static str {
    match p {
        0..=10 => "low",
        11..=40 => "medium",
        41..=70 => "high",
        _ => "full",
    }
}

fn hub() -> Hub {
    let mut h = Hub::new();
    h.register(Box::new(drv_ds4::Ds4Driver));
    h.register(Box::new(drv_g502x::G502xDriver));
    h.register(Box::new(drv_gamepads::GamepadDriver));
    h.register(Box::new(drv_steam::SteamDriver));
    h.register(Box::new(drv_xbox::XboxDriver));
    h
}

fn matches(p: &dyn Peripheral, q: &str) -> bool {
    let q = q.to_lowercase();
    let i = p.info();
    i.name.to_lowercase().contains(&q) || i.id.to_lowercase().contains(&q)
}

fn parse_action(s: &str) -> Result<ButtonAction> {
    use hub_core::function_name;
    let (kind, rest) = s.split_once(':').unwrap_or((s, ""));
    Ok(match kind {
        "none" | "disabled" => ButtonAction::Disabled,
        "mouse" => ButtonAction::Mouse(match rest {
            "left" => 0x0001,
            "right" => 0x0002,
            "middle" => 0x0004,
            "back" => 0x0008,
            "forward" => 0x0010,
            n => {
                let number = n.parse::<u32>().ok().filter(|n| (1..=16).contains(n));
                1u16 << (number.ok_or_else(|| anyhow::anyhow!("unknown mouse button '{n}'"))? - 1)
            }
        }),
        "key" => {
            let mut parts = rest.split('+');
            let usage = u8::from_str_radix(parts.next().unwrap_or(""), 16)?;
            let mut modifiers = 0;
            for m in parts {
                modifiers |= match m {
                    "ctrl" => 1,
                    "shift" => 2,
                    "alt" => 4,
                    "meta" => 8,
                    _ => bail!("unknown modifier '{m}'"),
                };
            }
            ButtonAction::Key { modifiers, usage }
        }
        "media" => ButtonAction::Consumer(u16::from_str_radix(rest, 16)?),
        "fn" => {
            let id = (0..=0x11u8)
                .find(|&i| function_name(i).replace(' ', "-") == rest)
                .ok_or_else(|| anyhow::anyhow!("unknown function '{rest}'"))?;
            ButtonAction::Function { id, data: 0 }
        }
        _ => bail!("cannot read action '{s}'"),
    })
}

fn find<'a>(devices: &'a mut [Box<dyn Peripheral>], q: &str) -> Result<&'a mut Box<dyn Peripheral>> {
    match devices.iter_mut().find(|d| matches(d.as_ref(), q)) {
        Some(d) => Ok(d),
        None => bail!("no device matching '{q}'"),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut api = HidApi::new()?;
    let mut devices = hub().scan(&mut api)?;
    match cli.cmd {
        Cmd::List => {
            if devices.is_empty() {
                println!("no devices found");
            }
            for d in &devices {
                let i = d.info();
                println!(
                    "{:<24} {:?} {}  {}{}",
                    i.name,
                    i.kind,
                    i.transport.label(),
                    i.id,
                    i.serial.as_ref().map_or(String::new(), |s| format!("  [{s}]"))
                );
                for c in d.capabilities() {
                    match c {
                        Capability::Battery => println!("    battery"),
                        Capability::Rgb { zones } => println!("    rgb: {}", zones.join(", ")),
                        Capability::Effects(e) => println!("    effects: {}", e.iter().map(|e| e.label()).collect::<Vec<_>>().join(", ")),
                        Capability::Dpi(l) => println!("    dpi: {}..{} ({} values)", l.first().unwrap_or(&0), l.last().unwrap_or(&0), l.len()),
                        Capability::ReportRate(l) => println!("    report rate: {l:?} Hz"),
                        Capability::HostMode => println!("    onboard/host mode"),
                        Capability::Tones(n) => println!("    tones: {n} voices"),
                        Capability::Speaker => println!("    speaker"),
                        Capability::Profiles { profiles, buttons } => {
                            println!("    onboard profiles: {profiles} profiles, {buttons} buttons")
                        }
                        Capability::Rumble(m) => println!("    rumble: {}", m.iter().map(|m| m.label()).collect::<Vec<_>>().join(", ")),
                    }
                }
            }
        }
        Cmd::Steam => {
            for line in drv_steam::sniff(&api, std::time::Duration::from_secs(3)) {
                println!("{line}");
            }
        }
        Cmd::Sniff { id } => {
            let (v, p) = id.split_once(':').ok_or_else(|| anyhow::anyhow!("use VID:PID, e.g. 045e:0b13"))?;
            let (v, p) = (u16::from_str_radix(v, 16)?, u16::from_str_radix(p, 16)?);
            let Some(info) = api.device_list().find(|d| d.vendor_id() == v && d.product_id() == p) else {
                bail!("no such device")
            };
            let dev = info.open_device(&api)?;
            let mut ids: std::collections::BTreeMap<u8, (u32, Vec<u8>)> = Default::default();
            let end = std::time::Instant::now() + std::time::Duration::from_secs(4);
            let mut buf = [0u8; 128];
            while std::time::Instant::now() < end {
                let n = dev.read_timeout(&mut buf, 100)?;
                if n > 0 {
                    let e = ids.entry(buf[0]).or_default();
                    e.0 += 1;
                    e.1 = buf[..n.min(16)].to_vec();
                }
            }
            for (id, (count, sample)) in ids {
                let hex: Vec<String> = sample.iter().map(|b| format!("{b:02x}")).collect();
                println!("id {id:#04x} x{count}: {}", hex.join(" "));
            }
            let mut desc = [0u8; 4096];
            if let Ok(n) = dev.get_report_descriptor(&mut desc) {
                println!("descriptor: {n} bytes");
                // Report ids declared by the descriptor (items 0x85 <id>), to see which exist.
                let ids: Vec<u8> = desc[..n].windows(2).filter(|w| w[0] == 0x85).map(|w| w[1]).collect();
                println!("report ids in descriptor: {ids:02x?}");
            }
            for id in [0x04u8, 0x01] {
                let mut b = [0u8; 64];
                b[0] = id;
                match dev.get_input_report(&mut b) {
                    Ok(n) => println!("get_input_report({id:#04x}) -> {n} bytes: {:02x?}", &b[..n.min(16)]),
                    Err(e) => println!("get_input_report({id:#04x}) failed: {e}"),
                }
            }
        }
        Cmd::Raw => {
            for d in api.device_list() {
                println!(
                    "{:04x}:{:04x} bus={:?} if={} usage={:04x}/{:04x} serial={:?} product={:?} path={}",
                    d.vendor_id(),
                    d.product_id(),
                    d.bus_type(),
                    d.interface_number(),
                    d.usage_page(),
                    d.usage(),
                    d.serial_number().unwrap_or(""),
                    d.product_string().unwrap_or(""),
                    d.path().to_string_lossy()
                );
            }
        }
        Cmd::Probe => {
            for line in drv_g502x::probe(&api) {
                println!("{line}");
            }
        }
        Cmd::Battery => {
            for d in devices.iter_mut().filter(|d| d.capabilities().contains(&Capability::Battery)) {
                match d.battery() {
                    Ok(Some(b)) => println!(
                        "{}: {}{}",
                        d.info().name,
                        match (b.percent, b.coarse) {
                            (Some(p), true) => format!("{} (4 levels)", coarse_label(p)),
                            (Some(p), false) => format!("{p}%"),
                            _ => "?".into(),
                        },
                        if b.charging { " (charging)" } else { "" }
                    ),
                    Ok(None) => println!("{}: unknown", d.info().name),
                    Err(e) => println!("{}: {e}", d.info().name),
                }
            }
        }
        Cmd::Rumble { device, strong, weak, left, right, ms } => {
            let d = find(&mut devices, &device)?;
            let on = [(Motor::Strong, strong), (Motor::Weak, weak), (Motor::TriggerLeft, left), (Motor::TriggerRight, right)];
            let on: Vec<_> = on.into_iter().filter(|(_, l)| *l > 0).collect();
            d.rumble(&on)?;
            std::thread::sleep(std::time::Duration::from_millis(ms));
            let off: Vec<_> = on.iter().map(|(m, _)| (*m, 0)).collect();
            d.rumble(&off)?;
        }
        Cmd::Play { device, file, speaker } => {
            let notes = hub_midi::parse(&std::fs::read(&file)?).map_err(|e| anyhow::anyhow!(e))?;
            let d = find(&mut devices, &device)?;
            if speaker {
                println!("{} notes, {} s", notes.len(), hub_midi::total_ms(&notes) / 1000);
                d.play_audio(&hub_midi::render_pcm(&notes, 32_000), 32_000, 1, 80)?;
                return Ok(());
            }
            let caps = d.capabilities();
            let events = if let Some(Capability::Tones(n)) = caps.iter().find(|c| matches!(c, Capability::Tones(_))) {
                hub_midi::plan_tones(&notes, *n as usize)
            } else if let Some(Capability::Rumble(m)) = caps.iter().find(|c| matches!(c, Capability::Rumble(_))) {
                hub_midi::plan_rhythm(&notes, m)
            } else {
                bail!("{} cannot play music", d.info().name)
            };
            println!("{} notes, {} events, {} s", notes.len(), events.len(), hub_midi::total_ms(&notes) / 1000);
            let start = std::time::Instant::now();
            for e in &events {
                if let Some(wait) = (std::time::Duration::from_millis(e.at_ms)).checked_sub(start.elapsed()) {
                    std::thread::sleep(wait);
                }
                match &e.action {
                    hub_midi::Action::Tone { voice, hz, duration_ms, volume } => d.tone(*voice, *hz, *duration_ms, *volume)?,
                    hub_midi::Action::Rumble(l) => d.rumble(l)?,
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(600));
            let off: Vec<_> = [Motor::Strong, Motor::Weak, Motor::TriggerLeft, Motor::TriggerRight].map(|m| (m, 0)).to_vec();
            let _ = d.rumble(&off);
        }
        Cmd::Speaker { device, file, tone, volume } => {
            let d = find(&mut devices, &device)?;
            let (samples, rate, channels) = match (file, tone) {
                (_, Some(hz)) => {
                    let pcm = (0..32_000u32 * 2)
                        .map(|i| (20_000.0 * (std::f32::consts::TAU * hz as f32 * i as f32 / 32_000.0).sin()) as i16)
                        .collect();
                    (pcm, 32_000, 1)
                }
                (Some(f), None) => {
                    let mut r = hound::WavReader::open(&f)?;
                    let spec = r.spec();
                    let samples: Vec<i16> = match (spec.sample_format, spec.bits_per_sample) {
                        (hound::SampleFormat::Int, 16) => r.samples::<i16>().collect::<Result<_, _>>()?,
                        (hound::SampleFormat::Int, bits) if bits <= 32 => {
                            let shift = bits - 16;
                            r.samples::<i32>().map(|s| s.map(|v| (v >> shift) as i16)).collect::<Result<_, _>>()?
                        }
                        (hound::SampleFormat::Float, _) => {
                            r.samples::<f32>().map(|s| s.map(|v| (v * 32767.0) as i16)).collect::<Result<_, _>>()?
                        }
                        _ => bail!("unsupported WAV format"),
                    };
                    (samples, spec.sample_rate, spec.channels)
                }
                (None, None) => bail!("give a WAV file or --tone HZ"),
            };
            println!("{} s at {rate} Hz", samples.len() / channels.max(1) as usize / rate as usize);
            d.play_audio(&samples, rate, channels, volume)?;
        }
        Cmd::Tone { device, voice, hz, ms, volume } => {
            find(&mut devices, &device)?.tone(voice, hz, ms, volume)?;
            std::thread::sleep(std::time::Duration::from_millis(ms as u64));
        }
        Cmd::Profiles { device } => {
            let d = find(&mut devices, &device)?;
            for p in d.profiles()? {
                println!(
                    "Profile {} {}{}: {} ms, DPI {:?} (default #{}, shift #{}), colour {:02x}{:02x}{:02x}",
                    p.index,
                    if p.name.is_empty() { String::new() } else { format!("\"{}\" ", p.name) },
                    if p.enabled { "" } else { "(disabled)" },
                    p.report_interval_ms,
                    p.dpi_levels,
                    p.default_dpi_index + 1,
                    p.shift_dpi_index + 1,
                    p.color.0,
                    p.color.1,
                    p.color.2
                );
                for (i, b) in p.buttons.iter().enumerate() {
                    println!("  button {:>2}: {}", i + 1, b.label());
                }
            }
        }
        Cmd::Profile { device, number } => {
            let d = find(&mut devices, &device)?;
            if let Some(n) = number {
                if n == 0 {
                    bail!("profile numbers start at 1");
                }
                d.set_active_profile(n - 1)?;
            }
            match d.active_profile()? {
                Some(i) => println!("running profile {}", i + 1),
                None => println!("running a profile that is not in the directory"),
            }
        }
        Cmd::Backup { device, file } => {
            let text = find(&mut devices, &device)?.export_profiles()?;
            std::fs::write(&file, text)?;
            println!("saved to {}", file.display());
        }
        Cmd::Restore { device, file } => {
            find(&mut devices, &device)?.import_profiles(&std::fs::read_to_string(&file)?)?;
            println!("restored from {}", file.display());
        }
        Cmd::Button { device, profile, button, action } => {
            let a = parse_action(&action)?;
            if profile == 0 || button == 0 {
                bail!("profile and button numbers start at 1");
            }
            find(&mut devices, &device)?.set_button(profile - 1, button - 1, a)?;
            println!("profile {profile} button {button}: {}", a.label());
        }
        Cmd::Dpi { device, value } => {
            let d = find(&mut devices, &device)?;
            if let Some(v) = value {
                d.set_dpi(v)?;
            }
            println!("dpi: {:?}", d.settings().dpi);
        }
        Cmd::Rate { device, hz } => {
            let d = find(&mut devices, &device)?;
            if let Some(v) = hz {
                d.set_report_rate(v)?;
            }
            println!("report rate: {:?} Hz", d.settings().report_rate);
        }
        Cmd::Mode { device, mode } => {
            let d = find(&mut devices, &device)?;
            match mode.as_str() {
                "host" => d.set_host_mode(true)?,
                "onboard" => d.set_host_mode(false)?,
                "show" => {}
                _ => bail!("mode must be host, onboard or show"),
            }
            println!("host mode: {:?}", d.settings().host_mode);
        }
        Cmd::Effect { device, effect, rgb, period, brightness } => {
            let effect = match effect.as_str() {
                "off" => Effect::Off,
                "static" => Effect::Static,
                "breathe" => Effect::Breathe,
                "cycle" => Effect::Cycle,
                _ => bail!("effect must be off, static, breathe or cycle"),
            };
            let color = match rgb {
                Some(h) => Rgb::from_hex(&h).ok_or_else(|| anyhow::anyhow!("colour must be RRGGBB"))?,
                None => Rgb(255, 255, 255),
            };
            find(&mut devices, &device)?
                .set_effect(0, EffectSetting { effect, color, period_ms: period, brightness })?;
        }
        Cmd::Color { device, rgb, zone, all } => {
            let Some(color) = Rgb::from_hex(&rgb) else { bail!("colour must be RRGGBB") };
            let mut hit = 0;
            for d in devices.iter_mut().filter(|d| matches(d.as_ref(), &device)) {
                d.set_rgb(zone, color)?;
                hit += 1;
                if !all {
                    break;
                }
            }
            if hit == 0 {
                bail!("no device matching '{device}'")
            }
        }
    }
    Ok(())
}
