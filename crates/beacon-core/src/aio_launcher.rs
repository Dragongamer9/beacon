//! Getting the BFME Foundation's All-in-One launcher itself: install it from
//! the Foundation's official download, open it, and pick up the games it
//! installs.
//!
//! **Beacon never downloads the games.** It downloads the *launcher* (the
//! Foundation's own program); the player downloads BFME, BFME II and RotWK
//! inside it, and Beacon then finds those installs (see [`crate::aio`]).
//!
//! Where the launcher comes from, read from its source
//! (github.com/MarcellVokk/BfmeFoundationProject,
//! `BfmeFoundationProject_AllInOneLauncher/Data/Consts.cs` and
//! `Core/LauncherUpdateManager.cs`): the launcher has no separate installer and
//! no GitHub release assets. It's a single self-contained .NET 8 exe
//! (`dotnet publish --self-contained -p:PublishSingleFile=true`, see its
//! `HowToPublish.txt`) that lives at
//! `%APPDATA%\BFME All In One Launcher\AllInOneLauncher.exe` and updates itself
//! from two fixed URLs:
//!
//! * [`BUILD_URL`]: the current build (the exe itself);
//! * [`VERSION_HASH_URL`]: that build's MD5, lowercase hex (the launcher's
//!   own "version": it compares it with the MD5 of its exe to decide whether
//!   to update).
//!
//! On its first start from that folder it creates its shortcuts and its
//! `Uninstall\All In One Launcher` entry (`InstallLocation` = that folder);
//! started from anywhere else while that exe exists, it refuses ("already
//! installed"). Beacon does what the launcher's own updater does: download
//! the build into that folder (https only, size-capped, MD5 checked when the
//! version answer is readable, `MZ` header checked), then start it there.
//!
//! * **Linux:** a Wine prefix of its own, `<data>/prefixes/aio-launcher/pfx`,
//!   created with `wineboot -u` through the default runner (no extra runtime:
//!   the exe carries .NET itself). The launcher's game folders
//!   (`C:\BFME1`…) land in that prefix's `drive_c`, which `scan` already
//!   walks (`<data>/prefixes`), and [`Beacon::adopt_launcher_games`] picks
//!   them up without a full scan.
//! * **Windows:** the real `%APPDATA%`; the exe asks for administrator rights
//!   (its manifest says `requireAdministrator`), so it's started through the
//!   UAC prompt ([`crate::native`]).

use crate::games::{self, FoundGame, GameId};
use crate::installs::{self, RegValue};
use crate::runner::{self, Command, Runner};
use crate::workshop::Http;
use crate::{options_ini, Beacon};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

/// The launcher's current build, as its own updater downloads it.
pub const BUILD_URL: &str = "https://arena-files.bfmeladder.com/application-builds/all-in-one-launcher-main";
/// The current build's MD5 (lowercase hex), as its own updater reads it.
pub const VERSION_HASH_URL: &str = "https://bfmeladder.com/api/applications/versionHash?name=all-in-one-launcher&version=main";
/// The launcher's folder under `%APPDATA%`.
pub const APP_DIR_NAME: &str = "BFME All In One Launcher";
pub const EXE_NAME: &str = "AllInOneLauncher.exe";
/// Its `…\CurrentVersion\Uninstall\<key>` entry.
pub const UNINSTALL_KEY: &str = "All In One Launcher";
/// Beacon's prefix folder for it (`<data>/prefixes/<this>`).
pub const PREFIX_DIR: &str = "aio-launcher";
/// Largest download accepted (the self-contained exe is a few hundred MB).
pub const MAX_BYTES: u64 = 1 << 30;
/// Smallest: anything less is an error page, not the launcher.
pub const MIN_BYTES: u64 = 1 << 20;
/// Creating the prefix: umu's first run also downloads the Steam Linux Runtime.
const PREFIX_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// What `aio-launcher status` (and `aio`'s `launcher`) report.
#[derive(Debug, Clone, Serialize)]
pub struct LauncherStatus {
    pub installed: bool,
    /// The launcher's exe.
    pub path: Option<PathBuf>,
    /// Linux: the Wine prefix it runs in (`<data>/prefixes/aio-launcher/pfx`); `null` on Windows.
    pub prefix: Option<PathBuf>,
    /// The build's MD5 (the launcher's own version id) as recorded when Beacon
    /// installed it; `null` when Beacon didn't install it or it has updated itself since.
    pub version: Option<String>,
    /// Beacon installed this copy.
    pub by_beacon: bool,
    /// `install` can run: on Windows always, on Linux when there's a runner.
    pub can_install: bool,
    /// Linux: the runner it runs with (the default runner, else the best one).
    pub runner: Option<String>,
    /// Where `install` downloads it from, and the Foundation's site.
    pub download_url: &'static str,
    pub site: &'static str,
    /// Beacon's log for the launcher (prefix creation, starts).
    pub log: PathBuf,
}

/// What `install` did.
#[derive(Debug, Clone, Serialize)]
pub struct LauncherInstall {
    pub path: PathBuf,
    pub prefix: Option<PathBuf>,
    pub bytes: u64,
    pub md5: String,
    /// The MD5 matched the one the Foundation publishes (false: that answer couldn't be read).
    pub verified: bool,
}

/// `<data>/aio-launcher.json`: what Beacon installed.
#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    path: PathBuf,
    md5: String,
    bytes: u64,
}

/// The MD5 from the version answer: 32 hex digits, maybe quoted or with a BOM.
pub fn parse_version_hash(text: &str) -> Option<String> {
    let t = text.trim().trim_start_matches('\u{feff}').trim().trim_matches('"').trim();
    (t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit())).then(|| t.to_ascii_lowercase())
}

/// `<appdata>\BFME All In One Launcher\AllInOneLauncher.exe`.
pub fn exe_in_appdata(appdata: &Path) -> PathBuf {
    appdata.join(APP_DIR_NAME).join(EXE_NAME)
}

/// The launcher's exe in a Wine prefix (any user's `%APPDATA%`).
pub fn find_in_prefix(pfx: &Path) -> Option<PathBuf> {
    let mut users: Vec<PathBuf> = std::fs::read_dir(pfx.join("drive_c").join("users"))
        .ok()?
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect();
    users.sort();
    users.into_iter().find_map(|u| {
        [u.join("AppData").join("Roaming"), u.join("Application Data")].into_iter().map(|a| exe_in_appdata(&a)).find(|e| e.is_file())
    })
}

/// The exe from the launcher's `Uninstall` entry (a Windows path): its
/// `InstallLocation` plus the exe name, or else its `DisplayIcon`.
pub fn exe_from_uninstall(values: &[RegValue]) -> Option<String> {
    let mine = |v: &RegValue| {
        v.key.replace('/', "\\").trim_end_matches('\\').rsplit('\\').next().map(|t| t.eq_ignore_ascii_case(UNINSTALL_KEY)).unwrap_or(false)
    };
    let get = |name: &str| {
        values
            .iter()
            .filter(|v| mine(v))
            .find(|v| v.name.eq_ignore_ascii_case(name))
            .map(|v| v.data.trim().trim_matches('"').trim().to_string())
            .filter(|s| !s.is_empty())
    };
    if let Some(dir) = get("InstallLocation") {
        return Some(format!("{}\\{EXE_NAME}", dir.trim_end_matches(['\\', '/'])));
    }
    get("DisplayIcon")
        .map(|i| i.split(',').next().unwrap_or("").trim().trim_matches('"').to_string())
        .filter(|i| i.to_ascii_lowercase().ends_with(".exe"))
}

/// The launcher's `Uninstall` entry on this PC (both views, machine and user).
#[cfg(windows)]
fn uninstall_values() -> Vec<RegValue> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let mut out = Vec::new();
    for hive in [RegKey::predef(HKEY_LOCAL_MACHINE), RegKey::predef(HKEY_CURRENT_USER)] {
        for view in ["SOFTWARE", r"SOFTWARE\WOW6432Node"] {
            let path = format!(r"{view}\Microsoft\Windows\CurrentVersion\Uninstall\{UNINSTALL_KEY}");
            let Ok(k) = hive.open_subkey(&path) else { continue };
            for (name, _) in k.enum_values().flatten() {
                if let Ok(data) = k.get_value::<String, _>(&name) {
                    out.push(RegValue { key: path.clone(), name, data });
                }
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn uninstall_values() -> Vec<RegValue> {
    Vec::new()
}

/// Is there a user folder in the prefix yet (made by `wineboot`)?
fn prefix_has_user(pfx: &Path) -> bool {
    std::fs::read_dir(pfx.join("drive_c").join("users"))
        .map(|rd| rd.flatten().any(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false) && !e.file_name().to_string_lossy().eq_ignore_ascii_case("public")))
        .unwrap_or(false)
}

/// Wait for a child, killing it after `limit`.
fn wait_with_timeout(child: &mut std::process::Child, limit: Duration) -> Result<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        if let Some(st) = child.try_wait()? {
            return Ok(st);
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!("it didn't finish within {} minutes and was stopped", limit.as_secs() / 60);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// A file writer that hashes what it writes and keeps the first two bytes.
struct Checked {
    file: std::fs::File,
    md5: md5::Md5,
    head: Vec<u8>,
}

impl Write for Checked {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use md5::Digest;
        let n = self.file.write(buf)?;
        self.md5.update(&buf[..n]);
        if self.head.len() < 2 {
            let take = (2 - self.head.len()).min(n);
            self.head.extend_from_slice(&buf[..take]);
        }
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Download the build into `part` and check it. Returns (bytes, md5).
fn download_checked(http: &dyn Http, part: &Path, expected: Option<&str>, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<(u64, String)> {
    use md5::Digest;
    let file = std::fs::File::create(part).with_context(|| format!("creating {}", part.display()))?;
    let mut out = Checked { file, md5: md5::Md5::new(), head: Vec::new() };
    let mut last_mb = u64::MAX;
    let bytes = http.download(BUILD_URL, &mut out, MAX_BYTES, &mut |n: u64| {
        let mb = n >> 20;
        if last_mb == u64::MAX || mb >= last_mb + 4 {
            last_mb = mb;
            progress(None, &format!("Downloading the All-in-One launcher from the BFME Foundation… {mb} MB"));
        }
    })?;
    out.flush()?;
    out.file.sync_all().ok();
    let md5: String = out.md5.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if out.head != b"MZ" {
        bail!("the download isn't a Windows program; the BFME Foundation's server may be having trouble — try again later or get the launcher at {}", crate::aio::LAUNCHER_URL);
    }
    if bytes < MIN_BYTES {
        bail!("the download is only {bytes} bytes, too small to be the launcher");
    }
    if let Some(want) = expected {
        if md5 != want {
            bail!("the download doesn't match the checksum the BFME Foundation publishes (got {md5}, expected {want}); nothing was installed");
        }
    }
    Ok((bytes, md5))
}

impl Beacon {
    /// `<data>/prefixes/aio-launcher` (the prefix itself is its `pfx`).
    pub fn launcher_prefix_root(&self) -> PathBuf {
        self.paths.data_dir.join("prefixes").join(PREFIX_DIR)
    }

    pub fn launcher_prefix(&self) -> PathBuf {
        self.launcher_prefix_root().join("pfx")
    }

    pub fn launcher_log(&self) -> PathBuf {
        self.paths.data_dir.join("logs").join("aio-launcher.log")
    }

    fn launcher_marker(&self) -> PathBuf {
        self.paths.data_dir.join("aio-launcher.json")
    }

    /// Linux: the default runner, else the best one found.
    pub fn launcher_runner(&self) -> Option<&Runner> {
        self.config.default_runner.as_deref().and_then(|id| self.runners.get(id)).or_else(|| self.runners.best())
    }

    /// The installed launcher's exe: Windows, `%APPDATA%` or its `Uninstall`
    /// entry; Linux, Beacon's launcher prefix.
    pub fn launcher_exe(&self) -> Option<PathBuf> {
        if self.native {
            if let Some(exe) = self.paths.appdata.as_deref().map(exe_in_appdata).filter(|e| e.is_file()) {
                return Some(exe);
            }
            return exe_from_uninstall(&uninstall_values()).map(PathBuf::from).filter(|p| p.is_file());
        }
        find_in_prefix(&self.launcher_prefix())
    }

    pub fn launcher_status(&self) -> LauncherStatus {
        let path = self.launcher_exe();
        let marker: Option<Marker> = std::fs::read_to_string(self.launcher_marker()).ok().and_then(|s| serde_json::from_str(&s).ok());
        let by_beacon = matches!((&marker, &path), (Some(m), Some(p)) if &m.path == p);
        // Same size as Beacon's download: still that build (it replaces itself when it updates).
        let version = marker
            .filter(|m| by_beacon && path.as_deref().and_then(|p| std::fs::metadata(p).ok()).map(|md| md.len() == m.bytes).unwrap_or(false))
            .map(|m| m.md5);
        let runner = if self.native { None } else { self.launcher_runner().map(|r| r.id.clone()) };
        LauncherStatus {
            installed: path.is_some(),
            path,
            prefix: (!self.native).then(|| self.launcher_prefix()),
            version,
            by_beacon,
            can_install: self.native || runner.is_some(),
            runner,
            download_url: BUILD_URL,
            site: crate::aio::LAUNCHER_URL,
            log: self.launcher_log(),
        }
    }

    fn open_launcher_log(&self, cmd: &Command, what: &str, append: bool) -> Result<std::fs::File> {
        let path = self.launcher_log();
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let mut f = std::fs::OpenOptions::new().create(true).append(append).write(true).truncate(!append).open(&path)?;
        writeln!(f, "== beacon {what}: {}", cmd.display())?;
        Ok(f)
    }

    /// Linux: create the launcher's prefix if it has no user folder yet. Returns its `%APPDATA%`.
    fn prepare_launcher_prefix(&self, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<PathBuf> {
        let runner = self
            .launcher_runner()
            .ok_or_else(|| anyhow!("no Wine, Proton or umu-launcher found; install GE-Proton, umu-launcher or Wine first"))?;
        let root = self.launcher_prefix_root();
        std::fs::create_dir_all(&root)?;
        let pfx = self.launcher_prefix();
        if !prefix_has_user(&pfx) {
            progress(Some(5), "Creating the launcher's Wine prefix (the first time can take a few minutes)");
            let cmd = runner::command_for(runner, self.runners.umu.as_deref(), &self.paths.home, &root, "wineboot", &["-u".to_string()], &root);
            let log = self.open_launcher_log(&cmd, "prefix", false)?;
            let mut child = cmd
                .to_std()
                .stdin(Stdio::null())
                .stdout(log.try_clone()?)
                .stderr(log)
                .spawn()
                .with_context(|| format!("starting {}", cmd.program.display()))?;
            let st = wait_with_timeout(&mut child, PREFIX_TIMEOUT).context("creating the launcher's Wine prefix")?;
            if !prefix_has_user(&pfx) {
                bail!("creating the launcher's Wine prefix failed ({st}); see {}", self.launcher_log().display());
            }
        }
        Ok(options_ini::appdata_dir(&pfx))
    }

    /// Download the All-in-One launcher from the BFME Foundation's official
    /// URL and put it where it installs itself (Linux: in its own Wine prefix).
    /// `progress(pct, text)` reports along the way. Start it with [`Beacon::open_launcher`].
    pub fn install_launcher(&self, http: &dyn Http, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<LauncherInstall> {
        progress(Some(1), "Asking the BFME Foundation for the launcher's current version");
        // Best effort: when the answer can't be read the download is still https-only and checked for size and type.
        let expected = http.get_text(VERSION_HASH_URL, 4096).ok().and_then(|t| parse_version_hash(&t));
        let (appdata, prefix) = if self.native {
            (self.paths.appdata.clone().ok_or_else(|| anyhow!("no %APPDATA% folder found"))?, None)
        } else {
            (self.prepare_launcher_prefix(progress)?, Some(self.launcher_prefix()))
        };
        let dir = appdata.join(APP_DIR_NAME);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let exe = dir.join(EXE_NAME);
        let part = dir.join(format!("{EXE_NAME}.beacon-download"));
        progress(Some(20), "Downloading the All-in-One launcher from the BFME Foundation");
        let (bytes, md5) = match download_checked(http, &part, expected.as_deref(), progress) {
            Ok(x) => x,
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                return Err(e);
            }
        };
        progress(Some(96), "Installing the launcher");
        if let Err(e) = std::fs::rename(&part, &exe) {
            let _ = std::fs::remove_file(&part);
            return Err(e).with_context(|| format!("putting the launcher at {} (close the launcher if it's open)", exe.display()));
        }
        let marker = Marker { path: exe.clone(), md5: md5.clone(), bytes };
        std::fs::write(self.launcher_marker(), serde_json::to_string(&marker)?)?;
        progress(Some(100), "The All-in-One launcher is installed");
        Ok(LauncherInstall { path: exe, prefix, bytes, md5, verified: expected.is_some() })
    }

    /// What `open_launcher` runs.
    pub fn launcher_command(&self) -> Result<Command> {
        let exe = self.launcher_exe().ok_or_else(|| anyhow!("the All-in-One launcher isn't installed yet (Get the All-in-One launcher, or `beacon aio-launcher install`)"))?;
        let cwd = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| self.paths.data_dir.clone());
        if self.native {
            return Ok(Command { program: exe, args: vec![], env: vec![], cwd });
        }
        let runner = self.launcher_runner().ok_or_else(|| anyhow!("no Wine, Proton or umu-launcher found"))?;
        Ok(runner::command_for(runner, self.runners.umu.as_deref(), &self.paths.home, &self.launcher_prefix_root(), &exe.to_string_lossy(), &[], &cwd))
    }

    /// Start the launcher, detached from Beacon (its own process group on
    /// Linux; through the UAC prompt on Windows). Output goes to its log.
    pub fn open_launcher(&self) -> Result<std::process::Child> {
        let cmd = self.launcher_command()?;
        let log = self.open_launcher_log(&cmd, "open", true)?;
        let mut c = cmd.to_std();
        c.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        match c.spawn() {
            Ok(child) => Ok(child),
            // Its manifest asks for administrator rights.
            #[cfg(windows)]
            Err(e) if e.raw_os_error() == Some(crate::native::ERROR_ELEVATION_REQUIRED) => crate::native::spawn_elevated(&self.paths.data_dir, &cmd),
            Err(e) => Err(e).with_context(|| format!("starting {}", cmd.program.display())),
        }
    }

    /// BFME games the launcher installed that Beacon has no folder for yet:
    /// fill them in (cheap, unlike a full `scan`). Linux: its prefix's
    /// `drive_c` and the folders its registry names; Windows: the folders the
    /// registry names, once the launcher is installed. Returns what it adopted.
    pub fn adopt_launcher_games(&mut self) -> Vec<FoundGame> {
        let bfme = [GameId::Bfme1, GameId::Bfme2, GameId::Rotwk];
        if bfme.iter().all(|g| self.game_dir(*g).is_some()) {
            return Vec::new();
        }
        let mut roots: Vec<PathBuf> = Vec::new();
        if self.native {
            if self.launcher_exe().is_none() {
                return Vec::new();
            }
            roots.extend(installs::hints(&installs::windows_registry()).into_iter().filter(|(g, _)| matches!(g, Some(x) if bfme.contains(x))).map(|(_, d)| PathBuf::from(d)));
        } else {
            let pfx = self.launcher_prefix();
            if !pfx.join("drive_c").is_dir() {
                return Vec::new();
            }
            roots.push(pfx.join("drive_c"));
            roots.extend(installs::prefix_hints(&pfx).into_iter().map(|(_, d)| d));
        }
        let mut found: Vec<FoundGame> = Vec::new();
        for r in roots {
            for f in games::scan(std::slice::from_ref(&r), 2) {
                if bfme.contains(&f.game) && !found.contains(&f) {
                    found.push(f);
                }
            }
        }
        let mut adopted = Vec::new();
        for f in found {
            if self.game_dir(f.game).is_none() {
                self.game_mut(f.game).path = Some(f.dir.clone());
                adopted.push(f);
            }
        }
        adopted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use serde_json::Value;
    use std::cell::RefCell;
    use std::fs;

    #[test]
    fn official_urls() {
        for u in [BUILD_URL, VERSION_HASH_URL] {
            assert!(u.starts_with("https://") && u.contains("bfmeladder.com/"), "{u}");
        }
    }

    #[test]
    fn version_answers() {
        let h = "0123456789abcdef0123456789ABCDEF";
        assert_eq!(parse_version_hash(h).as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(parse_version_hash(&format!("\u{feff}\"{h}\"\r\n")).as_deref(), Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(parse_version_hash(""), None);
        assert_eq!(parse_version_hash("<html>error</html>"), None);
        assert_eq!(parse_version_hash(&h[..31]), None);
        assert_eq!(parse_version_hash("H4sIAAAAAAAA/ytJLS4BAAx+f9gEAAAA"), None, "base64-gzip isn't taken as a hash");
    }

    #[test]
    fn uninstall_entry() {
        let v = |key: &str, name: &str, data: &str| RegValue { key: key.into(), name: name.into(), data: data.into() };
        let key = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\All In One Launcher";
        let vals = vec![
            v(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\All In One Launcher_is1", "InstallLocation", r"C:\Old"),
            v(key, "DisplayIcon", r"C:\Users\me\AppData\Roaming\BFME All In One Launcher\AllInOneLauncher.exe"),
            v(key, "InstallLocation", r"C:\Users\me\AppData\Roaming\BFME All In One Launcher\"),
            v(key, "UninstallString", r"C:\x\AllInOneLauncher.exe --Uninstall"),
        ];
        assert_eq!(exe_from_uninstall(&vals).as_deref(), Some(r"C:\Users\me\AppData\Roaming\BFME All In One Launcher\AllInOneLauncher.exe"));
        let icon_only = vec![v(key, "DisplayIcon", r#""D:\L\AllInOneLauncher.exe",0"#)];
        assert_eq!(exe_from_uninstall(&icon_only).as_deref(), Some(r"D:\L\AllInOneLauncher.exe"));
        assert_eq!(exe_from_uninstall(&vals[..1]), None, "the old installer's entry isn't the launcher");
        assert_eq!(exe_from_uninstall(&[]), None);
    }

    #[test]
    fn exe_in_a_prefix() {
        let t = tempfile::tempdir().unwrap();
        let pfx = t.path().join("pfx");
        fs::create_dir_all(pfx.join("drive_c/users/Public")).unwrap();
        assert_eq!(find_in_prefix(&pfx), None);
        assert!(!prefix_has_user(&pfx), "Public alone isn't a user");
        let exe = exe_in_appdata(&pfx.join("drive_c/users/steamuser/AppData/Roaming"));
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        assert_eq!(find_in_prefix(&pfx), None, "no exe yet");
        assert!(prefix_has_user(&pfx));
        fs::write(&exe, b"MZ").unwrap();
        assert_eq!(find_in_prefix(&pfx), Some(exe));
        // Old Wine's "Application Data".
        let t2 = tempfile::tempdir().unwrap();
        let old = exe_in_appdata(&t2.path().join("drive_c/users/me/Application Data"));
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, b"MZ").unwrap();
        assert_eq!(find_in_prefix(t2.path()), Some(old));
    }

    /// Serves the version answer and the build.
    struct Fake {
        hash: Option<String>,
        body: Vec<u8>,
        asked: RefCell<Vec<String>>,
    }

    impl Http for Fake {
        fn get_json(&self, _: &str, _: &[(&str, String)]) -> Result<Value> {
            bail!("not used")
        }
        fn download(&self, url: &str, out: &mut dyn Write, max: u64, progress: &mut dyn FnMut(u64)) -> Result<u64> {
            self.asked.borrow_mut().push(url.to_string());
            let data = if url == VERSION_HASH_URL {
                self.hash.clone().ok_or_else(|| anyhow!("HTTP 500"))?.into_bytes()
            } else if url == BUILD_URL {
                self.body.clone()
            } else {
                bail!("unexpected {url}")
            };
            if data.len() as u64 > max {
                bail!("too large");
            }
            out.write_all(&data)?;
            progress(data.len() as u64);
            Ok(data.len() as u64)
        }
    }

    fn build() -> Vec<u8> {
        let mut b = b"MZ".to_vec();
        b.resize(MIN_BYTES as usize + 10, 7);
        b
    }

    fn md5_of(data: &[u8]) -> String {
        use md5::Digest;
        md5::Md5::digest(data).iter().map(|b| format!("{b:02x}")).collect()
    }

    fn native_beacon(t: &Path) -> Beacon {
        let mut b = Beacon::load_with(Paths::under(&t.join("beacon"), &t.join("home")), "").unwrap();
        b.native = true;
        b.runners = runner::RunnerSet::native();
        b.paths.appdata = Some(t.join("home/AppData/Roaming"));
        b
    }

    /// Windows behaviour, on any OS: into %APPDATA%, checked against the published MD5.
    #[test]
    fn install_into_appdata() {
        let t = tempfile::tempdir().unwrap();
        let b = native_beacon(t.path());
        let st = b.launcher_status();
        assert!(!st.installed && st.can_install && st.prefix.is_none() && st.path.is_none());
        assert!(b.launcher_command().unwrap_err().to_string().contains("isn't installed"));

        let body = build();
        let fake = Fake { hash: Some(md5_of(&body).to_ascii_uppercase()), body: body.clone(), asked: RefCell::new(vec![]) };
        let mut lines = Vec::new();
        let done = b.install_launcher(&fake, &mut |p: Option<u8>, s: &str| lines.push((p, s.to_string()))).unwrap();
        let exe = t.path().join("home/AppData/Roaming/BFME All In One Launcher/AllInOneLauncher.exe");
        assert_eq!(done.path, exe);
        assert!(done.verified && done.bytes == body.len() as u64 && done.md5 == md5_of(&body));
        assert_eq!(fs::read(&exe).unwrap(), body);
        assert!(!exe.with_file_name("AllInOneLauncher.exe.beacon-download").exists());
        assert_eq!(*fake.asked.borrow(), vec![VERSION_HASH_URL.to_string(), BUILD_URL.to_string()]);
        assert!(lines.iter().any(|(p, _)| *p == Some(100)));

        let st = b.launcher_status();
        assert!(st.installed && st.by_beacon);
        assert_eq!(st.path.as_deref(), Some(exe.as_path()));
        assert_eq!(st.version.as_deref(), Some(md5_of(&body).as_str()));
        let c = b.launcher_command().unwrap();
        assert_eq!((c.program.clone(), c.args.len(), c.env.len()), (exe.clone(), 0, 0));
        assert_eq!(c.cwd, exe.parent().unwrap());
        // It updated itself: same path, other build — no longer the version Beacon recorded.
        fs::write(&exe, b"MZ newer build").unwrap();
        let st = b.launcher_status();
        assert!(st.installed && st.version.is_none());
    }

    #[test]
    fn bad_downloads_install_nothing() {
        let t = tempfile::tempdir().unwrap();
        let b = native_beacon(t.path());
        let exe = t.path().join("home/AppData/Roaming/BFME All In One Launcher/AllInOneLauncher.exe");
        let part = exe.with_file_name("AllInOneLauncher.exe.beacon-download");

        let wrong = Fake { hash: Some("0".repeat(32)), body: build(), asked: RefCell::new(vec![]) };
        assert!(b.install_launcher(&wrong, &mut |_: Option<u8>, _: &str| {}).unwrap_err().to_string().contains("checksum"));
        let page = Fake { hash: None, body: b"<html>busy</html>".repeat(100_000), asked: RefCell::new(vec![]) };
        assert!(b.install_launcher(&page, &mut |_: Option<u8>, _: &str| {}).unwrap_err().to_string().contains("isn't a Windows program"));
        let tiny = Fake { hash: None, body: b"MZ tiny".to_vec(), asked: RefCell::new(vec![]) };
        assert!(b.install_launcher(&tiny, &mut |_: Option<u8>, _: &str| {}).unwrap_err().to_string().contains("too small"));
        assert!(!exe.exists() && !part.exists());
        assert!(!b.launcher_status().installed);

        // No readable version answer: installed, but not verified.
        let unverified = Fake { hash: Some("H4sI-not-a-hash".into()), body: build(), asked: RefCell::new(vec![]) };
        assert!(!b.install_launcher(&unverified, &mut |_: Option<u8>, _: &str| {}).unwrap().verified);
        assert!(exe.is_file());
    }

    /// Linux: its own prefix (made with the runner's `wineboot -u`), started through the runner.
    #[cfg(unix)]
    #[test]
    fn install_and_open_in_a_prefix() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let bin = t.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        // Stub wine: makes the prefix's user folder like wineboot does, and records its arguments.
        fs::write(&wine, "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/users/tester\"\nprintf '%s\\n' \"$*\" >> \"$WINEPREFIX/../calls.txt\"\n").unwrap();
        fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();
        let mut b = Beacon::load_with(Paths::under(&t.path().join("beacon"), &t.path().join("home")), &bin.to_string_lossy()).unwrap();
        b.native = false;
        let st = b.launcher_status();
        assert!(!st.installed && st.can_install);
        assert_eq!(st.runner.as_deref(), Some("wine:system"));
        assert_eq!(st.prefix.as_deref(), Some(t.path().join("beacon/data/prefixes/aio-launcher/pfx").as_path()));

        let body = build();
        let fake = Fake { hash: Some(md5_of(&body)), body, asked: RefCell::new(vec![]) };
        let done = b.install_launcher(&fake, &mut |_: Option<u8>, _: &str| {}).unwrap();
        let pfx = b.launcher_prefix();
        assert_eq!(done.path, pfx.join("drive_c/users/tester/AppData/Roaming/BFME All In One Launcher/AllInOneLauncher.exe"));
        let calls = fs::read_to_string(b.launcher_prefix_root().join("calls.txt")).unwrap();
        assert_eq!(calls.lines().collect::<Vec<_>>(), vec!["wineboot -u"]);
        assert!(b.launcher_status().installed);

        // A second install doesn't make the prefix again.
        b.install_launcher(&fake, &mut |_: Option<u8>, _: &str| {}).unwrap();
        assert_eq!(fs::read_to_string(b.launcher_prefix_root().join("calls.txt")).unwrap().lines().count(), 1);

        let c = b.launcher_command().unwrap();
        assert_eq!(c.program, wine);
        assert_eq!(c.args, vec![done.path.to_string_lossy().into_owned()]);
        assert!(c.env.contains(&("WINEPREFIX".to_string(), pfx.to_string_lossy().into_owned())));
        let mut child = b.open_launcher().unwrap();
        assert!(child.wait().unwrap().success());
        assert!(fs::read_to_string(b.launcher_log()).unwrap().contains("== beacon open:"));
    }

    #[test]
    fn no_runner_no_install() {
        let t = tempfile::tempdir().unwrap();
        let mut b = Beacon::load_with(Paths::under(&t.path().join("beacon"), &t.path().join("home")), "").unwrap();
        b.native = false;
        b.runners = runner::RunnerSet::default();
        let st = b.launcher_status();
        assert!(!st.can_install && st.runner.is_none());
        let fake = Fake { hash: None, body: build(), asked: RefCell::new(vec![]) };
        assert!(b.install_launcher(&fake, &mut |_: Option<u8>, _: &str| {}).unwrap_err().to_string().contains("no Wine"));
    }

    /// Games the launcher downloaded into its prefix show up without a scan, and as the launcher's.
    #[test]
    fn games_in_the_launcher_prefix() {
        let t = tempfile::tempdir().unwrap();
        let mut b = Beacon::load_with(Paths::under(&t.path().join("beacon"), &t.path().join("home")), "").unwrap();
        b.native = false;
        assert!(b.adopt_launcher_games().is_empty(), "no prefix yet");
        let c = b.launcher_prefix().join("drive_c");
        for (d, exe) in [("BFME2", "lotrbfme2.exe"), ("RotWK", "lotrbfme2ep1.exe")] {
            fs::create_dir_all(c.join(d)).unwrap();
            fs::write(c.join(d).join(exe), b"").unwrap();
        }
        fs::create_dir_all(c.join("BFME Workshop/Mods")).unwrap();
        assert!(!b.status(GameId::Bfme2).installed);
        let got = b.adopt_launcher_games();
        assert_eq!(got.iter().map(|f| f.game).collect::<Vec<_>>(), vec![GameId::Bfme2, GameId::Rotwk]);
        assert!(b.status(GameId::Bfme2).installed && b.status(GameId::Rotwk).installed);
        assert_eq!(b.game(GameId::Bfme2).path.as_deref(), Some(c.join("BFME2").as_path()));
        assert!(b.adopt_launcher_games().is_empty(), "nothing new");
        let aio = b.aio_games();
        assert_eq!(aio.iter().map(|a| a.game).collect::<Vec<_>>(), vec![GameId::Bfme2, GameId::Rotwk]);
        // A full scan finds them too (Beacon's prefixes are a scan root).
        let mut fresh = Beacon::load_with(Paths::under(&t.path().join("beacon2"), &t.path().join("home")), "").unwrap();
        fresh.native = false;
        fresh.paths.data_dir = b.paths.data_dir.clone();
        let found = fresh.scan(&[]);
        assert!(found.iter().any(|f| f.game == GameId::Rotwk && f.dir == c.join("RotWK")), "{found:?}");
    }
}
