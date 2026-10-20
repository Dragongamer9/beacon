//! "Install from your own copy": a game's installer .exe, a folder (an
//! extracted or mounted disc), or a disc image (.iso). Images are extracted
//! with bsdtar or 7-Zip into a fresh folder under `<data>/tmp/` (Windows 10 and
//! later ship bsdtar as `tar.exe`), always as separate arguments, never through
//! a shell. Then the setup program is found and run like any installer: in
//! the game's prefix on Linux, directly on Windows.

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

/// What the player picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A setup program to run as it is.
    Installer(PathBuf),
    /// A folder holding the disc's files.
    Folder(PathBuf),
    /// A disc image to extract first.
    Image(PathBuf),
}

fn ext_is(p: &Path, exts: &[&str]) -> bool {
    p.extension().map(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x))).unwrap_or(false)
}

/// Check the path and say what it is. Paths are made absolute, so nothing can
/// be mistaken for a tool's option.
pub fn classify(path: &Path) -> Result<Source> {
    let abs = std::path::absolute(path).with_context(|| format!("{} isn't a usable path", path.display()))?;
    if abs.is_dir() {
        return Ok(Source::Folder(abs));
    }
    if !abs.is_file() {
        bail!("{} not found", path.display());
    }
    if ext_is(&abs, &["exe", "msi"]) {
        Ok(Source::Installer(abs))
    } else if ext_is(&abs, &["iso"]) {
        Ok(Source::Image(abs))
    } else {
        bail!("pick the game's installer (.exe), its disc image (.iso) or the disc's folder")
    }
}

/// The program `autorun.inf` starts (`open=…`), as a relative path.
pub fn autorun_open(text: &str) -> Option<String> {
    let mut in_autorun = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_autorun = l.eq_ignore_ascii_case("[autorun]");
            continue;
        }
        if !in_autorun {
            continue;
        }
        if let Some((k, v)) = l.split_once('=') {
            if k.trim().eq_ignore_ascii_case("open") {
                // "open=AutoRun.exe /x" → AutoRun.exe; quoted paths keep their spaces.
                let v = v.trim();
                let prog = if let Some(rest) = v.strip_prefix('"') { rest.split('"').next().unwrap_or("") } else { v.split_whitespace().next().unwrap_or("") };
                let prog = prog.trim().replace('\\', "/");
                if !prog.is_empty() && !prog.contains("..") && !prog.starts_with('/') && !prog.contains(':') {
                    return Some(prog);
                }
            }
        }
    }
    None
}

/// An existing file under `dir` matching `rel` case-insensitively, segment by segment.
fn find_ci(dir: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = dir.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty() && *s != ".") {
        let next = std::fs::read_dir(&cur).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(seg)).unwrap_or(false))?;
        cur = next;
    }
    cur.is_file().then_some(cur)
}

fn is_setup_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".exe") && (n.contains("setup") || n.contains("install")) && !n.contains("uninst") && !n.starts_with("unins")
}

/// The setup program on a disc or in an installer folder: `setup.exe` at the
/// top, then the program `autorun.inf` opens if it's a setup, then any
/// setup/installer exe two levels down (shallowest, shortest name first), then
/// whatever `autorun.inf` opens.
pub fn find_setup(dir: &Path) -> Option<PathBuf> {
    if let Some(p) = find_ci(dir, "setup.exe") {
        return Some(p);
    }
    let autorun = find_ci(dir, "autorun.inf").and_then(|f| std::fs::read(f).ok()).and_then(|b| autorun_open(&String::from_utf8_lossy(&b)));
    let autorun_exe = autorun.as_deref().and_then(|rel| find_ci(dir, rel)).filter(|p| ext_is(p, &["exe"]));
    if let Some(p) = autorun_exe.as_ref().filter(|p| is_setup_name(&p.file_name().unwrap_or_default().to_string_lossy())) {
        return Some(p.clone());
    }
    let mut cands: Vec<(usize, usize, PathBuf)> = walkdir::WalkDir::new(dir)
        .max_depth(3)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && is_setup_name(&e.file_name().to_string_lossy()))
        .map(|e| (e.depth(), e.file_name().len(), e.into_path()))
        .collect();
    cands.sort();
    cands.into_iter().map(|(_, _, p)| p).next().or(autorun_exe)
}

/// The extraction tools to try, in order, as (program, arguments).
pub fn extract_commands(image: &Path, dest: &Path) -> Vec<(PathBuf, Vec<String>)> {
    let i = image.to_string_lossy().into_owned();
    let d = dest.to_string_lossy().into_owned();
    let bsdtar = |prog: PathBuf| (prog, vec!["-xf".to_string(), i.clone(), "-C".to_string(), d.clone()]);
    let sevenzip = |prog: PathBuf| (prog, vec!["x".to_string(), "-y".to_string(), format!("-o{d}"), i.clone()]);
    let mut out = Vec::new();
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        out.push(bsdtar(root.join("System32").join("tar.exe")));
        out.push(sevenzip("7z".into()));
        for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Some(pf) = std::env::var_os(var) {
                out.push(sevenzip(PathBuf::from(pf).join("7-Zip").join("7z.exe")));
            }
        }
    } else {
        out.push(bsdtar("bsdtar".into()));
        out.push(sevenzip("7z".into()));
        out.push(sevenzip("7zz".into()));
    }
    out
}

/// Remove links an image may carry (a disc has no business pointing outside
/// itself). Returns how many were removed.
pub fn remove_links(dir: &Path) -> Result<usize> {
    let mut n = 0;
    for e in walkdir::WalkDir::new(dir).follow_links(false).into_iter().flatten() {
        if e.path_is_symlink() {
            std::fs::remove_file(e.path()).or_else(|_| std::fs::remove_dir(e.path()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Extract a disc image into `dest` (which must not exist yet).
pub fn extract_image(image: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        bail!("{} already exists", dest.display());
    }
    let mut tried = Vec::new();
    for (prog, args) in extract_commands(image, dest) {
        std::fs::create_dir_all(dest)?;
        match std::process::Command::new(&prog).args(&args).stdin(std::process::Stdio::null()).output() {
            Ok(out) if out.status.success() => {
                remove_links(dest)?;
                if std::fs::read_dir(dest).map(|mut r| r.next().is_none()).unwrap_or(true) {
                    tried.push(format!("{}: extracted nothing", prog.display()));
                    let _ = std::fs::remove_dir_all(dest);
                    continue;
                }
                return Ok(());
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr);
                tried.push(format!("{}: {}", prog.display(), err.lines().last().unwrap_or("").trim()));
                let _ = std::fs::remove_dir_all(dest);
            }
            Err(_) => {
                let _ = std::fs::remove_dir_all(dest);
            }
        }
    }
    if tried.is_empty() {
        bail!(if cfg!(windows) {
            "no tool to open the disc image: Windows 10 or later has tar.exe; or install 7-Zip, or mount the image (double-click it) and pick its drive"
        } else {
            "no tool to open the disc image: install libarchive (bsdtar) or 7-Zip (7z), or mount the image and pick its folder"
        });
    }
    Err(anyhow!("couldn't extract {}: {}", image.display(), tried.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn autorun() {
        let inf = "[AutoRun]\r\nopen=AutoRun.exe\r\nicon=game.ico\r\n";
        assert_eq!(autorun_open(inf).as_deref(), Some("AutoRun.exe"));
        assert_eq!(autorun_open("[autorun]\nOPEN = \"Setup Files\\setup.exe\" /auto\n").as_deref(), Some("Setup Files/setup.exe"));
        assert_eq!(autorun_open("[other]\nopen=x.exe\n"), None);
        assert_eq!(autorun_open("[autorun]\nopen=..\\..\\evil.exe\n"), None);
        assert_eq!(autorun_open("[autorun]\nopen=C:\\evil.exe\n"), None);
    }

    #[test]
    fn finds_the_setup() {
        let t = tempfile::tempdir().unwrap();
        // A disc with an autorun menu and the real setup one level down.
        let disc = t.path().join("disc");
        fs::create_dir_all(disc.join("Setup")).unwrap();
        fs::write(disc.join("AUTORUN.INF"), "[autorun]\nopen=AutoRun.exe\n").unwrap();
        fs::write(disc.join("AutoRun.exe"), b"").unwrap();
        fs::write(disc.join("Setup/Setup.exe"), b"").unwrap();
        fs::write(disc.join("Setup/unins000.exe"), b"").unwrap();
        assert_eq!(find_setup(&disc).unwrap(), disc.join("Setup/Setup.exe"));
        // setup.exe at the top wins.
        fs::write(disc.join("SETUP.EXE"), b"").unwrap();
        assert_eq!(find_setup(&disc).unwrap(), disc.join("SETUP.EXE"));
        // Only an autorun menu: use it.
        let menu = t.path().join("menu");
        fs::create_dir_all(&menu).unwrap();
        fs::write(menu.join("autorun.inf"), "[autorun]\nopen=Launch.exe\n").unwrap();
        fs::write(menu.join("Launch.exe"), b"").unwrap();
        assert_eq!(find_setup(&menu).unwrap(), menu.join("Launch.exe"));
        // AotR-style installer folder.
        let aotr = t.path().join("aotr");
        fs::create_dir_all(&aotr).unwrap();
        fs::write(aotr.join("AotR_Standalone_9.1_Installer.exe"), b"").unwrap();
        fs::write(aotr.join("readme.txt"), b"").unwrap();
        assert!(find_setup(&aotr).unwrap().ends_with("AotR_Standalone_9.1_Installer.exe"));
        assert!(find_setup(&t.path().join("missing")).is_none());
    }

    #[test]
    fn classifies() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("Setup.EXE");
        let iso = t.path().join("BFME2.ISO");
        let txt = t.path().join("notes.txt");
        for f in [&exe, &iso, &txt] {
            fs::write(f, b"").unwrap();
        }
        assert_eq!(classify(&exe).unwrap(), Source::Installer(exe.clone()));
        assert_eq!(classify(&iso).unwrap(), Source::Image(iso.clone()));
        assert_eq!(classify(t.path()).unwrap(), Source::Folder(t.path().to_path_buf()));
        assert!(classify(&txt).is_err());
        assert!(classify(&t.path().join("gone.iso")).is_err());
    }

    #[test]
    fn extraction_is_argument_lists() {
        let cmds = extract_commands(Path::new("/media/BFME2.iso"), Path::new("/data/tmp/x"));
        assert!(!cmds.is_empty());
        for (_, args) in &cmds {
            assert!(args.iter().any(|a| a == "/media/BFME2.iso"), "the image is one argument: {args:?}");
        }
        #[cfg(unix)]
        assert_eq!(cmds[0], (PathBuf::from("bsdtar"), vec!["-xf".into(), "/media/BFME2.iso".into(), "-C".into(), "/data/tmp/x".into()]));
    }

    #[cfg(unix)]
    #[test]
    fn links_are_removed() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("a.txt"), b"").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", t.path().join("evil")).unwrap();
        assert_eq!(remove_links(t.path()).unwrap(), 1);
        assert!(!t.path().join("evil").exists() && t.path().join("a.txt").exists());
    }
}
