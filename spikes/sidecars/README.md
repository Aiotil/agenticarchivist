# Sidecar prototype

Tests whether AgenticArchivist's archival files can be read and trusted without AgenticArchivist. Throwaway code for learning, not the product.

## What it does

Builds a small sample collection (invented family letters and a portrait) with real 16-bit TIFF and JPEG originals, then writes:

| File | Format |
| --- | --- |
| `IMG_0412.xmp` next to each original | XMP sidecar: Dublin Core, Photoshop, IPTC Core and Extension (Artwork or Object, Location Created, Digital Source Type), and the `agenticArchivist` namespace |
| `_agenticarchivist/derived/*_preview.jpg` | Generated previews with small embedded XMP |
| `_agenticarchivist/derived/*_restored.jpg` | Simulated AI restoration with embedded XMP marking it as AI-altered (IPTC Digital Source Type, AI System Used, AI Prompt Information) |
| `_agenticarchivist/works.vra.xml` | VRA Core 4 XML with work and image records |
| `_agenticarchivist/catalog.csv`, `catalog.json` | Flat catalogue |
| `_agenticarchivist/manifest-sha256.txt` | SHA-256 of originals, sidecars, and derived files |
| `_agenticarchivist/README.txt` | How to use the collection without the app |

Then it checks all of them with independent tools, rebuilds the whole catalogue from the sidecars alone, and edits a sidecar with ExifTool to simulate another app.

Examples of the generated files are in [results/example/](results/example/).

## Running it

Requires [ExifTool](https://exiftool.org) (`brew install exiftool`), plus `xmllint`, `shasum`, and `python3`, which macOS includes.

```sh
cargo run --release -p sidecar-spike
```

Output goes to `target/sidecar-spike/`. Unit tests (no external tools needed): `cargo test -p sidecar-spike`.

## Results

16 September 2026, ExifTool 13.55: **19 of 19 checks passed.** Full report: [results/2026-09-16-exiftool.md](results/2026-09-16-exiftool.md).

### Findings

- **ExifTool validates every sidecar and generated JPEG as OK** and reads every field: Dublin Core, the IPTC Artwork or Object structure (title, inventory number, physical description, dates), Location Created, Digital Source Type, and all 18 `agenticArchivist` properties including nested creators, Getty AAT terms, inscriptions, and per-field AI/manual sources. German text and em dashes survive.
- **The full catalogue rebuilds from sidecars alone**, identical to the source data: works, images and their order and role (recto/verso/detail), vocabulary terms with AAT URIs, inscriptions, dates, and which fields came from AI.
- **Edits from other tools are safe.** After ExifTool rewrote a sidecar with a new title, the reader picked up the change and every AgenticArchivist field survived ExifTool's re-serialisation.
- **AI restorations are clearly labelled** with IPTC's `compositeWithTrainedAlgorithmicMedia` source type and the 2025.1 AI properties, and name the original they came from by file and hash.
- **Sidecar naming works:** `IMG_0412.xmp` normally; `IMG_0600.tif.xmp` and `IMG_0600.jpg.xmp` when two originals share a base name.
- **Checksums work with standard tools:** `shasum -a 256 -c` verifies the manifest, and a single flipped bit in an original is caught and named.
- **Size:** about 6–7 KB per sidecar, so roughly 65 MB for 10,000 images.
- **No `bagit.txt`.** BagIt (RFC 8493) requires the payload inside a `data/` folder, and a browsable collection folder doesn't have one; writing `bagit.txt` would produce an invalid bag. The manifest uses the same `sha256sum` format BagIt uses. A true bag should come from an **Export as BagIt bag** command (APFS clones or hard links avoid copying originals).
- **The previous backend's XMP writer had errors** that this prototype avoids: `AOTitle` written as plain text (IPTC defines it as a language alternative), VRA Core XML nested inside the XMP packet (attributes without a namespace aren't valid RDF), and Getty links under an invented namespace. Types were checked against ExifTool's IPTC tag definitions.
- **Unit tests caught a reader bug:** trimming whitespace around `&amp;` or `&#x2014;` split text and dropped the spaces next to the entity ("Fish&Chips"). Fixed; the reader now trims only whole values.

### Not covered yet

- **Lightroom Classic, Bridge, and digiKam** aren't installed on the test Mac. ExifTool is the strictest common reader, but each app should be checked, especially whether each shows the IPTC Artwork or Object fields.
- **VRA Core XSD validation.** `works.vra.xml` is well-formed; validating against the Library of Congress schema needs that schema downloaded.
- **Sidecars written by other apps.** Lightroom and others may use RDF attribute shorthand or different structures; the reader handles attribute shorthand in a unit test but hasn't seen real Lightroom output.
- **XMP embedded in TIFF or HEIC derivatives**, and performance on large collections.

## Dependencies

anyhow, serde, serde_json, quick-xml, sha2, and image (TIFF and JPEG only), all within the [dependency policy](../../docs/dependency-policy.md). ExifTool is used only as an external checker, not bundled.
