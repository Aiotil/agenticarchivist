//! Signed, append-only metadata log.
//!
//! Each device appends only to its own folder,
//! `_agenticarchivist/log/<device-id>/NNNNNN.jsonl`, so Syncthing never sees
//! two devices write the same file. Every device reads all logs, verifies
//! signatures, checks roles, and materialises the result into SQLite.

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::identity::{DeviceIdentity, verify_with_cert};

pub const LOG_DIR: &str = "_agenticarchivist/log";
const ENTRIES_PER_SEGMENT: u64 = 1000;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hlc {
    pub ms: u64,
    pub counter: u32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Editor,
    Admin,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Op {
    /// First entry in every device log: the certificate that signs the rest.
    DeviceHello {
        cert: String,
    },
    Genesis {
        collection: String,
        folder: String,
    },
    AddMember {
        device: String,
        role: Role,
    },
    SetField {
        entity: String,
        field: String,
        value: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Body {
    pub v: u8,
    pub device: String,
    pub seq: u64,
    pub hlc: Hlc,
    /// Latest membership entry this device had seen when writing.
    pub membership_head: Option<String>,
    pub op: Op,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    #[serde(flatten)]
    pub body: Body,
    pub sig: String,
}

impl Entry {
    pub fn id(&self) -> String {
        crate::to_hex(Sha256::digest(serde_json::to_vec(&self.body).unwrap()))
    }
}

pub struct LogWriter {
    identity_key: ed25519_dalek::SigningKey,
    cert_der: Vec<u8>,
    device_id: String,
    dir: PathBuf,
    next_seq: u64,
    last_hlc: Hlc,
}

impl LogWriter {
    pub fn open(folder_root: &Path, identity: &DeviceIdentity) -> Result<Self> {
        let dir = folder_root.join(LOG_DIR).join(&identity.device_id);
        std::fs::create_dir_all(&dir)?;
        let existing = read_device_log(&dir)?;
        let mut writer = Self {
            identity_key: identity.signing_key.clone(),
            cert_der: identity.cert_der.clone(),
            device_id: identity.device_id.clone(),
            dir,
            next_seq: existing.last().map_or(1, |e| e.body.seq + 1),
            last_hlc: existing
                .last()
                .map_or(Hlc { ms: 0, counter: 0 }, |e| e.body.hlc),
        };
        if existing.is_empty() {
            let cert = STANDARD.encode(&writer.cert_der);
            writer.append(Op::DeviceHello { cert }, None)?;
        }
        Ok(writer)
    }

    /// Move the clock past everything seen from other devices.
    pub fn observe(&mut self, seen: Hlc) {
        if seen > self.last_hlc {
            self.last_hlc = seen;
        }
    }

    pub fn append(&mut self, op: Op, membership_head: Option<String>) -> Result<Entry> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        let hlc = if now > self.last_hlc.ms {
            Hlc {
                ms: now,
                counter: 0,
            }
        } else {
            Hlc {
                ms: self.last_hlc.ms,
                counter: self.last_hlc.counter + 1,
            }
        };
        let body = Body {
            v: 1,
            device: self.device_id.clone(),
            seq: self.next_seq,
            hlc,
            membership_head,
            op,
        };
        let bytes = serde_json::to_vec(&body)?;
        let sig =
            STANDARD.encode(ed25519_dalek::Signer::sign(&self.identity_key, &bytes).to_bytes());
        let entry = Entry { body, sig };

        let segment = (self.next_seq - 1) / ENTRIES_PER_SEGMENT + 1;
        let path = self.dir.join(format!("{segment:06}.jsonl"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        file.write_all(&line)?;
        file.sync_all()?;

        self.next_seq += 1;
        self.last_hlc = hlc;
        Ok(entry)
    }
}

fn read_device_log(dir: &Path) -> Result<Vec<Entry>> {
    let mut segments: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect(),
        Err(_) => return Ok(vec![]),
    };
    segments.sort();
    let mut entries = vec![];
    for seg in segments {
        let text = std::fs::read_to_string(&seg)?;
        for line in text.lines() {
            // A torn final line can only happen on the writer after a crash.
            if let Ok(entry) = serde_json::from_str::<Entry>(line) {
                entries.push(entry);
            }
        }
    }
    Ok(entries)
}

/// Read and verify every device log in a collection folder.
pub fn load_all(folder_root: &Path) -> Result<(Vec<Entry>, Vec<String>)> {
    let mut verified = vec![];
    let mut problems = vec![];
    let log_root = folder_root.join(LOG_DIR);
    let Ok(dirs) = std::fs::read_dir(&log_root) else {
        return Ok((verified, problems));
    };
    for dir in dirs.filter_map(|d| d.ok()) {
        let dir_name = dir.file_name().to_string_lossy().to_string();
        match verify_device_log(&dir.path(), &dir_name) {
            Ok(entries) => verified.extend(entries),
            Err(e) => problems.push(format!("log {dir_name}: {e:#}")),
        }
    }
    Ok((verified, problems))
}

fn verify_device_log(dir: &Path, dir_name: &str) -> Result<Vec<Entry>> {
    let entries = read_device_log(dir)?;
    let Some(first) = entries.first() else {
        return Ok(vec![]);
    };
    let Op::DeviceHello { cert } = &first.body.op else {
        bail!("first entry is not device-hello");
    };
    let cert_der = STANDARD.decode(cert)?;
    let cert_device = crate::deviceid::from_cert_der(&cert_der);
    if cert_device != dir_name {
        bail!("certificate belongs to {cert_device}, not this folder");
    }
    let mut last_seq = 0;
    for entry in &entries {
        if entry.body.device != dir_name {
            bail!(
                "entry {} claims device {}",
                entry.body.seq,
                entry.body.device
            );
        }
        if entry.body.seq <= last_seq {
            bail!("sequence out of order at {}", entry.body.seq);
        }
        last_seq = entry.body.seq;
        let bytes = serde_json::to_vec(&entry.body)?;
        verify_with_cert(&cert_der, &bytes, &entry.sig)
            .with_context(|| format!("bad signature on entry {}", entry.body.seq))?;
    }
    Ok(entries)
}

#[derive(Debug, Default)]
pub struct State {
    pub collection: Option<String>,
    pub members: BTreeMap<String, Role>,
    pub fields: BTreeMap<(String, String), (String, Hlc, String)>,
    pub latest_membership: Option<String>,
    pub max_hlc: Option<Hlc>,
    pub rejected: Vec<(String, String, String)>,
}

impl State {
    /// Stable fingerprint for comparing results across devices.
    pub fn digest(&self) -> String {
        let text = format!("{:?}|{:?}|{:?}", self.collection, self.members, self.fields);
        crate::to_hex(&Sha256::digest(text.as_bytes())[..8])
    }
}

/// Apply verified entries in a deterministic order and enforce roles.
pub fn materialise(mut entries: Vec<Entry>) -> State {
    entries.sort_by(|a, b| {
        (a.body.hlc, &a.body.device, a.body.seq).cmp(&(b.body.hlc, &b.body.device, b.body.seq))
    });
    let mut state = State::default();
    // Membership after each accepted-or-rejected membership entry, by entry id.
    let mut membership_at: BTreeMap<String, BTreeMap<String, Role>> = BTreeMap::new();

    for entry in &entries {
        state.max_hlc = state.max_hlc.max(Some(entry.body.hlc));
        let id = entry.id();
        let device = entry.body.device.clone();
        let members_then = entry
            .body
            .membership_head
            .as_ref()
            .and_then(|h| membership_at.get(h))
            .cloned()
            .unwrap_or_default();
        let role = members_then.get(&device).copied();

        match &entry.body.op {
            Op::DeviceHello { .. } => {}
            Op::Genesis { collection, .. } => {
                if state.collection.is_none() {
                    state.collection = Some(collection.clone());
                    state.members.insert(device.clone(), Role::Admin);
                    state.latest_membership = Some(id.clone());
                } else {
                    state
                        .rejected
                        .push((id.clone(), device, "second genesis".into()));
                }
                membership_at.insert(id, state.members.clone());
            }
            Op::AddMember {
                device: new_device,
                role: new_role,
            } => {
                if role == Some(Role::Admin) {
                    state.members.insert(new_device.clone(), *new_role);
                    state.latest_membership = Some(id.clone());
                } else {
                    state.rejected.push((
                        id.clone(),
                        device,
                        format!("{role:?} cannot add members"),
                    ));
                }
                membership_at.insert(id, state.members.clone());
            }
            Op::SetField {
                entity,
                field,
                value,
            } => {
                if matches!(role, Some(Role::Editor | Role::Admin)) {
                    // Entries are applied in HLC order, so the last write wins.
                    state.fields.insert(
                        (entity.clone(), field.clone()),
                        (value.clone(), entry.body.hlc, device),
                    );
                } else {
                    state
                        .rejected
                        .push((id, device, format!("{role:?} cannot edit")));
                }
            }
        }
    }
    state
}

/// Store the merged state in SQLite, as the library cache would.
pub fn write_sqlite(state: &State, db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(db_path);
    let conn = rusqlite::Connection::open(db_path)?;
    conn.execute_batch(
        "CREATE TABLE members (device TEXT PRIMARY KEY, role TEXT NOT NULL);
         CREATE TABLE fields (entity TEXT, field TEXT, value TEXT, hlc_ms INTEGER, hlc_counter INTEGER, device TEXT,
                              PRIMARY KEY (entity, field));
         CREATE TABLE rejected (entry_id TEXT, device TEXT, reason TEXT);",
    )?;
    for (device, role) in &state.members {
        conn.execute(
            "INSERT INTO members VALUES (?1, ?2)",
            (device, format!("{role:?}")),
        )?;
    }
    for ((entity, field), (value, hlc, device)) in &state.fields {
        conn.execute(
            "INSERT INTO fields VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (entity, field, value, hlc.ms as i64, hlc.counter, device),
        )?;
    }
    for (id, device, reason) in &state.rejected {
        conn.execute(
            "INSERT INTO rejected VALUES (?1, ?2, ?3)",
            (id, device, reason),
        )?;
    }
    Ok(())
}

pub fn membership_head(state: &State) -> Result<String> {
    state
        .latest_membership
        .clone()
        .ok_or_else(|| anyhow!("collection has no genesis yet"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oplog-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn roles_are_enforced_and_last_writer_wins() {
        let root = temp_dir("roles");
        let admin = DeviceIdentity::from_seat_secret(&[1; 32]).unwrap();
        let editor = DeviceIdentity::from_seat_secret(&[2; 32]).unwrap();
        let viewer = DeviceIdentity::from_seat_secret(&[3; 32]).unwrap();

        let mut a = LogWriter::open(&root, &admin).unwrap();
        let genesis = a
            .append(
                Op::Genesis {
                    collection: "c1".into(),
                    folder: "f1".into(),
                },
                None,
            )
            .unwrap();
        let add_e = a
            .append(
                Op::AddMember {
                    device: editor.device_id.clone(),
                    role: Role::Editor,
                },
                Some(genesis.id()),
            )
            .unwrap();
        let add_v = a
            .append(
                Op::AddMember {
                    device: viewer.device_id.clone(),
                    role: Role::Viewer,
                },
                Some(add_e.id()),
            )
            .unwrap();

        let mut e = LogWriter::open(&root, &editor).unwrap();
        e.observe(add_v.body.hlc);
        e.append(
            Op::SetField {
                entity: "w1".into(),
                field: "title".into(),
                value: "editor".into(),
            },
            Some(add_v.id()),
        )
        .unwrap();
        let mut v = LogWriter::open(&root, &viewer).unwrap();
        v.observe(add_v.body.hlc);
        v.append(
            Op::SetField {
                entity: "w1".into(),
                field: "title".into(),
                value: "viewer".into(),
            },
            Some(add_v.id()),
        )
        .unwrap();
        v.append(
            Op::AddMember {
                device: "X".into(),
                role: Role::Admin,
            },
            Some(add_v.id()),
        )
        .unwrap();

        let (entries, problems) = load_all(&root).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        let state = materialise(entries);
        assert_eq!(state.members.len(), 3);
        assert_eq!(state.fields[&("w1".into(), "title".into())].0, "editor");
        assert_eq!(state.rejected.len(), 2);
    }

    #[test]
    fn tampered_log_is_rejected() {
        let root = temp_dir("tamper");
        let admin = DeviceIdentity::from_seat_secret(&[4; 32]).unwrap();
        let mut a = LogWriter::open(&root, &admin).unwrap();
        a.append(
            Op::Genesis {
                collection: "c1".into(),
                folder: "f1".into(),
            },
            None,
        )
        .unwrap();
        let seg = root
            .join(LOG_DIR)
            .join(&admin.device_id)
            .join("000001.jsonl");
        let text = std::fs::read_to_string(&seg).unwrap().replace("c1", "c2");
        std::fs::write(&seg, text).unwrap();
        let (entries, problems) = load_all(&root).unwrap();
        assert!(entries.is_empty());
        assert_eq!(problems.len(), 1);
    }
}
