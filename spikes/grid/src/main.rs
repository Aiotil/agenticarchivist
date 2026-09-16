//! Grid prototype: serve a large thumbnail library over local HTTP, the way the
//! background process will serve the desktop window, plus a virtualised grid
//! page that benchmarks itself.
//!
//! Usage: `grid-spike [count] [port]` (defaults 50000, 18480), then open
//! `http://127.0.0.1:18480/?autorun=1` in the browser engine under test.

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

const INDEX: &str = include_str!("index.html");

struct AppState {
    count: usize,
    thumbs: PathBuf,
    results: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let count: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(50_000);
    let port: u16 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(18480);

    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let root = workspace.join("target/grid-spike");
    let thumbs = root.join("thumbs");
    generate_thumbnails(&thumbs, count)?;

    let state = Arc::new(AppState {
        count,
        thumbs,
        results: root.join("results.jsonl"),
    });
    let app = Router::new()
        .route("/", get(|| async { Html(INDEX) }))
        .route("/api/meta", get(meta))
        .route("/api/works", get(works))
        .route("/thumb/{id}", get(thumb))
        .route("/api/results", post(results))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    println!("Serving {count} thumbnails at http://127.0.0.1:{port}/ (autorun: /?autorun=1)");
    axum::serve(listener, app).await?;
    Ok(())
}

/// Three aspect ratios so the grid lays out mixed portrait, landscape, and square work.
fn dimensions(id: usize) -> (u32, u32) {
    match id % 3 {
        0 => (256, 192),
        1 => (192, 256),
        _ => (224, 224),
    }
}

fn generate_thumbnails(dir: &std::path::Path, count: usize) -> Result<()> {
    let marker = dir.join(format!(".complete-{count}"));
    if marker.exists() {
        println!("Using existing thumbnails in {}", dir.display());
        return Ok(());
    }
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    println!("Generating {count} thumbnails…");
    let started = std::time::Instant::now();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
    let total = std::thread::scope(|scope| -> Result<u64> {
        let handles: Vec<_> = (0..workers)
            .map(|w| {
                scope.spawn(move || -> Result<u64> {
                    let mut bytes = 0u64;
                    for id in (0..count).filter(|i| i % workers == w) {
                        let data = thumbnail_jpeg(id)?;
                        bytes += data.len() as u64;
                        std::fs::write(dir.join(format!("{id}.jpg")), data)?;
                    }
                    Ok(bytes)
                })
            })
            .collect();
        let mut total = 0;
        for h in handles {
            total += h.join().expect("thumbnail worker panicked")?;
        }
        Ok(total)
    })?;
    println!(
        "Generated {count} thumbnails, {:.0} MB total ({:.1} KB average), in {:.0} s",
        total as f64 / 1e6,
        total as f64 / count as f64 / 1e3,
        started.elapsed().as_secs_f64()
    );
    std::fs::write(marker, "")?;
    Ok(())
}

fn thumbnail_jpeg(id: usize) -> Result<Vec<u8>> {
    let (width, height) = dimensions(id);
    let hue = (id * 37 % 360) as f32;
    let img = image::ImageBuffer::from_fn(width, height, |x, y| {
        // A soft gradient with blocks, roughly like a photographed page.
        let t = (x + y) as f32 / (width + height) as f32;
        let block = ((x / 24 + y / 24 + id as u32) % 5) as f32 * 9.0;
        let (r, g, b) = hsv(hue, 0.35, 0.45 + 0.4 * t);
        image::Rgb([
            (r + block).min(255.0) as u8,
            (g + block).min(255.0) as u8,
            (b + block * 0.5).min(255.0) as u8,
        ])
    });
    let mut buf = std::io::Cursor::new(vec![]);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 80).encode_image(&img)?;
    Ok(buf.into_inner())
}

fn hsv(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    ((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0)
}

async fn meta(State(state): State<Arc<AppState>>) -> Json<Value> {
    Json(json!({ "count": state.count }))
}

#[derive(Deserialize)]
struct Page {
    offset: usize,
    limit: usize,
}

async fn works(State(state): State<Arc<AppState>>, Query(page): Query<Page>) -> Json<Value> {
    let end = (page.offset + page.limit.min(2000)).min(state.count);
    let items: Vec<Value> = (page.offset..end)
        .map(|id| {
            let (w, h) = dimensions(id);
            json!({
                "id": id,
                "title": format!("Work {:05} · Letter to Rose", id + 1),
                "date": format!("c. {}", 1900 + id % 90),
                "w": w,
                "h": h,
            })
        })
        .collect();
    Json(json!({ "items": items }))
}

async fn thumb(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> impl IntoResponse {
    let Some(n) = id
        .strip_suffix(".jpg")
        .and_then(|n| n.parse::<usize>().ok())
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(state.thumbs.join(format!("{n}.jpg"))).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn results(State(state): State<Arc<AppState>>, body: Bytes) -> StatusCode {
    let line = String::from_utf8_lossy(&body).replace('\n', " ");
    println!("RESULT {line}");
    let write = || -> Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&state.results)
            .context("opening results file")?;
        writeln!(f, "{line}")?;
        Ok(())
    };
    match write() {
        Ok(()) => StatusCode::NO_CONTENT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
