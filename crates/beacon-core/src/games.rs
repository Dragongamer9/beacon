//! The three games Beacon knows about, and how to find them on disk.

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
}

impl GameId {
    pub const ALL: [GameId; 3] = [GameId::Bfme1, GameId::Bfme2, GameId::Rotwk];

    pub fn slug(self) -> &'static str {
        match self {
            GameId::Bfme1 => "bfme1",
            GameId::Bfme2 => "bfme2",
            GameId::Rotwk => "rotwk",
        }
    }

    pub fn parse(s: &str) -> Option<GameId> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bfme1" | "bfme" | "1" | "lotrbfme" => Some(GameId::Bfme1),
            "bfme2" | "2" | "lotrbfme2" => Some(GameId::Bfme2),
            "rotwk" | "witchking" | "bfme2ep1" | "ep1" | "lotrbfme2ep1" => Some(GameId::Rotwk),
            _ => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            GameId::Bfme1 => "The Battle for Middle-earth",
            GameId::Bfme2 => "The Battle for Middle-earth II",
            GameId::Rotwk => "The Rise of the Witch-king",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            GameId::Bfme1 => "BFME",
            GameId::Bfme2 => "BFME II",
            GameId::Rotwk => "RotWK",
        }
    }

    /// Main executable, matched case-insensitively.
    pub fn exe(self) -> &'static str {
        match self {
            GameId::Bfme1 => "lotrbfme.exe",
            GameId::Bfme2 => "lotrbfme2.exe",
            GameId::Rotwk => "lotrbfme2ep1.exe",
        }
    }

    /// Key name under `HKLM\SOFTWARE\Electronic Arts\EA Games\`.
    pub fn registry_name(self) -> &'static str {
        match self {
            GameId::Bfme1 => "The Battle for Middle-earth",
            GameId::Bfme2 => "The Battle for Middle-earth II",
            GameId::Rotwk => "The Lord of the Rings, The Rise of the Witch-king",
        }
    }

    /// Folder under %APPDATA% holding Options.ini, saves and replays.
    pub fn user_data_leaf(self) -> &'static str {
        match self {
            GameId::Bfme1 => "My Battle for Middle-earth Files",
            GameId::Bfme2 => "My The Battle for Middle-earth II Files",
            GameId::Rotwk => "My The Lord of the Rings, The Rise of the Witch-king Files",
        }
    }

    /// RotWK is an expansion and reads BFME II's install.
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
    GameId::ALL.into_iter().find(|g| name.eq_ignore_ascii_case(g.exe()))
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
            let Some(game) = game_for_file(&entry.file_name().to_string_lossy()) else { continue };
            let Some(dir) = entry.path().parent() else { continue };
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
        assert_eq!(found[0], FoundGame { game: GameId::Bfme1, dir: b1.clone() });
        assert_eq!(found[2].game, GameId::Rotwk);
        assert!(find_exe_in(&b1, GameId::Bfme1).is_some());
        assert!(find_exe_in(&b1, GameId::Bfme2).is_none());
    }
}
