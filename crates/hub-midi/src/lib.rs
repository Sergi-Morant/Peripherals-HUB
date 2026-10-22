//! MIDI to vibration. A `.mid` file becomes a list of [`Note`]s, which are planned into timed
//! [`Event`]s for a particular device: real pitches on devices with tone generators (the Steam
//! Controller's haptics), a rhythm on plain rumble motors (pitch cannot change there).

use hub_core::Motor;
use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Note {
    pub start_ms: u64,
    pub duration_ms: u64,
    /// MIDI key, 0..=127 (69 = A4 = 440 Hz).
    pub key: u8,
    /// 1..=127
    pub velocity: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Tone { voice: usize, hz: u16, duration_ms: u16, volume: u8 },
    /// Set these motors to these levels (0..=100); everything else is left alone.
    Rumble(Vec<(Motor, u8)>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub at_ms: u64,
    pub action: Action,
}

/// Parse a Standard MIDI File. Percussion (channel 10) is skipped: it has no pitch.
pub fn parse(bytes: &[u8]) -> Result<Vec<Note>, String> {
    let smf = Smf::parse(bytes).map_err(|e| format!("not a readable MIDI file: {e}"))?;
    let ticks_per_beat = match smf.header.timing {
        Timing::Metrical(t) => t.as_int() as u64,
        Timing::Timecode(..) => return Err("SMPTE-timed MIDI files are not supported".into()),
    };
    // Absolute-tick events of all tracks: tempo changes and note on/off.
    let mut tempos: Vec<(u64, u64)> = vec![(0, 500_000)]; // (tick, microseconds per beat)
    let mut notes_on_off: Vec<(u64, u8, u8, bool, u8)> = Vec::new(); // (tick, channel, key, on, velocity)
    for track in &smf.tracks {
        let mut tick = 0u64;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(t)) => tempos.push((tick, t.as_int() as u64)),
                TrackEventKind::Midi { channel, message } if channel.as_int() != 9 => match message {
                    MidiMessage::NoteOn { key, vel } => {
                        notes_on_off.push((tick, channel.as_int(), key.as_int(), vel.as_int() > 0, vel.as_int()))
                    }
                    MidiMessage::NoteOff { key, .. } => {
                        notes_on_off.push((tick, channel.as_int(), key.as_int(), false, 0))
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    tempos.sort_by_key(|t| t.0);
    // Ticks to milliseconds through the tempo map (microseconds = ticks * tempo / ticks_per_beat).
    let to_ms = |tick: u64| -> u64 {
        let mut us: u128 = 0;
        let mut last_tick = 0u64;
        let mut tempo = 500_000u64;
        for &(t, new) in &tempos {
            if t >= tick {
                break;
            }
            us += (t - last_tick) as u128 * tempo as u128 / ticks_per_beat as u128;
            last_tick = t;
            tempo = new;
        }
        us += (tick - last_tick) as u128 * tempo as u128 / ticks_per_beat as u128;
        (us / 1000) as u64
    };
    notes_on_off.sort_by_key(|e| (e.0, e.3)); // offs before ons at the same tick
    let mut open: Vec<(u8, u8, u64, u8)> = Vec::new(); // channel, key, start tick, velocity
    let mut notes = Vec::new();
    for (tick, ch, key, on, vel) in notes_on_off {
        if on {
            open.push((ch, key, tick, vel));
        } else if let Some(i) = open.iter().position(|o| o.0 == ch && o.1 == key) {
            let (_, _, start, v) = open.remove(i);
            let (s, e) = (to_ms(start), to_ms(tick));
            notes.push(Note { start_ms: s, duration_ms: e.saturating_sub(s).max(1), key, velocity: v });
        }
    }
    notes.sort_by_key(|n| (n.start_ms, n.key));
    if notes.is_empty() {
        return Err("the file has no notes to play".into());
    }
    Ok(notes)
}

pub fn total_ms(notes: &[Note]) -> u64 {
    notes.iter().map(|n| n.start_ms + n.duration_ms).max().unwrap_or(0)
}

/// Frequency of a MIDI key, folded by octaves into the range small haptic actuators reproduce.
pub fn haptic_hz(key: u8) -> u16 {
    let mut hz = 440.0 * 2f64.powf((key as f64 - 69.0) / 12.0);
    while hz > 600.0 {
        hz /= 2.0;
    }
    while hz < 100.0 {
        hz *= 2.0;
    }
    hz.round() as u16
}

fn volume(velocity: u8) -> u8 {
    (40 + velocity as u16 * 60 / 127) as u8
}

/// Real pitches on `voices` actuators. Each voice plays one note at a time; a new note takes a
/// free voice (low notes on the lower voice), or cuts off the voice that started earliest.
pub fn plan_tones(notes: &[Note], voices: usize) -> Vec<Event> {
    if voices == 0 {
        return vec![];
    }
    let median = {
        let mut keys: Vec<u8> = notes.iter().map(|n| n.key).collect();
        keys.sort_unstable();
        keys.get(keys.len() / 2).copied().unwrap_or(60)
    };
    let mut busy: Vec<Option<(u64, u64)>> = vec![None; voices]; // (started, ends)
    let mut events = Vec::new();
    for n in notes {
        let free: Vec<usize> = (0..voices).filter(|&v| busy[v].is_none_or(|(_, end)| end <= n.start_ms)).collect();
        let voice = if free.is_empty() {
            (0..voices).min_by_key(|&v| busy[v].map_or(0, |(s, _)| s)).unwrap()
        } else if free.len() == voices && voices > 1 {
            // Everything is free: spread by pitch.
            if n.key < median { 0 } else { voices - 1 }
        } else {
            free[0]
        };
        busy[voice] = Some((n.start_ms, n.start_ms + n.duration_ms));
        events.push(Event {
            at_ms: n.start_ms,
            action: Action::Tone {
                voice,
                hz: haptic_hz(n.key),
                duration_ms: n.duration_ms.clamp(20, u16::MAX as u64) as u16,
                volume: volume(n.velocity),
            },
        });
    }
    events
}

/// A rhythm for plain motors: the song's pitch range is split into one band per motor (low notes
/// on the first motor), and each note switches its motor on at a strength set by its velocity.
pub fn plan_rhythm(notes: &[Note], motors: &[Motor]) -> Vec<Event> {
    if motors.is_empty() || notes.is_empty() {
        return vec![];
    }
    let lo = notes.iter().map(|n| n.key).min().unwrap() as usize;
    let hi = notes.iter().map(|n| n.key).max().unwrap() as usize;
    let band = |key: u8| ((key as usize - lo) * motors.len() / (hi - lo + 1)).min(motors.len() - 1);
    let mut events = Vec::new();
    for (m, motor) in motors.iter().enumerate() {
        // This motor's notes, with overlaps cut so a new note retriggers the motor.
        let mine: Vec<&Note> = notes.iter().filter(|n| band(n.key) == m).collect();
        for (i, n) in mine.iter().enumerate() {
            let end = n.start_ms + n.duration_ms;
            let end = mine.get(i + 1).map_or(end, |next| end.min(next.start_ms));
            let level = (30 + n.velocity as u16 * 70 / 127) as u8;
            events.push(Event { at_ms: n.start_ms, action: Action::Rumble(vec![(*motor, level)]) });
            events.push(Event { at_ms: end, action: Action::Rumble(vec![(*motor, 0)]) });
        }
    }
    // Stable sort keeps "off" before the next "on" only if it was emitted first; order by time,
    // then offs before ons so a retrigger at the same instant ends on the new level.
    events.sort_by_key(|e| (e.at_ms, matches!(&e.action, Action::Rumble(l) if l.iter().any(|(_, v)| *v > 0))));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny format-0 MIDI file: 480 ticks/beat, tempo 500000 (120 BPM), notes C4 then E4 (one beat each).
    fn tiny_midi() -> Vec<u8> {
        let mut track = vec![
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // tempo 500000
            0x00, 0x90, 60, 100, // note on C4
            0x83, 0x60, 0x80, 60, 0, // after 480 ticks: off
            0x00, 0x90, 64, 80, // note on E4
            0x83, 0x60, 0x80, 64, 0, // off
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let mut f = b"MThd".to_vec();
        f.extend([0, 0, 0, 6, 0, 0, 0, 1, 0x01, 0xE0]);
        f.extend(b"MTrk");
        f.extend((track.len() as u32).to_be_bytes());
        f.append(&mut track);
        f
    }

    #[test]
    fn parses_notes_with_tempo() {
        let notes = parse(&tiny_midi()).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[0].key, notes[0].start_ms, notes[0].duration_ms), (60, 0, 500));
        assert_eq!((notes[1].key, notes[1].start_ms, notes[1].duration_ms), (64, 500, 500));
        assert_eq!(total_ms(&notes), 1000);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"not midi").is_err());
    }

    #[test]
    fn hz_is_folded_into_haptic_range() {
        assert_eq!(haptic_hz(69), 440);
        assert!((100..=600).contains(&haptic_hz(20)));
        assert!((100..=600).contains(&haptic_hz(120)));
    }

    #[test]
    fn tones_use_both_voices_and_cut_when_full() {
        let notes = vec![
            Note { start_ms: 0, duration_ms: 1000, key: 40, velocity: 100 },
            Note { start_ms: 0, duration_ms: 1000, key: 80, velocity: 100 },
            Note { start_ms: 100, duration_ms: 1000, key: 60, velocity: 100 },
        ];
        let ev = plan_tones(&notes, 2);
        assert_eq!(ev.len(), 3);
        let voice = |e: &Event| match e.action {
            Action::Tone { voice, .. } => voice,
            _ => unreachable!(),
        };
        // The low note is on voice 0, the high note on voice 1; the third cuts off the earlier one.
        assert_eq!((voice(&ev[0]), voice(&ev[1])), (0, 1));
        assert!(voice(&ev[2]) < 2);
    }

    #[test]
    fn rhythm_splits_pitch_range_across_motors() {
        let notes = vec![
            Note { start_ms: 0, duration_ms: 200, key: 40, velocity: 127 },
            Note { start_ms: 300, duration_ms: 200, key: 90, velocity: 127 },
        ];
        let ev = plan_rhythm(&notes, &[Motor::Strong, Motor::Weak]);
        // low note on Strong (on, off), high note on Weak (on, off), in time order
        assert_eq!(ev.len(), 4);
        assert_eq!(ev[0].action, Action::Rumble(vec![(Motor::Strong, 100)]));
        assert_eq!(ev[1].action, Action::Rumble(vec![(Motor::Strong, 0)]));
        assert_eq!(ev[2].action, Action::Rumble(vec![(Motor::Weak, 100)]));
    }
}
