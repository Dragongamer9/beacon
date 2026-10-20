//! Mods. Two shapes cover what the BFME community ships:
//!
//! * `mod`      – .big files (or a folder of them) loaded with the game's
//!                own `-mod` switch.
//! * `launcher` – a mod with its own Windows launcher/installer exe, run
//!                inside the game's prefix so it sees the game's registry.
//!
//! Mods live in `<data>/mods/<game>/<ModName>/` (or as a single .big there),
//! optionally with a `beacon-mod.json` manifest, or are linked from anywhere.

use crate::games::GameId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModKind {
    Mod,
    Launcher,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModEntry {
    pub id: String,
    pub name: String,
    pub game: GameId,
    pub kind: ModKind,
    /// .big file or folder (kind=mod), or the exe to run (kind=launcher).
    pub path: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    /// True when it came from the mods folder rather than being linked.
    #[serde(default)]
    pub in_library: bool,
}

#[derive(Debug, Default, Deserialize)]
struct Manifest {
    name: Option<String>,
    kind: Option<ModKind>,
    /// Relative path to the .big/folder or launcher exe.
    entry: Option<String>,
    #[serde(default)]
    args: Vec<String>,
}

pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case(ext)).unwrap_or(false)
}

fn files_with_ext(dir: &Path, ext: &str, depth: usize) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(depth)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && has_ext(e.path(), ext))
        .map(|e| e.into_path())
        .collect()
}

/// Work out what a path is: a .big, a folder of .bigs, or a launcher.
pub fn inspect(game: GameId, path: &Path, name: Option<&str>) -> Option<ModEntry> {
    let fallback_name = path.file_stem()?.to_string_lossy().to_string();
    let mut entry = ModEntry {
        id: String::new(),
        name: name.map(str::to_string).unwrap_or(fallback_name),
        game,
        kind: ModKind::Mod,
        path: path.to_path_buf(),
        args: vec![],
        in_library: false,
    };

    if path.is_file() {
        if has_ext(path, "big") {
            entry.kind = ModKind::Mod;
        } else if has_ext(path, "exe") {
            entry.kind = ModKind::Launcher;
        } else {
            return None;
        }
    } else if path.is_dir() {
        let manifest: Manifest = std::fs::read_to_string(path.join("beacon-mod.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if let Some(n) = manifest.name.filter(|_| name.is_none()) {
            entry.name = n;
        }
        entry.args = manifest.args;
        match (manifest.kind, manifest.entry) {
            (Some(kind), Some(rel)) => {
                entry.kind = kind;
                entry.path = path.join(rel);
            }
            (Some(ModKind::Mod), None) => entry.kind = ModKind::Mod,
            (kind, rel) => {
                let rel_path = rel.map(|r| path.join(r));
                let bigs = files_with_ext(path, "big", 2);
                let exes = files_with_ext(path, "exe", 2);
                let launcher = exes
                    .iter()
                    .find(|e| e.file_name().unwrap().to_string_lossy().to_ascii_lowercase().contains("launcher"))
                    .or(if exes.len() == 1 { exes.first() } else { None });
                if kind == Some(ModKind::Launcher) || (bigs.is_empty() && launcher.is_some()) {
                    entry.kind = ModKind::Launcher;
                    entry.path = rel_path.or_else(|| launcher.cloned())?;
                } else if !bigs.is_empty() {
                    entry.kind = ModKind::Mod;
                    entry.path = rel_path.unwrap_or_else(|| path.to_path_buf());
                } else {
                    return None;
                }
            }
        }
    } else {
        return None;
    }
    entry.id = format!("{}:{}", game.slug(), slugify(&entry.name));
    Some(entry)
}

/// Everything in `<mods_dir>/<game>/`.
pub fn scan_library(mods_dir: &Path, game: GameId) -> Vec<ModEntry> {
    let dir = mods_dir.join(game.slug());
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out: Vec<ModEntry> = rd
        .flatten()
        .filter_map(|e| inspect(game, &e.path(), None))
        .map(|mut m| {
            m.in_library = true;
            m
        })
        .collect();
    out.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_kinds() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("mods/rotwk");
        std::fs::create_dir_all(root.join("Age of the Ring")).unwrap();
        std::fs::write(root.join("Age of the Ring/aotr.big"), b"").unwrap();
        std::fs::create_dir_all(root.join("Edain")).unwrap();
        std::fs::write(root.join("Edain/Edain Launcher.exe"), b"").unwrap();
        std::fs::write(root.join("Edain/readme.txt"), b"").unwrap();
        std::fs::write(root.join("widescreen.big"), b"").unwrap();
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        std::fs::create_dir_all(root.join("Custom")).unwrap();
        std::fs::write(root.join("Custom/beacon-mod.json"), r#"{"name":"My Mod","kind":"mod","entry":"data","args":["-noshellmap"]}"#).unwrap();

        let mods = scan_library(&t.path().join("mods"), GameId::Rotwk);
        let names: Vec<_> = mods.iter().map(|m| (m.name.as_str(), m.kind)).collect();
        assert_eq!(
            names,
            vec![("Age of the Ring", ModKind::Mod), ("Edain", ModKind::Launcher), ("My Mod", ModKind::Mod), ("widescreen", ModKind::Mod)]
        );
        let edain = &mods[1];
        assert!(edain.path.ends_with("Edain Launcher.exe"));
        assert_eq!(edain.id, "rotwk:edain");
        assert_eq!(mods[2].args, vec!["-noshellmap"]);
        assert!(mods[2].path.ends_with("Custom/data"));
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Age of the Ring 8.4!"), "age-of-the-ring-8-4");
    }
}
