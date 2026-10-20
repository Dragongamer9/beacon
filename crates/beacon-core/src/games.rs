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
}

/// Which shelf section a game sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Family {
    /// Battle for Middle-earth games (SAGE engine).
    Bfme,
    /// Other Lord of the Rings games.
    Lotr,
}

impl GameId {
    pub const ALL: [GameId; 5] = [GameId::Bfme1, GameId::Bfme2, GameId::Rotwk, GameId::Aotr, GameId::Conquest];

    pub fn slug(self) -> &'static str {
        match self {
            GameId::Bfme1 => "bfme1",
            GameId::Bfme2 => "bfme2",
            GameId::Rotwk => "rotwk",
            GameId::Aotr => "aotr",
            GameId::Conquest => "conquest",
        }
    }

    pub fn parse(s: &str) -> Option<GameId> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bfme1" | "bfme" | "1" | "lotrbfme" => Some(GameId::Bfme1),
            "bfme2" | "2" | "lotrbfme2" => Some(GameId::Bfme2),
            "rotwk" | "witchking" | "bfme2ep1" | "ep1" | "lotrbfme2ep1" => Some(GameId::Rotwk),
            "aotr" | "ageofthering" | "age-of-the-ring" | "age of the ring" => Some(GameId::Aotr),
            "conquest" | "lotrc" | "lotr-conquest" => Some(GameId::Conquest),
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
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            GameId::Bfme1 => "BFME",
            GameId::Bfme2 => "BFME II",
            GameId::Rotwk => "RotWK",
            GameId::Aotr => "AotR",
            GameId::Conquest => "Conquest",
        }
    }

    /// Main executable, matched case-insensitively.
    pub fn exe(self) -> &'static str {
        match self {
            GameId::Bfme1 => "lotrbfme.exe",
            GameId::Bfme2 => "lotrbfme2.exe",
            GameId::Rotwk | GameId::Aotr => "lotrbfme2ep1.exe",
            GameId::Conquest => "Conquest.exe",
        }
    }

    /// Key name under `HKLM\SOFTWARE\Electronic Arts\EA Games\`.
    /// AotR runs RotWK's engine and reads RotWK's key.
    pub fn registry_name(self) -> &'static str {
        match self {
            GameId::Bfme1 => "The Battle for Middle-earth",
            GameId::Bfme2 => "The Battle for Middle-earth II",
            GameId::Rotwk | GameId::Aotr => "The Lord of the Rings, The Rise of the Witch-king",
            // Not used: Conquest's own installer writes its registry.
            GameId::Conquest => "The Lord of the Rings - Conquest",
        }
    }

    /// Folder under %APPDATA% holding Options.ini, saves and replays.
    pub fn user_data_leaf(self) -> &'static str {
        match self {
            GameId::Bfme1 => "My Battle for Middle-earth Files",
            GameId::Bfme2 => "My The Battle for Middle-earth II Files",
            GameId::Rotwk => "My The Lord of the Rings, The Rise of the Witch-king Files",
            GameId::Aotr => "Age of the Ring",
            GameId::Conquest => "",
        }
    }

    pub fn family(self) -> Family {
        match self {
            GameId::Conquest => Family::Lotr,
            _ => Family::Bfme,
        }
    }

    /// BFME games: Beacon writes the EA registry keys, Options.ini
    /// resolution, and supports `-win` / `-mod`.
    pub fn is_sage(self) -> bool {
        self.family() == Family::Bfme
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
    let want = game.exe();
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        (name.eq_ignore_ascii_case(want) && e.path().is_file()).then(|| e.path())
    })
}

fn game_for_file(name: &str) -> Option<GameId> {
    [GameId::Bfme1, GameId::Bfme2, GameId::Rotwk, GameId::Conquest].into_iter().find(|g| name.eq_ignore_ascii_case(g.exe()))
}

/// `Conquest.exe` is a generic name; only count it when a nearby folder
/// says Conquest / LOTR.
fn looks_like_lotr_conquest(dir: &Path) -> bool {
    dir.ancestors().take(3).any(|a| {
        let n = squash(&a.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        n.contains("conquest") || n.contains("lotr") || n.contains("lordoftherings")
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
            let Some(mut game) = game_for_file(&entry.file_name().to_string_lossy()) else { continue };
            let Some(dir) = entry.path().parent() else { continue };
            if game == GameId::Conquest && !looks_like_lotr_conquest(dir) {
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
}
