//! Invite links: `https://aiotil.github.io/agenticarchivist/#v1.<payload>`.
//!
//! The payload sits after `#`, so browsers never send it to the web server.

use anyhow::{Result, anyhow, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::oplog::Role;

pub const JOIN_PAGE: &str = "https://aiotil.github.io/agenticarchivist/";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Invite {
    pub collection: String,
    pub folder: String,
    pub name: String,
    pub role: Role,
    /// Unix seconds.
    pub expires: u64,
    /// 32-byte seat secret, base64url.
    pub seat: String,
    /// Device IDs of current members, so the newcomer can reach any of them.
    pub members: Vec<String>,
    /// Spike only: fixed local addresses, standing in for Syncthing discovery.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub spike_addresses: BTreeMap<String, String>,
}

impl Invite {
    pub fn seat_secret(&self) -> Result<[u8; 32]> {
        URL_SAFE_NO_PAD
            .decode(&self.seat)?
            .try_into()
            .map_err(|_| anyhow!("seat secret is not 32 bytes"))
    }

    pub fn to_link(&self) -> Result<String> {
        let json = serde_json::to_vec(self)?;
        Ok(format!("{JOIN_PAGE}#v1.{}", URL_SAFE_NO_PAD.encode(json)))
    }

    pub fn from_link(link: &str) -> Result<Self> {
        let fragment = link.split_once('#').map(|(_, f)| f).unwrap_or(link);
        let Some(payload) = fragment.strip_prefix("v1.") else {
            bail!("not a v1 invite");
        };
        let invite: Invite = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        if invite.expires < now {
            bail!("invite expired");
        }
        Ok(invite)
    }
}
