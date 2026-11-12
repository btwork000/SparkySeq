<script setup lang="ts">
// Step editor for one pattern, against a grid of 1/4 to 1/16 notes, in two views: the
// 16 pads of a bank, or a piano roll of one pad (its notes from −12 to +12 semitones).
//
// Click an empty cell to add a note, drag a note to move it, drag its right edge to
// change its length, right-click (or double-click) to delete it. The selected note's
// velocity, length and humanize are also set in the bar above the grid; the arrow keys nudge it,
// Delete removes it, Ctrl+Z undoes. Notes sound on the SP-404MKII as they are placed.
// Nothing reaches the device until "Envoyer vers la 404", or, with "Synchro auto", a second
// after the last change.
//
// Each note is drawn on its 1/32 line; its humanize (how far it really starts from that
// line, ahead or behind) shows as a mark and a value, and stays with it when it moves.
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { api, type EditNote, type EditablePattern, type ExtraRecord } from "../api";
import { BANK_LETTERS, bpm, padLabel } from "../format";
import { ask, handleError, notify, store } from "../store";
import MidiImport from "./MidiImport.vue";
import { PITCH_RANGE, semitonesOf, type ImportPlan } from "../midiImport";
import {
  BAR_TICKS,
  GRIDS,
  HUMANIZE_MAX,
  STEP,
  anchorOf,
  humanizeOf,
  msToTicks,
  ticksToMs,
  addNote,
  clip,
  duration,
  floorTo,
  move,
  noteAt,
  position,
  resize,
  roundTo,
} from "../patternEdit";

const props = defineProps<{ slotIndex: number }>();
const emit = defineEmits<{ close: [] }>();

const ROW = 26;
const BAR_CHOICES = [2, 4, 8, 16];

const loading = ref(true);
const saving = ref(false);
const source = ref<EditablePattern | null>(null);

const model = reactive({
  notes: [] as EditNote[],
  extras: [] as ExtraRecord[],
  lengthTicks: 0,
  bars: 4,
  barsChanged: false,
});
/** Changed since the pattern was opened or last sent by hand ("Tout annuler"). */
const dirty = ref(false);
/** Changed since the device last received the pattern. */
const unsynced = ref(false);
const selected = ref<EditNote | null>(null);
const viewBank = ref(Math.floor(props.slotIndex / 16));
/** Zoom: pixels per 1/32 note, whatever the grid. */
const cell = ref<number>(Number(localStorage.getItem("sparkymk2.editor.cell")) || 14);
/** Grid step in ticks: where notes snap when placed, moved or stretched. */
const grid = ref<number>(
  GRIDS.some((g) => g.ticks === Number(localStorage.getItem("sparkymk2.editor.grid")))
    ? Number(localStorage.getItem("sparkymk2.editor.grid"))
    : 120,
);
const px = (ticks: number) => (ticks / STEP) * cell.value;
const listen = ref(true);
/** Length (ticks) and velocity of the last note placed or edited: the next one starts
 * like it, but never shorter than one grid step. */
const lastLength = ref(120);
const lastVelocity = ref(100);
let nextOrder = 1_000_000;

const width = computed(() => px(model.lengthTicks));
// ---- views: pads or piano roll ----

const view = ref<"pads" | "roll">("pads");
/** The pad shown in the piano roll. */
const rollPad = ref(props.slotIndex);
const NOTE_NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
/** Name of a pitch `semis` semitones from the sample's own pitch; samples are in C. */
const noteName = (semis: number) => NOTE_NAMES[((semis % 12) + 12) % 12];
const isBlack = (semis: number) => noteName(semis).includes("♯");
/** Byte 3 of a note `semis` semitones from the sample's pitch. */
const pitchFor = (semis: number) => (source.value?.defaultPitch ?? 0x8d) + semis;

/** A row of the grid: a pad, and in the piano roll the semitone it stands for. */
interface Row {
  pad: number;
  semis: number | null;
}
const rows = computed<Row[]>(() =>
  view.value === "pads"
    ? Array.from({ length: 16 }, (_, r) => ({ pad: viewBank.value * 16 + r, semis: null }))
    : Array.from({ length: 2 * PITCH_RANGE + 1 }, (_, r) => ({ pad: rollPad.value, semis: PITCH_RANGE - r })),
);
const visible = computed(() =>
  view.value === "pads"
    ? model.notes.filter((n) => Math.floor(n.pad / 16) === viewBank.value)
    : model.notes.filter((n) => n.pad === rollPad.value),
);
/** Row of a note in the current view. A pitch in the form not understood yet shows on 0. */
function rowOf(n: EditNote): number {
  if (view.value === "pads") return n.pad % 16;
  const semis = Math.max(-PITCH_RANGE, Math.min(PITCH_RANGE, semitonesOf(n.pitch) ?? 0));
  return PITCH_RANGE - semis;
}

function showRoll(pad?: number) {
  rollPad.value = pad ?? selected.value?.pad ?? rollPad.value;
  view.value = "roll";
}
const perBank = computed(() => {
  const c = new Array(10).fill(0);
  for (const n of model.notes) c[Math.floor(n.pad / 16)]++;
  return c;
});
const editable = computed(() => !!source.value?.fourFour);
const tempo = computed(() => (source.value ? source.value.bankTempo / 100 : 120));

// ---- loading and undo ----

function load(p: EditablePattern) {
  source.value = p;
  model.notes = p.notes.map((n) => ({ ...n }));
  model.extras = p.extras.map((e) => ({ ...e, bytes: [...e.bytes] }));
  model.lengthTicks = p.lengthTicks;
  model.bars = p.bars;
  model.barsChanged = false;
  dirty.value = false;
  unsynced.value = false;
  selected.value = null;
  history.length = 0;
}

async function open() {
  loading.value = true;
  try {
    load(await api.patternOpen(props.slotIndex));
  } catch (e) {
    handleError(e);
    emit("close");
  } finally {
    loading.value = false;
  }
}

interface Snapshot {
  notes: EditNote[];
  extras: ExtraRecord[];
  lengthTicks: number;
  bars: number;
  barsChanged: boolean;
}
const history: Snapshot[] = [];

/** Remember the state before a change, for Ctrl+Z. */
function checkpoint() {
  history.push({
    notes: model.notes.map((n) => ({ ...n })),
    extras: model.extras.map((e) => ({ ...e })),
    lengthTicks: model.lengthTicks,
    bars: model.bars,
    barsChanged: model.barsChanged,
  });
  if (history.length > 100) history.shift();
  dirty.value = true;
  unsynced.value = true;
}

function undo() {
  const s = history.pop();
  if (!s) return;
  Object.assign(model, s);
  selected.value = null;
  dirty.value = true;
  unsynced.value = true;
}

// ---- listening on the device ----

let previewPad: number | null = null;
let previewTimer: number | undefined;

async function stopPreview() {
  window.clearTimeout(previewTimer);
  const pad = previewPad;
  previewPad = null;
  if (pad !== null) await api.previewStop(pad).catch(() => {});
}

/** Play `pad` on the SP-404MKII for `ticks` at the bank tempo, or until released. */
async function audition(pad: number, ticks: number | null) {
  if (!listen.value && ticks !== null) return;
  await stopPreview();
  if (!store.pads[pad]?.hasSample && store.pads.length) return;
  previewPad = pad;
  try {
    await api.previewStart(pad);
  } catch (e) {
    previewPad = null;
    handleError(e);
    return;
  }
  if (ticks !== null) {
    const ms = (ticks / 480) * (60000 / tempo.value);
    previewTimer = window.setTimeout(stopPreview, Math.min(2000, Math.max(120, ms)));
  }
}

// ---- editing with the mouse ----

type Drag =
  /** `anchor`: the note's 1/32 line when the drag began; `humanize` travels with it. */
  | {
      kind: "move";
      note: EditNote;
      x: number;
      y: number;
      tick: number;
      anchor: number;
      humanize: number;
      row: number;
      moved: boolean;
    }
  /** `saved`: an undo step already covers this drag (the note was just added). */
  | { kind: "resize"; note: EditNote; moved: boolean; saved: boolean };
let drag: Drag | null = null;
const lanes = ref<HTMLElement | null>(null);

function locate(e: PointerEvent) {
  const box = lanes.value!.getBoundingClientRect();
  const x = e.clientX - box.left;
  const y = e.clientY - box.top;
  const tick = Math.max(0, (x / cell.value) * STEP);
  return { x, y, tick, row: Math.floor(y / ROW) };
}

function onDown(e: PointerEvent) {
  if (!editable.value || e.button !== 0) return;
  const at = locate(e);
  if (at.row < 0 || at.row >= rows.value.length) return;
  const row = rows.value[at.row];
  const pad = row.pad;
  const target = e.target as HTMLElement;
  const index = target.closest<HTMLElement>("[data-note]")?.dataset.note;
  lanes.value!.setPointerCapture(e.pointerId);

  if (index !== undefined) {
    const note = visible.value[Number(index)];
    selected.value = note;
    if (target.dataset.handle !== undefined) {
      drag = { kind: "resize", note, moved: false, saved: false };
    } else {
      drag = {
        kind: "move",
        note,
        x: at.x,
        y: at.y,
        tick: note.tick,
        anchor: anchorOf(note, model.lengthTicks),
        humanize: humanizeOf(note, model.lengthTicks),
        row: at.row,
        moved: false,
      };
      audition(note.pad, duration(note));
    }
    return;
  }

  // Empty space. A note sounding under the pointer (off the grid, say) is selected instead.
  // In the piano roll a pad plays one note at a time: a click on another pitch while a
  // note sounds adds a note there and cuts the one sounding, unless that one starts in
  // the same grid step.
  const hit = noteAt(model.notes, pad, at.tick);
  if (hit && (row.semis === null || rowOf(hit) === at.row || hit.tick >= floorTo(at.tick, grid.value))) {
    selected.value = hit;
    return;
  }
  checkpoint();
  const note = addNote(
    model.notes,
    pad,
    floorTo(at.tick, grid.value),
    Math.max(grid.value, lastLength.value),
    lastVelocity.value,
    { pitch: row.semis === null ? source.value!.defaultPitch : pitchFor(row.semis), order: nextOrder++ },
    model.lengthTicks,
  );
  if (!note) {
    history.pop();
    return;
  }
  selected.value = note;
  drag = { kind: "resize", note, moved: false, saved: true };
  audition(note.pad, duration(note));
}

function onMove(e: PointerEvent) {
  if (!drag) return;
  const at = locate(e);
  if (drag.kind === "resize") {
    const endTick = roundTo(at.tick, grid.value);
    const before = drag.note.gate;
    if (!drag.moved) {
      if (endTick === drag.note.tick + duration(drag.note)) return;
      if (!drag.saved) checkpoint();
      drag.moved = true;
    }
    resize(model.notes, drag.note, endTick, model.lengthTicks, grid.value);
    if (drag.note.gate !== before) lastLength.value = duration(drag.note);
    return;
  }
  const dx = ((at.x - drag.x) / cell.value) * STEP;
  const dr = Math.round((at.y - drag.y) / ROW);
  // The note's 1/32 line snaps to the grid once the pointer has gone at least half a grid
  // step; the note keeps its humanize. A plain click on a note does not move it.
  const shifted = Math.abs(dx) >= grid.value / 2;
  if (!drag.moved && !shifted && dr === 0) return;
  const target = rows.value[Math.min(rows.value.length - 1, Math.max(0, drag.row + dr))];
  const pad = target.pad;
  // Up and down: another pad, or in the piano roll another pitch of the same pad.
  const pitch = target.semis === null ? drag.note.pitch : pitchFor(target.semis);
  const tick = shifted ? roundTo(drag.anchor + dx, grid.value) + drag.humanize : drag.tick;
  const wasPad = drag.note.pad;
  const wasPitch = drag.note.pitch;
  if (tick === drag.note.tick && pad === wasPad && pitch === wasPitch) return;
  if (!drag.moved) {
    checkpoint();
    drag.moved = true;
  }
  if (move(model.notes, drag.note, tick, pad, model.lengthTicks)) {
    drag.note.pitch = pitch;
    if (pad !== wasPad || pitch !== wasPitch) audition(pad, duration(drag.note));
  }
}

function onUp() {
  drag = null;
}

function remove(note: EditNote) {
  checkpoint();
  model.notes = model.notes.filter((n) => n !== note);
  if (selected.value === note) selected.value = null;
}

function onContext(e: MouseEvent) {
  e.preventDefault();
  const index = (e.target as HTMLElement).closest<HTMLElement>("[data-note]")?.dataset.note;
  if (editable.value && index !== undefined) remove(visible.value[Number(index)]);
}

// ---- the selected note ----

function setVelocity(v: number) {
  const n = selected.value;
  if (!n || !Number.isFinite(v)) return;
  const value = Math.min(127, Math.max(1, Math.round(v)));
  if (value === n.velocity) return;
  checkpoint();
  n.velocity = value;
  lastVelocity.value = value;
}

/** Set the selected note's length in grid steps. */
function setSteps(s: number) {
  const n = selected.value;
  if (!n || !Number.isFinite(s)) return;
  checkpoint();
  resize(model.notes, n, n.tick + Math.max(1, Math.round(s)) * grid.value, model.lengthTicks, grid.value);
  lastLength.value = duration(n);
}

const selectedSteps = computed(() => (selected.value ? duration(selected.value) / grid.value : 0));

/** Humanize of the selected note, in ticks. */
const selectedHumanize = computed(() => (selected.value ? humanizeOf(selected.value, model.lengthTicks) : 0));
const ms = (ticks: number) => {
  const v = Math.round(ticksToMs(ticks, tempo.value));
  return v > 0 ? `+${v}` : String(v);
};

/** Set the selected note's humanize, in milliseconds: negative ahead of its 1/32 line,
 * positive behind it. The note keeps its length. Refused when it would touch another
 * note on its pad or start before the pattern. */
function setHumanize(value: number) {
  const n = selected.value;
  if (!n || !Number.isFinite(value)) return;
  const ticks = Math.max(-HUMANIZE_MAX, Math.min(HUMANIZE_MAX, msToTicks(value, tempo.value)));
  const tick = anchorOf(n, model.lengthTicks) + ticks;
  if (tick === n.tick) return;
  checkpoint();
  if (!move(model.notes, n, tick, n.pad, model.lengthTicks)) {
    history.pop();
    notify("Impossible : la note toucherait une autre note de ce pad, ou le début du pattern.");
  }
}
const gridLabel = computed(() => GRIDS.find((g) => g.ticks === grid.value)?.label ?? "");

function onKey(e: KeyboardEvent) {
  if (!editable.value || (e.target as HTMLElement).closest("input, select, textarea")) return;
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "z") {
    e.preventDefault();
    undo();
    return;
  }
  const n = selected.value;
  if (!n) return;
  if (e.key === "Delete" || e.key === "Backspace") {
    e.preventDefault();
    remove(n);
  } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
    e.preventDefault();
    setVelocity(n.velocity + (e.key === "ArrowUp" ? 1 : -1) * (e.shiftKey ? 1 : 8));
  } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
    e.preventDefault();
    checkpoint();
    const anchor = anchorOf(n, model.lengthTicks);
    const tick =
      roundTo(anchor, grid.value) + (e.key === "ArrowRight" ? 1 : -1) * grid.value + humanizeOf(n, model.lengthTicks);
    if (!move(model.notes, n, tick, n.pad, model.lengthTicks)) history.pop();
  }
}

onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKey);
  window.clearTimeout(syncTimer);
  stopPreview();
});

// ---- length, zoom, sending ----

async function setBars(bars: number) {
  const length = bars * BAR_TICKS;
  if (length === model.lengthTicks) return;
  const lost = model.notes.filter((n) => n.tick >= length).length;
  if (lost) {
    const ok = await ask(
      "Raccourcir le pattern ?",
      `${lost} note${lost > 1 ? "s" : ""} après la mesure ${bars} ${lost > 1 ? "seront supprimées" : "sera supprimée"}.`,
      [
        { label: "Annuler", value: "no" },
        { label: "Raccourcir", value: "yes", kind: "danger" },
      ],
    );
    if (ok !== "yes") return;
  }
  checkpoint();
  model.notes = clip(model.notes, length);
  model.extras = model.extras.filter((x) => x.tick < length);
  model.lengthTicks = length;
  model.bars = bars;
  model.barsChanged = true;
  if (selected.value && !model.notes.includes(selected.value)) selected.value = null;
}

function setCell(v: number) {
  cell.value = v;
  try {
    localStorage.setItem("sparkymk2.editor.cell", String(v));
  } catch {}
}

function setGrid(ticks: number) {
  grid.value = ticks;
  try {
    localStorage.setItem("sparkymk2.editor.grid", String(ticks));
  } catch {}
}

async function send() {
  const p = source.value;
  if (!p) return;
  const ok = await ask(
    `Envoyer ${p.label} vers la 404 ?`,
    p.exists
      ? `Le pattern ${p.label} du projet ${p.project} sera remplacé. L'ancienne version est d'abord gardée sur cet ordinateur.`
      : `Un nouveau pattern sera créé en ${p.label} dans le projet ${p.project}.`,
    [
      { label: "Annuler", value: "no" },
      { label: "Envoyer", value: "yes", kind: "primary" },
    ],
  );
  if (ok !== "yes") return;
  await stopPreview();
  saving.value = true;
  try {
    const backup = await api.patternWrite({
      slot: p.slot,
      project: p.project,
      lengthTicks: model.lengthTicks,
      bars: model.barsChanged ? model.bars : null,
      trailer: p.trailer,
      notes: model.notes,
      extras: model.extras,
    });
    store.patterns[p.slot] = true;
    backupKept = true;
    notify(
      backup ? `${p.label} envoyé. Ancienne version gardée dans ${backup}` : `${p.label} créé sur la 404.`,
      "info",
    );
    // Read back what the device now holds.
    load(await api.patternOpen(props.slotIndex));
  } catch (e) {
    handleError(e);
  } finally {
    saving.value = false;
  }
}

// ---- automatic sync ----
//
// With "Synchro auto" on, the pattern goes to the device a second after the last change,
// without asking, and the device reloads it; play it there. Only the first write of the
// session keeps a copy of what the device held; later ones replace the editor's own.

const SYNC_DELAY_MS = 1000;
const autoSync = ref(localStorage.getItem("sparkymk2.editor.autosync") === "1");
const syncState = ref<"idle" | "waiting" | "sending" | "done" | "error">("idle");
let syncTimer: number | undefined;
/** Bumped by every change, so a change made during a write is sent after it. */
let revision = 0;
/** Whether this session already kept a copy of the pattern the device held. */
let backupKept = false;

function setAutoSync(on: boolean) {
  autoSync.value = on;
  try {
    localStorage.setItem("sparkymk2.editor.autosync", on ? "1" : "0");
  } catch {}
  if (on && unsynced.value) scheduleSync();
  if (!on) {
    window.clearTimeout(syncTimer);
    syncState.value = "idle";
  }
}

function scheduleSync(delay = SYNC_DELAY_MS) {
  window.clearTimeout(syncTimer);
  syncState.value = "waiting";
  syncTimer = window.setTimeout(sync, delay);
}

watch(
  () => [model.notes, model.extras, model.lengthTicks],
  () => {
    revision++;
    if (autoSync.value && unsynced.value && editable.value) scheduleSync();
  },
  { deep: true },
);

async function sync() {
  const p = source.value;
  if (!p || !unsynced.value) return;
  // Not in the middle of a drag, nor alongside another write.
  if (drag || saving.value) return scheduleSync(300);
  const sent = revision;
  saving.value = true;
  syncState.value = "sending";
  try {
    const backup = await api.patternWrite({
      slot: p.slot,
      project: p.project,
      lengthTicks: model.lengthTicks,
      bars: model.barsChanged ? model.bars : null,
      trailer: p.trailer,
      notes: model.notes,
      extras: model.extras,
      backup: !backupKept,
    });
    if (backup) notify(`Synchro auto : l'ancienne version de ${p.label} est gardée dans ${backup}`, "info");
    backupKept = true;
    store.patterns[p.slot] = true;
    p.exists = true;
    syncState.value = "done";
    if (revision === sent) unsynced.value = false;
    else scheduleSync();
  } catch (e) {
    syncState.value = "error";
    handleError(e);
    setAutoSync(false);
    notify("Synchro auto désactivée après une erreur. Utilise « Envoyer vers la 404 ».");
  } finally {
    saving.value = false;
  }
}

const syncLabel = computed(
  () =>
    ({ idle: "", waiting: "en attente…", sending: "envoi…", done: "à jour", error: "erreur" })[syncState.value],
);

// ---- MIDI import ----

const importing = ref(false);

/** Put an imported sequence on `pad`: its notes there are replaced and other pads stay as
 * they are. The pattern grows to fit the sequence if it has to; with `repeat`, what the
 * other pads played is copied into the new bars. */
function applyImport(plan: ImportPlan, pad: number, repeat: boolean) {
  importing.value = false;
  checkpoint();
  const others = model.notes.filter((n) => n.pad !== pad);
  const kept = [...others];
  if (plan.bars > model.bars) {
    const oldLength = model.lengthTicks;
    const newLength = plan.bars * BAR_TICKS;
    if (repeat && oldLength > 0) {
      const copies: EditNote[] = [];
      for (let offset = oldLength; offset < newLength; offset += oldLength) {
        for (const n of others) copies.push({ ...n, tick: n.tick + offset, order: nextOrder++ });
      }
      kept.push(...clip(copies, newLength));
    }
    model.lengthTicks = newLength;
    model.bars = plan.bars;
    model.barsChanged = true;
  }
  // Number the imported notes after any copies, so every note has its own order.
  model.notes = kept.concat(plan.notes.map((n) => ({ ...n, order: nextOrder++ })));
  viewBank.value = Math.floor(pad / 16);
  rollPad.value = pad;
  selected.value = null;
  notify(`${plan.notes.length} notes importées sur ${padLabel(pad)}. ${autoSync.value ? "" : " Rien n'est envoyé avant « Envoyer vers la 404 »."}`, "info");
}

/** Notes each pad plays in the pattern, by pad index. */
const notesPerPad = computed(() => {
  const c = new Array(160).fill(0);
  for (const n of model.notes) c[n.pad]++;
  return c;
});

async function close() {
  // A pending automatic sync goes out before closing.
  if (autoSync.value && unsynced.value) {
    window.clearTimeout(syncTimer);
    while (saving.value) await new Promise((r) => setTimeout(r, 100));
    await sync();
  }
  if (unsynced.value) {
    const ok = await ask("Fermer l'éditeur ?", "Les modifications non envoyées seront perdues.", [
      { label: "Continuer", value: "no" },
      { label: "Fermer sans envoyer", value: "yes", kind: "danger" },
    ]);
    if (ok !== "yes") return;
  }
  emit("close");
}

/** Back to the pattern as it was opened. If it was synced since, the device gets it back
 * too (at once with "Synchro auto", otherwise with "Envoyer vers la 404"). */
async function revert() {
  if (!source.value) return;
  const wasSent = backupKept;
  load(source.value);
  if (wasSent) {
    unsynced.value = true;
    if (autoSync.value) scheduleSync(0);
  }
}

open();

// The grid: a line on every grid step, stronger on beats, strongest on bars. Every other
// beat is shaded so the pulse reads at a glance.
const gridStyle = computed(() => {
  const step = px(grid.value);
  const beat = px(480);
  const bar = px(BAR_TICKS);
  return {
    width: `${width.value}px`,
    height: `${rows.value.length * ROW}px`,
    backgroundImage: [
      `repeating-linear-gradient(to right, var(--line-strong) 0 1px, transparent 1px ${bar}px)`,
      `repeating-linear-gradient(to right, var(--line) 0 1px, transparent 1px ${beat}px)`,
      grid.value < 480 && step >= 5
        ? `repeating-linear-gradient(to right, rgba(255,255,255,0.05) 0 1px, transparent 1px ${step}px)`
        : "none",
      `repeating-linear-gradient(to right, transparent 0 ${beat}px, rgba(255,255,255,0.018) ${beat}px ${2 * beat}px)`,
      `repeating-linear-gradient(to bottom, transparent 0 ${ROW - 1}px, var(--line) ${ROW - 1}px ${ROW}px)`,
    ].join(","),
  };
});

// A note is drawn from its 1/32 line to where it really ends; `--h` places the mark of
// where it really starts.
const noteStyle = (n: EditNote) => {
  const anchor = anchorOf(n, model.lengthTicks);
  return {
    left: `${px(anchor)}px`,
    top: `${rowOf(n) * ROW + 3}px`,
    width: `${Math.max(3, px(n.tick + duration(n) - anchor) - 1)}px`,
    height: `${ROW - 6}px`,
    "--v": String(n.velocity / 127),
    "--h": `${px(n.tick - anchor)}px`,
  };
};

</script>

<template>
  <div class="editor panel">
    <header class="bar">
      <span class="slot mono">{{ source?.label ?? padLabel(slotIndex) }}</span>
      <span v-if="source" class="muted small">
        Projet {{ source.project }} · {{ bpm(source.bankTempo) }} BPM{{ source.exists ? "" : " · nouveau" }}
      </span>

      <div class="group">
        <span class="label">Vue</span>
        <div class="seg">
          <button :class="{ on: view === 'pads' }" @click="view = 'pads'">Pads</button>
          <button :class="{ on: view === 'roll' }" @click="showRoll()">Piano roll</button>
        </div>
      </div>

      <div class="group">
        <span class="label">Mesures</span>
        <div class="seg">
          <button
            v-for="b in BAR_CHOICES"
            :key="b"
            :class="{ on: model.lengthTicks === b * BAR_TICKS }"
            :disabled="!editable"
            @click="setBars(b)"
          >
            {{ b }}
          </button>
        </div>
      </div>

      <div class="group">
        <span class="label">Grille</span>
        <div class="seg">
          <button v-for="g in GRIDS" :key="g.ticks" :class="{ on: grid === g.ticks }" @click="setGrid(g.ticks)">
            {{ g.label }}
          </button>
        </div>
      </div>

      <div class="group">
        <span class="label">Zoom</span>
        <input type="range" min="6" max="30" :value="cell" @input="setCell(Number(($event.target as HTMLInputElement).value))" />
      </div>

      <label class="group check">
        <input v-model="listen" type="checkbox" />
        <span>Écouter sur la 404</span>
      </label>

      <label class="group check" title="Envoie le pattern à la 404 une seconde après chaque modification">
        <input type="checkbox" :checked="autoSync" :disabled="!editable" @change="setAutoSync(($event.target as HTMLInputElement).checked)" />
        <span>Synchro auto</span>
        <span v-if="autoSync && syncLabel" class="sync muted small" :class="syncState">{{ syncLabel }}</span>
      </label>

      <div class="spacer" />
      <button :disabled="!editable || saving" @click="importing = true">Importer MIDI…</button>
      <button :disabled="!dirty || saving" @click="revert">Tout annuler</button>
      <button class="primary" :disabled="!editable || saving || store.pending > 0" @click="send">
        <span v-if="saving" class="spinner" /> Envoyer vers la 404
      </button>
      <button @click="close">Fermer</button>
    </header>

    <div class="banks">
      <template v-if="view === 'pads'">
        <button
          v-for="(letter, b) in BANK_LETTERS"
          :key="letter"
          :class="{ on: viewBank === b }"
          @click="viewBank = b"
        >
          {{ letter }}<span v-if="perBank[b]" class="count">{{ perBank[b] }}</span>
        </button>
      </template>
      <label v-else class="group">
        <span class="label">Pad</span>
        <select v-model.number="rollPad" class="mono" @change="selected = null">
          <optgroup v-for="(letter, b) in BANK_LETTERS" :key="letter" :label="`Banque ${letter}`">
            <option v-for="r in 16" :key="r" :value="b * 16 + r - 1">
              {{ padLabel(b * 16 + r - 1) }}{{ store.pads[b * 16 + r - 1]?.name ? ` · ${store.pads[b * 16 + r - 1].name}` : ""
              }}{{ notesPerPad[b * 16 + r - 1] ? ` (${notesPerPad[b * 16 + r - 1]})` : "" }}
            </option>
          </optgroup>
        </select>
      </label>
      <div class="spacer" />
      <template v-if="selected">
        <span class="muted small mono">
          {{ padLabel(selected.pad) }} · {{ position(anchorOf(selected, model.lengthTicks)) }}
        </span>
        <label class="group">
          <span class="label">Vélocité</span>
          <input
            type="range"
            min="1"
            max="127"
            :value="selected.velocity"
            @change="setVelocity(Number(($event.target as HTMLInputElement).value))"
          />
          <span class="mono small num">{{ selected.velocity }}</span>
        </label>
        <label class="group">
          <span class="label">Longueur ({{ gridLabel }})</span>
          <input
            class="steps mono"
            type="number"
            min="1"
            :value="Number.isInteger(selectedSteps) ? selectedSteps : selectedSteps.toFixed(2)"
            @change="setSteps(Number(($event.target as HTMLInputElement).value))"
          />
        </label>
        <span v-if="semitonesOf(selected.pitch) !== 0" class="muted small mono" title="Hauteur de la note, en demi-tons">
          {{ semitonesOf(selected.pitch) === null ? "hauteur ?" : `${semitonesOf(selected.pitch)! > 0 ? "+" : ""}${semitonesOf(selected.pitch)} dt` }}
        </span>
        <label class="group" title="Écart avec la ligne de 1/32 la plus proche : négatif = en avance, positif = en retard">
          <span class="label">Humanize (ms)</span>
          <input
            class="steps mono"
            type="number"
            step="1"
            :value="Math.round(ticksToMs(selectedHumanize, tempo))"
            @change="setHumanize(Number(($event.target as HTMLInputElement).value))"
          />
          <span class="muted small mono">{{ selectedHumanize > 0 ? "+" : "" }}{{ selectedHumanize }} t</span>
        </label>
      </template>
      <span v-else class="muted small">
        Clic : ajouter · glisser : déplacer · bord droit : longueur · clic droit : supprimer · Ctrl+Z
      </span>
    </div>

    <MidiImport
      v-if="importing"
      :pad="selected?.pad ?? viewBank * 16"
      :first-order="nextOrder"
      :current-bars="model.bars"
      :existing="notesPerPad"
      @import="applyImport"
      @cancel="importing = false"
    />

    <div v-if="loading" class="placeholder muted"><span class="spinner" /> Lecture du pattern…</div>
    <div v-else-if="source && !editable" class="placeholder muted">
      Ce pattern n'est pas en 4/4 : l'éditeur ne gère que le 4/4 pour l'instant.
    </div>
    <div v-else class="scroller">
      <div class="labels">
        <div class="ruler-gap" />
        <div
          v-for="(row, r) in rows"
          :key="r"
          class="row-label"
          :class="{
            empty: row.semis === null && store.pads.length > 0 && !store.pads[row.pad]?.hasSample,
            black: row.semis !== null && isBlack(row.semis),
            root: row.semis === 0,
          }"
          :style="{ height: `${ROW}px` }"
          :title="row.semis === null ? 'Maintenir pour écouter. Double-clic : piano roll de ce pad' : 'Maintenir pour écouter le pad'"
          @pointerdown="audition(row.pad, null)"
          @pointerup="stopPreview"
          @pointerleave="stopPreview"
          @dblclick="row.semis === null && showRoll(row.pad)"
        >
          <template v-if="row.semis === null">
            <span class="mono">{{ padLabel(row.pad) }}</span>
            <span class="name">{{ store.pads[row.pad]?.name ?? "" }}</span>
          </template>
          <template v-else>
            <span class="mono">{{ noteName(row.semis) }}</span>
            <span class="name mono">{{ row.semis > 0 ? "+" : "" }}{{ row.semis }}</span>
          </template>
        </div>
      </div>
      <div class="track">
        <div class="ruler" :style="{ width: `${width}px` }">
          <span
            v-for="b in model.bars"
            :key="b"
            class="bar-num mono"
            :style="{ left: `${px((b - 1) * BAR_TICKS)}px` }"
          >
            {{ b }}
          </span>
        </div>
        <div
          ref="lanes"
          class="lanes"
          :style="gridStyle"
          @pointerdown="onDown"
          @pointermove="onMove"
          @pointerup="onUp"
          @pointercancel="onUp"
          @contextmenu="onContext"
        >
          <template v-if="view === 'roll'">
            <div
              v-for="(row, r) in rows"
              :key="`k${r}`"
              class="key-row"
              :class="{ black: isBlack(row.semis!), root: row.semis === 0 }"
              :style="{ top: `${r * ROW}px`, height: `${ROW}px` }"
            />
          </template>
          <div
            v-for="(n, i) in visible"
            :key="`${n.order}-${i}`"
            :data-note="i"
            class="note"
            :class="{ selected: n === selected }"
            :style="noteStyle(n)"
            @dblclick="remove(n)"
          >
            <span class="vel" />
            <template v-if="humanizeOf(n, model.lengthTicks) !== 0">
              <span class="onset" />
              <span v-if="px(duration(n)) >= 26" class="hum mono">{{ ms(humanizeOf(n, model.lengthTicks)) }}</span>
            </template>
            <span data-handle class="handle" />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.editor {
  display: flex;
  flex-direction: column;
  min-height: 0;
  height: 100%;
  overflow: hidden;
}

.bar,
.banks {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 14px;
  border-bottom: 1px solid var(--line);
  flex-wrap: wrap;
}

.slot {
  font-size: 20px;
  font-weight: 600;
  color: var(--teal);
}

.small {
  font-size: 12px;
}

.group {
  display: flex;
  align-items: center;
  gap: 6px;
}

.check {
  cursor: pointer;
  font-size: 13px;
}

.spacer {
  flex: 1;
}

.seg {
  display: flex;
}

.seg button,
.banks > button {
  padding: 3px 9px;
  border-radius: 0;
}

.seg button:first-child {
  border-radius: 6px 0 0 6px;
}

.seg button:last-child {
  border-radius: 0 6px 6px 0;
}

.seg button.on,
.banks > button.on {
  background: var(--teal-soft);
  color: var(--teal);
  border-color: rgba(69, 212, 184, 0.45);
}

.banks > button {
  border-radius: 6px;
  min-width: 34px;
  font-family: var(--mono);
}

.count {
  margin-left: 4px;
  font-size: 10px;
  color: var(--muted);
}

.num {
  width: 26px;
  text-align: right;
}

.steps {
  width: 64px;
}

.placeholder {
  margin: auto;
  display: flex;
  align-items: center;
  gap: 10px;
}

.scroller {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  align-items: flex-start;
}

.labels {
  position: sticky;
  left: 0;
  z-index: 2;
  background: var(--panel);
  border-right: 1px solid var(--line);
  width: 150px;
  flex-shrink: 0;
}

.ruler-gap,
.ruler {
  height: 22px;
  border-bottom: 1px solid var(--line);
}

.row-label {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 0 8px;
  font-size: 12px;
  border-bottom: 1px solid var(--line);
  box-sizing: border-box;
  cursor: pointer;
  user-select: none;
}

.row-label:hover {
  background: var(--panel-2);
}

.row-label.empty {
  color: var(--faint);
}

/* Piano roll: black keys darker, the sample's own pitch marked. */
.row-label.black {
  background: rgba(0, 0, 0, 0.22);
}

.row-label.root {
  color: var(--teal);
}

.key-row {
  position: absolute;
  left: 0;
  right: 0;
  pointer-events: none;
}

.key-row.black {
  background: rgba(0, 0, 0, 0.18);
}

.key-row.root {
  background: rgba(69, 212, 184, 0.06);
}

.sync.sending,
.sync.waiting {
  color: var(--accent);
}

.sync.done {
  color: var(--ok);
}

.sync.error {
  color: var(--danger);
}

.row-label .name {
  color: var(--muted);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.track {
  flex-shrink: 0;
}

.ruler {
  position: relative;
}

.bar-num {
  position: absolute;
  top: 3px;
  padding-left: 4px;
  font-size: 11px;
  color: var(--muted);
  border-left: 1px solid var(--line-strong);
  height: 18px;
}

.lanes {
  position: relative;
  cursor: crosshair;
  touch-action: none;
  user-select: none;
}

.note {
  position: absolute;
  box-sizing: border-box;
  border-radius: 3px;
  background: color-mix(in srgb, var(--teal) calc(35% + var(--v) * 65%), var(--panel-2));
  border: 1px solid rgba(69, 212, 184, 0.7);
  cursor: grab;
}

/* Where the note really starts, when it is off its 1/32 line. */
.onset {
  position: absolute;
  top: -3px;
  bottom: -3px;
  left: var(--h);
  width: 2px;
  background: var(--accent);
  pointer-events: none;
}

.hum {
  position: absolute;
  right: 7px;
  top: 50%;
  transform: translateY(-50%);
  font-size: 9px;
  line-height: 1;
  color: rgba(0, 0, 0, 0.7);
  pointer-events: none;
}

.note.selected {
  border-color: var(--accent);
  box-shadow: 0 0 0 1px var(--accent);
}

/* Velocity as a bar along the bottom of the note. */
.vel {
  position: absolute;
  left: 0;
  bottom: 0;
  height: 3px;
  width: calc(var(--v) * 100%);
  background: rgba(0, 0, 0, 0.35);
  pointer-events: none;
}

.handle {
  position: absolute;
  right: 0;
  top: 0;
  bottom: 0;
  width: 6px;
  cursor: ew-resize;
}

.note:hover .handle {
  background: rgba(255, 255, 255, 0.25);
}
</style>
