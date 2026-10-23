//! BFME, BFME II and RotWK straight from the BFME
//! Foundation's CDN: the same base packages (`original-BFME1`,
//! `original-BFME2`, `original-RotWK`) the All-in-One launcher installs,
//! fetched natively instead of through the launcher.
//!
//! What the launcher does (`BfmeSyncManager.InstallGame` →
//! `BfmeWorkshopSyncManager.Sync`), and Beacon the same way:
//! * every file of the package into `<folder>\<BFME1|BFME2|RotWK>`, skipping
//!   its ignored files (logo movies, `_zzlotr.big`, `mod.txt`…), `.wps`
//!   scripts and files for other languages; RotWK gets BFME II too;
//! * a made-up 20-character CD key in `ergc` (the launcher does exactly this);
//! * the EA registry keys, which Beacon writes into the game's prefix on
//!   Linux (its usual setup) or imports with `reg import` on Windows.
//!
//! Downloads resume (`.part` files), each file is checked against the
//! package's size and MD5, and files already there and matching are kept, so
//! running it again is a repair (`thorough` re-checks every MD5).

use super::sources::{self, How, Learned, Store, Target};
use super::{fetch_file, FileJob, Fetched, Transfer};
use crate::aio::{field, field_str};
use crate::games::GameId;
use crate::workshop::{language_code, safe_rel_path};
use crate::Beacon;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Files the All-in-One launcher never writes into a game folder (its `IgnoredGameFiles`).
pub const IGNORED: &[&str] = &[
    "_zzlotr.big", "cs01.vp6", "nlc_logo.vp6", "te_logo.vp6", "newlinelogo.vp6", "ealogo.vp6", "242.vp6", "intel.vp6", "thx.vp6", "mod.txt",
    "timer_frame.png",
];
/// Largest base game Beacon downloads.
pub const MAX_GAME_BYTES: u64 = 24 << 30;
/// Most files in one base game.
pub const MAX_FILES: usize = 100_000;
/// What Beacon leaves in a game folder it installed.
pub const MARKER: &str = "beacon-install.json";

/// The games that can be installed this way.
pub fn supported(g: GameId) -> bool {
    matches!(g, GameId::Bfme1 | GameId::Bfme2 | GameId::Rotwk)
}

/// The folder name, as the All-in-One launcher names it.
pub fn folder_name(g: GameId) -> &'static str {
    match g {
        GameId::Bfme1 => "BFME1",
        GameId::Bfme2 => "BFME2",
        _ => "RotWK",
    }
}

/// A package turned into files to fetch.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub game: GameId,
    pub guid: String,
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    pub files: Vec<FileJob>,
    /// Left out (ignored files, other languages, duplicates).
    pub skipped: Vec<String>,
    pub bytes: u64,
}

/// Turn the workshop's answer into a plan for `dir`. `lang` is the language
/// code (`EN`, `DE`…).
pub fn plan(entry: &Value, g: GameId, dir: &Path, lang: &str) -> Result<Plan> {
    let guid = field_str(entry, "Guid").ok_or_else(|| anyhow!("the workshop's answer has no Guid"))?;
    let game = field(entry, "Game").and_then(Value::as_i64);
    if game != crate::aio::game_index(g) {
        bail!("{guid} is a package for another game");
    }
    if field(entry, "Type").and_then(Value::as_i64) != Some(0) {
        bail!("{guid} isn't a base-game package");
    }
    let raw: &[Value] = field(entry, "Files").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    if raw.is_empty() || raw.len() > MAX_FILES {
        bail!("{guid} lists {} files", raw.len());
    }
    let mut files: Vec<FileJob> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut skipped = Vec::new();
    let mut bytes: u64 = 0;
    for f in raw {
        let name = field_str(f, "Name").unwrap_or_default();
        let base = name.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
        let file_lang = field_str(f, "Language").unwrap_or_default();
        let lang_ok = file_lang.is_empty() || file_lang.eq_ignore_ascii_case("ALL") || file_lang.split(' ').any(|l| l.eq_ignore_ascii_case(lang));
        if IGNORED.contains(&base.as_str()) || base.ends_with(".wps") || !lang_ok {
            skipped.push(name);
            continue;
        }
        let rel = safe_rel_path(&name).ok_or_else(|| anyhow!("unsafe file path in {guid}: {name:?}"))?;
        let key = rel.to_string_lossy().to_ascii_lowercase();
        if seen.contains(&key) {
            skipped.push(name);
            continue;
        }
        seen.push(key);
        let url = field_str(f, "Url").filter(|u| super::is_https(u)).ok_or_else(|| anyhow!("{name}: not an https download"))?;
        // 0 means the source doesn't know.
        let size = field(f, "Size").and_then(Value::as_u64).filter(|s| *s > 0);
        let md5 = field_str(f, "Md5").map(|m| m.trim().to_ascii_lowercase()).filter(|m| m.len() == 32 && m.chars().all(|c| c.is_ascii_hexdigit()));
        bytes = bytes.saturating_add(size.unwrap_or(0));
        files.push(FileJob { url, dest: dir.join(rel), size, md5 });
    }
    if bytes > MAX_GAME_BYTES {
        bail!("{guid} is {bytes} bytes, more than Beacon downloads for a game");
    }
    if !files.iter().any(|f| f.dest.file_name().map(|n| g.exes().iter().any(|e| n.to_string_lossy().eq_ignore_ascii_case(e))).unwrap_or(false)) {
        bail!("{guid} doesn't contain {}", g.exe());
    }
    Ok(Plan {
        game: g,
        name: field_str(entry, "Name").unwrap_or_else(|| guid.clone()),
        version: field_str(entry, "Version").unwrap_or_default(),
        guid,
        dir: dir.to_path_buf(),
        files,
        skipped,
        bytes,
    })
}

/// Download everything in the plan; `progress(pct, text)`.
pub fn fetch(t: &dyn Transfer, plan: &Plan, thorough: bool, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<(u64, usize)> {
    std::fs::create_dir_all(&plan.dir).with_context(|| format!("creating {}", plan.dir.display()))?;
    let total = plan.bytes.max(1);
    let mut done: u64 = 0;
    let mut downloaded: u64 = 0;
    let mut kept = 0usize;
    // Small files first, like the launcher: the game is recognisable early.
    let mut order: Vec<&FileJob> = plan.files.iter().collect();
    order.sort_by_key(|f| f.size.unwrap_or(0));
    let n = order.len();
    for (i, f) in order.into_iter().enumerate() {
        let label = f.dest.strip_prefix(&plan.dir).unwrap_or(&f.dest).to_string_lossy().into_owned();
        let start = done;
        let pct = |b: u64| Some((start + b).saturating_mul(100) / total).map(|p| p.min(99) as u8);
        progress(pct(0), &format!("{} ({}/{n}): {label}", plan.game.short(), i + 1));
        let r = fetch_file(t, f, thorough, &mut |b| progress(pct(b), &format!("{} ({}/{n}): {label}", plan.game.short(), i + 1)))
            .with_context(|| format!("{}: {label}", plan.game.short()))?;
        match r {
            Fetched::Already => kept += 1,
            Fetched::Downloaded { bytes, .. } => downloaded += bytes,
        }
        done += f.size.unwrap_or(0);
    }
    Ok((downloaded, kept))
}

/// `beacon-install.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub guid: String,
    pub name: String,
    pub version: String,
    pub files: usize,
    pub bytes: u64,
    /// Seconds since 1970.
    pub installed_at: u64,
}

pub fn read_marker(dir: &Path) -> Option<Marker> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(MARKER)).ok()?).ok()
}

/// One game done.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Installed {
    pub game: GameId,
    pub dir: PathBuf,
    pub name: String,
    pub version: String,
    pub files: usize,
    /// Files that were already there and fine.
    pub kept: usize,
    pub downloaded_bytes: u64,
    /// Set when the source was found somewhere new.
    pub note: Option<String>,
}

/// The result of asking for a game.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum Outcome {
    Done { games: Vec<Installed> },
    /// The source only turned up a guess; run again with `accept_guess` to use it.
    NeedsConfirm { game: GameId, note: String },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Check every file's MD5, not just its size (a repair).
    pub thorough: bool,
    /// Use a source Beacon only guessed at.
    pub accept_guess: bool,
    /// Download the game again even when it's installed (repair/update).
    pub reinstall: bool,
}

/// A 20-character key like the one the All-in-One launcher makes up.
pub fn made_up_key() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut out = String::new();
    let mut i = 0u64;
    while out.len() < 20 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let mut v = h.finish();
        for _ in 0..8 {
            if out.len() == 20 {
                break;
            }
            out.push(CHARS[(v % CHARS.len() as u64) as usize] as char);
            v /= CHARS.len() as u64;
        }
        i += 1;
    }
    out
}

/// A Windows .reg for the games, with native paths: the keys the games read
/// (both the `EA Games` and `Electronic Arts\Electronic Arts` spellings the
/// installers and the All-in-One launcher use), in the 32-bit view.
pub fn windows_reg(games: &[(GameId, PathBuf, String)], language: &str) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }
    let mut out = String::from("Windows Registry Editor Version 5.00\r\n");
    for (g, dir, key) in games {
        let mut path = dir.to_string_lossy().replace('/', "\\");
        if !path.ends_with('\\') {
            path.push('\\');
        }
        let parents: &[&str] = match g {
            GameId::Bfme1 => &["EA Games"],
            _ => &["EA Games", "Electronic Arts"],
        };
        for view in ["SOFTWARE\\WOW6432Node", "SOFTWARE"] {
            for parent in parents {
                let k = format!("HKEY_LOCAL_MACHINE\\{view}\\Electronic Arts\\{parent}\\{}", g.registry_name());
                out.push_str(&format!("\r\n[{k}]\r\n"));
                out.push_str(&format!("\"InstallPath\"=\"{}\"\r\n", esc(&path)));
                out.push_str(&format!("\"Language\"=\"{}\"\r\n", esc(language)));
                out.push_str(&format!("\"UserDataLeafName\"=\"{}\"\r\n", esc(g.user_data_leaf())));
                out.push_str("\"MapPackVersion\"=dword:00010000\r\n");
                out.push_str("\"UseLocalUserMaps\"=dword:00000000\r\n");
                out.push_str("\"Version\"=dword:00010003\r\n");
                out.push_str(&format!("\r\n[{k}\\ergc]\r\n@=\"{}\"\r\n", esc(key)));
            }
        }
    }
    out
}

// ---------------------------------------------------------------- Beacon

impl Beacon {
    /// The download engine's files.
    pub fn download_store(&self) -> Store {
        Store::new(&self.paths.config_file, &self.paths.data_dir)
    }

    /// Where downloaded games go.
    pub fn games_root(&self) -> PathBuf {
        self.download_store().settings().games_dir.unwrap_or_else(|| self.paths.data_dir.join("games"))
    }

    /// The games a download of `g` covers: BFME II first for RotWK when it isn't
    /// installed, then `g` unless it's installed already (`reinstall`: always).
    pub fn cdn_plan_games(&self, g: GameId, reinstall: bool) -> Vec<GameId> {
        let mut out = Vec::new();
        if let Some(req) = g.requires() {
            if self.game_dir(req).is_none() {
                out.push(req);
            }
        }
        if reinstall || self.game_dir(g).is_none() {
            out.push(g);
        }
        out
    }

    /// Download `g` (and what it needs) from the CDN. Doesn't change the
    /// config: hand the result to [`Beacon::cdn_adopt`]. Safe to run on a clone.
    pub fn cdn_download(&self, t: &dyn Transfer, g: GameId, opts: Options, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<Outcome> {
        if !supported(g) {
            bail!("{} isn't on the BFME Foundation's CDN (only BFME, BFME II and RotWK)", g.short());
        }
        let store = self.download_store();
        let list = store.list();
        let mut learned: Learned = store.learned();
        let before = learned.clone();
        let lang = language_code(&self.config.language);
        let root = self.games_root();
        let mut done = Vec::new();
        for game in self.cdn_plan_games(g, opts.reinstall) {
            let src = list.for_game(game).ok_or_else(|| anyhow!("sources.json has no source for {}", game.short()))?;
            progress(Some(0), &format!("Looking up {}", game.short()));
            let resolved = sources::resolve(t, &list, &mut learned, src);
            if learned != before {
                let _ = store.save_learned(&learned);
            }
            let resolved = resolved?;
            if resolved.how.needs_confirm() && !opts.accept_guess {
                let note = match &resolved.how {
                    How::Rediscovered { note, .. } => note.clone(),
                    How::Listed => String::new(),
                };
                return Ok(Outcome::NeedsConfirm { game, note });
            }
            let Target::Workshop { entry, .. } = &resolved.target else {
                bail!("the source for {} isn't a workshop package", game.short());
            };
            // An existing install of this game in Beacon's games folder is repaired in place.
            let dir = match self.game_dir(game) {
                Some(d) if opts.reinstall && read_marker(&d).is_some() => d,
                _ => root.join(folder_name(game)),
            };
            let p = plan(entry, game, &dir, &lang)?;
            let (downloaded_bytes, kept) = fetch(t, &p, opts.thorough, progress)?;
            let marker = Marker {
                guid: p.guid.clone(),
                name: p.name.clone(),
                version: p.version.clone(),
                files: p.files.len(),
                bytes: p.bytes,
                installed_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            };
            std::fs::write(dir.join(MARKER), serde_json::to_string_pretty(&marker)?)?;
            let note = match resolved.how {
                How::Rediscovered { note, .. } => Some(note),
                How::Listed => None,
            };
            done.push(Installed { game, dir, name: p.name, version: p.version, files: p.files.len(), kept, downloaded_bytes, note });
        }
        progress(Some(100), "Downloaded");
        Ok(Outcome::Done { games: done })
    }

    /// Point Beacon at freshly downloaded games and give each a CD key when it
    /// has none (made up, like the All-in-One launcher does). Save the config afterwards.
    pub fn cdn_adopt(&mut self, games: &[Installed]) -> Result<()> {
        for i in games {
            self.set_path(i.game, &i.dir)?;
            let cfg = self.game_mut(i.game);
            if cfg.cd_key.as_deref().map(str::trim).unwrap_or("").is_empty() {
                cfg.cd_key = Some(made_up_key());
            }
        }
        Ok(())
    }

    /// After adopting: write the registry. Linux: Beacon's usual prefix setup.
    /// Windows: `reg import` with the UAC prompt.
    pub fn cdn_register(&self, games: &[Installed]) -> Result<()> {
        if !self.native {
            for i in games {
                self.setup(i.game, true).with_context(|| format!("setting up {}'s prefix", i.game.short()))?;
            }
            return Ok(());
        }
        let entries: Vec<(GameId, PathBuf, String)> =
            games.iter().map(|i| (i.game, i.dir.clone(), self.game(i.game).cd_key.unwrap_or_else(made_up_key))).collect();
        let reg = windows_reg(&entries, &self.config.language);
        let file = self.paths.data_dir.join("registry").join("beacon-downloads.reg");
        std::fs::create_dir_all(file.parent().unwrap())?;
        // regedit/reg read .reg files as UTF-16 LE with a BOM.
        let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
        for u in reg.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(&file, bytes)?;
        register_windows(&self.paths.data_dir, &file)
    }
}

#[cfg(windows)]
fn register_windows(data_dir: &Path, file: &Path) -> Result<()> {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let cmd = crate::runner::Command {
        program: root.join("System32").join("reg.exe"),
        args: vec!["import".into(), file.to_string_lossy().into_owned()],
        env: vec![],
        cwd: file.parent().map(Path::to_path_buf).unwrap_or_else(|| data_dir.to_path_buf()),
    };
    let status = crate::native::spawn_elevated(data_dir, &cmd)?.wait()?;
    match status.code() {
        Some(0) => Ok(()),
        Some(1223) => bail!("the administrator prompt was declined; the games need their registry keys to start"),
        _ => bail!("importing the registry keys failed ({status})"),
    }
}

#[cfg(not(windows))]
fn register_windows(_data_dir: &Path, _file: &Path) -> Result<()> {
    bail!("not on Windows")
}

#[cfg(test)]
mod tests {
    use super::super::testnet::{md5_hex, FakeNet};
    use super::*;
    use crate::config::Paths;
    use serde_json::json;

    fn package(guid: &str, game: i64, files: &[(&str, &str, &[u8])]) -> Value {
        json!({
            "Guid": guid, "Name": "Vanilla", "Version": "1.0.0", "Game": game, "Type": 0,
            "Files": files.iter().map(|(name, lang, body)| json!({
                "Name": name, "Language": lang, "Size": body.len(), "Md5": md5_hex(body),
                "Url": format!("https://files.example/{}", md5_hex(body))
            })).collect::<Vec<_>>()
        })
    }

    #[test]
    fn plan_filters_like_the_launcher() {
        let p = package(
            "original-BFME1",
            0,
            &[
                ("lotrbfme.exe", "ALL", b"exe"),
                ("data\\movies\\EALogo.vp6", "ALL", b"logo"),
                ("Germansplash.jpg", "DE", b"de"),
                ("Englishsplash.jpg", "EN", b"en"),
                ("lang\\English.big", "EN PL", b"big"),
                ("scripts\\x.wps", "ALL", b"s"),
                ("LOTRBFME.EXE", "ALL", b"dup"),
            ],
        );
        let pl = plan(&p, GameId::Bfme1, Path::new("/g/BFME1"), "EN").unwrap();
        let names: Vec<String> = pl.files.iter().map(|f| f.dest.strip_prefix("/g/BFME1").unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names, vec!["lotrbfme.exe", "Englishsplash.jpg", "lang/English.big"]);
        assert_eq!(pl.skipped.len(), 4);
        // Wrong game, unsafe path, no exe.
        assert!(plan(&p, GameId::Bfme2, Path::new("/g"), "EN").is_err());
        assert!(plan(&package("x", 0, &[("..\\evil.dll", "ALL", b"e"), ("lotrbfme.exe", "ALL", b"x")]), GameId::Bfme1, Path::new("/g"), "EN").is_err());
        assert!(plan(&package("x", 0, &[("a.big", "ALL", b"e")]), GameId::Bfme1, Path::new("/g"), "EN").is_err());
    }

    fn beacon(root: &Path) -> Beacon {
        let paths = Paths::under(root, root);
        Beacon::load_with(paths, "").unwrap()
    }

    fn serve(net: &FakeNet, guid: &str, pkg: &Value, bodies: &[&[u8]]) {
        net.put(&format!("https://bfmeladder.com/api/workshop/download?guid={guid}"), pkg.to_string().as_bytes());
        for b in bodies {
            net.put(&format!("https://files.example/{}", md5_hex(b)), b);
        }
    }

    #[test]
    fn rotwk_brings_bfme2_and_gets_adopted() {
        let t = tempfile::tempdir().unwrap();
        let mut b = beacon(t.path());
        let net = FakeNet::default();
        let b2: &[&[u8]] = &[b"bfme2 exe", b"bfme2 data"];
        let rk: &[&[u8]] = &[b"rotwk exe"];
        serve(&net, "original-BFME2", &package("original-BFME2", 1, &[("lotrbfme2.exe", "ALL", b2[0]), ("INI.big", "ALL", b2[1])]), b2);
        serve(&net, "original-RotWK", &package("original-RotWK", 2, &[("lotrbfme2ep1.exe", "ALL", rk[0])]), rk);
        let out = b.cdn_download(&net, GameId::Rotwk, Options::default(), &mut |_, _| {}).unwrap();
        let Outcome::Done { games } = out else { panic!("{out:?}") };
        assert_eq!(games.iter().map(|g| g.game).collect::<Vec<_>>(), vec![GameId::Bfme2, GameId::Rotwk]);
        let root = b.games_root();
        assert_eq!(std::fs::read(root.join("BFME2/INI.big")).unwrap(), b"bfme2 data");
        assert!(read_marker(&root.join("RotWK")).is_some());
        b.cdn_adopt(&games).unwrap();
        assert_eq!(b.game_dir(GameId::Rotwk), Some(root.join("RotWK")));
        let key = b.game(GameId::Bfme2).cd_key.unwrap();
        assert_eq!(key.len(), 20);
        assert!(key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        // Installed now: asking again downloads nothing.
        assert_eq!(b.cdn_plan_games(GameId::Rotwk, false), Vec::<GameId>::new());
        assert_eq!(b.cdn_plan_games(GameId::Rotwk, true), vec![GameId::Rotwk]);
        let before = net.requests().len();
        let again = b.cdn_download(&net, GameId::Rotwk, Options { reinstall: true, ..Options::default() }, &mut |_, _| {}).unwrap();
        let Outcome::Done { games } = again else { panic!() };
        assert_eq!(games[0].kept, 1);
        // Only the API was asked; no file was downloaded again.
        assert_eq!(net.requests().len(), before + 1);
    }

    #[test]
    fn a_guess_waits_for_a_yes() {
        let t = tempfile::tempdir().unwrap();
        let b = beacon(t.path());
        let net = FakeNet::default();
        net.put("https://bfmeladder.com/api/workshop/download?guid=original-BFME1", br#"{"Guid": null}"#);
        let q = json!([{"Guid": "original-BFME1-2027", "Game": 0, "Type": 0}]).to_string();
        net.put("https://bfmeladder.com/api/workshop/query?keyword=~&game=0&type=0&sortMode=0&page=0", q.as_bytes());
        let body: &[&[u8]] = &[b"exe"];
        serve(&net, "original-BFME1-2027", &package("original-BFME1-2027", 0, &[("lotrbfme.exe", "ALL", body[0])]), body);
        let out = b.cdn_download(&net, GameId::Bfme1, Options::default(), &mut |_, _| {}).unwrap();
        assert!(matches!(out, Outcome::NeedsConfirm { game: GameId::Bfme1, .. }));
        assert!(!b.games_root().join("BFME1").exists());
        let out = b.cdn_download(&net, GameId::Bfme1, Options { accept_guess: true, ..Options::default() }, &mut |_, _| {}).unwrap();
        assert!(matches!(out, Outcome::Done { .. }));
    }

    #[test]
    fn only_bfme_games() {
        let t = tempfile::tempdir().unwrap();
        let b = beacon(t.path());
        assert!(b.cdn_download(&FakeNet::default(), GameId::Conquest, Options::default(), &mut |_, _| {}).is_err());
    }

    #[test]
    fn windows_reg_has_both_spellings_and_the_key() {
        let reg = windows_reg(&[(GameId::Bfme2, PathBuf::from(r"C:\Games\BFME2"), "ABC".into())], "english");
        assert!(reg.contains("[HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Electronic Arts\\Electronic Arts\\The Battle for Middle-earth II]"));
        assert!(reg.contains("[HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Electronic Arts\\EA Games\\The Battle for Middle-earth II\\ergc]"));
        assert!(reg.contains("\"InstallPath\"=\"C:\\\\Games\\\\BFME2\\\\\""));
        assert!(reg.contains("@=\"ABC\""));
    }

    #[test]
    fn keys_differ() {
        assert_ne!(made_up_key(), made_up_key());
    }
}
