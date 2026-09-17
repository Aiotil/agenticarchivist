//! AgenticArchivist core library.
//!
//! Holds the collection library and import. Will also hold the per-device
//! signed op-log, XMP sidecar and catalog writers, the Gemini client, and
//! Syncthing control. See `docs/architecture.md`.

pub mod import;
pub mod library;
pub mod media;
pub mod time;

/// Name of the per-collection folder that holds logs, derived images, and catalog files.
pub const COLLECTION_META_DIR: &str = "_agenticarchivist";

/// XMP namespace for AgenticArchivist's own sidecar fields.
///
/// Written into archival files, so it must never change.
pub const XMP_NAMESPACE: &str = "https://agenticarchivist.com/ns/xmp/1.0/";

/// Preferred prefix for [`XMP_NAMESPACE`] in sidecars.
pub const XMP_PREFIX: &str = "agenticArchivist";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xmp_namespace_is_versioned_https_uri() {
        assert!(XMP_NAMESPACE.starts_with("https://"));
        assert!(XMP_NAMESPACE.ends_with("/1.0/"));
    }
}
