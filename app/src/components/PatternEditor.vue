<script setup lang="ts">
// Step editor for one pattern: 16 pads of a bank against a 1/32 grid.
//
// Click an empty cell to add a note, drag a note to move it, drag its right edge to
// change its length, right-click (or double-click) to delete it. The selected note's
// velocity and length are also set in the bar above the grid; the arrow keys nudge it,
// Delete removes it, Ctrl+Z undoes. Notes sound on the SP-404MKII as they are placed.
// Nothing reaches the device until "Envoyer vers la 404".
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { api, type EditNote, type EditablePattern, type ExtraRecord } from "../api";
import { BANK_LETTERS, bpm, padLabel } from "../format";
import { ask, handleError, notify, store } from "../store";
import {
  STEP,
  STEPS_PER_BAR,
  addNote,
  clip,
  duration,
  move,
  noteAt,
  position,
  resize,
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
const dirty = ref(false);
const selected = ref<EditNote | null>(null);
const viewBank = ref(Math.floor(props.slotIndex / 16));
const cell = ref<number>(Number(localStorage.getItem("sparkymk2.editor.cell")) || 14);
const listen = ref(true);
/** Length and velocity of the last note placed or edited: the next one starts like it. */
const lastSteps = ref(2);
const lastVelocity = ref(100);
let nextOrder = 1_000_000;

const steps = computed(() => Math.ceil(model.lengthTicks / STEP));
const width = computed(() => steps.value * cell.value);
const rows = computed(() => Array.from({ length: 16 }, (_, r) => viewBank.value * 16 + r));
const visible = computed(() => model.notes.filter((n) => Math.floor(n.pad / 16) === viewBank.value));
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
}

function undo() {
  const s = history.pop();
  if (!s) return;
  Object.assign(model, s);
  selected.value = null;
  dirty.value = true;
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
  | { kind: "move"; note: EditNote; x: number; y: number; step: number; row: number; moved: boolean }
  /** `saved`: an undo step already covers this drag (the note was just added). */
  | { kind: "resize"; note: EditNote; moved: boolean; saved: boolean };
let drag: Drag | null = null;
const lanes = ref<HTMLElement | null>(null);

function locate(e: PointerEvent) {
  const box = lanes.value!.getBoundingClientRect();
  const x = e.clientX - box.left;
  const y = e.clientY - box.top;
  return { x, y, step: Math.floor(x / cell.value), row: Math.floor(y / ROW) };
}

function onDown(e: PointerEvent) {
  if (!editable.value || e.button !== 0) return;
  const at = locate(e);
  if (at.row < 0 || at.row > 15) return;
  const pad = rows.value[at.row];
  const target = e.target as HTMLElement;
  const index = target.closest<HTMLElement>("[data-note]")?.dataset.note;
  lanes.value!.setPointerCapture(e.pointerId);

  if (index !== undefined) {
    const note = visible.value[Number(index)];
    selected.value = note;
    if (target.dataset.handle !== undefined) {
      drag = { kind: "resize", note, moved: false, saved: false };
    } else {
      drag = { kind: "move", note, x: at.x, y: at.y, step: Math.round(note.tick / STEP), row: at.row, moved: false };
      audition(note.pad, duration(note));
    }
    return;
  }

  // Empty space. A cell partly covered by a note off the grid selects that note.
  const hit = noteAt(model.notes, pad, at.step * STEP);
  if (hit) {
    selected.value = hit;
    return;
  }
  checkpoint();
  const note = addNote(
    model.notes,
    pad,
    at.step,
    lastSteps.value,
    lastVelocity.value,
    { pitch: source.value!.defaultPitch, order: nextOrder++ },
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
    const endStep = Math.round(at.x / cell.value);
    const before = drag.note.gate;
    if (!drag.moved) {
      if (endStep * STEP === drag.note.tick + duration(drag.note)) return;
      if (!drag.saved) checkpoint();
      drag.moved = true;
    }
    resize(model.notes, drag.note, endStep, model.lengthTicks);
    if (drag.note.gate !== before) lastSteps.value = Math.max(1, Math.round(duration(drag.note) / STEP));
    return;
  }
  const ds = Math.round((at.x - drag.x) / cell.value);
  const dr = Math.round((at.y - drag.y) / ROW);
  if (!drag.moved && ds === 0 && dr === 0) return;
  const row = Math.min(15, Math.max(0, drag.row + dr));
  const pad = rows.value[row];
  const step = drag.step + ds;
  const wasPad = drag.note.pad;
  const wasTick = drag.note.tick;
  if (step * STEP === wasTick && pad === wasPad) return;
  if (!drag.moved) {
    checkpoint();
    drag.moved = true;
  }
  if (move(model.notes, drag.note, step, pad, model.lengthTicks) && pad !== wasPad) {
    audition(pad, duration(drag.note));
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

function setSteps(s: number) {
  const n = selected.value;
  if (!n || !Number.isFinite(s)) return;
  checkpoint();
  resize(model.notes, n, Math.floor(n.tick / STEP) + Math.max(1, Math.round(s)), model.lengthTicks);
  lastSteps.value = Math.max(1, Math.round(duration(n) / STEP));
}

const selectedSteps = computed(() => (selected.value ? duration(selected.value) / STEP : 0));

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
    const step = Math.round(n.tick / STEP) + (e.key === "ArrowRight" ? 1 : -1);
    if (!move(model.notes, n, step, n.pad, model.lengthTicks)) history.pop();
  }
}

onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKey);
  stopPreview();
});

// ---- length, zoom, sending ----

async function setBars(bars: number) {
  const length = bars * STEPS_PER_BAR * STEP;
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
  localStorage.setItem("sparkymk2.editor.cell", String(v));
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

async function close() {
  if (dirty.value) {
    const ok = await ask("Fermer l'éditeur ?", "Les modifications non envoyées seront perdues.", [
      { label: "Continuer", value: "no" },
      { label: "Fermer sans envoyer", value: "yes", kind: "danger" },
    ]);
    if (ok !== "yes") return;
  }
  emit("close");
}

async function revert() {
  if (source.value) load(source.value);
}

open();

// The grid: faint lines every step, stronger on beats, strongest on bars.
const gridStyle = computed(() => {
  const c = cell.value;
  return {
    width: `${width.value}px`,
    height: `${16 * ROW}px`,
    backgroundImage: [
      `repeating-linear-gradient(to right, var(--line-strong) 0 1px, transparent 1px ${c * STEPS_PER_BAR}px)`,
      `repeating-linear-gradient(to right, var(--line) 0 1px, transparent 1px ${c * 8}px)`,
      c >= 8 ? `repeating-linear-gradient(to right, rgba(255,255,255,0.035) 0 1px, transparent 1px ${c}px)` : "none",
      `repeating-linear-gradient(to bottom, transparent 0 ${ROW - 1}px, var(--line) ${ROW - 1}px ${ROW}px)`,
    ].join(","),
  };
});

const noteStyle = (n: EditNote) => ({
  left: `${(n.tick / STEP) * cell.value}px`,
  top: `${(n.pad % 16) * ROW + 3}px`,
  width: `${Math.max(3, (duration(n) / STEP) * cell.value - 1)}px`,
  height: `${ROW - 6}px`,
  "--v": String(n.velocity / 127),
});
</script>

<template>
  <div class="editor panel">
    <header class="bar">
      <span class="slot mono">{{ source?.label ?? padLabel(slotIndex) }}</span>
      <span v-if="source" class="muted small">
        Projet {{ source.project }} · {{ bpm(source.bankTempo) }} BPM{{ source.exists ? "" : " · nouveau" }}
      </span>

      <div class="group">
        <span class="label">Mesures</span>
        <div class="seg">
          <button
            v-for="b in BAR_CHOICES"
            :key="b"
            :class="{ on: model.lengthTicks === b * STEPS_PER_BAR * STEP }"
            :disabled="!editable"
            @click="setBars(b)"
          >
            {{ b }}
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

      <div class="spacer" />
      <button :disabled="!dirty || saving" @click="revert">Tout annuler</button>
      <button class="primary" :disabled="!editable || saving || store.pending > 0" @click="send">
        <span v-if="saving" class="spinner" /> Envoyer vers la 404
      </button>
      <button @click="close">Fermer</button>
    </header>

    <div class="banks">
      <button
        v-for="(letter, b) in BANK_LETTERS"
        :key="letter"
        :class="{ on: viewBank === b }"
        @click="viewBank = b"
      >
        {{ letter }}<span v-if="perBank[b]" class="count">{{ perBank[b] }}</span>
      </button>
      <div class="spacer" />
      <template v-if="selected">
        <span class="muted small mono">{{ padLabel(selected.pad) }} · {{ position(selected.tick) }}</span>
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
          <span class="label">Longueur (1/32)</span>
          <input
            class="steps mono"
            type="number"
            min="1"
            :value="Number.isInteger(selectedSteps) ? selectedSteps : selectedSteps.toFixed(2)"
            @change="setSteps(Number(($event.target as HTMLInputElement).value))"
          />
        </label>
      </template>
      <span v-else class="muted small">
        Clic : ajouter · glisser : déplacer · bord droit : longueur · clic droit : supprimer · Ctrl+Z
      </span>
    </div>

    <div v-if="loading" class="placeholder muted"><span class="spinner" /> Lecture du pattern…</div>
    <div v-else-if="source && !editable" class="placeholder muted">
      Ce pattern n'est pas en 4/4 : l'éditeur ne gère que le 4/4 pour l'instant.
    </div>
    <div v-else class="scroller">
      <div class="labels">
        <div class="ruler-gap" />
        <div
          v-for="pad in rows"
          :key="pad"
          class="row-label"
          :class="{ empty: store.pads.length > 0 && !store.pads[pad]?.hasSample }"
          :style="{ height: `${ROW}px` }"
          title="Maintenir pour écouter"
          @pointerdown="audition(pad, null)"
          @pointerup="stopPreview"
          @pointerleave="stopPreview"
        >
          <span class="mono">{{ padLabel(pad) }}</span>
          <span class="name">{{ store.pads[pad]?.name ?? "" }}</span>
        </div>
      </div>
      <div class="track">
        <div class="ruler" :style="{ width: `${width}px` }">
          <span
            v-for="b in model.bars"
            :key="b"
            class="bar-num mono"
            :style="{ left: `${(b - 1) * STEPS_PER_BAR * cell}px` }"
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
          <div
            v-for="(n, i) in visible"
            :key="`${n.order}-${i}`"
            :data-note="i"
            class="note"
            :class="{ selected: n === selected, off: n.tick % STEP !== 0 }"
            :style="noteStyle(n)"
            @dblclick="remove(n)"
          >
            <span class="vel" />
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
  overflow: hidden;
}

.note.off {
  border-style: dashed;
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
