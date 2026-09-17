//! Importing a folder of originals into a collection's library.
//!
//! The source folder is only read. Each supported file is fingerprinted with
//! SHA-256, its source metadata recorded, and temporary thumbnails written
//! from the original (turned by its EXIF orientation). PDFs also get every page
//! rendered, so later steps can treat pages like photos. Running an import again
//! skips files whose size and modification time haven't changed.

use crate::COLLECTION_META_DIR;
use crate::library::{ImportCounts, Library, NewImage, NewOriginal};
use crate::media::{self, Kind, SourceMetadata};
use crate::time::{iso_utc, unix_nanos};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

pub struct Options {
    pub workers: usize,
    /// Files modified more recently than this are assumed to be still copying and are left for the next import.
    pub settle: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            workers: std::thread::available_parallelism().map_or(4, |n| n.get()),
            settle: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Progress {
    Scanned { found: usize },
    Hashed { done: usize, total: usize },
    Processed { done: usize, total: usize },
}

#[derive(Debug, Default)]
pub struct Report {
    pub counts: ImportCounts,
    /// Pictures added: one per photo, one per PDF page.
    pub images_added: u64,
    pub still_writing: u64,
    pub unsupported: u64,
    pub bytes_hashed: u64,
    pub scan_time: Duration,
    pub hash_time: Duration,
    pub process_time: Duration,
    pub errors: Vec<(PathBuf, String)>,
}

#[derive(Clone, Debug)]
struct Candidate {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
    kind: Kind,
}

pub fn import_folder(
    lib: &mut Library,
    source: &Path,
    opts: &Options,
    mut progress: impl FnMut(Progress),
) -> Result<Report> {
    let source = source
        .canonicalize()
        .with_context(|| format!("can't open {}", source.display()))?;
    if !source.is_dir() {
        bail!("{} is not a folder", source.display());
    }
    // Never import the collection's own files, even if it sits inside the source folder.
    let collection = lib.meta_dir().parent().and_then(|p| p.canonicalize().ok());

    let mut report = Report::default();
    let import_id = lib.begin_import(&source)?;

    let started = Instant::now();
    let mut candidates = Vec::new();
    scan(&source, collection.as_deref(), &mut candidates, &mut report)?;
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    report.counts.found = candidates.len() as u64;
    report.scan_time = started.elapsed();
    progress(Progress::Scanned {
        found: candidates.len(),
    });

    // Skip files already imported and unchanged, and files still being written.
    let known = lib.known_locations()?;
    let now = SystemTime::now();
    let mut to_hash = Vec::new();
    for c in candidates {
        let recent = now
            .duration_since(c.modified)
            .map_or(true, |age| age < opts.settle);
        if recent {
            report.still_writing += 1;
        } else if known
            .get(&c.path)
            .is_some_and(|k| k.size == c.size && k.modified_ns == unix_nanos(c.modified))
        {
            report.counts.unchanged += 1;
        } else {
            to_hash.push(c);
        }
    }

    // Fingerprint.
    let started = Instant::now();
    let total = to_hash.len();
    let mut hashed = Vec::with_capacity(total);
    let mut done = 0;
    parallel_map(
        to_hash,
        opts.workers,
        |c| {
            let sha = sha256_file(&c.path);
            (c, sha)
        },
        |(c, sha)| {
            done += 1;
            match sha {
                Ok(sha) => {
                    report.bytes_hashed += c.size;
                    hashed.push((c, sha));
                }
                Err(e) => fail(lib, import_id, &mut report, &c.path, &e)?,
            }
            progress(Progress::Hashed { done, total });
            Ok(())
        },
    )?;
    report.hash_time = started.elapsed();

    // Sort out duplicates; the first file with new content becomes the original.
    let mut claimed = HashSet::new();
    let mut new_originals = Vec::new();
    let mut later_duplicates = Vec::new();
    hashed.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    for (c, sha) in hashed {
        if claimed.contains(&sha) {
            later_duplicates.push((c, sha));
        } else if lib.has_original(&sha)? {
            let same_path = known.get(&c.path).is_some_and(|k| k.sha256 == sha);
            lib.add_location(&c.path, &sha, c.size, unix_nanos(c.modified), import_id)?;
            if same_path {
                report.counts.unchanged += 1;
            } else {
                report.counts.duplicates += 1;
            }
        } else {
            claimed.insert(sha.clone());
            new_originals.push((c, sha));
        }
    }

    // Read metadata and write thumbnails and page renders.
    let started = Instant::now();
    let total = new_originals.len();
    let mut done = 0;
    let mut added = HashSet::new();
    let meta_dir = lib.meta_dir().to_path_buf();
    parallel_map(
        new_originals,
        opts.workers,
        |(c, sha)| {
            let derived = derive(&meta_dir, &c, &sha);
            (c, sha, derived)
        },
        |(c, sha, derived)| {
            done += 1;
            match derived {
                Ok((meta, images)) => {
                    let name = c
                        .path
                        .file_name()
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    let original = NewOriginal {
                        sha256: &sha,
                        size: c.size,
                        kind: c.kind,
                        file_name: &name,
                        sequence_number: media::sequence_number(&c.path),
                        meta: &meta,
                        file_modified: &iso_utc(c.modified),
                    };
                    lib.add_original(
                        &original,
                        &images,
                        &c.path,
                        unix_nanos(c.modified),
                        import_id,
                    )?;
                    report.counts.added += 1;
                    report.images_added += images.len() as u64;
                    added.insert(sha);
                }
                Err(e) => fail(lib, import_id, &mut report, &c.path, &e)?,
            }
            progress(Progress::Processed { done, total });
            Ok(())
        },
    )?;
    report.process_time = started.elapsed();

    for (c, sha) in later_duplicates {
        if added.contains(&sha) {
            lib.add_location(&c.path, &sha, c.size, unix_nanos(c.modified), import_id)?;
            report.counts.duplicates += 1;
        } else {
            let e = anyhow::anyhow!("same content as a file that failed to import");
            fail(lib, import_id, &mut report, &c.path, &e)?;
        }
    }

    lib.finish_import(import_id, &report.counts)?;
    Ok(report)
}

fn fail(
    lib: &Library,
    import_id: i64,
    report: &mut Report,
    path: &Path,
    e: &anyhow::Error,
) -> Result<()> {
    let message = format!("{e:#}");
    lib.add_error(import_id, path, &message)?;
    report.counts.failed += 1;
    report.errors.push((path.to_path_buf(), message));
    Ok(())
}

fn scan(
    root: &Path,
    collection: Option<&Path>,
    out: &mut Vec<Candidate>,
    report: &mut Report,
) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if dir == root => {
                return Err(e).with_context(|| format!("can't read {}", dir.display()));
            }
            Err(e) => {
                report.errors.push((dir, format!("can't read folder: {e}")));
                continue;
            }
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                if name != COLLECTION_META_DIR && Some(path.as_path()) != collection {
                    stack.push(path);
                }
            } else if file_type.is_file() {
                let Some(kind) = media::kind_for(&path) else {
                    report.unsupported += 1;
                    continue;
                };
                let md = entry.metadata()?;
                out.push(Candidate {
                    path,
                    size: md.len(),
                    modified: md.modified()?,
                    kind,
                });
            }
        }
    }
    Ok(())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Reads metadata and writes derived files for one new original.
fn derive(meta_dir: &Path, c: &Candidate, sha: &str) -> Result<(SourceMetadata, Vec<NewImage>)> {
    if c.size < 100 {
        bail!("file is too small to be an image ({} bytes)", c.size);
    }
    let meta = media::probe(&c.path, c.kind)?;
    let rel_dir = Library::derived_dir(sha);
    let dir = meta_dir.join(&rel_dir);
    std::fs::create_dir_all(&dir)?;

    let mut images = Vec::new();
    match c.kind {
        Kind::Image => {
            let thumbnail = format!("{rel_dir}/thumb.jpg");
            atomic(&meta_dir.join(&thumbnail), |tmp| {
                media::write_thumbnail(
                    &c.path,
                    tmp,
                    media::THUMBNAIL_EDGE,
                    media::THUMBNAIL_QUALITY,
                )
            })?;
            let (width, height) = meta.display_size();
            images.push(NewImage {
                page: 0,
                width,
                height,
                render: None,
                thumbnail,
            });
        }
        Kind::Pdf => {
            for page in 1..=meta.page_count {
                let render = format!("{rel_dir}/page-{page:04}.jpg");
                let render_path = meta_dir.join(&render);
                let (width, height) = atomic(&render_path, |tmp| {
                    media::render_pdf_page(
                        &c.path,
                        page,
                        media::PDF_DPI,
                        media::PDF_MAX_EDGE,
                        tmp,
                        media::PDF_QUALITY,
                    )
                })
                .with_context(|| format!("page {page}"))?;
                let thumbnail = format!("{rel_dir}/page-{page:04}-thumb.jpg");
                atomic(&meta_dir.join(&thumbnail), |tmp| {
                    media::write_thumbnail(
                        &render_path,
                        tmp,
                        media::THUMBNAIL_EDGE,
                        media::THUMBNAIL_QUALITY,
                    )
                })?;
                images.push(NewImage {
                    page,
                    width,
                    height,
                    render: Some(render),
                    thumbnail,
                });
            }
        }
    }
    Ok((meta, images))
}

/// Writes through a temporary name so a crash never leaves a half-written file.
fn atomic<T>(dest: &Path, write: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let name = dest
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let tmp = dest.with_file_name(format!(".{name}.partial"));
    let value = write(&tmp).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    std::fs::rename(&tmp, dest)?;
    Ok(value)
}

/// Runs `work` on a pool of threads and hands each result to `on_result` on
/// the calling thread, which owns the database connection.
fn parallel_map<T: Send, R: Send>(
    items: Vec<T>,
    workers: usize,
    work: impl Fn(T) -> R + Sync,
    mut on_result: impl FnMut(R) -> Result<()>,
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let queue = Mutex::new(items.into_iter());
    let stop = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..workers.max(1) {
            let tx = tx.clone();
            let (queue, stop, work) = (&queue, &stop, &work);
            scope.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let Some(item) = queue.lock().expect("queue poisoned").next() else {
                        break;
                    };
                    if tx.send(work(item)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for result in rx {
            if let Err(e) = on_result(result) {
                stop.store(true, Ordering::Relaxed);
                return Err(e);
            }
        }
        Ok(())
    })
}
