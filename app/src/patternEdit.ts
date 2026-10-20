// Editing rules of the pattern editor, kept apart from the component so they stay small
// and readable. Times are in ticks (480 per quarter note). The grid step is chosen in the
// editor (1/4 to 1/32); the finest, a 1/32 note, is `STEP`.
//
// A note's stored gate is its duration minus one tick (a 1/16 note is 119), as the
// SP-404MKII writes it. Two notes on the same pad never overlap: a note stops where the
// next one on its pad starts. Overlaps already in a pattern recorded on the device are
// left as they are, and so are notes off the grid until they are moved.
import type { EditNote } from "./api";

/** Ticks in a 1/32 note, the finest grid step. */
export const STEP = 60;
export const STEPS_PER_BAR = 32;
export const BAR_TICKS = STEP * STEPS_PER_BAR;

/** The grid steps the editor offers, in ticks. */
export const GRIDS = [
  { label: "1/4", ticks: 480 },
  { label: "1/8", ticks: 240 },
  { label: "1/16", ticks: 120 },
  { label: "1/32", ticks: 60 },
];

/** `tick` rounded down to the grid. */
export const floorTo = (tick: number, grid: number) => Math.floor(tick / grid) * grid;
/** `tick` rounded to the nearest grid line. */
export const roundTo = (tick: number, grid: number) => Math.round(tick / grid) * grid;

export const duration = (n: EditNote) => n.gate + 1;
export const end = (n: EditNote) => n.tick + n.gate + 1;

/** Bar, beat and step (all 1-based) of a tick, as the device shows positions. */
export function position(tick: number): string {
  const step = Math.floor(tick / STEP);
  const bar = Math.floor(step / STEPS_PER_BAR) + 1;
  const beat = Math.floor((step % STEPS_PER_BAR) / 8) + 1;
  const sub = (step % 8) + 1;
  const off = tick % STEP ? " (hors grille)" : "";
  return `${bar}.${beat}.${sub}${off}`;
}

function samePad(notes: EditNote[], pad: number, except?: EditNote): EditNote[] {
  return notes.filter((n) => n.pad === pad && n !== except);
}

/** The note on `pad` that sounds at `tick`, if any. */
export function noteAt(notes: EditNote[], pad: number, tick: number): EditNote | undefined {
  return notes.find((n) => n.pad === pad && n.tick <= tick && tick < end(n));
}

/** Where the next note on `pad` after `tick` starts, or `limit` when there is none. */
export function nextStart(notes: EditNote[], pad: number, tick: number, limit: number, except?: EditNote): number {
  let next = limit;
  for (const n of samePad(notes, pad, except)) if (n.tick > tick && n.tick < next) next = n.tick;
  return next;
}

/** Whether a note could sit on `pad` from `tick` for `length` ticks without touching
 * another note on that pad. */
export function fits(notes: EditNote[], pad: number, tick: number, length: number, except?: EditNote): boolean {
  return samePad(notes, pad, except).every((n) => end(n) <= tick || n.tick >= tick + length);
}

/**
 * Add a note at `tick` on `pad`, `ticks` long, shortened to stop before the next note on
 * the pad and at the end of the pattern. A note already sounding there on that pad is cut
 * short so the two do not overlap. Returns the new note, or null when `tick` is past the
 * end.
 */
export function addNote(
  notes: EditNote[],
  pad: number,
  tick: number,
  ticks: number,
  velocity: number,
  defaults: { pitch: number; order: number },
  length: number,
): EditNote | null {
  if (tick < 0 || tick >= length) return null;
  for (const n of samePad(notes, pad)) {
    if (n.tick < tick && end(n) > tick) n.gate = tick - n.tick - 1;
  }
  const stop = Math.min(tick + ticks, nextStart(notes, pad, tick, length));
  const note: EditNote = {
    tick,
    pad,
    velocity,
    gate: stop - tick - 1,
    pitch: defaults.pitch,
    offVelocity: 0x40,
    flags: 0,
    order: defaults.order,
  };
  notes.push(note);
  return note;
}

/** Move the end of `note` to `endTick`: at least to the first grid line after the note
 * starts, and no further than the next note on its pad or the end of the pattern. */
export function resize(notes: EditNote[], note: EditNote, endTick: number, length: number, grid: number) {
  const first = floorTo(note.tick, grid) + grid;
  const last = nextStart(notes, note.pad, note.tick, length, note);
  const stop = Math.min(Math.max(first, endTick), last);
  note.gate = Math.max(1, stop - note.tick) - 1;
}

/** Move `note` to `tick` on `pad`, keeping its length. Returns false, leaving the note
 * where it was, when it would overlap another note or leave the pattern. */
export function move(notes: EditNote[], note: EditNote, tick: number, pad: number, length: number): boolean {
  if (tick < 0 || tick + duration(note) > length) return false;
  if (!fits(notes, pad, tick, duration(note), note)) return false;
  note.tick = tick;
  note.pad = pad;
  return true;
}

/** Cut the pattern to `length` ticks: notes from there on go, the rest stop by the end. */
export function clip(notes: EditNote[], length: number): EditNote[] {
  const kept = notes.filter((n) => n.tick < length);
  for (const n of kept) if (end(n) > length) n.gate = length - n.tick - 1;
  return kept;
}
