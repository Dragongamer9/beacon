//! The BFME Foundation's All-in-One launcher (<https://bfmeladder.com>).
//!
//! Beacon never downloads the base games: it finds copies the All-in-One
//! launcher (or anything else) installed and uses them, and links to the
//! launcher for people who want it. What the launcher leaves behind, from its
//! source (github.com/MarcellVokk/BfmeFoundationProject):
//!
//! * **Game folders:** `<drive>:\BFME1`, `<drive>:\BFME2`, `<drive>:\RotWK`
//!   (the drive picked in its install dialog). Inside Wine that's
//!   `<prefix>/drive_c/BFME1` etc.
//! * **Registry** (`HKLM\SOFTWARE\WOW6432Node\…`, `InstallPath` with a trailing
//!   `\`): `Electronic Arts\EA Games\The Battle for Middle-earth`,
//!   `Electronic Arts\Electronic Arts\The Battle for Middle-earth II`,
//!   `Electronic Arts\Electronic Arts\The Lord of the Rings, The Rise of the
//!   Witch-king` (read by [`crate::installs`]), plus `App Paths\<exe>`.
//! * **Config:** `%APPDATA%\BFME Workshop\Config\active_patch_<0|1|2>.json` (the
//!   workshop package each game is synced to), `active_enhancements_<n>.json`,
//!   and `%APPDATA%\BFME Workshop\Library\<guid>.json` (downloaded packages).
//! * **Mods:** `<drive>:\BFME Workshop\Mods\<Name>-<Guid>\`; the active one is
//!   named in `mod.txt` in the game folder and started with `-mod`.
//! * **User data leaf names** differ from EA's: `My Battle for Middle-earth II
//!   Files`, `My Rise of the Witch-king Files`.

use crate::games::GameId;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Where to get the All-in-One launcher.
pub const LAUNCHER_URL: &str = "https://bfmeladder.com";

/// The launcher's folder name for a game (`<drive>:\<name>`).
pub fn folder_name(g: GameId) -> Option<&'static str> {
    match g {
        GameId::Bfme1 => Some("BFME1"),
        GameId::Bfme2 => Some("BFME2"),
        GameId::Rotwk => Some("RotWK"),
        _ => None,
    }
}

/// The launcher's (and its workshop's) game number: 0 BFME, 1 BFME II, 2 RotWK.
pub fn game_index(g: GameId) -> Option<i64> {
    match g {
        GameId::Bfme1 => Some(0),
        GameId::Bfme2 => Some(1),
        GameId::Rotwk => Some(2),
        _ => None,
    }
}

pub fn game_for_index(i: i64) -> Option<GameId> {
    match i {
        0 => Some(GameId::Bfme1),
        1 => Some(GameId::Bfme2),
        2 => Some(GameId::Rotwk),
        _ => None,
    }
}

/// `%APPDATA%\BFME Workshop`.
pub fn workshop_dir(appdata: &Path) -> PathBuf {
    appdata.join("BFME Workshop")
}

/// A field of a JSON object, matching the name case-insensitively (the
/// launcher writes PascalCase with Newtonsoft; other writers may not).
pub fn field<'a>(v: &'a Value, name: &str) -> Option<&'a Value> {
    v.as_object()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, x)| x)
}

pub fn field_str(v: &Value, name: &str) -> Option<String> {
    field(v, name).and_then(Value::as_str).map(str::to_string)
}

/// "Name (Version)" of the package a game is synced to, from
/// `active_patch_<n>.json`.
pub fn active_patch(appdata: &Path, g: GameId) -> Option<String> {
    let i = game_index(g)?;
    let text = std::fs::read_to_string(workshop_dir(appdata).join("Config").join(format!("active_patch_{i}.json"))).ok()?;
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    let name = field_str(&v, "Name")?;
    Some(match field_str(&v, "Version").filter(|s| !s.is_empty()) {
        Some(ver) => format!("{name} ({ver})"),
        None => name,
    })
}

/// The active mod's folder as the launcher wrote it in `<game>/mod.txt` (a Windows path).
pub fn active_mod(game_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(game_dir.join("mod.txt")).ok()?;
    let t = text.trim().trim_start_matches('\u{feff}').trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// The Wine prefix a host path sits in (the folder holding `drive_c`).
pub fn prefix_of(path: &Path) -> Option<PathBuf> {
    path.ancestors().find(|a| a.file_name().map(|n| n.eq_ignore_ascii_case("drive_c")).unwrap_or(false)).and_then(|d| d.parent()).map(Path::to_path_buf)
}

/// Is this game folder one the All-in-One launcher manages? It keeps a
/// `BFME Workshop` folder (cache, mods) next to the game folders, and an
/// `active_patch_<n>.json` in its config.
pub fn manages(game_dir: &Path, g: GameId, appdatas: &[PathBuf]) -> bool {
    let next_to = game_dir.parent().map(|p| p.join("BFME Workshop").is_dir()).unwrap_or(false);
    let named = folder_name(g).map(|n| game_dir.file_name().map(|f| f.eq_ignore_ascii_case(n)).unwrap_or(false)).unwrap_or(false);
    let config = game_index(g)
        .map(|i| appdatas.iter().any(|a| workshop_dir(a).join("Config").join(format!("active_patch_{i}.json")).is_file()))
        .unwrap_or(false);
    next_to || (named && config)
}

/// A mod the launcher's workshop put on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkshopMod {
    pub game: GameId,
    pub name: String,
    pub guid: String,
    pub path: PathBuf,
}

/// `<Name>-<Guid>` → (Name, Guid). Guids are UUIDs (36 chars) for user packages.
pub fn split_mod_dir(dir_name: &str) -> Option<(String, String)> {
    if dir_name.len() > 37 && dir_name.is_char_boundary(dir_name.len() - 37) {
        let (name, rest) = dir_name.split_at(dir_name.len() - 37);
        let guid = rest.strip_prefix('-')?;
        let uuid_like = guid.len() == 36
            && guid.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
        if uuid_like && !name.trim().is_empty() {
            return Some((name.trim().to_string(), guid.to_ascii_lowercase()));
        }
    }
    None
}

/// The game a downloaded package is for, from `Library\<guid>.json`.
fn library_game(appdatas: &[PathBuf], guid: &str) -> Option<GameId> {
    appdatas.iter().find_map(|a| {
        let text = std::fs::read_to_string(workshop_dir(a).join("Library").join(format!("{guid}.json"))).ok()?;
        let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
        field(&v, "Game").and_then(Value::as_i64).and_then(game_for_index)
    })
}

/// Mods in `<parent>\BFME Workshop\Mods\` next to the given game folders. The
/// game comes from the launcher's library; when it isn't there and only one
/// game shares that folder, that game.
pub fn workshop_mods(game_dirs: &[(GameId, PathBuf)], appdatas: &[PathBuf]) -> Vec<WorkshopMod> {
    let mut out: Vec<WorkshopMod> = Vec::new();
    let mut parents: Vec<PathBuf> = game_dirs.iter().filter_map(|(_, d)| d.parent().map(Path::to_path_buf)).collect();
    parents.sort();
    parents.dedup();
    for parent in parents {
        let sharing: Vec<GameId> = game_dirs.iter().filter(|(_, d)| d.parent() == Some(parent.as_path())).map(|(g, _)| *g).collect();
        let Ok(rd) = std::fs::read_dir(parent.join("BFME Workshop").join("Mods")) else { continue };
        for e in rd.flatten() {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let Some((name, guid)) = split_mod_dir(&e.file_name().to_string_lossy()) else { continue };
            let game = library_game(appdatas, &guid).or(if sharing.len() == 1 { Some(sharing[0]) } else { None });
            if let Some(game) = game {
                out.push(WorkshopMod { game, name, guid, path: e.path() });
            }
        }
    }
    out.sort_by(|a, b| a.game.cmp(&b.game).then(a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn mod_folder_names() {
        assert_eq!(
            split_mod_dir("Edain Mod-1b4e28ba-2fa1-11d2-883f-0016d3cca427"),
            Some(("Edain Mod".into(), "1b4e28ba-2fa1-11d2-883f-0016d3cca427".into()))
        );
        assert_eq!(split_mod_dir("official-1"), None);
        assert_eq!(split_mod_dir("Name-not-a-guid-at-all-but-long-enough-xxxxxxxxx"), None);
    }

    #[test]
    fn launcher_layout() {
        let t = tempfile::tempdir().unwrap();
        // <pfx>/drive_c/{BFME2,RotWK,BFME Workshop}, %APPDATA% in the prefix's user folder.
        let c = t.path().join("pfx/drive_c");
        let appdata = c.join("users/me/AppData/Roaming");
        for d in ["BFME2", "RotWK"] {
            fs::create_dir_all(c.join(d)).unwrap();
        }
        let guid = "1b4e28ba-2fa1-11d2-883f-0016d3cca427";
        let other = "2c4e28ba-2fa1-11d2-883f-0016d3cca427";
        fs::create_dir_all(c.join(format!("BFME Workshop/Mods/Edain Mod-{guid}"))).unwrap();
        fs::create_dir_all(c.join(format!("BFME Workshop/Mods/Unknown-{other}"))).unwrap();
        fs::create_dir_all(workshop_dir(&appdata).join("Config")).unwrap();
        fs::create_dir_all(workshop_dir(&appdata).join("Library")).unwrap();
        fs::write(workshop_dir(&appdata).join(format!("Library/{guid}.json")), format!(r#"{{"Guid":"{guid}","Name":"Edain Mod","Game":2,"Type":1}}"#)).unwrap();
        fs::write(workshop_dir(&appdata).join("Config/active_patch_1.json"), "\u{feff}{\"Guid\":\"x\",\"Name\":\"Patch 2.22\",\"Version\":\"40\"}").unwrap();
        fs::write(c.join("RotWK/mod.txt"), format!("C:\\BFME Workshop\\Mods\\Edain Mod-{guid}\r\n")).unwrap();

        let dirs = vec![(GameId::Bfme2, c.join("BFME2")), (GameId::Rotwk, c.join("RotWK"))];
        let appdatas = vec![appdata.clone()];
        assert!(manages(&c.join("BFME2"), GameId::Bfme2, &appdatas));
        assert!(!manages(&t.path().join("elsewhere/BFME2"), GameId::Bfme2, &[]));
        assert_eq!(active_patch(&appdata, GameId::Bfme2).as_deref(), Some("Patch 2.22 (40)"));
        assert_eq!(active_patch(&appdata, GameId::Rotwk), None);
        assert_eq!(active_mod(&c.join("RotWK")).unwrap(), format!("C:\\BFME Workshop\\Mods\\Edain Mod-{guid}"));
        assert_eq!(prefix_of(&c.join("RotWK")).unwrap(), t.path().join("pfx"));

        let mods = workshop_mods(&dirs, &appdatas);
        assert_eq!(mods.len(), 1, "the unknown one shares a folder with two games: {mods:?}");
        assert_eq!((mods[0].game, mods[0].name.as_str()), (GameId::Rotwk, "Edain Mod"));
        // Only one game next to it: that game.
        let mods = workshop_mods(&dirs[..1], &[]);
        assert_eq!(mods.len(), 2);
        assert!(mods.iter().all(|m| m.game == GameId::Bfme2));
    }
}
