//! Community versions of the "Other Games".
//!
//! Two things live here:
//! * **Editions** – playable variants Beacon can launch, picked per game.
//!   SWGB editions are different executables (Expanding Fronts ships its
//!   own `battlegrounds_x2.exe`); Universe at War editions are mod folders
//!   under `<game>/Mods/` loaded with the Petroglyph engine's `MODPATH=`.
//!   The community edition is the default whenever it's installed.
//! * **Steps** – the guided setup for each game's community version: open
//!   the page, then hand Beacon the download (archive or extracted folder)
//!   to merge into the game, add under Mods, or run as an installer.

use crate::games::GameId;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Edition {
    pub id: &'static str,
    pub name: &'static str,
    pub blurb: &'static str,
    pub community: bool,
    /// Executables for this edition, in preference order.
    #[serde(skip)]
    pub exes: &'static [&'static str],
    /// Folder-name needles for a mod folder under `<game>/Mods/`.
    #[serde(skip)]
    pub modpath: &'static [&'static str],
}

/// What a step's main action does with the file or folder the player picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// Only a link (forum, project page).
    Link,
    /// Run the picked .exe in the game's prefix.
    Installer,
    /// Extract if needed, then merge into the game folder (backing up).
    Merge,
    /// Extract if needed, then copy to `<game>/Mods/<mods_name>`.
    Mods,
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub id: &'static str,
    pub title: &'static str,
    pub detail: &'static str,
    pub url: Option<&'static str>,
    pub action: Action,
    /// Folder name under Mods (Action::Mods).
    #[serde(skip)]
    pub mods_name: &'static str,
}

pub fn editions(g: GameId) -> Vec<Edition> {
    let e = |id, name, blurb, community, exes, modpath| Edition { id, name, blurb, community, exes, modpath };
    match g {
        GameId::Swgb => vec![
            e("expanding-fronts", "Expanding Fronts", "The community expansion: new factions, campaign and balance. Online play is through Voobly.", true, &["battlegrounds_x2.exe"], &[]),
            e("clone-campaigns", "Clone Campaigns", "The Saga edition's expansion, unmodded.", false, &["battlegrounds_x1.exe"], &[]),
            e("original", "Original game", "The base game without the expansion.", false, &["Battlegrounds.exe"], &[]),
        ],
        GameId::Uaw => vec![
            e("community-patch", "Community Patch", "Fan-voted balance and bug-fix patch from the UaW Discord.", true, &[], &["communitypatch"]),
            e("human-resistance", "Human Resistance Reborn", "Community mod adding a playable Human faction with improved AI.", true, &[], &["humanresistance", "humanmod", "hrr"]),
            e("vanilla", "Patch 3 (vanilla)", "The last official version.", false, &[], &[]),
        ],
        _ => vec![],
    }
}

pub fn steps(g: GameId) -> Vec<Step> {
    let s = |id, title, detail, url, action, mods_name| Step { id, title, detail, url, action, mods_name };
    use Action::*;
    match g {
        GameId::Swgb => vec![
            s("expanding-fronts", "Expanding Fronts",
              "Download the FULL Expanding Fronts zip from ModDB (1.5.5, about 1.15 GB), then pick it here. Beacon merges its Game files into your install and backs up anything it replaces.",
              Some("https://www.moddb.com/mods/star-wars-galactic-battlegrounds-expanding-fronts/downloads"), Merge, ""),
        ],
        GameId::Uaw => vec![
            s("patch3", "Official Patch 3",
              "The community versions need Patch 3. Steam copies already have it; for disc copies, download it and run it here.",
              Some("https://www.moddb.com/games/universe-at-war-earth-assault/downloads/patch-3-universe-at-war-earth-assault"), Installer, ""),
            s("community-patch", "Community Patch",
              "Download the Community Patch archive, then pick it here. Beacon adds it under Mods and selects it.",
              Some("https://imperium-ww.pl/2023/10/19/universe-at-war-earth-assault-community-patch-2-0/"), Mods, "CommunityPatch"),
            s("human-resistance", "Human Resistance Reborn (optional)",
              "Download the full version, then pick it here to add it under Mods.",
              Some("https://www.moddb.com/mods/human-resistance-reborn"), Mods, "HumanResistanceReborn"),
            s("xliveless", "xliveless",
              "Put xliveless's xlive.dll in the game folder (pick it here to copy it in). Games for Windows Live is gone; campaign and skirmish work.",
              Some("https://github.com/ValveSoftware/Proton/issues/1891"), Merge, ""),
        ],
        GameId::Exigo => vec![
            s("patch14", "Patch 1.4",
              "The version the community plays: multiplayer and balance fixes over retail 1.0b–1.3. Old saves won't load after it. Download it, then run it here.",
              Some("https://www.moddb.com/downloads/patch-14"), Installer, ""),
            s("intel-fix", "Integrated graphics fix",
              "Only if the game says your video card doesn't meet the minimum requirements. Pick the fixed exe (or its archive) to put it in the game folder.",
              Some("https://www.moddb.com/downloads/armies-of-exigo-fix-win-en-exe-file-for-intel"), Merge, ""),
            s("forum", "Armies of Exigo community",
              "Forum with downloads and guides for playing together today.",
              Some("https://armiesofexigo.boards.net/"), Link, ""),
        ],
        GameId::Rol => vec![
            s("patch", "Latest official patch",
              "Rise of Legends Heaven hosts the official patches. Download the one for your region, then run it here.",
              Some("https://rol.heavengames.com/"), Installer, ""),
            s("crash-fix", "Long-game crash fix",
              "Community fix for crashes after long play sessions, from the Rise of Legends Heaven forum. Pick the fix's files or archive to merge them into the game.",
              Some("https://rol.heavengames.com/cgi-bin/forums/display.cgi?action=st&fn=2&tn=994"), Merge, ""),
            s("user-patch", "User Patch Project",
              "A community patch in progress on the RoL Discord (sound overlap, long-game crashes, editor fixes). Check here for a release.",
              Some("https://www.moddb.com/games/rise-of-legends/news/rise-of-legends-user-patch-project"), Link, ""),
        ],
        _ => vec![],
    }
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// Whether an edition is present in `game_dir`, and its mod folder if it has one.
pub fn locate(e: &Edition, game_dir: &Path) -> Option<Option<PathBuf>> {
    if !e.exes.is_empty() {
        let found = e.exes.iter().any(|x| child_ci(game_dir, x).map(|p| p.is_file()).unwrap_or(false));
        return found.then_some(None);
    }
    if !e.modpath.is_empty() {
        let mods = child_ci(game_dir, "Mods")?;
        let hit = std::fs::read_dir(mods).ok()?.flatten().map(|d| d.path()).filter(|p| p.is_dir()).find(|p| {
            let n = squash(&p.file_name().unwrap_or_default().to_string_lossy());
            e.modpath.iter().any(|needle| n.contains(needle))
        })?;
        return Some(Some(hit));
    }
    Some(None) // the plain game
}

/// `MODPATH=` value for a mod folder: relative to the game folder when
/// inside it (what the engine expects), otherwise a Wine path.
pub fn modpath_arg(game_dir: &Path, mod_dir: &Path) -> String {
    modpath_arg_for(game_dir, mod_dir, false)
}

/// `modpath_arg`, with `native` (Windows) giving a plain path for a folder outside the game.
pub fn modpath_arg_for(game_dir: &Path, mod_dir: &Path, native: bool) -> String {
    match mod_dir.strip_prefix(game_dir) {
        Ok(rel) => format!("MODPATH={}", rel.to_string_lossy().replace('/', "\\")),
        Err(_) if native => format!("MODPATH={}", mod_dir.to_string_lossy()),
        Err(_) => format!("MODPATH={}", crate::registry::wine_path(mod_dir, false)),
    }
}

// ---- files ------------------------------------------------------------

/// Existing child of `dir` whose name matches case-insensitively.
fn child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

/// Extract an archive with whatever the system has (7z, bsdtar, unzip).
pub fn extract(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let a = archive.to_string_lossy().to_string();
    let d = dest.to_string_lossy().to_string();
    let tries: Vec<(&str, Vec<String>)> = vec![
        ("7z", vec!["x".into(), "-y".into(), format!("-o{d}"), a.clone()]),
        ("7zz", vec!["x".into(), "-y".into(), format!("-o{d}"), a.clone()]),
        ("bsdtar", vec!["-xf".into(), a.clone(), "-C".into(), d.clone()]),
        ("unzip", vec!["-q".into(), "-o".into(), a.clone(), "-d".into(), d.clone()]),
    ];
    let mut tried = Vec::new();
    for (bin, args) in tries {
        match std::process::Command::new(bin).args(&args).output() {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => tried.push(format!("{bin}: {}", String::from_utf8_lossy(&out.stderr).trim())),
            Err(_) => continue, // not installed
        }
    }
    if tried.is_empty() {
        bail!("no archive tool found — install 7-Zip (7z), libarchive (bsdtar) or unzip, or extract it yourself and pick the folder");
    }
    bail!("couldn't extract {}: {}", archive.display(), tried.join("; "))
}

/// Skip wrapper folders: descend while the folder holds exactly one
/// directory and nothing else.
pub fn content_root(dir: &Path) -> PathBuf {
    let mut cur = dir.to_path_buf();
    loop {
        let entries: Vec<PathBuf> = std::fs::read_dir(&cur).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        if entries.len() == 1 && entries[0].is_dir() && !entries[0].file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("data")).unwrap_or(false) {
            cur = entries[0].clone();
        } else {
            return cur;
        }
    }
}

/// Copy `src` into `dst`, matching existing names case-insensitively (so
/// `DATA` merges into `Data` as it would on Windows). Files that would be
/// overwritten are first copied to `backup`. Returns files written.
pub fn merge_dir(src: &Path, dst: &Path, backup: &Path) -> Result<usize> {
    std::fs::create_dir_all(dst)?;
    let mut n = 0;
    for entry in std::fs::read_dir(src)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let target = child_ci(dst, &name).unwrap_or_else(|| dst.join(&name));
        let ft = entry.file_type()?;
        if ft.is_dir() {
            let sub_backup = backup.join(target.file_name().unwrap());
            n += merge_dir(&entry.path(), &target, &sub_backup)?;
        } else {
            if target.is_file() {
                std::fs::create_dir_all(backup)?;
                std::fs::copy(&target, backup.join(target.file_name().unwrap())).with_context(|| format!("backing up {}", target.display()))?;
            }
            std::fs::copy(entry.path(), &target).with_context(|| format!("copying to {}", target.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// Pick the part of an extracted download to merge into `game_dir`: when the
/// game folder is called "Game" and the download has a "Game" folder (the
/// Expanding Fronts layout), merge that; otherwise the whole thing.
pub fn merge_source(root: &Path, game_dir: &Path) -> PathBuf {
    let game_is_game = game_dir.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("game")).unwrap_or(false);
    if game_is_game {
        if let Some(inner) = child_ci(root, "Game").filter(|p| p.is_dir()) {
            return inner;
        }
    }
    root.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn tables() {
        for g in [GameId::Swgb, GameId::Uaw, GameId::Exigo, GameId::Rol] {
            assert!(!steps(g).is_empty(), "{g:?} needs community steps");
            assert!(steps(g).iter().all(|s| s.url.map(|u| u.starts_with("https://")).unwrap_or(true)));
        }
        assert!(editions(GameId::Swgb)[0].community && editions(GameId::Uaw)[0].community);
        assert!(editions(GameId::Bfme1).is_empty());
    }

    #[test]
    fn locate_editions() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path().join("Game");
        fs::create_dir_all(g.join("mods/Community_Patch_2.0/Data")).unwrap();
        fs::write(g.join("battlegrounds_x1.exe"), b"").unwrap();
        let swgb = editions(GameId::Swgb);
        assert!(locate(&swgb[0], &g).is_none(), "EF not installed yet");
        assert_eq!(locate(&swgb[1], &g), Some(None));
        let uaw = editions(GameId::Uaw);
        let cp = locate(&uaw[0], &g).unwrap().unwrap();
        assert!(cp.ends_with("Community_Patch_2.0"));
        assert_eq!(modpath_arg(&g, &cp), "MODPATH=mods\\Community_Patch_2.0");
        assert!(locate(&uaw[1], &g).is_none());
        assert_eq!(locate(&uaw[2], &g), Some(None), "vanilla is always there");
    }

    // Needs a case-sensitive file system to tell `DATA` from `Data`.
    #[cfg(unix)]
    #[test]
    fn merge_is_case_insensitive_and_backs_up() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("SWGB/Game");
        fs::create_dir_all(game.join("Data")).unwrap();
        fs::write(game.join("Data/genie_x1.dat"), b"old").unwrap();
        fs::write(game.join("battlegrounds_x1.exe"), b"x1").unwrap();
        // Download layout: wrapper folder → GAME → DATA (different case).
        let dl = t.path().join("dl/EF 1.5.5");
        fs::create_dir_all(dl.join("GAME/DATA")).unwrap();
        fs::write(dl.join("GAME/DATA/genie_x1.dat"), b"new").unwrap();
        fs::write(dl.join("GAME/battlegrounds_x2.exe"), b"x2").unwrap();
        fs::write(dl.join("GAME/../Expanding Fronts Manual Installation Guide.pdf"), b"").unwrap();

        let root = content_root(&t.path().join("dl"));
        assert!(root.ends_with("EF 1.5.5"));
        let src = merge_source(&root, &game);
        assert!(src.ends_with("GAME"));
        let backup = t.path().join("backup");
        assert_eq!(merge_dir(&src, &game, &backup).unwrap(), 2);
        assert_eq!(fs::read(game.join("Data/genie_x1.dat")).unwrap(), b"new");
        assert!(!game.join("DATA").exists(), "merged into existing Data");
        assert_eq!(fs::read(backup.join("Data/genie_x1.dat")).unwrap(), b"old");
        assert!(game.join("battlegrounds_x2.exe").is_file());
        assert!(locate(&editions(GameId::Swgb)[0], &game).is_some());
    }
}
