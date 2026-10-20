//! The games Beacon knows about, and how to find them on disk.
//!
//! Age of the Ring Standalone is a fan-made game built on RotWK's engine.
//! It ships its own copies of BFME II and RotWK and its own
//! `lotrbfme2ep1.exe` (in `<AotR root>/aotr/`), so the scanner has to tell
//! it apart from a real RotWK install by where that exe sits.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameId {
    Bfme1,
    Bfme2,
    Rotwk,
    /// Age of the Ring Standalone (fan remake).
    Aotr,
    /// The Lord of the Rings: Conquest (Pandemic, 2009).
    Conquest,
    /// Universe at War: Earth Assault (Petroglyph, 2007).
    Uaw,
    /// Armies of Exigo (Black Hole Entertainment, 2004).
    Exigo,
    /// Rise of Nations: Rise of Legends (Big Huge Games, 2006).
    Rol,
    /// Star Wars: Galactic Battlegrounds Saga (LucasArts, 2001).
    Swgb,
}

/// Which shelf section a game sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// Battle for Middle-earth games (SAGE engine).
    Bfme,
    /// Other Lord of the Rings games.
    Lotr,
    /// Everything else.
    Other,
}

impl GameId {
    pub const ALL: [GameId; 9] = [
        GameId::Bfme1,
        GameId::Bfme2,
        GameId::Rotwk,
        GameId::Aotr,
        GameId::Conquest,
        GameId::Uaw,
        GameId::Exigo,
        GameId::Rol,
        GameId::Swgb,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            GameId::Bfme1 => "bfme1",
            GameId::Bfme2 => "bfme2",
            GameId::Rotwk => "rotwk",
            GameId::Aotr => "aotr",
            GameId::Conquest => "conquest",
            GameId::Uaw => "uaw",
            GameId::Exigo => "exigo",
            GameId::Rol => "rol",
            GameId::Swgb => "swgb",
        }
    }

    pub fn parse(s: &str) -> Option<GameId> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bfme1" | "bfme" | "1" | "lotrbfme" => Some(GameId::Bfme1),
            "bfme2" | "2" | "lotrbfme2" => Some(GameId::Bfme2),
            "rotwk" | "witchking" | "bfme2ep1" | "ep1" | "lotrbfme2ep1" => Some(GameId::Rotwk),
            "aotr" | "ageofthering" | "age-of-the-ring" | "age of the ring" => Some(GameId::Aotr),
            "conquest" | "lotrc" | "lotr-conquest" => Some(GameId::Conquest),
            "uaw" | "universeatwar" | "universe-at-war" => Some(GameId::Uaw),
            "exigo" | "armiesofexigo" | "armies-of-exigo" | "aoe" => Some(GameId::Exigo),
            "rol" | "riseoflegends" | "rise-of-legends" => Some(GameId::Rol),
            "swgb" | "galacticbattlegrounds" | "battlegrounds" => Some(GameId::Swgb),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            GameId::Bfme1 => "The Battle for Middle-earth",
            GameId::Bfme2 => "The Battle for Middle-earth II",
            GameId::Rotwk => "The Rise of the Witch-king",
            GameId::Aotr => "Age of the Ring",
            GameId::Conquest => "The Lord of the Rings: Conquest",
            GameId::Uaw => "Universe at War: Earth Assault",
            GameId::Exigo => "Armies of Exigo",
            GameId::Rol => "Rise of Nations: Rise of Legends",
            GameId::Swgb => "Star Wars: Galactic Battlegrounds Saga",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            GameId::Bfme1 => "BFME",
            GameId::Bfme2 => "BFME II",
            GameId::Rotwk => "RotWK",
            GameId::Aotr => "AotR",
            GameId::Conquest => "Conquest",
            GameId::Uaw => "UaW",
            GameId::Exigo => "Exigo",
            GameId::Rol => "RoL",
            GameId::Swgb => "SWGB",
        }
    }

    /// Main executable (the first of `exes()`), for messages.
    pub fn exe(self) -> &'static str {
        self.exes()[0]
    }

    /// Executables that identify the game, in order of preference,
    /// matched case-insensitively.
    pub fn exes(self) -> &'static [&'static str] {
        match self {
            GameId::Bfme1 => &["lotrbfme.exe"],
            GameId::Bfme2 => &["lotrbfme2.exe"],
            GameId::Rotwk | GameId::Aotr => &["lotrbfme2ep1.exe"],
            GameId::Conquest => &["Conquest.exe"],
            GameId::Uaw => &["UAWEA.exe"],
            GameId::Exigo => &["Exigo.exe"],
            GameId::Rol => &["RiseOfLegends.exe", "rol.exe"],
            // Saga: Clone Campaigns exe first; the base-game exe has a CD check.
            GameId::Swgb => &["battlegrounds_x1.exe", "battlegrounds_x2.exe", "Battlegrounds.exe"],
        }
    }

    /// Key name under `HKLM\SOFTWARE\Electronic Arts\EA Games\`.
    /// AotR runs RotWK's engine and reads RotWK's key.
    pub fn registry_name(self) -> &'static str {
        match self {
            GameId::Bfme1 => "The Battle for Middle-earth",
            GameId::Bfme2 => "The Battle for Middle-earth II",
            GameId::Rotwk | GameId::Aotr => "The Lord of the Rings, The Rise of the Witch-king",
            // Not used: these games' own installers write their registry.
            _ => "",
        }
    }

    /// Folder under %APPDATA% holding Options.ini, saves and replays.
    pub fn user_data_leaf(self) -> &'static str {
        match self {
            GameId::Bfme1 => "My Battle for Middle-earth Files",
            GameId::Bfme2 => "My The Battle for Middle-earth II Files",
            GameId::Rotwk => "My The Lord of the Rings, The Rise of the Witch-king Files",
            GameId::Aotr => "Age of the Ring",
            _ => "",
        }
    }

    pub fn family(self) -> Family {
        match self {
            GameId::Conquest => Family::Lotr,
            GameId::Uaw | GameId::Exigo | GameId::Rol | GameId::Swgb => Family::Other,
            _ => Family::Bfme,
        }
    }

    /// BFME games: Beacon writes the EA registry keys, Options.ini
    /// resolution, and supports `-win` / `-mod`.
    pub fn is_sage(self) -> bool {
        self.family() == Family::Bfme
    }

    /// Steam app id, for copies installed through Steam.
    pub fn steam_appid(self) -> Option<u32> {
        match self {
            GameId::Uaw => Some(10430),
            GameId::Swgb => Some(356500),
            _ => None,
        }
    }

    /// The game's Steam store page, for games still sold there. Only Galactic
    /// Battlegrounds Saga is: Universe at War (app 10430) was delisted in 2014, and
    /// Armies of Exigo and Rise of Legends were never on Steam.
    pub fn store_url(self) -> Option<&'static str> {
        match self {
            GameId::Swgb => Some("https://store.steampowered.com/app/356500/"),
            _ => None,
        }
    }

    /// Run inside a Wine virtual desktop unless the player turns it off.
    /// SWGB's fullscreen mode shifts mouse clicks under Proton.
    pub fn default_virtual_desktop(self) -> bool {
        self == GameId::Swgb
    }

    /// Folder-name hints required before trusting a generic exe name.
    fn folder_hints(self) -> &'static [&'static str] {
        match self {
            GameId::Conquest => &["conquest", "lotr", "lordoftherings"],
            GameId::Rol => &["legend"],
            _ => &[],
        }
    }

    /// Games whose registry carries the player's CD key (`ergc`).
    pub fn wants_cd_key(self) -> bool {
        matches!(self, GameId::Bfme1 | GameId::Bfme2 | GameId::Rotwk)
    }

    /// RotWK is an expansion and reads BFME II's install. AotR brings its own.
    pub fn requires(self) -> Option<GameId> {
        match self {
            GameId::Rotwk => Some(GameId::Bfme2),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoundGame {
    pub game: GameId,
    pub dir: PathBuf,
}

/// Case-insensitive lookup of the game's exe directly inside `dir`.
pub fn find_exe_in(dir: &Path, game: GameId) -> Option<PathBuf> {
    let files: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    game.exes().iter().find_map(|want| {
        files.iter().find(|p| p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(want)).unwrap_or(false)).cloned()
    })
}

/// A specific exe in `dir`, matched case-insensitively.
pub fn find_exe_in_named(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.is_file() && p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(name)).unwrap_or(false))
}

/// The game an exe name belongs to (case-insensitive). AotR shares RotWK's exe
/// name; the scanner reclassifies it by folder.
pub fn game_for_exe(name: &str) -> Option<GameId> {
    GameId::ALL.into_iter().filter(|g| *g != GameId::Aotr).find(|g| g.exes().iter().any(|e| name.eq_ignore_ascii_case(e)))
}

/// Generic exe names (Conquest.exe, rol.exe) only count when a nearby
/// folder name agrees.
fn folder_agrees(game: GameId, dir: &Path) -> bool {
    let hints = game.folder_hints();
    hints.is_empty()
        || dir.ancestors().take(3).any(|a| {
            let n = squash(&a.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            hints.iter().any(|h| n.contains(h))
        })
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// The Age of the Ring install folder containing `path`, if any: the
/// nearest ancestor named like "Age of the Ring" / "AgeoftheRing", or the
/// parent of an `aotr` folder.
pub fn aotr_root(path: &Path) -> Option<PathBuf> {
    let mut p = Some(path);
    for _ in 0..4 {
        let cur = p?;
        let name = squash(&cur.file_name()?.to_string_lossy());
        if name.contains("ageofthering") {
            return Some(cur.to_path_buf());
        }
        if name == "aotr" {
            return cur.parent().map(Path::to_path_buf);
        }
        p = cur.parent();
    }
    None
}

/// The folder holding AotR's own exe, given the install root or the exe's folder.
pub fn aotr_game_dir(path: &Path) -> Option<PathBuf> {
    if find_exe_in(path, GameId::Aotr).is_some() && path.file_name().map(|n| squash(&n.to_string_lossy()) == "aotr").unwrap_or(false) {
        return Some(path.to_path_buf());
    }
    let root = aotr_root(path).unwrap_or_else(|| path.to_path_buf());
    std::fs::read_dir(&root).ok()?.flatten().map(|e| e.path()).find(|d| {
        d.is_dir() && squash(&d.file_name().unwrap_or_default().to_string_lossy()) == "aotr" && find_exe_in(d, GameId::Aotr).is_some()
    })
}

/// Folders worth skipping while scanning (Wine plumbing, caches, huge trees).
fn skip_dir(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "dosdevices" | "windows" | "node_modules" | ".cache" | ".git" | "proc" | "sys" | "shadercache"
    )
}

/// Walk `roots` looking for any of the three game executables.
pub fn scan(roots: &[PathBuf], max_depth: usize) -> Vec<FoundGame> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let walker = WalkDir::new(root)
            .max_depth(max_depth)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !(e.file_type().is_dir() && e.depth() > 0 && skip_dir(&e.file_name().to_string_lossy())));
        for entry in walker.flatten() {
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(mut game) = game_for_exe(&entry.file_name().to_string_lossy()) else { continue };
            let Some(dir) = entry.path().parent() else { continue };
            if !folder_agrees(game, dir) {
                continue;
            }
            // Inside an Age of the Ring install: its own exe is AotR, and the
            // base-game copies it bundles aren't separate installs.
            if aotr_root(dir).is_some() {
                if game == GameId::Rotwk && aotr_game_dir(dir).as_deref() == Some(dir) {
                    game = GameId::Aotr;
                } else {
                    continue;
                }
            }
            if seen.insert((game, dir.to_path_buf())) {
                out.push(FoundGame { game, dir: dir.to_path_buf() });
            }
        }
    }
    out.sort_by(|a, b| a.game.cmp(&b.game).then(a.dir.cmp(&b.dir)));
    out
}

/// Where installs usually end up on a Linux gaming box.
#[cfg(not(windows))]
pub fn default_scan_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        home.join("Games"),
        home.join(".wine/drive_c"),
        home.join(".local/share/bottles/bottles"),
        home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
        home.join(".local/share/Steam/steamapps/compatdata"),
    ];
    if let Ok(user) = std::env::var("USER") {
        roots.push(PathBuf::from("/run/media").join(user));
    }
    roots
}

/// Where installs usually end up on Windows (the ones that exist): the
/// publishers' and stores' folders, Steam libraries, `%USERPROFILE%\Games` and
/// the BFME All-in-One launcher's `<drive>:\BFME1`, `BFME2`, `RotWK`.
#[cfg(windows)]
pub fn default_scan_roots(home: &Path) -> Vec<PathBuf> {
    let env = |k: &str| std::env::var(k).ok();
    let mut roots = crate::installs::windows_scan_roots(home, &env);
    if let Some(steam) = crate::installs::steam_root_from(&crate::installs::windows_registry()) {
        roots.extend(crate::installs::steam_common_dirs(&steam));
    }
    roots.retain(|p| p.is_dir());
    roots.dedup();
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aliases() {
        assert_eq!(GameId::parse("BFME"), Some(GameId::Bfme1));
        assert_eq!(GameId::parse("bfme2"), Some(GameId::Bfme2));
        assert_eq!(GameId::parse("RotWK"), Some(GameId::Rotwk));
        assert_eq!(GameId::parse("AotR"), Some(GameId::Aotr));
        assert_eq!(GameId::parse("conquest"), Some(GameId::Conquest));
        assert_eq!(GameId::Conquest.family(), Family::Lotr);
        assert!(GameId::Aotr.is_sage() && !GameId::Conquest.is_sage());
        assert_eq!(GameId::parse("bfme3"), None);
    }

    #[test]
    #[cfg(unix)]   // makes a Wine-style "dosdevices/c:" folder, which isn't a valid name on Windows
    fn scan_finds_games_case_insensitively() {
        let t = tempfile::tempdir().unwrap();
        let b1 = t.path().join("Games/BFME1");
        let b2 = t.path().join("Games/deep/er/BFME2");
        let rk = t.path().join("Games/deep/er/RotWK");
        let junk = t.path().join("Games/prefix/dosdevices/c:");
        for d in [&b1, &b2, &rk, &junk] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(b1.join("LOTRBFME.EXE"), b"").unwrap();
        std::fs::write(b2.join("lotrbfme2.exe"), b"").unwrap();
        std::fs::write(rk.join("lotrbfme2ep1.exe"), b"").unwrap();
        std::fs::write(junk.join("lotrbfme.exe"), b"").unwrap();

        let found = scan(&[t.path().join("Games")], 8);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().all(|f| f.game != GameId::Aotr));
        assert_eq!(found[0], FoundGame { game: GameId::Bfme1, dir: b1.clone() });
        assert_eq!(found[2].game, GameId::Rotwk);
        assert!(find_exe_in(&b1, GameId::Bfme1).is_some());
        assert!(find_exe_in(&b1, GameId::Bfme2).is_none());
    }

    #[test]
    fn aotr_standalone_is_its_own_game() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Games/age-of-the-ring/drive_c/AgeoftheRing");
        // Layout per the Lutris installer: <root>/aotr/lotrbfme2ep1.exe,
        // plus bundled base games that must not be picked up as installs.
        for (d, exe) in [("aotr", "lotrbfme2ep1.exe"), ("BFME2", "lotrbfme2.exe"), ("RotWK", "lotrbfme2ep1.exe")] {
            std::fs::create_dir_all(root.join(d)).unwrap();
            std::fs::write(root.join(d).join(exe), b"").unwrap();
        }
        let real = t.path().join("Games/RotWK");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("lotrbfme2ep1.exe"), b"").unwrap();

        let found = scan(&[t.path().join("Games")], 9);
        assert_eq!(found, vec![FoundGame { game: GameId::Rotwk, dir: real.clone() }, FoundGame { game: GameId::Aotr, dir: root.join("aotr") }]);
        assert_eq!(aotr_root(&root.join("aotr")).unwrap(), root);
        assert_eq!(aotr_game_dir(&root).unwrap(), root.join("aotr"));
        assert!(aotr_root(&real).is_none());
    }

    #[test]
    fn conquest_detection_needs_a_conquest_folder() {
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("Games/The Lord of the Rings - Conquest");
        let other = t.path().join("Games/SomeStrategyGame/bin");
        for d in [&real, &other] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("Conquest.exe"), b"").unwrap();
        }
        // The dedicated-server tools aren't the game.
        std::fs::write(real.join("ConquestServer.exe"), b"").unwrap();
        let found = scan(&[t.path().join("Games")], 6);
        assert_eq!(found, vec![FoundGame { game: GameId::Conquest, dir: real }]);
    }

    #[test]
    fn other_games_detected() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path().join("Games");
        let swgb = g.join("steamapps/common/Star Wars Galactic Battlegrounds Saga/Game");
        let uaw = g.join("Universe at War Earth Assault");
        let exigo = g.join("Armies of Exigo/Exigo Files");
        let rol = g.join("Rise of Legends");
        let not_rol = g.join("Tools");
        for (d, exe) in [(&swgb, "Battlegrounds.exe"), (&swgb, "battlegrounds_x1.exe"), (&uaw, "UAWEA.exe"), (&uaw, "LaunchUAW.exe"), (&exigo, "Exigo.exe"), (&rol, "RiseOfLegends.exe"), (&not_rol, "rol.exe")] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join(exe), b"").unwrap();
        }
        let found = scan(&[g], 8);
        let pairs: Vec<_> = found.iter().map(|f| (f.game, f.dir.clone())).collect();
        assert_eq!(pairs, vec![(GameId::Uaw, uaw), (GameId::Exigo, exigo), (GameId::Rol, rol), (GameId::Swgb, swgb.clone())]);
        // The Clone Campaigns exe wins over the base one.
        assert!(find_exe_in(&swgb, GameId::Swgb).unwrap().ends_with("battlegrounds_x1.exe"));
        assert_eq!(GameId::Swgb.family(), Family::Other);
        assert_eq!(GameId::parse("swgb").unwrap().steam_appid(), Some(356500));
    }

    #[test]
    fn store_links_only_for_games_still_sold() {
        assert_eq!(GameId::Swgb.store_url(), Some("https://store.steampowered.com/app/356500/"));
        for g in GameId::ALL.into_iter().filter(|g| *g != GameId::Swgb) {
            assert_eq!(g.store_url(), None, "{g:?}");
        }
        assert_eq!(game_for_exe("LOTRBFME2.EXE"), Some(GameId::Bfme2));
        assert_eq!(game_for_exe("lotrbfme2ep1.exe"), Some(GameId::Rotwk));
        assert_eq!(game_for_exe("setup.exe"), None);
    }
}
