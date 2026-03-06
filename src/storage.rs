//! Persistent storage for feed subscriptions and read state.
//!
//! Config is written to `~/.local/share/rsstui/feeds.json` as pretty-printed
//! JSON. On first run the file is absent and a default (empty) config is used.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::PathBuf,
};

use crate::error::AppError;

/// Persisted record for a single feed subscription.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeedConfig {
    /// The subscribed feed URL.
    pub url: String,

    /// Article IDs that the user has explicitly marked as read.
    /// Stored as a `HashSet` so membership tests are O(1).
    pub read_ids: HashSet<String>,
}

/// Top-level structure of the on-disk config file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageConfig {
    /// Ordered list of feed subscriptions (preserved insertion order).
    pub feeds: Vec<FeedConfig>,
}

/// Returns the absolute path of the config file, creating its parent directory
/// if necessary.
///
/// Path: `$HOME/.local/share/rsstui/feeds.json`
///
/// # Errors
///
/// Returns `AppError::Io` if the directory cannot be created.
pub fn config_path() -> Result<PathBuf, AppError> {
    // Prefer $XDG_DATA_HOME when set; otherwise fall back to ~/.local/share.
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".local/share"))
                .unwrap_or_else(|_| PathBuf::from(".local/share"))
        });

    let dir = base.join("rsstui");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("feeds.json"))
}

/// Loads the config from disk.
///
/// Returns a default (empty) `StorageConfig` when the file does not yet exist.
///
/// # Errors
///
/// * `AppError::Io`    — the file exists but cannot be read.
/// * `AppError::Serde` — the file exists but contains invalid JSON.
pub fn load_config() -> Result<StorageConfig, AppError> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(StorageConfig::default());
    }
    let data = fs::read_to_string(&path)?;
    let config = serde_json::from_str(&data)?;
    Ok(config)
}

/// Atomically saves `config` to disk, overwriting any previous file.
///
/// # Arguments
///
/// * `config` — The configuration to persist.
///
/// # Errors
///
/// * `AppError::Io`    — the file cannot be written.
/// * `AppError::Serde` — serialization fails (should not happen in practice).
pub fn save_config(config: &StorageConfig) -> Result<(), AppError> {
    let path = config_path()?;
    let data = serde_json::to_string_pretty(config)?;
    // Write to a temp file first, then rename for atomicity.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &data)?;
    fs::rename(&tmp, &path)?;
    Ok(())
}
