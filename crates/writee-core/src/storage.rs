//! SQLite-backed persistence for a single `.writee` document.

use crate::document::{Document, Object, ObjectId};
use crate::geom::Aabb;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const KIND_STROKE: i64 = 0;
const KIND_ARROW: i64 = 1;
const KIND_TEXTBOX: i64 = 2;
const KIND_SHAPE: i64 = 3;
const KIND_SUBNOTE: i64 = 4;
const KIND_LINK: i64 = 5;
const KIND_IMAGE: i64 = 6;

const META_KEY_TITLE: &str = "title";
const META_KEY_MODE: &str = "mode";       // "canvas" | "markdown"
const META_KEY_LOCKED_NOTES: &str = "locked_notes"; // "true" to disable subnote creation

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub struct DocStore {
    conn: Connection,
}

impl DocStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS objects (
                id      INTEGER PRIMARY KEY,
                kind    INTEGER NOT NULL,
                min_x   REAL NOT NULL,
                min_y   REAL NOT NULL,
                max_x   REAL NOT NULL,
                max_y   REAL NOT NULL,
                blob    BLOB NOT NULL
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS object_rtree USING rtree(
                id, min_x, max_x, min_y, max_y
             );
             CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
             );",
        )?;
        Ok(Self { conn })
    }

    pub fn load_all(&self) -> Result<Document, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, kind, blob FROM objects ORDER BY id ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, Vec<u8>>(2)?))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (id, kind, blob) = row?;
            let obj = match kind {
                KIND_STROKE => Object::Stroke(serde_json::from_slice(&blob)?),
                KIND_ARROW => Object::Arrow(serde_json::from_slice(&blob)?),
                KIND_TEXTBOX => Object::TextBox(serde_json::from_slice(&blob)?),
                KIND_SHAPE => Object::Shape(serde_json::from_slice(&blob)?),
                KIND_SUBNOTE => Object::SubNote(serde_json::from_slice(&blob)?),
                KIND_LINK => Object::Link(serde_json::from_slice(&blob)?),
                KIND_IMAGE => Object::Image(serde_json::from_slice(&blob)?),
                other => {
                    log::warn!("unknown object kind {other}, skipping");
                    continue;
                }
            };
            records.push((id as ObjectId, obj));
        }
        Ok(Document::from_records(records))
    }

    /// Insert a brand-new object (caller chooses the id, usually one freshly
    /// allocated by [`Document::add`]).
    pub fn insert(&self, id: ObjectId, obj: &Object) -> Result<(), StorageError> {
        let (kind, blob) = serialize_obj(obj)?;
        let bbox = sanitize_bbox(obj.bbox());
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO objects (id, kind, min_x, min_y, max_x, max_y, blob)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id as i64, kind, bbox.min.x, bbox.min.y, bbox.max.x, bbox.max.y, blob],
        )?;
        tx.execute("DELETE FROM object_rtree WHERE id = ?1", params![id as i64])?;
        tx.execute(
            "INSERT INTO object_rtree (id, min_x, max_x, min_y, max_y)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id as i64, bbox.min.x, bbox.max.x, bbox.min.y, bbox.max.y],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Overwrite an existing object (same id). Used for in-place edits like
    /// translating a selection or editing a text box.
    pub fn update(&self, id: ObjectId, obj: &Object) -> Result<(), StorageError> {
        self.insert(id, obj)
    }

    pub fn delete(&self, id: ObjectId) -> Result<(), StorageError> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM objects WHERE id = ?1", params![id as i64])?;
        tx.execute("DELETE FROM object_rtree WHERE id = ?1", params![id as i64])?;
        tx.commit()?;
        Ok(())
    }

    pub fn max_object_id(&self) -> Result<ObjectId, StorageError> {
        let v: Option<i64> = self
            .conn
            .query_row("SELECT MAX(id) FROM objects", [], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(v.unwrap_or(0) as ObjectId)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>, StorageError> {
        let val: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(val)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn document_mode(&self) -> Result<DocumentMode, StorageError> {
        Ok(self
            .get_meta(META_KEY_MODE)?
            .as_deref()
            .map(DocumentMode::from_str)
            .unwrap_or(DocumentMode::Canvas))
    }

    pub fn set_document_mode(&self, mode: DocumentMode) -> Result<(), StorageError> {
        self.set_meta(META_KEY_MODE, mode.as_str())
    }

    pub fn title(&self) -> Result<Option<String>, StorageError> {
        self.get_meta(META_KEY_TITLE)
    }

    pub fn set_title(&self, title: &str) -> Result<(), StorageError> {
        self.set_meta(META_KEY_TITLE, title)
    }

    pub fn locked_notes(&self) -> Result<bool, StorageError> {
        Ok(self.get_meta(META_KEY_LOCKED_NOTES)?.as_deref() == Some("true"))
    }

    pub fn set_locked_notes(&self, locked: bool) -> Result<(), StorageError> {
        self.set_meta(META_KEY_LOCKED_NOTES, if locked { "true" } else { "false" })
    }
}

/// Top-level rendering mode for a `.writee` file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentMode {
    Canvas,
    Markdown,
}

impl DocumentMode {
    pub fn from_str(s: &str) -> Self {
        match s {
            "markdown" => DocumentMode::Markdown,
            _ => DocumentMode::Canvas,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            DocumentMode::Canvas => "canvas",
            DocumentMode::Markdown => "markdown",
        }
    }
}

fn serialize_obj(obj: &Object) -> Result<(i64, Vec<u8>), StorageError> {
    Ok(match obj {
        Object::Stroke(s) => (KIND_STROKE, serde_json::to_vec(s)?),
        Object::Arrow(a) => (KIND_ARROW, serde_json::to_vec(a)?),
        Object::TextBox(t) => (KIND_TEXTBOX, serde_json::to_vec(t)?),
        Object::Shape(s) => (KIND_SHAPE, serde_json::to_vec(s)?),
        Object::SubNote(n) => (KIND_SUBNOTE, serde_json::to_vec(n)?),
        Object::Link(l) => (KIND_LINK, serde_json::to_vec(l)?),
        Object::Image(i) => (KIND_IMAGE, serde_json::to_vec(i)?),
    })
}

fn sanitize_bbox(b: Aabb) -> Aabb {
    use glam::Vec2;
    let ok = b.min.x.is_finite() && b.min.y.is_finite() && b.max.x.is_finite() && b.max.y.is_finite();
    if ok && b.min.x <= b.max.x && b.min.y <= b.max.y {
        b
    } else {
        Aabb { min: Vec2::ZERO, max: Vec2::ZERO }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke::{InkPoint, Stroke};

    fn temp_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("writee-test-{}-{}.writee", tag, std::process::id()))
    }

    #[test]
    fn round_trip_one_stroke() {
        let path = temp_path("stroke");
        let _ = std::fs::remove_file(&path);

        let store = DocStore::open(&path).unwrap();
        let mut s = Stroke::new(5.0);
        s.push(InkPoint { x: 0.0, y: 0.0, pressure: 0.5, tilt_x: 0.0, tilt_y: 0.0, t_ms: 0 });
        s.push(InkPoint { x: 10.0, y: 5.0, pressure: 0.8, tilt_x: 0.0, tilt_y: 0.0, t_ms: 16 });
        store.insert(1, &Object::Stroke(s)).unwrap();
        drop(store);

        let store = DocStore::open(&path).unwrap();
        let doc = store.load_all().unwrap();
        assert_eq!(doc.len(), 1);
        assert_eq!(store.max_object_id().unwrap(), 1);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn round_trip_arrow_and_text() {
        use crate::arrow::Arrow;
        use crate::textbox::TextBox;
        use glam::Vec2;

        let path = temp_path("ax-tx");
        let _ = std::fs::remove_file(&path);
        let store = DocStore::open(&path).unwrap();
        store.insert(1, &Object::Arrow(Arrow::new(Vec2::ZERO, Vec2::new(50.0, 25.0)))).unwrap();
        let mut t = TextBox::new(Vec2::new(60.0, 60.0), 24.0);
        t.content = "hello".into();
        store.insert(2, &Object::TextBox(t)).unwrap();
        drop(store);

        let store = DocStore::open(&path).unwrap();
        let doc = store.load_all().unwrap();
        assert_eq!(doc.len(), 2);

        std::fs::remove_file(&path).ok();
    }
}
