//! Mods and patches downloaded inside Beacon, no
//! browser: from a `sources.json` mod source (the catalog's Download button)
//! or any ModDB link you paste.
//!
//! Two steps, so you see what you're getting first:
//! 1. [`Beacon::mod_lookup`] follows the source to the file (name, size, MD5).
//! 2. [`Beacon::mod_install`] downloads it (resumable, checked), opens it
//!    (.zip/.7z/.rar), and puts it where it belongs **for the game it's for**:
//!    * **library**: `.big` files (or a mod's launcher) → `<mods>/<game>/<Name>/`,
//!      a mod like any other that loads with `-mod`;
//!    * **game-folder**: files that replace the game's own (a fixed exe, a patch's
//!      data) are merged into the game's folder, wherever Beacon found it, with
//!      every replaced file backed up under `<data>/backups/<game>/`;
//!    * **installer**: a setup program is kept under `<data>/tools/<game>-mods/`
//!      and run in the game's own prefix (where the registry points it at the
//!      game).
//!
//!    `auto` (the default) looks at the files to choose.

use super::sources::{self, How, Source, Target};
use super::{fetch_file, FileJob, Transfer};
use crate::games::GameId;
use crate::workshop::safe_rel_path;
use crate::Beacon;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModRef {
    /// A mod source in `sources.json`, by its id or its catalog id.
    Source(String),
    /// A ModDB link (mod, game, downloads list or one download's page).
    Link(String),
}

/// What a lookup found.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct Found {
    pub game: GameId,
    /// Name for the mod (the catalog's, the source's title, or the download page's).
    pub name: String,
    pub file: String,
    pub size: Option<u64>,
    pub url: String,
    pub md5: Option<String>,
    pub install: String,
    pub catalog_id: Option<String>,
    /// Set when the file was found somewhere new; `confirm` says it's only a guess.
    pub note: Option<String>,
}

/// Where a mod went.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Installed {
    /// In Beacon's mods library (select it in Mods).
    Library { game: GameId, dir: PathBuf, mod_id: Option<String> },
    /// Merged into the game's folder.
    GameFolder { game: GameId, dir: PathBuf, files: usize, backup: PathBuf },
    /// A setup program to run in the game's prefix.
    Installer { game: GameId, exe: PathBuf },
}

/// Kinds of file, by their first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniff {
    Archive,
    Exe,
    Big,
    Other,
}

pub fn sniff(path: &Path) -> Sniff {
    let mut head = [0u8; 8];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    let h = &head[..n];
    if h.starts_with(b"PK\x03\x04") || h.starts_with(b"7z\xBC\xAF\x27\x1C") || h.starts_with(b"Rar!") {
        Sniff::Archive
    } else if h.starts_with(b"MZ") {
        Sniff::Exe
    } else if h.starts_with(b"BIGF") || h.starts_with(b"BIG4") {
        Sniff::Big
    } else {
        Sniff::Other
    }
}

/// A safe one-segment name.
fn safe_name(s: &str, fallback: &str) -> String {
    let clean: String = s.chars().map(|c| if c.is_alphanumeric() || " -_.()+,'&!".contains(c) { c } else { '_' }).take(80).collect();
    let clean = clean.trim().trim_matches('.').to_string();
    match safe_rel_path(&clean) {
        Some(p) if p.components().count() == 1 && !clean.is_empty() => clean,
        _ => fallback.to_string(),
    }
}

fn files(dir: &Path, depth: usize, ext: &str) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(depth)
        .follow_links(false)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file() && e.path().extension().map(|x| x.eq_ignore_ascii_case(ext)).unwrap_or(false))
        .map(|e| e.into_path())
        .collect()
}

fn is_setup(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("setup") || n.contains("install") || n.ends_with(".msi")
}

/// Does the game folder already have a file with this name (case-insensitive)?
fn replaces(game_dir: Option<&Path>, name: &str) -> bool {
    game_dir
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)))
        .unwrap_or(false)
}

/// Choose how an extracted mod goes in (for `auto`).
pub fn choose(root: &Path, game_dir: Option<&Path>) -> &'static str {
    if !files(root, 4, "big").is_empty() {
        return "library";
    }
    let exes = files(root, 3, "exe");
    if exes.iter().any(|e| is_setup(&e.file_name().unwrap_or_default().to_string_lossy())) {
        return "installer";
    }
    // Files that match the game's own (a fixed exe or dll): they replace them.
    let top: Vec<String> = std::fs::read_dir(root).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    if top.iter().any(|n| replaces(game_dir, n)) {
        return "game-folder";
    }
    if !exes.is_empty() {
        return "installer";
    }
    "game-folder"
}

impl Beacon {
    /// Mod sources in `sources.json` for a game.
    pub fn mod_sources(&self, g: GameId) -> Vec<Source> {
        self.download_store().list().mods_for(g).into_iter().cloned().collect()
    }

    /// Find the file a mod source or ModDB link leads to now. Nothing is downloaded.
    pub fn mod_lookup(&self, t: &dyn Transfer, g: GameId, what: &ModRef) -> Result<Found> {
        let store = self.download_store();
        let list = store.list();
        match what {
            ModRef::Source(id) => {
                let src = list
                    .mods_for(g)
                    .into_iter()
                    .find(|s| s.id.eq_ignore_ascii_case(id) || s.catalog_id.as_deref().map(|c| c.eq_ignore_ascii_case(id)).unwrap_or(false))
                    .ok_or_else(|| anyhow!("no download for {id} ({})", g.short()))?;
                let mut learned = store.learned();
                let r = sources::resolve(t, &list, &mut learned, src)?;
                let note = match &r.how {
                    How::Rediscovered { note, confirm } => Some(if *confirm { format!("{note} (only a guess)") } else { note.clone() }),
                    How::Listed => None,
                };
                let Target::File { url, name, size, md5 } = r.target else { bail!("{} isn't a single file", src.title) };
                Ok(Found {
                    game: g,
                    name: src.title.clone(),
                    file: name,
                    size,
                    url,
                    md5,
                    install: src.install.clone().unwrap_or_else(|| "auto".into()),
                    catalog_id: src.catalog_id.clone(),
                    note,
                })
            }
            ModRef::Link(link) => {
                let (url, info) = super::moddb::resolve(t, link, None, &[])?;
                let file = info.file_name.clone().unwrap_or_else(|| "download".into());
                let name = info
                    .title
                    .clone()
                    .map(|t| t.trim_end_matches(" file").to_string())
                    .unwrap_or_else(|| Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Mod".into()));
                Ok(Found { game: g, name, file, size: info.size, url, md5: info.md5, install: "auto".into(), catalog_id: None, note: None })
            }
        }
    }

    /// Download what `mod_lookup` found and put it where it belongs for its game.
    /// Doesn't change the config (an installer still has to run); safe on a clone.
    pub fn mod_install(&self, t: &dyn Transfer, f: &Found, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<Installed> {
        let g = f.game;
        let store = self.download_store();
        let dl_dir = store.dir.join("files").join("mods").join(g.slug());
        let file = dl_dir.join(safe_name(&f.file, "mod.download"));
        let label = format!("Downloading {}", f.file);
        let size = f.size;
        fetch_file(t, &FileJob { url: f.url.clone(), dest: file.clone(), size: f.size, md5: f.md5.clone() }, false, &mut |b| {
            let pct = size.filter(|s| *s > 0).map(|s| (b.saturating_mul(90) / s).min(89) as u8);
            progress(pct, &label);
        })?;
        let kind = sniff(&file);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let staging = self.paths.data_dir.join("tmp").join(format!("mod-{}-{stamp}", g.slug()));
        let _ = std::fs::remove_dir_all(&staging);
        let root = match kind {
            Sniff::Archive => {
                progress(Some(90), &format!("Extracting {}", f.file));
                crate::media::extract_with(&file, &staging, "no tool to extract it: install libarchive (bsdtar) or 7-Zip (7z), or unrar for .rar files")?;
                crate::editions::content_root(&staging)
            }
            Sniff::Exe | Sniff::Big => {
                std::fs::create_dir_all(&staging)?;
                let ext = if kind == Sniff::Exe { "exe" } else { "big" };
                let mut name = safe_name(&f.file, &format!("mod.{ext}"));
                if !name.to_ascii_lowercase().ends_with(&format!(".{ext}")) {
                    name.push_str(&format!(".{ext}"));
                }
                std::fs::copy(&file, staging.join(&name))?;
                staging.clone()
            }
            Sniff::Other => bail!("{} isn't an archive, a program or a .big file; Beacon doesn't know what to do with it", f.file),
        };
        let game_dir = self.game_dir(g);
        let how = match f.install.as_str() {
            "library" | "game-folder" | "installer" => f.install.as_str(),
            _ => choose(&root, game_dir.as_deref()),
        };
        let result = (|| -> Result<Installed> {
            match how {
                "library" => {
                    progress(Some(96), "Adding it to Beacon's mods");
                    let lib = self.paths.mods_dir().join(g.slug());
                    std::fs::create_dir_all(&lib)?;
                    let target = lib.join(safe_name(&f.name, "Mod"));
                    if target.exists() {
                        std::fs::remove_dir_all(&target).with_context(|| format!("replacing {}", target.display()))?;
                    }
                    if std::fs::rename(&root, &target).is_err() {
                        crate::editions::merge_dir(&root, &target, &self.paths.data_dir.join("tmp").join("unused-backup"))?;
                    }
                    if !target.join("beacon-mod.json").exists() {
                        let manifest = serde_json::json!({ "name": f.name });
                        std::fs::write(target.join("beacon-mod.json"), serde_json::to_string_pretty(&manifest)?)?;
                    }
                    let mod_id = crate::mods::inspect(g, &target, None).map(|m| m.id);
                    Ok(Installed::Library { game: g, dir: target, mod_id })
                }
                "game-folder" => {
                    let dir = game_dir.clone().ok_or_else(|| anyhow!("{} isn't installed yet; install or locate it first, then download this again (it's kept, so nothing downloads twice)", g.short()))?;
                    progress(Some(96), &format!("Copying it into {}'s folder (replaced files are backed up)", g.short()));
                    let backup = self.paths.data_dir.join("backups").join(g.slug()).join((stamp / 1000).to_string());
                    let n = crate::editions::merge_dir(&crate::editions::merge_source(&root, &dir), &dir, &backup)?;
                    Ok(Installed::GameFolder { game: g, dir, files: n, backup })
                }
                _ => {
                    let exe = files(&root, 3, "exe")
                        .into_iter()
                        .min_by_key(|e| (!is_setup(&e.file_name().unwrap_or_default().to_string_lossy()), e.components().count()))
                        .ok_or_else(|| anyhow!("{} has no setup program to run", f.file))?;
                    let rel = exe.strip_prefix(&root).map(Path::to_path_buf).map_err(|_| anyhow!("internal: setup outside its folder"))?;
                    let tools = self.paths.data_dir.join("tools").join(format!("{}-mods", g.slug())).join(safe_name(&f.name, "mod"));
                    if tools.exists() {
                        std::fs::remove_dir_all(&tools)?;
                    }
                    std::fs::create_dir_all(tools.parent().unwrap())?;
                    if std::fs::rename(&root, &tools).is_err() {
                        crate::editions::merge_dir(&root, &tools, &self.paths.data_dir.join("tmp").join("unused-backup"))?;
                    }
                    Ok(Installed::Installer { game: g, exe: tools.join(rel) })
                }
            }
        })();
        let _ = std::fs::remove_dir_all(&staging);
        let installed = result?;
        // The download stays for an installer (it may be run again) and is removed once a mod is in place.
        if !matches!(installed, Installed::Installer { .. }) {
            let _ = std::fs::remove_file(&file);
        }
        progress(Some(100), "Done");
        Ok(installed)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testnet::{md5_hex, FakeNet};
    use super::*;
    use crate::config::Paths;

    #[test]
    fn sniffing_and_choosing() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("x");
        for (bytes, want) in [(&b"PK\x03\x04rest"[..], Sniff::Archive), (b"Rar!\x1a\x07", Sniff::Archive), (b"MZ\x90\x00", Sniff::Exe), (b"BIGF....", Sniff::Big), (b"<html>", Sniff::Other)] {
            std::fs::write(&p, bytes).unwrap();
            assert_eq!(sniff(&p), want);
        }
        let game = t.path().join("Exigo Files");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("Exigo.exe"), b"MZ").unwrap();
        let m = |files: &[&str]| {
            let d = tempfile::tempdir().unwrap();
            for f in files {
                let p = d.path().join(f);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, b"x").unwrap();
            }
            choose(d.path(), Some(&game))
        };
        assert_eq!(m(&["Edain/__edain.big", "readme.txt"]), "library");
        assert_eq!(m(&["Setup Patch 2.02.exe"]), "installer");
        assert_eq!(m(&["exigo.EXE"]), "game-folder");
        assert_eq!(m(&["SomeTool.exe"]), "installer");
        assert_eq!(m(&["Data/levels.pak"]), "game-folder");
    }

    fn beacon(root: &Path) -> Beacon {
        Beacon::load_with(Paths::under(root, root), "").unwrap()
    }

    #[test]
    fn a_big_goes_to_the_library_and_an_exe_fix_into_the_game() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        let net = FakeNet::default();
        // A single .big from a link.
        let big = b"BIGF fake archive";
        net.put("https://cdn.example/m.big", big);
        let f = Found {
            game: GameId::Rotwk,
            name: "Tiny Mod".into(),
            file: "tiny.big".into(),
            size: Some(big.len() as u64),
            url: "https://cdn.example/m.big".into(),
            md5: Some(md5_hex(big)),
            install: "auto".into(),
            catalog_id: None,
            note: None,
        };
        let r = b.mod_install(&net, &f, &mut |_, _| {}).unwrap();
        let Installed::Library { dir, .. } = r else { panic!("{r:?}") };
        assert!(dir.join("tiny.big").is_file() && dir.join("beacon-mod.json").is_file());
        assert!(dir.starts_with(b.paths.mods_dir().join("rotwk")));
        // A fixed exe for an installed game: merged into its folder, the original backed up.
        let game = t.path().join("games/Exigo");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("Exigo.exe"), b"MZ old").unwrap();
        b.set_path(GameId::Exigo, &game).unwrap();
        let exe = b"MZ fixed";
        net.put("https://cdn.example/fix.exe", exe);
        let f = Found { game: GameId::Exigo, name: "Intel fix".into(), file: "Exigo.exe".into(), size: Some(exe.len() as u64), url: "https://cdn.example/fix.exe".into(), md5: Some(md5_hex(exe)), install: "auto".into(), catalog_id: None, note: None };
        let r = b.mod_install(&net, &f, &mut |_, _| {}).unwrap();
        let Installed::GameFolder { files, backup, .. } = r else { panic!("{r:?}") };
        assert_eq!(files, 1);
        assert_eq!(std::fs::read(game.join("Exigo.exe")).unwrap(), exe);
        assert_eq!(std::fs::read(backup.join("Exigo.exe")).unwrap(), b"MZ old");
        // Game-folder mods need the game installed first.
        let f = Found { game: GameId::Conquest, install: "game-folder".into(), url: "https://cdn.example/fix.exe".into(), ..f };
        assert!(b.mod_install(&net, &f, &mut |_, _| {}).unwrap_err().to_string().contains("isn't installed yet"));
    }

    #[test]
    fn sources_for_the_catalog() {
        let t = tempfile::tempdir().unwrap();
        let b = beacon(t.path());
        let ids: Vec<Option<String>> = b.mod_sources(GameId::Rotwk).into_iter().map(|s| s.catalog_id).collect();
        assert!(ids.contains(&Some("edain".into())));
        assert!(b.mod_sources(GameId::Swgb).is_empty());
    }
}
