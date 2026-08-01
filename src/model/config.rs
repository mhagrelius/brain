//! The one piece of state that cannot live in the vault: which vault.
//!
//! Everything else Brain knows is in the notes. This file exists only because
//! the app has to find them at startup, and it is deliberately tiny — losing it
//! costs one trip through the folder chooser, not any data.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped only if the shape below changes incompatibly.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u32,
    /// The vault folder. `None` before the first run has chosen one.
    #[serde(default)]
    pub vault: Option<PathBuf>,
    /// The note to reopen, as a vault-relative path.
    #[serde(default)]
    pub last_note: Option<String>,
    /// The window's size when it was last closed. Restoring it is the
    /// difference between an app that remembers you and one that does not.
    #[serde(default)]
    pub window_width: Option<i32>,
    #[serde(default)]
    pub window_height: Option<i32>,
    #[serde(default)]
    pub window_maximized: bool,
    /// Whether the last session was reading rather than editing. A mode you
    /// chose and the app forgot is a mode you have to choose every launch.
    #[serde(default)]
    pub reading_mode: bool,
}

fn default_version() -> u32 {
    SCHEMA_VERSION
}

/// What happened when the config was read.
///
/// Returned alongside the config rather than logged, so the UI decides whether
/// anything is worth saying — which for a missing config on first run, it is
/// not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    Loaded,
    /// No config yet. The normal first run.
    Fresh,
    /// Unreadable, so it was set aside and started over. Nothing is lost but
    /// the vault path, which the chooser will ask for again.
    Recovered {
        backup: PathBuf,
    },
}

#[derive(Debug)]
pub enum ConfigError {
    Io { path: PathBuf, source: io::Error },
    Serialize(serde_json::Error),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Serialize(source) => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// `$XDG_CONFIG_HOME/brain/config.json`, falling back to `~/.config`.
pub fn default_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("brain").join("config.json")
}

impl Config {
    pub fn load(path: &Path) -> (Self, LoadOutcome) {
        let Ok(text) = fs::read_to_string(path) else {
            return (Self::default_new(), LoadOutcome::Fresh);
        };
        match serde_json::from_str::<Self>(&text) {
            Ok(config) => (config, LoadOutcome::Loaded),
            Err(_) => {
                // Keep the unreadable file rather than deleting it: it is the
                // only copy of whatever was in there.
                let backup = path.with_extension("json.corrupt");
                let _ = fs::rename(path, &backup);
                (Self::default_new(), LoadOutcome::Recovered { backup })
            }
        }
    }

    fn default_new() -> Self {
        Self {
            version: SCHEMA_VERSION,
            ..Default::default()
        }
    }

    /// Write atomically: tmp, flush, fsync, rename.
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let text = serde_json::to_string_pretty(self).map_err(ConfigError::Serialize)?;

        let temporary = path.with_extension("json.tmp");
        let io = |source| ConfigError::Io {
            path: temporary.clone(),
            source,
        };
        let mut file = fs::File::create(&temporary).map_err(io)?;
        file.write_all(text.as_bytes()).map_err(io)?;
        file.flush().map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);

        fs::rename(&temporary, path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_config_is_a_first_run_not_an_error() {
        let directory = tempfile::tempdir().expect("temp dir");
        let (config, outcome) = Config::load(&directory.path().join("config.json"));
        assert_eq!(outcome, LoadOutcome::Fresh);
        assert_eq!(config.vault, None);
        assert_eq!(config.version, SCHEMA_VERSION);
    }

    #[test]
    fn a_saved_config_reads_back() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("brain/config.json");
        let config = Config {
            version: SCHEMA_VERSION,
            vault: Some(PathBuf::from("/home/someone/Notes")),
            last_note: Some("Rust ownership.md".into()),
            window_width: Some(1100),
            window_height: Some(760),
            window_maximized: false,
            reading_mode: true,
        };
        config.save(&path).expect("save");

        let (read, outcome) = Config::load(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(read, config);
    }

    #[test]
    fn an_unreadable_config_is_set_aside_not_deleted() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("config.json");
        fs::write(&path, "{not json").expect("write");

        let (config, outcome) = Config::load(&path);
        assert_eq!(config.vault, None);
        match outcome {
            LoadOutcome::Recovered { backup } => {
                assert_eq!(fs::read_to_string(backup).expect("backup"), "{not json");
            }
            other => panic!("expected recovery, got {other:?}"),
        }
    }

    #[test]
    fn an_older_config_missing_fields_still_loads() {
        // Every field is defaulted, so a config written by an earlier build
        // opens rather than being treated as corrupt.
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("config.json");
        fs::write(&path, r#"{"vault":"/notes"}"#).expect("write");

        let (config, outcome) = Config::load(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(config.vault, Some(PathBuf::from("/notes")));
        assert_eq!(config.version, SCHEMA_VERSION);
        // Absent window geometry is not a corrupt config; it is a config
        // written before the app remembered any.
        assert_eq!(config.window_width, None);
        assert!(!config.window_maximized);
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let directory = tempfile::tempdir().expect("temp dir");
        let path = directory.path().join("config.json");
        Config::default_new().save(&path).expect("save");

        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .expect("read dir")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn the_default_path_follows_xdg() {
        // Guarded so the test does not depend on the developer's environment.
        let previous = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/xdg");
        assert_eq!(default_path(), PathBuf::from("/tmp/xdg/brain/config.json"));
        match previous {
            Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}
