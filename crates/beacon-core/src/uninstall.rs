//! Uninstalling a game, and what Beacon will (and won't) delete.
//!
//! [`Beacon::uninstall_plan`] says what an uninstall would do before anything
//! happens: the game's install folder and its size, whether Beacon is willing
//! to delete it (and why not), the game's own uninstaller if it ships one, the
//! game's Wine prefix (where saves and settings live) and the games that
//! need this one (RotWK needs BFME II).
//!
//! [`Beacon::uninstall`] then deletes the folder when asked and allowed, and the
//! prefix only when asked, and forgets the folder. Everything else about the
//! game (CD key, resolution, runner, mod choice) is kept for a reinstall.
//!
//! Beacon never deletes: a folder it can't confirm is the game's (no game exe in
//! it), a Steam copy (Steam uninstalls those), a folder holding another game
//! Beacon knows, your home folder or anything above it, Beacon's own data folder,
//! or a top-level or shared folder (`/`, `C:\`, `drive_c`, `Program Files`,
//! `Games`, `steamapps`…).

use crate::games::{self, GameId};
use crate::Beacon;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub game: GameId,
    /// The game's install folder (what deleting the files removes).
    pub dir: Option<PathBuf>,
    pub bytes: u64,
    /// Beacon will delete the files.
    pub can_delete: bool,
    /// Why it won't, when it won't.
    pub why_not: Option<String>,
    /// The game's own uninstaller, if its folder has one.
    pub uninstaller: Option<PathBuf>,
    /// A Steam copy: uninstall it in Steam.
    pub steam: bool,
    /// Beacon's Wine prefix for the game (saves and settings), when there is one.
    pub prefix: Option<PathBuf>,
    pub prefix_bytes: u64,
    /// The install folder is inside that prefix (deleting the prefix removes the game too).
    pub inside_prefix: bool,
    /// Installed games that need this one.
    pub needed_by: Vec<GameId>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Done {
    pub game: GameId,
    pub deleted_dir: Option<PathBuf>,
    pub deleted_prefix: Option<PathBuf>,
    pub freed: u64,
}

/// Folder names that are never an install folder of their own.
const SHARED: &[&str] = &[
    "drive_c", "program files", "program files (x86)", "programdata", "games", "steamapps", "common", "users", "home", "windows",
    "appdata", "roaming", "local", "documents", "desktop", "downloads", "ea games", "electronic arts", "origin games", "sega",
    "lucasarts", "microsoft games", "pfx", "prefixes", "bottles", "media", "mnt", "run", "opt", "usr", "var", "tmp",
];

/// The folder a game was installed into, from the folder its exe is in: Age of the Ring's
/// root (not just its `aotr` folder), Exigo's folder above `Exigo Files`, and the like.
pub fn install_root(g: GameId, exe_dir: &Path) -> PathBuf {
    if g == GameId::Aotr {
        if let Some(root) = games::aotr_root(exe_dir) {
            return root;
        }
    }
    let name = exe_dir.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let inner = matches!((g, name.as_str()), (GameId::Exigo, "exigo files") | (GameId::Swgb, "game") | (_, "bin") | (_, "binaries"));
    match exe_dir.parent() {
        Some(p) if inner && !SHARED.contains(&p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default().as_str()) => p.to_path_buf(),
        _ => exe_dir.to_path_buf(),
    }
}

/// Size of everything under `p` (links not followed).
pub fn size_of(p: &Path) -> u64 {
    walkdir::WalkDir::new(p).follow_links(false).into_iter().flatten().filter(|e| e.file_type().is_file()).filter_map(|e| e.metadata().ok()).map(|m| m.len()).sum()
}

/// Why `dir` must not be deleted, if it mustn't (`protected` = folders that must survive, and everything above them).
pub fn refusal(dir: &Path, protected: &[PathBuf], others: &[PathBuf]) -> Option<String> {
    if !dir.is_absolute() {
        return Some("its folder isn't a full path".into());
    }
    if std::fs::symlink_metadata(dir).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return Some("its folder is a link".into());
    }
    let normal = dir.components().filter(|c| matches!(c, Component::Normal(_))).count();
    let has_drive = dir.components().any(|c| matches!(c, Component::Prefix(_)));
    if normal < if has_drive { 1 } else { 2 } {
        return Some(format!("{} is a top-level folder", dir.display()));
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if SHARED.contains(&name.as_str()) {
        return Some(format!("{} is a shared folder, not the game's own", dir.display()));
    }
    if dir.components().any(|c| c.as_os_str().eq_ignore_ascii_case("steamapps")) {
        return Some("it's a Steam copy: uninstall it in Steam".into());
    }
    for p in protected {
        if p.starts_with(dir) {
            return Some(format!("{} holds {}", dir.display(), p.display()));
        }
    }
    for o in others {
        if o.starts_with(dir) {
            return Some(format!("{} also holds another game ({})", dir.display(), o.display()));
        }
    }
    None
}

/// An uninstaller in the game's folder (`unins000.exe`, `Uninstall.exe`, `uninst.exe`…).
pub fn find_uninstaller(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .max_depth(2)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.ends_with(".exe") && (n.starts_with("unins") || n.contains("uninstall") || n.starts_with("uninst"))
        })
        .map(|e| e.into_path())
        .collect();
    found.sort_by_key(|p| p.components().count());
    found.into_iter().next()
}

impl Beacon {
    /// What uninstalling `g` would do. Nothing changes.
    pub fn uninstall_plan(&self, g: GameId) -> Plan {
        let exe_dir = self.game_dir(g);
        let dir = exe_dir.as_deref().map(|d| install_root(g, d));
        let prefix_root = self.paths.prefix_root(g);
        let prefix = (!self.native && prefix_root.is_dir()).then(|| prefix_root.clone());
        let inside_prefix = dir.as_deref().map(|d| d.starts_with(&prefix_root)).unwrap_or(false);
        let steam = self.steam_appid(g).is_some() || dir.as_deref().map(|d| d.components().any(|c| c.as_os_str().eq_ignore_ascii_case("steamapps"))).unwrap_or(false);
        let mut protected = vec![self.paths.home.clone(), self.paths.data_dir.clone(), self.paths.mods_dir(), self.paths.data_dir.join("prefixes")];
        if let Some(c) = self.paths.config_file.parent() {
            protected.push(c.to_path_buf());
        }
        protected.extend(self.scan_roots());
        let others: Vec<PathBuf> = GameId::ALL.into_iter().filter(|o| *o != g).filter_map(|o| self.game_dir(o)).collect();
        let why_not = match &dir {
            None => Some(format!("{} isn't installed (no folder with {} set)", g.short(), g.exe())),
            Some(d) => refusal(d, &protected, &others),
        };
        let needed_by: Vec<GameId> = GameId::ALL.into_iter().filter(|o| o.requires() == Some(g) && self.game_dir(*o).is_some()).collect();
        Plan {
            game: g,
            bytes: dir.as_deref().map(size_of).unwrap_or(0),
            can_delete: why_not.is_none(),
            why_not,
            uninstaller: dir.as_deref().and_then(find_uninstaller),
            steam,
            prefix_bytes: prefix.as_deref().map(size_of).unwrap_or(0),
            prefix,
            inside_prefix,
            needed_by,
            dir,
        }
    }

    /// Uninstall `g`: delete its folder (`delete_files`, only when the plan allows it) and/or
    /// its Wine prefix (`remove_prefix`), and forget the folder. Save the config afterwards.
    pub fn uninstall(&mut self, g: GameId, delete_files: bool, remove_prefix: bool) -> Result<Done> {
        let plan = self.uninstall_plan(g);
        let mut done = Done { game: g, deleted_dir: None, deleted_prefix: None, freed: 0 };
        if delete_files {
            let dir = plan.dir.clone().ok_or_else(|| anyhow!("{} isn't installed", g.short()))?;
            if let Some(why) = &plan.why_not {
                bail!("Beacon won't delete {}: {why}", dir.display());
            }
            // The game folder only goes when the prefix that holds it isn't going anyway.
            if !(remove_prefix && plan.inside_prefix && plan.prefix.is_some()) {
                std::fs::remove_dir_all(&dir).with_context(|| {
                    format!("deleting {} (on Windows, a game in Program Files may need its own uninstaller)", dir.display())
                })?;
                done.freed += plan.bytes;
                done.deleted_dir = Some(dir);
            }
        }
        if remove_prefix {
            if let Some(p) = &plan.prefix {
                std::fs::remove_dir_all(p).with_context(|| format!("deleting {}", p.display()))?;
                done.freed += plan.prefix_bytes;
                if plan.inside_prefix && delete_files && done.deleted_dir.is_none() {
                    done.deleted_dir = plan.dir.clone();
                }
                done.deleted_prefix = Some(p.clone());
            }
        }
        // Forget the folder (kept when nothing was deleted and the game is still there: that's "remove from Beacon").
        self.game_mut(g).path = None;
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use std::fs;

    fn beacon(root: &Path) -> Beacon {
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        Beacon::load_with(Paths::under(&root.join("beacon"), &home), "").unwrap()
    }

    fn install(dir: &Path, exe: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(exe), vec![0u8; 1000]).unwrap();
    }

    #[test]
    fn roots() {
        assert_eq!(install_root(GameId::Exigo, Path::new("/g/Armies of Exigo/Exigo Files")), PathBuf::from("/g/Armies of Exigo"));
        assert_eq!(install_root(GameId::Bfme2, Path::new("/g/BFME2")), PathBuf::from("/g/BFME2"));
        assert_eq!(install_root(GameId::Exigo, Path::new("/g/Games/Exigo Files")), PathBuf::from("/g/Games/Exigo Files"));
    }

    #[cfg(unix)]
    #[test]
    fn refuses_what_isnt_the_games_alone() {
        let home = PathBuf::from("/home/me");
        let protected = vec![home.clone(), home.join(".local/share/beacon")];
        assert!(refusal(Path::new("/home/me/Games/BFME2"), &protected, &[]).is_none());
        assert!(refusal(Path::new("/home/me"), &protected, &[]).is_some());
        assert!(refusal(Path::new("/home"), &protected, &[]).is_some());
        assert!(refusal(Path::new("/BFME2"), &protected, &[]).is_some());
        assert!(refusal(Path::new("/home/me/Games"), &protected, &[]).is_some());
        assert!(refusal(Path::new("/x/drive_c"), &protected, &[]).is_some());
        assert!(refusal(Path::new("/home/me/.steam/steam/steamapps/common/SWGB"), &protected, &[]).unwrap().contains("Steam"));
        assert!(refusal(Path::new("/home/me/Games/EA"), &protected, &[PathBuf::from("/home/me/Games/EA/RotWK")]).unwrap().contains("another game"));
        assert!(refusal(Path::new("relative/BFME2"), &protected, &[]).is_some());
    }

    #[test]
    fn uninstall_keeps_saves_unless_asked() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        let dir = t.path().join("home/Games/BFME2");
        install(&dir, "lotrbfme2.exe");
        fs::write(dir.join("unins000.exe"), b"MZ").unwrap();
        let rotwk = t.path().join("home/Games/RotWK");
        install(&rotwk, "lotrbfme2ep1.exe");
        b.set_path(GameId::Bfme2, &dir).unwrap();
        b.set_path(GameId::Rotwk, &rotwk).unwrap();
        b.game_mut(GameId::Bfme2).cd_key = Some("KEY".into());
        let pfx = b.paths.prefix_root(GameId::Bfme2).join("pfx/drive_c/users/me/AppData/Roaming/My Battle for Middle-earth(tm) II Files");
        fs::create_dir_all(&pfx).unwrap();
        fs::write(pfx.join("save.BfME2Save"), b"save").unwrap();

        let plan = b.uninstall_plan(GameId::Bfme2);
        assert!(plan.can_delete, "{:?}", plan.why_not);
        assert_eq!(plan.bytes, 1002);
        assert_eq!(plan.uninstaller.as_deref(), Some(dir.join("unins000.exe").as_path()));
        assert_eq!(plan.needed_by, vec![GameId::Rotwk]);
        assert_eq!(plan.prefix.is_some(), !b.native);
        assert!(!plan.inside_prefix);

        let done = b.uninstall(GameId::Bfme2, true, false).unwrap();
        assert_eq!(done.deleted_dir.as_deref(), Some(dir.as_path()));
        assert!(!dir.exists());
        assert!(pfx.join("save.BfME2Save").exists(), "saves stay");
        assert_eq!(b.game(GameId::Bfme2).path, None);
        assert_eq!(b.game(GameId::Bfme2).cd_key.as_deref(), Some("KEY"), "settings stay for a reinstall");
        assert!(rotwk.exists());

        // Not installed now: nothing to delete, but the prefix can still go (Linux).
        assert!(b.uninstall(GameId::Bfme2, true, false).is_err());
        if !b.native {
            let done = b.uninstall(GameId::Bfme2, false, true).unwrap();
            assert!(done.deleted_prefix.is_some());
            assert!(!b.paths.prefix_root(GameId::Bfme2).exists());
        }
    }

    #[test]
    fn a_game_inside_its_prefix_goes_with_it() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        let dir = b.paths.prefix_root(GameId::Conquest).join("pfx/drive_c/Program Files/LOTR Conquest");
        install(&dir, "Conquest.exe");
        b.set_path(GameId::Conquest, &dir).unwrap();
        let plan = b.uninstall_plan(GameId::Conquest);
        assert!(plan.can_delete && plan.inside_prefix, "{plan:?}");
        let done = b.uninstall(GameId::Conquest, true, true).unwrap();
        assert!(done.deleted_dir.is_some() && !dir.exists());
        assert_eq!(done.deleted_prefix.is_some(), !b.native);
    }

    #[test]
    fn steam_copies_are_left_to_steam() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        let dir = t.path().join("home/.steam/steam/steamapps/common/Star Wars Galactic Battlegrounds Saga/Game");
        install(&dir, "battlegrounds_x1.exe");
        b.set_path(GameId::Swgb, &dir).unwrap();
        let plan = b.uninstall_plan(GameId::Swgb);
        assert!(plan.steam && !plan.can_delete);
        assert!(b.uninstall(GameId::Swgb, true, false).is_err());
        assert!(dir.exists());
        // Removing it from Beacon only is fine.
        b.uninstall(GameId::Swgb, false, false).unwrap();
        assert!(dir.exists() && b.game(GameId::Swgb).path.is_none());
    }
}
