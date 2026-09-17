//! The collection's SQLite library: a local cache of what has been imported.
//!
//! Lives at `<collection>/_agenticarchivist/cache/library.sqlite`. Derived
//! files live under `_agenticarchivist/derived/`, named by the original's SHA-256.

use crate::COLLECTION_META_DIR;
use crate::media::SourceMetadata;
use crate::time::now_iso;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE imports (
    id           INTEGER PRIMARY KEY,
    source_root  TEXT NOT NULL,
    started_at   TEXT NOT NULL,
    finished_at  TEXT,
    found        INTEGER NOT NULL DEFAULT 0,
    added        INTEGER NOT NULL DEFAULT 0,
    duplicates   INTEGER NOT NULL DEFAULT 0,
    unchanged    INTEGER NOT NULL DEFAULT 0,
    failed       INTEGER NOT NULL DEFAULT 0
);

-- One row per unique file content.
CREATE TABLE originals (
    sha256           TEXT PRIMARY KEY,
    size             INTEGER NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('image', 'pdf')),
    media_type       TEXT,
    file_name        TEXT NOT NULL,
    sequence_number  INTEGER,
    width            INTEGER NOT NULL,
    height           INTEGER NOT NULL,
    orientation      INTEGER NOT NULL,
    page_count       INTEGER NOT NULL,
    captured_at      TEXT,
    captured_at_from TEXT NOT NULL CHECK (captured_at_from IN ('exif', 'file')),
    camera_make      TEXT,
    camera_model     TEXT,
    lens_model       TEXT,
    gps_latitude     REAL,
    gps_longitude    REAL,
    import_id        INTEGER NOT NULL REFERENCES imports(id),
    imported_at      TEXT NOT NULL
);

-- Every place a file with that content has been seen. Size and modification
-- time let a later import skip unchanged files without hashing them again.
CREATE TABLE locations (
    path         TEXT PRIMARY KEY,
    sha256       TEXT NOT NULL REFERENCES originals(sha256),
    size         INTEGER NOT NULL,
    modified_ns  INTEGER NOT NULL,
    import_id    INTEGER NOT NULL REFERENCES imports(id),
    seen_at      TEXT NOT NULL
);
CREATE INDEX locations_sha256 ON locations(sha256);

-- One row per picture to process: a photo is one image, a PDF has one per page.
CREATE TABLE images (
    id           INTEGER PRIMARY KEY,
    sha256       TEXT NOT NULL REFERENCES originals(sha256),
    page         INTEGER NOT NULL,           -- 0 for a photo, 1… for PDF pages
    width        INTEGER NOT NULL,           -- as displayed
    height       INTEGER NOT NULL,
    render       TEXT,                       -- PDF page render, relative to the metadata folder
    thumbnail    TEXT NOT NULL,              -- relative to the metadata folder
    thumbnail_from TEXT NOT NULL CHECK (thumbnail_from IN ('original', 'working')),
    UNIQUE (sha256, page)
);

CREATE TABLE import_errors (
    import_id  INTEGER NOT NULL REFERENCES imports(id),
    path       TEXT NOT NULL,
    message    TEXT NOT NULL
);
";

pub struct Library {
    conn: Connection,
    meta_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct KnownLocation {
    pub sha256: String,
    pub size: u64,
    pub modified_ns: i64,
}

#[derive(Clone, Debug)]
pub struct NewOriginal<'a> {
    pub sha256: &'a str,
    pub size: u64,
    pub kind: crate::media::Kind,
    pub file_name: &'a str,
    pub sequence_number: Option<i64>,
    pub meta: &'a SourceMetadata,
    /// File modification time as RFC 3339, used when EXIF has no capture time.
    pub file_modified: &'a str,
}

#[derive(Clone, Debug)]
pub struct NewImage {
    pub page: u32,
    pub width: u32,
    pub height: u32,
    pub render: Option<String>,
    pub thumbnail: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportCounts {
    pub found: u64,
    pub added: u64,
    pub duplicates: u64,
    pub unchanged: u64,
    pub failed: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub originals: u64,
    pub photos: u64,
    pub pdfs: u64,
    pub images: u64,
    pub bytes: u64,
    pub duplicate_locations: u64,
    pub imports: u64,
    pub errors: u64,
    pub with_exif_date: u64,
    pub with_gps: u64,
    pub rotated: u64,
}

/// A row for the contact sheet.
#[derive(Clone, Debug)]
pub struct SheetImage {
    pub sha256: String,
    pub page: u32,
    pub page_count: u32,
    pub kind: String,
    pub file_name: String,
    pub path: String,
    pub thumbnail: String,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    pub captured_at: Option<String>,
    pub captured_at_from: String,
    pub camera: Option<String>,
}

impl Library {
    /// Opens or creates the library of the collection at `collection`.
    pub fn open(collection: &Path) -> Result<Self> {
        let meta_dir = collection.join(COLLECTION_META_DIR);
        std::fs::create_dir_all(meta_dir.join("cache"))
            .with_context(|| format!("creating {}", meta_dir.display()))?;
        std::fs::create_dir_all(meta_dir.join("derived"))?;
        let conn = Connection::open(meta_dir.join("cache/library.sqlite"))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        match version {
            0 => conn.execute_batch(&format!(
                "BEGIN; {SCHEMA} PRAGMA user_version = {SCHEMA_VERSION}; COMMIT;"
            ))?,
            SCHEMA_VERSION => {}
            v => bail!("library schema version {v} is newer than this app understands"),
        }
        Ok(Self { conn, meta_dir })
    }

    /// `<collection>/_agenticarchivist`
    pub fn meta_dir(&self) -> &Path {
        &self.meta_dir
    }

    /// Folder for files derived from one original, relative to the metadata folder.
    pub fn derived_dir(sha256: &str) -> String {
        format!("derived/{}/{sha256}", &sha256[..2])
    }

    pub fn begin_import(&self, source_root: &Path) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO imports (source_root, started_at) VALUES (?1, ?2)",
            params![source_root.to_string_lossy(), now_iso()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn finish_import(&self, id: i64, c: &ImportCounts) -> Result<()> {
        self.conn.execute(
            "UPDATE imports SET finished_at = ?2, found = ?3, added = ?4, duplicates = ?5,
             unchanged = ?6, failed = ?7 WHERE id = ?1",
            params![
                id,
                now_iso(),
                c.found as i64,
                c.added as i64,
                c.duplicates as i64,
                c.unchanged as i64,
                c.failed as i64
            ],
        )?;
        Ok(())
    }

    pub fn known_locations(&self) -> Result<HashMap<PathBuf, KnownLocation>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, sha256, size, modified_ns FROM locations")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                PathBuf::from(r.get::<_, String>(0)?),
                KnownLocation {
                    sha256: r.get(1)?,
                    size: r.get::<_, i64>(2)? as u64,
                    modified_ns: r.get(3)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn has_original(&self, sha256: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM originals WHERE sha256 = ?1",
                [sha256],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Records where a file was seen. Returns true if this path is new.
    pub fn add_location(
        &self,
        path: &Path,
        sha256: &str,
        size: u64,
        modified_ns: i64,
        import_id: i64,
    ) -> Result<bool> {
        let path = path.to_string_lossy();
        let existed = self
            .conn
            .query_row("SELECT 1 FROM locations WHERE path = ?1", [&path], |_| {
                Ok(())
            })
            .optional()?
            .is_some();
        self.conn.execute(
            "INSERT INTO locations (path, sha256, size, modified_ns, import_id, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (path) DO UPDATE SET sha256 = excluded.sha256, size = excluded.size,
               modified_ns = excluded.modified_ns, import_id = excluded.import_id,
               seen_at = excluded.seen_at",
            params![path, sha256, size as i64, modified_ns, import_id, now_iso()],
        )?;
        Ok(!existed)
    }

    /// Adds a new original with its images and first location, in one transaction.
    pub fn add_original(
        &mut self,
        o: &NewOriginal,
        images: &[NewImage],
        path: &Path,
        modified_ns: i64,
        import_id: i64,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let m = o.meta;
        let (captured_at, from) = match &m.captured_at {
            Some(t) => (t.as_str(), "exif"),
            None => (o.file_modified, "file"),
        };
        let now = now_iso();
        tx.execute(
            "INSERT INTO originals (sha256, size, kind, media_type, file_name, sequence_number,
               width, height, orientation, page_count, captured_at, captured_at_from,
               camera_make, camera_model, lens_model, gps_latitude, gps_longitude,
               import_id, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                o.sha256, o.size as i64, o.kind.as_str(), m.media_type, o.file_name, o.sequence_number,
                m.width, m.height, m.orientation, m.page_count, captured_at, from,
                m.camera_make, m.camera_model, m.lens_model, m.gps_latitude, m.gps_longitude,
                import_id, now
            ],
        )?;
        for img in images {
            tx.execute(
                "INSERT INTO images (sha256, page, width, height, render, thumbnail, thumbnail_from)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'original')",
                params![o.sha256, img.page, img.width, img.height, img.render, img.thumbnail],
            )?;
        }
        tx.execute(
            "INSERT INTO locations (path, sha256, size, modified_ns, import_id, seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT (path) DO UPDATE SET sha256 = excluded.sha256, size = excluded.size,
               modified_ns = excluded.modified_ns, import_id = excluded.import_id,
               seen_at = excluded.seen_at",
            params![
                path.to_string_lossy(),
                o.sha256,
                o.size as i64,
                modified_ns,
                import_id,
                now
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn add_error(&self, import_id: i64, path: &Path, message: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO import_errors (import_id, path, message) VALUES (?1, ?2, ?3)",
            params![import_id, path.to_string_lossy(), message],
        )?;
        Ok(())
    }

    pub fn summary(&self) -> Result<Summary> {
        let one = |sql: &str| -> Result<u64> {
            Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))?.max(0) as u64)
        };
        Ok(Summary {
            originals: one("SELECT count(*) FROM originals")?,
            photos: one("SELECT count(*) FROM originals WHERE kind = 'image'")?,
            pdfs: one("SELECT count(*) FROM originals WHERE kind = 'pdf'")?,
            images: one("SELECT count(*) FROM images")?,
            bytes: one("SELECT coalesce(sum(size), 0) FROM originals")?,
            duplicate_locations: one(
                "SELECT count(*) - (SELECT count(*) FROM originals) FROM locations",
            )?,
            imports: one("SELECT count(*) FROM imports")?,
            errors: one(
                "SELECT count(*) FROM import_errors WHERE import_id = (SELECT max(id) FROM imports)",
            )?,
            with_exif_date: one("SELECT count(*) FROM originals WHERE captured_at_from = 'exif'")?,
            with_gps: one("SELECT count(*) FROM originals WHERE gps_latitude IS NOT NULL")?,
            rotated: one("SELECT count(*) FROM originals WHERE orientation <> 1")?,
        })
    }

    /// Errors from the most recent import.
    pub fn latest_errors(&self) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, message FROM import_errors
             WHERE import_id = (SELECT max(id) FROM imports) ORDER BY path",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Images in capture order, then file name and page.
    pub fn sheet_images(&self) -> Result<Vec<SheetImage>> {
        let mut stmt = self.conn.prepare(
            "SELECT o.sha256, i.page, o.page_count, o.kind, o.file_name,
                    (SELECT min(path) FROM locations l WHERE l.sha256 = o.sha256),
                    i.thumbnail, i.width, i.height, o.orientation, o.captured_at,
                    o.captured_at_from, trim(coalesce(o.camera_make, '') || ' ' || coalesce(o.camera_model, ''))
             FROM images i JOIN originals o ON o.sha256 = i.sha256
             ORDER BY o.captured_at, o.file_name, i.page",
        )?;
        let rows = stmt.query_map([], |r| {
            let camera: String = r.get(12)?;
            Ok(SheetImage {
                sha256: r.get(0)?,
                page: r.get(1)?,
                page_count: r.get(2)?,
                kind: r.get(3)?,
                file_name: r.get(4)?,
                path: r.get(5)?,
                thumbnail: r.get(6)?,
                width: r.get(7)?,
                height: r.get(8)?,
                orientation: r.get(9)?,
                captured_at: r.get(10)?,
                captured_at_from: r.get(11)?,
                camera: (!camera.is_empty()).then_some(camera),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
