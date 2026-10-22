//! Beacon core: everything the launcher does, minus the UI.
//!
//! The flow for a launch is:
//! 1. `status()` – is the game folder there, is there a runner, is RotWK's
//!    BFME II present?
//! 2. `setup()` – first run, or after something changed: write the EA
//!    registry keys into the game's own prefix with regedit.
//! 3. `apply_options()` – write the chosen resolution into Options.ini.
//! 4. `launch_command()` / `launch()` – start the game (or a mod's launcher)
//!    through Wine, Proton or umu, logging to `<data>/logs/<game>.log`.
//!
//! On Windows (`Beacon::native`) there are no prefixes: the game's exe runs
//! directly, `setup` has nothing to do, and Options.ini is written under the
//! real `%APPDATA%`.

pub mod aio;
pub mod aio_launcher;
pub mod catalog;
pub mod community_launcher;
pub mod config;
/// Private build: Beacon's own downloads (see `downloads/mod.rs`).
pub mod downloads;
pub mod editions;
pub mod games;
pub mod installs;
pub mod media;
pub mod mods;
pub mod native;
pub mod options_ini;
pub mod registry;
pub mod report;
pub mod runner;
pub mod workshop;

use anyhow::{anyhow, bail, Context, Result};
use config::{split_args, Config, GameConfig, Paths};
use games::{FoundGame, GameId};
use mods::{ModEntry, ModKind};
use runner::{Command, Runner, RunnerSet};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use config::Resolution;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub level: Level,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameStatus {
    pub id: GameId,
    pub family: games::Family,
    pub title: String,
    pub short: String,
    pub path: Option<PathBuf>,
    pub installed: bool,
    pub runner: Option<Runner>,
    /// The game's own runner pick (`None` = default/automatic).
    pub runner_choice: Option<String>,
    pub has_cd_key: bool,
    pub resolution: Option<Resolution>,
    pub windowed: bool,
    pub extra_args: String,
    pub prefix_ready: bool,
    /// Wine virtual desktop on (non-BFME games).
    pub virtual_desktop: bool,
    /// Steam app id, when the game is installed in a Steam library.
    pub steam_appid: Option<u32>,
    pub via_steam: bool,
    /// Active edition's name (Other Games).
    pub edition: Option<String>,
    pub selected_mod: Option<String>,
    /// Extra exe the game ships (AotR's own launcher, for updates).
    pub tool_launcher: Option<PathBuf>,
    /// No errors (warnings are fine).
    pub ready: bool,
    pub issues: Vec<Issue>,
    // ---- 0.0.6: what a front end (Hearth) needs to show the right settings.
    /// BFME games: resolution goes into Options.ini, `-win` and `-mod` apply.
    #[serde(default)]
    pub sage: bool,
    /// The game's registry carries a CD key (BFME, BFME II, RotWK).
    #[serde(default)]
    pub wants_cd_key: bool,
    /// Another game that must be set up too (RotWK needs BFME II).
    #[serde(default)]
    pub requires: Option<GameId>,
    /// The player's virtual-desktop choice (`None` = the game's default).
    #[serde(default)]
    pub virtual_desktop_choice: Option<bool>,
    /// The game's default for the virtual desktop.
    #[serde(default)]
    pub virtual_desktop_default: bool,
    /// Whether `setup` has work to do (`None` when it can't tell yet: no folder or no runner).
    #[serde(default)]
    pub needs_setup: Option<bool>,
    /// The game's log file, and whether there's one yet.
    #[serde(default)]
    pub log_file: PathBuf,
    #[serde(default)]
    pub has_log: bool,
    /// The game has community versions (editions and/or setup steps).
    #[serde(default)]
    pub has_community: bool,
    /// The game's Steam store page, for games still sold there (only SWGB).
    #[serde(default)]
    pub store_url: Option<String>,
    // ---- 0.0.7
    /// The game's community launcher (only UaW so far): `{path, url}`, `path` = the launcher exe the player
    /// chose (`null` until then), `url` = where it's downloaded from. Left out for games without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_launcher: Option<community_launcher::LauncherRef>,
}

/// A catalog entry plus the id of the matching installed mod, if any.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogItem {
    #[serde(flatten)]
    pub entry: catalog::CatalogEntry,
    pub installed: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditionStatus {
    #[serde(flatten)]
    pub edition: editions::Edition,
    pub installed: bool,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepStatus {
    #[serde(flatten)]
    pub step: editions::Step,
    pub done: bool,
}

/// A game's community-version setup: editions to pick from and steps.
#[derive(Debug, Clone, Serialize)]
pub struct Community {
    pub editions: Vec<EditionStatus>,
    pub steps: Vec<StepStatus>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AotrBundle {
    pub bfme2: Option<PathBuf>,
    pub rotwk: Option<PathBuf>,
    /// AotR's own launcher (handles updates); may not work under Wine.
    pub launcher: Option<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SetupMarker {
    fingerprint: String,
}

#[derive(Clone)]
pub struct Beacon {
    pub paths: Paths,
    pub config: Config,
    pub runners: RunnerSet,
    /// Windows: games run directly (no Wine prefixes, no regedit import).
    pub native: bool,
}

/// What `install-from` will run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstall {
    /// The setup program.
    pub setup: PathBuf,
    /// Where a disc image was extracted (kept until the next install for that
    /// game: installers often start helpers that keep reading their files).
    pub extracted: Option<PathBuf>,
}

/// A game the BFME All-in-One launcher manages, for `beacon aio`.
#[derive(Debug, Clone, Serialize)]
pub struct AioGame {
    pub game: GameId,
    pub dir: PathBuf,
    /// The workshop package it's synced to, e.g. "Patch 2.22 (40)".
    pub active_patch: Option<String>,
    /// The launcher's active mod (its `mod.txt`), as it wrote it.
    pub active_mod: Option<String>,
}

impl Beacon {
    pub fn load() -> Result<Beacon> {
        let paths = Paths::from_env()?;
        let path_env = std::env::var("PATH").unwrap_or_default();
        Beacon::load_with(paths, &path_env)
    }

    pub fn load_with(paths: Paths, path_env: &str) -> Result<Beacon> {
        let config = Config::load(&paths.config_file)?;
        let native = cfg!(windows);
        let runners = if native { RunnerSet::native() } else { runner::discover(&paths.home, path_env) };
        Ok(Beacon { paths, config, runners, native })
    }

    /// Look for runners again (after installing a Proton build, say).
    pub fn refresh_runners(&mut self) {
        let path_env = std::env::var("PATH").unwrap_or_default();
        self.runners = if self.native { RunnerSet::native() } else { runner::discover(&self.paths.home, &path_env) };
    }

    /// A host path as the game sees it: a `Z:\…` path inside Wine, the path itself on Windows.
    pub fn game_path(&self, p: &Path) -> String {
        if self.native {
            p.to_string_lossy().into_owned()
        } else {
            registry::wine_path(p, false)
        }
    }

    pub fn save(&self) -> Result<()> {
        self.config.save(&self.paths.config_file)
    }

    pub fn game(&self, g: GameId) -> GameConfig {
        self.config.games.get(&g).cloned().unwrap_or_default()
    }

    pub fn game_mut(&mut self, g: GameId) -> &mut GameConfig {
        self.config.games.entry(g).or_default()
    }

    /// Point a game at a folder. Accepts the folder or the exe itself.
    pub fn set_path(&mut self, g: GameId, path: &Path) -> Result<()> {
        let mut dir = if path.is_file() { path.parent().unwrap_or(path).to_path_buf() } else { path.to_path_buf() };
        if g == GameId::Aotr {
            // Accept the install root ("AgeoftheRing") as well as its aotr folder.
            dir = games::aotr_game_dir(&dir).unwrap_or(dir);
        }
        if games::find_exe_in(&dir, g).is_none() {
            bail!("{} not found in {}", g.exe(), dir.display());
        }
        self.game_mut(g).path = Some(dir);
        Ok(())
    }

    pub fn scan_roots(&self) -> Vec<PathBuf> {
        let mut roots = games::default_scan_roots(&self.paths.home);
        // Games installed through Beacon live inside its own prefixes.
        roots.push(self.paths.data_dir.join("prefixes"));
        // Private build: games Beacon downloaded itself.
        roots.push(self.games_root());
        roots.extend(self.config.scan_roots.iter().cloned());
        roots
    }

    /// Wine prefixes worth reading the registry of (Beacon's own, `~/.wine`,
    /// Bottles, Lutris-style `~/Games/<x>`), plus any the caller names. Steam's
    /// compatdata isn't read: one prefix per Steam game would be slow.
    pub fn wine_prefixes(&self, extra_roots: &[PathBuf]) -> Vec<PathBuf> {
        if self.native {
            return Vec::new();
        }
        let h = &self.paths.home;
        let mut roots = vec![
            h.join(".wine"),
            self.paths.data_dir.join("prefixes"),
            h.join(".local/share/bottles/bottles"),
            h.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
            h.join("Games"),
        ];
        roots.extend(self.config.scan_roots.iter().cloned());
        roots.extend(extra_roots.iter().cloned());
        installs::find_prefixes(&roots)
    }

    /// Folders the registry points at: on Windows the real registry (the games'
    /// installers, EA/Origin, Steam, the All-in-One launcher); on Linux each
    /// Wine prefix's `system.reg` (an All-in-One launcher run under Wine, say).
    pub fn install_hints(&self, extra_roots: &[PathBuf]) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        if cfg!(windows) {
            for (_, dir) in installs::hints(&installs::windows_registry()) {
                out.push(PathBuf::from(dir));
            }
        }
        for pfx in self.wine_prefixes(extra_roots) {
            out.extend(installs::prefix_hints(&pfx).into_iter().map(|(_, d)| d));
        }
        let mut seen = Vec::new();
        out.retain(|d| d.is_dir() && !seen.contains(d) && {
            seen.push(d.clone());
            true
        });
        out
    }

    /// Scan for installs; games without a folder get the first one found.
    pub fn scan(&mut self, extra_roots: &[PathBuf]) -> Vec<FoundGame> {
        self.scan_with(extra_roots, &mut |_: usize, _: usize, _: &Path| {})
    }

    /// `scan`, telling `progress` about each folder before it's searched:
    /// (index, number of folders, folder).
    pub fn scan_with(&mut self, extra_roots: &[PathBuf], progress: &mut dyn FnMut(usize, usize, &Path)) -> Vec<FoundGame> {
        let mut roots: Vec<(PathBuf, usize)> = self.scan_roots().into_iter().map(|r| (r, 9)).collect();
        roots.extend(extra_roots.iter().cloned().map(|r| (r, 9)));
        // Install folders from the registry are searched a few levels deep.
        for d in self.install_hints(extra_roots) {
            if !roots.iter().any(|(r, _)| *r == d) {
                roots.push((d, 3));
            }
        }
        let mut found: Vec<FoundGame> = Vec::new();
        for (i, (root, depth)) in roots.iter().enumerate() {
            progress(i, roots.len(), root);
            for f in games::scan(std::slice::from_ref(root), *depth) {
                if !found.contains(&f) {
                    found.push(f);
                }
            }
        }
        found.sort_by(|a, b| a.game.cmp(&b.game).then(a.dir.cmp(&b.dir)));
        for f in &found {
            let cfg = self.game_mut(f.game);
            let current_ok = cfg.path.as_deref().map(|p| games::find_exe_in(p, f.game).is_some()).unwrap_or(false);
            if !current_ok {
                cfg.path = Some(f.dir.clone());
            }
        }
        found
    }

    pub fn runner_for(&self, g: GameId) -> Option<&Runner> {
        let cfg = self.game(g);
        cfg.runner
            .as_deref()
            .and_then(|id| self.runners.get(id))
            .or_else(|| self.config.default_runner.as_deref().and_then(|id| self.runners.get(id)))
            .or_else(|| self.runners.best())
    }

    fn game_dir(&self, g: GameId) -> Option<PathBuf> {
        self.game(g).path.filter(|p| games::find_exe_in(p, g).is_some())
    }

    pub fn status(&self, g: GameId) -> GameStatus {
        let cfg = self.game(g);
        let mut issues = Vec::new();
        let installed = self.game_dir(g).is_some();
        match &cfg.path {
            None => issues.push(Issue { level: Level::Error, message: "Game folder not set — scan or pick its folder.".into() }),
            Some(p) if !installed => issues.push(Issue { level: Level::Error, message: format!("{} not found in {}", g.exe(), p.display()) }),
            _ => {}
        }
        if let Some(req) = g.requires() {
            if self.game_dir(req).is_none() {
                issues.push(Issue { level: Level::Error, message: format!("{} needs {} installed and set up too.", g.short(), req.short()) });
            }
        }
        let runner = self.runner_for(g).cloned();
        if runner.is_none() {
            issues.push(Issue { level: Level::Error, message: "No Wine, Proton or umu-launcher found.".into() });
        }
        if g.wants_cd_key() && cfg.cd_key.as_deref().map(str::trim).unwrap_or("").is_empty() {
            issues.push(Issue { level: Level::Warn, message: "No CD key set. Add the key from your copy if the game asks you to reinstall.".into() });
        }
        if g == GameId::Uaw && installed && !self.game_dir(g).map(|d| d.join("xlive.dll").is_file() || d.join("XLive.dll").is_file()).unwrap_or(false) {
            issues.push(Issue {
                level: Level::Warn,
                message: "Games for Windows Live is gone: put the community xliveless xlive.dll in the game folder (campaign and skirmish only).".into(),
            });
        }
        let active = self.active_edition(g);
        if installed && !self.via_steam(g) {
            let eds = editions::editions(g);
            if eds.iter().any(|e| e.community) && !active.as_ref().map(|(e, _)| e.community).unwrap_or(false) {
                let any_community = self.edition_list(g).iter().any(|e| e.installed && e.edition.community);
                issues.push(Issue {
                    level: Level::Warn,
                    message: if any_community {
                        "Playing the original version — the community version is installed; pick it under Game settings → Version.".into()
                    } else {
                        "Community version not set up yet — Game settings → Community version walks you through it.".into()
                    },
                });
            }
        }
        let selected_mod = self.config.selected_mod.get(&g).cloned().filter(|id| self.find_mod(g, id).is_some());
        let needs_setup = if installed && runner.is_some() && !self.via_steam(g) { self.needs_setup(g).ok() } else { None };
        let log_file = self.paths.log_file(g);
        GameStatus {
            id: g,
            family: g.family(),
            title: g.title().into(),
            short: g.short().into(),
            path: cfg.path.clone(),
            installed,
            runner,
            runner_choice: cfg.runner.clone().filter(|id| self.runners.get(id).is_some()),
            has_cd_key: cfg.cd_key.as_deref().map(|k| !k.trim().is_empty()).unwrap_or(false),
            resolution: cfg.resolution,
            windowed: cfg.windowed,
            extra_args: cfg.extra_args.clone(),
            prefix_ready: !self.native && self.run_root(g).join("pfx/drive_c").is_dir(),
            virtual_desktop: self.virtual_desktop(g),
            steam_appid: self.steam_appid(g),
            via_steam: self.via_steam(g),
            edition: active.map(|(e, _)| e.name.to_string()),
            selected_mod,
            tool_launcher: if g == GameId::Aotr { self.aotr_bundled().launcher } else { None },
            ready: !issues.iter().any(|i| i.level == Level::Error),
            issues,
            sage: g.is_sage(),
            wants_cd_key: g.wants_cd_key(),
            requires: g.requires(),
            virtual_desktop_choice: if g.is_sage() || self.native { None } else { cfg.virtual_desktop },
            virtual_desktop_default: !self.native && !g.is_sage() && g.default_virtual_desktop(),
            needs_setup,
            has_log: log_file.is_file(),
            log_file,
            has_community: !editions::editions(g).is_empty() || !editions::steps(g).is_empty(),
            store_url: g.store_url().map(str::to_string),
            community_launcher: self.community_launcher_ref(g),
        }
    }

    /// Steam app id if this copy sits in a Steam library.
    pub fn steam_appid(&self, g: GameId) -> Option<u32> {
        let dir = self.game(g).path?;
        let in_steam = dir.components().any(|c| c.as_os_str().eq_ignore_ascii_case("steamapps"));
        g.steam_appid().filter(|_| in_steam)
    }

    pub fn via_steam(&self, g: GameId) -> bool {
        self.game(g).via_steam && self.steam_appid(g).is_some()
    }

    /// Wine's virtual desktop (never on Windows: there's no Wine).
    pub fn virtual_desktop(&self, g: GameId) -> bool {
        !self.native && !g.is_sage() && self.game(g).virtual_desktop.unwrap_or_else(|| g.default_virtual_desktop())
    }

    // ---- community versions -------------------------------------------

    pub fn edition_list(&self, g: GameId) -> Vec<EditionStatus> {
        let dir = self.game_dir(g);
        let active = self.active_edition(g).map(|(e, _)| e.id);
        editions::editions(g)
            .into_iter()
            .map(|edition| {
                let installed = dir.as_deref().and_then(|d| editions::locate(&edition, d)).is_some();
                let is_active = Some(edition.id) == active;
                EditionStatus { edition, installed, active: is_active }
            })
            .collect()
    }

    /// The edition a launch uses: the chosen one if installed, else the
    /// first installed community edition, else the first installed one.
    pub fn active_edition(&self, g: GameId) -> Option<(editions::Edition, Option<PathBuf>)> {
        let dir = self.game_dir(g)?;
        let found: Vec<(editions::Edition, Option<PathBuf>)> =
            editions::editions(g).into_iter().filter_map(|e| editions::locate(&e, &dir).map(|m| (e, m))).collect();
        let chosen = self.game(g).edition;
        let pick = chosen
            .as_deref()
            .and_then(|id| found.iter().position(|(e, _)| e.id == id))
            .or_else(|| found.iter().position(|(e, _)| e.community))
            .or(if found.is_empty() { None } else { Some(0) })?;
        Some(found[pick].clone())
    }

    pub fn community(&self, g: GameId) -> Community {
        let done = self.game(g).done_steps;
        Community {
            editions: self.edition_list(g),
            steps: editions::steps(g).into_iter().map(|step| StepStatus { done: done.iter().any(|d| d == step.id), step }).collect(),
        }
    }

    pub fn set_edition(&mut self, g: GameId, id: Option<&str>) -> Result<()> {
        if let Some(id) = id {
            let e = self.edition_list(g).into_iter().find(|e| e.edition.id == id).ok_or_else(|| anyhow!("{} has no edition {id}", g.short()))?;
            if !e.installed {
                bail!("{} isn't installed yet", e.edition.name);
            }
        }
        self.game_mut(g).edition = id.map(str::to_string);
        Ok(())
    }

    pub fn mark_step(&mut self, g: GameId, id: &str) {
        let done = &mut self.game_mut(g).done_steps;
        if !done.iter().any(|d| d == id) {
            done.push(id.to_string());
        }
    }

    /// Hand Beacon a community download for a step: an archive (extracted
    /// with 7z/bsdtar/unzip), an extracted folder, or a single file. It's
    /// merged into the game folder or added under Mods, with replaced files
    /// backed up to `<data>/backups/<game>/<time>/`. Returns a summary.
    pub fn add_files(&mut self, g: GameId, step_id: &str, source: &Path) -> Result<String> {
        self.add_files_with(g, step_id, source, &mut |_: &str| {})
    }

    /// `add_files`, telling `progress` what it's doing (extracting, copying).
    pub fn add_files_with(&mut self, g: GameId, step_id: &str, source: &Path, progress: &mut dyn FnMut(&str)) -> Result<String> {
        let step = editions::steps(g).into_iter().find(|s| s.id == step_id).ok_or_else(|| anyhow!("{} has no step {step_id}", g.short()))?;
        let dir = self.game_dir(g).ok_or_else(|| anyhow!("set {}'s game folder first", g.short()))?;
        if !source.exists() {
            bail!("{} not found", source.display());
        }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let backup = self.paths.data_dir.join("backups").join(g.slug()).join(stamp.to_string());
        let tmp = self.paths.data_dir.join("tmp").join(format!("{}-{stamp}", g.slug()));
        let is_archive = source.is_file()
            && source
                .extension()
                .map(|e| ["zip", "7z", "rar", "tar", "gz", "tgz", "xz", "bz2"].iter().any(|x| e.eq_ignore_ascii_case(x)))
                .unwrap_or(false);
        let root = if is_archive {
            progress(&format!("Extracting {}", source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
            editions::extract(source, &tmp)?;
            editions::content_root(&tmp)
        } else if source.is_dir() {
            editions::content_root(source)
        } else {
            source.to_path_buf()
        };
        progress(match step.action {
            editions::Action::Mods => "Adding it under the game's Mods folder",
            _ => "Copying files into the game folder (anything replaced is backed up first)",
        });

        let result = (|| -> Result<String> {
            match step.action {
                editions::Action::Merge => {
                    if root.is_file() {
                        let name = root.file_name().unwrap();
                        let target = dir.join(name);
                        if target.is_file() {
                            std::fs::create_dir_all(&backup)?;
                            std::fs::copy(&target, backup.join(name))?;
                        }
                        std::fs::copy(&root, &target)?;
                        return Ok(format!("Copied {} into the game folder", name.to_string_lossy()));
                    }
                    let n = editions::merge_dir(&editions::merge_source(&root, &dir), &dir, &backup)?;
                    Ok(format!("Merged {n} files into the game folder"))
                }
                editions::Action::Mods => {
                    if !root.is_dir() {
                        bail!("pick the mod's archive or folder");
                    }
                    let mods = dir.join("Mods");
                    let target = mods.join(step.mods_name);
                    let n = editions::merge_dir(&root, &target, &backup)?;
                    Ok(format!("Added {} under Mods ({n} files)", step.title))
                }
                _ => bail!("\"{}\" isn't installed from files", step.title),
            }
        })();
        let _ = std::fs::remove_dir_all(&tmp);
        let msg = result?;
        self.mark_step(g, step.id);
        // Adding a community edition makes it the one that launches.
        if editions::editions(g).iter().any(|e| e.id == step.id) {
            let _ = self.set_edition(g, Some(step.id));
        }
        Ok(msg)
    }

    pub fn statuses(&self) -> Vec<GameStatus> {
        GameId::ALL.into_iter().map(|g| self.status(g)).collect()
    }

    // ---- mods ---------------------------------------------------------

    pub fn mods(&self, g: GameId) -> Vec<ModEntry> {
        let mut all = mods::scan_library(&self.paths.mods_dir(), g);
        all.extend(self.config.linked_mods.iter().filter(|m| m.game == g).cloned());
        all
    }

    pub fn find_mod(&self, g: GameId, id: &str) -> Option<ModEntry> {
        self.mods(g).into_iter().find(|m| m.id == id)
    }

    pub fn link_mod(&mut self, g: GameId, path: &Path, name: Option<&str>) -> Result<ModEntry> {
        let m = mods::inspect(g, path, name).ok_or_else(|| anyhow!("{} doesn't look like a mod (.big, folder of .big files, or launcher .exe)", path.display()))?;
        if self.find_mod(g, &m.id).is_some() {
            bail!("a mod called \"{}\" already exists for {}", m.name, g.short());
        }
        self.config.linked_mods.push(m.clone());
        Ok(m)
    }

    /// Known mods for a game, marked with what's already installed.
    pub fn catalog(&self, g: GameId) -> Vec<CatalogItem> {
        let installed = self.mods(g);
        catalog::for_game(g)
            .into_iter()
            .map(|entry| {
                let installed = installed.iter().find(|m| catalog::matches(&entry, m)).map(|m| m.id.clone());
                CatalogItem { entry, installed }
            })
            .collect()
    }

    /// Search the usual places for known mods already on disk and link
    /// the ones Beacon doesn't have yet. Returns what was added.
    pub fn find_mods(&mut self, extra_roots: &[PathBuf]) -> Vec<ModEntry> {
        self.find_mods_with(extra_roots, &mut |_: usize, _: usize, _: &Path| {})
    }

    /// `find_mods`, telling `progress` about each folder before it's searched.
    pub fn find_mods_with(&mut self, extra_roots: &[PathBuf], progress: &mut dyn FnMut(usize, usize, &Path)) -> Vec<ModEntry> {
        let game_dirs: Vec<PathBuf> = GameId::ALL.into_iter().filter_map(|g| self.game_dir(g)).collect();
        let mut roots = catalog::default_mod_roots(&self.paths.home, &game_dirs);
        roots.extend(extra_roots.iter().cloned());
        let mut discovered: Vec<ModEntry> = Vec::new();
        for (i, root) in roots.iter().enumerate() {
            progress(i, roots.len(), root);
            for m in catalog::discover(std::slice::from_ref(root), 4) {
                if !discovered.iter().any(|d| d.path == m.path) {
                    discovered.push(m);
                }
            }
        }
        // Mods the All-in-One launcher's workshop put next to the games.
        let dirs: Vec<(GameId, PathBuf)> = GameId::ALL.into_iter().filter_map(|g| self.game_dir(g).map(|d| (g, d))).collect();
        for wm in aio::workshop_mods(&dirs, &self.aio_appdatas()) {
            if let Some(m) = mods::inspect(wm.game, &wm.path, Some(&wm.name)) {
                if !discovered.iter().any(|d| d.path == m.path) {
                    discovered.push(m);
                }
            }
        }
        let mut added = Vec::new();
        for m in discovered {
            let known = GameId::ALL.into_iter().flat_map(|g| self.mods(g)).any(|k| k.path == m.path || k.id == m.id);
            if !known {
                self.config.linked_mods.push(m.clone());
                added.push(m);
            }
        }
        added
    }

    pub fn unlink_mod(&mut self, id: &str) -> bool {
        let before = self.config.linked_mods.len();
        self.config.linked_mods.retain(|m| m.id != id);
        self.config.selected_mod.retain(|_, v| v != id);
        before != self.config.linked_mods.len()
    }

    pub fn select_mod(&mut self, g: GameId, id: Option<&str>) -> Result<()> {
        match id {
            None => {
                self.config.selected_mod.remove(&g);
            }
            Some(id) => {
                self.find_mod(g, id).ok_or_else(|| anyhow!("no mod {id} for {}", g.short()))?;
                self.config.selected_mod.insert(g, id.to_string());
            }
        }
        Ok(())
    }

    // ---- prefix setup -------------------------------------------------

    /// The registry file for a game's prefix. RotWK's also carries BFME II's
    /// keys, since the expansion reads the base game's install.
    pub fn registry_for(&self, g: GameId) -> Result<String> {
        if g == GameId::Aotr {
            return self.aotr_registry();
        }
        if !g.is_sage() {
            // Non-BFME games (Conquest) get their registry from their own
            // installer; an empty import just creates the prefix.
            self.game_dir(g).ok_or_else(|| anyhow!("{} folder not set or missing {}", g.short(), g.exe()))?;
            return Ok("REGEDIT4\r\n".into());
        }
        let mut entries = Vec::new();
        let mut owned: Vec<(GameId, PathBuf, Option<String>)> = Vec::new();
        for game in g.requires().into_iter().chain([g]) {
            let dir = self.game_dir(game).ok_or_else(|| anyhow!("{} folder not set or missing {}", game.short(), game.exe()))?;
            owned.push((game, dir, self.game(game).cd_key));
        }
        for (game, dir, key) in &owned {
            entries.push(registry::RegGame { game: *game, install_dir: dir, cd_key: key.as_deref() });
        }
        Ok(registry::build_reg(&entries, &self.config.language))
    }

    /// AotR Standalone bundles BFME II and RotWK. Its own launcher writes
    /// the registry while it runs; since Beacon starts the game exe directly
    /// (the launcher is unreliable under Wine), Beacon writes the equivalent:
    /// BFME II and RotWK keys pointing at the bundled copies, with RotWK's
    /// user-data folder set to "Age of the Ring" so settings and saves land
    /// where AotR expects them.
    fn aotr_registry(&self) -> Result<String> {
        let dir = self.game_dir(GameId::Aotr).ok_or_else(|| anyhow!("Age of the Ring folder not set"))?;
        let bundled = self.aotr_bundled();
        let rotwk_dir = bundled.rotwk.unwrap_or_else(|| dir.clone());
        let key = self.game(GameId::Aotr).cd_key;
        let mut entries = Vec::new();
        if let Some(b2) = &bundled.bfme2 {
            entries.push(registry::RegGame { game: GameId::Bfme2, install_dir: b2, cd_key: key.as_deref() });
        }
        entries.push(registry::RegGame { game: GameId::Aotr, install_dir: &rotwk_dir, cd_key: key.as_deref() });
        Ok(registry::build_reg(&entries, &self.config.language))
    }

    /// The base-game copies and launcher inside an AotR install.
    pub fn aotr_bundled(&self) -> AotrBundle {
        let mut out = AotrBundle::default();
        let Some(dir) = self.game_dir(GameId::Aotr) else { return out };
        let Some(root) = games::aotr_root(&dir) else { return out };
        for e in walkdir::WalkDir::new(&root).max_depth(3).into_iter().flatten() {
            if !e.file_type().is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            let parent = e.path().parent().unwrap().to_path_buf();
            if name == "lotrbfme2.exe" && out.bfme2.is_none() {
                out.bfme2 = Some(parent);
            } else if name == "lotrbfme2ep1.exe" && parent != dir && out.rotwk.is_none() {
                out.rotwk = Some(parent);
            } else if name.ends_with(".exe") && name.contains("launcher") && out.launcher.is_none() {
                out.launcher = Some(e.into_path());
            }
        }
        out
    }

    fn require_runner(&self, g: GameId) -> Result<&Runner> {
        self.runner_for(g).ok_or_else(|| anyhow!("no Wine, Proton or umu-launcher found"))
    }

    fn command(&self, g: GameId, target: &str, args: &[String], cwd: &Path) -> Result<Command> {
        let runner = self.require_runner(g)?;
        let mut cmd = runner::command_for(runner, self.runners.umu.as_deref(), &self.paths.home, &self.run_root(g), target, args, cwd);
        for (k, v) in &self.game(g).env {
            cmd.env.retain(|(ek, _)| ek != k);
            cmd.env.push((k.clone(), v.clone()));
        }
        Ok(cmd)
    }

    fn setup_fingerprint(&self, g: GameId, reg: &str) -> Result<String> {
        Ok(registry::fingerprint(&format!("{}\n{reg}", self.require_runner(g)?.id)))
    }

    pub fn needs_setup(&self, g: GameId) -> Result<bool> {
        if self.native || self.launcher_owns(g) {
            // No prefix of Beacon's: the game's installer (or the All-in-One launcher, in its own prefix) wrote its
            // registry, with the CD key it made up; Beacon's would have none.
            return Ok(false);
        }
        let reg = self.registry_for(g)?;
        let fp = self.setup_fingerprint(g, &reg)?;
        let marker: Option<SetupMarker> = std::fs::read_to_string(self.paths.prefix_root(g).join("beacon-setup.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        Ok(marker.map(|m| m.fingerprint != fp).unwrap_or(true) || !self.paths.prefix_root(g).join("pfx").is_dir())
    }

    /// The regedit command `setup()` would run, after writing the .reg file.
    pub fn setup_command(&self, g: GameId) -> Result<Command> {
        if self.native {
            bail!("on Windows the game's own installer sets it up; there's no prefix");
        }
        let reg = self.registry_for(g)?;
        let reg_path = self.paths.reg_file(g);
        std::fs::create_dir_all(reg_path.parent().unwrap())?;
        std::fs::write(&reg_path, &reg)?;
        let root = self.paths.prefix_root(g);
        std::fs::create_dir_all(&root)?;
        self.command(g, "regedit", &["/S".into(), registry::wine_path(&reg_path, false)], &root)
    }

    /// Create the prefix if needed and import the registry keys.
    /// Returns true if it did any work.
    pub fn setup(&self, g: GameId, force: bool) -> Result<bool> {
        if self.native || self.launcher_owns(g) || self.via_steam(g) || (!force && !self.needs_setup(g)?) {
            return Ok(false);
        }
        let cmd = self.setup_command(g)?;
        let log = self.open_log(g, &cmd, "setup")?;
        let status = cmd
            .to_std()
            .stdout(log.try_clone()?)
            .stderr(log)
            .status()
            .with_context(|| format!("starting {}", cmd.program.display()))?;
        if !status.success() {
            bail!("prefix setup failed ({status}); see {}", self.paths.log_file(g).display());
        }
        let reg = self.registry_for(g)?;
        let marker = SetupMarker { fingerprint: self.setup_fingerprint(g, &reg)? };
        std::fs::write(self.paths.prefix_root(g).join("beacon-setup.json"), serde_json::to_string(&marker)?)?;
        Ok(true)
    }

    /// Before running an installer or patch in the game's prefix: set the
    /// prefix up when the game is there to set up. A game that's still being
    /// installed has no folder yet (its registry can't be written), so its
    /// prefix is simply created by the installer's own run.
    pub fn prepare_prefix(&self, g: GameId) -> Result<bool> {
        if self.native {
            return Ok(false);
        }
        std::fs::create_dir_all(self.paths.prefix_root(g))?;
        if self.via_steam(g) || self.registry_for(g).is_err() {
            return Ok(false);
        }
        self.setup(g, false)
    }

    /// Write the chosen resolution into Options.ini inside the prefix.
    pub fn apply_options(&self, g: GameId) -> Result<Option<PathBuf>> {
        if g == GameId::Uaw && !self.native {
            return self.fix_uaw_dx10();
        }
        if !g.is_sage() {
            return Ok(None);
        }
        let Some(res) = self.game(g).resolution else { return Ok(None) };
        let dir = if self.native {
            // Windows: the real %APPDATA%, under the leaf the game's registry names
            // (the All-in-One launcher's differ from EA's).
            let Some(appdata) = self.paths.appdata.clone() else { return Ok(None) };
            let leaf = if cfg!(windows) { installs::user_data_leaf(&installs::windows_registry(), g) } else { None };
            appdata.join(leaf.unwrap_or_else(|| g.user_data_leaf().to_string()))
        } else {
            let pfx = self.run_root(g).join("pfx");
            if !pfx.join("drive_c").is_dir() {
                return Ok(None);
            }
            // The launcher's prefix names its own leaf (BFME II's and RotWK's differ from EA's).
            let leaf = if self.launcher_owns(g) { installs::user_data_leaf(&installs::prefix_registry(&pfx), g) } else { None };
            options_ini::appdata_dir(&pfx).join(leaf.unwrap_or_else(|| g.user_data_leaf().to_string()))
        };
        std::fs::create_dir_all(&dir)?;
        let file = dir.join("Options.ini");
        let current = std::fs::read_to_string(&file).unwrap_or_default();
        std::fs::write(&file, options_ini::set_value(&current, "Resolution", &res.ini_value()))?;
        Ok(Some(file))
    }

    /// UaW's DX10 renderer crashes under Wine/Proton. The game writes
    /// `DX10Enabled` to the registry on its first (crashing) run; flip it to
    /// 0 so later runs use DX9. Runs before each launch, while Wine is down.
    fn fix_uaw_dx10(&self) -> Result<Option<PathBuf>> {
        let reg = self.paths.prefix_root(GameId::Uaw).join("pfx/user.reg");
        let Ok(text) = std::fs::read_to_string(&reg) else { return Ok(None) };
        let on = "\"DX10Enabled\"=dword:00000001";
        if !text.contains(on) {
            return Ok(None);
        }
        std::fs::write(&reg, text.replace(on, "\"DX10Enabled\"=dword:00000000"))?;
        Ok(Some(reg))
    }

    // ---- launching ----------------------------------------------------

    /// What a launch would run. `mod_id = None` uses the saved selection;
    /// `Some("")` forces vanilla.
    pub fn launch_command(&self, g: GameId, mod_id: Option<&str>) -> Result<Command> {
        let status = self.status(g);
        if let Some(err) = status.issues.iter().find(|i| i.level == Level::Error) {
            bail!("{}", err.message);
        }
        let dir = self.game_dir(g).unwrap();
        let cfg = self.game(g);
        let chosen = match mod_id {
            Some("") => None,
            Some(id) => Some(self.find_mod(g, id).ok_or_else(|| anyhow!("no mod {id} for {}", g.short()))?),
            None => status.selected_mod.as_deref().and_then(|id| self.find_mod(g, id)),
        };

        if let Some(m) = chosen.as_ref().filter(|m| m.kind == ModKind::Launcher) {
            let cwd = m.path.parent().unwrap_or(&dir).to_path_buf();
            return self.command(g, &m.path.to_string_lossy(), &m.args, &cwd);
        }

        if self.via_steam(g) {
            let id = self.steam_appid(g).unwrap().to_string();
            let path_env = std::env::var("PATH").unwrap_or_default();
            let opener = if self.native { "explorer" } else { "xdg-open" };
            return Ok(match runner::which_in("steam", &path_env) {
                Some(steam) => Command { program: steam, args: vec!["-applaunch".into(), id], env: vec![], cwd: dir },
                None => Command { program: opener.into(), args: vec![format!("steam://rungameid/{id}")], env: vec![], cwd: dir },
            });
        }
        let mut args = Vec::new();
        if let Some(m) = chosen.as_ref().filter(|_| !g.is_sage()) {
            bail!("{} doesn't load mods with -mod; install \"{}\" into the game folder or add its own launcher instead", g.short(), m.name);
        }
        if cfg.windowed && g.is_sage() {
            args.push("-win".to_string());
        }
        if let Some(m) = &chosen {
            args.push("-mod".into());
            args.push(self.game_path(&m.path));
            args.extend(m.args.iter().cloned());
        }
        args.extend(split_args(&cfg.extra_args));
        let mut exe = games::find_exe_in(&dir, g).unwrap();
        if let Some((edition, mod_dir)) = self.active_edition(g) {
            if let Some(x) = edition.exes.iter().find_map(|x| games::find_exe_in_named(&dir, x)) {
                exe = x;
            }
            if let Some(m) = mod_dir {
                args.insert(0, editions::modpath_arg_for(&dir, &m, self.native));
            }
        }
        if self.virtual_desktop(g) {
            let size = cfg.resolution.unwrap_or(Resolution { width: 1280, height: 1024 });
            let mut wrapped = vec![format!("/desktop=Beacon,{size}"), registry::wine_path(&exe, false)];
            wrapped.extend(args);
            return self.command(g, "explorer", &wrapped, &dir);
        }
        self.command(g, &exe.to_string_lossy(), &args, &dir)
    }

    /// Run any Windows exe (a patch, a mod installer) in the game's prefix.
    pub fn run_in_prefix_command(&self, g: GameId, exe: &Path) -> Result<Command> {
        if !exe.is_file() {
            bail!("{} not found", exe.display());
        }
        let cwd = exe.parent().unwrap_or(Path::new("/")).to_path_buf();
        self.command(g, &exe.to_string_lossy(), &[], &cwd)
    }

    fn open_log(&self, g: GameId, cmd: &Command, what: &str) -> Result<std::fs::File> {
        use std::io::Write;
        let path = self.paths.log_file(g);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let mut f = std::fs::OpenOptions::new().create(true).append(what.starts_with("launch")).write(true).open(&path)?;
        if what == "setup" {
            f.set_len(0)?;
        }
        writeln!(f, "== beacon {what}: {}", cmd.display())?;
        Ok(f)
    }

    /// Set up if needed, apply options, and start. The caller owns the child.
    pub fn launch(&self, g: GameId, mod_id: Option<&str>) -> Result<std::process::Child> {
        let cmd = self.launch_command(g, mod_id)?;
        self.setup(g, false)?;
        self.apply_options(g)?;
        self.spawn_logged(g, &cmd)
    }

    pub fn spawn_logged(&self, g: GameId, cmd: &Command) -> Result<std::process::Child> {
        let log = self.open_log(g, cmd, "launch")?;
        match cmd.to_std().stdout(log.try_clone()?).stderr(log).spawn() {
            Ok(child) => Ok(child),
            // Windows: installers marked "requires administrator" go through the UAC prompt.
            #[cfg(windows)]
            Err(e) if e.raw_os_error() == Some(native::ERROR_ELEVATION_REQUIRED) => {
                let _ = self.open_log(g, cmd, "launch (with administrator rights)");
                native::spawn_elevated(&self.paths.data_dir, cmd)
            }
            Err(e) => Err(e).with_context(|| format!("starting {}", cmd.program.display())),
        }
    }

    // ---- install from your own copy ------------------------------------

    /// Get a game's installer ready from the player's own copy: an installer
    /// .exe as it is, a disc folder's setup program, or a disc image extracted
    /// into `<data>/tmp/install-<game>-<time>/` first. Run the result with
    /// `run_in_prefix_command` + `spawn_logged` (in the prefix on Linux,
    /// directly on Windows), then `scan` to pick up the install.
    pub fn prepare_install(&self, g: GameId, source: &Path, progress: &mut dyn FnMut(&str)) -> Result<PreparedInstall> {
        let tmp = self.paths.data_dir.join("tmp");
        match media::classify(source)? {
            media::Source::Installer(exe) => Ok(PreparedInstall { setup: exe, extracted: None }),
            media::Source::Folder(dir) => {
                let setup = media::find_setup(&dir).ok_or_else(|| anyhow!("no setup program found in {}; pick the installer .exe itself", dir.display()))?;
                Ok(PreparedInstall { setup, extracted: None })
            }
            media::Source::Image(iso) => {
                // Earlier extracted copies for this game are done with by now.
                let prefix = format!("install-{}-", g.slug());
                if let Ok(rd) = std::fs::read_dir(&tmp) {
                    for e in rd.flatten() {
                        if e.file_name().to_string_lossy().starts_with(&prefix) && e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                            let _ = std::fs::remove_dir_all(e.path());
                        }
                    }
                }
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let dest = tmp.join(format!("{prefix}{stamp}"));
                progress(&format!("Extracting {} (this can take a few minutes)", iso.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                media::extract_image(&iso, &dest)?;
                progress("Looking for the setup program");
                let setup = media::find_setup(&dest).ok_or_else(|| anyhow!("no setup program on the disc image; extracted to {}", dest.display()))?;
                Ok(PreparedInstall { setup, extracted: Some(dest) })
            }
        }
    }

    // ---- BFME All-in-One launcher ---------------------------------------

    /// Folders that may be the All-in-One launcher's `%APPDATA%`: the real
    /// one on Windows; in Wine, each prefix's user AppData (Beacon's own
    /// prefixes and the prefixes the games sit in).
    pub fn aio_appdatas(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        if self.native {
            out.extend(self.paths.appdata.clone());
        }
        let mut pfxs = self.wine_prefixes(&[]);
        // The launcher's own prefix (Beacon installs it there), even before its registry exists.
        if !self.native && self.launcher_prefix().join("drive_c").is_dir() {
            pfxs.push(self.launcher_prefix());
        }
        pfxs.extend(GameId::ALL.into_iter().filter_map(|g| self.game_dir(g)).filter_map(|d| aio::prefix_of(&d)));
        for p in pfxs {
            let a = options_ini::appdata_dir(&p);
            if !out.contains(&a) {
                out.push(a);
            }
        }
        out
    }

    /// The installed BFME games the All-in-One launcher manages.
    pub fn aio_games(&self) -> Vec<AioGame> {
        let appdatas = self.aio_appdatas();
        [GameId::Bfme1, GameId::Bfme2, GameId::Rotwk]
            .into_iter()
            .filter_map(|g| {
                let dir = self.game_dir(g)?;
                aio::manages(&dir, g, &appdatas).then(|| AioGame {
                    game: g,
                    active_patch: appdatas.iter().find_map(|a| aio::active_patch(a, g)),
                    active_mod: aio::active_mod(&dir),
                    dir,
                })
            })
            .collect()
    }

    // ---- workshop --------------------------------------------------------

    /// The workshop's fan content for a game, marked with what's installed.
    pub fn workshop_list(&self, http: &dyn workshop::Http, g: GameId, keyword: &str, page: u32) -> Result<(Vec<workshop::Item>, usize)> {
        let (mut items, hidden) = workshop::list(http, g, keyword, page)?;
        self.mark_workshop_installed(g, &mut items);
        Ok((items, hidden))
    }

    /// Fill in `installed` (the Beacon mod id) for workshop items already in the mods library.
    pub fn mark_workshop_installed(&self, g: GameId, items: &mut [workshop::Item]) {
        let have = workshop::installed(&self.paths.mods_dir(), g);
        for it in items.iter_mut() {
            it.installed = have
                .iter()
                .find(|(guid, _)| guid == &it.id)
                .and_then(|(_, dir)| mods::inspect(g, dir, None))
                .map(|m| m.id);
        }
    }

    /// Download a workshop entry (fan content only) into the mods library.
    pub fn workshop_install(
        &self,
        http: &dyn workshop::Http,
        g: GameId,
        id: &str,
        progress: &mut dyn FnMut(Option<u8>, &str),
    ) -> Result<(workshop::Installed, Option<ModEntry>)> {
        workshop::game_number(g)?;
        progress(Some(1), "Asking the workshop for the package");
        let v = workshop::fetch(http, id)?;
        let pkg = workshop::parse_package(&v, &workshop::language_code(&self.config.language))?;
        if pkg.item.game != g {
            bail!("{} is for {}, not {}", pkg.item.name, pkg.item.game.short(), g.short());
        }
        let done = workshop::install(http, &pkg, &self.paths.mods_dir(), &self.paths.data_dir.join("tmp"), progress)?;
        let m = mods::inspect(g, &done.dir, None).map(|mut m| {
            m.in_library = true;
            m
        });
        Ok((done, m))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Env {
        _t: tempfile::TempDir,
        beacon: Beacon,
        games: PathBuf,
    }

    /// A fake system: three games, a GE-Proton build, and a stub `wine`
    /// that records its arguments so setup can run for real.
    fn env() -> Env {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let games = home.join("Games");
        for (d, exe) in [("BFME1", "lotrbfme.exe"), ("BFME2", "lotrbfme2.exe"), ("RotWK", "lotrbfme2ep1.exe")] {
            fs::create_dir_all(games.join(d)).unwrap();
            fs::write(games.join(d).join(exe), b"").unwrap();
        }
        let bin = t.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        // Stub wine: create the prefix like the real thing and log args.
        fs::write(&wine, "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/users/tester\"\nprintf '%s\\n' \"$*\" >> \"$WINEPREFIX/../calls.txt\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let paths = Paths::under(&t.path().join("beacon"), &home);
        let beacon = Beacon::load_with(paths, &bin.to_string_lossy()).unwrap();
        Env { _t: t, beacon, games }
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn scan_assigns_and_status() {
        let mut e = env();
        assert!(!e.beacon.status(GameId::Bfme2).ready);
        let found = e.beacon.scan(&[]);
        assert_eq!(found.len(), 3);
        assert!(!e.beacon.status(GameId::Aotr).installed);
        let st = e.beacon.status(GameId::Rotwk);
        assert!(st.ready, "{:?}", st.issues);
        assert_eq!(st.runner.unwrap().id, "wine:system");
        assert!(st.issues.iter().any(|i| i.level == Level::Warn));
    }

    #[test]
    fn rotwk_needs_bfme2() {
        let mut e = env();
        e.beacon.set_path(GameId::Rotwk, &e.games.join("RotWK")).unwrap();
        let st = e.beacon.status(GameId::Rotwk);
        assert!(!st.ready);
        assert!(e.beacon.launch_command(GameId::Rotwk, None).is_err());
        e.beacon.set_path(GameId::Bfme2, &e.games.join("BFME2/lotrbfme2.exe")).unwrap();
        assert!(e.beacon.status(GameId::Rotwk).ready);
        let reg = e.beacon.registry_for(GameId::Rotwk).unwrap();
        assert!(reg.contains("The Battle for Middle-earth II]"));
        assert!(reg.contains("The Rise of the Witch-king]"));
    }

    #[test]
    fn set_path_rejects_wrong_folder() {
        let mut e = env();
        assert!(e.beacon.set_path(GameId::Bfme1, &e.games.join("BFME2")).is_err());
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn launch_args_with_mods() {
        let mut e = env();
        e.beacon.scan(&[]);
        let g = GameId::Rotwk;
        e.beacon.game_mut(g).windowed = true;
        e.beacon.game_mut(g).extra_args = "-noshellmap".into();
        let mods = e.beacon.paths.mods_dir().join("rotwk");
        fs::create_dir_all(mods.join("Age of the Ring")).unwrap();
        fs::write(mods.join("Age of the Ring/aotr.big"), b"").unwrap();
        fs::create_dir_all(mods.join("Edain")).unwrap();
        fs::write(mods.join("Edain/EdainLauncher.exe"), b"").unwrap();

        let vanilla = e.beacon.launch_command(g, None).unwrap();
        assert!(vanilla.args[0].ends_with("lotrbfme2ep1.exe"));
        assert_eq!(&vanilla.args[1..], &["-win", "-noshellmap"]);
        assert_eq!(vanilla.cwd, e.games.join("RotWK"));

        e.beacon.select_mod(g, Some("rotwk:age-of-the-ring")).unwrap();
        let aotr = e.beacon.launch_command(g, None).unwrap();
        assert_eq!(aotr.args[2], "-mod");
        assert!(aotr.args[3].starts_with("Z:\\") && aotr.args[3].ends_with("\\Age of the Ring"));

        let forced_vanilla = e.beacon.launch_command(g, Some("")).unwrap();
        assert!(!forced_vanilla.args.contains(&"-mod".to_string()));

        let edain = e.beacon.launch_command(g, Some("rotwk:edain")).unwrap();
        assert!(edain.args[0].ends_with("EdainLauncher.exe"));
        assert_eq!(edain.args.len(), 1);
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn setup_runs_once_and_writes_options() {
        let mut e = env();
        e.beacon.scan(&[]);
        let g = GameId::Bfme2;
        e.beacon.game_mut(g).cd_key = Some("KEY-FROM-BOX".into());
        e.beacon.game_mut(g).resolution = Resolution::parse("1920x1080");
        assert!(e.beacon.needs_setup(g).unwrap());
        assert!(e.beacon.setup(g, false).unwrap());
        assert!(!e.beacon.needs_setup(g).unwrap());
        assert!(!e.beacon.setup(g, false).unwrap(), "second setup should be skipped");

        let calls = fs::read_to_string(e.beacon.paths.prefix_root(g).join("calls.txt")).unwrap();
        assert!(calls.starts_with("regedit /S Z:\\"), "{calls}");
        let reg = fs::read_to_string(e.beacon.paths.reg_file(g)).unwrap();
        assert!(reg.contains("@=\"KEY-FROM-BOX\""));

        let ini = e.beacon.apply_options(g).unwrap().unwrap();
        assert!(ini.ends_with("users/tester/AppData/Roaming/My The Battle for Middle-earth II Files/Options.ini"));
        assert_eq!(fs::read_to_string(ini).unwrap(), "Resolution = 1920 1080\r\n");

        // Changing the key invalidates the setup.
        e.beacon.game_mut(g).cd_key = Some("OTHER".into());
        assert!(e.beacon.needs_setup(g).unwrap());
    }

    #[test]
    fn per_game_env_overrides() {
        let mut e = env();
        e.beacon.scan(&[]);
        e.beacon.game_mut(GameId::Bfme1).env.insert("WINEDEBUG".into(), "+seh".into());
        let c = e.beacon.launch_command(GameId::Bfme1, None).unwrap();
        assert_eq!(c.env.iter().filter(|(k, _)| k == "WINEDEBUG").count(), 1);
        assert!(c.env.contains(&("WINEDEBUG".into(), "+seh".into())));
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn aotr_standalone_setup() {
        let mut e = env();
        let root = e.games.join("AgeoftheRing");
        for (d, exe) in [("aotr", "lotrbfme2ep1.exe"), ("BFME2", "lotrbfme2.exe"), ("RotWK", "lotrbfme2ep1.exe")] {
            fs::create_dir_all(root.join(d)).unwrap();
            fs::write(root.join(d).join(exe), b"").unwrap();
        }
        fs::write(root.join("AotR Launcher.exe"), b"").unwrap();
        e.beacon.scan(&[]);
        // The bundled RotWK copy must not replace the real RotWK install.
        assert_eq!(e.beacon.game(GameId::Rotwk).path.unwrap(), e.games.join("RotWK"));
        assert_eq!(e.beacon.game(GameId::Aotr).path.unwrap(), root.join("aotr"));
        let st = e.beacon.status(GameId::Aotr);
        assert!(st.ready, "{:?}", st.issues);
        assert!(st.issues.is_empty(), "AotR needs no CD key: {:?}", st.issues);
        assert!(st.tool_launcher.unwrap().ends_with("AotR Launcher.exe"));

        let reg = e.beacon.registry_for(GameId::Aotr).unwrap();
        assert!(reg.contains(r#"\\AgeoftheRing\\BFME2\\""#), "{reg}");
        assert!(reg.contains(r#"\\AgeoftheRing\\RotWK\\""#), "{reg}");
        assert!(reg.contains("\"UserDataLeafName\"=\"Age of the Ring\""));

        let cmd = e.beacon.launch_command(GameId::Aotr, None).unwrap();
        assert!(cmd.args[0].ends_with("AgeoftheRing/aotr/lotrbfme2ep1.exe"));
        assert_eq!(cmd.cwd, root.join("aotr"));
        assert!(cmd.env.iter().any(|(k, v)| k == "WINEPREFIX" && v.ends_with("prefixes/aotr/pfx")));

        // Pointing at the install root works too.
        e.beacon.set_path(GameId::Aotr, &root).unwrap();
        assert_eq!(e.beacon.game(GameId::Aotr).path.unwrap(), root.join("aotr"));
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn conquest_runs_plain() {
        let mut e = env();
        let dir = e.games.join("LOTR Conquest");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Conquest.exe"), b"").unwrap();
        e.beacon.scan(&[]);
        let g = GameId::Conquest;
        assert_eq!(e.beacon.game(g).path.unwrap(), dir);
        let st = e.beacon.status(g);
        assert!(st.ready && st.issues.is_empty(), "{:?}", st.issues);
        assert_eq!(st.family, games::Family::Lotr);
        // No BFME switches, no EA keys, no Options.ini.
        e.beacon.game_mut(g).windowed = true;
        e.beacon.game_mut(g).resolution = Resolution::parse("1280x800");
        let cmd = e.beacon.launch_command(g, None).unwrap();
        assert!(cmd.args[0].ends_with("LOTR Conquest/Conquest.exe"));
        assert_eq!(cmd.args.len(), 1);
        assert_eq!(e.beacon.registry_for(g).unwrap(), "REGEDIT4\r\n");
        assert!(e.beacon.setup(g, true).unwrap());
        assert!(e.beacon.apply_options(g).unwrap().is_none());
        // -mod style mods are refused with a clear message; launchers are fine.
        let big = e.games.join("x.big");
        fs::write(&big, b"").unwrap();
        let m = e.beacon.link_mod(g, &big, Some("X")).unwrap();
        assert!(e.beacon.launch_command(g, Some(&m.id)).unwrap_err().to_string().contains("-mod"));
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn other_games() {
        let mut e = env();
        let swgb = e.games.join("steamapps/common/Star Wars Galactic Battlegrounds Saga/Game");
        let uaw = e.games.join("Universe at War Earth Assault");
        for (d, exe) in [(&swgb, "battlegrounds_x1.exe"), (&uaw, "UAWEA.exe")] {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join(exe), b"").unwrap();
        }
        e.beacon.scan(&[]);

        // SWGB: virtual desktop by default, sized 1280x1024 unless set.
        let st = e.beacon.status(GameId::Swgb);
        assert!(st.ready && st.virtual_desktop && st.steam_appid == Some(356500), "{st:?}");
        let c = e.beacon.launch_command(GameId::Swgb, None).unwrap();
        assert_eq!(c.args[0], "explorer");
        assert_eq!(c.args[1], "/desktop=Beacon,1280x1024");
        assert!(c.args[2].starts_with("Z:\\") && c.args[2].ends_with("\\battlegrounds_x1.exe"), "{c:?}");
        e.beacon.game_mut(GameId::Swgb).resolution = Resolution::parse("1024x768");
        assert_eq!(e.beacon.launch_command(GameId::Swgb, None).unwrap().args[1], "/desktop=Beacon,1024x768");
        e.beacon.game_mut(GameId::Swgb).virtual_desktop = Some(false);
        assert!(e.beacon.launch_command(GameId::Swgb, None).unwrap().args[0].ends_with("battlegrounds_x1.exe"));

        // Through Steam: no prefix, Steam does the launch.
        e.beacon.game_mut(GameId::Swgb).via_steam = true;
        let c = e.beacon.launch_command(GameId::Swgb, None).unwrap();
        assert!(c.args.iter().any(|a| a.contains("356500")), "{c:?}");
        assert!(!e.beacon.setup(GameId::Swgb, true).unwrap());

        // UaW: warns about xlive.dll, and flips DX10Enabled before launch.
        let st = e.beacon.status(GameId::Uaw);
        assert!(st.ready && st.issues.iter().any(|i| i.message.contains("xlive.dll")));
        assert_eq!(st.steam_appid, None, "not in a Steam library");
        fs::write(uaw.join("xlive.dll"), b"").unwrap();
        assert!(!e.beacon.status(GameId::Uaw).issues.iter().any(|i| i.message.contains("xlive.dll")));
        let reg = e.beacon.paths.prefix_root(GameId::Uaw).join("pfx/user.reg");
        fs::create_dir_all(reg.parent().unwrap()).unwrap();
        fs::write(&reg, "[Software\\\\Petroglyph]\n\"DX10Enabled\"=dword:00000001\n").unwrap();
        assert!(e.beacon.apply_options(GameId::Uaw).unwrap().is_some());
        assert!(fs::read_to_string(&reg).unwrap().contains("\"DX10Enabled\"=dword:00000000"));
        assert!(e.beacon.apply_options(GameId::Uaw).unwrap().is_none(), "already fixed");
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn community_versions() {
        let mut e = env();
        let game = e.games.join("steamapps/common/Star Wars Galactic Battlegrounds Saga/Game");
        let uaw = e.games.join("Universe at War Earth Assault");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("battlegrounds_x1.exe"), b"").unwrap();
        fs::create_dir_all(&uaw).unwrap();
        fs::write(uaw.join("UAWEA.exe"), b"").unwrap();
        fs::write(uaw.join("xlive.dll"), b"").unwrap();
        e.beacon.scan(&[]);

        // SWGB starts on Clone Campaigns and nags about the community version.
        let st = e.beacon.status(GameId::Swgb);
        assert_eq!(st.edition.as_deref(), Some("Clone Campaigns"));
        assert!(st.issues.iter().any(|i| i.message.contains("Community version not set up")));

        // Add Expanding Fronts from an extracted download.
        let dl = e.games.join("Downloads/Expanding Fronts 1.5.5");
        fs::create_dir_all(dl.join("GAME/DATA")).unwrap();
        fs::write(dl.join("GAME/battlegrounds_x2.exe"), b"").unwrap();
        fs::write(dl.join("GAME/DATA/x.dat"), b"").unwrap();
        let msg = e.beacon.add_files(GameId::Swgb, "expanding-fronts", &dl).unwrap();
        assert!(msg.contains("Merged 2 files"), "{msg}");
        let st = e.beacon.status(GameId::Swgb);
        assert_eq!(st.edition.as_deref(), Some("Expanding Fronts"));
        assert!(st.issues.is_empty(), "{:?}", st.issues);
        let c = e.beacon.launch_command(GameId::Swgb, None).unwrap();
        assert!(c.args.iter().any(|a| a.ends_with("\\battlegrounds_x2.exe")), "{c:?}");
        assert!(e.beacon.community(GameId::Swgb).steps[0].done);
        // Can switch back to vanilla Clone Campaigns.
        e.beacon.set_edition(GameId::Swgb, Some("clone-campaigns")).unwrap();
        assert!(e.beacon.status(GameId::Swgb).issues.iter().any(|i| i.message.contains("Playing the original")));
        assert!(e.beacon.set_edition(GameId::Swgb, Some("original")).is_err(), "not installed");

        // UaW: Community Patch from a folder lands under Mods and loads via MODPATH.
        let cp = e.games.join("Downloads/Community_Patch_2.0");
        fs::create_dir_all(cp.join("Data/XML")).unwrap();
        fs::write(cp.join("Data/XML/units.xml"), b"").unwrap();
        e.beacon.add_files(GameId::Uaw, "community-patch", &cp).unwrap();
        assert!(uaw.join("Mods/CommunityPatch/Data/XML/units.xml").is_file());
        let c = e.beacon.launch_command(GameId::Uaw, None).unwrap();
        assert_eq!(c.args[1], "MODPATH=Mods\\CommunityPatch", "{c:?}");
        // A single file (xlive.dll) is copied in, with a backup of the old one.
        let x = e.games.join("Downloads/xlive.dll");
        fs::write(&x, b"new").unwrap();
        e.beacon.add_files(GameId::Uaw, "xliveless", &x).unwrap();
        assert_eq!(fs::read(uaw.join("xlive.dll")).unwrap(), b"new");
        assert!(e.beacon.add_files(GameId::Uaw, "patch3", &x).is_err(), "installers run, not merge");
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn status_tells_front_ends_what_applies() {
        let mut e = env();
        let mut seen = Vec::new();
        e.beacon.scan_with(&[], &mut |i: usize, n: usize, root: &Path| seen.push((i, n, root.to_path_buf())));
        assert!(!seen.is_empty() && seen.iter().all(|(i, n, _)| i < n), "{seen:?}");
        assert!(seen.iter().any(|(_, _, r)| r.ends_with("Games")));

        let st = e.beacon.status(GameId::Bfme2);
        assert!(st.sage && st.wants_cd_key && st.requires.is_none());
        assert_eq!(st.needs_setup, Some(true));
        assert!(!st.has_log && st.log_file.ends_with("logs/bfme2.log"));
        assert!(!st.has_community && st.virtual_desktop_choice.is_none() && !st.virtual_desktop_default);
        assert_eq!(e.beacon.status(GameId::Rotwk).requires, Some(GameId::Bfme2));
        assert!(e.beacon.setup(GameId::Bfme2, false).unwrap());
        let st = e.beacon.status(GameId::Bfme2);
        assert_eq!(st.needs_setup, Some(false));
        assert!(st.has_log, "setup writes the log");

        let swgb = e.beacon.status(GameId::Swgb);
        assert!(!swgb.sage && !swgb.wants_cd_key && swgb.has_community && swgb.virtual_desktop_default);
        assert_eq!(swgb.needs_setup, None, "not installed");
        e.beacon.game_mut(GameId::Swgb).virtual_desktop = Some(false);
        assert_eq!(e.beacon.status(GameId::Swgb).virtual_desktop_choice, Some(false));

        // The new fields are in the JSON, and old JSON without them still reads.
        let v = serde_json::to_value(e.beacon.status(GameId::Bfme2)).unwrap();
        for k in ["sage", "wants_cd_key", "requires", "virtual_desktop_choice", "virtual_desktop_default", "needs_setup", "log_file", "has_log", "has_community"] {
            assert!(v.get(k).is_some(), "{k} missing");
        }
        let mut old = v.as_object().unwrap().clone();
        for k in ["sage", "wants_cd_key", "requires", "virtual_desktop_choice", "virtual_desktop_default", "needs_setup", "log_file", "has_log", "has_community"] {
            old.remove(k);
        }
        let back: GameStatus = serde_json::from_value(serde_json::Value::Object(old)).unwrap();
        assert!(!back.sage);
    }

    // Runs the stub `wine` (a shell script) and checks Wine command lines.
    #[cfg(unix)]
    #[test]
    fn installers_run_before_the_game_is_there() {
        let e = env();
        // Conquest isn't installed yet: its prefix can't get registry keys, but an
        // installer must still be able to run in it.
        assert!(e.beacon.registry_for(GameId::Conquest).is_err());
        assert!(!e.beacon.prepare_prefix(GameId::Conquest).unwrap());
        assert!(e.beacon.paths.prefix_root(GameId::Conquest).is_dir());
        let exe = e.games.join("setup.exe");
        fs::write(&exe, b"").unwrap();
        assert!(e.beacon.run_in_prefix_command(GameId::Conquest, &exe).is_ok());
    }

    #[test]
    fn add_files_reports_progress() {
        let mut e = env();
        let uaw = e.games.join("Universe at War Earth Assault");
        fs::create_dir_all(&uaw).unwrap();
        fs::write(uaw.join("UAWEA.exe"), b"").unwrap();
        e.beacon.scan(&[]);
        let x = e.games.join("xlive.dll");
        fs::write(&x, b"x").unwrap();
        let mut said = Vec::new();
        e.beacon.add_files_with(GameId::Uaw, "xliveless", &x, &mut |t: &str| said.push(t.to_string())).unwrap();
        assert!(said.iter().any(|t| t.starts_with("Copying files")), "{said:?}");
    }

    /// Windows behaviour, checked on any OS: no prefix, the exe itself, Options.ini under %APPDATA%.
    #[test]
    fn native_mode() {
        let mut e = env();
        e.beacon.native = true;
        e.beacon.runners = RunnerSet::native();
        let appdata = e.games.parent().unwrap().join("AppData/Roaming");
        e.beacon.paths.appdata = Some(appdata.clone());
        e.beacon.scan(&[]);
        let g = GameId::Bfme2;
        let st = e.beacon.status(g);
        assert!(st.ready, "{:?}", st.issues);
        assert_eq!(st.runner.as_ref().unwrap().kind, runner::RunnerKind::Native);
        assert_eq!((st.prefix_ready, st.needs_setup), (false, Some(false)));
        assert!(!e.beacon.setup(g, true).unwrap(), "nothing to set up");
        assert!(!e.beacon.prepare_prefix(g).unwrap());
        assert!(e.beacon.setup_command(g).is_err());
        assert!(!e.beacon.paths.data_dir.join("prefixes").exists(), "no prefixes on Windows");

        e.beacon.game_mut(g).windowed = true;
        let mods = e.beacon.paths.mods_dir().join("bfme2").join("My Mod");
        fs::create_dir_all(&mods).unwrap();
        fs::write(mods.join("my.big"), b"").unwrap();
        let c = e.beacon.launch_command(g, Some("bfme2:my-mod")).unwrap();
        assert!(c.program.ends_with("lotrbfme2.exe"), "{c:?}");
        assert_eq!(c.args, vec!["-win".to_string(), "-mod".into(), mods.to_string_lossy().into_owned()]);
        assert_eq!(c.cwd, e.games.join("BFME2"));
        assert!(c.env.is_empty());

        e.beacon.game_mut(g).resolution = Resolution::parse("1920x1080");
        let ini = e.beacon.apply_options(g).unwrap().unwrap();
        assert_eq!(ini, appdata.join("My The Battle for Middle-earth II Files/Options.ini"));
        assert_eq!(fs::read_to_string(ini).unwrap(), "Resolution = 1920 1080\r\n");

        // No Wine virtual desktop, and the store link is there for SWGB.
        let swgb = e.games.join("SWGB/Game");
        fs::create_dir_all(&swgb).unwrap();
        fs::write(swgb.join("battlegrounds_x1.exe"), b"").unwrap();
        e.beacon.set_path(GameId::Swgb, &swgb).unwrap();
        let st = e.beacon.status(GameId::Swgb);
        assert!(!st.virtual_desktop && !st.virtual_desktop_default);
        assert_eq!(st.store_url.as_deref(), Some("https://store.steampowered.com/app/356500/"));
        assert!(e.beacon.launch_command(GameId::Swgb, None).unwrap().program.ends_with("battlegrounds_x1.exe"));
        let v = serde_json::to_value(e.beacon.status(GameId::Uaw)).unwrap();
        assert!(v["store_url"].is_null(), "UaW is delisted");
    }

    #[test]
    fn install_from_installer_or_folder() {
        let e = env();
        let disc = e.games.join("BFME2 Disc");
        fs::create_dir_all(disc.join("Setup")).unwrap();
        fs::write(disc.join("autorun.inf"), "[autorun]\nopen=AutoRun.exe\n").unwrap();
        fs::write(disc.join("AutoRun.exe"), b"").unwrap();
        fs::write(disc.join("Setup/Setup.exe"), b"").unwrap();
        let p = e.beacon.prepare_install(GameId::Bfme2, &disc, &mut |_: &str| {}).unwrap();
        assert_eq!((p.setup, p.extracted), (disc.join("Setup/Setup.exe"), None));
        let exe = disc.join("AutoRun.exe");
        assert_eq!(e.beacon.prepare_install(GameId::Bfme2, &exe, &mut |_: &str| {}).unwrap().setup, exe);
        let empty = e.games.join("Empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(e.beacon.prepare_install(GameId::Bfme2, &empty, &mut |_: &str| {}).unwrap_err().to_string().contains("no setup program"));
        assert!(e.beacon.prepare_install(GameId::Bfme2, &e.games.join("nope.iso"), &mut |_: &str| {}).is_err());
    }

    /// The All-in-One launcher run inside a Wine prefix: its registry points at
    /// a folder outside every scan root, and it's recognised as the launcher's.
    #[cfg(unix)]
    #[test]
    fn all_in_one_launcher_in_wine() {
        let mut e = env();
        let home = e.games.parent().unwrap().to_path_buf();
        let outside = home.parent().unwrap().join("Other Drive");
        let b2 = outside.join("BFME2");
        fs::create_dir_all(&b2).unwrap();
        fs::write(b2.join("lotrbfme2.exe"), b"").unwrap();
        fs::create_dir_all(outside.join("BFME Workshop/Mods")).unwrap();
        let pfx = home.join("Games/aio-launcher");
        let appdata = pfx.join("drive_c/users/me/AppData/Roaming");
        fs::create_dir_all(appdata.join("BFME Workshop/Config")).unwrap();
        fs::write(appdata.join("BFME Workshop/Config/active_patch_1.json"), r#"{"Guid":"p","Name":"Patch 2.22","Version":"40"}"#).unwrap();
        let win = format!("Z:{}\\\\", b2.to_string_lossy().replace('/', "\\\\"));
        let reg = format!(
            "WINE REGISTRY Version 2\n\n[Software\\\\Wow6432Node\\\\Electronic Arts\\\\Electronic Arts\\\\The Battle for Middle-earth II] 1\n\"InstallPath\"=\"{win}\"\n"
        );
        fs::write(pfx.join("system.reg"), reg).unwrap();

        assert!(e.beacon.wine_prefixes(&[]).contains(&pfx));
        assert!(e.beacon.install_hints(&[]).contains(&b2), "{:?}", e.beacon.install_hints(&[]));
        let found = e.beacon.scan(&[]);
        assert!(found.iter().any(|f| f.game == GameId::Bfme2 && f.dir == b2), "{found:?}");
        e.beacon.set_path(GameId::Bfme2, &b2).unwrap();
        let aio = e.beacon.aio_games();
        assert_eq!(aio.len(), 1, "{aio:?}");
        assert_eq!((aio[0].game, aio[0].active_patch.as_deref()), (GameId::Bfme2, Some("Patch 2.22 (40)")));
    }

    #[test]
    fn link_and_unlink() {
        let mut e = env();
        let big = e.games.join("widescreen.big");
        fs::write(&big, b"").unwrap();
        let m = e.beacon.link_mod(GameId::Bfme1, &big, Some("Widescreen Fix")).unwrap();
        assert_eq!(m.id, "bfme1:widescreen-fix");
        assert!(e.beacon.link_mod(GameId::Bfme1, &big, Some("Widescreen Fix")).is_err());
        e.beacon.select_mod(GameId::Bfme1, Some(&m.id)).unwrap();
        assert!(e.beacon.unlink_mod(&m.id));
        assert!(e.beacon.config.selected_mod.is_empty());
    }
}
