//! `sources.json`: what Beacon can download and how to find it *right now*.
//!
//! Nothing here is a fixed download link. Each source says how to look the
//! file up, and Beacon does that at download time:
//!
//! * `workshop`: the BFME Foundation's API (`workshop/download?guid=`), which
//!   answers with the current file URLs and checksums. If the API stops
//!   answering, Beacon reads the All-in-One launcher's source on GitHub for
//!   the server it uses now, remembers it, and carries on. If the package's
//!   id changed, it searches the workshop for the game's base package (that
//!   guess is shown to you before anything downloads).
//! * `page`: a web page scanned for a link whose file name matches a wildcard
//!   pattern (`AotR_Standalone_*_Installer.zip`), newest version first,
//!   optionally through intermediate pages (`steps`). When nothing matches any
//!   more, the closest link (same extension, most words in common) is offered,
//!   never downloaded without a yes.
//! * `github`: the latest release's asset matching a pattern.
//! * `direct`: a plain https URL (checked by "Check sources").
//!
//! The list itself is updated without a rebuild: a copy is bundled, and
//! Beacon fetches a newer `sources.json` from the private repo (with a
//! read-only GitHub token) at most every few hours. The higher `version` wins.

use super::text::{self, glob, natural_cmp};
use super::{host, is_https, Transfer};
use crate::games::GameId;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The list compiled into this build (the repo's `sources.json`).
pub const BUNDLED: &str = include_str!("../../../../sources.json");
/// The `schema` this build understands.
pub const SCHEMA: u32 = 1;
/// How often Beacon looks for a newer list on its own.
pub const UPDATE_EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceList {
    pub schema: u32,
    pub version: u64,
    #[serde(default)]
    pub workshop: WorkshopCfg,
    #[serde(default)]
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkshopCfg {
    /// The API root, e.g. `https://bfmeladder.com/api`.
    pub api: String,
    /// Files that name the server the launcher uses (its source on GitHub).
    pub discover: Vec<String>,
    /// Text right before the server's URL in those files.
    pub discover_pattern: String,
    /// Added to the discovered server to get the API root.
    pub api_suffix: String,
}

impl Default for WorkshopCfg {
    fn default() -> Self {
        WorkshopCfg {
            api: "https://bfmeladder.com/api".into(),
            discover: vec![],
            discover_pattern: "WorkshopServerHost = \"".into(),
            api_suffix: "/api".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub game: Option<GameId>,
    /// The game whose community launcher this is (UaW).
    #[serde(default)]
    pub launcher_for: Option<GameId>,
    /// The game this archive holds, ready to play (extracted into the games folder).
    #[serde(default)]
    pub archive_for: Option<GameId>,
    /// The folder name under the games folder, for `archive_for` (default: the game's id).
    #[serde(default)]
    pub install_folder: Option<String>,
    #[serde(flatten)]
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Kind {
    Workshop {
        guid: String,
        /// When `guid` is gone: the base package is the one whose id starts with this.
        #[serde(default)]
        guid_prefix: Option<String>,
    },
    Page {
        page: String,
        /// Wildcard patterns for the links to follow first (each on the page the previous one led to).
        #[serde(default)]
        steps: Vec<String>,
        /// Wildcard pattern for the file's name (or the link's URL or text).
        pattern: String,
    },
    Github {
        /// `owner/repo`.
        repo: String,
        /// Wildcard pattern for the asset's name.
        asset: String,
    },
    /// A public Google Drive folder: the newest file matching `patterns`.
    DriveFolder {
        /// The folder's id (or its link).
        folder: String,
        patterns: Vec<String>,
        /// Names with this word win (e.g. "launcher").
        #[serde(default)]
        prefer: Option<String>,
        /// How many levels of subfolders to look through when the folder has no match itself.
        #[serde(default = "one")]
        depth: u8,
    },
    /// A single public Google Drive file (its id, or a `…/file/d/<id>/…` link).
    DriveFile {
        file: String,
        /// Its name, for messages and the downloaded file (Drive doesn't say without an API key).
        #[serde(default)]
        name: Option<String>,
    },
    Direct {
        url: String,
        #[serde(default)]
        md5: Option<String>,
        #[serde(default)]
        size: Option<u64>,
    },
}

fn one() -> u8 {
    1
}

impl SourceList {
    pub fn parse(text: &str) -> Result<SourceList> {
        let l: SourceList = serde_json::from_str(text).context("sources.json isn't valid")?;
        if l.schema != SCHEMA {
            bail!("sources.json schema {} (this Beacon understands {SCHEMA}); update Beacon", l.schema);
        }
        let mut seen = Vec::new();
        for s in &l.sources {
            if s.id.trim().is_empty() || seen.contains(&s.id) {
                bail!("sources.json: empty or repeated id {:?}", s.id);
            }
            seen.push(s.id.clone());
            let urls: Vec<&str> = match &s.kind {
                Kind::Page { page, .. } => vec![page.as_str()],
                Kind::Direct { url, .. } => vec![url.as_str()],
                _ => vec![],
            };
            if let Kind::DriveFile { file, .. } = &s.kind {
                if super::drive::file_id(file).is_none() {
                    bail!("sources.json: {} needs a Drive file id or link", s.id);
                }
            }
            if let Kind::DriveFolder { folder, patterns, .. } = &s.kind {
                if super::drive::folder_id(folder).is_none() || patterns.is_empty() {
                    bail!("sources.json: {} needs a Drive folder id and at least one pattern", s.id);
                }
            }
            if let Some(bad) = urls.into_iter().find(|u| !is_https(u)) {
                bail!("sources.json: {} isn't an https link ({bad})", s.id);
            }
        }
        Ok(l)
    }

    pub fn bundled() -> SourceList {
        SourceList::parse(BUNDLED).expect("the bundled sources.json is valid")
    }

    pub fn get(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|s| s.id.eq_ignore_ascii_case(id))
    }

    /// The source that installs a game's base files.
    pub fn for_game(&self, g: GameId) -> Option<&Source> {
        self.sources.iter().find(|s| s.game == Some(g))
    }

    /// The source of a game's ready-to-play archive.
    pub fn archive_for(&self, g: GameId) -> Option<&Source> {
        self.sources.iter().find(|s| s.archive_for == Some(g))
    }

    /// The source of a game's community launcher.
    pub fn launcher_for(&self, g: GameId) -> Option<&Source> {
        self.sources.iter().find(|s| s.launcher_for == Some(g))
    }
}

// ---------------------------------------------------------------- where things live

/// Private-build settings (`<config dir>/private.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivateSettings {
    /// The public GitHub repo (`owner/repo`) that Beacon updates itself and `sources.json` from.
    pub sources_repo: String,
    pub sources_branch: String,
    /// No longer used (updates come from a public repo); an old token is read and dropped on the next save.
    #[serde(skip_serializing)]
    pub github_token: Option<String>,
    /// Where downloaded games go (default `<data>/games`).
    pub games_dir: Option<PathBuf>,
}

impl Default for PrivateSettings {
    fn default() -> Self {
        PrivateSettings { sources_repo: "Dragongamer9/beacon-private".into(), sources_branch: "main".into(), github_token: None, games_dir: None }
    }
}

/// Files the download engine keeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    /// `<config dir>/private.json`.
    pub settings_file: PathBuf,
    /// `<data>/downloads`.
    pub dir: PathBuf,
}

/// What Beacon worked out by itself and keeps until the list says otherwise.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Learned {
    /// An API root found through discovery, and the list version it was found under.
    pub workshop_api: Option<String>,
    pub workshop_api_list_version: u64,
}

impl Store {
    pub fn new(config_file: &Path, data_dir: &Path) -> Store {
        let cfg_dir = config_file.parent().map(Path::to_path_buf).unwrap_or_else(|| data_dir.to_path_buf());
        Store { settings_file: cfg_dir.join("private.json"), dir: data_dir.join("downloads") }
    }

    pub fn settings(&self) -> PrivateSettings {
        std::fs::read_to_string(&self.settings_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save_settings(&self, s: &PrivateSettings) -> Result<()> {
        write_private(&self.settings_file, &serde_json::to_string_pretty(s)?)
    }

    fn cached_file(&self) -> PathBuf {
        self.dir.join("sources.json")
    }
    fn stamp_file(&self) -> PathBuf {
        self.dir.join("sources-updated")
    }
    fn learned_file(&self) -> PathBuf {
        self.dir.join("learned.json")
    }
    pub fn status_file(&self) -> PathBuf {
        self.dir.join("source-status.json")
    }

    /// The list in use: the bundled one, or the downloaded copy when it's newer.
    pub fn list(&self) -> SourceList {
        let bundled = SourceList::bundled();
        let cached = std::fs::read_to_string(self.cached_file()).ok().and_then(|t| SourceList::parse(&t).ok());
        match cached {
            Some(c) if c.version > bundled.version => c,
            _ => bundled,
        }
    }

    pub fn learned(&self) -> Learned {
        std::fs::read_to_string(self.learned_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save_learned(&self, l: &Learned) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        write_atomic(&self.learned_file(), &serde_json::to_string_pretty(l)?)
    }

    fn last_update(&self) -> Option<SystemTime> {
        let secs: u64 = std::fs::read_to_string(self.stamp_file()).ok()?.trim().parse().ok()?;
        Some(UNIX_EPOCH + Duration::from_secs(secs))
    }

    /// Fetch `sources.json` from the updates repo (public, no login); keep it when it's newer.
    /// Without `force`, does nothing when it looked less than [`UPDATE_EVERY`] ago.
    pub fn update(&self, t: &dyn Transfer, force: bool) -> Result<Update> {
        let current = self.list().version;
        let s = self.settings();
        if !force {
            if let Some(last) = self.last_update() {
                if SystemTime::now().duration_since(last).map(|d| d < UPDATE_EVERY).unwrap_or(true) {
                    return Ok(Update { from: current, to: current, skipped: Some("checked recently".into()) });
                }
            }
        }
        let repo = check_repo(&s.sources_repo)?;
        let branch: String = s.sources_branch.chars().filter(|c| c.is_ascii_alphanumeric() || "-_./".contains(*c)).collect();
        let url = format!("https://raw.githubusercontent.com/{repo}/{branch}/sources.json");
        let body = t.get_text(&url, &[], super::MAX_TEXT_BYTES).map_err(|e| {
            let m = format!("{e:#}");
            if m.contains("HTTP 404") || m.contains("HTTP 403") || m.contains("HTTP 401") {
                anyhow!("{m}: {repo} has no public sources.json on {branch} (is the repo public?)")
            } else {
                e
            }
        })?;
        let new = SourceList::parse(&body)?;
        std::fs::create_dir_all(&self.dir)?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        write_atomic(&self.stamp_file(), &now.to_string())?;
        if new.version > current {
            write_atomic(&self.cached_file(), &body)?;
            Ok(Update { from: current, to: new.version, skipped: None })
        } else {
            Ok(Update { from: current, to: current, skipped: None })
        }
    }
}

/// `owner/repo`, trimmed, or an error.
pub fn check_repo(repo: &str) -> Result<String> {
    let r = repo.trim().trim_start_matches("https://github.com/").trim_end_matches('/').trim_end_matches(".git");
    let parts: Vec<&str> = r.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") || r.chars().any(|c| !(c.is_ascii_alphanumeric() || "-_./".contains(c))) {
        bail!("{repo:?} isn't a GitHub repo (owner/repo)");
    }
    Ok(r.to_string())
}

/// What `update` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Update {
    pub from: u64,
    pub to: u64,
    /// Why it didn't look, if it didn't.
    pub skipped: Option<String>,
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

/// Write a file only its owner can read.
fn write_private(path: &Path, text: &str) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        use std::io::Write;
        let mut f = o.open(&tmp)?;
        f.write_all(text.as_bytes())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

// ---------------------------------------------------------------- resolving

/// How a source was found this time.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "how", rename_all = "kebab-case")]
pub enum How {
    /// Where the list says.
    Listed,
    /// Somewhere else Beacon found by itself; `confirm` = only a guess, ask first.
    Rediscovered { note: String, confirm: bool },
}

impl How {
    pub fn needs_confirm(&self) -> bool {
        matches!(self, How::Rediscovered { confirm: true, .. })
    }
}

/// What a source points at now.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A workshop package (its full answer, files included) and the API it came from.
    Workshop { api: String, entry: Value },
    /// A single file.
    File { url: String, name: String, size: Option<u64>, md5: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub source: String,
    pub how: How,
    pub target: Target,
}

/// The headers the Foundation's API expects from an unauthenticated client.
fn workshop_headers() -> [(&'static str, String); 3] {
    [("AuthAccountUuid", "unauthenticated".into()), ("AuthAccountPassword", String::new()), ("Accept", "application/json".into())]
}

/// `workshop/download` for `guid` on `api`, if it answers with that package and files.
fn workshop_entry(t: &dyn Transfer, api: &str, guid: &str) -> Result<Value> {
    let url = format!("{}/workshop/download?guid={}", api.trim_end_matches('/'), url_encode(guid));
    let v = t.get_json(&url, &workshop_headers())?;
    let got = crate::aio::field_str(&v, "Guid").unwrap_or_default();
    let files = crate::aio::field(&v, "Files").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
    if !got.eq_ignore_ascii_case(guid) || files == 0 {
        bail!("the workshop has no package {guid}");
    }
    Ok(v)
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// The API root the launcher's source on GitHub names now.
pub fn discover_api(t: &dyn Transfer, cfg: &WorkshopCfg) -> Result<String> {
    let mut last = anyhow!("no discovery files listed");
    for u in &cfg.discover {
        match t.get_text(u, &[], 1 << 20) {
            Ok(body) => {
                if let Some(api) = find_after(&body, &cfg.discover_pattern) {
                    return Ok(format!("{}{}", api.trim_end_matches('/'), cfg.api_suffix));
                }
                last = anyhow!("{} no longer names the server", text::file_name(u));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The https URL right after `marker` in `body`, up to the closing quote.
fn find_after(body: &str, marker: &str) -> Option<String> {
    let start = body.find(marker)? + marker.len();
    let rest = &body[start..];
    let end = rest.find(['"', '\'', ';', '\n']).unwrap_or(rest.len());
    let url = rest[..end].trim();
    is_https(url).then(|| url.to_string())
}

/// Find a source's file or package now. `learned` is updated when Beacon
/// finds the workshop somewhere new (the caller saves it).
pub fn resolve(t: &dyn Transfer, list: &SourceList, learned: &mut Learned, src: &Source) -> Result<Resolved> {
    let r = |how: How, target: Target| -> Result<Resolved> { Ok(Resolved { source: src.id.clone(), how, target }) };
    match &src.kind {
        Kind::Workshop { guid, guid_prefix } => {
            let (api, entry, how) = resolve_workshop(t, list, learned, src, guid, guid_prefix.as_deref())?;
            r(how, Target::Workshop { api, entry })
        }
        Kind::Direct { url, md5, size } => r(How::Listed, Target::File { url: url.clone(), name: text::file_name(url), size: *size, md5: md5.clone() }),
        Kind::Page { page, steps, pattern } => {
            let (url, name, how) = resolve_page(t, page, steps, pattern)?;
            r(how, Target::File { url, name, size: None, md5: None })
        }
        Kind::Github { repo, asset } => {
            let (url, name, size, how) = resolve_github(t, repo, asset)?;
            r(how, Target::File { url, name, size, md5: None })
        }
        Kind::DriveFile { file, name } => {
            let id = super::drive::file_id(file).ok_or_else(|| anyhow!("{} isn't a Drive file", src.id))?;
            let (url, size) = super::drive::download_url(t, &id)?;
            let name = name.clone().unwrap_or_else(|| format!("{id}.zip"));
            r(How::Listed, Target::File { url, name, size, md5: None })
        }
        Kind::DriveFolder { folder, patterns, prefer, depth } => {
            let folder = super::drive::folder_id(folder).ok_or_else(|| anyhow!("{} isn't a Drive folder", src.id))?;
            let (entry, how) = match super::drive::pick(t, &folder, patterns, prefer.as_deref(), *depth)? {
                Some((e, path)) if path.is_empty() => (e, How::Listed),
                Some((e, path)) => {
                    let note = format!("found in the subfolder {}", path.join(" / "));
                    (e, How::Rediscovered { note, confirm: false })
                }
                None => match super::drive::near_miss(t, &folder, patterns)? {
                    Some(e) => {
                        let note = format!("no file matches {} any more; the closest is {}", patterns.join(" or "), e.name);
                        (e, How::Rediscovered { note, confirm: true })
                    }
                    None => bail!("no file matching {} in the Google Drive folder", patterns.join(" or ")),
                },
            };
            let (url, size) = super::drive::download_url(t, &entry.id)?;
            r(how, Target::File { url, name: entry.name, size, md5: None })
        }
    }
}

fn resolve_workshop(
    t: &dyn Transfer,
    list: &SourceList,
    learned: &mut Learned,
    src: &Source,
    guid: &str,
    prefix: Option<&str>,
) -> Result<(String, Value, How)> {
    // 1. Where we found it last (unless the list has been updated since), then the list's API.
    let mut apis: Vec<String> = Vec::new();
    if let Some(a) = &learned.workshop_api {
        if learned.workshop_api_list_version >= list.version {
            apis.push(a.clone());
        }
    }
    if !apis.contains(&list.workshop.api) {
        apis.push(list.workshop.api.clone());
    }
    let mut errors = Vec::new();
    let mut reachable: Option<String> = None;
    for api in &apis {
        match workshop_entry(t, api, guid) {
            Ok(v) => {
                let how = if *api == list.workshop.api {
                    How::Listed
                } else {
                    How::Rediscovered { note: format!("the workshop is at {} (found earlier)", host(api)), confirm: false }
                };
                return Ok((api.clone(), v, how));
            }
            Err(e) => {
                if format!("{e:#}").contains("no package") {
                    reachable.get_or_insert(api.clone());
                }
                errors.push(format!("{}: {e:#}", host(api)));
            }
        }
    }
    // 2. The server moved: ask the launcher's source where it is now.
    if reachable.is_none() {
        match discover_api(t, &list.workshop) {
            Ok(api) if !apis.contains(&api) => match workshop_entry(t, &api, guid) {
                Ok(v) => {
                    learned.workshop_api = Some(api.clone());
                    learned.workshop_api_list_version = list.version;
                    let note = format!("the workshop moved to {}; Beacon found it in the All-in-One launcher's source", host(&api));
                    return Ok((api, v, How::Rediscovered { note, confirm: false }));
                }
                Err(e) => {
                    if format!("{e:#}").contains("no package") {
                        reachable = Some(api.clone());
                    }
                    errors.push(format!("{} (discovered): {e:#}", host(&api)));
                }
            },
            Ok(_) => {}
            Err(e) => errors.push(format!("discovery: {e:#}")),
        }
    }
    // 3. The server answers but the package id changed: search for the game's base package.
    if let (Some(api), Some(prefix), Some(g)) = (&reachable, prefix, src.game.and_then(crate::aio::game_index)) {
        let url = format!("{}/workshop/query?keyword=~&game={g}&type=0&sortMode=0&page=0", api.trim_end_matches('/'));
        if let Ok(v) = t.get_json(&url, &workshop_headers()) {
            let candidate = v.as_array().into_iter().flatten().find_map(|e| {
                let id = crate::aio::field_str(e, "Guid")?;
                let game = crate::aio::field(e, "Game").and_then(Value::as_i64)?;
                (game == g && id.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) && !id.eq_ignore_ascii_case(guid)).then_some(id)
            });
            if let Some(id) = candidate {
                let entry = workshop_entry(t, api, &id)?;
                let name = crate::aio::field_str(&entry, "Name").unwrap_or_default();
                let note = format!("{guid} is gone; the closest package is {id} ({name})");
                return Ok((api.clone(), entry, How::Rediscovered { note, confirm: true }));
            }
        }
        errors.push(format!("no {prefix}… package for this game on {}", host(api)));
    }
    bail!("couldn't find {} ({}): {}", src.title, guid, errors.join("; "))
}

/// A link's name for matching and sorting: the URL's file name, else the link text.
fn link_name(l: &text::Link) -> String {
    let f = text::file_name(&l.url);
    if f.contains('.') { f } else if !l.text.is_empty() { l.text.clone() } else { f }
}

fn link_matches(pattern: &str, l: &text::Link) -> bool {
    glob(pattern, &l.url) || glob(pattern, &text::file_name(&l.url)) || (!l.text.is_empty() && glob(pattern, &l.text))
}

fn resolve_page(t: &dyn Transfer, page: &str, steps: &[String], pattern: &str) -> Result<(String, String, How)> {
    let mut url = page.to_string();
    let mut notes: Vec<String> = Vec::new();
    let mut html = t.get_text(&url, &[], super::MAX_TEXT_BYTES)?;
    for step in steps {
        let links = text::links(&html, &url);
        // The file itself may already be linked here (a step was dropped from the site).
        if links.iter().any(|l| link_matches(pattern, l)) {
            notes.push(format!("the file is linked before the {step} page now"));
            break;
        }
        let next = links.iter().find(|l| link_matches(step, l)).ok_or_else(|| {
            anyhow!("no link matching {step} on {} (the page changed?)", host(&url))
        })?;
        url = next.url.clone();
        html = t.get_text(&url, &[], super::MAX_TEXT_BYTES)?;
    }
    let links = text::links(&html, &url);
    let mut exact: Vec<&text::Link> = links.iter().filter(|l| link_matches(pattern, l)).collect();
    exact.sort_by(|a, b| natural_cmp(&link_name(b), &link_name(a)));
    if let Some(l) = exact.first() {
        let how = if notes.is_empty() { How::Listed } else { How::Rediscovered { note: notes.join("; "), confirm: false } };
        return Ok((l.url.clone(), link_name(l), how));
    }
    // Nothing matches: the closest link with the same kind of file.
    let ext = text::extension(pattern);
    let mut near: Vec<(&text::Link, f32)> = links
        .iter()
        .filter(|l| ext.as_deref().map(|e| text::extension(&link_name(l)).as_deref() == Some(e)).unwrap_or(true))
        .map(|l| (l, text::similarity(pattern, &link_name(l))))
        .filter(|(_, s)| *s >= 0.5)
        .collect();
    near.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| natural_cmp(&link_name(b.0), &link_name(a.0))));
    if let Some((l, _)) = near.first() {
        let name = link_name(l);
        let note = format!("no link matches {pattern} any more; the closest is {name}");
        return Ok((l.url.clone(), name, How::Rediscovered { note, confirm: true }));
    }
    bail!("no link matching {pattern} on {} (the page changed?)", host(&url))
}

fn resolve_github(t: &dyn Transfer, repo: &str, asset: &str) -> Result<(String, String, Option<u64>, How)> {
    if repo.split('/').count() != 2 || repo.chars().any(|c| !(c.is_ascii_alphanumeric() || "-_./".contains(c))) {
        bail!("{repo:?} isn't owner/repo");
    }
    let headers = [("Accept", "application/vnd.github+json".to_string()), ("X-GitHub-Api-Version", "2022-11-28".to_string())];
    let releases = t.get_json(&format!("https://api.github.com/repos/{repo}/releases?per_page=15"), &headers)?;
    let assets = |r: &Value| -> Vec<(String, String, Option<u64>)> {
        r.get("assets")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| {
                let name = a.get("name")?.as_str()?.to_string();
                let url = a.get("browser_download_url")?.as_str()?.to_string();
                is_https(&url).then(|| (name, url, a.get("size").and_then(Value::as_u64)))
            })
            .collect()
    };
    let list: Vec<&Value> = releases
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| !r.get("draft").and_then(Value::as_bool).unwrap_or(false) && !r.get("prerelease").and_then(Value::as_bool).unwrap_or(false))
        .collect();
    for r in list.iter().copied() {
        let mut found: Vec<_> = assets(r).into_iter().filter(|(n, _, _)| glob(asset, n)).collect();
        found.sort_by(|a, b| natural_cmp(&b.0, &a.0));
        if let Some((name, url, size)) = found.into_iter().next() {
            return Ok((url, name, size, How::Listed));
        }
    }
    if let Some(r) = list.first().copied() {
        let mut near: Vec<_> = assets(r).into_iter().map(|a| (text::similarity(asset, &a.0), a)).filter(|(s, _)| *s >= 0.5).collect();
        near.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((_, (name, url, size))) = near.into_iter().next() {
            let note = format!("no asset matches {asset} any more; the closest is {name}");
            return Ok((url, name, size, How::Rediscovered { note, confirm: true }));
        }
    }
    bail!("no release of {repo} has a file matching {asset}")
}

// ---------------------------------------------------------------- checking

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    /// Found where the list says, and the file answers.
    Ok,
    /// Found somewhere new by itself; downloads work.
    Moved,
    /// Only a guess was found; a download will ask first.
    NeedsConfirm,
    Broken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub title: String,
    pub state: State,
    pub detail: String,
}

/// Look every source up and touch the file it points at (headers only).
pub fn check_all(t: &dyn Transfer, list: &SourceList, learned: &mut Learned) -> Vec<Check> {
    list.sources.iter().map(|s| check_one(t, list, learned, s)).collect()
}

pub fn check_one(t: &dyn Transfer, list: &SourceList, learned: &mut Learned, s: &Source) -> Check {
    let mk = |state, detail: String| Check { id: s.id.clone(), title: s.title.clone(), state, detail };
    let r = match resolve(t, list, learned, s) {
        Ok(r) => r,
        Err(e) => return mk(State::Broken, format!("{e:#}")),
    };
    let (probe_url, what) = match &r.target {
        Target::File { url, name, .. } => (url.clone(), name.clone()),
        Target::Workshop { entry, .. } => {
            let files = crate::aio::field(entry, "Files").and_then(Value::as_array).cloned().unwrap_or_default();
            let total: u64 = files.iter().filter_map(|f| crate::aio::field(f, "Size").and_then(Value::as_u64)).sum();
            let first = files.iter().filter_map(|f| crate::aio::field_str(f, "Url")).find(|u| is_https(u));
            let Some(u) = first else { return mk(State::Broken, "the package lists no https files".into()) };
            let name = crate::aio::field_str(entry, "Name").unwrap_or_default();
            (u, format!("{name}: {} files, {:.1} GB", files.len(), total as f64 / 1e9))
        }
    };
    if let Err(e) = t.open(&probe_url, 0, &[]) {
        return mk(State::Broken, format!("{what}: the file doesn't answer: {e:#}"));
    }
    match r.how {
        How::Listed => mk(State::Ok, what),
        How::Rediscovered { note, confirm: false } => mk(State::Moved, format!("{what} ({note})")),
        How::Rediscovered { note, confirm: true } => mk(State::NeedsConfirm, format!("{what} ({note})")),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testnet::FakeNet;
    use super::*;
    use serde_json::json;

    fn list(extra: Value) -> SourceList {
        let mut v = json!({
            "schema": 1, "version": 3,
            "workshop": {"api": "https://old.example/api",
                         "discover": ["https://raw.example/Manager.cs"],
                         "discover_pattern": "WorkshopServerHost = \"", "api_suffix": "/api"},
            "sources": [
                {"id": "bfme1", "title": "BFME", "game": "bfme1", "kind": "workshop", "guid": "original-BFME1", "guid_prefix": "original-"}
            ]
        });
        if let Some(a) = extra.as_array() {
            v["sources"].as_array_mut().unwrap().extend(a.iter().cloned());
        }
        SourceList::parse(&v.to_string()).unwrap()
    }

    fn entry(guid: &str) -> String {
        json!({"Guid": guid, "Name": "Vanilla (1.03)", "Game": 0, "Type": 0,
               "Files": [{"Name": "lotrbfme.exe", "Url": "https://files.example/a", "Md5": "x", "Size": 5, "Language": "ALL"}]})
        .to_string()
    }

    #[test]
    fn bundled_list_is_valid() {
        let l = SourceList::bundled();
        for g in [GameId::Bfme1, GameId::Bfme2, GameId::Rotwk] {
            assert!(matches!(l.for_game(g).map(|s| &s.kind), Some(Kind::Workshop { .. })), "{g:?}");
        }
        assert!(l.workshop.api.starts_with("https://"));
    }

    #[test]
    fn rejects_bad_lists() {
        assert!(SourceList::parse(r#"{"schema": 2, "version": 1}"#).is_err());
        assert!(SourceList::parse(r#"{"schema": 1, "version": 1, "sources": [{"id": "a", "title": "A", "kind": "direct", "url": "http://x/y"}]}"#).is_err());
        assert!(SourceList::parse(r#"{"schema": 1, "version": 1, "sources": [{"id": "a", "title": "A", "kind": "direct", "url": "https://x/y"}, {"id": "a", "title": "B", "kind": "direct", "url": "https://x/z"}]}"#).is_err());
    }

    #[test]
    fn workshop_where_listed() {
        let l = list(json!([]));
        let net = FakeNet::with(&[("https://old.example/api/workshop/download?guid=original-BFME1", entry("original-BFME1").as_bytes())]);
        let mut learned = Learned::default();
        let r = resolve(&net, &l, &mut learned, l.get("bfme1").unwrap()).unwrap();
        assert_eq!(r.how, How::Listed);
        assert!(matches!(r.target, Target::Workshop { ref api, .. } if api == "https://old.example/api"));
    }

    #[test]
    fn workshop_moved_is_found_and_remembered() {
        let l = list(json!([]));
        let manager = "public static string WorkshopServerHost = \"https://new.example\";";
        let net = FakeNet::with(&[
            ("https://raw.example/Manager.cs", manager.as_bytes()),
            ("https://new.example/api/workshop/download?guid=original-BFME1", entry("original-BFME1").as_bytes()),
        ]);
        let mut learned = Learned::default();
        let r = resolve(&net, &l, &mut learned, l.get("bfme1").unwrap()).unwrap();
        assert!(matches!(r.how, How::Rediscovered { confirm: false, .. }));
        assert_eq!(learned.workshop_api.as_deref(), Some("https://new.example/api"));
        // Next time it goes straight there.
        let before = net.requests().len();
        resolve(&net, &l, &mut learned, l.get("bfme1").unwrap()).unwrap();
        assert_eq!(net.requests().len(), before + 1);
    }

    #[test]
    fn renamed_package_is_a_guess() {
        let l = list(json!([]));
        let q = json!([{"Guid": "original-BFME1-v2", "Game": 0, "Type": 0}, {"Guid": "edain", "Game": 0, "Type": 1}]).to_string();
        let net = FakeNet::with(&[
            ("https://old.example/api/workshop/download?guid=original-BFME1", br#"{"Guid": null}"#),
            ("https://old.example/api/workshop/query?keyword=~&game=0&type=0&sortMode=0&page=0", q.as_bytes()),
            ("https://old.example/api/workshop/download?guid=original-BFME1-v2", entry("original-BFME1-v2").as_bytes()),
        ]);
        let mut learned = Learned::default();
        let r = resolve(&net, &l, &mut learned, l.get("bfme1").unwrap()).unwrap();
        assert!(r.how.needs_confirm());
    }

    #[test]
    fn workshop_gone_is_an_error() {
        let l = list(json!([]));
        let net = FakeNet::default();
        let mut learned = Learned::default();
        let e = resolve(&net, &l, &mut learned, l.get("bfme1").unwrap()).unwrap_err();
        assert!(format!("{e:#}").contains("couldn't find BFME"));
    }

    #[test]
    fn page_newest_match_through_steps() {
        let l = list(json!([{"id": "aotr", "title": "AotR", "kind": "page", "page": "https://mods.example/aotr/downloads",
                              "steps": ["*/downloads/start/*"], "pattern": "AotR_Standalone_*_Installer.zip"}]));
        let page1 = r#"<a href="/downloads/start/2">9.2</a> <a href="/downloads/start/3">9.3</a>"#;
        let page2 = r#"<a href="https://cdn.example/m/AotR_Standalone_9.2_Installer.zip">a</a>
                       <a href="https://cdn.example/m/AotR_Standalone_9.10_Installer.zip">b</a>"#;
        let net = FakeNet::with(&[("https://mods.example/aotr/downloads", page1.as_bytes()), ("https://mods.example/downloads/start/2", page2.as_bytes())]);
        let mut learned = Learned::default();
        let r = resolve(&net, &l, &mut learned, l.get("aotr").unwrap()).unwrap();
        assert_eq!(r.how, How::Listed);
        assert!(matches!(r.target, Target::File { ref name, .. } if name == "AotR_Standalone_9.10_Installer.zip"));
    }

    #[test]
    fn page_renamed_file_is_a_guess() {
        let l = list(json!([{"id": "aotr", "title": "AotR", "kind": "page", "page": "https://mods.example/aotr", "pattern": "AotR_Standalone_*_Installer.zip"}]));
        let page = r#"<a href="https://cdn.example/AotR-Standalone-Setup-9.4.zip">x</a><a href="https://cdn.example/edain.zip">y</a>"#;
        let net = FakeNet::with(&[("https://mods.example/aotr", page.as_bytes())]);
        let mut learned = Learned::default();
        let r = resolve(&net, &l, &mut learned, l.get("aotr").unwrap()).unwrap();
        assert!(r.how.needs_confirm());
        assert!(matches!(r.target, Target::File { ref name, .. } if name == "AotR-Standalone-Setup-9.4.zip"));
        // Nothing close: an error, not a random file.
        net.put("https://mods.example/aotr", br#"<a href="https://cdn.example/edain.zip">y</a>"#);
        assert!(resolve(&net, &l, &mut learned, l.get("aotr").unwrap()).is_err());
    }

    #[test]
    fn github_latest_matching_asset() {
        let l = list(json!([{"id": "xl", "title": "xliveless", "kind": "github", "repo": "someone/xliveless", "asset": "xlive*.zip"}]));
        let rel = json!([
            {"draft": false, "prerelease": true, "assets": [{"name": "xlive-0.9.zip", "browser_download_url": "https://gh.example/0.9", "size": 1}]},
            {"draft": false, "prerelease": false, "assets": [{"name": "xlive-0.8.zip", "browser_download_url": "https://gh.example/0.8", "size": 2}]}
        ]).to_string();
        let net = FakeNet::with(&[("https://api.github.com/repos/someone/xliveless/releases?per_page=15", rel.as_bytes())]);
        let mut learned = Learned::default();
        let r = resolve(&net, &l, &mut learned, l.get("xl").unwrap()).unwrap();
        assert!(matches!(r.target, Target::File { ref url, size: Some(2), .. } if url == "https://gh.example/0.8"));
    }

    #[test]
    fn checks_report_each_state() {
        let l = list(json!([{"id": "d", "title": "Direct", "kind": "direct", "url": "https://files.example/gone"}]));
        let net = FakeNet::with(&[
            ("https://old.example/api/workshop/download?guid=original-BFME1", entry("original-BFME1").as_bytes()),
            ("https://files.example/a", b"hello"),
        ]);
        let mut learned = Learned::default();
        let c = check_all(&net, &l, &mut learned);
        assert_eq!(c[0].state, State::Ok);
        assert_eq!(c[1].state, State::Broken);
    }

    #[test]
    fn newer_downloaded_list_wins_without_a_login() {
        let t = tempfile::tempdir().unwrap();
        let store = Store::new(&t.path().join("cfg/config.json"), &t.path().join("data"));
        assert_eq!(store.list(), SourceList::bundled());
        let mut newer: Value = serde_json::from_str(BUNDLED).unwrap();
        newer["version"] = json!(SourceList::bundled().version + 5);
        let net = FakeNet::with(&[("https://raw.githubusercontent.com/Dragongamer9/beacon-private/main/sources.json", newer.to_string().as_bytes())]);
        let u = store.update(&net, true).unwrap();
        assert_eq!(u.to, SourceList::bundled().version + 5);
        assert_eq!(store.list().version, u.to);
        // Recently checked: not again unless forced.
        assert!(store.update(&net, false).unwrap().skipped.is_some());
        // An old token in private.json is dropped on the next save.
        std::fs::write(&store.settings_file, r#"{"sources_repo": "a/b", "github_token": "ghp_old"}"#).unwrap();
        let s = store.settings();
        assert_eq!(s.sources_repo, "a/b");
        store.save_settings(&s).unwrap();
        assert!(!std::fs::read_to_string(&store.settings_file).unwrap().contains("ghp_old"));
    }

    #[test]
    fn repos() {
        assert_eq!(check_repo(" https://github.com/Dragongamer9/beacon/ ").unwrap(), "Dragongamer9/beacon");
        assert_eq!(check_repo("a/b.git").unwrap(), "a/b");
        assert!(check_repo("a").is_err());
        assert!(check_repo("a/../b").is_err());
        assert!(check_repo("a/b c").is_err());
    }
}
