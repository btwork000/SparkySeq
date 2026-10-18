//! Pattern editor: open a pattern of the current project as editable notes, and write an
//! edited pattern back (`sp404_formats::pattern_edit`).
//!
//! Every record the editor does not show (control changes, records nobody understands
//! yet) travels to the frontend and back untouched, as `extras`, and so do the bytes of a
//! note the editor does not change (pitch, note-off velocity, flags).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sp404_device::pattern_path;
use sp404_formats::pattern_edit::{BAR_TICKS, EditablePattern, PITCH_ORIGINAL, Record};
use tauri::{AppHandle, Manager, State};

use crate::AppState;
use crate::commands::{CmdResult, Failure, pad_index, with_device};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EditNote {
    tick: u32,
    /// 0..160, bank × 16 + pad.
    pad: u16,
    velocity: u8,
    /// Stored gate: duration in ticks minus one.
    gate: u16,
    pitch: u8,
    off_velocity: u8,
    flags: u8,
    /// Position in the file, to keep the order of records on the same tick.
    order: u32,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ExtraRecord {
    tick: u32,
    bytes: [u8; 8],
    order: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditablePatternDto {
    slot: u16,
    label: String,
    /// Whether the slot held a pattern; an empty slot opens as a new 4-bar pattern.
    exists: bool,
    /// 1-based.
    project: u8,
    length_ticks: u32,
    bars: u32,
    bar_ticks: u32,
    four_four: bool,
    /// Bank tempo of the slot's bank, BPM × 100.
    bank_tempo: u16,
    trailer: [u8; 8],
    notes: Vec<EditNote>,
    extras: Vec<ExtraRecord>,
    /// Byte 3 for a new note without transposition.
    default_pitch: u8,
}

fn to_dto(
    slot: u16,
    label: String,
    exists: bool,
    project: u8,
    bank_tempo: u16,
    p: &EditablePattern,
) -> EditablePatternDto {
    let mut notes = Vec::new();
    let mut extras = Vec::new();
    for (i, r) in p.records.iter().enumerate() {
        let order = i as u32;
        match r.note_pad() {
            Some(pad) => notes.push(EditNote {
                tick: r.tick,
                pad: pad.index(),
                velocity: r.bytes[4],
                gate: u16::from_le_bytes([r.bytes[6], r.bytes[7]]),
                pitch: r.bytes[3],
                off_velocity: r.bytes[5],
                flags: r.bytes[2] & 0xF0,
                order,
            }),
            None => extras.push(ExtraRecord {
                tick: r.tick,
                bytes: r.bytes,
                order,
            }),
        }
    }
    EditablePatternDto {
        slot,
        label,
        exists,
        project,
        length_ticks: p.length,
        bars: p.bars(),
        bar_ticks: BAR_TICKS,
        four_four: p.is_four_four(),
        bank_tempo,
        trailer: p.trailer,
        notes,
        extras,
        default_pitch: PITCH_ORIGINAL,
    }
}

/// Open the pattern in `slot` of the current project for editing.
#[tauri::command]
pub async fn pattern_open(state: State<'_, AppState>, slot: u16) -> CmdResult<EditablePatternDto> {
    with_device(&state, move |dev| {
        let slot_index = pad_index(slot)?;
        let project = dev.current_project()? + 1;
        let settings = dev.project_settings(project - 1)?;
        let tempo = settings.bank(usize::from(slot_index.bank())).tempo;
        let exists = dev.pattern_exists(slot_index)?;
        let pattern = if exists {
            EditablePattern::parse(&dev.read_file(&pattern_path(project, slot_index))?)?
        } else {
            EditablePattern::empty(4)
        };
        Ok(to_dto(
            slot,
            slot_index.to_string(),
            exists,
            project,
            tempo,
            &pattern,
        ))
    })
    .await
}

/// Where the previous version of a pattern is kept before it is replaced.
fn backup_dir(app: &AppHandle, project: u8) -> Result<PathBuf, Failure> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| Failure::Other(format!("no place to keep pattern backups: {e}")))?;
    Ok(base
        .join("pattern-backups")
        .join(format!("PROJECT_{project:02}")))
}

/// Write an edited pattern into `slot` of the current project. `project` (1-based) is the
/// project the pattern was opened from: if another project is current now, nothing is
/// written. `bars` is set when the length changed in the editor. The pattern being
/// replaced is first saved under the app's data folder. Returns that backup's path.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn pattern_write(
    app: AppHandle,
    state: State<'_, AppState>,
    slot: u16,
    project: u8,
    length_ticks: u32,
    bars: Option<u8>,
    trailer: [u8; 8],
    notes: Vec<EditNote>,
    extras: Vec<ExtraRecord>,
) -> CmdResult<Option<String>> {
    let dir = backup_dir(&app, project).map_err(|e| match e {
        Failure::Other(s) => s,
        Failure::Device(e) => e.to_string(),
    })?;
    with_device(&state, move |dev| {
        let slot_index = pad_index(slot)?;
        let current = dev.current_project()? + 1;
        if current != project {
            return Err(Failure::Other(format!(
                "the pattern was opened from project {project}, but project {current} is \
                 now selected on the SP-404MKII; nothing was written"
            )));
        }

        let mut ordered: Vec<(u32, u32, Record)> = Vec::with_capacity(notes.len() + extras.len());
        for n in &notes {
            let pad = pad_index(n.pad)?;
            let record = Record::note(
                n.tick,
                pad,
                n.pitch,
                n.velocity,
                n.off_velocity,
                n.gate,
                n.flags,
            );
            ordered.push((n.tick, n.order, record));
        }
        for e in &extras {
            ordered.push((
                e.tick,
                e.order,
                Record {
                    tick: e.tick,
                    bytes: e.bytes,
                },
            ));
        }
        ordered.sort_by_key(|&(tick, order, _)| (tick, order));
        let mut pattern = EditablePattern {
            records: ordered.into_iter().map(|(_, _, r)| r).collect(),
            length: length_ticks.max(1),
            trailer,
        };
        match bars {
            Some(bars) => pattern.set_bars(bars),
            None => pattern.clip_to_length(),
        }
        let bytes = pattern.encode();

        let path = pattern_path(project, slot_index);
        let backup = if dev.pattern_exists(slot_index)? {
            let old = dev.read_file(&path)?;
            std::fs::create_dir_all(&dir).map_err(|e| Failure::Other(e.to_string()))?;
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let file = dir.join(format!("PTN{:05}-{stamp}.BIN", slot_index.index() + 1));
            std::fs::write(&file, old).map_err(|e| Failure::Other(e.to_string()))?;
            Some(file.display().to_string())
        } else {
            None
        };
        log::info!(
            "writing pattern {slot_index} of project {project}: {} records, {} ticks",
            pattern.records.len(),
            pattern.length
        );
        dev.write_pattern(project - 1, slot_index, &bytes)?;
        Ok(backup)
    })
    .await
}
