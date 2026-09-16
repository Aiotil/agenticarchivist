# Architecture

This is a summary of the current design. It will change as the prototypes answer open questions.

## Principles

1. **Every device in a collection holds a copy.** There is no central server that owns the data.
2. **Originals are never modified.** Corrections and restorations are separate files.
3. **The database is a cache.** Everything catalogued is also written in open formats next to the files, and the app can rebuild its database from the folder.
4. **Gemini is the only online service.** Each person uses their own API key.
5. **Total hosting stays under $5 a month.**

## Components

| Layer | Built with | Platforms | Job |
| --- | --- | --- | --- |
| Core (`core/`) | Rust | All | Op-log, SQLite library, sidecars and catalog files, Gemini client, job claims, Syncthing control |
| Background process (`daemon/`) | Rust | Mac, Windows | Folder watching, ingest, AI jobs, local HTTP API, supervises bundled Syncthing |
| Syncthing | Upstream binary, pinned | Mac, Windows (iPhone via gomobile) | File transfer, discovery, relays |
| Desktop window (`desktop/`) | Tauri 2 + Svelte 5 | Mac, then Windows | Library grid, work editor, grouping review, devices and sharing |
| iPhone app (`ios/`) | Swift + core via UniFFI | iOS | RAW bracket capture, browsing, partial copy |

## Sync

Syncthing moves files. Conflicts are avoided because no two devices ever write the same file:

- **Originals** are written once at ingest.
- **Derived images** (previews, thumbnails, restorations) are named by the original's SHA-256 and written once.
- **Metadata edits** are appended to signed log segments in a folder owned by one device: `_agenticarchivist/log/<device-id>/`.
- **Sidecars and catalog files** are generated locally from the merged log and excluded from sync.

Each device replays all logs into SQLite. Plain fields are last-writer-wins by hybrid logical clock; tags and grouping are add-wins sets.

Devices find each other through Syncthing's public discovery servers, connect directly where possible, and use the community relay pool only when no direct path works. Home upload speed is the main bottleneck, so first copies of large collections can be seeded from a drive.

## Collection folder

```
My Collection/
├── 1943 Letter to Rose/
│   ├── IMG_0412.tif            original, never modified
│   └── IMG_0412.xmp            sidecar, generated locally
└── _agenticarchivist/
    ├── README.txt
    ├── works.vra.xml           VRA Core 4 XML
    ├── catalog.csv, catalog.json
    ├── bagit.txt, manifest-sha256.txt
    ├── log/<device-id>/        signed op-log segments (synced)
    ├── derived/                previews and thumbnails (synced)
    └── cache/                  SQLite library (local, rebuildable)
```

Sidecars use XMP with Dublin Core, IPTC Core and Extension, VRA Core, and Getty AAT links. AgenticArchivist's own fields use the namespace `https://agenticarchivist.com/ns/xmp/1.0/`, which must never change.

## People and roles

- A **person** has an Ed25519 identity key in the system keychain.
- A **device** has a Syncthing device ID certified by the person key.
- **Roles:** viewer (receive-only), editor (catalogue, add images, run AI jobs), admin (invite, change roles, remove).
- Each log entry names the latest membership entry its author had seen; entries from authors without the required role are ignored everywhere.

## Invites

An invite is a link sent by Messages or email, for example `https://join.agenticarchivist.com/#v1.…`.

1. The sender creates a one-time seat secret, derives a device identity from it, and writes a signed `add-member` entry that pre-approves that identity.
2. The recipient opens the link, downloads the app, and joins. The app replaces the seat identity with a fresh one, so the link works once.
3. Any online member device can accept the newcomer, so the sender doesn't need to be online.

The secret lives in the URL fragment, which browsers never send to a server. Links expire after 7 days and can be protected with a PIN sent separately.

## AI

- **Classification, titling, restoration:** Gemini, called directly with the person's own key.
- **Grouping:** sequence rules, perceptual hashes, and image embeddings (DINOv2 or SigLIP 2 via ONNX Runtime) narrow candidates before any Gemini call.
- **OCR:** Apple Vision on Apple platforms; Windows.Media.Ocr or Tesseract on Windows.
- **Face recognition:** out of scope. No available model has both an open licence and clear training-data provenance.

Before calling Gemini, a device writes a `claim job` entry with a lease so other devices skip the job.

## Distribution

| Platform | Channel | Signing |
| --- | --- | --- |
| Mac | GitHub Releases, Homebrew cask | Developer ID, notarised |
| iPhone | App Store | Apple distribution |
| Windows | GitHub Releases (NSIS), winget, Scoop | SignPath Foundation |
| Android | Later | — |

## Build order

1. Prototypes: bundled Syncthing with invite links; 50,000-thumbnail grid in Tauri on Mac and Windows; XMP sidecars read back by Lightroom, digiKam, and exiftool
2. Mac app on a single device
3. Sync between Macs
4. Invites, roles, and sharing
5. iPhone
6. Windows
7. Migration from the previous cloud version
8. Android
