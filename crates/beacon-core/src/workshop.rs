//! The BFME Foundation workshop: fan-made mods, maps and patches for BFME,
//! BFME II and RotWK, through the public API the All-in-One launcher uses
//! (from its source, github.com/MarcellVokk/BfmeFoundationProject,
//! `BfmeFoundationProject_WorkshopKit`):
//!
//! * `GET https://bfmeladder.com/api/workshop/query?keyword=&game=&type=&sortMode=&page=`
//!   → a list of entry previews (`Guid, Name, Version, Description, Author,
//!   Game, Type, Size, ArtworkUrl, Metadata{Downloads}`…). Empty values are sent
//!   as `~`; the unauthenticated headers are `AuthAccountUuid: unauthenticated`
//!   and an empty `AuthAccountPassword`.
//! * `GET https://bfmeladder.com/api/workshop/download?guid=` → the full entry,
//!   with `Files: [{Guid, Name, Url, Md5, Language, Size}]`. `Name` is the path
//!   inside the package, `Md5` lowercase hex.
//!
//! `Game` is 0 (BFME), 1 (BFME II) or 2 (RotWK). `Type` is 0 patch, 1 mod,
//! 2 enhancement, 3 map pack, 4 snapshot.
//!
//! **Fan content only.** Beacon never downloads the base games or EA's own
//! files: entries whose guid marks them as a base game (`original-…`,
//! `exp-original-…`), an official EA patch (`official-…`) or a snapshot of
//! someone's install (`snapshot-…`, type 4) are left out of the list and refused
//! by `install`; inside fan packages, EA's executables, `game.dat`, logo movies
//! and the base game's own archives are skipped, and a package that carries the
//! base game's archives is refused outright.
//!
//! Downloads: https only (redirects too), size limits per file and per
//! package, the MD5 checked when the workshop gives one, every path checked
//! (no `..`, no absolute paths, no drive letters) and written as new plain
//! files under a fresh folder, which then replaces `<mods>/<game>/<Name>` as a
//! whole. The package lands in Beacon's mods library with a `beacon-mod.json`,
//! so it shows up like any other mod and loads with `-mod`.

use crate::aio::{self, field, field_str};
use crate::games::GameId;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const API: &str = "https://bfmeladder.com/api";
/// Shown with the list, so people can see where the content comes from.
pub const SITE: &str = "https://bfmeladder.com";

/// Largest single file Beacon downloads.
pub const MAX_FILE_BYTES: u64 = 4 << 30;
/// Largest package (all files together).
pub const MAX_PACKAGE_BYTES: u64 = 8 << 30;
/// Most files in one package.
pub const MAX_FILES: usize = 50_000;
/// Largest API answer read.
const MAX_JSON_BYTES: u64 = 32 << 20;

/// What kind of entry, by the workshop's `Type`.
pub fn kind_name(t: i64) -> Option<&'static str> {
    match t {
        0 => Some("patch"),
        1 => Some("mod"),
        2 => Some("enhancement"),
        3 => Some("maps"),
        _ => None,
    }
}

/// Why an entry isn't fan content Beacon will touch, or `None` when it is.
pub fn refusal(guid: &str, kind: i64) -> Option<&'static str> {
    let g = guid.to_ascii_lowercase();
    if g.starts_with("original-") || g.starts_with("exp-original-") {
        Some("that's a base game package; Beacon never downloads the games themselves")
    } else if g.starts_with("official-") {
        Some("that's EA's official patch, not fan content")
    } else if g.starts_with("snapshot-") || kind == 4 {
        Some("that's a snapshot of someone's game folder, not fan content")
    } else if kind_name(kind).is_none() {
        Some("Beacon doesn't know that kind of workshop entry")
    } else {
        None
    }
}

/// The base game's own archives (all three games). A fan package shouldn't
/// carry these; one that carries several is a copy of the game, not a mod.
const BASE_ARCHIVES: &[&str] = &[
    "apt.big", "audio.big", "audioenglish.big", "aptenglish.big", "cursors.big", "english.big", "englishaudio.big",
    "englishspeech.big", "gensec.big", "ini.big", "maps.big", "music.big", "shaders.big", "speech.big",
    "speechenglish.big", "terrain.big", "textures.big", "w3d.big", "window.big", "lotrbfme.exe", "lotrbfme2.exe",
    "lotrbfme2ep1.exe", "game.dat",
];

/// Files never taken from a package: EA's executables and logo movies, and
/// the launcher's own bookkeeping (`mod.txt`, its key-binding archive). Mods
/// may ship their own archives under base-game names (`lang/English.big` with
/// their strings), so those are only counted by [`looks_like_base_game`].
pub fn skip_file(name: &str) -> bool {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_ascii_lowercase();
    matches!(
        base.as_str(),
        "game.dat" | "lotrbfme.exe" | "lotrbfme2.exe" | "lotrbfme2ep1.exe" | "_zzlotr.big" | "mod.txt" | "timer_frame.png"
            | "cs01.vp6" | "nlc_logo.vp6" | "te_logo.vp6" | "newlinelogo.vp6" | "ealogo.vp6" | "242.vp6" | "intel.vp6" | "thx.vp6"
    )
}

/// Does this file list look like the base game rather than fan content?
pub fn looks_like_base_game(names: &[&str]) -> bool {
    let hits = names
        .iter()
        .filter(|n| {
            let base = n.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
            BASE_ARCHIVES.contains(&base.as_str())
        })
        .count();
    hits >= 3
}

/// A path from a package as a safe relative path: `\` and `/` both separate,
/// `.` segments dropped; anything absolute, with a drive letter, `..`, a colon,
/// control characters or a Windows device name is refused.
pub fn safe_rel_path(name: &str) -> Option<PathBuf> {
    let n = name.replace('\\', "/");
    if n.starts_with('/') || n.len() > 400 {
        return None;
    }
    let mut out = PathBuf::new();
    for seg in n.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        let stem = seg.split('.').next().unwrap_or("").trim().to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
        if seg == ".." || seg.contains(':') || seg.chars().any(|c| c.is_control()) || device || seg.ends_with(' ') || seg.ends_with('.') {
            return None;
        }
        out.push(seg);
    }
    let ok = out.components().count() > 0 && out.components().all(|c| matches!(c, Component::Normal(_)));
    ok.then_some(out)
}

/// A folder name for a package: letters, digits, spaces, `-_.()'&!+,`; at most 60 characters.
pub fn folder_for(name: &str, guid: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_alphanumeric() || " -_()'&!+,".contains(c) { c } else { ' ' }).collect();
    s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.chars().take(60).collect();
    let s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() || safe_rel_path(&s).is_none() {
        let short: String = guid.chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect();
        format!("workshop-{short}")
    } else {
        s
    }
}

/// One entry in the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Item {
    /// The workshop's guid; `workshop-install <game> <id>` takes it.
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// `patch`, `mod`, `enhancement` or `maps`.
    pub kind: &'static str,
    pub game: GameId,
    /// Bytes, as the workshop reports it.
    pub size: u64,
    pub downloads: u64,
    pub artwork_url: Option<String>,
    /// Id of the Beacon mod it's installed as, if it is.
    pub installed: Option<String>,
}

fn num(v: &Value, name: &str) -> Option<i64> {
    field(v, name).and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok())))
}

/// Text from the workshop for display: control characters removed, length capped.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect::<String>().trim().to_string()
}

fn https(url: &str) -> Option<String> {
    let u = url.trim();
    let scheme_ok = u.len() > 8 && u.get(..8).map(|p| p.eq_ignore_ascii_case("https://")).unwrap_or(false);
    (scheme_ok && !u.chars().any(|c| c.is_whitespace() || c.is_control())).then(|| u.to_string())
}

/// One preview from the list, if it's fan content for a game Beacon knows.
pub fn parse_item(v: &Value) -> Option<Item> {
    let id = field_str(v, "Guid")?;
    let kind_code = num(v, "Type")?;
    if refusal(&id, kind_code).is_some() || id.trim().is_empty() || id.len() > 200 || id.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let game = aio::game_for_index(num(v, "Game")?)?;
    Some(Item {
        name: clean(&field_str(v, "Name").unwrap_or_default(), 200),
        version: clean(&field_str(v, "Version").unwrap_or_default(), 60),
        author: clean(&field_str(v, "Author").unwrap_or_default(), 100),
        description: clean(&field_str(v, "Description").unwrap_or_default(), 2000),
        kind: kind_name(kind_code)?,
        game,
        size: num(v, "Size").unwrap_or(0).max(0) as u64,
        downloads: field(v, "Metadata").and_then(|m| num(m, "Downloads")).unwrap_or(0).max(0) as u64,
        artwork_url: field_str(v, "ArtworkUrl").as_deref().and_then(https),
        installed: None,
        id,
    })
}

/// The list answer: fan items, and how many entries were left out.
pub fn parse_list(v: &Value, g: GameId) -> (Vec<Item>, usize) {
    let all: &[Value] = v.as_array().map(Vec::as_slice).unwrap_or(&[]);
    let items: Vec<Item> = all.iter().filter_map(parse_item).filter(|i| i.game == g).collect();
    let hidden = all.len() - items.len();
    (items, hidden)
}

/// A file to download.
#[derive(Debug, Clone, PartialEq)]
pub struct FileSpec {
    pub path: PathBuf,
    pub url: String,
    pub md5: Option<String>,
    pub size: u64,
}

/// A full entry, ready to install.
#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub item: Item,
    pub files: Vec<FileSpec>,
    /// Files left out (EA files, other languages, duplicates), by name.
    pub skipped: Vec<String>,
}

/// Turn `workshop/download` into a package, refusing what isn't fan content
/// or can't be downloaded safely. `language` is the game's language code
/// (`EN`, `DE`…); files for other languages are skipped.
pub fn parse_package(v: &Value, language: &str) -> Result<Package> {
    let guid = field_str(v, "Guid").ok_or_else(|| anyhow!("the workshop's answer has no Guid"))?;
    let kind = num(v, "Type").ok_or_else(|| anyhow!("the workshop's answer has no Type"))?;
    if let Some(why) = refusal(&guid, kind) {
        bail!("{why}");
    }
    let item = parse_item(v).ok_or_else(|| anyhow!("not a workshop entry Beacon can install"))?;
    let raw: &[Value] = field(v, "Files").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
    if raw.len() > MAX_FILES {
        bail!("the package has {} files, more than Beacon accepts ({MAX_FILES})", raw.len());
    }
    let names: Vec<String> = raw.iter().filter_map(|f| field_str(f, "Name")).collect();
    if looks_like_base_game(&names.iter().map(String::as_str).collect::<Vec<_>>()) {
        bail!("the package carries the base game's own files; Beacon only installs fan content");
    }
    let mut files: Vec<FileSpec> = Vec::new();
    let mut skipped = Vec::new();
    let mut total: u64 = 0;
    for f in raw {
        let name = field_str(f, "Name").unwrap_or_default();
        let lang = field_str(f, "Language").unwrap_or_default();
        let lang_ok = lang.is_empty() || lang.eq_ignore_ascii_case("ALL") || lang.split(' ').any(|l| l.eq_ignore_ascii_case(language));
        // .wps files are the launcher's scripts; it never writes them out either.
        if skip_file(&name) || !lang_ok || name.to_ascii_lowercase().ends_with(".wps") {
            skipped.push(name);
            continue;
        }
        let path = safe_rel_path(&name).ok_or_else(|| anyhow!("unsafe file path in the package: {name:?}"))?;
        let url = field_str(f, "Url").as_deref().and_then(https).ok_or_else(|| anyhow!("{name}: not an https download"))?;
        let size = num(f, "Size").unwrap_or(0).max(0) as u64;
        if size > MAX_FILE_BYTES {
            bail!("{name} is {size} bytes, more than Beacon downloads in one file");
        }
        let md5 = field_str(f, "Md5").map(|m| m.trim().to_ascii_lowercase()).filter(|m| m.len() == 32 && m.chars().all(|c| c.is_ascii_hexdigit()));
        let key = path.to_string_lossy().to_ascii_lowercase();
        if files.iter().any(|x| x.path.to_string_lossy().to_ascii_lowercase() == key) {
            skipped.push(name);
            continue;
        }
        total = total.saturating_add(size);
        files.push(FileSpec { path, url, md5, size });
    }
    if total > MAX_PACKAGE_BYTES {
        bail!("the package is {total} bytes, more than Beacon downloads ({MAX_PACKAGE_BYTES})");
    }
    if files.is_empty() {
        bail!("the package has no files Beacon can install");
    }
    Ok(Package { item, files, skipped })
}

// ---------------------------------------------------------------- HTTP

/// The network side, so installs can be tested without it.
pub trait Http {
    /// GET a JSON answer from the workshop API.
    fn get_json(&self, url: &str, query: &[(&str, String)]) -> Result<Value>;
    /// GET a file into `out`, at most `max` bytes, calling `progress(bytes so far)`.
    fn download(&self, url: &str, out: &mut dyn Write, max: u64, progress: &mut dyn FnMut(u64)) -> Result<u64>;
}

/// The real thing: ureq over rustls, https only, with timeouts.
pub struct Net {
    agent: ureq::Agent,
}

impl Net {
    pub fn new() -> Net {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .timeout_write(Duration::from_secs(60))
            .https_only(true)
            .try_proxy_from_env(true)
            .user_agent(concat!("beacon/", env!("CARGO_PKG_VERSION")))
            .build();
        Net { agent }
    }
}

impl Default for Net {
    fn default() -> Self {
        Net::new()
    }
}

fn http_err(e: ureq::Error) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, _) => anyhow!("the workshop answered HTTP {code}"),
        ureq::Error::Transport(t) => anyhow!("couldn't reach the workshop: {t}"),
    }
}

impl Http for Net {
    fn get_json(&self, url: &str, query: &[(&str, String)]) -> Result<Value> {
        let mut req = self
            .agent
            .get(url)
            .timeout(Duration::from_secs(45))
            .set("AuthAccountUuid", "unauthenticated")
            .set("AuthAccountPassword", "")
            .set("Accept", "application/json");
        for (k, v) in query {
            req = req.query(k, v);
        }
        let resp = req.call().map_err(http_err)?;
        let mut body = Vec::new();
        resp.into_reader().take(MAX_JSON_BYTES + 1).read_to_end(&mut body).context("reading the workshop's answer")?;
        if body.len() as u64 > MAX_JSON_BYTES {
            bail!("the workshop's answer is too large");
        }
        serde_json::from_slice(&body).context("the workshop's answer isn't JSON")
    }

    fn download(&self, url: &str, out: &mut dyn Write, max: u64, progress: &mut dyn FnMut(u64)) -> Result<u64> {
        if https(url).is_none() {
            bail!("not an https download: {url}");
        }
        let resp = self.agent.get(url).call().map_err(http_err)?;
        if let Some(len) = resp.header("Content-Length").and_then(|l| l.trim().parse::<u64>().ok()) {
            if len > max {
                bail!("the download is {len} bytes, more than expected ({max})");
            }
        }
        let mut r = resp.into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            let n = r.read(&mut buf).context("downloading")?;
            if n == 0 {
                break;
            }
            total += n as u64;
            if total > max {
                bail!("the download is larger than expected ({max} bytes)");
            }
            out.write_all(&buf[..n])?;
            progress(total);
        }
        Ok(total)
    }
}

// ---------------------------------------------------------------- list and install

/// The language code the workshop tags files with, from Beacon's `language`
/// setting (the registry's Language value, e.g. `english`).
pub fn language_code(language: &str) -> String {
    let l = language.trim().to_ascii_lowercase();
    match l.as_str() {
        "english" | "english uk" | "english us" | "" => "EN".into(),
        "french" => "FR".into(),
        "german" => "DE".into(),
        "italian" => "IT".into(),
        "spanish" => "ES".into(),
        "swedish" => "SV".into(),
        "dutch" => "NL".into(),
        "polish" => "PL".into(),
        "norwegian" => "NO".into(),
        "russian" => "RU".into(),
        "turkish" => "TR".into(),
        other => other.chars().take(2).collect::<String>().to_ascii_uppercase(),
    }
}

/// The workshop's game number for a Beacon game, or a usage-style error.
pub fn game_number(g: GameId) -> Result<i64> {
    aio::game_index(g).ok_or_else(|| anyhow!("the BFME workshop has content for BFME, BFME II and RotWK only"))
}

/// Mods in `<mods>/<game>/` that came from the workshop: (guid, folder).
pub fn installed(mods_dir: &Path, g: GameId) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(mods_dir.join(g.slug())) else { return vec![] };
    rd.flatten()
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path().join("beacon-mod.json")).ok()?;
            let v: Value = serde_json::from_str(&text).ok()?;
            let guid = field(&v, "workshop").and_then(|w| field_str(w, "guid"))?;
            Some((guid, e.path()))
        })
        .collect()
}

/// List a page of fan content for a game. Returns (items, hidden count).
pub fn list(http: &dyn Http, g: GameId, keyword: &str, page: u32) -> Result<(Vec<Item>, usize)> {
    let n = game_number(g)?;
    let kw = keyword.trim();
    let q = [
        ("keyword", if kw.is_empty() { "~".to_string() } else { kw.chars().take(100).collect() }),
        ("game", n.to_string()),
        ("type", "-1".to_string()),
        ("sortMode", "0".to_string()),
        ("page", page.to_string()),
    ];
    let v = http.get_json(&format!("{API}/workshop/query"), &q)?;
    Ok(parse_list(&v, g))
}

/// Fetch one entry's full description (files included).
pub fn fetch(http: &dyn Http, id: &str) -> Result<Value> {
    if id.is_empty() || id.len() > 200 || id.chars().any(|c| c.is_control() || c.is_whitespace() || c == '/' || c == '\\') {
        bail!("that isn't a workshop id");
    }
    if let Some(why) = refusal(id, 1) {
        bail!("{why}");
    }
    http.get_json(&format!("{API}/workshop/download"), &[("guid", id.to_string())])
}

/// What an install did.
#[derive(Debug, Clone, Serialize)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: &'static str,
    pub dir: PathBuf,
    pub files: usize,
    pub bytes: u64,
    pub skipped: Vec<String>,
}

fn md5_hex(d: md5::Md5) -> String {
    use md5::Digest;
    d.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A writer that hashes what goes through it.
struct Hashing<'a> {
    file: &'a mut std::fs::File,
    md5: md5::Md5,
}

impl Write for Hashing<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use md5::Digest;
        let n = self.file.write(buf)?;
        self.md5.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Download a package into `<tmp>/…`, check it, then move it to
/// `<mods>/<game>/<folder>` (replacing an older copy of the same entry).
/// `progress(pct, text)` reports along the way.
pub fn install(
    http: &dyn Http,
    pkg: &Package,
    mods_dir: &Path,
    tmp_root: &Path,
    progress: &mut dyn FnMut(Option<u8>, &str),
) -> Result<Installed> {
    let g = pkg.item.game;
    let lib = mods_dir.join(g.slug());
    std::fs::create_dir_all(&lib)?;
    // The folder: an earlier install of this entry, else a free name.
    let existing = installed(mods_dir, g).into_iter().find(|(guid, _)| guid == &pkg.item.id).map(|(_, d)| d);
    let target = match &existing {
        Some(d) => d.clone(),
        None => {
            let base = folder_for(&pkg.item.name, &pkg.item.id);
            let mut t = lib.join(&base);
            if t.exists() {
                let short: String = pkg.item.id.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
                t = lib.join(format!("{base} ({short})"));
            }
            if t.exists() {
                bail!("{} already exists and isn't this workshop entry", t.display());
            }
            t
        }
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let tmp = tmp_root.join(format!("workshop-{stamp}"));
    std::fs::create_dir_all(&tmp)?;
    let result = (|| -> Result<u64> {
        // Map packs go under maps\, where the game looks for maps inside a -mod folder.
        let base = if pkg.item.kind == "maps" { tmp.join("maps") } else { tmp.clone() };
        let declared: u64 = pkg.files.iter().map(|f| f.size).sum();
        let mut done: u64 = 0;
        let n = pkg.files.len();
        for (i, f) in pkg.files.iter().enumerate() {
            let dest = base.join(&f.path);
            if !dest.starts_with(&tmp) {
                bail!("unsafe file path: {}", f.path.display());
            }
            std::fs::create_dir_all(dest.parent().unwrap_or(&base))?;
            // create_new: never follows or overwrites anything already there.
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&dest).with_context(|| format!("creating {}", dest.display()))?;
            let label = f.path.to_string_lossy().into_owned();
            let max = if f.size > 0 { f.size } else { MAX_FILE_BYTES };
            let mut w = Hashing { file: &mut file, md5: <md5::Md5 as md5::Digest>::new() };
            let start = done;
            let got = http.download(&f.url, &mut w, max, &mut |b| {
                let pct = if declared > 0 { Some((5 + (start + b) * 90 / declared).min(95) as u8) } else { Some((5 + i * 90 / n.max(1)) as u8) };
                progress(pct, &format!("Downloading {label}"));
            })?;
            let hash = md5_hex(w.md5);
            file.flush()?;
            if f.size > 0 && got != f.size {
                bail!("{label}: got {got} bytes, the workshop said {}", f.size);
            }
            if let Some(want) = &f.md5 {
                if &hash != want {
                    bail!("{label}: checksum doesn't match (MD5 {hash}, expected {want})");
                }
            }
            done += got;
            if done > MAX_PACKAGE_BYTES {
                bail!("the package is larger than Beacon downloads");
            }
        }
        let manifest = serde_json::json!({
            "name": pkg.item.name,
            "kind": "mod",
            "workshop": {
                "guid": pkg.item.id,
                "version": pkg.item.version,
                "kind": pkg.item.kind,
                "author": pkg.item.author,
                "source": SITE,
            },
        });
        std::fs::write(tmp.join("beacon-mod.json"), serde_json::to_string_pretty(&manifest)?)?;
        Ok(done)
    })();
    let bytes = match result {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    progress(Some(97), "Adding it to Beacon's mods");
    if target.exists() {
        std::fs::remove_dir_all(&target).with_context(|| format!("replacing {}", target.display()))?;
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e).with_context(|| format!("moving the download to {}", target.display()));
    }
    Ok(Installed {
        id: pkg.item.id.clone(),
        name: pkg.item.name.clone(),
        version: pkg.item.version.clone(),
        kind: pkg.item.kind,
        dir: target,
        files: pkg.files.len(),
        bytes,
        skipped: pkg.skipped.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;
    use std::collections::HashMap;

    fn list_fixture() -> Value {
        json!([
            {"Guid": "original-RotWK", "Name": "The Rise of the Witch-king", "Game": 2, "Type": 0, "Size": 6000000000u64},
            {"Guid": "official-2", "Name": "Patch 2.01", "Game": 2, "Type": 0},
            {"Guid": "exp-original-RotWK", "Name": "RotWK (expansion)", "Game": 2, "Type": 0},
            {"Guid": "snapshot-RotWK-1", "Name": "My snapshot", "Game": 2, "Type": 4},
            {"Guid": "6f1c2a7e-0000-4000-8000-000000000001", "Name": "Edain Mod", "Version": "4.8.1", "Author": "Edain Team",
             "Description": "Lore-focused overhaul", "Game": 2, "Type": 1, "Size": 2500000000u64,
             "ArtworkUrl": "https://workshop-files.bfmeladder.com/a.png", "Metadata": {"Downloads": 1234, "Versions": ["4.8.1"]}},
            // camelCase keys work too.
            {"guid": "6f1c2a7e-0000-4000-8000-000000000002", "name": "Map Pack", "game": 2, "type": 3, "size": 1000, "artworkUrl": "http://insecure/x.png"},
            {"Guid": "6f1c2a7e-0000-4000-8000-000000000003", "Name": "BFME2 thing", "Game": 1, "Type": 1},
        ])
    }

    #[test]
    fn list_keeps_fan_content_only() {
        let (items, hidden) = parse_list(&list_fixture(), GameId::Rotwk);
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Edain Mod", "Map Pack"]);
        assert_eq!(hidden, 5);
        let e = &items[0];
        assert_eq!((e.kind, e.size, e.downloads, e.version.as_str()), ("mod", 2_500_000_000, 1234, "4.8.1"));
        assert_eq!(e.artwork_url.as_deref(), Some("https://workshop-files.bfmeladder.com/a.png"));
        assert_eq!(items[1].kind, "maps");
        assert!(items[1].artwork_url.is_none(), "http links are dropped");
        let v = serde_json::to_value(e).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["artwork_url", "author", "description", "downloads", "game", "id", "installed", "kind", "name", "size", "version"]);
    }

    #[test]
    fn languages() {
        assert_eq!(language_code("english"), "EN");
        assert_eq!(language_code("German"), "DE");
        assert_eq!(language_code("portuguese"), "PO");
    }

    #[test]
    fn refusals() {
        assert!(refusal("original-BFME1", 0).is_some());
        assert!(refusal("Original-RotWK", 0).is_some());
        assert!(refusal("exp-original-RotWK", 0).is_some());
        assert!(refusal("official-1", 0).is_some());
        assert!(refusal("snapshot-BFME2-x", 4).is_some());
        assert!(refusal("abc", 4).is_some());
        assert!(refusal("abc", 9).is_some());
        assert!(refusal("abc", 1).is_none() && refusal("abc", 0).is_none());
    }

    #[test]
    fn safe_paths() {
        assert_eq!(safe_rel_path("data\\ini\\weapon.ini").unwrap(), Path::new("data/ini/weapon.ini"));
        assert_eq!(safe_rel_path("./a//b.big").unwrap(), Path::new("a/b.big"));
        for bad in ["../evil.big", "a/../../b", "/etc/passwd", "\\\\server\\share\\x", "C:\\Windows\\x.dll", "c:x", "a/CON/b", "nul.txt", "COM1", "a\u{0}b", "", "./", "trailing./x", "x "] {
            assert!(safe_rel_path(bad).is_none(), "{bad:?} should be refused");
        }
        assert!(safe_rel_path("Console.big").is_some());
        assert_eq!(folder_for("Edain Mod 4.8!", "g"), "Edain Mod 4.8!".replace('.', " ").split_whitespace().collect::<Vec<_>>().join(" "));
        assert_eq!(folder_for("../..", "6f1c2a7e-0000"), "workshop-6f1c2a7e0000");
        assert_eq!(folder_for("CON", "abc"), "workshop-abc");
    }

    #[test]
    fn base_game_files_are_never_taken() {
        assert!(skip_file("lotrbfme2ep1.exe") && skip_file("data/GAME.DAT") && skip_file("_zzlotr.big") && skip_file("EALogo.vp6"));
        assert!(!skip_file("aotr_main.big") && !skip_file("data/ini/weapon.ini") && !skip_file("lang/English.big"));
        assert!(looks_like_base_game(&["ini.big", "w3d.big", "textures.big", "readme.txt"]));
        assert!(!looks_like_base_game(&["ini.big", "mymod.big"]));
    }

    fn package_json(files: Value) -> Value {
        json!({"Guid": "6f1c2a7e-0000-4000-8000-000000000001", "Name": "Edain Mod", "Version": "4.8.1", "Game": 2, "Type": 1, "Files": files})
    }

    #[test]
    fn packages() {
        let p = parse_package(
            &package_json(json!([
                {"Name": "edain.big", "Url": "https://workshop-files.bfmeladder.com/1", "Md5": "D41D8CD98F00B204E9800998ECF8427E", "Language": "ALL", "Size": 0},
                {"Name": "lang\\german.big", "Url": "https://x/2", "Language": "DE", "Size": 3},
                {"Name": "lang\\english.big", "Url": "https://x/3", "Language": "EN FR", "Size": 3},
                {"Name": "lotrbfme2ep1.exe", "Url": "https://x/4", "Language": "ALL", "Size": 3},
                {"Name": "script.wps", "Url": "https://x/5"},
                {"Name": "EDAIN.BIG", "Url": "https://x/6"},
            ])),
            "EN",
        )
        .unwrap();
        let paths: Vec<String> = p.files.iter().map(|f| f.path.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(paths, vec!["edain.big", "lang/english.big"]);
        assert_eq!(p.files[0].md5.as_deref(), Some("d41d8cd98f00b204e9800998ecf8427e"));
        assert_eq!(p.skipped.len(), 4);

        let err = |files: Value| parse_package(&package_json(files), "EN").unwrap_err().to_string();
        assert!(err(json!([{"Name": "../x.big", "Url": "https://x/1"}])).contains("unsafe"));
        assert!(err(json!([{"Name": "x.big", "Url": "http://x/1"}])).contains("https"));
        assert!(err(json!([{"Name": "x.big", "Url": "https://x/1", "Size": MAX_FILE_BYTES + 1}])).contains("more than"));
        assert!(err(json!([{"Name": "ini.big", "Url": "https://x/1"}, {"Name": "w3d.big", "Url": "https://x/2"}, {"Name": "maps.big", "Url": "https://x/3"}])).contains("base game"));
        assert!(err(json!([])).contains("no files"));
        let base = json!({"Guid": "original-RotWK", "Name": "RotWK", "Game": 2, "Type": 0, "Files": []});
        assert!(parse_package(&base, "EN").unwrap_err().to_string().contains("never downloads"));
    }

    /// Serves files from memory and records what was asked.
    struct Fake {
        files: HashMap<String, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl Http for Fake {
        fn get_json(&self, url: &str, query: &[(&str, String)]) -> Result<Value> {
            self.asked.borrow_mut().push(format!("{url}?{}", query.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")));
            Ok(list_fixture())
        }
        fn download(&self, url: &str, out: &mut dyn Write, max: u64, progress: &mut dyn FnMut(u64)) -> Result<u64> {
            self.asked.borrow_mut().push(url.to_string());
            let data = self.files.get(url).ok_or_else(|| anyhow!("404"))?;
            if data.len() as u64 > max {
                bail!("larger than expected");
            }
            out.write_all(data)?;
            progress(data.len() as u64);
            Ok(data.len() as u64)
        }
    }

    fn md5_of(data: &[u8]) -> String {
        use md5::Digest;
        let mut h = md5::Md5::new();
        h.update(data);
        md5_hex(h)
    }

    #[test]
    fn install_into_the_mods_library() {
        let t = tempfile::tempdir().unwrap();
        let mods = t.path().join("mods");
        let tmp = t.path().join("tmp");
        let fake = Fake {
            files: [("https://f/1".to_string(), b"BIGF-edain".to_vec()), ("https://f/2".to_string(), b"[ini]".to_vec())].into_iter().collect(),
            asked: RefCell::new(vec![]),
        };
        let (items, _) = list(&fake, GameId::Rotwk, "", 0).unwrap();
        assert_eq!(items.len(), 2);
        assert!(fake.asked.borrow()[0].contains("keyword=~&game=2&type=-1"), "{:?}", fake.asked.borrow());
        assert!(list(&fake, GameId::Swgb, "", 0).is_err());

        let files = json!([
            {"Name": "edain.big", "Url": "https://f/1", "Md5": md5_of(b"BIGF-edain"), "Size": 10},
            {"Name": "data\\ini\\x.ini", "Url": "https://f/2", "Size": 5},
        ]);
        let pkg = parse_package(&package_json(files.clone()), "EN").unwrap();
        let mut said = Vec::new();
        let done = install(&fake, &pkg, &mods, &tmp, &mut |p, s| said.push((p, s.to_string()))).unwrap();
        assert_eq!(done.dir, mods.join("rotwk/Edain Mod"));
        assert_eq!(std::fs::read(done.dir.join("edain.big")).unwrap(), b"BIGF-edain");
        assert!(done.dir.join("data/ini/x.ini").is_file());
        assert!(said.iter().any(|(_, s)| s.starts_with("Downloading")));
        assert_eq!(installed(&mods, GameId::Rotwk), vec![(pkg.item.id.clone(), done.dir.clone())]);
        // It's a mod like any other.
        let m = crate::mods::scan_library(&mods, GameId::Rotwk);
        assert_eq!((m.len(), m[0].name.as_str(), m[0].kind), (1, "Edain Mod", crate::mods::ModKind::Mod));
        assert!(std::fs::read_dir(&tmp).unwrap().next().is_none(), "nothing left in tmp");

        // Reinstalling replaces the same folder.
        install(&fake, &pkg, &mods, &tmp, &mut |_, _| {}).unwrap();
        assert_eq!(std::fs::read_dir(mods.join("rotwk")).unwrap().count(), 1);

        // A bad checksum or size leaves nothing behind.
        let bad = parse_package(&package_json(json!([{"Name": "edain.big", "Url": "https://f/1", "Md5": "0".repeat(32), "Size": 10}])), "EN").unwrap();
        let e = install(&fake, &bad, &t.path().join("mods2"), &tmp, &mut |_, _| {}).unwrap_err();
        assert!(e.to_string().contains("checksum"), "{e}");
        let short = parse_package(&package_json(json!([{"Name": "edain.big", "Url": "https://f/1", "Size": 99}])), "EN").unwrap();
        assert!(install(&fake, &short, &t.path().join("mods2"), &tmp, &mut |_, _| {}).is_err());
        assert!(!t.path().join("mods2/rotwk/Edain Mod").exists());
        assert!(std::fs::read_dir(&tmp).unwrap().next().is_none());
    }

    #[test]
    fn maps_go_under_maps() {
        let t = tempfile::tempdir().unwrap();
        let fake = Fake { files: [("https://f/m".to_string(), b"map".to_vec())].into_iter().collect(), asked: RefCell::new(vec![]) };
        let v = json!({"Guid": "6f1c2a7e-0000-4000-8000-00000000000a", "Name": "Rhun Maps", "Game": 0, "Type": 3,
                       "Files": [{"Name": "map wor rhun\\map wor rhun.map", "Url": "https://f/m", "Size": 3}]});
        let pkg = parse_package(&v, "EN").unwrap();
        let done = install(&fake, &pkg, &t.path().join("mods"), &t.path().join("tmp"), &mut |_, _| {}).unwrap();
        assert!(done.dir.join("maps/map wor rhun/map wor rhun.map").is_file());
        assert!(done.dir.starts_with(t.path().join("mods/bfme1")));
    }

    #[test]
    fn fetch_refuses_base_games_without_asking() {
        let fake = Fake { files: HashMap::new(), asked: RefCell::new(vec![]) };
        assert!(fetch(&fake, "original-BFME1").is_err());
        assert!(fetch(&fake, "official-0").is_err());
        assert!(fetch(&fake, "../x").is_err());
        assert!(fake.asked.borrow().is_empty(), "nothing was requested");
    }
}
