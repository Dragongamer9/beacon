//! Beacon updates itself from a public GitHub repo's
//! releases (the updates repo in `private.json`), no login.
//!
//! * **Check:** `releases/latest`, compared with this build's version.
//! * **Download:** the right package for how Beacon is installed, from the
//!   release's public download link, checked against the release's
//!   `SHA256SUMS`.
//! * **Install**, by how Beacon was installed:
//!   - Linux AppImage (`$APPIMAGE`): the new AppImage replaces the old file.
//!   - Linux tar.gz or any folder Beacon can write to: `beacon` and
//!     `beacon-app` are replaced next to the running ones.
//!   - Linux .deb/.rpm (`/usr/bin`): the package is installed with `pkexec`
//!     (a password prompt). On atomic Fedoras (rpm-ostree) the package is kept
//!     for you to install.
//!   - Windows, a folder Beacon can write to (the per-user installer, the
//!     portable zip): the running exes are renamed to `.old` (Windows allows
//!     that) and the new ones put in place; `.old` files go on the next start.
//!   - Windows, Program Files (the MSI): the new MSI runs, with the UAC prompt.
//!
//!   Then Beacon restarts (or, for the MSI, exits so the installer can work).

use super::{fetch_file, FileJob, Transfer};
use crate::Beacon;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// This build's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// `1.2.3` (a leading `v` and anything after `-` or `+` ignored) as numbers.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>());
    let a = it.next()?.ok()?;
    let b = it.next().unwrap_or(Ok(0)).ok()?;
    let c = it.next().unwrap_or(Ok(0)).ok()?;
    Some((a, b, c))
}

/// Is `candidate` newer than `current`?
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Asset {
    pub name: String,
    /// The public download link (`github.com/<repo>/releases/download/<tag>/<name>`).
    pub url: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Release {
    pub version: String,
    pub tag: String,
    pub url: String,
    pub notes: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Check {
    pub current: String,
    pub latest: Option<Release>,
    pub newer: bool,
    /// Why it couldn't look (no network, no public releases…).
    pub problem: Option<String>,
    /// How this copy would be updated.
    pub method: Method,
}

/// How Beacon is installed, so how it updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Method {
    /// Replace the AppImage file.
    AppImage { path: PathBuf },
    /// Replace the binaries in a folder Beacon can write to.
    Folder { dir: PathBuf },
    /// Install a system package with pkexec.
    Package { format: String },
    /// rpm-ostree and other systems Beacon can't update by itself: the package is kept for you.
    Manual { format: String },
    /// Windows, Program Files: run the MSI.
    Msi,
    /// Running from `cargo run` or somewhere unexpected.
    Unsupported { why: String },
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".beacon-write-test-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn on_path(prog: &str) -> bool {
    std::env::var_os("PATH").map(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file())).unwrap_or(false)
}

/// Work out how this copy updates. `exe` is the running `beacon-app` (or `beacon`).
pub fn method_for(exe: &Path, appimage: Option<&Path>) -> Method {
    if let Some(a) = appimage {
        return Method::AppImage { path: a.to_path_buf() };
    }
    let Some(dir) = exe.parent() else { return Method::Unsupported { why: "no folder for the running program".into() } };
    if dir.components().any(|c| c.as_os_str() == "target") {
        return Method::Unsupported { why: "a development build (cargo); rebuild it instead".into() };
    }
    if cfg!(windows) {
        return if writable(dir) { Method::Folder { dir: dir.to_path_buf() } } else { Method::Msi };
    }
    if writable(dir) {
        return Method::Folder { dir: dir.to_path_buf() };
    }
    let format = if on_path("dpkg") { "deb" } else { "rpm" }.to_string();
    if Path::new("/run/ostree-booted").exists() || !on_path("pkexec") {
        return Method::Manual { format };
    }
    Method::Package { format }
}

/// This copy's update method.
pub fn current_method() -> Method {
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file());
    match std::env::current_exe() {
        Ok(exe) => method_for(&exe, appimage.as_deref()),
        Err(e) => Method::Unsupported { why: format!("can't tell where Beacon runs from: {e}") },
    }
}

/// The release asset this method needs.
pub fn asset_name(m: &Method, version: &str) -> Option<String> {
    Some(match m {
        Method::AppImage { .. } => "Beacon-x86_64.AppImage".into(),
        Method::Folder { .. } if cfg!(windows) => "beacon-windows-x64.zip".into(),
        Method::Folder { .. } => "beacon-x86_64-linux.tar.gz".into(),
        Method::Package { format } | Method::Manual { format } if format == "deb" => format!("beacon_{version}_amd64.deb"),
        Method::Package { .. } | Method::Manual { .. } => format!("beacon-{version}-1.x86_64.rpm"),
        Method::Msi => format!("Beacon-{version}-x64.msi"),
        Method::Unsupported { .. } => return None,
    })
}

/// The newest full release of a public `repo`.
pub fn latest(t: &dyn Transfer, repo: &str) -> Result<Release> {
    let repo = super::sources::check_repo(repo)?;
    let h = [("Accept", "application/vnd.github+json".to_string()), ("X-GitHub-Api-Version", "2022-11-28".to_string())];
    let v = t.get_json(&format!("https://api.github.com/repos/{repo}/releases/latest"), &h).map_err(|e| {
        let m = format!("{e:#}");
        if m.contains("HTTP 404") {
            anyhow!("{m}: {repo} has no public releases (is the repo public, and has it made a release?)")
        } else if m.contains("HTTP 403") || m.contains("HTTP 429") {
            anyhow!("{m}: GitHub's limit for checks without a login (60 an hour) is used up; try again later")
        } else {
            e
        }
    })?;
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or_else(|| anyhow!("the release has no tag"))?.to_string();
    let assets = v
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Asset {
                name: a.get("name")?.as_str()?.to_string(),
                url: a.get("browser_download_url")?.as_str().filter(|u| super::is_https(u))?.to_string(),
                size: a.get("size").and_then(Value::as_u64),
            })
        })
        .collect();
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        url: v.get("html_url").and_then(Value::as_str).unwrap_or_default().to_string(),
        notes: v.get("body").and_then(Value::as_str).unwrap_or_default().chars().take(4000).collect(),
        tag,
        assets,
    })
}

/// Download one asset to `dest` (resumable).
fn download_asset(t: &dyn Transfer, a: &Asset, dest: &Path, progress: &mut dyn FnMut(u64)) -> Result<()> {
    fetch_file(t, &FileJob { url: a.url.clone(), dest: dest.to_path_buf(), size: a.size, md5: None }, false, progress)?;
    Ok(())
}

fn sha256_file(p: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The checksum `SHA256SUMS` lists for `name`.
pub fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let (hash, file) = l.trim().split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

/// What `apply` did and what's left to do.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "next", rename_all = "kebab-case")]
pub enum Applied {
    /// The new version is in place: start this program (then quit).
    Restart { program: PathBuf },
    /// An installer is running (the MSI): quit so it can replace the files.
    Quit,
    /// Downloaded but not installed: `file`, with how to install it.
    Manual { file: PathBuf, how: String },
}

impl Beacon {
    /// Is there a newer release in the updates repo?
    pub fn update_check(&self, t: &dyn Transfer) -> Check {
        let method = current_method();
        let s = self.download_store().settings();
        match latest(t, &s.sources_repo) {
            Ok(r) => Check { current: CURRENT.into(), newer: is_newer(&r.version, CURRENT), latest: Some(r), problem: None, method },
            Err(e) => Check { current: CURRENT.into(), latest: None, newer: false, problem: Some(format!("{e:#}")), method },
        }
    }

    /// Download the newest release for this copy, check it, and put it in place.
    pub fn update_apply(&self, t: &dyn Transfer, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<Applied> {
        let s = self.download_store().settings();
        progress(None, "Looking for the newest Beacon");
        let rel = latest(t, &s.sources_repo)?;
        if !is_newer(&rel.version, CURRENT) {
            bail!("Beacon {CURRENT} is already the newest");
        }
        let method = current_method();
        if let Method::Unsupported { why } = &method {
            bail!("this copy can't update itself: {why}");
        }
        let want = asset_name(&method, &rel.version).ok_or_else(|| anyhow!("no package for this kind of install"))?;
        let asset = rel.assets.iter().find(|a| a.name == want).ok_or_else(|| anyhow!("release {} has no {want}", rel.tag))?;
        let dir = self.download_store().dir.join("self-update").join(&rel.version);
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(&asset.name);
        let label = format!("Downloading Beacon {}", rel.version);
        let size = asset.size;
        download_asset(t, asset, &file, &mut |b| {
            let pct = size.filter(|s| *s > 0).map(|s| (b.saturating_mul(95) / s).min(95) as u8);
            progress(pct, &label);
        })?;
        progress(Some(96), "Checking the download");
        let sums_asset = rel.assets.iter().find(|a| a.name == "SHA256SUMS").ok_or_else(|| anyhow!("release {} has no SHA256SUMS to check against", rel.tag))?;
        let sums_file = dir.join("SHA256SUMS");
        let _ = std::fs::remove_file(&sums_file);
        download_asset(t, sums_asset, &sums_file, &mut |_| {})?;
        let sums = std::fs::read_to_string(&sums_file)?;
        let want_sum = sum_for(&sums, &asset.name).ok_or_else(|| anyhow!("SHA256SUMS doesn't list {}", asset.name))?;
        let have = sha256_file(&file)?;
        if have != want_sum {
            let _ = std::fs::remove_file(&file);
            bail!("{} doesn't match its checksum (SHA-256 {have}, expected {want_sum}); nothing was changed", asset.name);
        }
        progress(Some(98), "Putting the new version in place");
        install(&method, &file, &dir, &self.paths.data_dir)
    }
}

/// Replace `target` with `new` (same folder): the new one is copied next to it, then renamed over it.
#[cfg(unix)]
fn replace_unix(new: &Path, target: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let staged = target.with_file_name(format!(".{}.new", target.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::copy(new, &staged).with_context(|| format!("copying the new version next to {}", target.display()))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&staged, target).with_context(|| format!("replacing {}", target.display()))?;
    Ok(())
}

/// Windows: a running exe can't be overwritten, but it can be renamed.
#[cfg(windows)]
fn replace_windows(new: &Path, target: &Path) -> Result<()> {
    let old = target.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    if target.exists() {
        std::fs::rename(target, &old).with_context(|| format!("moving {} aside", target.display()))?;
    }
    if let Err(e) = std::fs::copy(new, target) {
        let _ = std::fs::rename(&old, target);
        return Err(e).with_context(|| format!("putting the new {} in place", target.display()));
    }
    Ok(())
}

/// Windows: remove the `.old` exes a previous update left (call at start-up).
pub fn cleanup_old() {
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        for n in ["beacon-app.exe.old", "beacon.exe.old"] {
            let _ = std::fs::remove_file(dir.join(n));
        }
    }
}

fn find_named(root: &Path, name: &str) -> Option<PathBuf> {
    walkdir::WalkDir::new(root).max_depth(4).into_iter().flatten().find(|e| e.file_type().is_file() && e.file_name().to_string_lossy() == name).map(|e| e.into_path())
}

fn install(method: &Method, file: &Path, work: &Path, data_dir: &Path) -> Result<Applied> {
    let _ = data_dir;
    match method {
        #[cfg(unix)]
        Method::AppImage { path } => {
            replace_unix(file, path)?;
            Ok(Applied::Restart { program: path.clone() })
        }
        Method::Folder { dir } => {
            let unpacked = work.join("unpacked");
            let _ = std::fs::remove_dir_all(&unpacked);
            if cfg!(unix) {
                // The tar.gz: plain tar is always there.
                std::fs::create_dir_all(&unpacked)?;
                let st = std::process::Command::new("tar").arg("-xzf").arg(file).arg("-C").arg(&unpacked).status().context("starting tar")?;
                if !st.success() {
                    bail!("extracting the update failed ({st})");
                }
                crate::media::remove_links(&unpacked)?;
            } else {
                crate::media::extract_with(file, &unpacked, "no tool to extract the update (Windows 10 or later has tar.exe)")?;
            }
            let names: &[&str] = if cfg!(windows) { &["beacon-app.exe", "beacon.exe"] } else { &["beacon-app", "beacon"] };
            let mut replaced = 0;
            for n in names {
                let target = dir.join(n);
                if !target.exists() && *n != names[0] {
                    continue;
                }
                let new = find_named(&unpacked, n).ok_or_else(|| anyhow!("the update has no {n}"))?;
                #[cfg(unix)]
                replace_unix(&new, &target)?;
                #[cfg(windows)]
                replace_windows(&new, &target)?;
                replaced += 1;
            }
            let _ = std::fs::remove_dir_all(&unpacked);
            if replaced == 0 {
                bail!("nothing to replace in {}", dir.display());
            }
            Ok(Applied::Restart { program: dir.join(names[0]) })
        }
        Method::Package { format } => {
            let args: Vec<String> = if format == "deb" {
                vec!["dpkg".into(), "-i".into(), file.to_string_lossy().into_owned()]
            } else {
                vec!["rpm".into(), "-U".into(), "--replacepkgs".into(), file.to_string_lossy().into_owned()]
            };
            let st = std::process::Command::new("pkexec").args(&args).status().context("starting pkexec")?;
            if !st.success() {
                bail!("installing the {format} package failed or was cancelled ({st}); it's kept at {}", file.display());
            }
            let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/beacon-app"));
            Ok(Applied::Restart { program: exe })
        }
        Method::Manual { format } => {
            let how = if format == "rpm" && Path::new("/run/ostree-booted").exists() {
                format!("rpm-ostree install {} (then reboot)", file.display())
            } else if format == "deb" {
                format!("sudo apt install {}", file.display())
            } else {
                format!("sudo dnf install {}", file.display())
            };
            Ok(Applied::Manual { file: file.to_path_buf(), how })
        }
        #[cfg(windows)]
        Method::Msi => {
            let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
            let cmd = crate::runner::Command {
                program: root.join("System32").join("msiexec.exe"),
                args: vec!["/i".into(), file.to_string_lossy().into_owned(), "/passive".into()],
                env: vec![],
                cwd: work.to_path_buf(),
            };
            crate::native::spawn_elevated(data_dir, &cmd)?;
            Ok(Applied::Quit)
        }
        Method::Unsupported { why } => bail!("this copy can't update itself: {why}"),
        #[allow(unreachable_patterns)]
        _ => bail!("this kind of install doesn't update on this system"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testnet::FakeNet;
    use super::*;
    use serde_json::json;

    #[test]
    fn versions() {
        assert!(is_newer("0.0.10", "0.0.9"));
        assert!(is_newer("v0.1.0", "0.0.9"));
        assert!(!is_newer("0.0.9", "0.0.9"));
        assert!(!is_newer("0.0.8", "0.0.9"));
        assert!(!is_newer("garbage", "0.0.9"));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("1.2.3-beta+x"), Some((1, 2, 3)));
    }

    #[test]
    fn sums() {
        let s = format!("{}  Beacon-x86_64.AppImage\n{} *beacon_amd64.deb\n", "a".repeat(64), "B".repeat(64));
        assert_eq!(sum_for(&s, "Beacon-x86_64.AppImage"), Some("a".repeat(64)));
        assert_eq!(sum_for(&s, "beacon_amd64.deb"), Some("b".repeat(64)));
        assert_eq!(sum_for(&s, "x"), None);
    }

    #[test]
    fn methods_and_assets() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("bin/beacon-app");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        let app = t.path().join("Beacon.AppImage");
        assert_eq!(method_for(&exe, Some(&app)), Method::AppImage { path: app.clone() });
        assert_eq!(asset_name(&Method::AppImage { path: app }, "0.0.10").unwrap(), "Beacon-x86_64.AppImage");
        assert!(matches!(method_for(&exe, None), Method::Folder { .. }));
        assert!(matches!(method_for(Path::new("/x/target/release/beacon-app"), None), Method::Unsupported { .. }));
        assert_eq!(asset_name(&Method::Package { format: "deb".into() }, "0.0.10").unwrap(), "beacon_0.0.10_amd64.deb");
        assert_eq!(asset_name(&Method::Manual { format: "rpm".into() }, "0.0.10").unwrap(), "beacon-0.0.10-1.x86_64.rpm");
        assert_eq!(asset_name(&Method::Msi, "0.0.10").unwrap(), "Beacon-0.0.10-x64.msi");
    }

    #[test]
    fn reads_the_latest_public_release_and_downloads_it() {
        let net = FakeNet::default();
        let rel = json!({
            "tag_name": "v0.0.10", "html_url": "https://github.com/o/r/releases/tag/v0.0.10", "body": "notes",
            "assets": [{"name": "Beacon-x86_64.AppImage", "url": "https://api.github.com/repos/o/r/releases/assets/1",
                        "browser_download_url": "https://github.com/o/r/releases/download/v0.0.10/Beacon-x86_64.AppImage", "size": 3}]
        });
        net.put("https://api.github.com/repos/o/r/releases/latest", rel.to_string().as_bytes());
        let r = latest(&net, "o/r").unwrap();
        assert_eq!(r.version, "0.0.10");
        assert_eq!(r.assets[0].size, Some(3));
        net.put("https://github.com/o/r/releases/download/v0.0.10/Beacon-x86_64.AppImage", b"new");
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("Beacon-x86_64.AppImage");
        download_asset(&net, &r.assets[0], &dest, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        // A repo with no public releases says so.
        assert!(format!("{:#}", latest(&net, "o/none").unwrap_err()).contains("no public releases"));
        assert!(latest(&net, "not a repo").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn replaces_a_file_in_place() {
        let t = tempfile::tempdir().unwrap();
        let (new, target) = (t.path().join("new"), t.path().join("Beacon.AppImage"));
        std::fs::write(&new, b"v2").unwrap();
        std::fs::write(&target, b"v1").unwrap();
        let r = install(&Method::AppImage { path: target.clone() }, &new, t.path(), t.path()).unwrap();
        assert_eq!(r, Applied::Restart { program: target.clone() });
        assert_eq!(std::fs::read(&target).unwrap(), b"v2");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&target).unwrap().permissions().mode() & 0o111, 0o111);
    }
}
