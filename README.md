# AgenticArchivist

An open-source archive for photographs, documents, and artworks that lives on your own devices.

Every Mac, PC, and phone that belongs to a collection keeps its own copy, originals included, synced directly between devices. Metadata is also written to standard XMP sidecars and catalog files next to the originals, so the collection stays usable without this app. The only online service it calls is Google Gemini, using each person's own API key.

> **Status:** early design and prototyping. Nothing here is ready to use yet.

## Goals

- **Many copies, no central server.** Any device that belongs to a collection is a backup.
- **Low running costs.** No cloud storage or transfer bills; total hosting under $5 a month.
- **Archival formats.** Originals are never modified. Metadata is written to XMP (Dublin Core, IPTC, VRA Core, Getty AAT), VRA Core XML, CSV/JSON, and a BagIt checksum manifest.
- **Family and colleagues.** Share a collection by sending a link over Messages or email.

## Platforms

Mac first, then iPhone, then Windows. Android later.

## Repository layout

| Path | Contents |
| --- | --- |
| `core/` | Rust library: op-log, library database, sidecars, Gemini client, Syncthing control |
| `daemon/` | Rust background process: folder watching, ingest, AI jobs, sync supervision |
| `desktop/` | Tauri + Svelte app for Mac and Windows (not started) |
| `ios/` | Swift iPhone capture app (not started) |
| `site/` | Static site served by GitHub Pages: invite join page |
| `docs/` | Architecture and dependency policy |

## Building

Requires a Rust toolchain (stable, via [rustup](https://rustup.rs)).

```sh
cargo build
cargo test
```

## Documentation

- [Architecture](docs/architecture.md)
- [Dependency policy](docs/dependency-policy.md)
- [Contributing](CONTRIBUTING.md)
- [Security](SECURITY.md)

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT licence ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without any additional terms or conditions.

AgenticArchivist uses [Syncthing](https://syncthing.net) for file transfer. It is not affiliated with or endorsed by the Syncthing project.
