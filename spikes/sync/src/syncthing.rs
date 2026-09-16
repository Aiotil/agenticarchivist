//! Supervising a bundled Syncthing process and driving its REST API.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::{Child, Command};

use crate::identity::DeviceIdentity;

pub struct Instance {
    pub name: String,
    pub home: PathBuf,
    pub folder_path: PathBuf,
    pub gui_port: u16,
    pub listen_port: u16,
    api_key: String,
    binary: PathBuf,
    child: Option<Child>,
    http: reqwest::Client,
}

impl Instance {
    pub fn new(name: &str, run_dir: &Path, binary: &Path, gui_port: u16, listen_port: u16) -> Self {
        Self {
            name: name.to_string(),
            home: run_dir.join(name).join("syncthing-home"),
            folder_path: run_dir.join(name).join("Grandpa Saul Letters"),
            gui_port,
            listen_port,
            api_key: format!("spike-{name}-{}", crate::to_hex(rand::random::<[u8; 8]>())),
            binary: binary.to_path_buf(),
            child: None,
            http: reqwest::Client::new(),
        }
    }

    pub fn listen_address(&self) -> String {
        format!("tcp://127.0.0.1:{}", self.listen_port)
    }

    /// Create config and keys. With `identity`, Syncthing keeps that certificate.
    pub fn prepare(&self, identity: Option<&DeviceIdentity>) -> Result<()> {
        std::fs::create_dir_all(&self.home)?;
        std::fs::create_dir_all(&self.folder_path)?;
        if let Some(id) = identity {
            id.write_to_home(&self.home)?;
        }
        let out = std::process::Command::new(&self.binary)
            .args(["generate", "--no-port-probing"])
            .arg(format!("--home={}", self.home.display()))
            .output()
            .context("running syncthing generate")?;
        if !out.status.success() {
            bail!(
                "syncthing generate failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        // Keep the spike off the public network: no global discovery, relays,
        // NAT traversal, usage reporting, crash reports, or upgrades.
        let config_path = self.home.join("config.xml");
        let mut config = std::fs::read_to_string(&config_path)?;
        for (from, to) in [
            (
                "<globalAnnounceEnabled>true</globalAnnounceEnabled>",
                "<globalAnnounceEnabled>false</globalAnnounceEnabled>".to_string(),
            ),
            (
                "<localAnnounceEnabled>true</localAnnounceEnabled>",
                "<localAnnounceEnabled>false</localAnnounceEnabled>".to_string(),
            ),
            (
                "<relaysEnabled>true</relaysEnabled>",
                "<relaysEnabled>false</relaysEnabled>".to_string(),
            ),
            (
                "<natEnabled>true</natEnabled>",
                "<natEnabled>false</natEnabled>".to_string(),
            ),
            (
                "<urAccepted>0</urAccepted>",
                "<urAccepted>-1</urAccepted>".to_string(),
            ),
            (
                "<autoUpgradeIntervalH>12</autoUpgradeIntervalH>",
                "<autoUpgradeIntervalH>0</autoUpgradeIntervalH>".to_string(),
            ),
            (
                "<crashReportingEnabled>true</crashReportingEnabled>",
                "<crashReportingEnabled>false</crashReportingEnabled>".to_string(),
            ),
            (
                "<startBrowser>true</startBrowser>",
                "<startBrowser>false</startBrowser>".to_string(),
            ),
            (
                "<listenAddress>default</listenAddress>",
                format!("<listenAddress>{}</listenAddress>", self.listen_address()),
            ),
        ] {
            if !config.contains(from) {
                bail!("{}: config.xml has no {from}", self.name);
            }
            config = config.replace(from, &to);
        }
        std::fs::write(config_path, config)?;
        Ok(())
    }

    pub async fn start(&mut self) -> Result<()> {
        let log = std::fs::File::create(self.home.join("syncthing.log"))?;
        let child = Command::new(&self.binary)
            .arg("serve")
            .arg(format!("--home={}", self.home.display()))
            .arg(format!("--gui-address=http://127.0.0.1:{}", self.gui_port))
            .arg(format!("--gui-apikey={}", self.api_key))
            .args([
                "--no-browser",
                "--no-restart",
                "--no-upgrade",
                "--no-port-probing",
            ])
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .kill_on_drop(true)
            .spawn()
            .context("starting syncthing")?;
        self.child = Some(child);

        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if self.get("/rest/system/ping").await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        bail!(
            "{} did not start; see {}",
            self.name,
            self.home.join("syncthing.log").display()
        )
    }

    pub async fn stop(&mut self) -> Result<()> {
        let _ = self.post("/rest/system/shutdown", json!({})).await;
        if let Some(mut child) = self.child.take()
            && tokio::time::timeout(Duration::from_secs(15), child.wait())
                .await
                .is_err()
        {
            child.kill().await?;
        }
        Ok(())
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.gui_port, path)
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(self.url(path))
            .header("X-API-Key", &self.api_key)
            .send()
            .await?;
        let resp = resp.error_for_status()?;
        let text = resp.text().await?;
        Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    async fn put(&self, path: &str, body: Value) -> Result<()> {
        let resp = self
            .http
            .put(self.url(path))
            .header("X-API-Key", &self.api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            bail!(
                "PUT {path}: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    async fn post(&self, path: &str, body: Value) -> Result<()> {
        self.http
            .post(self.url(path))
            .header("X-API-Key", &self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }

    pub async fn my_id(&self) -> Result<String> {
        let status = self.get("/rest/system/status").await?;
        status["myID"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("no myID"))
    }

    pub async fn ensure_device(&self, device_id: &str, name: &str, address: &str) -> Result<()> {
        let mut device = self.get("/rest/config/defaults/device").await?;
        device["deviceID"] = json!(device_id);
        device["name"] = json!(name);
        device["addresses"] = json!([address]);
        self.put(&format!("/rest/config/devices/{device_id}"), device)
            .await
    }

    pub async fn ensure_folder(
        &self,
        folder_id: &str,
        label: &str,
        devices: &[String],
        receive_only: bool,
    ) -> Result<()> {
        let mut folder = self.get("/rest/config/defaults/folder").await?;
        folder["id"] = json!(folder_id);
        folder["label"] = json!(label);
        folder["path"] = json!(self.folder_path.display().to_string());
        folder["type"] = json!(if receive_only {
            "receiveonly"
        } else {
            "sendreceive"
        });
        folder["devices"] = json!(
            devices
                .iter()
                .map(|d| json!({ "deviceID": d }))
                .collect::<Vec<_>>()
        );
        folder["rescanIntervalS"] = json!(5);
        folder["fsWatcherEnabled"] = json!(true);
        folder["fsWatcherDelayS"] = json!(1);
        self.put(&format!("/rest/config/folders/{folder_id}"), folder)
            .await
    }

    pub async fn scan(&self, folder_id: &str) -> Result<()> {
        self.post(&format!("/rest/db/scan?folder={folder_id}"), json!({}))
            .await
    }

    pub async fn folder_status(&self, folder_id: &str) -> Result<Value> {
        self.get(&format!("/rest/db/status?folder={folder_id}"))
            .await
    }

    pub async fn connected_devices(&self) -> Result<Vec<String>> {
        let conns = self.get("/rest/system/connections").await?;
        Ok(conns["connections"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(_, v)| v["connected"] == json!(true))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default())
    }
}
