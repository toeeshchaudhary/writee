//! Workspace bootstrap + file listing.
//!
//! `_index.writee` is the file picker host.
//! `_welcome.writee` is the first-run onboarding canvas — locked_notes=true
//! so the Note tool refuses on it, but every other tool works normally.

use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};

use writee_core::{DocStore, Object, SubNote, TextBox};

#[derive(Clone)]
pub struct Workspace {
    pub root: PathBuf,
    pub current_file: PathBuf,
}

/// RAII exclusive-lock wrapper for `<workspace>/.writee.lock`. Held for the
/// lifetime of the app so a second writee window opening the same workspace
/// hits an error and can refuse to start (instead of racing SQLite writes
/// against the first window).
pub struct WorkspaceLock {
    _file: File,
    path: PathBuf,
}

impl WorkspaceLock {
    pub fn try_acquire(root: &Path) -> Result<Self> {
        let path = root.join(".writee.lock");
        let file = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening lockfile {}", path.display()))?;
        file.try_lock_exclusive().with_context(|| {
            format!(
                "workspace already locked at {} — another writee window appears to have it open",
                path.display()
            )
        })?;
        // Stamp the lock with our pid so a user investigating the file can
        // tell which process holds it.
        let _ = std::io::Write::write_all(
            &mut std::io::BufWriter::new(&file),
            format!("{}\n", std::process::id()).as_bytes(),
        );
        Ok(Self { _file: file, path })
    }
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        // fs2 unlocks on file close (which Drop does), but proactively try.
        let _ = fs2::FileExt::unlock(&self._file);
        let _ = fs::remove_file(&self.path);
    }
}

/// Result of `Workspace::build_tree` — `roots` are filenames with no
/// linking parent; `children` maps each parent filename to the filenames
/// it links to.
#[derive(Debug, Clone, Default)]
pub struct WorkspaceTree {
    pub roots: Vec<String>,
    pub children: std::collections::BTreeMap<String, Vec<String>>,
}

/// Workspace-relative path to the trash directory. We hide it with a `.`
/// prefix so the sidebar `list_files` scan skips it without extra logic
/// (the scan only counts top-level `.writee` files).
pub const TRASH_DIR_NAME: &str = ".trash";

const INDEX_FILE_NAME: &str = "_index.writee";
pub const WELCOME_FILE_NAME: &str = "_welcome.writee";

impl Workspace {
    pub fn discover_or_create() -> Result<Self> {
        let root = resolve_root()?;
        ensure_welcome(&root)?;
        let current_file = root.join(WELCOME_FILE_NAME);
        Ok(Self { root, current_file })
    }

    pub fn index_file_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE_NAME)
    }

    pub fn welcome_file_path(&self) -> PathBuf {
        self.root.join(WELCOME_FILE_NAME)
    }

    pub fn trash_dir(&self) -> PathBuf {
        self.root.join(TRASH_DIR_NAME)
    }

    /// Every `.writee` file currently sitting in `.trash/`, sorted newest-first.
    pub fn list_trash(&self) -> Vec<PathBuf> {
        let dir = self.trash_dir();
        let Ok(read) = fs::read_dir(&dir) else { return Vec::new() };
        let mut files: Vec<PathBuf> = read
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("writee"))
            .collect();
        // Filenames are timestamped on entry, so a reverse lexical sort
        // == newest first.
        files.sort_by(|a, b| b.cmp(a));
        files
    }

    /// Move a `.writee` (and its WAL/SHM siblings if present) into `.trash/`.
    /// Returns the new in-trash path.
    pub fn trash_file(&self, path: &Path) -> Result<PathBuf> {
        let dir = self.trash_dir();
        fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("note");
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let target = dir.join(format!("{ts}_{stem}.writee"));
        fs::rename(path, &target)
            .with_context(|| format!("trashing {} → {}", path.display(), target.display()))?;
        // SQLite WAL/SHM sidecars: try to move them alongside; ignore if
        // missing.
        for ext in ["writee-wal", "writee-shm"] {
            let src = path.with_extension(ext);
            if src.exists() {
                let _ = fs::rename(&src, dir.join(format!("{ts}_{stem}.{ext}")));
            }
        }
        Ok(target)
    }

    /// Restore a trashed file back to `<root>/<original-stem>.writee`. Strips
    /// the timestamp prefix; if the destination exists, appends `-N`.
    pub fn restore_from_trash(&self, trashed: &Path) -> Result<PathBuf> {
        let raw = trashed
            .file_stem()
            .and_then(|s| s.to_str())
            .context("invalid trash filename")?;
        // Filename format on trash: "<unixts>_<original-stem>". Strip the
        // numeric prefix.
        let stem = raw
            .split_once('_')
            .map(|(_, rest)| rest)
            .unwrap_or(raw);
        let mut dest = self.root.join(format!("{stem}.writee"));
        let mut k = 2usize;
        while dest.exists() {
            dest = self.root.join(format!("{stem}-{k}.writee"));
            k += 1;
        }
        fs::rename(trashed, &dest)
            .with_context(|| format!("restoring {} → {}", trashed.display(), dest.display()))?;
        // Best-effort restore of sidecars.
        let trashed_dir = trashed.parent().unwrap_or(&self.root);
        let trashed_stem = trashed.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        for ext in ["writee-wal", "writee-shm"] {
            let src = trashed_dir.join(format!("{trashed_stem}.{ext}"));
            if src.exists() {
                let _ = fs::rename(&src, dest.with_extension(ext));
            }
        }
        Ok(dest)
    }

    /// Files (by name) whose `SubNote`s link to `file`. Used to render the
    /// "Backlinks" sidebar section. O(workspace size) — fine for hundreds.
    pub fn backlinks(&self, file: &str) -> Vec<String> {
        use writee_core::IndexEntry;
        let mut out = std::collections::BTreeSet::new();
        for path in self.list_files() {
            let Some(parent_name) =
                path.file_name().and_then(|s| s.to_str()).map(String::from)
            else {
                continue;
            };
            if parent_name == file {
                continue;
            }
            let Ok(store) = DocStore::open(&path) else { continue };
            let Ok(doc) = store.load_all() else { continue };
            for (_, obj) in doc.objects() {
                if let Object::SubNote(n) = obj {
                    if n.target_file == file {
                        out.insert(parent_name.clone());
                        break;
                    }
                    let in_entries = n
                        .index_entries
                        .as_ref()
                        .map(|es| {
                            es.iter().any(|e| matches!(e, IndexEntry::File { file: f } if f == file))
                        })
                        .unwrap_or(false);
                    let in_legacy = n
                        .index_files
                        .as_ref()
                        .map(|fs| fs.iter().any(|f| f == file))
                        .unwrap_or(false);
                    if in_entries || in_legacy {
                        out.insert(parent_name.clone());
                        break;
                    }
                }
            }
        }
        out.into_iter().collect()
    }

    /// Walk every `.writee` in the workspace and rewrite any SubNote that
    /// links to `old_name` (either `target_file` or an `IndexEntry::File`)
    /// so it links to `new_name` instead. Persists changes through each
    /// store. Returns the count of files mutated.
    pub fn rewrite_links(&self, old_name: &str, new_name: &str) -> Result<usize> {
        use writee_core::IndexEntry;
        if old_name == new_name {
            return Ok(0);
        }
        let mut touched = 0usize;
        for path in self.list_files() {
            // Don't try to rewrite the file we just renamed — caller has
            // already moved it on disk, so `path` here is the *new* name.
            if path.file_name().and_then(|s| s.to_str()) == Some(new_name) {
                continue;
            }
            let Ok(store) = DocStore::open(&path) else { continue };
            let Ok(doc) = store.load_all() else { continue };
            let mut file_touched = false;
            for (id, obj) in doc.objects() {
                if let Object::SubNote(n) = obj {
                    let mut updated = n.clone();
                    let mut changed = false;
                    if updated.target_file == old_name {
                        updated.target_file = new_name.to_string();
                        changed = true;
                    }
                    if let Some(entries) = updated.index_entries.as_mut() {
                        for e in entries.iter_mut() {
                            if let IndexEntry::File { file } = e {
                                if file == old_name {
                                    *file = new_name.to_string();
                                    changed = true;
                                }
                            }
                        }
                    }
                    if let Some(files) = updated.index_files.as_mut() {
                        for f in files.iter_mut() {
                            if f == old_name {
                                *f = new_name.to_string();
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        let _ = store.update(id, &Object::SubNote(updated));
                        file_touched = true;
                    }
                }
            }
            if file_touched {
                touched += 1;
            }
        }
        Ok(touched)
    }

    /// Permanently delete a file inside `.trash/` along with its sidecars.
    pub fn purge_from_trash(&self, trashed: &Path) -> Result<()> {
        fs::remove_file(trashed)
            .with_context(|| format!("purging {}", trashed.display()))?;
        let stem = trashed.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let dir = trashed.parent().unwrap_or(&self.root);
        for ext in ["writee-wal", "writee-shm"] {
            let p = dir.join(format!("{stem}.{ext}"));
            if p.exists() {
                let _ = fs::remove_file(&p);
            }
        }
        Ok(())
    }

    /// Scan every `.writee` in the workspace and return a parent→children map
    /// derived from each file's SubNote target_file references. Files no other
    /// file links to are roots; otherwise they appear under each parent that
    /// links them. Cheap enough at workspace-scale; we re-scan once per
    /// sidebar refresh (only on mtime change at the call site).
    pub fn build_tree(&self) -> WorkspaceTree {
        use writee_core::{DocStore, Object};
        let files = self.list_files();
        let names: Vec<String> = files
            .iter()
            .filter_map(|p| p.file_name().and_then(|s| s.to_str()).map(String::from))
            .collect();
        let mut children: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut has_parent: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for path in &files {
            let Ok(store) = DocStore::open(path) else { continue };
            let Ok(doc) = store.load_all() else { continue };
            let Some(parent_name) = path.file_name().and_then(|s| s.to_str()).map(String::from)
            else { continue };
            for (_, obj) in doc.objects() {
                if let Object::SubNote(n) = obj {
                    if n.is_linked() && names.iter().any(|x| x == &n.target_file) {
                        children.entry(parent_name.clone()).or_default().push(n.target_file.clone());
                        has_parent.insert(n.target_file.clone());
                    }
                }
            }
        }
        // De-dup children lists.
        for v in children.values_mut() {
            v.sort();
            v.dedup();
        }
        let mut roots: Vec<String> = names
            .into_iter()
            .filter(|n| !has_parent.contains(n))
            .collect();
        roots.sort();
        WorkspaceTree { roots, children }
    }

    /// Every `.writee` in the workspace except the index file itself.
    pub fn list_files(&self) -> Vec<PathBuf> {
        let Ok(read) = fs::read_dir(&self.root) else { return Vec::new() };
        let index = self.index_file_path();
        let mut files: Vec<PathBuf> = read
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("writee"))
            .filter(|p| p != &index)
            .collect();
        files.sort();
        files
    }

    pub fn next_untitled(&self) -> PathBuf {
        for i in 1.. {
            let name = if i == 1 {
                "untitled.writee".to_string()
            } else {
                format!("untitled-{i}.writee")
            };
            let candidate = self.root.join(&name);
            if !candidate.exists() {
                return candidate;
            }
        }
        unreachable!()
    }

    pub fn next_file(&self) -> Option<PathBuf> {
        let files = self.list_files();
        if files.len() < 2 {
            return None;
        }
        let idx = files.iter().position(|p| p == &self.current_file)?;
        Some(files[(idx + 1) % files.len()].clone())
    }
}

fn resolve_root() -> Result<PathBuf> {
    let dirs = directories_next::ProjectDirs::from("dev", "writee", "writee")
        .context("could not resolve user config directory")?;
    let config_dir = dirs.config_dir().to_path_buf();
    fs::create_dir_all(&config_dir).ok();
    let marker = config_dir.join("workspace");

    let root = if marker.exists() {
        let raw = fs::read_to_string(&marker).context("reading workspace marker")?;
        PathBuf::from(raw.trim())
    } else {
        let user_dirs = directories_next::UserDirs::new();
        let documents = user_dirs
            .as_ref()
            .and_then(|u| u.document_dir().map(Path::to_path_buf))
            .unwrap_or_else(|| dirs.data_dir().to_path_buf());
        let default = documents.join("Writee");
        fs::create_dir_all(&default).context("creating default workspace")?;
        fs::write(&marker, default.to_string_lossy().as_bytes())
            .context("writing workspace marker")?;
        log::info!("workspace initialised at {}", default.display());
        default
    };

    if !root.exists() {
        fs::create_dir_all(&root).with_context(|| format!("workspace {} missing", root.display()))?;
    }
    Ok(root)
}

/// Create `_welcome.writee` from a built-in template if it doesn't exist.
fn ensure_welcome(root: &Path) -> Result<()> {
    let path = root.join(WELCOME_FILE_NAME);
    if path.exists() {
        return Ok(());
    }
    let store = DocStore::open(&path).context("create welcome file")?;
    store.set_title("Welcome").ok();
    store.set_locked_notes(true).ok();

    // Seed with friendly callouts. World coords are around (0,0) so the
    // initial view (centered on origin) lands right on the content.
    let mut id: u64 = 1;
    let insert = |obj: Object, id: u64| {
        let _ = store.insert(id, &obj);
    };

    insert(
        Object::TextBox(TextBox {
            origin: glam::Vec2::new(-560.0, -280.0),
            font_size: 32.0,
            content: "writee".into(),
            font_name: None,
            cursor: 0,
        }),
        id,
    );
    id += 1;
    insert(
        Object::TextBox(TextBox {
            origin: glam::Vec2::new(-560.0, -230.0),
            font_size: 14.0,
            content:
                "Press N to drop a note anywhere. Notes start as sticky text; right-click one to convert\nit into its own linked file, mark it as an index, or change its mode (canvas / markdown)."
                    .into(),
            font_name: None,
            cursor: 0,
        }),
        id,
    );
    id += 1;
    insert(
        Object::SubNote(SubNote::new_locked_index(
            glam::Vec2::new(-180.0, -180.0),
            "Workspace".into(),
        )),
        id,
    );

    Ok(())
}
