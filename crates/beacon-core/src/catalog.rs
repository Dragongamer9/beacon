//! Built-in list of the well-known BFME mods and community patches, so the
//! launcher can point players at them and recognise them once installed.
//!
//! Beacon doesn't redistribute mods: each entry links to the mod's own page
//! (ModDB), and once downloaded the mod is added like any other (a .big,
//! a folder, or the mod's own launcher/installer run in the game's prefix).
//! Links that aren't a confirmed project page use ModDB search, which
//! always resolves.

use crate::games::GameId;
use crate::mods::{slugify, ModEntry};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    /// Gameplay/content mod.
    Mod,
    /// Community patch that updates the base game.
    Patch,
    /// A standalone game (download + installer), e.g. AotR Standalone.
    Game,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub game: GameId,
    pub kind: EntryKind,
    pub blurb: &'static str,
    pub url: String,
    /// Lowercase name fragments used to recognise it once installed.
    #[serde(skip)]
    pub matches: &'static [&'static str],
}

fn search(q: &str) -> String {
    format!("https://www.moddb.com/search?q={}", q.replace(' ', "+"))
}

pub fn entries() -> Vec<CatalogEntry> {
    use EntryKind::*;
    use GameId::*;
    let e = |id, name, game, kind, blurb, url: String, matches| CatalogEntry { id, name, game, kind, blurb, url, matches };
    vec![
        // ---- Rise of the Witch-king
        e("edain", "Edain Mod", Rotwk, Mod,
          "Large lore-focused overhaul with its own launcher and submods. Install the full mod, then add its launcher here.",
          "https://www.moddb.com/mods/edain-mod".into(), &["edain"]),
        e("aotr-mod", "Age of the Ring (8.x mod version)", Rotwk, Mod,
          "The older, pre-standalone release that ran as a RotWK mod. For current AotR, use the Age of the Ring tile.",
          search("Age of the Ring"), &["age of the ring", "aotr"]),
        e("dwarf-holds", "The Dwarf Holds", Rotwk, Mod,
          "Adds the Dwarven kingdoms as separate playable factions.",
          search("The Dwarf Holds"), &["dwarf holds"]),
        e("elven-alliance", "Elven Alliance", Rotwk, Mod,
          "Expands the Elven factions with new units and structures.",
          search("Elven Alliance"), &["elven alliance"]),
        e("rotwk-202", "RotWK Patch 2.02", Rotwk, Patch,
          "Community balance patch many RotWK mods expect. Run its installer in RotWK's prefix.",
          search("Rise of the Witch-king patch 2.02"), &["2.02", "202"]),
        // ---- Age of the Ring Standalone
        e("aotr-standalone", "Age of the Ring Standalone", Aotr, Game,
          "The fan remake, with both base games bundled — no RotWK or CD key needed. Download the installer zip (about 14 GB), extract it, then use Install in Beacon.",
          "https://www.moddb.com/mods/the-horse-lords-a-total-modification-for-bfme/downloads/age-of-the-ring-version-90".into(), &[]),
        // ---- LOTR: Conquest
        e("mordorwide", "MordorWide (online multiplayer)", Conquest, Patch,
          "Community re-implementation of EA's old Conquest servers. Needs game patch 1.1 and a patched Conquest.exe; their patch turns off SSL checks on login, so use a unique password.",
          "https://mordorwi.de/howtoplay".into(), &["mordorwide"]),
        e("conquest-fixes", "Conquest Fixes", Conquest, Mod,
          "Bug fixes for the vanilla game and restored cut levels, without changing balance.",
          "https://www.moddb.com/games/lotr-conquest/downloads/conquest-fixes".into(), &["conquest fixes", "conquestfixes"]),
        // ---- Other games
        e("uaw-xliveless", "xliveless (removes Games for Windows Live)", Uaw, Patch,
          "GFWL's servers are gone and the game won't start with it. Put xliveless's xlive.dll in the game folder; campaign and skirmish work. Beacon handles the DX10 setting itself.",
          "https://github.com/ValveSoftware/Proton/issues/1891".into(), &[]),
        e("exigo-intel-fix", "Integrated graphics fix", Exigo, Patch,
          "Replacement exe for the \"video card does not meet minimum requirements\" error on Intel graphics.",
          "https://www.moddb.com/downloads/armies-of-exigo-fix-win-en-exe-file-for-intel".into(), &[]),
        e("exigo-moddb", "Maps & mods on ModDB", Exigo, Mod,
          "Community downloads for Armies of Exigo.",
          "https://www.moddb.com/games/armies-of-exigo/downloads".into(), &[]),
        e("rol-heaven", "Rise of Legends Heaven", Rol, Patch,
          "Long-running community site with the official patches, guides and fixes.",
          "https://rol.heavengames.com/".into(), &[]),
        e("swgb-steam", "Galactic Battlegrounds Saga on Steam", Swgb, Game,
          "Still sold. Beacon runs battlegrounds_x1.exe directly, skipping Steam's broken player.exe.",
          "https://store.steampowered.com/app/356500/".into(), &[]),
        e("swgb-fix", "Crash & stuck-key fix", Swgb, Patch,
          "Patched battlegrounds_x1.exe that fixes the crash after ~20 minutes and stuck arrow keys under Wine. Steam and GOG builds.",
          "https://davidgow.net/hacks/swgb.html".into(), &[]),
        // ---- BFME II
        e("patch-222", "BFME Patch 2.22 / All-in-One", Bfme2, Patch,
          "Community patch for BFME II, also packaged with the All in One launcher. Run its installer in BFME II's prefix.",
          "https://www.moddb.com/mods/battle-for-middle-earth-patch-222".into(), &["2.22", "222"]),
        e("divide-conquer", "Divide and Conquer", Bfme2, Mod,
          "Splits the base factions into more distinct sub-factions.",
          search("Divide and Conquer battle for middle-earth"), &["divide and conquer"]),
        e("helms-deep-last-hope", "Helm's Deep: Last Hope", Bfme2, Mod,
          "Siege-focused mod built around large Helm's Deep battles.",
          search("Helms Deep Last Hope"), &["last hope", "helms deep", "helm's deep"]),
        // ---- BFME I
        e("bfme1-108", "BFME Patch 1.08", Bfme1, Patch,
          "Community balance and bug-fix patch for the first game.",
          search("BFME Patch 1.08"), &["1.08", "108"]),
        e("mee", "Middle-earth Extended Edition", Bfme1, Mod,
          "Extends the original factions and adds content cut from the base game.",
          search("Middle-earth Extended Edition"), &["extended edition", "mee"]),
        e("return-of-shadow", "Return of Shadow", Bfme1, Mod,
          "Long-running overhaul adding new factions and heroes.",
          search("Return of Shadow"), &["return of shadow"]),
        e("shadow-and-flame-bfme1", "Shadow and Flame", Bfme1, Mod,
          "Overhaul with reworked factions and visuals.",
          search("Shadow and Flame battle for middle-earth"), &["shadow and flame"]),
        e("king-of-the-west", "King of the West", Bfme1, Mod,
          "Gameplay and content overhaul for the first game.",
          search("King of the West battle for middle-earth"), &["king of the west"]),
        e("legends-3rd-age", "Legends of the 3rd Age", Bfme1, Mod,
          "Adds legendary heroes and units from across the Third Age.",
          search("Legends of 3rd Age"), &["legends of 3rd age", "legends of the 3rd age", "legends of the third age"]),
    ]
}

pub fn for_game(g: GameId) -> Vec<CatalogEntry> {
    entries().into_iter().filter(|e| e.game == g).collect()
}

/// Does this installed mod look like that catalog entry?
pub fn matches(entry: &CatalogEntry, m: &ModEntry) -> bool {
    if entry.game != m.game {
        return false;
    }
    let hay = format!("{} {}", m.name, m.path.to_string_lossy()).to_ascii_lowercase();
    let slug_hay = slugify(&hay);
    entry.matches.iter().any(|needle| {
        // Short numeric needles (patch versions) only count in the name,
        // so a "2.02" somewhere in a path doesn't mislabel a mod.
        if needle.chars().all(|c| c.is_ascii_digit() || c == '.') {
            m.name.to_ascii_lowercase().contains(needle)
        } else {
            hay.contains(needle) || slug_hay.contains(&slugify(needle))
        }
    })
}

/// Look for folders named like catalog mods under `roots` (a few levels
/// deep) and return the ones that look installable, as mods to link.
pub fn discover(roots: &[PathBuf], max_depth: usize) -> Vec<ModEntry> {
    let catalog = entries();
    let mut out: Vec<ModEntry> = Vec::new();
    for root in roots.iter().filter(|r| r.is_dir()) {
        let walker = walkdir::WalkDir::new(root).max_depth(max_depth).follow_links(false).into_iter().filter_entry(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            !(e.depth() > 0 && matches!(n.as_str(), "dosdevices" | "windows" | "node_modules" | ".cache" | ".git" | "pfx" | "prefixes"))
        });
        for e in walker.flatten() {
            if !e.file_type().is_dir() || e.depth() == 0 {
                continue;
            }
            // Anything inside an AotR Standalone install is the game itself
            // (the Age of the Ring tile), not a mod.
            if crate::games::aotr_root(e.path()).and_then(|r| crate::games::aotr_game_dir(&r)).is_some() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            for entry in catalog.iter().filter(|c| c.kind == EntryKind::Mod) {
                let hit = entry.matches.iter().any(|n| n.len() > 3 && !n.chars().all(|c| c.is_ascii_digit() || c == '.') && name.contains(n));
                if !hit {
                    continue;
                }
                if let Some(mut m) = crate::mods::inspect(entry.game, e.path(), Some(entry.name)) {
                    // Keep the folder's own name visible when it differs.
                    let folder = e.file_name().to_string_lossy().to_string();
                    if !folder.eq_ignore_ascii_case(entry.name) {
                        m.name = format!("{} ({folder})", entry.name);
                        m.id = format!("{}:{}", entry.game.slug(), slugify(&m.name));
                    }
                    if !out.iter().any(|o: &ModEntry| o.path == m.path) {
                        out.push(m);
                    }
                }
            }
        }
    }
    out
}

/// Folders worth checking for already-installed mods.
pub fn default_mod_roots(home: &Path, game_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = vec![home.join("Games"), home.join("Downloads"), home.join("Mods"), home.join(".wine/drive_c")];
    for d in game_dirs {
        roots.push(d.clone());
        if let Some(p) = d.parent() {
            roots.push(p.to_path_buf());
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_has_entries_and_ids_unique() {
        let all = entries();
        for g in GameId::ALL {
            assert!(all.iter().any(|e| e.game == g));
        }
        let mut ids: Vec<_> = all.iter().map(|e| e.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len());
        assert!(all.iter().all(|e| e.url.starts_with("https://")));
    }

    #[test]
    fn discovers_installed_mods() {
        let t = tempfile::tempdir().unwrap();
        let edain = t.path().join("Games/Edain Mod 4.8");
        std::fs::create_dir_all(&edain).unwrap();
        std::fs::write(edain.join("Edain Launcher.exe"), b"").unwrap();
        let aotr = t.path().join("Games/RotWK/aotr");
        std::fs::create_dir_all(&aotr).unwrap();
        std::fs::write(aotr.join("aotr_main.big"), b"").unwrap();
        let empty = t.path().join("Games/edain-notes");
        std::fs::create_dir_all(&empty).unwrap();

        // An AotR Standalone install must not show up as a mod.
        let standalone = t.path().join("Games/Age of the Ring");
        std::fs::create_dir_all(standalone.join("aotr")).unwrap();
        std::fs::write(standalone.join("aotr/lotrbfme2ep1.exe"), b"").unwrap();
        std::fs::write(standalone.join("aotr/aotr.big"), b"").unwrap();

        let found = discover(&[t.path().join("Games")], 4);
        let names: Vec<_> = found.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(found.len(), 2, "{names:?}");
        assert!(names.iter().any(|n| n.starts_with("Edain Mod")));
        assert!(names.iter().any(|n| n.starts_with("Age of the Ring")));

        let cat = for_game(GameId::Rotwk);
        let e = cat.iter().find(|c| c.id == "edain").unwrap();
        assert!(found.iter().any(|m| matches(e, m)));
    }
}
