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

pub mod catalog;
pub mod config;
pub mod games;
pub mod mods;
pub mod options_ini;
pub mod registry;
pub mod runner;

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
    pub selected_mod: Option<String>,
    /// Extra exe the game ships (AotR's own launcher, for updates).
    pub tool_launcher: Option<PathBuf>,
    /// No errors (warnings are fine).
    pub ready: bool,
    pub issues: Vec<Issue>,
}

/// A catalog entry plus the id of the matching installed mod, if any.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogItem {
    #[serde(flatten)]
    pub entry: catalog::CatalogEntry,
    pub installed: Option<String>,
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
}

impl Beacon {
    pub fn load() -> Result<Beacon> {
        let paths = Paths::from_env()?;
        let path_env = std::env::var("PATH").unwrap_or_default();
        Beacon::load_with(paths, &path_env)
    }

    pub fn load_with(paths: Paths, path_env: &str) -> Result<Beacon> {
        let config = Config::load(&paths.config_file)?;
        let runners = runner::discover(&paths.home, path_env);
        Ok(Beacon { paths, config, runners })
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
        roots.extend(self.config.scan_roots.iter().cloned());
        roots
    }

    /// Scan for installs; games without a folder get the first one found.
    pub fn scan(&mut self, extra_roots: &[PathBuf]) -> Vec<FoundGame> {
        let mut roots = self.scan_roots();
        roots.extend(extra_roots.iter().cloned());
        let found = games::scan(&roots, 9);
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
        let selected_mod = self.config.selected_mod.get(&g).cloned().filter(|id| self.find_mod(g, id).is_some());
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
            prefix_ready: self.paths.prefix_root(g).join("pfx/drive_c").is_dir(),
            selected_mod,
            tool_launcher: if g == GameId::Aotr { self.aotr_bundled().launcher } else { None },
            ready: !issues.iter().any(|i| i.level == Level::Error),
            issues,
        }
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
        let game_dirs: Vec<PathBuf> = GameId::ALL.into_iter().filter_map(|g| self.game_dir(g)).collect();
        let mut roots = catalog::default_mod_roots(&self.paths.home, &game_dirs);
        roots.extend(extra_roots.iter().cloned());
        let mut added = Vec::new();
        for m in catalog::discover(&roots, 4) {
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
        let mut cmd = runner::command_for(runner, self.runners.umu.as_deref(), &self.paths.home, &self.paths.prefix_root(g), target, args, cwd);
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
        let reg = self.registry_for(g)?;
        let fp = self.setup_fingerprint(g, &reg)?;
        let marker: Option<SetupMarker> = std::fs::read_to_string(self.paths.prefix_root(g).join("beacon-setup.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok());
        Ok(marker.map(|m| m.fingerprint != fp).unwrap_or(true) || !self.paths.prefix_root(g).join("pfx").is_dir())
    }

    /// The regedit command `setup()` would run, after writing the .reg file.
    pub fn setup_command(&self, g: GameId) -> Result<Command> {
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
        if !force && !self.needs_setup(g)? {
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

    /// Write the chosen resolution into Options.ini inside the prefix.
    pub fn apply_options(&self, g: GameId) -> Result<Option<PathBuf>> {
        if !g.is_sage() {
            return Ok(None);
        }
        let Some(res) = self.game(g).resolution else { return Ok(None) };
        let pfx = self.paths.prefix_root(g).join("pfx");
        if !pfx.join("drive_c").is_dir() {
            return Ok(None);
        }
        let dir = options_ini::appdata_dir(&pfx).join(g.user_data_leaf());
        std::fs::create_dir_all(&dir)?;
        let file = dir.join("Options.ini");
        let current = std::fs::read_to_string(&file).unwrap_or_default();
        std::fs::write(&file, options_ini::set_value(&current, "Resolution", &res.ini_value()))?;
        Ok(Some(file))
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

        let mut args = Vec::new();
        if let Some(m) = chosen.as_ref().filter(|_| !g.is_sage()) {
            bail!("{} doesn't load mods with -mod; install \"{}\" into the game folder or add its own launcher instead", g.short(), m.name);
        }
        if cfg.windowed && g.is_sage() {
            args.push("-win".to_string());
        }
        if let Some(m) = &chosen {
            args.push("-mod".into());
            args.push(registry::wine_path(&m.path, false));
            args.extend(m.args.iter().cloned());
        }
        args.extend(split_args(&cfg.extra_args));
        let exe = games::find_exe_in(&dir, g).unwrap();
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
        let mut f = std::fs::OpenOptions::new().create(true).append(what == "launch").write(true).open(&path)?;
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
        cmd.to_std()
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .with_context(|| format!("starting {}", cmd.program.display()))
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
