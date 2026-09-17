//! AgenticArchivist background process.
//!
//! For now a command line for import. Will watch folders, run AI jobs, write
//! sidecars, serve a local API to the desktop window, and supervise the
//! bundled Syncthing.

use agenticarchivist_core::import::{self, Options, Progress};
use agenticarchivist_core::library::Library;
use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const USAGE: &str = "\
Usage:
  agenticarchivist-daemon import <source folder> <collection folder>
  agenticarchivist-daemon status <collection folder>
  agenticarchivist-daemon contact-sheet <collection folder>

import         Reads originals from the source folder (never changes it) into the
               collection's library at <collection>/_agenticarchivist/.
status         Summarises the library and the latest import's errors.
contact-sheet  Writes an HTML page of every thumbnail with its source metadata.";

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["import", source, collection] => run_import(Path::new(source), Path::new(collection)),
        ["status", collection] => status(Path::new(collection)),
        ["contact-sheet", collection] => {
            let path = contact_sheet(Path::new(collection))?;
            println!("{}", path.display());
            Ok(())
        }
        ["--version"] => {
            println!("agenticarchivist-daemon {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn run_import(source: &Path, collection: &Path) -> Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("can't open {}", source.display()))?;
    std::fs::create_dir_all(collection)?;
    let collection = collection.canonicalize()?;
    if source.starts_with(collection.join(agenticarchivist_core::COLLECTION_META_DIR)) {
        bail!("the source folder is inside the collection's metadata folder");
    }
    let mut lib = Library::open(&collection)?;
    println!(
        "Importing {}\n     into {}",
        source.display(),
        collection.display()
    );

    let started = Instant::now();
    let mut last_print = Instant::now();
    let report = import::import_folder(&mut lib, &source, &Options::default(), |p| {
        let (label, done, total) = match p {
            Progress::Scanned { found } => {
                println!("Found {found} supported files");
                return;
            }
            Progress::Hashed { done, total } => ("Fingerprinting", done, total),
            Progress::Processed { done, total } => ("Reading and making thumbnails", done, total),
        };
        if done == total || last_print.elapsed().as_millis() > 250 {
            print!("\r{label}: {done}/{total}   ");
            if done == total {
                println!();
            }
            let _ = std::io::stdout().flush();
            last_print = Instant::now();
        }
    })?;

    let c = &report.counts;
    let secs = started.elapsed().as_secs_f64();
    println!();
    println!(
        "Added       {} originals ({} pictures, counting PDF pages)",
        c.added, report.images_added
    );
    println!("Unchanged   {}", c.unchanged);
    println!(
        "Duplicates  {} (same content already in the library)",
        c.duplicates
    );
    println!("Failed      {}", c.failed);
    if report.still_writing > 0 {
        println!(
            "Skipped     {} still being written; run import again",
            report.still_writing
        );
    }
    if report.unsupported > 0 {
        println!(
            "Ignored     {} files of unsupported types",
            report.unsupported
        );
    }
    println!(
        "Time        {secs:.1} s (scan {:.1} s, fingerprint {:.1} s for {:.0} MB, read + thumbnails {:.1} s)",
        report.scan_time.as_secs_f64(),
        report.hash_time.as_secs_f64(),
        report.bytes_hashed as f64 / 1e6,
        report.process_time.as_secs_f64(),
    );
    for (path, message) in report.errors.iter().take(20) {
        println!("  ✗ {}: {message}", path.display());
    }
    if report.errors.len() > 20 {
        println!("  … and {} more (see status)", report.errors.len() - 20);
    }
    Ok(())
}

fn status(collection: &Path) -> Result<()> {
    let lib = Library::open(collection)?;
    let s = lib.summary()?;
    println!(
        "Library     {}",
        lib.meta_dir().join("cache/library.sqlite").display()
    );
    println!(
        "Originals   {} ({} photos, {} PDFs), {:.1} GB",
        s.originals,
        s.photos,
        s.pdfs,
        s.bytes as f64 / 1e9
    );
    println!("Pictures    {} (counting PDF pages)", s.images);
    println!("Duplicates  {} extra copies seen", s.duplicate_locations);
    println!("EXIF date   {} of {} photos", s.with_exif_date, s.photos);
    println!("GPS         {}", s.with_gps);
    println!(
        "Rotated     {} with EXIF orientation other than upright",
        s.rotated
    );
    println!("Imports     {}", s.imports);
    let errors = lib.latest_errors()?;
    println!("Errors      {} in the latest import", errors.len());
    for (path, message) in errors {
        println!("  ✗ {path}: {message}");
    }
    Ok(())
}

fn contact_sheet(collection: &Path) -> Result<PathBuf> {
    let lib = Library::open(collection)?;
    let images = lib.sheet_images()?;
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    };
    let mut cards = String::new();
    for i in &images {
        let page = if i.kind == "pdf" {
            format!(" · page {} of {}", i.page, i.page_count)
        } else {
            String::new()
        };
        let date = i.captured_at.as_deref().unwrap_or("");
        let date_note = if i.captured_at_from == "file" {
            " (file date)"
        } else {
            ""
        };
        let rotated = if i.orientation != 1 {
            format!(" · EXIF orientation {}", i.orientation)
        } else {
            String::new()
        };
        let camera = i
            .camera
            .as_deref()
            .map(|c| format!("<br>{}", esc(c)))
            .unwrap_or_default();
        cards.push_str(&format!(
            "<figure title=\"{path}\"><div class=\"frame\"><img loading=\"lazy\" src=\"../{thumb}\" alt=\"\"></div>\
             <figcaption><b>{name}</b>{page}<br>{date}{date_note}<br>{w}×{h}{rotated}{camera}<br><code>{sha}</code></figcaption></figure>\n",
            path = esc(&i.path),
            thumb = esc(&i.thumbnail),
            name = esc(&i.file_name),
            date = esc(date),
            w = i.width,
            h = i.height,
            sha = &i.sha256[..12],
        ));
    }
    let html = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Contact sheet</title>
<style>
:root {{ color-scheme: light dark; --bg:#f4f5f2; --frame:#e1e4de; --ink:#1d2320; --muted:#5d6762; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#121614; --frame:#252b28; --ink:#e2e7e3; --muted:#9aa49f; }} }}
body {{ margin:0; padding:24px; background:var(--bg); color:var(--ink); font:12px/1.45 -apple-system, system-ui, sans-serif; }}
h1 {{ font-size:18px; margin:0 0 4px; }}
p {{ margin:0 0 20px; color:var(--muted); }}
main {{ display:grid; grid-template-columns:repeat(auto-fill, minmax(200px, 1fr)); gap:20px; }}
figure {{ margin:0; }}
.frame {{ aspect-ratio:1; background:var(--frame); display:flex; align-items:center; justify-content:center; }}
img {{ max-width:100%; max-height:100%; display:block; }}
figcaption {{ padding-top:6px; color:var(--muted); overflow-wrap:anywhere; }}
figcaption b {{ color:var(--ink); font-weight:600; }}
code {{ font-size:11px; }}
</style></head><body>
<h1>{count} pictures</h1>
<p>Temporary thumbnails made from the originals and turned by EXIF orientation only. Cropping and true orientation come in the next step.</p>
<main>
{cards}</main></body></html>
"#,
        count = images.len(),
    );
    let path = lib.meta_dir().join("cache/contact-sheet.html");
    std::fs::write(&path, html)?;
    Ok(path)
}
