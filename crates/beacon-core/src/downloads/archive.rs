//! **Private build only.** A game delivered as one archive (Rise of Legends'
//! Google Drive zip). Beacon downloads it, extracts it, and looks at what's
//! inside:
//!
//! * **the game itself** (one of its exes, e.g. `RiseOfLegends.exe`): the
//!   folder goes to `<games>/<folder>` and Beacon points the game at it;
//! * **a launcher or installer** (other Windows programs, no game exe): it's
//!   kept in `<data>/tools/<game>-launcher/<name>/` and remembered, and Beacon
//!   runs it in the game's prefix ("Run a patch or installer"), then finds the
//!   game it put there, like any installer.
//!
//! Extraction uses Beacon's usual safe path (bsdtar or 7-Zip, separate
//! arguments, links removed), into a staging folder first.

use super::sources::{self, How, Target};
use super::{fetch_file, FileJob, Transfer};
use crate::community_launcher;
use crate::games::GameId;
use crate::workshop::safe_rel_path;
use crate::Beacon;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A downloaded archive.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fetched {
    pub game: GameId,
    pub file: PathBuf,
    pub name: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum Outcome {
    Ready(Fetched),
    NeedsConfirm { game: GameId, note: String },
}

/// What the archive turned out to hold.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Unpacked {
    /// The game, ready to play, in `dir`.
    Game { game: GameId, dir: PathBuf },
    /// A launcher or installer to run in the game's prefix.
    Launcher { game: GameId, exe: PathBuf },
}

/// Launchers from archives, per game (`<data>/downloads/launchers.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Launchers {
    pub exes: BTreeMap<GameId, PathBuf>,
}

fn safe_name(s: &str, fallback: &str) -> String {
    let clean: String = s.chars().map(|c| if c.is_alphanumeric() || " -_.()+,".contains(c) { c } else { '_' }).take(80).collect();
    let clean = clean.trim().trim_matches('.').to_string();
    match safe_rel_path(&clean) {
        Some(p) if p.components().count() == 1 && !clean.is_empty() => clean,
        _ => fallback.to_string(),
    }
}

/// The game's exe inside `root` (up to four folders down), its exes in order of preference.
fn find_game_exe(root: &Path, g: GameId) -> Option<PathBuf> {
    let files: Vec<PathBuf> = walkdir::WalkDir::new(root)
        .max_depth(5)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    g.exes().iter().find_map(|want| {
        let mut hits: Vec<&PathBuf> = files.iter().filter(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(want)).unwrap_or(false)).collect();
        // The shallowest one (a game folder, not a copy inside a backup).
        hits.sort_by_key(|p| p.components().count());
        hits.first().map(|p| (*p).clone())
    })
}

/// Move everything in `from` into `to`, replacing files with the same path (a repair over an install).
fn merge_into(from: &Path, to: &Path) -> Result<()> {
    for e in walkdir::WalkDir::new(from).min_depth(1).contents_first(false).follow_links(false) {
        let e = e?;
        let rel = e.path().strip_prefix(from).map_err(|_| anyhow!("internal: outside the staging folder"))?;
        let dest = to.join(rel);
        if e.file_type().is_dir() {
            std::fs::create_dir_all(&dest)?;
        } else if e.file_type().is_file() {
            if dest.is_dir() {
                bail!("{} is a folder in the install but a file in the archive", rel.display());
            }
            if dest.exists() {
                std::fs::remove_file(&dest)?;
            }
            if std::fs::rename(e.path(), &dest).is_err() {
                std::fs::copy(e.path(), &dest).with_context(|| format!("copying {}", rel.display()))?;
            }
        }
    }
    Ok(())
}

impl Beacon {
    /// Whether `sources.json` has a ready-to-play archive for this game.
    pub fn has_archive_source(&self, g: GameId) -> bool {
        self.download_store().list().archive_for(g).is_some()
    }

    fn launchers_file(&self) -> PathBuf {
        self.download_store().dir.join("launchers.json")
    }

    /// Launchers that came out of archives, per game (only ones still there).
    pub fn archive_launchers(&self) -> Launchers {
        let mut l: Launchers = std::fs::read_to_string(self.launchers_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        l.exes.retain(|_, p| p.is_file());
        l
    }

    /// Download the archive for `g` into `<data>/downloads/files/<game>/` (resumable). Safe on a clone.
    pub fn archive_download(&self, t: &dyn Transfer, g: GameId, accept_guess: bool, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<Outcome> {
        let store = self.download_store();
        let list = store.list();
        let src = list.archive_for(g).ok_or_else(|| anyhow!("sources.json has no download for {}", g.short()))?;
        progress(None, &format!("Looking up {}", g.short()));
        let mut learned = store.learned();
        let before = learned.clone();
        let resolved = sources::resolve(t, &list, &mut learned, src);
        if learned != before {
            let _ = store.save_learned(&learned);
        }
        let resolved = resolved?;
        let note = match &resolved.how {
            How::Rediscovered { note, .. } => Some(note.clone()),
            How::Listed => None,
        };
        if resolved.how.needs_confirm() && !accept_guess {
            return Ok(Outcome::NeedsConfirm { game: g, note: note.unwrap_or_default() });
        }
        let Target::File { url, name, size, md5 } = resolved.target else {
            bail!("the source for {} isn't a single file", g.short());
        };
        let file = store.dir.join("files").join(g.slug()).join(safe_name(&name, &format!("{}.zip", g.slug())));
        let label = format!("Downloading {name}");
        fetch_file(t, &FileJob { url, dest: file.clone(), size, md5 }, false, &mut |b| {
            let pct = size.filter(|s| *s > 0).map(|s| (b.saturating_mul(90) / s).min(89) as u8);
            progress(pct, &label);
        })?;
        Ok(Outcome::Ready(Fetched { game: g, file, name, note }))
    }

    /// Extract a downloaded archive and work out what it is. The game goes to
    /// `<games>/<folder>` (merged over an earlier copy); a launcher goes to the
    /// tools folder. Doesn't change the config (see [`Beacon::archive_adopt`]); safe on a clone.
    pub fn archive_unpack(&self, g: GameId, file: &Path, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<Unpacked> {
        let list = self.download_store().list();
        let folder = list.archive_for(g).and_then(|s| s.install_folder.clone()).map(|f| safe_name(&f, g.slug())).unwrap_or_else(|| g.slug().to_string());
        let root = self.games_root();
        std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
        let staging = root.join(format!(".{folder}.beacon-tmp"));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).with_context(|| format!("removing {}", staging.display()))?;
        }
        progress(Some(90), &format!("Extracting {}", file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        crate::media::extract_with(file, &staging, "no tool to extract it: install libarchive (bsdtar) or 7-Zip (7z), or unrar for .rar files; on Windows 10 or later tar.exe is built in")
            .with_context(|| format!("extracting {}", file.display()))?;
        let result = (|| -> Result<Unpacked> {
            if let Some(exe) = find_game_exe(&staging, g) {
                let rel = exe.parent().and_then(|p| p.strip_prefix(&staging).ok()).map(Path::to_path_buf).unwrap_or_default();
                let target = root.join(&folder);
                progress(Some(97), "Putting the game in place");
                if target.exists() {
                    merge_into(&staging, &target)?;
                    std::fs::remove_dir_all(&staging).ok();
                } else {
                    std::fs::rename(&staging, &target).with_context(|| format!("moving the game to {}", target.display()))?;
                }
                return Ok(Unpacked::Game { game: g, dir: target.join(rel) });
            }
            // No game exe: a launcher or an installer.
            let exe = community_launcher::pick_exe(&staging).ok_or_else(|| anyhow!("the download has no {} and no Windows program to run", g.exe()))?;
            let rel = exe.strip_prefix(&staging).map(Path::to_path_buf).map_err(|_| anyhow!("internal: launcher outside its folder"))?;
            let name = safe_name(&file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), "launcher");
            let tools = self.paths.data_dir.join("tools").join(format!("{}-launcher", g.slug()));
            std::fs::create_dir_all(&tools)?;
            let dest = tools.join(&name);
            if dest.exists() {
                std::fs::remove_dir_all(&dest).with_context(|| format!("replacing {} (close that launcher if it's open)", dest.display()))?;
            }
            if std::fs::rename(&staging, &dest).is_err() {
                merge_into(&staging, &dest)?;
                std::fs::remove_dir_all(&staging).ok();
            }
            Ok(Unpacked::Launcher { game: g, exe: dest.join(rel) })
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        let r = result?;
        if matches!(r, Unpacked::Game { .. }) {
            // The game is extracted; the archive is only taking space now.
            let _ = std::fs::remove_file(file);
        }
        Ok(r)
    }

    /// Point Beacon at an unpacked game, or remember an unpacked launcher. Save the config afterwards.
    pub fn archive_adopt(&mut self, u: &Unpacked) -> Result<()> {
        match u {
            Unpacked::Game { game, dir } => self.set_path(*game, dir),
            Unpacked::Launcher { game, exe } => {
                let mut l = self.archive_launchers();
                l.exes.insert(*game, exe.clone());
                let file = self.launchers_file();
                std::fs::create_dir_all(file.parent().unwrap())?;
                std::fs::write(&file, serde_json::to_string_pretty(&l)?)?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    fn beacon(root: &Path) -> Beacon {
        Beacon::load_with(Paths::under(root, root), "").unwrap()
    }

    #[test]
    fn finds_the_game_exe_shallowest_first() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::create_dir_all(r.join("Rise Of Legends/backup")).unwrap();
        std::fs::write(r.join("Rise Of Legends/RiseOfLegends.exe"), b"MZ").unwrap();
        std::fs::write(r.join("Rise Of Legends/backup/riseoflegends.EXE"), b"MZ").unwrap();
        assert_eq!(find_game_exe(r, GameId::Rol), Some(r.join("Rise Of Legends/RiseOfLegends.exe")));
        assert_eq!(find_game_exe(r, GameId::Uaw), None);
    }

    #[test]
    fn merges_over_an_install() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        std::fs::create_dir_all(a.join("Data")).unwrap();
        std::fs::create_dir_all(b.join("Data")).unwrap();
        std::fs::write(a.join("Data/x.bhg"), b"new").unwrap();
        std::fs::write(b.join("Data/x.bhg"), b"old").unwrap();
        std::fs::write(b.join("saves.dat"), b"keep").unwrap();
        merge_into(&a, &b).unwrap();
        assert_eq!(std::fs::read(b.join("Data/x.bhg")).unwrap(), b"new");
        assert_eq!(std::fs::read(b.join("saves.dat")).unwrap(), b"keep");
    }

    #[test]
    fn rol_has_a_source_and_launchers_are_remembered() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        assert!(b.has_archive_source(GameId::Rol));
        assert!(!b.has_archive_source(GameId::Exigo));
        let exe = t.path().join("data/tools/rol-launcher/x/Launcher.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"MZ").unwrap();
        b.archive_adopt(&Unpacked::Launcher { game: GameId::Rol, exe: exe.clone() }).unwrap();
        assert_eq!(b.archive_launchers().exes.get(&GameId::Rol), Some(&exe));
        std::fs::remove_file(&exe).unwrap();
        assert!(b.archive_launchers().exes.is_empty());
    }

    #[test]
    fn names_are_safe() {
        assert_eq!(safe_name("Rise Of Legends Game Files.zip", "x"), "Rise Of Legends Game Files.zip");
        assert_eq!(safe_name("..", "x"), "x");
        assert_eq!(safe_name("a/b\\c.zip", "x"), "a_b_c.zip");
    }
}
