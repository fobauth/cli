use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::error::{Result, file};

pub fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "Fob", "fob")
}

fn config_dir() -> Result<PathBuf> {
    let dir = dirs()
        .ok_or("Couldn't find your home directory.")?
        .config_dir()
        .to_owned();
    fs::create_dir_all(&dir).map_err(|error| file(&dir, error))?;
    Ok(dir)
}

#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub org: Option<String>,
    pub host: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        config_dir()
            .ok()
            .and_then(|dir| fs::read_to_string(dir.join("config.json")).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = config_dir()?.join("config.json");
        let text = serde_json::to_string_pretty(self).expect("config serializes");
        fs::write(&path, text).map_err(|error| file(&path, error))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: u64,
    pub token_endpoint: String,
    pub revocation_endpoint: Option<String>,
}

/// Everyone's tokens, keyed by tenant origin. Held with the lock taken, so one `fob` at a
/// time reads, refreshes and saves.
pub struct TokenFile {
    _lock: File,
    path: PathBuf,
    pub entries: BTreeMap<String, Tokens>,
}

impl TokenFile {
    pub fn lock() -> Result<Self> {
        let dir = config_dir()?;
        let lock_path = dir.join("tokens.lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| file(&lock_path, error))?;
        lock.lock().map_err(|error| file(&lock_path, error))?;
        let path = dir.join("tokens.json");
        let entries = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Ok(Self {
            _lock: lock,
            path,
            entries,
        })
    }

    pub fn save(&self) -> Result<()> {
        let temporary = self.path.with_extension("json.tmp");
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let text = serde_json::to_string_pretty(&self.entries).expect("tokens serialize");
        options
            .open(&temporary)
            .and_then(|mut out| out.write_all(text.as_bytes()))
            .and_then(|()| fs::rename(&temporary, &self.path))
            .map_err(|error| file(&self.path, error))
    }
}
