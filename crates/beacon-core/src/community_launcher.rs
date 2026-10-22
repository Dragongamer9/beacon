//! A game's community launcher: a program the game's community distributes
//! that downloads (and updates) the game itself. So far only Universe at War
//! has one.
//!
//! **Beacon never downloads it.** The UaW launcher is only distributed from a
//! Google Drive folder with several versions, no stable direct-download URL and
//! no checksums, so Beacon only opens that folder in the browser
//! ([`launcher_url`]). The player downloads the version they want and hands it
//! to Beacon ([`Beacon::set_community_launcher`]):
//!
//! * a standalone `.exe` is copied to `<data>/tools/<game>-launcher/<name>/`;
//! * a `.zip` is extracted there with the same safe extraction as disc images
//!   ([`crate::media::extract_with`]: bsdtar or 7-Zip with separate arguments,
//!   no shell, links removed), and its launcher exe is picked ([`pick_exe`]).
//!
//! [`Beacon::open_community_launcher`] runs it: on Linux in the game's own Wine
//! prefix (`<data>/prefixes/<game>/pfx`, created first when needed), detached
//! in its own process group, logged to `<data>/logs/<game>-launcher.log`; on
//! Windows directly (through the UAC prompt when it asks for administrator
//! rights). The games it downloads land in that prefix's `drive_c` (or
//! wherever the player points it); [`Beacon::adopt_community_games`] and the
//! normal scan (which walks `<data>/prefixes`) find them.

use crate::aio_launcher::{prefix_has_user, wait_with_timeout};
use crate::games::{self, FoundGame, GameId};
use crate::runner::Command;
use crate::{installs, workshop, Beacon};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

/// Where each game's community launcher is distributed (opened in the browser,
/// never downloaded by Beacon). Add a row to give another game one.
pub const LAUNCHER_URLS: &[(GameId, &str)] = &[(GameId::Uaw, "https://drive.google.com/drive/folders/167UET84ZWXrMd9zpxEYiQWTQZexCGZfT")];

/// What the player is told next to the link.
pub const HOW_TO: &str = "Download the launcher version you want, then choose it below. Games are downloaded inside it; Beacon finds them afterwards.";

/// Creating the game's prefix: umu's first run also downloads the Steam Linux Runtime.
const PREFIX_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// How deep the folders a launcher downloads into are searched.
const ADOPT_DEPTH: usize = 7;

/// The page a game's community launcher is downloaded from, if it has one.
pub fn launcher_url(g: GameId) -> Option<&'static str> {
    LAUNCHER_URLS.iter().find(|(id, _)| *id == g).map(|(_, u)| *u)
}

/// In `status --json` (`GameStatus.community_launcher`), for games that have one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherRef {
    /// The launcher exe the player chose (`null` until one is chosen, or when it's gone).
    pub path: Option<PathBuf>,
    /// Where it's downloaded from (opened in the browser).
    pub url: String,
}

/// What `community-launcher <game> status` reports.
#[derive(Debug, Clone, Serialize)]
pub struct LauncherStatus {
    pub game: GameId,
    /// Where it's downloaded from (opened in the browser; Beacon never downloads it).
    pub url: &'static str,
    /// The launcher exe the player chose, when it's there.
    pub path: Option<PathBuf>,
    /// One was chosen but its exe is gone (choose it again).
    pub missing: bool,
    /// `open` can run: a launcher is chosen and, on Linux, there's a runner.
    pub can_open: bool,
    /// Linux: the Wine prefix it runs in (the game's own); `null` on Windows.
    pub prefix: Option<PathBuf>,
    /// Linux: the runner it runs with (the game's runner).
    pub runner: Option<String>,
    /// Where chosen launchers are kept (`<data>/tools/<game>-launcher`).
    pub tools_dir: PathBuf,
    /// Beacon's log for it.
    pub log: PathBuf,
    /// The game itself is installed (found).
    pub game_installed: bool,
}

/// What the player handed Beacon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Exe,
    Zip,
}

fn ext_is(p: &Path, ext: &str) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case(ext)).unwrap_or(false)
}

/// Check a launcher the player picked: an absolute path to an existing `.exe`, or the archive it came in
/// (`.zip`, `.7z` or `.rar`; all three are extracted the same safe way).
pub fn validate_source(p: &Path) -> Result<SourceKind> {
    if !p.is_absolute() {
        bail!("give the launcher's full path ({} isn't absolute)", p.display());
    }
    if !p.exists() {
        bail!("{} not found", p.display());
    }
    if !p.is_file() {
        bail!("{} isn't a file; pick the launcher's .exe or the archive it came in", p.display());
    }
    if ext_is(p, "exe") {
        Ok(SourceKind::Exe)
    } else if ext_is(p, "zip") || ext_is(p, "7z") || ext_is(p, "rar") {
        Ok(SourceKind::Zip)
    } else {
        bail!("pick the launcher's .exe or the archive it came in (.zip, .7z or .rar)")
    }
}

/// Exes that come along with a launcher but aren't it.
fn is_helper_exe(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("unins")
        || n.contains("uninstall")
        || ["vc_redist", "vcredist", "dxsetup", "dxwebsetup", "dotnet", "ndp4", "windowsdesktop-runtime", "crashpad", "crashreport", "createdump"]
            .iter()
            .any(|h| n.contains(h))
}

/// The launcher exe in an extracted folder: `.exe` files up to three folders
/// down, helpers (uninstallers, runtime installers, crash reporters) left out;
/// names with "launcher" first, setup/installer names last, then the
/// shallowest, the shortest name, and the path.
pub fn pick_exe(dir: &Path) -> Option<PathBuf> {
    let mut cands: Vec<(u8, usize, usize, PathBuf)> = walkdir::WalkDir::new(dir)
        .max_depth(4)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.ends_with(".exe") || is_helper_exe(&name) {
                return None;
            }
            let rank = if name.contains("launcher") {
                0
            } else if name.contains("setup") || name.contains("install") {
                2
            } else {
                1
            };
            Some((rank, e.depth(), name.len(), e.into_path()))
        })
        .collect();
    cands.sort();
    cands.into_iter().next().map(|(_, _, _, p)| p)
}

/// A folder name for a launcher, from its file name: letters, digits, spaces
/// and `-_.()+,`, at most 60 characters; "launcher" when nothing usable is left.
pub fn folder_name(source: &Path) -> String {
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let s: String = stem.chars().map(|c| if c.is_alphanumeric() || " -_.()+,".contains(c) { c } else { ' ' }).collect();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(60).collect();
    let s = s.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    match workshop::safe_rel_path(&s) {
        Some(p) if !s.is_empty() && p.components().count() == 1 => s,
        _ => "launcher".into(),
    }
}

/// A Windows program starts with `MZ`.
fn is_windows_program(p: &Path) -> bool {
    let mut head = [0u8; 2];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut head)).map(|_| &head == b"MZ").unwrap_or(false)
}

/// Every entry under `dir` really is under it (no links, nothing resolving outside).
fn all_inside(dir: &Path) -> bool {
    let Ok(root) = dir.canonicalize() else { return false };
    walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .all(|e| e.map(|e| !e.path_is_symlink() && e.path().canonicalize().map(|c| c.starts_with(&root)).unwrap_or(false)).unwrap_or(false))
}

impl Beacon {
    /// `<data>/tools/<game>-launcher`: where chosen launchers are kept, one folder each.
    pub fn community_launcher_dir(&self, g: GameId) -> PathBuf {
        self.paths.data_dir.join("tools").join(format!("{}-launcher", g.slug()))
    }

    /// `<data>/logs/<game>-launcher.log`.
    pub fn community_launcher_log(&self, g: GameId) -> PathBuf {
        self.paths.data_dir.join("logs").join(format!("{}-launcher.log", g.slug()))
    }

    /// The chosen launcher exe, when it's still there.
    pub fn community_launcher_exe(&self, g: GameId) -> Option<PathBuf> {
        launcher_url(g)?;
        self.game(g).community_launcher.filter(|p| p.is_file())
    }

    /// For `GameStatus`: `{path, url}` for games that have a community launcher.
    pub fn community_launcher_ref(&self, g: GameId) -> Option<LauncherRef> {
        launcher_url(g).map(|url| LauncherRef { path: self.community_launcher_exe(g), url: url.to_string() })
    }

    /// `None` for games without a community launcher.
    pub fn community_launcher_status(&self, g: GameId) -> Option<LauncherStatus> {
        let url = launcher_url(g)?;
        let path = self.community_launcher_exe(g);
        let runner = if self.native { None } else { self.runner_for(g).map(|r| r.id.clone()) };
        Some(LauncherStatus {
            game: g,
            url,
            missing: path.is_none() && self.game(g).community_launcher.is_some(),
            can_open: path.is_some() && (self.native || runner.is_some()),
            path,
            prefix: (!self.native).then(|| self.paths.prefix_root(g).join("pfx")),
            runner,
            tools_dir: self.community_launcher_dir(g),
            log: self.community_launcher_log(g),
            game_installed: self.game_dir(g).is_some(),
        })
    }

    fn no_community_launcher(g: GameId) -> anyhow::Error {
        anyhow!("{} has no community launcher", g.short())
    }

    /// Use a launcher the player downloaded: an absolute path to its `.exe`
    /// (copied) or the `.zip` it came in (extracted safely), kept in
    /// `<data>/tools/<game>-launcher/<name>/`, and remembered for the game
    /// (save the config afterwards). Returns the launcher exe.
    pub fn set_community_launcher(&mut self, g: GameId, source: &Path, progress: &mut dyn FnMut(&str)) -> Result<PathBuf> {
        launcher_url(g).ok_or_else(|| Self::no_community_launcher(g))?;
        let kind = validate_source(source)?;
        let tools = self.community_launcher_dir(g);
        std::fs::create_dir_all(&tools).with_context(|| format!("creating {}", tools.display()))?;

        // Already one of Beacon's copies (chosen again): just remember it.
        let inside = match (source.canonicalize(), tools.canonicalize()) {
            (Ok(s), Ok(t)) => s.starts_with(&t),
            _ => false,
        };
        if inside && kind == SourceKind::Exe {
            if !is_windows_program(source) {
                bail!("{} isn't a Windows program", source.display());
            }
            self.game_mut(g).community_launcher = Some(source.to_path_buf());
            return Ok(source.to_path_buf());
        }

        let name = folder_name(source);
        let dest = tools.join(&name);
        let staging = tools.join(format!(".{name}.beacon-tmp"));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).with_context(|| format!("removing {}", staging.display()))?;
        }
        let staged = (|| -> Result<PathBuf> {
            match kind {
                SourceKind::Exe => {
                    if !is_windows_program(source) {
                        bail!("{} isn't a Windows program", source.display());
                    }
                    progress("Copying the launcher");
                    std::fs::create_dir_all(&staging)?;
                    let file = source
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .filter(|n| workshop::safe_rel_path(n).map(|p| p.components().count() == 1).unwrap_or(false))
                        .unwrap_or_else(|| "launcher.exe".into());
                    let to = staging.join(file);
                    std::fs::copy(source, &to).with_context(|| format!("copying {} to {}", source.display(), to.display()))?;
                    Ok(to)
                }
                SourceKind::Zip => {
                    progress(&format!("Extracting {}", source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
                    crate::media::extract_with(
                        source,
                        &staging,
                        if cfg!(windows) {
                            "no tool to open the .zip: Windows 10 or later has tar.exe; or install 7-Zip, or extract it yourself and pick the launcher's .exe"
                        } else {
                            "no tool to open the .zip: install libarchive (bsdtar) or 7-Zip (7z), or extract it yourself and pick the launcher's .exe"
                        },
                    )
                    .with_context(|| format!("extracting {}", source.display()))?;
                    if !all_inside(&staging) {
                        bail!("the .zip has entries that point outside its folder; nothing was used");
                    }
                    let exe = pick_exe(&staging).ok_or_else(|| anyhow!("no launcher .exe found in {}", source.display()))?;
                    if !is_windows_program(&exe) {
                        bail!("{} in the .zip isn't a Windows program", exe.file_name().unwrap_or_default().to_string_lossy());
                    }
                    Ok(exe)
                }
            }
        })();
        let staged = match staged {
            Ok(x) => x,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let rel = staged.strip_prefix(&staging).map(Path::to_path_buf).map_err(|_| anyhow!("internal: launcher outside its folder"))?;
        progress("Putting the launcher in place");
        if dest.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dest) {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e).with_context(|| format!("replacing {} (close that launcher if it's open)", dest.display()));
            }
        }
        if let Err(e) = std::fs::rename(&staging, &dest) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e).with_context(|| format!("putting the launcher in {}", dest.display()));
        }
        let exe = dest.join(rel);
        self.game_mut(g).community_launcher = Some(exe.clone());
        Ok(exe)
    }

    /// What `open_community_launcher` runs: the exe itself on Windows; on
    /// Linux the exe through the game's runner, in the game's prefix.
    pub fn community_launcher_command(&self, g: GameId) -> Result<Command> {
        launcher_url(g).ok_or_else(|| Self::no_community_launcher(g))?;
        let exe = self.community_launcher_exe(g).ok_or_else(|| match self.game(g).community_launcher {
            Some(p) => anyhow!("the {} launcher at {} is gone; choose it again", g.short(), p.display()),
            None => anyhow!("no {} launcher chosen yet: download it ({}), then choose it (`beacon community-launcher {} set <path>`)", g.short(), launcher_url(g).unwrap_or(""), g.slug()),
        })?;
        let cwd = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| self.paths.data_dir.clone());
        if self.native {
            return Ok(Command { program: exe, args: vec![], env: vec![], cwd });
        }
        self.command(g, &exe.to_string_lossy(), &[], &cwd)
    }

    fn open_community_log(&self, g: GameId, cmd: &Command, what: &str) -> Result<std::fs::File> {
        let path = self.community_launcher_log(g);
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(f, "== beacon {what}: {}", cmd.display())?;
        Ok(f)
    }

    /// Linux: get the game's prefix ready for its launcher: the usual setup
    /// when the game is already there, then `wineboot -u` if the prefix has no
    /// user folder yet.
    fn prepare_community_prefix(&self, g: GameId, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<()> {
        if self.native {
            return Ok(());
        }
        progress(Some(5), &format!("Preparing {}'s Wine prefix", g.short()));
        self.prepare_prefix(g)?;
        let root = self.paths.prefix_root(g);
        let pfx = root.join("pfx");
        if prefix_has_user(&pfx) {
            return Ok(());
        }
        progress(Some(10), &format!("Creating {}'s Wine prefix (the first time can take a few minutes)", g.short()));
        let cmd = self.command(g, "wineboot", &["-u".to_string()], &root)?;
        let log = self.open_community_log(g, &cmd, "prefix")?;
        let mut child = cmd
            .to_std()
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .with_context(|| format!("starting {}", cmd.program.display()))?;
        let st = wait_with_timeout(&mut child, PREFIX_TIMEOUT).with_context(|| format!("creating {}'s Wine prefix", g.short()))?;
        if !prefix_has_user(&pfx) {
            bail!("creating {}'s Wine prefix failed ({st}); see {}", g.short(), self.community_launcher_log(g).display());
        }
        Ok(())
    }

    /// Start the game's community launcher, detached from Beacon (its own
    /// process group on Linux; through the UAC prompt on Windows when it asks
    /// for administrator rights). Output goes to its log. When it closes, call
    /// [`Beacon::rescan_after_community_launcher`].
    pub fn open_community_launcher(&self, g: GameId, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<std::process::Child> {
        // Fail fast (no launcher, no runner) before touching the prefix.
        self.community_launcher_command(g)?;
        self.prepare_community_prefix(g, progress)?;
        let cmd = self.community_launcher_command(g)?;
        progress(Some(90), &format!("Starting the {} launcher", g.short()));
        let log = self.open_community_log(g, &cmd, "open")?;
        let mut c = cmd.to_std();
        c.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        match c.spawn() {
            Ok(child) => Ok(child),
            #[cfg(windows)]
            Err(e) if e.raw_os_error() == Some(crate::native::ERROR_ELEVATION_REQUIRED) => {
                let _ = self.open_community_log(g, &cmd, "open (with administrator rights)");
                crate::native::spawn_elevated(&self.paths.data_dir, &cmd)
            }
            Err(e) => Err(e).with_context(|| format!("starting {}", cmd.program.display())),
        }
    }

    /// The game, downloaded by its community launcher, when Beacon has no
    /// folder for it yet: looks in the game's prefix (`drive_c`, and the
    /// folders its registry names) and the launchers' own folder, without a
    /// full scan. Fills in the folder; returns what it found.
    pub fn adopt_community_games(&mut self, g: GameId) -> Vec<FoundGame> {
        if launcher_url(g).is_none() || self.game_dir(g).is_some() {
            return Vec::new();
        }
        let mut roots = vec![self.community_launcher_dir(g)];
        if !self.native {
            let pfx = self.paths.prefix_root(g).join("pfx");
            if pfx.join("drive_c").is_dir() {
                roots.push(pfx.join("drive_c"));
                roots.extend(installs::prefix_hints(&pfx).into_iter().filter(|(x, _)| x.is_none() || *x == Some(g)).map(|(_, d)| d));
            }
        }
        let mut found: Vec<FoundGame> = Vec::new();
        for r in roots {
            for f in games::scan(std::slice::from_ref(&r), ADOPT_DEPTH) {
                if f.game == g && !found.contains(&f) {
                    found.push(f);
                }
            }
        }
        if let Some(f) = found.first() {
            self.game_mut(g).path = Some(f.dir.clone());
        }
        found
    }

    /// After the community launcher closed: [`Beacon::adopt_community_games`],
    /// and when that finds nothing, a full scan (also of the launchers' folder).
    /// Returns where the game was found (empty: not found, or it already had a folder).
    pub fn rescan_after_community_launcher(&mut self, g: GameId) -> Vec<FoundGame> {
        if self.game_dir(g).is_some() {
            return Vec::new();
        }
        let found = self.adopt_community_games(g);
        if !found.is_empty() {
            return found;
        }
        let tools = self.community_launcher_dir(g);
        self.scan(&[tools]).into_iter().filter(|f| f.game == g).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use std::fs;

    fn beacon(t: &Path) -> Beacon {
        let mut b = Beacon::load_with(Paths::under(&t.join("beacon"), &t.join("home")), "").unwrap();
        b.native = false;
        b
    }

    #[test]
    fn only_uaw_has_one_and_never_a_download_url() {
        assert_eq!(launcher_url(GameId::Uaw), Some("https://drive.google.com/drive/folders/167UET84ZWXrMd9zpxEYiQWTQZexCGZfT"));
        for g in GameId::ALL.into_iter().filter(|g| *g != GameId::Uaw) {
            assert_eq!(launcher_url(g), None, "{g:?}");
        }
        for (_, u) in LAUNCHER_URLS {
            assert!(u.starts_with("https://") && !u.chars().any(|c| c.is_whitespace()), "{u}");
        }
    }

    #[test]
    fn validation() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("UaW Launcher.EXE");
        let zip = t.path().join("launcher-1.2.Zip");
        let txt = t.path().join("readme.txt");
        for f in [&exe, &zip, &txt] {
            fs::write(f, b"MZ").unwrap();
        }
        assert_eq!(validate_source(&exe).unwrap(), SourceKind::Exe);
        assert_eq!(validate_source(&zip).unwrap(), SourceKind::Zip);
        assert!(validate_source(&txt).unwrap_err().to_string().contains(".exe or the archive"));
        assert!(validate_source(Path::new("relative/launcher.exe")).unwrap_err().to_string().contains("isn't absolute"));
        assert!(validate_source(&t.path().join("gone.exe")).unwrap_err().to_string().contains("not found"));
        let dir = t.path().join("folder.exe");
        fs::create_dir_all(&dir).unwrap();
        assert!(validate_source(&dir).unwrap_err().to_string().contains("isn't a file"));
    }

    #[test]
    fn picks_the_launcher_from_an_extracted_folder() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        for f in ["unins000.exe", "vc_redist.x86.exe", "Tools/CrashReporter.exe", "Tools/helper.exe", "Bin/UaW Community Launcher.exe", "Setup.exe", "readme.txt"] {
            let p = d.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"MZ").unwrap();
        }
        assert_eq!(pick_exe(d).unwrap(), d.join("Bin/UaW Community Launcher.exe"), "a launcher name wins over depth");
        fs::remove_file(d.join("Bin/UaW Community Launcher.exe")).unwrap();
        assert_eq!(pick_exe(d).unwrap(), d.join("Tools/helper.exe"), "setup names come last");
        fs::write(d.join("UaW.exe"), b"MZ").unwrap();
        assert_eq!(pick_exe(d).unwrap(), d.join("UaW.exe"), "then the shallowest");
        let only_helpers = t.path().join("h");
        fs::create_dir_all(&only_helpers).unwrap();
        fs::write(only_helpers.join("uninstall.exe"), b"MZ").unwrap();
        assert_eq!(pick_exe(&only_helpers), None);
        assert_eq!(pick_exe(&t.path().join("missing")), None);
    }

    #[test]
    fn folder_names() {
        assert_eq!(folder_name(Path::new("/d/UaW Launcher v1.2.zip")), "UaW Launcher v1.2");
        assert_eq!(folder_name(Path::new("/d/a<b>:c|.exe")), "a b c");
        assert_eq!(folder_name(Path::new("/d/CON.exe")), "launcher");
        assert_eq!(folder_name(Path::new("/d/....exe")), "launcher");
        assert_eq!(folder_name(Path::new(&format!("/d/x{}.exe", "y".repeat(100)))).chars().count(), 60);
    }

    #[test]
    fn choose_a_standalone_exe() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        let st = b.community_launcher_status(GameId::Uaw).unwrap();
        assert!(st.path.is_none() && !st.missing && !st.can_open);
        assert!(b.community_launcher_status(GameId::Swgb).is_none());
        assert_eq!(b.community_launcher_ref(GameId::Uaw).unwrap(), LauncherRef { path: None, url: launcher_url(GameId::Uaw).unwrap().into() });
        assert!(b.community_launcher_command(GameId::Uaw).unwrap_err().to_string().contains("no UaW launcher chosen"));

        let dl = t.path().join("Downloads");
        fs::create_dir_all(&dl).unwrap();
        let src = dl.join("UaW Launcher 1.3.exe");
        fs::write(&src, b"MZ launcher").unwrap();
        let exe = b.set_community_launcher(GameId::Uaw, &src, &mut |_: &str| {}).unwrap();
        assert_eq!(exe, b.community_launcher_dir(GameId::Uaw).join("UaW Launcher 1.3/UaW Launcher 1.3.exe"));
        assert_eq!(fs::read(&exe).unwrap(), b"MZ launcher");
        assert!(src.is_file(), "the download is left alone");
        assert_eq!(b.game(GameId::Uaw).community_launcher.as_deref(), Some(exe.as_path()));
        assert_eq!(b.status(GameId::Uaw).community_launcher.unwrap().path.as_deref(), Some(exe.as_path()));
        assert!(b.status(GameId::Swgb).community_launcher.is_none());

        // Choosing Beacon's own copy again keeps it where it is.
        assert_eq!(b.set_community_launcher(GameId::Uaw, &exe, &mut |_: &str| {}).unwrap(), exe);
        assert!(exe.is_file());
        // Choosing the same download again replaces the copy.
        fs::write(&src, b"MZ newer").unwrap();
        assert_eq!(b.set_community_launcher(GameId::Uaw, &src, &mut |_: &str| {}).unwrap(), exe);
        assert_eq!(fs::read(&exe).unwrap(), b"MZ newer");
        assert!(!b.community_launcher_dir(GameId::Uaw).join(".UaW Launcher 1.3.beacon-tmp").exists());

        // Not a Windows program, a game without one: refused, nothing changes.
        let fake = dl.join("fake.exe");
        fs::write(&fake, b"<html>").unwrap();
        assert!(b.set_community_launcher(GameId::Uaw, &fake, &mut |_: &str| {}).unwrap_err().to_string().contains("isn't a Windows program"));
        assert!(!b.community_launcher_dir(GameId::Uaw).join("fake").exists());
        assert!(b.set_community_launcher(GameId::Swgb, &src, &mut |_: &str| {}).unwrap_err().to_string().contains("no community launcher"));
        assert_eq!(b.game(GameId::Uaw).community_launcher.as_deref(), Some(exe.as_path()));

        // The copy is gone: reported, and open says so.
        fs::remove_dir_all(exe.parent().unwrap()).unwrap();
        let st = b.community_launcher_status(GameId::Uaw).unwrap();
        assert!(st.missing && st.path.is_none() && !st.can_open);
        assert!(b.community_launcher_command(GameId::Uaw).unwrap_err().to_string().contains("is gone"));
    }

    /// The launcher runs in UaW's own prefix, through UaW's runner.
    #[cfg(unix)]
    #[test]
    fn opens_in_the_games_prefix() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        fs::write(&wine, "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/users/tester\"\nprintf '%s\\n' \"$*\" >> \"$WINEPREFIX/../calls.txt\"\n").unwrap();
        fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();
        let mut b = Beacon::load_with(Paths::under(&t.path().join("beacon"), &t.path().join("home")), &bin.to_string_lossy()).unwrap();
        b.native = false;
        let src = t.path().join("UaWLauncher.exe");
        fs::write(&src, b"MZ").unwrap();
        let exe = b.set_community_launcher(GameId::Uaw, &src, &mut |_: &str| {}).unwrap();
        assert!(b.community_launcher_status(GameId::Uaw).unwrap().can_open);
        let c = b.community_launcher_command(GameId::Uaw).unwrap();
        assert_eq!(c.program, wine);
        assert_eq!(c.args, vec![exe.to_string_lossy().into_owned()]);
        let pfx = b.paths.prefix_root(GameId::Uaw).join("pfx");
        assert!(c.env.contains(&("WINEPREFIX".to_string(), pfx.to_string_lossy().into_owned())));
        assert_eq!(c.cwd, exe.parent().unwrap());
        let mut child = b.open_community_launcher(GameId::Uaw, &mut |_: Option<u8>, _: &str| {}).unwrap();
        assert!(child.wait().unwrap().success());
        let calls = fs::read_to_string(b.paths.prefix_root(GameId::Uaw).join("calls.txt")).unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), vec!["wineboot -u".to_string(), exe.to_string_lossy().into_owned()]);
        let log = fs::read_to_string(b.community_launcher_log(GameId::Uaw)).unwrap();
        assert!(log.contains("== beacon prefix:") && log.contains("== beacon open:"));
        // The prefix exists now: no second wineboot.
        b.open_community_launcher(GameId::Uaw, &mut |_: Option<u8>, _: &str| {}).unwrap().wait().unwrap();
        let calls = fs::read_to_string(b.paths.prefix_root(GameId::Uaw).join("calls.txt")).unwrap();
        assert_eq!(calls.lines().filter(|l| *l == "wineboot -u").count(), 1);
    }

    /// The game the launcher downloaded into UaW's prefix is found without a full scan, and by a full scan.
    #[test]
    fn finds_the_game_in_its_prefix() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        assert!(b.adopt_community_games(GameId::Uaw).is_empty(), "no prefix yet");
        let game = b.paths.prefix_root(GameId::Uaw).join("pfx/drive_c/Games/Universe at War Earth Assault");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("UAWEA.exe"), b"").unwrap();
        assert!(!b.status(GameId::Uaw).installed);
        let got = b.adopt_community_games(GameId::Uaw);
        assert_eq!(got, vec![FoundGame { game: GameId::Uaw, dir: game.clone() }]);
        assert!(b.status(GameId::Uaw).installed);
        assert!(b.adopt_community_games(GameId::Uaw).is_empty(), "nothing new");
        assert!(b.rescan_after_community_launcher(GameId::Uaw).is_empty(), "already has its folder");
        assert!(b.adopt_community_games(GameId::Swgb).is_empty());

        let mut fresh = beacon(&t.path().join("other"));
        fresh.paths.data_dir = b.paths.data_dir.clone();
        let found = fresh.scan(&[]);
        assert!(found.iter().any(|f| f.game == GameId::Uaw && f.dir == game), "{found:?}");
    }
}
