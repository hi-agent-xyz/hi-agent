//! The roster on disk. Addresses and labels only — the credential for an entry
//! lives in [`super::credentials`], keyed by the same id.
//!
//! Rosters do not sync between apps, which falls out of the design rather than
//! being a rule: adding a core means acquiring a credential for it, and a
//! credential is issued to one surface.

use std::cell::RefCell;
use std::fs;

use crate::paths::{log, roster_file};

use super::models::{RosterEntry, RosterSnapshot};

/// Everything in this shell runs on the GTK main thread, so the roster needs a
/// `RefCell` and not a lock. The Windows twin uses a monitor because its model
/// changes state on whatever thread noticed; here the main context is the only
/// thread there is.
#[derive(Default)]
pub struct RosterStore {
    snapshot: RefCell<RosterSnapshot>,
}

impl RosterStore {
    pub fn load(&self) {
        let path = roster_file();
        let Ok(text) = fs::read_to_string(&path) else {
            return;
        };
        match serde_json::from_str::<RosterSnapshot>(&text) {
            Ok(snapshot) => *self.snapshot.borrow_mut() = snapshot,
            Err(e) => {
                // A roster that will not parse is recoverable — the entries can
                // be added again, and the alternative is an app that will not
                // start. The file is left where it is so it can be looked at.
                log(format!("roster unreadable, starting empty: {e}"));
            }
        }
    }

    pub fn entries(&self) -> Vec<RosterEntry> {
        self.snapshot.borrow().entries.clone()
    }

    pub fn attached_id(&self) -> Option<String> {
        self.snapshot.borrow().attached_id.clone()
    }

    pub fn find(&self, id: &str) -> Option<RosterEntry> {
        self.snapshot
            .borrow()
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
    }

    pub fn local(&self) -> Option<RosterEntry> {
        self.snapshot
            .borrow()
            .entries
            .iter()
            .find(|entry| entry.is_local)
            .cloned()
    }

    pub fn first(&self) -> Option<RosterEntry> {
        self.snapshot.borrow().entries.first().cloned()
    }

    pub fn attached(&self) -> Option<RosterEntry> {
        let id = self.attached_id()?;
        self.find(&id)
    }

    /// Add or update an entry, then persist.
    pub fn put(&self, entry: RosterEntry) {
        {
            let mut snapshot = self.snapshot.borrow_mut();
            match snapshot.entries.iter_mut().find(|e| e.id == entry.id) {
                Some(existing) => {
                    existing.base_url = entry.base_url;
                    existing.label = entry.label;
                }
                None => snapshot.entries.push(entry),
            }
        }
        self.save();
    }

    pub fn attach(&self, id: Option<&str>) {
        self.snapshot.borrow_mut().attached_id = id.map(str::to_string);
        self.save();
    }

    /// Forget a core. The local engine cannot be forgotten — it is this machine,
    /// and removing it would leave the shell supervising a process it has no
    /// entry for. The credential is cleared by the caller, which is async.
    pub fn remove(&self, id: &str) -> bool {
        {
            let mut snapshot = self.snapshot.borrow_mut();
            let Some(index) = snapshot
                .entries
                .iter()
                .position(|entry| entry.id == id && !entry.is_local)
            else {
                return false;
            };
            snapshot.entries.remove(index);
            if snapshot.attached_id.as_deref() == Some(id) {
                snapshot.attached_id = snapshot.entries.first().map(|entry| entry.id.clone());
            }
        }
        self.save();
        true
    }

    fn save(&self) {
        let path = roster_file();
        let Ok(text) = serde_json::to_string_pretty(&*self.snapshot.borrow()) else {
            return;
        };
        // Write beside and rename, so an interrupted save cannot leave a
        // half-written roster where a whole one was.
        let temp = path.with_extension("json.tmp");
        if let Err(e) = fs::write(&temp, text).and_then(|()| fs::rename(&temp, &path)) {
            log(format!("roster not saved: {e}"));
        }
    }
}
