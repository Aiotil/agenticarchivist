# Sync prototype

Tests the riskiest parts of the sync design on one machine, with three Syncthing instances standing in for three computers. This is throwaway code for learning, not the product.

## What it tests

1. A Rust program supervises bundled Syncthing processes and configures them only through the REST API.
2. A signed, per-device metadata log syncs through Syncthing without conflicts and merges to the same state on every device.
3. An invite link lets a new device join, including when the person who sent the invite is offline.
4. A viewer's edits are ignored.

## Scenario

| Instance | Role | How it joins |
| --- | --- | --- |
| A | Admin | Creates the collection |
| C | Editor | Invite link from A, while A is online |
| B | Viewer | Invite link from A, after A has gone offline |

Steps:

1. A generates a normal Syncthing identity; the program checks its own device ID calculation against Syncthing's.
2. A writes a test collection (random "originals" plus previews in `_agenticarchivist/derived/`) and a genesis log entry.
3. A creates an invite for C: a random 32-byte seat secret, from which A derives C's future device identity and writes a signed `add-member` entry for it.
4. C derives the same identity from the link, starts Syncthing with it, and receives the collection.
5. A and C edit the same field at the same time, plus different fields, and each writes a different local sidecar for the same image.
6. A invites B, C picks up the new `add-member` entry from A's log, then A shuts down.
7. B joins using only the link and syncs everything from C.
8. B appends edits and a forged admin entry to its own log; merging ignores them.

## Running it

Download Syncthing v2.1.5 for macOS from the [official releases](https://github.com/syncthing/syncthing/releases/tag/v2.1.5) and check its SHA-256 against `sha256sum.txt.asc`:

```sh
mkdir -p vendor/syncthing && cd vendor/syncthing
curl -LO https://github.com/syncthing/syncthing/releases/download/v2.1.5/syncthing-macos-universal-v2.1.5.zip
curl -LO https://github.com/syncthing/syncthing/releases/download/v2.1.5/sha256sum.txt.asc
grep macos-universal sha256sum.txt.asc && shasum -a 256 syncthing-macos-universal-v2.1.5.zip
ditto -x -k syncthing-macos-universal-v2.1.5.zip .
cd ../..
cargo run --release -p sync-spike
```

Options: `SPIKE_FILES` (default 30), `SPIKE_FILE_MB` (default 4), `SYNCTHING_BIN` (path to another Syncthing binary). Each run writes to `target/sync-spike/run-<time>/` and leaves a `report.md` there. Delete the run folder afterwards; it holds three copies of the test data.

The instances use fixed loopback ports (GUI 18401–18403, sync 22101–22103) and have global discovery, local discovery, relays, NAT traversal, usage reporting, crash reporting, and upgrades turned off, so nothing leaves the machine.

## Results

First run, 16 September 2026, macOS, Syncthing v2.1.5: **all 17 checks passed.** Full report: [results/2026-09-16-mac-loopback.md](results/2026-09-16-mac-loopback.md).

### Findings

- **Syncthing 2.x identifies devices with Ed25519 certificates.** Ed25519 signatures are deterministic, so a certificate built from a fixed key and fixed fields is byte-identical every time. An invite only needs a 32-byte secret for both sides to agree on the new device's ID. Syncthing accepted the derived certificate unchanged.
- **The device log can be signed with the Syncthing device key itself.** Each log starts with the device's certificate, so a log entry is tied to exactly the device that membership entries grant roles to.
- **One writer per file works.** Concurrent edits on two devices produced no Syncthing conflict copies, and both merged to the same state. Sidecars excluded by `.stignore` stayed different on each device, as intended.
- **Inviter offline works.** Because the `add-member` entry travels in the log, any member that has seen it accepts the newcomer.
- **Invite link length:** 544 characters, including spike-only local addresses. Without them it will be shorter.
- **Viewer protection has two layers.** A receive-only Syncthing folder never sends the viewer's changes, and merge-time role checks reject them even if they arrive.
- **Watch disk space.** The test Mac had 4.6 GB free (99% full). Syncthing's default minimum free space (1%) made it pause and retry, which is why B took 28 s. The product must show a clear "not enough space" state rather than a silent retry.

### Not covered yet

- Real networks: discovery, NAT traversal and relays were off; devices used fixed loopback addresses.
- Two physical Macs, large collections (100 GB+), and interrupted transfers.
- Person identity keys above device keys; roles are currently granted per device.
- Replacing the seat identity with a fresh one after joining (single-use links).
- Removing members, deletions and tombstones, and copy counts for the two-copy rule.
- Syncthing on iOS through gomobile.

## Dependencies

All Rust crates here meet the [dependency policy](../../docs/dependency-policy.md): anyhow, serde, serde_json, base64, rand, sha2, tokio, reqwest, rusqlite (bundled SQLite), ed25519-dalek, and rcgen. Syncthing is downloaded separately and not committed.
