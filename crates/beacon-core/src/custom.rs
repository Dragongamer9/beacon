//! Games added by hand: any Windows program or installer, each in a Wine prefix of its own, with the runner and
//! options chosen per game. This is Faugus Launcher's feature set (https://github.com/Faugus/faugus-launcher, MIT)
//! rebuilt in Beacon: icon taken from the .exe, launch arguments and working folder, GAMEID/STORE for umu's
//! protonfixes, environment variables, DXVK/VKD3D on or off, MangoHud, GameMode, gamescope, Proton options, the
//! Wine tools for the prefix, an install flow (run the installer in a new prefix, then pick the game's program),
//! import/export, play time and last played, and importing an existing Faugus Launcher library.
//! See THIRD-PARTY-NOTICES.md.
//!
//! The list lives in `custom-games.json` next to Beacon's config.json, and every change reads it again first and
//! writes it at once, so Beacon's window and the `beacon` tool (Hearth) never undo each other's changes.
//! Play time goes to `playtime.json` ([`crate::playtime`]) under the key `custom:<id>`.
//!
//! In Beacon these games are listed under Other Games; Hearth shows them in its library through
//! `beacon custom list --json` and starts them with `beacon custom launch <id> --wait` (docs in the README).

use crate::playtime::{self, PlayStats};
use crate::runner::{self, Command, Runner, RunnerKind};
use crate::{Beacon, Issue, Level};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomGame {
    /// Lowercase letters, digits and dashes; made from the title.
    pub id: String,
    pub title: String,
    /// The program to start (a Linux path). None while it's being installed (pick it afterwards).
    pub exe: Option<PathBuf>,
    /// Arguments for the program, shell-style.
    pub args: String,
    /// Working folder (default: the program's folder).
    pub cwd: Option<PathBuf>,
    /// The Wine prefix (WINEPREFIX). None: Beacon's own, `<data>/prefixes/custom/<id>/pfx`.
    pub prefix: Option<PathBuf>,
    /// Runner id (`beacon runners`); None: Beacon's default, then the best found.
    pub runner: Option<String>,
    /// PNG icon (taken from the exe), cover (portrait) and hero (wide) images, in `<data>/art/<id>/`.
    pub icon: Option<PathBuf>,
    pub cover: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    /// DXVK / VKD3D-Proton off (Proton: PROTON_USE_WINED3D / PROTON_NO_D3D12; Wine: not copied into the prefix).
    pub no_dxvk: bool,
    pub no_vkd3d: bool,
    pub mangohud: bool,
    pub gamemode: bool,
    pub gamescope: bool,
    /// gamescope's own options, e.g. "-W 1920 -H 1080 -f".
    pub gamescope_args: String,
    /// Proton options that are on ([`FLAGS`] keys).
    pub flags: Vec<String>,
    /// umu's GAMEID (e.g. umu-12345 or a Steam app id) and STORE (steam, egs, gog, ubisoft, …) for protonfixes.
    pub umu_id: Option<String>,
    pub store: Option<String>,
    /// Unix time it was added.
    pub added: u64,
    /// The installer it was installed with (the install flow).
    pub installer: Option<PathBuf>,
    /// Where it came from: None (added in Beacon), "faugus" (Faugus Launcher), "import".
    pub source: Option<String>,
}

/// Proton options: (key, variable, value, label).
pub const FLAGS: &[(&str, &str, &str, &str)] = &[
    ("nvapi", "PROTON_ENABLE_NVAPI", "1", "NVIDIA NVAPI / DLSS"),
    ("wayland", "PROTON_ENABLE_WAYLAND", "1", "Native Wayland (Proton 10 and newer)"),
    ("hdr", "PROTON_ENABLE_HDR", "1", "HDR (with Wayland)"),
    ("wow64", "PROTON_USE_WOW64", "1", "WoW64 (no 32-bit libraries)"),
    ("fsr", "WINE_FULLSCREEN_FSR", "1", "FSR upscaling for lower resolutions (GE / UMU)"),
    ("sdl", "PROTON_PREFER_SDL", "1", "Controllers through SDL"),
    ("no-esync", "PROTON_NO_ESYNC", "1", "Turn off esync"),
    ("no-fsync", "PROTON_NO_FSYNC", "1", "Turn off fsync"),
    ("hide-nvidia", "PROTON_HIDE_NVIDIA_GPU", "1", "Hide an NVIDIA GPU from the game"),
    ("large-address", "PROTON_FORCE_LARGE_ADDRESS_AWARE", "1", "Large address aware (old 32-bit games)"),
    ("log", "PROTON_LOG", "1", "Proton log (in your home folder)"),
];

/// umu's STORE values.
pub const STORES: &[&str] = &["none", "steam", "egs", "gog", "amazon", "battlenet", "ea", "humble", "itchio", "ubisoft", "zoomplatform"];

/// The Wine tools for a game's prefix.
pub const WINE_TOOLS: &[(&str, &str)] = &[
    ("winecfg", "Wine configuration"),
    ("winetricks", "Winetricks"),
    ("regedit", "Registry editor"),
    ("cmd", "Command prompt"),
    ("taskmgr", "Task manager"),
    ("explorer", "File explorer"),
    ("uninstaller", "Uninstall programs"),
    ("control", "Control panel"),
    ("kill", "Stop everything in the prefix"),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Library {
    version: u32,
    games: Vec<CustomGame>,
}

pub fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && !s.starts_with('-') && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// An id from a title: "The Witcher 3: Wild Hunt" → "the-witcher-3-wild-hunt".
pub fn slug(title: &str) -> String {
    let mut s = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s: String = s.trim_end_matches('-').chars().take(48).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() {
        "game".into()
    } else {
        s
    }
}

fn unique_id(base: &str, taken: &[CustomGame]) -> String {
    if !taken.iter().any(|g| g.id == base) {
        return base.to_string();
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken.iter().any(|g| &g.id == id)).unwrap()
}

/// A title from a program's path: its name, or its folder's when the name says nothing ("game.exe").
pub fn title_from_exe(exe: &Path) -> String {
    let stem = exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let generic = ["game", "launcher", "start", "play", "run", "main", "client", "app", "win64", "win32", "shipping", "bin"];
    let lower = stem.to_ascii_lowercase();
    let name = if generic.iter().any(|g| lower == *g || lower.ends_with("-win64-shipping")) {
        exe.ancestors()
            .skip(1)
            .filter_map(|a| a.file_name().map(|n| n.to_string_lossy().to_string()))
            .find(|n| !["bin", "binaries", "win64", "win32", "x64", "x86", "game"].contains(&n.to_ascii_lowercase().as_str()))
            .unwrap_or(stem)
    } else {
        stem
    };
    name.replace(['_', '.'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a program looks like an installer (offer the install flow for it).
pub fn looks_like_installer(p: &Path) -> bool {
    let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    n.ends_with(".msi") || n.starts_with("setup") || n.contains("install") || n.starts_with("gog_") || n.contains("_setup")
}

fn expand_home(s: &str, home: &Path) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else if s == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(s)
    }
}

/// What `beacon custom list` reports for each game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomStatus {
    #[serde(flatten)]
    pub game: CustomGame,
    /// "custom:<id>": its key in playtime.json, and Hearth's id after "beacon:".
    pub key: String,
    /// Shelf section: always "other".
    pub family: String,
    /// The program is there.
    pub installed: bool,
    /// No errors (warnings are fine).
    pub ready: bool,
    pub issues: Vec<Issue>,
    /// The runner a launch uses (the choice, else the default, else the best found).
    pub resolved_runner: Option<Runner>,
    pub wineprefix: PathBuf,
    pub prefix_ready: bool,
    /// Beacon made the prefix (removing the game can delete it).
    pub own_prefix: bool,
    pub play_seconds: u64,
    pub last_played: u64,
    pub sessions: u64,
    pub log_file: PathBuf,
    pub has_log: bool,
    pub installer_like: bool,
}

/// One field change, for the CLI (`beacon custom set <id> <key> <value>`) and Beacon's window.
pub const SET_KEYS: &[&str] = &[
    "title", "exe", "args", "cwd", "prefix", "runner", "env", "dxvk", "vkd3d", "mangohud", "gamemode", "gamescope", "gamescope-args", "flag", "umu-id", "store", "icon", "cover", "hero",
];

fn on_off(v: &str) -> Result<bool> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => bail!("expected on or off"),
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub added: Vec<String>,
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RemoveReport {
    pub id: String,
    pub deleted_prefix: Option<PathBuf>,
    /// Why the prefix was kept, when it was asked to go.
    pub kept_prefix: Option<String>,
}

impl Beacon {
    // ---- the list ---------------------------------------------------------------------------------

    pub fn custom_file(&self) -> PathBuf {
        self.paths.config_file.parent().map(|d| d.join("custom-games.json")).unwrap_or_else(|| PathBuf::from("custom-games.json"))
    }
    pub fn playtime_file(&self) -> PathBuf {
        self.paths.data_dir.join("playtime.json")
    }
    pub fn custom_art_dir(&self, id: &str) -> PathBuf {
        self.paths.data_dir.join("art").join(id)
    }
    pub fn custom_default_prefix(&self, id: &str) -> PathBuf {
        self.paths.data_dir.join("prefixes/custom").join(id).join("pfx")
    }
    pub fn custom_log(&self, id: &str) -> PathBuf {
        self.paths.data_dir.join("logs").join(format!("custom-{id}.log"))
    }

    fn custom_load(&self) -> Result<Library> {
        let f = self.custom_file();
        match std::fs::read_to_string(&f) {
            Ok(s) => serde_json::from_str(&s).with_context(|| format!("reading {}", f.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Library { version: 1, games: vec![] }),
            Err(e) => Err(e).with_context(|| format!("reading {}", f.display())),
        }
    }

    fn custom_save(&self, lib: &Library) -> Result<()> {
        let f = self.custom_file();
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = f.with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&tmp, serde_json::to_string_pretty(&Library { version: 1, games: lib.games.clone() })?)?;
        std::fs::rename(&tmp, &f)?;
        Ok(())
    }

    /// Every game added by hand, in the order they were added.
    pub fn custom_games(&self) -> Vec<CustomGame> {
        self.custom_load().map(|l| l.games).unwrap_or_default()
    }

    pub fn custom_game(&self, id: &str) -> Result<CustomGame> {
        self.custom_games().into_iter().find(|g| g.id == id).ok_or_else(|| anyhow!("no game \"{id}\" added in Beacon (see `beacon custom list`)"))
    }

    /// Change one game: the list is read again, changed, and written at once.
    pub fn custom_update(&self, id: &str, f: impl FnOnce(&mut CustomGame) -> Result<()>) -> Result<CustomGame> {
        let mut lib = self.custom_load()?;
        let g = lib.games.iter_mut().find(|g| g.id == id).ok_or_else(|| anyhow!("no game \"{id}\" added in Beacon"))?;
        f(g)?;
        let out = g.clone();
        self.custom_save(&lib)?;
        Ok(out)
    }

    fn custom_insert(&self, mut g: CustomGame) -> Result<CustomGame> {
        let mut lib = self.custom_load()?;
        g.id = unique_id(&slug(if g.id.is_empty() { &g.title } else { &g.id }), &lib.games);
        if g.added == 0 {
            g.added = playtime::now();
        }
        lib.games.push(g.clone());
        self.custom_save(&lib)?;
        Ok(g)
    }

    /// Take the icon out of the program (or an .ico/.png given as the icon) into the game's art folder.
    pub fn custom_take_icon(&self, id: &str, from: &Path) -> Result<Option<PathBuf>> {
        let out = self.custom_art_dir(id).join("icon.png");
        Ok(crate::pe_icon::extract_to_png(from, &out)?.then_some(out))
    }

    /// Add a Windows program as a game. `title` empty: made from the program's name.
    pub fn custom_add(&self, exe: &Path, title: Option<&str>, args: &str, runner: Option<&str>) -> Result<CustomGame> {
        if !exe.is_file() {
            bail!("{} isn't there", exe.display());
        }
        if let Some(r) = runner {
            self.runners.get(r).ok_or_else(|| anyhow!("no runner {r}; see `beacon runners`"))?;
        }
        let exe = std::path::absolute(exe).unwrap_or_else(|_| exe.to_path_buf());
        let title = title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| title_from_exe(&exe));
        if title.chars().any(|c| c.is_control()) || title.chars().count() > 120 {
            bail!("that title can't be used");
        }
        let g = self.custom_insert(CustomGame { title, exe: Some(exe.clone()), args: args.to_string(), runner: runner.map(str::to_string), ..Default::default() })?;
        let icon = self.custom_take_icon(&g.id, &exe).unwrap_or(None);
        if icon.is_some() {
            return self.custom_update(&g.id, |x| {
                x.icon = icon;
                Ok(())
            });
        }
        Ok(g)
    }

    /// The install flow, step 1: a new game (no program yet) with its own new prefix, remembering the installer.
    /// Run [`Beacon::custom_run_command`] with the installer, then [`Beacon::exe_candidates`] and set `exe`.
    pub fn custom_install_begin(&self, installer: &Path, title: Option<&str>, runner: Option<&str>) -> Result<CustomGame> {
        if !installer.is_file() {
            bail!("{} isn't there", installer.display());
        }
        if let Some(r) = runner {
            self.runners.get(r).ok_or_else(|| anyhow!("no runner {r}; see `beacon runners`"))?;
        }
        let installer = std::path::absolute(installer).unwrap_or_else(|_| installer.to_path_buf());
        let title = title.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).unwrap_or_else(|| {
            let t = title_from_exe(&installer);
            let cleaned: String = t
                .split_whitespace()
                .filter(|w| !["setup", "installer", "install", "gog"].contains(&w.to_ascii_lowercase().as_str()))
                .collect::<Vec<_>>()
                .join(" ");
            if cleaned.is_empty() { t } else { cleaned }
        });
        let g = self.custom_insert(CustomGame { title, installer: Some(installer.clone()), runner: runner.map(str::to_string), ..Default::default() })?;
        std::fs::create_dir_all(self.custom_prefix(&g).parent().unwrap_or(Path::new("/")))?;
        let icon = self.custom_take_icon(&g.id, &installer).unwrap_or(None);
        if icon.is_some() {
            return self.custom_update(&g.id, |x| {
                x.icon = icon;
                Ok(())
            });
        }
        Ok(g)
    }

    /// Programs an installer put in the game's prefix, likeliest first (biggest, skipping uninstallers,
    /// redistributables and Windows' own).
    pub fn exe_candidates(&self, g: &CustomGame) -> Vec<PathBuf> {
        let c = self.custom_prefix(g).join("drive_c");
        let skip_dirs = ["windows", "programdata", "temp", "common files", "internet explorer", "windows media player", "windows nt", "microsoft.net", "dotnet", "windowspowershell", "_commonredist", "redist", "directx", "vcredist"];
        let skip_names = ["unins", "setup", "install", "vcredist", "vc_redist", "dxsetup", "dxwebsetup", "crashhandler", "crashreport", "crashpad", "report", "dotnet", "ndp", "oalinst", "physx", "ue4prereq", "uninstall", "updater", "helper", "cefprocess", "webhelper"];
        let mut found: Vec<(u64, PathBuf)> = walkdir::WalkDir::new(&c)
            .max_depth(8)
            .into_iter()
            .filter_entry(|e| !(e.file_type().is_dir() && skip_dirs.contains(&e.file_name().to_string_lossy().to_ascii_lowercase().as_str())))
            .flatten()
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.ends_with(".exe") && !skip_names.iter().any(|s| n.contains(s))
            })
            .map(|e| (e.metadata().map(|m| m.len()).unwrap_or(0), e.into_path()))
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0));
        found.into_iter().take(25).map(|(_, p)| p).collect()
    }

    pub fn custom_prefix(&self, g: &CustomGame) -> PathBuf {
        g.prefix.clone().unwrap_or_else(|| self.custom_default_prefix(&g.id))
    }

    pub fn custom_runner(&self, g: &CustomGame) -> Option<&Runner> {
        g.runner
            .as_deref()
            .and_then(|id| self.runners.get(id))
            .or_else(|| self.config.default_runner.as_deref().and_then(|id| self.runners.get(id)))
            .or_else(|| self.runners.best())
    }

    pub fn custom_status(&self, g: &CustomGame) -> CustomStatus {
        let mut issues = Vec::new();
        let installed = g.exe.as_deref().map(|p| p.is_file()).unwrap_or(false);
        match &g.exe {
            None if g.installer.is_some() => issues.push(Issue { level: Level::Error, message: "Installed? Choose the game's program (Game settings → Program).".into() }),
            None => issues.push(Issue { level: Level::Error, message: "No program chosen.".into() }),
            Some(p) if !installed => issues.push(Issue { level: Level::Error, message: format!("{} isn't there anymore.", p.display()) }),
            _ => {}
        }
        let runner = self.custom_runner(g).cloned();
        if runner.is_none() {
            issues.push(Issue { level: Level::Error, message: "No Wine, Proton or umu-launcher found — get one under Launcher → Compatibility tools.".into() });
        }
        if let Some(id) = g.runner.as_deref().filter(|id| self.runners.get(id).is_none()) {
            issues.push(Issue { level: Level::Warn, message: format!("Its runner {id} is gone; using {} instead.", runner.as_ref().map(|r| r.name.as_str()).unwrap_or("none")) });
        }
        let path_env = self.path_env.clone();
        if !self.native {
            for (on, bin, name) in [(g.mangohud && !g.gamescope, "mangohud", "MangoHud"), (g.gamemode, "gamemoderun", "GameMode"), (g.gamescope, "gamescope", "gamescope")] {
                if on && runner::which_in(bin, &path_env).is_none() {
                    issues.push(Issue { level: Level::Warn, message: format!("{name} is on but isn't installed; the game starts without it.") });
                }
            }
            if runner.as_ref().map(|r| r.kind == RunnerKind::Wine).unwrap_or(false) && !g.no_dxvk && crate::tools::beacon_newest(&self.paths.data_dir, crate::tools::Kind::Dxvk).is_none() {
                issues.push(Issue { level: Level::Warn, message: "DXVK isn't installed for Wine runners (Launcher → Compatibility tools); DirectX 9–11 games use Wine's slower OpenGL path.".into() });
            }
        }
        let wineprefix = self.custom_prefix(g);
        let stats: PlayStats = playtime::get(&self.playtime_file(), &format!("custom:{}", g.id));
        let log_file = self.custom_log(&g.id);
        CustomStatus {
            key: format!("custom:{}", g.id),
            family: "other".into(),
            installed,
            ready: !issues.iter().any(|i| i.level == Level::Error),
            issues,
            resolved_runner: runner,
            prefix_ready: !self.native && wineprefix.join("drive_c").is_dir(),
            own_prefix: g.prefix.is_none(),
            wineprefix,
            play_seconds: stats.seconds,
            last_played: stats.last_played,
            sessions: stats.sessions,
            has_log: log_file.is_file(),
            log_file,
            installer_like: g.exe.as_deref().map(looks_like_installer).unwrap_or(false),
            game: g.clone(),
        }
    }

    pub fn custom_statuses(&self) -> Vec<CustomStatus> {
        self.custom_games().iter().map(|g| self.custom_status(g)).collect()
    }

    // ---- changing one ------------------------------------------------------------------------------

    /// One change, by key ([`SET_KEYS`]). `env`: "KEY=VALUE" (an empty value removes KEY); `flag`: "+key" or
    /// "-key"; images and paths: a file, or "" to clear; runner "auto" for automatic.
    pub fn custom_set(&self, id: &str, key: &str, value: &str) -> Result<CustomGame> {
        if value.chars().any(|c| c.is_control() && c != '\t') {
            bail!("that value can't be used");
        }
        let path_or_none = |v: &str| -> Option<PathBuf> { (!v.trim().is_empty()).then(|| expand_home(v.trim(), &self.paths.home)) };
        match key {
            "runner" if value != "auto" && self.runners.get(value).is_none() => bail!("no runner {value}; see `beacon runners`"),
            "exe" if !path_or_none(value).map(|p| p.is_file()).unwrap_or(false) => bail!("{value} isn't a file"),
            "cwd" | "prefix" if path_or_none(value).map(|p| p.exists() && !p.is_dir()).unwrap_or(false) => bail!("{value} isn't a folder"),
            "icon" | "cover" | "hero" if path_or_none(value).map(|p| !p.is_file()).unwrap_or(false) => bail!("{value} isn't a file"),
            "store" if !value.is_empty() && !STORES.contains(&value) => bail!("STORE is one of {}", STORES.join(", ")),
            "umu-id" if value.len() > 64 || value.contains(char::is_whitespace) => bail!("that GAMEID can't be used"),
            k if !SET_KEYS.contains(&k) => bail!("unknown setting {k} (one of {})", SET_KEYS.join(", ")),
            _ => {}
        }
        // Images are copied into Beacon's art folder (the icon converted to PNG) so they survive the original moving.
        let art = self.custom_art_dir(id);
        let copied = match key {
            "icon" => match path_or_none(value) {
                Some(p) => Some(Some(self.custom_take_icon(id, &p)?.ok_or_else(|| anyhow!("{} has no icon in it", p.display()))?)),
                None => Some(None),
            },
            "cover" | "hero" => match path_or_none(value) {
                Some(p) => {
                    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).filter(|e| ["png", "jpg", "jpeg", "webp"].contains(&e.as_str())).ok_or_else(|| anyhow!("use a .png, .jpg or .webp picture"))?;
                    std::fs::create_dir_all(&art)?;
                    let out = art.join(format!("{key}.{ext}"));
                    for old in ["png", "jpg", "jpeg", "webp"] {
                        let _ = std::fs::remove_file(art.join(format!("{key}.{old}")));
                    }
                    std::fs::copy(&p, &out).with_context(|| format!("copying {}", p.display()))?;
                    Some(Some(out))
                }
                None => Some(None),
            },
            _ => None,
        };
        let new_exe_icon = key == "exe" && self.custom_game(id).map(|g| g.icon.is_none()).unwrap_or(false);
        let mut g = self.custom_update(id, |g| {
            match key {
                "title" => {
                    let t = value.trim();
                    if t.is_empty() || t.chars().count() > 120 {
                        bail!("a title is 1 to 120 characters");
                    }
                    g.title = t.to_string();
                }
                "exe" => g.exe = path_or_none(value).map(|p| std::path::absolute(&p).unwrap_or(p)),
                "args" => g.args = value.to_string(),
                "cwd" => g.cwd = path_or_none(value),
                "prefix" => g.prefix = path_or_none(value),
                "runner" => g.runner = (value != "auto").then(|| value.to_string()),
                "env" => {
                    let (k, v) = value.split_once('=').ok_or_else(|| anyhow!("use KEY=VALUE (an empty value removes it)"))?;
                    let k = k.trim();
                    if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || k.as_bytes()[0].is_ascii_digit() {
                        bail!("\"{k}\" isn't a variable name");
                    }
                    if v.is_empty() {
                        g.env.remove(k);
                    } else {
                        g.env.insert(k.to_string(), v.to_string());
                    }
                }
                "dxvk" => g.no_dxvk = !on_off(value)?,
                "vkd3d" => g.no_vkd3d = !on_off(value)?,
                "mangohud" => g.mangohud = on_off(value)?,
                "gamemode" => g.gamemode = on_off(value)?,
                "gamescope" => g.gamescope = on_off(value)?,
                "gamescope-args" => g.gamescope_args = value.trim().to_string(),
                "flag" => {
                    let (on, k) = match value.chars().next() {
                        Some('+') => (true, &value[1..]),
                        Some('-') => (false, &value[1..]),
                        _ => bail!("use +<option> or -<option>"),
                    };
                    if !FLAGS.iter().any(|(f, ..)| *f == k) {
                        bail!("unknown option {k} (one of {})", FLAGS.iter().map(|f| f.0).collect::<Vec<_>>().join(", "));
                    }
                    g.flags.retain(|f| f != k);
                    if on {
                        g.flags.push(k.to_string());
                    }
                }
                "umu-id" => g.umu_id = (!value.trim().is_empty()).then(|| value.trim().to_string()),
                "store" => g.store = (!value.is_empty()).then(|| value.to_string()),
                "icon" => g.icon = copied.clone().flatten(),
                "cover" => g.cover = copied.clone().flatten(),
                "hero" => g.hero = copied.clone().flatten(),
                _ => {}
            }
            Ok(())
        })?;
        if new_exe_icon {
            if let Some(exe) = g.exe.clone() {
                if let Ok(Some(icon)) = self.custom_take_icon(id, &exe) {
                    g = self.custom_update(id, |x| {
                        x.icon = Some(icon);
                        Ok(())
                    })?;
                }
            }
        }
        Ok(g)
    }

    /// Remove a game. `delete_prefix`: also delete its Wine prefix, when that's safe (it's a Wine prefix, it isn't
    /// a top folder or your home, and no other game uses it).
    pub fn custom_remove(&self, id: &str, delete_prefix: bool) -> Result<RemoveReport> {
        let mut lib = self.custom_load()?;
        let pos = lib.games.iter().position(|g| g.id == id).ok_or_else(|| anyhow!("no game \"{id}\" added in Beacon"))?;
        let g = lib.games.remove(pos);
        let mut report = RemoveReport { id: id.to_string(), ..Default::default() };
        if delete_prefix && !self.native {
            let pfx = self.custom_prefix(&g);
            let shared = lib.games.iter().any(|o| self.custom_prefix(o) == pfx);
            let home = &self.paths.home;
            // Beacon's own prefixes go with their root (<data>/prefixes/custom/<id>).
            let target = if g.prefix.is_none() { pfx.parent().map(Path::to_path_buf).unwrap_or(pfx.clone()) } else { pfx.clone() };
            let deep_enough = target.components().count() >= 4 && &target != home && !home.starts_with(&target);
            if shared {
                report.kept_prefix = Some("another game uses the same prefix".into());
            } else if !pfx.join("drive_c").is_dir() && !pfx.join("system.reg").is_file() {
                report.kept_prefix = Some(if pfx.exists() { "it doesn't look like a Wine prefix".into() } else { "there was no prefix yet".into() });
            } else if !deep_enough {
                report.kept_prefix = Some("that folder is too close to the top of the disk to delete safely".into());
            } else {
                std::fs::remove_dir_all(&target).with_context(|| format!("deleting {}", target.display()))?;
                report.deleted_prefix = Some(target);
            }
        }
        self.custom_save(&lib)?;
        let _ = std::fs::remove_dir_all(self.custom_art_dir(id));
        let _ = playtime::forget(&self.playtime_file(), &format!("custom:{id}"));
        Ok(report)
    }

    // ---- running ---------------------------------------------------------------------------------

    /// The command that runs `target` (a program's path, or a Wine builtin) in the game's prefix with its options.
    /// `game`: the game itself (gamescope, GameMode and MangoHud wrap it), not a tool or an installer.
    pub fn custom_command(&self, g: &CustomGame, target: &str, args: &[String], cwd: &Path, game: bool) -> Result<Command> {
        let r = self.custom_runner(g).ok_or_else(|| anyhow!("no Wine, Proton or umu-launcher found — get one under Launcher → Compatibility tools"))?;
        let pfx = self.custom_prefix(g);
        let mut cmd = runner::command_for_prefix(r, self.runners.umu.as_deref(), &self.paths.home, &pfx, target, args, cwd);
        let set = |cmd: &mut Command, k: &str, v: String| {
            cmd.env.retain(|(ek, _)| ek != k);
            cmd.env.push((k.to_string(), v));
        };
        if r.kind != RunnerKind::Native {
            let protonish = matches!(r.kind, RunnerKind::Proton | RunnerKind::Umu);
            if protonish {
                if let Some(id) = &g.umu_id {
                    set(&mut cmd, "GAMEID", id.clone());
                }
                if let Some(s) = &g.store {
                    set(&mut cmd, "STORE", s.clone());
                }
                if g.no_dxvk {
                    set(&mut cmd, "PROTON_USE_WINED3D", "1".into());
                }
                if g.no_vkd3d {
                    set(&mut cmd, "PROTON_NO_D3D12", "1".into());
                }
            } else {
                // Wine: DXVK / VKD3D-Proton DLLs copied into the prefix (custom_prepare), used ahead of Wine's own.
                let mut native: Vec<String> = Vec::new();
                for (off, kind) in [(g.no_dxvk, crate::tools::Kind::Dxvk), (g.no_vkd3d, crate::tools::Kind::Vkd3d)] {
                    if off {
                        continue;
                    }
                    if let Some(dir) = crate::tools::beacon_newest(&self.paths.data_dir, kind) {
                        for e in std::fs::read_dir(dir.join("x64")).into_iter().flatten().flatten() {
                            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                            if let Some(stem) = n.strip_suffix(".dll") {
                                native.push(stem.to_string());
                            }
                        }
                    }
                }
                if !native.is_empty() {
                    native.sort();
                    native.dedup();
                    set(&mut cmd, "WINEDLLOVERRIDES", format!("mscoree,mshtml=;{}=n,b", native.join(",")));
                }
            }
            for (k, var, val, _) in FLAGS {
                if g.flags.iter().any(|f| f == *k) {
                    set(&mut cmd, var, val.to_string());
                }
            }
        }
        for (k, v) in &g.env {
            set(&mut cmd, k, v.clone());
        }
        if game && !self.native {
            let path_env = self.path_env.clone();
            // Inside a gamescope session (Hearth's Game Mode, SteamOS) a nested gamescope only gets in the way.
            let nested = std::env::var_os("GAMESCOPE_WAYLAND_DISPLAY").is_some();
            let gamescope = if g.gamescope && !nested { runner::which_in("gamescope", &path_env) } else { None };
            let mut prefix: Vec<String> = Vec::new();
            if let Some(gm) = g.gamemode.then(|| runner::which_in("gamemoderun", &path_env)).flatten() {
                prefix.push(gm.to_string_lossy().into_owned());
            }
            if g.mangohud && gamescope.is_none() {
                if let Some(m) = runner::which_in("mangohud", &path_env) {
                    prefix.push(m.to_string_lossy().into_owned());
                }
            }
            let mut wrapped: Vec<String> = Vec::new();
            let mut program = cmd.program.clone();
            if let Some(gs) = gamescope {
                wrapped.extend(crate::config::split_args(&g.gamescope_args));
                if g.mangohud {
                    wrapped.push("--mangoapp".into());
                }
                wrapped.push("--".into());
                program = gs;
            }
            if program != cmd.program || !prefix.is_empty() {
                let first = if program != cmd.program { None } else { Some(PathBuf::from(prefix.remove(0))) };
                wrapped.extend(prefix);
                wrapped.push(cmd.program.to_string_lossy().into_owned());
                wrapped.extend(cmd.args.drain(..));
                cmd.program = first.unwrap_or(program);
                cmd.args = wrapped;
            }
        }
        Ok(cmd)
    }

    /// What a launch runs.
    pub fn custom_launch_command(&self, id: &str) -> Result<Command> {
        let g = self.custom_game(id)?;
        let st = self.custom_status(&g);
        if let Some(e) = st.issues.iter().find(|i| i.level == Level::Error) {
            bail!("{}", e.message);
        }
        let exe = g.exe.clone().unwrap();
        let cwd = g.cwd.clone().filter(|d| d.is_dir()).unwrap_or_else(|| exe.parent().unwrap_or(Path::new("/")).to_path_buf());
        self.custom_command(&g, &exe.to_string_lossy(), &crate::config::split_args(&g.args), &cwd, true)
    }

    /// Before running anything in the prefix: create it (Wine runners: `wineboot`), and for Wine runners copy the
    /// DXVK / VKD3D-Proton DLLs in. Proton and umu make their prefix themselves.
    pub fn custom_prepare(&self, g: &CustomGame) -> Result<()> {
        if self.native {
            return Ok(());
        }
        let pfx = self.custom_prefix(g);
        std::fs::create_dir_all(pfx.parent().unwrap_or(Path::new("/")))?;
        let Some(r) = self.custom_runner(g) else { return Ok(()) };
        if r.kind != RunnerKind::Wine {
            return Ok(());
        }
        if !pfx.join("drive_c/windows").is_dir() {
            let cmd = self.custom_command(g, "wineboot", &["-u".into()], &pfx, false)?;
            let log = self.custom_open_log(&g.id, &cmd, "prefix")?;
            let st = cmd.to_std().stdout(log.try_clone()?).stderr(log).status().with_context(|| format!("starting {}", cmd.program.display()))?;
            if !st.success() {
                bail!("creating the prefix failed ({st}); see {}", self.custom_log(&g.id).display());
            }
        }
        for (off, kind) in [(g.no_dxvk, crate::tools::Kind::Dxvk), (g.no_vkd3d, crate::tools::Kind::Vkd3d)] {
            if let (false, Some(dir)) = (off, crate::tools::beacon_newest(&self.paths.data_dir, kind)) {
                if pfx.join("drive_c/windows").is_dir() {
                    crate::tools::install_dlls(&dir, &pfx)?;
                }
            }
        }
        Ok(())
    }

    fn custom_open_log(&self, id: &str, cmd: &Command, what: &str) -> Result<std::fs::File> {
        use std::io::Write;
        let path = self.custom_log(id);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(what != "launch").write(true).open(&path)?;
        if what == "launch" {
            f.set_len(0)?;
        }
        writeln!(f, "== beacon {what}: {}", cmd.display())?;
        Ok(f)
    }

    /// Start a command for a game with its output in the game's log.
    pub fn custom_spawn(&self, id: &str, cmd: &Command, what: &str) -> Result<std::process::Child> {
        let log = self.custom_open_log(id, cmd, what)?;
        cmd.to_std().stdout(log.try_clone()?).stderr(log).spawn().with_context(|| format!("starting {}", cmd.program.display()))
    }

    /// Prepare the prefix and start the game. The caller owns the child (and records play time when it closes:
    /// [`Beacon::custom_played`]).
    pub fn custom_launch(&self, id: &str) -> Result<std::process::Child> {
        let cmd = self.custom_launch_command(id)?;
        self.custom_prepare(&self.custom_game(id)?)?;
        self.custom_spawn(id, &cmd, "launch")
    }

    /// Adds a finished session to the game's play time.
    pub fn custom_played(&self, id: &str, seconds: u64) -> Result<PlayStats> {
        playtime::record(&self.playtime_file(), &format!("custom:{id}"), seconds)
    }

    /// Run another program in the game's prefix (an installer, a patch, a mod tool).
    pub fn custom_run_command(&self, id: &str, exe: &Path) -> Result<Command> {
        let g = self.custom_game(id)?;
        if !exe.is_file() {
            bail!("{} isn't there", exe.display());
        }
        let cwd = exe.parent().unwrap_or(Path::new("/")).to_path_buf();
        let lower = exe.to_string_lossy().to_ascii_lowercase();
        if lower.ends_with(".msi") {
            return self.custom_command(&g, "msiexec", &["/i".into(), exe.to_string_lossy().into_owned()], &cwd, false);
        }
        self.custom_command(&g, &exe.to_string_lossy(), &[], &cwd, false)
    }

    /// A Wine tool for the game's prefix ([`WINE_TOOLS`]); `extra`: winetricks verbs (none opens its window).
    pub fn custom_tool_command(&self, id: &str, tool: &str, extra: &[String]) -> Result<Command> {
        if self.native {
            bail!("there's no Wine prefix on Windows");
        }
        let g = self.custom_game(id)?;
        let pfx = self.custom_prefix(&g);
        let r = self.custom_runner(&g).ok_or_else(|| anyhow!("no Wine, Proton or umu-launcher found"))?.clone();
        let target = match tool {
            "winecfg" | "regedit" | "taskmgr" | "control" => tool,
            "cmd" => "wineconsole",
            "explorer" => "explorer",
            "uninstaller" => "uninstaller",
            "kill" => "wineserver",
            "winetricks" => "winetricks",
            _ => bail!("unknown tool {tool} (one of {})", WINE_TOOLS.iter().map(|t| t.0).collect::<Vec<_>>().join(", ")),
        };
        if extra.iter().any(|v| v.starts_with('-') && v != "-q" && v != "--unattended" || v.chars().any(|c| !(c.is_ascii_alphanumeric() || "_-=.".contains(c)))) {
            bail!("winetricks verbs are letters, digits, _ - = and . only");
        }
        if tool == "winetricks" {
            // umu runs winetricks itself in the prefix (with Proton's Wine); otherwise the system's winetricks with
            // the runner's wine.
            if r.kind == RunnerKind::Umu || (r.kind == RunnerKind::Proton && self.runners.umu.is_some()) {
                return self.custom_command(&g, "winetricks", extra, &pfx, false);
            }
            let path_env = self.path_env.clone();
            let wt = runner::which_in("winetricks", &path_env).ok_or_else(|| anyhow!("winetricks isn't installed (or install umu-launcher, which brings its own)"))?;
            let wine = match r.kind {
                RunnerKind::Wine => r.path.clone(),
                _ => ["files/bin/wine", "dist/bin/wine"].iter().map(|p| r.path.join(p)).find(|p| p.is_file()).ok_or_else(|| anyhow!("{} has no wine binary for winetricks", r.name))?,
            };
            let mut args = vec![];
            if extra.is_empty() {
                args.push("--gui".to_string());
            }
            args.extend(extra.iter().cloned());
            let pfx_s = if pfx.file_name().map(|n| n == "pfx").unwrap_or(false) || r.kind == RunnerKind::Wine { pfx.clone() } else { pfx.join("pfx") };
            let wineserver = wine.with_file_name("wineserver");
            return Ok(Command {
                program: wt,
                args,
                env: vec![("WINEPREFIX".into(), pfx_s.to_string_lossy().into()), ("WINE".into(), wine.to_string_lossy().into()), ("WINESERVER".into(), wineserver.to_string_lossy().into())],
                cwd: pfx,
            });
        }
        if tool == "kill" {
            return self.custom_command(&g, "wineserver", &["-k".into()], &pfx, false).map(|mut c| {
                // Plain Wine: wineserver sits next to wine.
                if r.kind == RunnerKind::Wine {
                    c.program = r.path.with_file_name("wineserver");
                    c.args = vec!["-k".into()];
                }
                c
            });
        }
        let args: Vec<String> = if tool == "explorer" { vec!["C:\\".into()] } else { vec![] };
        self.custom_command(&g, target, &args, &pfx, false)
    }

    // ---- import / export -----------------------------------------------------------------------------

    /// Everything added by hand (and its play time), for another PC or a backup.
    pub fn custom_export(&self, file: &Path) -> Result<usize> {
        let games = self.custom_games();
        let pt = playtime::load(&self.playtime_file());
        let times: BTreeMap<String, PlayStats> = games.iter().filter_map(|g| pt.get(&format!("custom:{}", g.id)).map(|s| (g.id.clone(), *s))).collect();
        let v = serde_json::json!({"beacon_custom_games": 1, "exported": playtime::now(), "games": games, "playtime": times});
        if let Some(d) = file.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(file, serde_json::to_string_pretty(&v)?).with_context(|| format!("writing {}", file.display()))?;
        Ok(games.len())
    }

    /// Import a Beacon export or a Faugus Launcher games.json. Games whose program is already in the list are skipped.
    pub fn custom_import(&self, file: &Path) -> Result<ImportReport> {
        let text = std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
        let v: serde_json::Value = serde_json::from_str(&text).with_context(|| format!("{} isn't JSON", file.display()))?;
        if v.is_array() {
            let base = file.parent().unwrap_or(Path::new("."));
            return self.import_faugus_list(&v, base);
        }
        if v["beacon_custom_games"].is_null() {
            bail!("{} isn't a Beacon export or a Faugus Launcher games.json", file.display());
        }
        let games: Vec<CustomGame> = serde_json::from_value(v["games"].clone()).context("reading the exported games")?;
        let mut report = ImportReport::default();
        for mut g in games {
            if self.custom_games().iter().any(|x| x.exe.is_some() && x.exe == g.exe) {
                report.skipped.push(format!("{} (already added)", g.title));
                continue;
            }
            let old = g.id.clone();
            // Art files stay where they were; Beacon's art folder for the new id is filled when they're there.
            g.source = Some(g.source.clone().unwrap_or_else(|| "import".into()));
            let added = self.custom_insert(g.clone())?;
            for (k, p) in [("icon", &g.icon), ("cover", &g.cover), ("hero", &g.hero)] {
                if let Some(p) = p.as_ref().filter(|p| p.is_file()) {
                    let _ = self.custom_set(&added.id, k, &p.to_string_lossy());
                }
            }
            if let Ok(s) = serde_json::from_value::<PlayStats>(v["playtime"][&old].clone()) {
                let _ = playtime::merge(&self.playtime_file(), &format!("custom:{}", added.id), s);
            }
            report.added.push(added.id);
        }
        Ok(report)
    }

    /// Faugus Launcher's games.json, if Faugus is (or was) set up here: native (XDG data, older versions XDG
    /// config) or its Flatpak.
    pub fn faugus_games_file(&self) -> Option<PathBuf> {
        let h = &self.paths.home;
        let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| h.join(".local/share"));
        let conf = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| h.join(".config"));
        let fp = h.join(".var/app/io.github.Faugus.faugus-launcher");
        [
            data.join("faugus-launcher/games.json"),
            h.join(".local/share/faugus-launcher/games.json"),
            conf.join("faugus-launcher/games.json"),
            h.join(".config/faugus-launcher/games.json"),
            fp.join("data/faugus-launcher/games.json"),
            fp.join("config/faugus-launcher/games.json"),
        ]
        .into_iter()
        .find(|p| p.is_file())
    }

    /// Import every game from an existing Faugus Launcher setup.
    pub fn import_faugus(&self) -> Result<ImportReport> {
        let f = self.faugus_games_file().ok_or_else(|| anyhow!("no Faugus Launcher library found (~/.local/share/faugus-launcher/games.json or ~/.config/faugus-launcher/games.json)"))?;
        self.custom_import(&f)
    }

    /// A Faugus runner name as one of Beacon's runners.
    fn faugus_runner(&self, name: &str) -> Option<String> {
        let n = name.trim().trim_matches('"');
        if n.is_empty() || n == "Default" {
            return None;
        }
        if let Some(r) = self.runners.runners.iter().find(|r| r.name == n) {
            return Some(r.id.clone());
        }
        let l = n.to_ascii_lowercase();
        let pick = |want: &str| self.runners.runners.iter().find(|r| r.name.to_ascii_lowercase().contains(want)).map(|r| r.id.clone());
        if l.starts_with("umu-proton") {
            return pick("umu-proton").or_else(|| self.runners.get("umu").map(|r| r.id.clone()));
        }
        if l.starts_with("ge-proton") {
            return pick("ge-proton");
        }
        if l.contains("cachyos") {
            return pick("cachyos");
        }
        if l.contains("proton-em") {
            return pick("proton-em");
        }
        None
    }

    fn import_faugus_list(&self, list: &serde_json::Value, faugus_dir: &Path) -> Result<ImportReport> {
        let truthy = |v: &serde_json::Value| match v {
            serde_json::Value::Bool(b) => *b,
            serde_json::Value::String(s) => !s.is_empty() && s != "False" && s != "false",
            _ => false,
        };
        let s = |v: &serde_json::Value| v.as_str().unwrap_or("").trim().to_string();
        let home = self.paths.home.clone();
        let mut report = ImportReport::default();
        for item in list.as_array().into_iter().flatten().take(2000) {
            let title = s(&item["title"]);
            if title.is_empty() {
                continue;
            }
            let runner = s(&item["runner"]);
            if runner == "Linux-Native" || runner == "Steam" {
                report.skipped.push(format!("{title} (a {} game, not a Windows one)", if runner == "Steam" { "Steam" } else { "native Linux" }));
                continue;
            }
            let path = s(&item["path"]);
            if path.is_empty() {
                report.skipped.push(format!("{title} (no program)"));
                continue;
            }
            let exe = expand_home(&path, &home);
            if self.custom_games().iter().any(|g| g.exe.as_deref() == Some(exe.as_path())) {
                report.skipped.push(format!("{title} (already added)"));
                continue;
            }
            let mut env = BTreeMap::new();
            for tok in crate::config::split_args(&s(&item["launch_arguments"])) {
                if let Some((k, v)) = tok.split_once('=') {
                    if !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                        env.insert(k.to_string(), v.to_string());
                    }
                }
            }
            let fix = s(&item["protonfix"]);
            let umu_id = if fix.is_empty() { None } else if fix.bytes().all(|b| b.is_ascii_digit()) { Some(format!("umu-{fix}")) } else { Some(fix) };
            let prefix = s(&item["prefix"]);
            let mut flags = Vec::new();
            if truthy(&item["sdl_enabled"]) {
                flags.push("sdl".to_string());
            }
            let g = CustomGame {
                id: s(&item["gameid"]),
                title: title.clone(),
                exe: Some(exe.clone()),
                args: s(&item["game_arguments"]),
                prefix: (!prefix.is_empty()).then(|| expand_home(&prefix, &home)),
                runner: self.faugus_runner(&runner),
                env,
                mangohud: truthy(&item["mangohud"]),
                gamemode: truthy(&item["gamemode"]),
                flags,
                umu_id,
                source: Some("faugus".into()),
                ..Default::default()
            };
            let fid = g.id.clone();
            let added = self.custom_insert(g)?;
            // Art: Faugus keeps icons, covers and banners next to games.json (icons/<gameid>.ico|png …).
            let find_art = |dir: &str, explicit: &serde_json::Value| -> Option<PathBuf> {
                let e = s(explicit);
                if !e.is_empty() {
                    let p = expand_home(&e, &home);
                    if p.is_file() {
                        return Some(p);
                    }
                }
                ["png", "ico", "jpg", "jpeg", "webp"].iter().map(|x| faugus_dir.join(dir).join(format!("{fid}.{x}"))).find(|p| p.is_file())
            };
            let mut set_any = false;
            if let Some(p) = find_art("icons", &item["icon"]) {
                set_any |= self.custom_set(&added.id, "icon", &p.to_string_lossy()).is_ok();
            }
            if !set_any && exe.is_file() {
                if let Ok(Some(icon)) = self.custom_take_icon(&added.id, &exe) {
                    let _ = self.custom_update(&added.id, |x| {
                        x.icon = Some(icon);
                        Ok(())
                    });
                }
            }
            if let Some(p) = find_art("covers", &item["cover"]) {
                let _ = self.custom_set(&added.id, "cover", &p.to_string_lossy());
            }
            if let Some(p) = find_art("banners", &serde_json::Value::Null) {
                let _ = self.custom_set(&added.id, "hero", &p.to_string_lossy());
            }
            let secs = item["playtime"].as_u64().or_else(|| s(&item["playtime"]).parse().ok()).unwrap_or(0);
            let last = playtime::parse_iso(&s(&item["last_played"])).unwrap_or(0);
            if secs > 0 || last > 0 {
                let _ = playtime::merge(&self.playtime_file(), &format!("custom:{}", added.id), PlayStats { seconds: secs, last_played: last, sessions: 0 });
            }
            report.added.push(added.id);
        }
        Ok(report)
    }
}

/// A small base64 encoder (images for Beacon's window, as data: URLs).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// An image file as a data: URL (at most 4 MB), for Beacon's window.
pub fn data_url(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    if m.len() > 4 << 20 {
        return None;
    }
    let mime = match p.extension()?.to_string_lossy().to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => return None,
    };
    Some(format!("data:{mime};base64,{}", base64(&std::fs::read(p).ok()?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use std::fs;

    fn env() -> (tempfile::TempDir, Beacon, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let games = home.join("Games");
        fs::create_dir_all(&games).unwrap();
        let bin = t.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        for b in ["wine", "umu-run", "gamemoderun", "mangohud"] {
            fs::write(bin.join(b), "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/windows/system32\"\nexit 0\n").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(bin.join(b), fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let b = Beacon::load_with(Paths::under(&t.path().join("beacon"), &home), &bin.to_string_lossy()).unwrap();
        (t, b, games)
    }

    #[test]
    fn ids_titles_and_installers() {
        assert_eq!(slug("The Witcher 3: Wild Hunt"), "the-witcher-3-wild-hunt");
        assert_eq!(slug("  ¡¿  "), "game");
        assert!(valid_id("the-witcher-3") && !valid_id("-x") && !valid_id("A") && !valid_id("a b") && !valid_id(""));
        assert_eq!(title_from_exe(Path::new("/g/Hollow_Knight/hollow_knight.exe")), "hollow knight");
        assert_eq!(title_from_exe(Path::new("/g/Celeste/bin/game.exe")), "Celeste");
        assert!(looks_like_installer(Path::new("/d/setup_witcher3_v4.04.exe")) && looks_like_installer(Path::new("/d/x.msi")));
        assert!(!looks_like_installer(Path::new("/g/witcher3.exe")));
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    // Runs stub wine/umu-run scripts.
    #[cfg(unix)]
    #[test]
    fn add_set_launch_remove() {
        let (_t, b, games) = env();
        let dir = games.join("Some Game");
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("SomeGame.exe");
        fs::write(&exe, crate::pe_icon::tests::tiny_exe()).unwrap();
        let g = b.custom_add(&exe, None, "-windowed", None).unwrap();
        assert_eq!((g.id.as_str(), g.title.as_str()), ("somegame", "SomeGame"));
        assert!(g.icon.as_ref().unwrap().is_file(), "icon taken from the exe");
        let g2 = b.custom_add(&exe, Some("SomeGame"), "", None).unwrap();
        assert_eq!(g2.id, "somegame-2");
        assert_eq!(b.custom_games().len(), 2);

        let st = b.custom_status(&b.custom_game("somegame").unwrap());
        assert!(st.ready && st.installed && st.own_prefix, "{:?}", st.issues);
        assert!(st.wineprefix.ends_with("prefixes/custom/somegame/pfx"));
        assert_eq!(st.key, "custom:somegame");

        // Runner umu (Proton-ish): GAMEID/STORE, DXVK off, a flag, env, wrappers.
        b.custom_set("somegame", "runner", "umu").unwrap();
        b.custom_set("somegame", "umu-id", "umu-292030").unwrap();
        b.custom_set("somegame", "store", "gog").unwrap();
        b.custom_set("somegame", "dxvk", "off").unwrap();
        b.custom_set("somegame", "flag", "+nvapi").unwrap();
        b.custom_set("somegame", "env", "DXVK_HUD=fps").unwrap();
        b.custom_set("somegame", "gamemode", "on").unwrap();
        b.custom_set("somegame", "mangohud", "on").unwrap();
        assert!(b.custom_set("somegame", "store", "nope").is_err());
        assert!(b.custom_set("somegame", "flag", "+nope").is_err());
        assert!(b.custom_set("somegame", "env", "1X=2").is_err());
        assert!(b.custom_set("somegame", "runner", "proton:missing").is_err());
        assert!(b.custom_set("somegame", "nope", "x").is_err());
        let c = b.custom_launch_command("somegame").unwrap();
        assert!(c.program.ends_with("gamemoderun"), "{c:?}");
        assert!(c.args[0].ends_with("mangohud") && c.args[1].ends_with("umu-run") && c.args[2].ends_with("SomeGame.exe") && c.args[3] == "-windowed", "{c:?}");
        for kv in [("GAMEID", "umu-292030"), ("STORE", "gog"), ("PROTON_USE_WINED3D", "1"), ("PROTON_ENABLE_NVAPI", "1"), ("DXVK_HUD", "fps")] {
            assert!(c.env.contains(&(kv.0.to_string(), kv.1.to_string())), "{kv:?} in {:?}", c.env);
        }
        assert_eq!(c.cwd, dir);
        b.custom_set("somegame", "flag", "-nvapi").unwrap();
        b.custom_set("somegame", "env", "DXVK_HUD=").unwrap();
        let g = b.custom_game("somegame").unwrap();
        assert!(g.flags.is_empty() && g.env.is_empty());

        // Wine tools and another program in the prefix.
        let t = b.custom_tool_command("somegame", "winecfg", &[]).unwrap();
        assert!(t.program.ends_with("umu-run") && t.args == vec!["winecfg"], "{t:?}");
        let t = b.custom_tool_command("somegame", "winetricks", &["vcrun2019".into()]).unwrap();
        assert_eq!(t.args, vec!["winetricks", "vcrun2019"]);
        assert!(b.custom_tool_command("somegame", "winetricks", &["x;rm".into()]).is_err());
        assert!(b.custom_tool_command("somegame", "rm", &[]).is_err());
        let patch = dir.join("patch.msi");
        fs::write(&patch, b"").unwrap();
        assert_eq!(b.custom_run_command("somegame", &patch).unwrap().args[0], "msiexec");

        // Launch for real (stub umu-run), play time recorded.
        let mut child = b.custom_launch("somegame").unwrap();
        assert!(child.wait().unwrap().success());
        b.custom_played("somegame", 120).unwrap();
        let st = b.custom_status(&b.custom_game("somegame").unwrap());
        assert_eq!((st.play_seconds, st.sessions), (120, 1));
        assert!(st.has_log);

        // Remove with its prefix (Beacon's own; the stub made drive_c).
        fs::create_dir_all(st.wineprefix.join("drive_c")).unwrap();
        let r = b.custom_remove("somegame", true).unwrap();
        assert!(r.deleted_prefix.unwrap().ends_with("prefixes/custom/somegame"));
        assert!(!st.wineprefix.exists());
        assert!(b.custom_game("somegame").is_err());
        assert_eq!(playtime::get(&b.playtime_file(), "custom:somegame").seconds, 0);
        // A prefix outside Beacon that isn't a prefix stays.
        b.custom_set("somegame-2", "prefix", &games.to_string_lossy()).unwrap();
        let r = b.custom_remove("somegame-2", true).unwrap();
        assert!(r.deleted_prefix.is_none() && r.kept_prefix.is_some());
        assert!(games.is_dir());
    }

    // Runs stub scripts.
    #[cfg(unix)]
    #[test]
    fn wine_runner_prefix_and_dxvk() {
        let (_t, b, games) = env();
        let exe = games.join("old.exe");
        fs::write(&exe, b"MZ").unwrap();
        let g = b.custom_add(&exe, Some("Old Game"), "", Some("wine:system")).unwrap();
        let dx = crate::runner::beacon_proton_dir(&b.paths.data_dir).parent().unwrap().join("dxvk/dxvk-2.7.1");
        for d in ["x64", "x32"] {
            fs::create_dir_all(dx.join(d)).unwrap();
            fs::write(dx.join(d).join("d3d9.dll"), d).unwrap();
        }
        let c = b.custom_launch_command(&g.id).unwrap();
        assert!(c.env.contains(&("WINEDLLOVERRIDES".into(), "mscoree,mshtml=;d3d9=n,b".into())), "{:?}", c.env);
        b.custom_prepare(&g).unwrap();
        let pfx = b.custom_prefix(&g);
        assert!(pfx.join("drive_c/windows/system32/d3d9.dll").is_file(), "wineboot made the prefix, DXVK copied in");
        b.custom_set(&g.id, "dxvk", "off").unwrap();
        assert!(!b.custom_launch_command(&g.id).unwrap().env.iter().any(|(k, v)| k == "WINEDLLOVERRIDES" && v.contains("d3d9")));
        let t = b.custom_tool_command(&g.id, "kill", &[]).unwrap();
        assert!(t.program.ends_with("wineserver") && t.args == vec!["-k"]);
    }

    #[cfg(unix)]
    #[test]
    fn install_flow_finds_the_program() {
        let (_t, b, games) = env();
        let setup = games.join("setup_cool_game_1.0.exe");
        fs::write(&setup, b"MZ").unwrap();
        let g = b.custom_install_begin(&setup, None, Some("umu")).unwrap();
        assert_eq!(g.title, "cool game 1 0");
        assert!(g.exe.is_none());
        let st = b.custom_status(&g);
        assert!(!st.ready && st.issues[0].message.contains("Choose the game's program"));
        let c = b.custom_run_command(&g.id, &setup).unwrap();
        assert!(c.env.iter().any(|(k, v)| k == "WINEPREFIX" && v.ends_with("custom/cool-game-1-0/pfx")));
        // What the installer left behind.
        let pf = b.custom_prefix(&g).join("drive_c/Program Files/Cool Game");
        fs::create_dir_all(&pf).unwrap();
        fs::write(pf.join("CoolGame.exe"), vec![0u8; 5000]).unwrap();
        fs::write(pf.join("unins000.exe"), vec![0u8; 9000]).unwrap();
        fs::write(pf.join("Launcher.exe"), vec![0u8; 100]).unwrap();
        fs::create_dir_all(b.custom_prefix(&g).join("drive_c/windows/system32")).unwrap();
        fs::write(b.custom_prefix(&g).join("drive_c/windows/system32/notepad.exe"), vec![0u8; 90000]).unwrap();
        let c = b.exe_candidates(&g);
        assert_eq!(c.iter().map(|p| p.file_name().unwrap().to_string_lossy().to_string()).collect::<Vec<_>>(), vec!["CoolGame.exe", "Launcher.exe"]);
        let g = b.custom_set(&g.id, "exe", &c[0].to_string_lossy()).unwrap();
        assert!(b.custom_status(&g).ready);
    }

    #[test]
    fn export_import_and_faugus() {
        let (t, b, games) = env();
        let exe = games.join("a.exe");
        fs::write(&exe, b"MZ").unwrap();
        let g = b.custom_add(&exe, Some("Alpha"), "", None).unwrap();
        b.custom_played(&g.id, 600).unwrap();
        let file = t.path().join("export.json");
        assert_eq!(b.custom_export(&file).unwrap(), 1);
        // Into another Beacon.
        let (_t2, b2, _) = env();
        let r = b2.custom_import(&file).unwrap();
        assert_eq!(r.added, vec!["alpha"]);
        assert_eq!(playtime::get(&b2.playtime_file(), "custom:alpha").seconds, 600);
        assert_eq!(b2.custom_import(&file).unwrap().skipped.len(), 1, "already there");

        // Faugus Launcher's games.json, with its icons folder.
        let fdir = b.paths.home.join(".config/faugus-launcher");
        fs::create_dir_all(fdir.join("icons")).unwrap();
        let wexe = games.join("Witcher/witcher3.exe");
        fs::create_dir_all(wexe.parent().unwrap()).unwrap();
        fs::write(&wexe, b"MZ").unwrap();
        fs::write(fdir.join("icons/the-witcher-3.png"), b"\x89PNG\r\n\x1a\nfake").unwrap();
        let faugus = serde_json::json!([
            {"gameid": "the-witcher-3", "title": "The Witcher 3", "path": wexe.to_string_lossy().replace(&*b.paths.home.to_string_lossy(), "~"),
             "prefix": "~/Faugus/the-witcher-3", "launch_arguments": "PROTON_LOG=1 DXVK_ASYNC=1", "game_arguments": "--skip-intro",
             "mangohud": true, "gamemode": "", "sdl_enabled": "", "protonfix": "292030", "runner": "UMU-Proton Latest",
             "playtime": 7200, "last_played": "2025-07-14T12:34:56.1"},
            {"gameid": "native", "title": "Native Thing", "path": "/usr/bin/true", "runner": "Linux-Native"}
        ]);
        fs::write(fdir.join("games.json"), faugus.to_string()).unwrap();
        assert_eq!(b.faugus_games_file().unwrap(), fdir.join("games.json"));
        let r = b.import_faugus().unwrap();
        assert_eq!(r.added, vec!["the-witcher-3"]);
        assert_eq!(r.skipped.len(), 1);
        let w = b.custom_game("the-witcher-3").unwrap();
        assert_eq!(w.exe.as_deref(), Some(wexe.as_path()));
        assert_eq!(w.prefix.as_deref(), Some(b.paths.home.join("Faugus/the-witcher-3").as_path()));
        assert_eq!((w.args.as_str(), w.mangohud, w.gamemode), ("--skip-intro", true, false));
        assert_eq!(w.umu_id.as_deref(), Some("umu-292030"));
        assert_eq!(w.runner.as_deref(), Some("umu"));
        assert_eq!(w.env.get("PROTON_LOG").map(String::as_str), Some("1"));
        assert_eq!(w.source.as_deref(), Some("faugus"));
        assert!(w.icon.unwrap().ends_with("art/the-witcher-3/icon.png"));
        let pt = playtime::get(&b.playtime_file(), "custom:the-witcher-3");
        assert_eq!((pt.seconds, pt.last_played), (7200, 1752496496));
        assert!(!b.custom_status(&b.custom_game("the-witcher-3").unwrap()).own_prefix);
        assert_eq!(b.import_faugus().unwrap().added.len(), 0, "imported once");
    }
}
