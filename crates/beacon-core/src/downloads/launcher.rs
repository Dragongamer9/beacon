//! Community launchers (Universe at War's) downloaded
//! through `sources.json`: the newest launcher in the community's Google Drive
//! folder, looked up at download time. Beacon then uses it exactly like one
//! you picked yourself (`set_community_launcher`: the .exe copied or the .zip
//! extracted into `<data>/tools/<game>-launcher/<name>/`), and the game is
//! downloaded inside it.

use super::sources::{self, How, Target};
use super::{fetch_file, FileJob, Transfer};
use crate::community_launcher;
use crate::games::GameId;
use crate::workshop::safe_rel_path;
use crate::Beacon;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A launcher download, ready to hand to `set_community_launcher`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fetched {
    pub game: GameId,
    /// The downloaded .zip or .exe.
    pub file: PathBuf,
    /// Its name in the folder, e.g. "UaW Launcher 1.3.zip".
    pub name: String,
    /// The launcher already in use is this one; nothing was downloaded.
    pub up_to_date: bool,
    /// Set when the file was found somewhere new.
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum Outcome {
    Ready(Fetched),
    /// Only a guess turned up; ask, then run again with `accept_guess`.
    NeedsConfirm { game: GameId, note: String },
}

/// A safe single-file name for the download, keeping its extension.
fn file_name_for(name: &str) -> Result<String> {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "zip" | "7z" | "rar" | "exe") {
        bail!("the newest launcher is {name}, a .{ext} file; Beacon handles .exe, .zip, .7z and .rar (extract it yourself and pick its .exe)");
    }
    let clean: String = name.chars().map(|c| if c.is_alphanumeric() || " -_.()+,".contains(c) { c } else { '_' }).take(100).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    match safe_rel_path(&clean) {
        Some(p) if p.components().count() == 1 && !clean.is_empty() => Ok(clean),
        _ => Ok(format!("launcher.{ext}")),
    }
}

impl Beacon {
    /// Whether `sources.json` has a launcher download for this game.
    pub fn has_launcher_source(&self, g: GameId) -> bool {
        self.download_store().list().launcher_for(g).is_some()
    }

    /// The launcher in use came from a download called `name`.
    fn launcher_in_use_is(&self, g: GameId, name: &str) -> bool {
        let folder = community_launcher::folder_name(Path::new(name));
        self.community_launcher_exe(g).map(|exe| exe.starts_with(self.community_launcher_dir(g).join(folder))).unwrap_or(false)
    }

    /// Find the newest community launcher for `g` and download it into
    /// `<data>/downloads/files/<game>-launcher/` (resuming a stopped download).
    /// When the launcher in use is already that one, nothing is downloaded
    /// unless `force`. Doesn't change the config; safe on a clone.
    pub fn launcher_download(
        &self,
        t: &dyn Transfer,
        g: GameId,
        accept_guess: bool,
        force: bool,
        progress: &mut dyn FnMut(Option<u8>, &str),
    ) -> Result<Outcome> {
        let store = self.download_store();
        let list = store.list();
        let src = list.launcher_for(g).ok_or_else(|| anyhow!("sources.json has no launcher download for {}", g.short()))?;
        progress(None, &format!("Looking for the newest {} launcher", g.short()));
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
            bail!("the launcher source for {} isn't a single file", g.short());
        };
        if !force && self.launcher_in_use_is(g, &name) {
            let file = self.community_launcher_exe(g).unwrap_or_default();
            return Ok(Outcome::Ready(Fetched { game: g, file, name, up_to_date: true, note }));
        }
        let dir = store.dir.join("files").join(format!("{}-launcher", g.slug()));
        let file = dir.join(file_name_for(&name)?);
        let job = FileJob { url, dest: file.clone(), size, md5 };
        let label = format!("Downloading {name}");
        fetch_file(t, &job, false, &mut |b| {
            let pct = size.filter(|s| *s > 0).map(|s| (b.saturating_mul(100) / s).min(99) as u8);
            progress(pct, &label);
        })?;
        progress(Some(100), "Downloaded");
        Ok(Outcome::Ready(Fetched { game: g, file, name, up_to_date: false, note }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::testnet::FakeNet;
    use super::*;
    use crate::config::Paths;

    const FOLDER: &str = "167UET84ZWXrMd9zpxEYiQWTQZexCGZfT";

    fn embedded(entries: &[(&str, &str)]) -> String {
        entries
            .iter()
            .map(|(id, name)| format!("<div class=\"flip-entry\" id=\"entry-{id}\"><a href=\"https://drive.google.com/file/d/{id}/view\"><div class=\"flip-entry-title\">{name}</div></a></div>"))
            .collect()
    }

    fn exe_bytes() -> Vec<u8> {
        let mut v = b"MZ".to_vec();
        v.extend_from_slice(&[0u8; 200]);
        v
    }

    #[test]
    fn downloads_the_newest_and_then_knows_it_has_it() {
        let t = tempfile::tempdir().unwrap();
        let mut b = Beacon::load_with(Paths::under(t.path(), t.path()), "").unwrap();
        assert!(b.has_launcher_source(GameId::Uaw));
        assert!(!b.has_launcher_source(GameId::Bfme1));
        let net = FakeNet::default();
        net.put(
            &format!("https://drive.google.com/embeddedfolderview?id={FOLDER}"),
            embedded(&[("1aaaaaaaaaaaaaaaaaaaaaaa", "UaW Launcher 1.2.exe"), ("1bbbbbbbbbbbbbbbbbbbbbbb", "UaW Launcher 1.3.exe")]).as_bytes(),
        );
        net.put("https://drive.usercontent.google.com/download?id=1bbbbbbbbbbbbbbbbbbbbbbb&export=download&confirm=t", &exe_bytes());
        let out = b.launcher_download(&net, GameId::Uaw, false, false, &mut |_, _| {}).unwrap();
        let Outcome::Ready(f) = out else { panic!("{out:?}") };
        assert_eq!(f.name, "UaW Launcher 1.3.exe");
        assert!(!f.up_to_date);
        assert_eq!(std::fs::read(&f.file).unwrap(), exe_bytes());
        // Hand it to Beacon like a launcher picked by hand.
        let exe = b.set_community_launcher(GameId::Uaw, &f.file, &mut |_| {}).unwrap();
        assert_eq!(b.game(GameId::Uaw).community_launcher.as_deref(), Some(exe.as_path()));
        // Same version again: nothing to download.
        let Outcome::Ready(again) = b.launcher_download(&net, GameId::Uaw, false, false, &mut |_, _| {}).unwrap() else { panic!() };
        assert!(again.up_to_date);
    }

    #[test]
    fn other_archives_are_refused() {
        assert!(file_name_for("UaW Launcher.tar.xz").is_err());
        assert_eq!(file_name_for("UaW Launcher.rar").unwrap(), "UaW Launcher.rar");
        assert_eq!(file_name_for("UaW Launcher 2.7Z").unwrap(), "UaW Launcher 2.7Z");
        assert_eq!(file_name_for("UaW Launcher 1.3.zip").unwrap(), "UaW Launcher 1.3.zip");
        assert_eq!(file_name_for("../../x.exe").unwrap(), "_.._x.exe");
    }
}
