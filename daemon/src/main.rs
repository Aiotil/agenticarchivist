//! AgenticArchivist background process.
//!
//! Will watch folders, ingest originals, run AI jobs, write sidecars, serve a
//! local API to the desktop window, and supervise the bundled Syncthing.

fn main() {
    println!(
        "agenticarchivist-daemon {} (metadata folder: {})",
        env!("CARGO_PKG_VERSION"),
        agenticarchivist_core::COLLECTION_META_DIR
    );
}
