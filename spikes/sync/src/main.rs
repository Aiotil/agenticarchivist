//! Sync prototype: three Syncthing instances on one machine.
//!
//! A (admin) creates a collection and invites C (editor). A and C make
//! conflicting metadata edits that must merge identically. A invites B
//! (viewer), goes offline, and B must still join and receive everything
//! from C using only the invite link.

mod deviceid;
mod identity;
mod invite;
mod oplog;
mod syncthing;

use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use identity::DeviceIdentity;
use invite::Invite;
use oplog::{LogWriter, Op, Role, State};
use syncthing::{Instance, Network};

const FOLDER_ID: &str = "grandpa-saul-letters";
const SYNC_TIMEOUT: Duration = Duration::from_secs(300);

/// Local files that each device generates for itself and never syncs.
const STIGNORE: &str = "\
// AgenticArchivist: generated on each device, never synced
*.xmp
/_agenticarchivist/cache
/_agenticarchivist/catalog.csv
/_agenticarchivist/catalog.json
/_agenticarchivist/works.vra.xml
/_agenticarchivist/manifest-sha256.txt
/_agenticarchivist/bagit.txt
";

struct Report {
    lines: Vec<String>,
    failures: usize,
}

impl Report {
    fn check(&mut self, ok: bool, what: impl Into<String>) {
        let what = what.into();
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        self.lines
            .push(format!("- {} {what}", if ok { "PASS" } else { "FAIL" }));
        if !ok {
            self.failures += 1;
        }
    }

    fn note(&mut self, what: impl Into<String>) {
        let what = what.into();
        println!("     {what}");
        self.lines.push(format!("  - {what}"));
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let files: usize = env_or("SPIKE_FILES", 30);
    let file_mb: usize = env_or("SPIKE_FILE_MB", 4);
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let binary = std::env::var("SYNCTHING_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            workspace.join("vendor/syncthing/syncthing-macos-universal-v2.1.5/syncthing")
        });
    ensure!(
        binary.exists(),
        "Syncthing binary not found at {}",
        binary.display()
    );

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let run_dir = workspace.join(format!("target/sync-spike/run-{stamp}"));
    std::fs::create_dir_all(&run_dir)?;
    println!("Run directory: {}", run_dir.display());

    let mut report = Report {
        lines: vec![],
        failures: 0,
    };
    let relay_mode = std::env::args().nth(1).as_deref() == Some("relay");
    let result = if relay_mode {
        let mut a = Instance::new("A-relay", &run_dir, &binary, 18411, 0, Network::RelayOnly);
        let mut b = Instance::new("B-relay", &run_dir, &binary, 18412, 0, Network::RelayOnly);
        let result = relay_scenario(&mut a, &mut b, files, file_mb, &mut report).await;
        for inst in [&mut a, &mut b] {
            let _ = inst.stop().await;
        }
        result
    } else {
        let mut a = Instance::new(
            "A-admin-mac",
            &run_dir,
            &binary,
            18401,
            22101,
            Network::Loopback,
        );
        let mut c = Instance::new(
            "C-editor-mac",
            &run_dir,
            &binary,
            18402,
            22102,
            Network::Loopback,
        );
        let mut b = Instance::new(
            "B-viewer-mac",
            &run_dir,
            &binary,
            18403,
            22103,
            Network::Loopback,
        );
        let result = scenario(&mut a, &mut c, &mut b, files, file_mb, &mut report).await;
        for inst in [&mut a, &mut c, &mut b] {
            let _ = inst.stop().await;
        }
        result
    };
    if let Err(e) = &result {
        report.check(false, format!("scenario aborted: {e:#}"));
    }

    let summary = format!(
        "# Sync prototype run ({})\n\nSyncthing: {}\nFiles: {files} × {file_mb} MB\n\n{}\n\nResult: {} failure(s)\n",
        if relay_mode { "relay only" } else { "loopback" },
        binary.display(),
        report.lines.join("\n"),
        report.failures
    );
    std::fs::write(run_dir.join("report.md"), &summary)?;
    println!(
        "\nReport written to {}",
        run_dir.join("report.md").display()
    );
    if report.failures > 0 {
        bail!("{} check(s) failed", report.failures);
    }
    Ok(())
}

async fn scenario(
    a: &mut Instance,
    c: &mut Instance,
    b: &mut Instance,
    files: usize,
    file_mb: usize,
    report: &mut Report,
) -> Result<()> {
    let mut addresses = BTreeMap::new();

    // A: a normal install creates the collection.
    a.prepare(None)?;
    a.start().await?;
    let a_id = DeviceIdentity::load_from_home(&a.home)?;
    let st_id = a.my_id().await?;
    report.check(
        st_id == a_id.device_id,
        format!("device ID computed from the certificate matches Syncthing's ({st_id})"),
    );
    addresses.insert(a_id.device_id.clone(), a.listen_address());

    std::fs::write(a.folder_path.join(".stignore"), STIGNORE)?;
    let total_bytes = write_test_collection(&a.folder_path, files, file_mb)?;
    let mut a_log = LogWriter::open(&a.folder_path, &a_id)?;
    let collection = crate::to_hex(rand::random::<[u8; 8]>());
    a_log.append(
        Op::Genesis {
            collection: collection.clone(),
            folder: FOLDER_ID.into(),
        },
        None,
    )?;
    let a_state = merge(&a.folder_path, &a.home)?;
    reconcile(a, &a_id.device_id, &a_state, &addresses).await?;

    // A invites C (editor) while A is online.
    let (c_link, c_seat) =
        create_invite(&mut a_log, a, &a_id, &collection, Role::Editor, &addresses)?;
    report.check(
        c_link.len() < 1000,
        format!("invite link is {} characters", c_link.len()),
    );
    let c_invite = Invite::from_link(&c_link)?;
    let c_id = DeviceIdentity::from_seat_secret(&c_invite.seat_secret()?)?;
    ensure!(c_id.device_id == c_seat, "seat identity mismatch");
    addresses.insert(c_id.device_id.clone(), c.listen_address());
    let a_state = merge(&a.folder_path, &a.home)?;
    reconcile(a, &a_id.device_id, &a_state, &addresses).await?;

    join_from_invite(c, &c_id, &c_invite).await?;
    let st_c = c.my_id().await?;
    report.check(
        st_c == c_id.device_id,
        "C's Syncthing runs with the identity derived from the invite secret, exactly as A predicted",
    );

    let started = Instant::now();
    wait_for_same_tree(a, c, report, "C receives the collection from A").await?;
    let secs = started.elapsed().as_secs_f64();
    report.note(format!(
        "{:.0} MB in {secs:.1} s ({:.0} MB/s over loopback)",
        total_bytes as f64 / 1e6,
        total_bytes as f64 / 1e6 / secs
    ));

    // Conflicting metadata edits on A and C.
    let c_state = merge(&c.folder_path, &c.home)?;
    let mut c_log = LogWriter::open(&c.folder_path, &c_id)?;
    if let Some(h) = c_state.max_hlc {
        c_log.observe(h);
    }
    let head_a = oplog::membership_head(&merge(&a.folder_path, &a.home)?)?;
    let head_c = oplog::membership_head(&c_state)?;
    a_log.append(
        set("work-001", "title", "Letter to Rose"),
        Some(head_a.clone()),
    )?;
    tokio::time::sleep(Duration::from_millis(5)).await;
    c_log.append(
        set("work-001", "title", "1943 Letter to Rose"),
        Some(head_c.clone()),
    )?;
    c_log.append(set("work-001", "date", "1943"), Some(head_c))?;
    a_log.append(
        set("work-002", "title", "Wedding portrait, c. 1950"),
        Some(head_a),
    )?;

    // Different sidecar content on each side: must stay local and never conflict.
    std::fs::write(
        c.folder_path.join("work-001/IMG_0001.xmp"),
        "<x:xmpmeta>C's local sidecar</x:xmpmeta>",
    )?;
    std::fs::write(
        a.folder_path.join("work-001/IMG_0001.xmp"),
        "<x:xmpmeta>A's local sidecar</x:xmpmeta>",
    )?;
    a.scan(FOLDER_ID).await?;
    c.scan(FOLDER_ID).await?;

    wait_for_same_tree(a, c, report, "both devices' logs reach both devices").await?;
    wait_for_same_tree(c, a, report, "A is fully up to date with C").await?;
    let a_state = merge(&a.folder_path, &a.home)?;
    let c_state = merge(&c.folder_path, &c.home)?;
    report.check(
        a_state.digest() == c_state.digest(),
        format!("A and C merge to the same state ({})", a_state.digest()),
    );
    let title = &a_state.fields[&("work-001".to_string(), "title".to_string())].0;
    report.check(
        title == "1943 Letter to Rose",
        format!("concurrent title edits resolve to the later write: \"{title}\""),
    );
    report.check(
        a_state.fields.len() == 3,
        "edits to different fields are all kept",
    );
    let a_xmp = std::fs::read_to_string(a.folder_path.join("work-001/IMG_0001.xmp"))?;
    report.check(
        a_xmp.contains("A's local"),
        "sidecars stay local to each device",
    );
    report.check(
        count_conflicts(&a.folder_path) + count_conflicts(&c.folder_path) == 0,
        "no Syncthing conflict copies on A or C",
    );

    // A invites B (viewer), then goes offline.
    let (b_link, b_seat) =
        create_invite(&mut a_log, a, &a_id, &collection, Role::Viewer, &addresses)?;
    let b_invite = Invite::from_link(&b_link)?;
    let b_id = DeviceIdentity::from_seat_secret(&b_invite.seat_secret()?)?;
    ensure!(b_id.device_id == b_seat, "seat identity mismatch");
    addresses.insert(b_id.device_id.clone(), b.listen_address());
    a.scan(FOLDER_ID).await?;

    // C must learn about the new seat from A's log before A disappears.
    let deadline = Instant::now() + SYNC_TIMEOUT;
    loop {
        let state = merge(&c.folder_path, &c.home)?;
        if state.members.contains_key(&b_seat) {
            reconcile(c, &c_id.device_id, &state, &addresses).await?;
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "C never saw B's add-member entry"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    report.check(
        true,
        "C accepted B's seat from the signed add-member entry in A's log",
    );

    a.stop().await?;
    report.note("A (the inviter) is now offline");

    join_from_invite(b, &b_id, &b_invite).await?;
    let started = Instant::now();
    wait_for_same_tree(
        c,
        b,
        report,
        "B joins with A offline and receives the whole collection from C",
    )
    .await?;
    report.note(format!(
        "B synced in {:.1} s",
        started.elapsed().as_secs_f64()
    ));
    let connected = b.connected_devices().await?;
    report.check(
        connected.contains(&c_id.device_id) && !connected.contains(&a_id.device_id),
        "B is connected to C only",
    );

    let b_state = merge(&b.folder_path, &b.home)?;
    let c_state = merge(&c.folder_path, &c.home)?;
    report.check(
        b_state.digest() == c_state.digest(),
        "B merges to the same state as C",
    );
    report.check(
        b_state.members.get(&b_id.device_id) == Some(&Role::Viewer),
        "B's role in the merged log is viewer",
    );

    // A viewer's edits are ignored when merging.
    let mut b_log = LogWriter::open(&b.folder_path, &b_id)?;
    if let Some(h) = b_state.max_hlc {
        b_log.observe(h);
    }
    let head_b = oplog::membership_head(&b_state)?;
    b_log.append(
        set("work-001", "title", "Viewer was here"),
        Some(head_b.clone()),
    )?;
    b_log.append(
        Op::AddMember {
            device: "FORGED".into(),
            role: Role::Admin,
        },
        Some(head_b),
    )?;
    let b_state = merge(&b.folder_path, &b.home)?;
    let title = &b_state.fields[&("work-001".to_string(), "title".to_string())].0;
    report.check(
        title == "1943 Letter to Rose" && !b_state.members.contains_key("FORGED"),
        "a viewer's own log entries are rejected when merging (title unchanged, no forged admin)",
    );
    report.note("B's folder is receive-only, so Syncthing never sends B's log; merge-time checks are the second line of defence");
    Ok(())
}

/// Two devices that may only reach each other through public discovery and
/// the community relay pool, as two homes behind strict routers would.
async fn relay_scenario(
    a: &mut Instance,
    b: &mut Instance,
    files: usize,
    file_mb: usize,
    report: &mut Report,
) -> Result<()> {
    let no_addresses = BTreeMap::new();

    a.prepare(None)?;
    a.start().await?;
    let a_id = DeviceIdentity::load_from_home(&a.home)?;
    ensure!(a.my_id().await? == a_id.device_id, "device ID mismatch");
    std::fs::write(a.folder_path.join(".stignore"), STIGNORE)?;
    let total_bytes = write_test_collection(&a.folder_path, files, file_mb)?;
    let mut a_log = LogWriter::open(&a.folder_path, &a_id)?;
    let collection = to_hex(rand::random::<[u8; 8]>());
    a_log.append(
        Op::Genesis {
            collection: collection.clone(),
            folder: FOLDER_ID.into(),
        },
        None,
    )?;
    let a_state = merge(&a.folder_path, &a.home)?;
    reconcile(a, &a_id.device_id, &a_state, &no_addresses).await?;

    let (link, seat) = create_invite(
        &mut a_log,
        a,
        &a_id,
        &collection,
        Role::Editor,
        &no_addresses,
    )?;
    let invite = Invite::from_link(&link)?;
    report.check(
        invite.spike_addresses.is_empty(),
        format!(
            "invite carries only device IDs, no addresses ({} characters)",
            link.len()
        ),
    );
    let a_state = merge(&a.folder_path, &a.home)?;
    reconcile(a, &a_id.device_id, &a_state, &no_addresses).await?;

    let b_id = DeviceIdentity::from_seat_secret(&invite.seat_secret()?)?;
    ensure!(b_id.device_id == seat, "seat identity mismatch");
    // Resolve the members' current addresses ourselves and hand them to
    // Syncthing. Syncthing caches a "not found" lookup for the ~30 minutes the
    // discovery server asks for, which would stall a device that looked a
    // member up a moment too early.
    b.prepare(Some(&b_id))?;
    std::fs::write(b.folder_path.join(".stignore"), STIGNORE)?;
    b.start().await?;
    let resolve_started = Instant::now();
    let a_addresses = loop {
        if let Some(found) = public_lookup_addresses(&a_id.device_id) {
            break found;
        }
        if resolve_started.elapsed() > Duration::from_secs(600) {
            report.check(
                false,
                "A becomes visible in public discovery within 10 minutes",
            );
            bail!("A never appeared in discovery");
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
    };
    report.check(
        true,
        format!(
            "B resolved A through public discovery after {:.0} s: {}",
            resolve_started.elapsed().as_secs_f64(),
            a_addresses.join(", ").split('?').next().unwrap_or("")
        ),
    );
    let mut with_dynamic = a_addresses.clone();
    with_dynamic.push("dynamic".into());
    b.ensure_device_addresses(&a_id.device_id, "A", &with_dynamic)
        .await?;
    b.ensure_folder(
        &invite.folder,
        &invite.name,
        &[b_id.device_id.clone(), a_id.device_id.clone()],
        false,
    )
    .await?;

    let started = Instant::now();
    let deadline = started + Duration::from_secs(600);
    let mut next_diag = started;
    let connection = loop {
        if let Some(conn) = b.connection(&a_id.device_id).await? {
            break conn;
        }
        if Instant::now() >= next_diag {
            next_diag += Duration::from_secs(30);
            let t = started.elapsed().as_secs();
            for (label, inst, other) in [("A", &*a, &b_id.device_id), ("B", &*b, &a_id.device_id)] {
                let status = inst.get("/rest/system/status").await.unwrap_or_default();
                let cache = inst.get("/rest/system/discovery").await.unwrap_or_default();
                report.note(format!(
                    "t={t}s {label}: discovery={} listeners={} cached lookup of other={}",
                    status["discoveryStatus"],
                    status["connectionServiceStatus"],
                    cache[other.as_str()]
                ));
            }
            for (label, id) in [("A", &a_id.device_id), ("B", &b_id.device_id)] {
                report.note(format!(
                    "t={t}s public lookup {label}: {}",
                    public_lookup(id)
                ));
            }
        }
        if Instant::now() > deadline {
            report.check(false, "B finds and connects to A within 10 minutes");
            bail!("no connection");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };
    let conn_type = connection["type"].as_str().unwrap_or("unknown").to_string();
    let relay_host = connection["address"]
        .as_str()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();
    report.check(
        true,
        format!(
            "B connected to A {:.0} s after being given A's address",
            started.elapsed().as_secs_f64()
        ),
    );
    report.check(
        conn_type.starts_with("relay"),
        format!("connection went through a community relay (type {conn_type}, {relay_host})"),
    );

    let transfer_started = Instant::now();
    wait_for_same_tree_within(
        a,
        b,
        report,
        "B receives the collection over the relay",
        Duration::from_secs(1800),
    )
    .await?;
    let secs = transfer_started.elapsed().as_secs_f64();
    let mbit = total_bytes as f64 * 8.0 / 1e6 / secs;
    report.note(format!(
        "{:.0} MB in {secs:.0} s: {mbit:.1} Mbit/s through the relay",
        total_bytes as f64 / 1e6
    ));
    report.note(format!(
        "at that rate 1 GB takes {:.0} minutes and 1 TB about {:.0} days",
        8000.0 / mbit / 60.0,
        8_000_000.0 / mbit / 86400.0
    ));

    let a_state = merge(&a.folder_path, &a.home)?;
    let b_state = merge(&b.folder_path, &b.home)?;
    report.check(
        a_state.digest() == b_state.digest(),
        "A and B merge to the same state over the relay",
    );
    Ok(())
}

/// Ask the public discovery server what it knows about a device.
fn public_lookup(device_id: &str) -> String {
    let url = format!("https://discovery-lookup.syncthing.net/v2/?device={device_id}");
    match std::process::Command::new("curl")
        .args(["-s", "-m", "10", "-w", " HTTP %{http_code}", &url])
        .output()
    {
        Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        Err(e) => format!("curl failed: {e}"),
    }
}

/// Current addresses for a device from the public discovery server, if listed.
fn public_lookup_addresses(device_id: &str) -> Option<Vec<String>> {
    let url = format!("https://discovery-lookup.syncthing.net/v2/?device={device_id}");
    let out = std::process::Command::new("curl")
        .args(["-s", "-m", "10", "-f", &url])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let addresses: Vec<String> = json["addresses"]
        .as_array()?
        .iter()
        .filter_map(|a| a.as_str().map(str::to_string))
        .collect();
    (!addresses.is_empty()).then_some(addresses)
}

/// Lowercase hexadecimal encoding.
pub fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn set(entity: &str, field: &str, value: &str) -> Op {
    Op::SetField {
        entity: entity.into(),
        field: field.into(),
        value: value.into(),
    }
}

fn env_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn merge(folder: &Path, home: &Path) -> Result<State> {
    let (entries, problems) = oplog::load_all(folder)?;
    for p in problems {
        println!("     log problem: {p}");
    }
    let state = oplog::materialise(entries);
    oplog::write_sqlite(&state, &home.join("library.sqlite"))?;
    Ok(state)
}

/// Make Syncthing's devices and folder sharing match the merged membership.
async fn reconcile(
    inst: &Instance,
    own_id: &str,
    state: &State,
    addresses: &BTreeMap<String, String>,
) -> Result<()> {
    let mut devices = vec![own_id.to_string()];
    for device in state.members.keys() {
        if device == own_id {
            continue;
        }
        // Without a known address, Syncthing finds the device through discovery.
        let addr = addresses.get(device).map_or("dynamic", String::as_str);
        inst.ensure_device(device, &short(device), addr).await?;
        devices.push(device.clone());
    }
    let receive_only = state.members.get(own_id) == Some(&Role::Viewer);
    inst.ensure_folder(FOLDER_ID, "Grandpa Saul Letters", &devices, receive_only)
        .await
}

fn create_invite(
    log: &mut LogWriter,
    inst: &Instance,
    own: &DeviceIdentity,
    collection: &str,
    role: Role,
    addresses: &BTreeMap<String, String>,
) -> Result<(String, String)> {
    let state = merge(&inst.folder_path, &inst.home)?;
    let secret: [u8; 32] = rand::random();
    let seat = DeviceIdentity::from_seat_secret(&secret)?;
    log.append(
        Op::AddMember {
            device: seat.device_id.clone(),
            role,
        },
        Some(oplog::membership_head(&state)?),
    )?;

    let mut members: Vec<String> = state.members.keys().cloned().collect();
    if !members.contains(&own.device_id) {
        members.push(own.device_id.clone());
    }
    let invite = Invite {
        collection: collection.into(),
        folder: FOLDER_ID.into(),
        name: "Grandpa Saul Letters".into(),
        role,
        expires: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs()
            + 7 * 86400,
        seat: URL_SAFE_NO_PAD.encode(secret),
        spike_addresses: members
            .iter()
            .filter_map(|m| addresses.get(m).map(|a| (m.clone(), a.clone())))
            .collect(),
        members,
    };
    Ok((invite.to_link()?, seat.device_id))
}

/// What a newcomer's app does after the person pastes an invite link.
async fn join_from_invite(
    inst: &mut Instance,
    identity: &DeviceIdentity,
    invite: &Invite,
) -> Result<()> {
    inst.prepare(Some(identity))?;
    std::fs::write(inst.folder_path.join(".stignore"), STIGNORE)?;
    inst.start().await?;
    let mut devices = vec![identity.device_id.clone()];
    for member in &invite.members {
        let addr = invite
            .spike_addresses
            .get(member)
            .map_or("dynamic", String::as_str);
        inst.ensure_device(member, &short(member), addr).await?;
        devices.push(member.clone());
    }
    inst.ensure_folder(
        &invite.folder,
        &invite.name,
        &devices,
        invite.role == Role::Viewer,
    )
    .await
}

fn short(device_id: &str) -> String {
    device_id.chars().take(7).collect()
}

fn write_test_collection(root: &Path, files: usize, file_mb: usize) -> Result<u64> {
    let mut total = 0u64;
    let derived = root.join("_agenticarchivist/derived");
    std::fs::create_dir_all(&derived)?;
    for i in 1..=files {
        let work = root.join(format!("work-{:03}", i.div_ceil(2)));
        std::fs::create_dir_all(&work)?;
        let mut data = vec![0u8; file_mb * 1024 * 1024];
        for chunk in data.chunks_mut(32) {
            let r: [u8; 32] = rand::random();
            chunk.copy_from_slice(&r[..chunk.len()]);
        }
        let sha = crate::to_hex(Sha256::digest(&data));
        std::fs::write(work.join(format!("IMG_{i:04}.tif")), &data)?;
        let preview = &data[..(256 * 1024).min(data.len())];
        std::fs::write(derived.join(format!("{sha}_preview.jpg")), preview)?;
        total += data.len() as u64 + preview.len() as u64;
    }
    Ok(total)
}

/// Hash of every synced file, skipping Syncthing markers and local-only files.
fn tree_digest(root: &Path) -> Result<(String, usize)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = path.strip_prefix(root)?.to_string_lossy().to_string();
            // Skip Syncthing markers, its in-progress ".syncthing.*.tmp" downloads,
            // and local-only files.
            if name.starts_with(".st")
                || name.starts_with(".syncthing.")
                || name.ends_with(".xmp")
                || rel.starts_with("_agenticarchivist/cache")
            {
                continue;
            }
            if entry.file_type()?.is_dir() {
                walk(&path, root, out)?;
            } else {
                // A file can be renamed away between listing and reading while
                // Syncthing is still writing; the next poll picks it up.
                match std::fs::read(&path) {
                    Ok(bytes) => out.push((rel, crate::to_hex(Sha256::digest(bytes)))),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(())
    }
    let mut files = vec![];
    walk(root, root, &mut files)?;
    files.sort();
    let digest = crate::to_hex(&Sha256::digest(format!("{files:?}").as_bytes())[..8]);
    Ok((digest, files.len()))
}

fn count_conflicts(root: &Path) -> usize {
    fn walk(dir: &Path) -> usize {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return 0;
        };
        rd.filter_map(|e| e.ok())
            .map(|e| {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    walk(&e.path())
                } else {
                    usize::from(e.file_name().to_string_lossy().contains(".sync-conflict-"))
                }
            })
            .sum()
    }
    walk(root)
}

async fn wait_for_same_tree(
    from: &Instance,
    to: &Instance,
    report: &mut Report,
    what: &str,
) -> Result<()> {
    wait_for_same_tree_within(from, to, report, what, SYNC_TIMEOUT).await
}

async fn wait_for_same_tree_within(
    from: &Instance,
    to: &Instance,
    report: &mut Report,
    what: &str,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let (d_from, n_from) = tree_digest(&from.folder_path)?;
        let (d_to, _) = tree_digest(&to.folder_path)?;
        let idle = to
            .folder_status(FOLDER_ID)
            .await
            .map(|s| s["needFiles"] == 0)
            .unwrap_or(false);
        if d_from == d_to && idle {
            report.check(true, format!("{what} ({n_from} files identical)"));
            return Ok(());
        }
        if Instant::now() > deadline {
            report.check(
                false,
                format!("{what}: timed out after {}s", timeout.as_secs()),
            );
            bail!("sync timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
