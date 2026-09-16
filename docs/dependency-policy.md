# Dependency policy

Every dependency must meet **both** rules.

1. **Licence.** An open-source licence compatible with an app licensed MIT OR Apache-2.0 that also ships on the iOS App Store and as signed Mac and Windows binaries.
   - Permissive licences (MIT, Apache-2.0, BSD, ISC, Zlib, CC0, public domain) are fine.
   - MPL-2.0 and CDDL-1.0 are fine: publish any changes to their files and link to their source.
   - LGPL is allowed only on desktop, as a separately replaceable dynamic library. Never on iOS.
   - GPL and AGPL are not allowed.
   - Model weights must be under an open licence with no non-commercial clause, and trained on data that permits it.
2. **Maintenance.** Several years of active development with more than one regular contributor or an organisation behind it. Young single-maintainer projects and self-described pre-production releases don't qualify.

Exceptions are recorded below with the reason and the fallback.

## Approved

| Dependency | Licence | Use |
| --- | --- | --- |
| Syncthing | MPL-2.0 | File sync (bundled, unmodified, pinned version) |
| Tauri 2 + official plugins | MIT OR Apache-2.0 | Desktop window, updater, autostart |
| Svelte 5 | MIT | Desktop UI |
| UniFFI | MPL-2.0 | Rust ↔ Swift/Kotlin bindings |
| SQLite, rusqlite | Public domain, MIT | Library database |
| tokio, axum, reqwest | MIT, MIT OR Apache-2.0 | Async runtime, local API, Gemini calls |
| notify | CC0-1.0 | Folder watching |
| ed25519-dalek, sha2 | BSD-3-Clause, MIT OR Apache-2.0 | Log signatures, checksums |
| quick-xml | MIT | XMP and VRA Core XML |
| image, tiff | MIT OR Apache-2.0, MIT | Image decoding, 16-bit TIFF |
| Little CMS | MIT | Colour management (Windows) |
| LibRaw | CDDL-1.0 (elected) | RAW decoding (Windows) |
| PDFium | BSD-3-Clause AND Apache-2.0 | PDF pages (Windows) |
| Tesseract 5 | Apache-2.0 | OCR fallback |
| ONNX Runtime | MIT | Image embeddings |
| DINOv2, SigLIP 2 weights | Apache-2.0 | Similarity for grouping |

## Accepted exceptions

| Dependency | Why it's an exception | Fallback |
| --- | --- | --- |
| Syncthing on iOS via gomobile | No upstream iOS support; Go API not stable | iroh 1.x for iOS only |
| `ort` crate | 2.0 still a release candidate | Pin exact version; `tract` |

## Rejected

| Dependency | Reason |
| --- | --- |
| iroh-blobs | Self-described as not production quality |
| Exiv2, gexiv2, rexiv2 | GPL |
| MuPDF, Poppler | AGPL, GPL |
| libheif, libde265, x265 | LGPL/GPL and HEVC patent licensing; use OS codecs |
| rawler / dnglab | LGPL, can't be dynamically linked from Rust |
| libvips | LGPL, heavy on iOS |
| ocrs, oar-ocr, Rust Tesseract crates | Young, single-maintainer, or dormant |
| InsightFace, AuraFace, SFace | Non-commercial weights or unclear training data |

Last reviewed: 16 September 2026.
