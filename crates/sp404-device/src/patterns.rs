//! Writing a pattern file to the device (pattern editor).
//!
//! Not captured from the official app: its pattern import copies files into `PTN/`, and a
//! whole-project restore makes the device reload the project with `B1` afterwards. This
//! writes one pattern file the same way and sends the same reload, without the restore's
//! first step, which erases the current project. _(?)_ until tried on hardware.

use std::time::Duration;

use sp404_proto::control::{self, reply, restore};
use sp404_proto::{Channel, Message, PadIndex};

use crate::{Device, Error, Result, is_channel};

fn short(payload: &[u8]) -> Message {
    Message::short(Channel::Control, payload)
}

/// Path of a pattern file. `project` is 1-based (the `PROJECT_NN` folder).
pub fn pattern_path(project: u8, slot: PadIndex) -> String {
    format!(
        "ROLAND/SP-404MKII/PROJECT_{project:02}/PTN/PTN{:05}.BIN",
        slot.index() + 1
    )
}

impl Device {
    /// Replace the pattern in `slot` of the current project with `data` (a complete
    /// `PTNnnnnn.BIN`) and make the device reload the project from the card. `project` is
    /// 0-based and **must be the current project**.
    pub fn write_pattern(&self, project: u8, slot: PadIndex, data: &[u8]) -> Result<()> {
        let current = self.current_project()?;
        if current != project {
            return Err(Error::Refused(format!(
                "write pattern into project {}: project {} is current (select it first)",
                project + 1,
                current + 1
            )));
        }
        let path = pattern_path(project + 1, slot);
        self.fire(&short(&control::edit_begin()))?;
        let result = (|| -> Result<()> {
            // Nothing may be playing while the file changes under the device.
            self.transact(
                &short(&control::stop()),
                "stop",
                Duration::from_secs(3),
                |m| is_channel(m, Channel::Control) && m.payload().first() == Some(&reply::STOP),
            )?;
            self.write_file(&path, data)?;
            let reloaded = self.transact(
                &short(&restore::reload(project)),
                "reload project",
                Duration::from_secs(15),
                |m| {
                    is_channel(m, Channel::Control)
                        && m.payload().first() == Some(&restore::RELOAD_REPLY)
                },
            )?;
            if let Some(&code) = reloaded.payload().get(1).filter(|&&c| c != 0) {
                return Err(Error::Refused(format!("reload project: status {code}")));
            }
            let mut inbox = self.inbox.lock().unwrap();
            Self::wait(
                &mut inbox,
                "project reload",
                Duration::from_secs(15),
                &mut |m: &Message| {
                    is_channel(m, Channel::Control)
                        && m.payload().first() == Some(&restore::RELOADED)
                },
            )?;
            Ok(())
        })();
        self.fire(&short(&control::edit_end()))?;
        result
    }
}
