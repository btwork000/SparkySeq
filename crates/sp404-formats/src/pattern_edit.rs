//! Editing and writing `PTN/PTNnnnnn.BIN` pattern files (layout: `docs/re/04-patterns.md`).
//!
//! [`EditablePattern`] keeps every record of a file except the time fillers, each with its
//! absolute tick, so that a pattern read from the device can be written back without
//! losing bytes nobody understands yet. Only the ticks are recomputed: the deltas and the
//! fillers are rebuilt from them when encoding.
//!
//! The pattern editor works in 4/4 only. Assumptions still to confirm on hardware are
//! marked _(?)_.

use sp404_proto::PadIndex;

use crate::FormatError;

/// Ticks in one 4/4 bar (480 ticks per quarter note).
pub const BAR_TICKS: u32 = 4 * crate::pattern::PPQ as u32;

/// Pattern lengths the editor offers, in bars.
pub const EDITOR_BARS: [u8; 4] = [2, 4, 8, 16];

/// Byte 3 of a note the device recorded without transposition. The device writes `0x8D`
/// for ordinary notes, and `0x8D + n` for a note pitched n semitones up _(?)_.
pub const PITCH_ORIGINAL: u8 = 0x8D;

const RECORD: usize = 8;
const NOTE_FIRST: u8 = 0x2F;
const NOTE_LAST: u8 = 0x7F;
const FILLER: u8 = 0x80;
const END: u8 = 0x8C;

/// One record of a pattern file at its absolute tick. Byte 0 (the delta) is not kept: it is
/// recomputed from the ticks when the pattern is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub tick: u32,
    pub bytes: [u8; RECORD],
}

impl Record {
    /// A note on `pad`. `gate` is the stored gate length: the device stores the note's
    /// duration in ticks minus one (a 1/16 note is `0x77` = 119).
    pub fn note(
        tick: u32,
        pad: PadIndex,
        pitch: u8,
        velocity: u8,
        off_velocity: u8,
        gate: u16,
        flags: u8,
    ) -> Self {
        let index = pad.index();
        let group = (index / 80) as u8;
        let within = (index % 80) as u8;
        let [gate_lo, gate_hi] = gate.to_le_bytes();
        Record {
            tick,
            bytes: [
                0,
                NOTE_FIRST + within,
                (flags & 0xF0) | group,
                pitch,
                velocity.clamp(1, 127),
                off_velocity,
                gate_lo,
                gate_hi,
            ],
        }
    }

    /// The pad of a note record, `None` for any other record.
    pub fn note_pad(&self) -> Option<PadIndex> {
        if !(NOTE_FIRST..=NOTE_LAST).contains(&self.bytes[1]) {
            return None;
        }
        let group = u16::from(self.bytes[2] & 0x0F);
        PadIndex::new(group * 80 + u16::from(self.bytes[1] - NOTE_FIRST))
    }

    fn is_filler(bytes: &[u8]) -> bool {
        bytes[1] == FILLER && bytes[2..].iter().all(|&b| b == 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditablePattern {
    /// Every record but the fillers and the end record, in file order.
    pub records: Vec<Record>,
    /// Pattern length in ticks (the tick of the end record).
    pub length: u32,
    /// The 8 bytes after the end record: byte 0 = length in bars, byte 4 = time signature
    /// (0 = 4/4), byte 5 = `0x80` + loop start bar (0-based), byte 6 = loop end bar,
    /// byte 7 must be non-zero.
    pub trailer: [u8; RECORD],
}

impl EditablePattern {
    /// An empty 4/4 pattern of `bars` bars, looping over all of them.
    pub fn empty(bars: u8) -> Self {
        let bars = bars.max(1);
        EditablePattern {
            records: Vec::new(),
            length: u32::from(bars) * BAR_TICKS,
            trailer: [bars, 0, 0, 0, 0, 0x80, bars, 1],
        }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() % RECORD != 0 || bytes.is_empty() {
            return Err(FormatError::BadPattern("length is not a multiple of 8"));
        }
        let mut records = Vec::new();
        let mut tick = 0u32;
        let mut chunks = bytes.chunks_exact(RECORD);
        let mut trailer = None;
        for r in chunks.by_ref() {
            if r[1] == END {
                trailer = chunks.next().map(|t| <[u8; RECORD]>::try_from(t).unwrap());
                break;
            }
            if !Record::is_filler(r) {
                let mut bytes: [u8; RECORD] = r.try_into().unwrap();
                bytes[0] = 0;
                records.push(Record { tick, bytes });
            }
            tick += u32::from(r[0]);
        }
        let trailer = trailer.ok_or(FormatError::BadPattern("no end record or trailer"))?;
        Ok(EditablePattern {
            records,
            length: tick,
            trailer,
        })
    }

    /// Whether the pattern is in 4/4, the only time signature the editor handles.
    pub fn is_four_four(&self) -> bool {
        let code = self.trailer[4];
        code == 0 || code >= 7
    }

    /// Length in whole bars, rounded up.
    pub fn bars(&self) -> u32 {
        self.length.div_ceil(BAR_TICKS).max(1)
    }

    /// Change the length to `bars` bars. Records from the new end on are dropped, notes are
    /// cut so they end with the pattern, and the loop is reset to the whole pattern.
    pub fn set_bars(&mut self, bars: u8) {
        let bars = bars.max(1);
        self.length = u32::from(bars) * BAR_TICKS;
        self.trailer[0] = bars;
        self.trailer[5] = 0x80;
        self.trailer[6] = bars;
        if self.trailer[7] == 0 {
            self.trailer[7] = 1;
        }
        self.clip_to_length();
    }

    /// Drop records at or past the end and shorten notes that would ring past it.
    pub fn clip_to_length(&mut self) {
        let length = self.length;
        self.records.retain(|r| r.tick < length);
        for r in &mut self.records {
            if r.note_pad().is_some() {
                let gate = u32::from(u16::from_le_bytes([r.bytes[6], r.bytes[7]]));
                let max = length - r.tick - 1;
                if gate > max {
                    [r.bytes[6], r.bytes[7]] = (max as u16).to_le_bytes();
                }
            }
        }
    }

    /// Encode as a pattern file. Records are written in tick order (file order within a
    /// tick); deltas over 255 ticks are bridged with filler records, as the device does.
    pub fn encode(&self) -> Vec<u8> {
        let mut records: Vec<Record> = self
            .records
            .iter()
            .copied()
            .filter(|r| r.tick < self.length)
            .collect();
        records.sort_by_key(|r| r.tick); // stable: file order within a tick

        let filler = |tick| Record {
            tick,
            bytes: [0, FILLER, 0, 0, 0, 0, 0, 0],
        };
        let mut timeline = Vec::with_capacity(records.len() + 2);
        if records.first().is_none_or(|r| r.tick > 0) {
            timeline.push(filler(0));
        }
        timeline.extend(records);
        timeline.push(Record {
            tick: self.length,
            bytes: [0, END, 0, 0, 0, 0, 0, 0],
        });

        let mut out = Vec::with_capacity((timeline.len() + 8) * RECORD);
        for pair in timeline.windows(2) {
            let (this, next) = (pair[0], pair[1]);
            let mut bytes = this.bytes;
            let mut tick = this.tick;
            loop {
                let step = (next.tick - tick).min(255);
                bytes[0] = step as u8;
                out.extend_from_slice(&bytes);
                tick += step;
                if tick >= next.tick {
                    break;
                }
                bytes = filler(0).bytes;
            }
        }
        out.extend_from_slice(&[0, END, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&self.trailer);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pattern::{Event, Pattern};

    fn pad(name: &str) -> PadIndex {
        PadIndex::all().find(|p| p.to_string() == name).unwrap()
    }

    #[test]
    fn empty_pattern_is_a_valid_file() {
        let p = EditablePattern::empty(4);
        let bytes = p.encode();
        let parsed = Pattern::parse(&bytes).unwrap();
        assert_eq!(parsed.length, 4 * BAR_TICKS);
        assert!(parsed.events.is_empty());
        assert_eq!(&bytes[bytes.len() - 8..], &[4, 0, 0, 0, 0, 0x80, 4, 1]);
        assert_eq!(parsed.beats_per_bar(), Some(4));
    }

    #[test]
    fn notes_land_on_their_ticks_with_fillers_between() {
        let mut p = EditablePattern::empty(2);
        p.records.push(Record::note(
            1000,
            pad("A1"),
            PITCH_ORIGINAL,
            90,
            0x40,
            59,
            0,
        ));
        p.records.push(Record::note(
            0,
            pad("J16"),
            PITCH_ORIGINAL,
            127,
            0x40,
            119,
            0,
        ));
        p.records.push(Record::note(
            1000,
            pad("F3"),
            PITCH_ORIGINAL,
            30,
            0x40,
            1919,
            0,
        ));
        let bytes = p.encode();
        let records: Vec<&[u8]> = bytes.chunks(8).collect();
        let end = records.iter().position(|r| r[1] == END).unwrap();
        let ticks: u32 = records[..end].iter().map(|r| u32::from(r[0])).sum();
        assert_eq!(ticks, 2 * BAR_TICKS);
        let parsed = Pattern::parse(&bytes).unwrap();
        assert_eq!(parsed.length, 2 * BAR_TICKS);
        let notes: Vec<(u32, String, u8, u16)> = parsed
            .events
            .iter()
            .map(|e| match *e {
                Event::Note {
                    tick,
                    pad,
                    velocity,
                    length,
                    ..
                } => (tick, pad.to_string(), velocity, length),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            notes,
            vec![
                (0, "J16".into(), 127, 119),
                (1000, "A1".into(), 90, 59),
                (1000, "F3".into(), 30, 1919),
            ]
        );
    }

    #[test]
    fn unknown_records_survive_a_round_trip() {
        // A device-style file: note, fillers, an unexplained record carrying note data,
        // a control change, a once-per-file record, end and trailer.
        let file = [
            [0x00, 0x30, 0x00, 0x8D, 0x5A, 0x40, 0x77, 0x00],
            [0xFF, 0x80, 0, 0, 0, 0, 0, 0],
            [0xBD, 0x80, 0, 0, 0, 0, 0, 0],
            [0x42, 0x80, 0x00, 0x8D, 0x5A, 0x40, 0x77, 0x00],
            [0x10, 0x8E, 0x02, 0x00, 0x00, 0x07, 0x64, 0x00],
            [0x5B, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF],
            [0x00, END, 0, 0, 0, 0, 0, 0],
            [0x01, 0, 0, 0, 0, 0x80, 0x01, 0x01],
        ]
        .concat();
        let p = EditablePattern::parse(&file).unwrap();
        assert_eq!(p.records.len(), 4);
        assert_eq!(p.length, 255 + 0xBD + 0x42 + 0x10 + 0x5B);
        let again = EditablePattern::parse(&p.encode()).unwrap();
        assert_eq!(again, p);
        assert_eq!(
            Pattern::parse(&p.encode()).unwrap(),
            Pattern::parse(&file).unwrap()
        );
    }

    #[test]
    fn shrinking_drops_and_cuts_notes() {
        let mut p = EditablePattern::empty(4);
        p.records.push(Record::note(
            100,
            pad("B2"),
            PITCH_ORIGINAL,
            100,
            0x40,
            5000,
            0,
        ));
        p.records.push(Record::note(
            5000,
            pad("B3"),
            PITCH_ORIGINAL,
            100,
            0x40,
            59,
            0,
        ));
        p.set_bars(2);
        assert_eq!(p.records.len(), 1);
        let gate = u16::from_le_bytes([p.records[0].bytes[6], p.records[0].bytes[7]]);
        assert_eq!(u32::from(gate), 2 * BAR_TICKS - 100 - 1);
        assert_eq!(p.trailer[0], 2);
        assert_eq!(p.trailer[6], 2);
    }

    #[test]
    fn note_records_address_all_banks() {
        for name in ["A1", "E16", "F1", "J16"] {
            let r = Record::note(0, pad(name), PITCH_ORIGINAL, 100, 0x40, 59, 0x40);
            assert_eq!(r.note_pad().unwrap().to_string(), name);
            assert_eq!(r.bytes[2] & 0xF0, 0x40);
        }
    }
}
