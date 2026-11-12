<script setup lang="ts">
// Import a MIDI file onto one pad of the pattern being edited. Shows how the notes fit the
// SP-404MKII's two octaves before anything changes, and lets the sequence be transposed.
import { computed, ref } from "vue";
import { BANK_LETTERS, padLabel } from "../format";
import { parseMidi, planImport, type ImportPlan, type MidiFile } from "../midiImport";
import { store } from "../store";

const props = defineProps<{ pad: number; firstOrder: number; currentBars: number }>();
const emit = defineEmits<{ import: [plan: ImportPlan, pad: number]; cancel: [] }>();

const file = ref<MidiFile | null>(null);
const fileName = ref("");
const problem = ref("");
const pad = ref(props.pad);
const reference = ref(60);
const transpose = ref(0);

const plan = computed(() =>
  file.value ? planImport(file.value, pad.value, reference.value, transpose.value, props.firstOrder) : null,
);
const bars = computed(() => Math.max(props.currentBars, plan.value?.bars ?? 0));

async function choose(e: Event) {
  const input = e.target as HTMLInputElement;
  const f = input.files?.[0];
  input.value = "";
  if (!f) return;
  problem.value = "";
  try {
    const parsed = parseMidi(await f.arrayBuffer());
    if (!parsed.notes.length) throw new Error("aucune note dans ce fichier");
    file.value = parsed;
    fileName.value = f.name;
    transpose.value = 0;
  } catch (err) {
    file.value = null;
    problem.value = `Lecture impossible : ${err instanceof Error ? err.message : String(err)}.`;
  }
}

/** The transpose that leaves the fewest notes out of range: whole octaves first, then
 * the smallest shift. A melody wider than two octaves keeps some notes out, folded. */
function fit() {
  const f = file.value;
  if (!f) return;
  let best = { t: 0, out: Infinity, octave: false };
  for (let t = -36; t <= 36; t++) {
    const out = planImport(f, pad.value, reference.value, t, 0).outOfRange;
    const octave = t % 12 === 0;
    const better =
      out < best.out ||
      (out === best.out && octave && !best.octave) ||
      (out === best.out && octave === best.octave && Math.abs(t) < Math.abs(best.t));
    if (better) best = { t, out, octave };
  }
  transpose.value = best.t;
}

const signed = (n: number) => (n > 0 ? `+${n}` : n < 0 ? `−${-n}` : "0");
const pads = Array.from({ length: 160 }, (_, i) => i);
</script>

<template>
  <div class="backdrop" @click.self="emit('cancel')">
    <div class="dialog panel" role="dialog" aria-modal="true">
      <h2>Importer une séquence MIDI</h2>

      <label class="file">
        <input type="file" accept=".mid,.midi,audio/midi" @change="choose" />
        <span class="button">Choisir un fichier…</span>
        <span class="muted small">{{ fileName || "Aucun fichier" }}</span>
      </label>
      <p v-if="problem" class="warn">{{ problem }}</p>

      <div class="rows">
        <label>
          <span class="label">Pad qui joue la séquence</span>
          <select v-model.number="pad" class="mono">
            <optgroup v-for="(letter, b) in BANK_LETTERS" :key="letter" :label="`Banque ${letter}`">
              <option v-for="i in pads.slice(b * 16, b * 16 + 16)" :key="i" :value="i">
                {{ padLabel(i) }}{{ store.pads[i]?.name ? ` · ${store.pads[i].name}` : "" }}
              </option>
            </optgroup>
          </select>
        </label>
        <label>
          <span class="label">Note du sample (hauteur d'origine)</span>
          <input v-model.number="reference" class="num mono" type="number" min="0" max="127" />
        </label>
        <div>
          <span class="label">Transposer la séquence</span>
          <div class="transpose">
            <button @click="transpose -= 12">−12</button>
            <button @click="transpose -= 1">−1</button>
            <span class="mono value">{{ signed(transpose) }}</span>
            <button @click="transpose += 1">+1</button>
            <button @click="transpose += 12">+12</button>
          </div>
        </div>
      </div>

      <template v-if="plan">
        <div v-if="plan.outOfRange" class="alert">
          <strong>{{ plan.outOfRange }} note{{ plan.outOfRange > 1 ? "s sortent" : " sort" }} de l'ambitus de la 404</strong>
          (de {{ signed(plan.lowest) }} à {{ signed(plan.highest) }} demi-tons ; la 404 joue de −12 à +12).
          Transpose la séquence, sinon {{ plan.outOfRange > 1 ? "elles seront ramenées" : "elle sera ramenée" }} d'une
          octave.
          <button class="fit" @click="fit">Transposer automatiquement</button>
        </div>
        <ul class="summary small">
          <li>{{ plan.notes.length }} notes sur {{ padLabel(pad) }}, de {{ signed(plan.lowest) }} à {{ signed(plan.highest) }} demi-tons.</li>
          <li>Pattern de {{ bars }} mesures{{ bars > currentBars ? ` (allongé, il en a ${currentBars})` : "" }}.</li>
          <li v-if="plan.chordNotes">
            {{ plan.chordNotes }} note{{ plan.chordNotes > 1 ? "s" : "" }} d'accords ignorée{{ plan.chordNotes > 1 ? "s" : "" }} : un pad joue une note à la fois (la plus haute est gardée).
          </li>
          <li v-if="plan.tooLate">{{ plan.tooLate }} note{{ plan.tooLate > 1 ? "s" : "" }} après 16 mesures ignorée{{ plan.tooLate > 1 ? "s" : "" }}.</li>
          <li>Les notes déjà présentes sur {{ padLabel(pad) }} sont remplacées ; les autres pads ne changent pas.</li>
          <li class="muted">Le tempo du fichier est ignoré : le pattern joue au tempo de la banque.</li>
        </ul>
      </template>

      <div class="buttons">
        <button @click="emit('cancel')">Annuler</button>
        <button class="primary" :disabled="!plan || !plan.notes.length" @click="plan && emit('import', plan, pad)">
          Importer
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.backdrop {
  position: fixed;
  inset: 0;
  z-index: 20;
  display: grid;
  place-items: center;
  background: rgba(5, 6, 8, 0.6);
}

.dialog {
  width: 520px;
  max-width: calc(100vw - 32px);
  padding: 20px;
  box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
  display: flex;
  flex-direction: column;
  gap: 14px;
}

h2 {
  margin: 0;
  font-size: 16px;
}

.small {
  font-size: 12px;
}

.file {
  display: flex;
  align-items: center;
  gap: 10px;
  cursor: pointer;
}

.file input {
  display: none;
}

.button {
  padding: 5px 12px;
  border: 1px solid var(--line-strong);
  border-radius: 6px;
  background: var(--panel-2);
}

.rows {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}

.rows > :first-child {
  grid-column: 1 / -1;
}

.rows label,
.rows > div {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.num {
  width: 80px;
}

.transpose {
  display: flex;
  align-items: center;
  gap: 4px;
}

.transpose button {
  padding: 3px 8px;
}

.value {
  min-width: 34px;
  text-align: center;
}

.alert {
  padding: 10px 12px;
  border-radius: 6px;
  border: 1px solid var(--accent-line);
  background: var(--accent-soft);
  font-size: 13px;
  line-height: 1.45;
}

.fit {
  display: block;
  margin-top: 8px;
}

.warn {
  margin: 0;
  color: var(--danger);
  font-size: 13px;
}

.summary {
  margin: 0;
  padding-left: 18px;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.buttons {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
</style>
