//! Persistent settings: the API key and an optional app `clientId`.
//!
//! Stored as JSON in `$XDG_CONFIG_HOME/mikai/config.json`, readable only by the owner.

use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// `clientId` of the registered mikai-cli app (https://mikaicli.deitylamb.dev).
/// Not a secret. Forks can override it at build time with `MIKAI_CLIENT_ID=mkc_… cargo build`.
pub const BUILTIN_CLIENT_ID: &str = match option_env!("MIKAI_CLIENT_ID") {
    Some(id) => id,
    None => "mkc_51665457f597e95679c7a10c",
};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        Ok(dirs::config_dir()
            .context("не знайдено теку налаштувань")?
            .join("mikai")
            .join("config.json"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("зіпсований {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("не вдалося прочитати {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        fs::create_dir_all(path.parent().unwrap())?;
        let text = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            use std::io::Write;
            opts.open(&tmp)?.write_all(text.as_bytes())?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn client_id(&self) -> Option<String> {
        std::env::var("MIKAI_CLIENT_ID")
            .ok()
            .or_else(|| self.client_id.clone())
            .or_else(|| Some(BUILTIN_CLIENT_ID.to_string()))
            .filter(|s| !s.is_empty())
    }
}
