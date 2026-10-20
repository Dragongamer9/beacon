//! Settings on disk (`~/.config/beacon/config.json`) and where Beacon keeps
//! its prefixes, logs and mods (`~/.local/share/beacon/`).

use crate::games::GameId;
use crate::mods::ModEntry;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub fn parse(s: &str) -> Option<Resolution> {
        let (w, h) = s.trim().split_once(|c| c == 'x' || c == 'X' || c == ' ')?;
        let (width, height) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
        (width >= 640 && height >= 480).then_some(Resolution { width, height })
    }
    pub fn ini_value(&self) -> String {
        format!("{} {}", self.width, self.height)
    }
}

impl std::fmt::Display for Resolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameConfig {
    /// Folder holding the game's exe.
    pub path: Option<PathBuf>,
    /// Runner id; falls back to the global default, then the best found.
    pub runner: Option<String>,
    /// The CD key from the player's own copy (stored in the `ergc` key).
    pub cd_key: Option<String>,
    pub resolution: Option<Resolution>,
    pub windowed: bool,
    /// Extra game arguments, shell-style.
    pub extra_args: String,
    /// Extra environment variables for the runner (e.g. DXVK_HUD=fps).
    pub env: BTreeMap<String, String>,
    /// Run inside a Wine virtual desktop (`None` = the game's default).
    /// The desktop size is `resolution`, or 1280x1024.
    pub virtual_desktop: Option<bool>,
    /// For Steam copies: hand the launch to Steam instead of Beacon's prefix.
    pub via_steam: bool,
    /// Chosen edition (Other Games); `None` = community edition if installed.
    pub edition: Option<String>,
    /// Community-version setup steps the player has completed.
    pub done_steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub default_runner: Option<String>,
    /// Value for the registry's Language entry.
    pub language: String,
    /// Extra folders to scan besides the defaults.
    pub scan_roots: Vec<PathBuf>,
    pub games: BTreeMap<GameId, GameConfig>,
    /// Mods added from outside the mods folder.
    pub linked_mods: Vec<ModEntry>,
    /// Last mod picked per game (`None`/missing = vanilla).
    pub selected_mod: BTreeMap<GameId, String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            default_runner: None,
            language: "english".into(),
            scan_roots: vec![],
            games: BTreeMap::new(),
            linked_mods: vec![],
            selected_mod: BTreeMap::new(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).with_context(|| format!("reading {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paths {
    pub home: PathBuf,
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    /// XDG locations, overridable with BEACON_CONFIG / BEACON_DATA_DIR.
    pub fn from_env() -> Result<Paths> {
        let home = dirs::home_dir().context("no home directory")?;
        let config_file = std::env::var_os("BEACON_CONFIG")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::config_dir().unwrap_or_else(|| home.join(".config")).join("beacon/config.json"));
        let data_dir = std::env::var_os("BEACON_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::data_dir().unwrap_or_else(|| home.join(".local/share")).join("beacon"));
        Ok(Paths { home, config_file, data_dir })
    }

    /// Everything under one folder (tests, portable installs).
    pub fn under(root: &Path, home: &Path) -> Paths {
        Paths { home: home.to_path_buf(), config_file: root.join("config.json"), data_dir: root.join("data") }
    }

    pub fn prefix_root(&self, g: GameId) -> PathBuf {
        self.data_dir.join("prefixes").join(g.slug())
    }
    pub fn reg_file(&self, g: GameId) -> PathBuf {
        self.data_dir.join("registry").join(format!("{}.reg", g.slug()))
    }
    pub fn log_file(&self, g: GameId) -> PathBuf {
        self.data_dir.join("logs").join(format!("{}.log", g.slug()))
    }
    pub fn mods_dir(&self) -> PathBuf {
        self.data_dir.join("mods")
    }
}

/// Split a string into arguments, honouring single and double quotes.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_parse() {
        assert_eq!(Resolution::parse("1920x1080"), Some(Resolution { width: 1920, height: 1080 }));
        assert_eq!(Resolution::parse("1280 800").unwrap().ini_value(), "1280 800");
        assert_eq!(Resolution::parse("10x10"), None);
        assert_eq!(Resolution::parse("wide"), None);
    }

    #[test]
    fn args_split() {
        assert_eq!(split_args(r#"-mod "C:\My Mods\x" -noshellmap ''"#), vec!["-mod", "C:\\My Mods\\x", "-noshellmap", ""]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn config_roundtrip() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("c/config.json");
        let mut c = Config::default();
        c.games.entry(GameId::Rotwk).or_default().resolution = Resolution::parse("2560x1440");
        c.selected_mod.insert(GameId::Rotwk, "rotwk:edain".into());
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap(), c);
        assert_eq!(Config::load(&t.path().join("missing.json")).unwrap(), Config::default());
    }
}
