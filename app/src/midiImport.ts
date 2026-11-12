// Importing a Standard MIDI File into a pattern: every note of the file plays one pad, at
// a pitch relative to a reference note (the pitch the sample was recorded at).
//
// Pitch is written as byte 3 of the note: 0x8D for the sample's own pitch, 0x8D + n for n
// semitones up or down _(to confirm on the SP-404MKII)_. A note at the reference pitch is
// written exactly as an ordinary note, so the rhythm imports right whatever the pitch
// encoding turns out to be. The SP-404MKII plays two octaves around the sample's pitch.
import type { EditNote } from "./api";
import { BAR_TICKS } from "./patternEdit";

/** A note read from the file, in the file's own ticks. */
export interface MidiNote {
  tick: number;
  length: number;
  key: number;
  velocity: number;
}

export interface MidiFile {
  /** Ticks per quarter note. */
  ppq: number;
  notes: MidiNote[];
}

/** Semitones the SP-404MKII can play either way of the sample's pitch. */
export const PITCH_RANGE = 12;
const PITCH_ORIGINAL = 0x8d;
/** The device's ticks per quarter note. */
const PPQ = 480;

/** Read the notes of every track and channel of a Standard MIDI File. */
export function parseMidi(data: ArrayBuffer): MidiFile {
  const b = new Uint8Array(data);
  let p = 0;
  const u32 = () => ((b[p++] << 24) | (b[p++] << 16) | (b[p++] << 8) | b[p++]) >>> 0;
  const u16 = () => (b[p++] << 8) | b[p++];
  const text = (n: number) => String.fromCharCode(...b.subarray(p, p + n));
  const varlen = () => {
    let v = 0;
    for (let i = 0; i < 4; i++) {
      const c = b[p++];
      v = (v << 7) | (c & 0x7f);
      if (!(c & 0x80)) break;
    }
    return v;
  };

  if (b.length < 14 || text(4) !== "MThd") throw new Error("ce n'est pas un fichier MIDI");
  p = 4;
  const headerLength = u32();
  const start = p;
  u16(); // format: tracks are merged whatever it is
  const tracks = u16();
  const division = u16();
  if (division & 0x8000) throw new Error("fichier MIDI en temps SMPTE, non géré");
  p = start + headerLength;

  const notes: MidiNote[] = [];
  for (let t = 0; t < tracks && p + 8 <= b.length; t++) {
    const id = text(4);
    p += 4;
    const length = u32();
    const end = Math.min(b.length, p + length);
    if (id !== "MTrk") {
      p = end;
      continue;
    }
    let tick = 0;
    let status = 0;
    /** Notes sounding, by channel × 128 + key, waiting for their note-off. */
    const open = new Map<number, { tick: number; velocity: number }[]>();
    const close = (channel: number, key: number) => {
      const stack = open.get(channel * 128 + key);
      const on = stack?.shift();
      if (on) notes.push({ tick: on.tick, length: Math.max(1, tick - on.tick), key, velocity: on.velocity });
    };
    while (p < end) {
      tick += varlen();
      let s = b[p];
      if (s & 0x80) p++;
      else s = status; // running status
      if (s === 0xff) {
        p++; // meta type
        p += varlen();
        continue;
      }
      if (s === 0xf0 || s === 0xf7) {
        p += varlen();
        continue;
      }
      status = s;
      const kind = s & 0xf0;
      const channel = s & 0x0f;
      if (kind === 0x90 || kind === 0x80) {
        const key = b[p++];
        const velocity = b[p++];
        if (kind === 0x90 && velocity > 0) {
          const k = channel * 128 + key;
          if (!open.has(k)) open.set(k, []);
          open.get(k)!.push({ tick, velocity });
        } else {
          close(channel, key);
        }
      } else if (kind === 0xc0 || kind === 0xd0) {
        p += 1;
      } else {
        p += 2;
      }
    }
    // Notes never released end with their track.
    for (const [k, stack] of open) for (const _ of stack) close(Math.floor(k / 128), k % 128);
    p = end;
  }
  notes.sort((a, z) => a.tick - z.tick || z.key - a.key);
  return { ppq: division, notes };
}

export interface ImportPlan {
  notes: EditNote[];
  /** Pattern length in bars needed: 2, 4, 8 or 16. */
  bars: number;
  /** Notes outside the playable range before folding, with the transpose applied. */
  outOfRange: number;
  /** Lowest and highest note, in semitones from the reference, with the transpose. */
  lowest: number;
  highest: number;
  /** Notes dropped because another started on the same tick (only the highest stays). */
  chordNotes: number;
  /** Notes dropped because they start after 16 bars. */
  tooLate: number;
}

/**
 * Turn the file's notes into notes on `pad`, `transpose` semitones up or down, relative to
 * `reference` (the MIDI note at the sample's own pitch). Notes still out of range after
 * the transpose are folded by octaves into range. The pad plays one note at a time: of
 * notes starting together the highest stays, and a note stops where the next one starts.
 */
export function planImport(
  file: MidiFile,
  pad: number,
  reference: number,
  transpose: number,
  firstOrder: number,
): ImportPlan {
  const scale = (t: number) => Math.round((t * PPQ) / file.ppq);
  let outOfRange = 0;
  let chordNotes = 0;
  let tooLate = 0;
  let lowest = Infinity;
  let highest = -Infinity;
  const limit = 16 * BAR_TICKS;

  const kept: { tick: number; length: number; semis: number; velocity: number }[] = [];
  for (const n of file.notes) {
    const semis = n.key - reference + transpose;
    lowest = Math.min(lowest, semis);
    highest = Math.max(highest, semis);
    if (Math.abs(semis) > PITCH_RANGE) outOfRange++;
    const tick = scale(n.tick);
    if (tick >= limit) {
      tooLate++;
      continue;
    }
    const last = kept[kept.length - 1];
    if (last && last.tick === tick) {
      chordNotes++; // sorted highest first on a tick: keep the first one
      continue;
    }
    kept.push({ tick, length: Math.max(1, scale(n.tick + n.length) - tick), semis, velocity: n.velocity });
  }

  const end = kept.reduce((m, n) => Math.max(m, n.tick + n.length), 0);
  const bars = [2, 4, 8, 16].find((b) => b * BAR_TICKS >= end) ?? 16;
  const length = bars * BAR_TICKS;

  const notes = kept.map((n, i) => {
    let semis = n.semis;
    while (semis > PITCH_RANGE) semis -= 12;
    while (semis < -PITCH_RANGE) semis += 12;
    const next = i + 1 < kept.length ? kept[i + 1].tick : length;
    const stop = Math.min(n.tick + n.length, next, length);
    return {
      tick: n.tick,
      pad,
      velocity: Math.min(127, Math.max(1, n.velocity)),
      gate: Math.max(1, stop - n.tick) - 1,
      pitch: PITCH_ORIGINAL + semis,
      offVelocity: 0x40,
      flags: 0,
      order: firstOrder + i,
    } satisfies EditNote;
  });

  return {
    notes,
    bars,
    outOfRange,
    lowest: Number.isFinite(lowest) ? lowest : 0,
    highest: Number.isFinite(highest) ? highest : 0,
    chordNotes,
    tooLate,
  };
}

/** Semitones a note is transposed by, from its pitch byte, or null when the byte is in
 * the other form the device writes (bit 7 clear), whose meaning is not known yet. */
export function semitonesOf(pitch: number): number | null {
  if (pitch === 0) return 0;
  return pitch & 0x80 ? pitch - PITCH_ORIGINAL : null;
}
