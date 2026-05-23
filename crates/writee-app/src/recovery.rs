//! Crash recovery for in-progress edits.
//!
//! SQLite already persists every committed object via `DocStore::insert`,
//! but transient state — the wet stroke as the user is drawing it, the
//! in-progress text-box edit, the in-progress inline-note edit — only lives
//! in memory until commit. A `kill -9` mid-stroke loses everything since
//! the last commit.
//!
//! This module periodically dumps that transient state to a single JSON
//! file at `<workspace>/.writee-recovery.json`. On next launch, the App
//! checks for it and prompts the user to restore.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use writee_core::{InkPoint, ObjectId};

pub const RECOVERY_FILE_NAME: &str = ".writee-recovery.json";

/// Minimum time between snapshot writes. Anything faster is wasted disk IO;
/// anything slower risks losing too much typing in a crash.
pub const SNAPSHOT_INTERVAL_SECS: u64 = 2;

/// Maximum age (relative to "now" on launch) for which we'll offer to
/// restore. Older snapshots are assumed to be stale workspace state from a
/// previous session that already exited cleanly.
pub const MAX_AGE_SECS: u64 = 600; // 10 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoverySnapshot {
    /// Unix-seconds at the time of write.
    pub written_at: u64,
    /// The path of the file the user was editing.
    pub current_file: PathBuf,
    /// In-progress ink stroke (the unsaved wet trail).
    #[serde(default)]
    pub wet_stroke: Vec<InkPoint>,
    /// In-progress TextBox edit: id + the typed content at last snapshot.
    #[serde(default)]
    pub editing_text: Option<(ObjectId, String)>,
    /// In-progress inline-note edit.
    #[serde(default)]
    pub editing_note: Option<(ObjectId, String)>,
}

impl RecoverySnapshot {
    pub fn now(
        current_file: PathBuf,
        wet_stroke: Vec<InkPoint>,
        editing_text: Option<(ObjectId, String)>,
        editing_note: Option<(ObjectId, String)>,
    ) -> Self {
        Self {
            written_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            current_file,
            wet_stroke,
            editing_text,
            editing_note,
        }
    }

    /// True if the snapshot has anything worth restoring.
    pub fn has_content(&self) -> bool {
        !self.wet_stroke.is_empty()
            || self.editing_text.is_some()
            || self.editing_note.is_some()
    }
}

pub fn path_for(root: &Path) -> PathBuf {
    root.join(RECOVERY_FILE_NAME)
}

pub fn write(root: &Path, snap: &RecoverySnapshot) -> Result<()> {
    let path = path_for(root);
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec(snap)?;
    fs::write(&tmp, body)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

pub fn load(root: &Path) -> Option<RecoverySnapshot> {
    let path = path_for(root);
    let body = fs::read(&path).ok()?;
    let snap: RecoverySnapshot = serde_json::from_slice(&body).ok()?;
    let age = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(snap.written_at);
    if age > MAX_AGE_SECS {
        return None;
    }
    if !snap.has_content() {
        return None;
    }
    Some(snap)
}

pub fn clear(root: &Path) {
    let _ = fs::remove_file(path_for(root));
}
