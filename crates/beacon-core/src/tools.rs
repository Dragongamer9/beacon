//! Compatibility tools: download, update and remove Proton, Wine, DXVK and VKD3D-Proton builds, for Beacon
//! itself and for the other launchers on this PC (Steam, Lutris, Heroic, Bottles, native or Flatpak). The
//! feature set follows ProtonPlus (https://github.com/Vysp3r/ProtonPlus, GPL-3.0): which tools, where each
//! launcher looks for them, a "Latest" install that updates itself. See THIRD-PARTY-NOTICES.md.
//!
//! Only the projects' own GitHub releases are used (api.github.com, then the release asset). A download is
//! checked against the SHA-256/SHA-512 checksum the release publishes next to it when there is one (GE-Proton,
//! UMU-Proton, Proton-CachyOS, Kron4ek's Wine…); a mismatch throws the download away. Archives are unpacked
//! with the system's `tar` (or `bsdtar`) into a staging folder next to the target, then moved into place, so a
//! failed install never leaves half a tool behind.

use crate::workshop::Http;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Proton,
    Wine,
    Dxvk,
    Vkd3d,
    /// umu-launcher itself (Beacon only).
    Umu,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// GitHub owner/repo.
    pub repo: &'static str,
    pub blurb: &'static str,
    /// The project no longer makes releases (its last one still installs).
    pub archived: bool,
    /// Name of the folder a "Latest" install goes in (`<base>-Latest`).
    #[serde(skip)]
    base: &'static str,
    /// The release asset: its name (lowercase) contains all of `want`, none of `avoid`, and is a tarball.
    #[serde(skip)]
    want: &'static [&'static str],
    #[serde(skip)]
    avoid: &'static [&'static str],
    /// Folder names of copies installed without Beacon (lowercase prefixes).
    #[serde(skip)]
    names: &'static [&'static str],
}

const fn tool(
    id: &'static str,
    name: &'static str,
    kind: Kind,
    repo: &'static str,
    blurb: &'static str,
    archived: bool,
    base: &'static str,
    want: &'static [&'static str],
    avoid: &'static [&'static str],
    names: &'static [&'static str],
) -> Tool {
    Tool { id, name, kind, repo, blurb, archived, base, want, avoid, names }
}

pub const TOOLS: &[Tool] = &[
    tool("ge-proton", "GE-Proton", Kind::Proton, "GloriousEggroll/proton-ge-custom", "Proton with extra game fixes, media codecs and FSR. The usual pick.", false, "GE-Proton", &["ge-proton"], &[], &["ge-proton"]),
    tool("umu-proton", "UMU-Proton", Kind::Proton, "Open-Wine-Components/umu-proton", "Valve's Proton as umu-launcher ships it, with protonfixes.", false, "UMU-Proton", &["umu-proton"], &[], &["umu-proton"]),
    tool("proton-cachyos", "Proton-CachyOS", Kind::Proton, "CachyOS/proton-cachyos", "CachyOS's Proton build with newer Wine and performance patches.", false, "Proton-CachyOS", &["x86_64"], &["x86_64_v"], &["proton-cachyos"]),
    tool("proton-em", "Proton-EM", Kind::Proton, "Etaash-mathamsetty/Proton", "Proton with experimental fixes (NTSync, Wayland, HDR).", false, "Proton-EM", &["proton-em"], &[], &["proton-em"]),
    tool("proton-sarek", "Proton-Sarek", Kind::Proton, "pythonlover02/Proton-Sarek", "Proton tuned for older GPUs (DXVK 1.10 for pre-Vulkan-1.3 cards).", false, "Proton-Sarek", &["proton-sarek"], &["async"], &["proton-sarek"]),
    tool("wine-ge", "Wine-GE", Kind::Wine, "GloriousEggroll/wine-ge-custom", "GloriousEggroll's Wine for Lutris and Heroic (no new releases; GE-Proton replaced it).", true, "Wine-GE", &["wine-lutris"], &[], &["wine-lutris-ge", "lutris-ge"]),
    tool("kron4ek-wine", "Wine (Kron4ek)", Kind::Wine, "Kron4ek/Wine-Builds", "Plain upstream Wine, portable builds.", false, "Wine-Kron4ek", &["-amd64.tar"], &["staging", "tkg", "proton", "wow64"], &[]),
    tool("kron4ek-wine-staging", "Wine Staging (Kron4ek)", Kind::Wine, "Kron4ek/Wine-Builds", "Wine with the Staging patch set, portable builds.", false, "Wine-Kron4ek-Staging", &["-staging-amd64.tar"], &["tkg", "wow64"], &[]),
    tool("dxvk", "DXVK", Kind::Dxvk, "doitsujin/dxvk", "Direct3D 8/9/10/11 on Vulkan, for Wine runners (Proton has its own).", false, "DXVK", &["dxvk-"], &["native"], &["dxvk-"]),
    tool("dxvk-async", "DXVK-async", Kind::Dxvk, "Sporif/dxvk-async", "DXVK with asynchronous shader compiling (no new releases).", true, "DXVK-async", &["dxvk-async"], &[], &["dxvk-async"]),
    tool("vkd3d-proton", "VKD3D-Proton", Kind::Vkd3d, "HansKristian-Work/vkd3d-proton", "Direct3D 12 on Vulkan, for Wine runners (Proton has its own).", false, "VKD3D-Proton", &["vkd3d-proton-"], &[], &["vkd3d-proton"]),
    tool("umu-launcher", "umu-launcher", Kind::Umu, "Open-Wine-Components/umu-launcher", "Runs Proton outside Steam with the Steam Linux Runtime and protonfixes. Beacon uses it when it's there.", false, "umu-launcher", &["zipapp"], &[], &[]),
];

pub fn find(id: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.id == id)
}

/// Where a tool can go.
#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub id: &'static str,
    pub name: &'static str,
    /// The launcher is on this PC (Beacon always is).
    pub detected: bool,
}

const TARGETS: &[(&str, &str)] = &[
    ("beacon", "Beacon"),
    ("steam", "Steam"),
    ("steam-flatpak", "Steam (Flatpak)"),
    ("lutris", "Lutris"),
    ("lutris-flatpak", "Lutris (Flatpak)"),
    ("heroic", "Heroic"),
    ("heroic-flatpak", "Heroic (Flatpak)"),
    ("bottles", "Bottles"),
    ("bottles-flatpak", "Bottles (Flatpak)"),
];

fn steam_root(home: &Path) -> PathBuf {
    let r = home.join(".steam/root");
    if r.is_dir() {
        r
    } else {
        home.join(".local/share/Steam")
    }
}

fn target_root(home: &Path, id: &str) -> Option<PathBuf> {
    let fp = |app: &str| home.join(".var/app").join(app);
    Some(match id {
        "steam" => steam_root(home),
        "steam-flatpak" => fp("com.valvesoftware.Steam"),
        "lutris" => home.join(".local/share/lutris"),
        "lutris-flatpak" => fp("net.lutris.Lutris"),
        "heroic" => home.join(".config/heroic"),
        "heroic-flatpak" => fp("com.heroicgameslauncher.hgl"),
        "bottles" => home.join(".local/share/bottles"),
        "bottles-flatpak" => fp("com.usebottles.bottles"),
        _ => return None,
    })
}

/// The folder a tool of `kind` goes in for target `id`, or None when that launcher doesn't take that kind.
pub fn target_dir(home: &Path, data_dir: &Path, id: &str, kind: Kind) -> Option<PathBuf> {
    use Kind::*;
    let c = |s: &str| data_dir.join("compat").join(s);
    let fp = |app: &str| home.join(".var/app").join(app);
    let lutris = |base: PathBuf| match kind {
        Proton | Wine => Some(base.join("runners/wine")),
        Dxvk => Some(base.join("runtime/dxvk")),
        Vkd3d => Some(base.join("runtime/vkd3d")),
        Umu => None,
    };
    let heroic = |base: PathBuf| match kind {
        Proton => Some(base.join("tools/proton")),
        Wine => Some(base.join("tools/wine")),
        _ => None,
    };
    let bottles = |base: PathBuf| match kind {
        Proton | Wine => Some(base.join("runners")),
        Dxvk => Some(base.join("dxvk")),
        Vkd3d => Some(base.join("vkd3d")),
        Umu => None,
    };
    match id {
        "beacon" => Some(match kind {
            Proton => c("proton"),
            Wine => c("wine"),
            Dxvk => c("dxvk"),
            Vkd3d => c("vkd3d"),
            Umu => c("umu-launcher"),
        }),
        "steam" => (kind == Proton).then(|| steam_root(home).join("compatibilitytools.d")),
        "steam-flatpak" => (kind == Proton).then(|| fp("com.valvesoftware.Steam").join("data/Steam/compatibilitytools.d")),
        "lutris" => lutris(home.join(".local/share/lutris")),
        "lutris-flatpak" => lutris(fp("net.lutris.Lutris").join("data/lutris")),
        "heroic" => heroic(home.join(".config/heroic")),
        "heroic-flatpak" => heroic(fp("com.heroicgameslauncher.hgl").join("config/heroic")),
        "bottles" => bottles(home.join(".local/share/bottles")),
        "bottles-flatpak" => bottles(fp("com.usebottles.bottles").join("data/bottles")),
        _ => None,
    }
}

pub fn targets(home: &Path) -> Vec<Target> {
    TARGETS
        .iter()
        .map(|(id, name)| Target { id, name, detected: *id == "beacon" || target_root(home, id).map(|p| p.is_dir()).unwrap_or(false) })
        .collect()
}

pub fn valid_target(id: &str) -> bool {
    TARGETS.iter().any(|(t, _)| *t == id)
}

// ---------------------------------------------------------------- releases

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub published: String,
    pub prerelease: bool,
    pub asset: String,
    pub url: String,
    pub size: u64,
    /// The checksum file the release publishes for the asset, if any.
    pub checksum_url: Option<String>,
}

fn is_tarball(n: &str) -> bool {
    [".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2", ".tgz", ".tar"].iter().any(|e| n.ends_with(e))
}

/// The tool's asset and its checksum file in one GitHub release (the API's JSON).
pub fn parse_release(t: &Tool, r: &Value) -> Option<Release> {
    if r["draft"] == true {
        return None;
    }
    let assets = r["assets"].as_array()?;
    let pick = assets.iter().find(|a| {
        let n = a["name"].as_str().unwrap_or("").to_ascii_lowercase();
        is_tarball(&n) && t.want.iter().all(|w| n.contains(w)) && !t.avoid.iter().any(|w| n.contains(w))
    })?;
    let asset = pick["name"].as_str()?.to_string();
    let url = pick["browser_download_url"].as_str().filter(|u| u.starts_with("https://github.com/"))?.to_string();
    let stem = asset.trim_end_matches(".tar.gz").trim_end_matches(".tar.xz").trim_end_matches(".tar.zst").trim_end_matches(".tar.bz2").trim_end_matches(".tgz").trim_end_matches(".tar");
    let sums = [format!("{asset}.sha512sum"), format!("{asset}.sha256sum"), format!("{stem}.sha512sum"), format!("{stem}.sha256sum"), format!("{asset}.sha512"), format!("{asset}.sha256")];
    let lists = ["sha512sums.txt", "sha256sums.txt", "sha512sums", "sha256sums", "checksums.txt", "sha256sum.txt", "sha512sum.txt"];
    let checksum_url = assets
        .iter()
        .find(|a| sums.iter().any(|s| a["name"].as_str() == Some(s.as_str())))
        .or_else(|| assets.iter().find(|a| lists.iter().any(|l| a["name"].as_str().map(|n| n.eq_ignore_ascii_case(l)).unwrap_or(false))))
        .and_then(|a| a["browser_download_url"].as_str())
        .filter(|u| u.starts_with("https://github.com/"))
        .map(str::to_string);
    Some(Release {
        tag: r["tag_name"].as_str()?.to_string(),
        name: r["name"].as_str().filter(|s| !s.trim().is_empty()).or(r["tag_name"].as_str()).unwrap_or("").to_string(),
        published: r["published_at"].as_str().unwrap_or("").to_string(),
        prerelease: r["prerelease"] == true,
        asset,
        url,
        size: pick["size"].as_u64().unwrap_or(0),
        checksum_url,
    })
}

fn cache_file(data_dir: &Path, t: &Tool) -> PathBuf {
    data_dir.join("cache/tools").join(format!("{}.json", t.id))
}

/// The tool's releases, newest first (cached for 30 minutes: GitHub allows 60 unauthenticated calls an hour).
pub fn releases(http: &dyn Http, data_dir: &Path, t: &Tool, fresh: bool) -> Result<Vec<Release>> {
    let cache = cache_file(data_dir, t);
    if !fresh {
        if let Ok(m) = std::fs::metadata(&cache) {
            let young = m.modified().ok().and_then(|t| t.elapsed().ok()).map(|e| e.as_secs() < 1800).unwrap_or(false);
            if young {
                if let Some(v) = std::fs::read_to_string(&cache).ok().and_then(|s| serde_json::from_str::<Vec<Release>>(&s).ok()) {
                    return Ok(v);
                }
            }
        }
    }
    let url = format!("https://api.github.com/repos/{}/releases?per_page=40", t.repo);
    let v = http.get_github(&url).with_context(|| format!("asking GitHub for {}'s releases", t.name))?;
    let list: Vec<Release> = v.as_array().ok_or_else(|| anyhow!("GitHub's answer isn't a list of releases"))?.iter().filter_map(|r| parse_release(t, r)).collect();
    if let Some(d) = cache.parent() {
        let _ = std::fs::create_dir_all(d);
        let _ = std::fs::write(&cache, serde_json::to_string(&list).unwrap_or_default());
    }
    Ok(list)
}

/// The releases last read from GitHub, however old (for showing the list offline).
pub fn cached_releases(data_dir: &Path, t: &Tool) -> Option<Vec<Release>> {
    std::fs::read_to_string(cache_file(data_dir, t)).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// Every tool's state in one target, with the newest release when it can be had (`online`: ask GitHub when the
/// cache is old). Errors (GitHub unreachable, rate limit) are returned beside the list.
pub fn overview(http: &dyn Http, home: &Path, data_dir: &Path, target: &str, online: bool) -> (Vec<ToolState>, Vec<String>) {
    let mut errors = Vec::new();
    let list = TOOLS
        .iter()
        .filter(|t| target_dir(home, data_dir, target, t.kind).is_some())
        .map(|t| {
            let rels = if online {
                match releases(http, data_dir, t, false) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        errors.push(format!("{}: {e:#}", t.name));
                        cached_releases(data_dir, t)
                    }
                }
            } else {
                cached_releases(data_dir, t)
            };
            let n = rels.as_deref().and_then(newest).map(|r| r.tag.clone());
            state(home, data_dir, t, target, n.as_deref())
        })
        .collect();
    errors.dedup();
    (list, errors)
}

/// The newest release that isn't a pre-release (or the newest one, if all are).
pub fn newest(list: &[Release]) -> Option<&Release> {
    list.iter().find(|r| !r.prerelease).or_else(|| list.first())
}

// ---------------------------------------------------------------- what's installed

const MARKER: &str = ".beacon-tool.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub tool: String,
    pub tag: String,
    pub latest: bool,
    #[serde(default)]
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Installed {
    /// The folder's name in the target folder.
    pub name: String,
    pub path: PathBuf,
    /// The tool it is (Beacon's own installs say so; others are recognized by name), if known.
    pub tool: Option<String>,
    pub tag: Option<String>,
    pub latest: bool,
    pub by_beacon: bool,
}

fn guess_tool(name: &str, kind: Kind) -> Option<&'static Tool> {
    let n = name.to_ascii_lowercase();
    if kind == Kind::Wine && n.starts_with("wine-") && n.contains("amd64") && !n.contains("lutris") {
        return find(if n.contains("staging") { "kron4ek-wine-staging" } else { "kron4ek-wine" });
    }
    // Longer prefixes first: "dxvk-async" before "dxvk-".
    let mut all: Vec<&Tool> = TOOLS.iter().filter(|t| t.kind == kind || (kind == Kind::Wine && t.kind == Kind::Proton)).collect();
    all.sort_by_key(|t| std::cmp::Reverse(t.names.iter().map(|p| p.len()).max().unwrap_or(0)));
    all.into_iter().find(|t| t.names.iter().any(|p| n.starts_with(p)))
}

/// Everything in a target folder for a kind of tool (Proton targets that also hold Wine show both).
pub fn installed_in(dir: &Path, kind: Kind) -> Vec<Installed> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out: Vec<Installed> = rd
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let path = e.path();
            let marker: Option<Marker> = std::fs::read_to_string(path.join(MARKER)).ok().and_then(|s| serde_json::from_str(&s).ok());
            let tool = marker.as_ref().map(|m| m.tool.clone()).or_else(|| guess_tool(&name, kind).map(|t| t.id.to_string()));
            Some(Installed {
                tag: marker.as_ref().map(|m| m.tag.clone()),
                latest: marker.as_ref().map(|m| m.latest).unwrap_or(false),
                by_beacon: marker.is_some(),
                tool,
                name,
                path,
            })
        })
        .collect();
    out.sort_by(|a, b| crate::runner::natural_cmp(&b.name, &a.name));
    out
}

fn norm(s: &str) -> String {
    s.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Whether an installed copy is release `tag`.
pub fn is_release(i: &Installed, tag: &str) -> bool {
    if let Some(t) = &i.tag {
        return t == tag;
    }
    let t = norm(tag.trim_start_matches(['v', 'V']));
    let t = t.trim_start_matches("cachyos").to_string();
    !t.is_empty() && norm(&i.name).contains(&t)
}

/// One tool's state in one target: what's installed and what the newest release is.
#[derive(Debug, Clone, Serialize)]
pub struct ToolState {
    #[serde(flatten)]
    pub tool: Tool,
    pub target: String,
    /// The target takes this kind of tool.
    pub supported: bool,
    pub dir: Option<PathBuf>,
    pub installed: Vec<Installed>,
    /// Newest release, when it was asked for (None offline).
    pub newest: Option<String>,
    /// The newest release is installed (or a "Latest" install is on it).
    pub up_to_date: bool,
    pub has_latest: bool,
}

pub fn state(home: &Path, data_dir: &Path, t: &Tool, target: &str, newest_tag: Option<&str>) -> ToolState {
    let dir = target_dir(home, data_dir, target, t.kind);
    let installed: Vec<Installed> = dir.as_deref().map(|d| installed_in(d, t.kind)).unwrap_or_default().into_iter().filter(|i| i.tool.as_deref() == Some(t.id)).collect();
    let up_to_date = newest_tag.map(|n| installed.iter().any(|i| is_release(i, n))).unwrap_or(false);
    ToolState {
        tool: t.clone(),
        target: target.to_string(),
        supported: dir.is_some(),
        has_latest: installed.iter().any(|i| i.latest),
        dir,
        installed,
        newest: newest_tag.map(str::to_string),
        up_to_date,
    }
}

/// The newest installed copy of a kind of tool in Beacon's own folder (DXVK and VKD3D-Proton for Wine runners).
pub fn beacon_newest(data_dir: &Path, kind: Kind) -> Option<PathBuf> {
    let dir = target_dir(Path::new("/"), data_dir, "beacon", kind)?;
    let list = installed_in(&dir, kind);
    list.iter().find(|i| i.latest).or_else(|| list.first()).map(|i| i.path.clone())
}

// ---------------------------------------------------------------- install, remove, update

/// Writes the download and hashes it as it goes.
struct Hashing<W: Write> {
    out: W,
    sha256: sha2::Sha256,
    sha512: sha2::Sha512,
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use sha2::Digest;
        let n = self.out.write(buf)?;
        self.sha256.update(&buf[..n]);
        self.sha512.update(&buf[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

/// The hash for `asset` in a checksum file: "<hex>  <name>" lines (or a single bare hash).
pub fn checksum_for(text: &str, asset: &str) -> Option<String> {
    let hexes: Vec<(String, Option<String>)> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let h = it.next()?.to_ascii_lowercase();
            if !(h.len() == 64 || h.len() == 128) || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            Some((h, it.next().map(|n| n.trim_start_matches('*').rsplit('/').next().unwrap_or("").to_string())))
        })
        .collect();
    hexes.iter().find(|(_, n)| n.as_deref() == Some(asset)).map(|(h, _)| h.clone()).or_else(|| (hexes.len() == 1 && hexes[0].1.is_none()).then(|| hexes[0].0.clone()))
}

#[derive(Debug, Clone, Serialize)]
pub struct Done {
    pub tool: String,
    pub target: String,
    pub tag: String,
    pub path: PathBuf,
    pub latest: bool,
    pub verified: bool,
    pub bytes: u64,
}

fn untar(archive: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut tried = Vec::new();
    for bin in ["tar", "bsdtar"] {
        match std::process::Command::new(bin).arg("-xf").arg(archive).arg("-C").arg(dest).output() {
            Ok(o) if o.status.success() => return Ok(()),
            Ok(o) => tried.push(format!("{bin}: {}", String::from_utf8_lossy(&o.stderr).trim().chars().take(300).collect::<String>())),
            Err(_) => continue,
        }
    }
    if tried.is_empty() {
        bail!("no tar found to unpack {}", archive.display());
    }
    bail!("couldn't unpack {}: {}", archive.display(), tried.join("; "))
}

fn safe_child(dir: &Path, name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.starts_with('.') || name.contains('/') || name.contains('\\') || name.contains('\0') {
        bail!("\"{name}\" isn't a tool folder");
    }
    Ok(dir.join(name))
}

/// Steam names tools by compatibilitytool.vdf; a "Latest" copy gets its own name there, or Steam would show two
/// tools with the same name (or hide one).
pub fn rename_vdf(text: &str, internal: &str, display: &str) -> String {
    let Some(ct) = text.find("\"compat_tools\"") else { return text.to_string() };
    let after = ct + "\"compat_tools\"".len();
    let mut out = text.to_string();
    // The first quoted string after compat_tools' opening brace is the tool's internal name.
    if let Some(b) = out[after..].find('{').map(|i| after + i + 1) {
        if let Some(q1) = out[b..].find('"').map(|i| b + i) {
            if let Some(q2) = out[q1 + 1..].find('"').map(|i| q1 + 1 + i) {
                out.replace_range(q1 + 1..q2, internal);
            }
        }
    }
    if let Some(d) = out.find("\"display_name\"") {
        let s = d + "\"display_name\"".len();
        if let Some(q1) = out[s..].find('"').map(|i| s + i) {
            if let Some(q2) = out[q1 + 1..].find('"').map(|i| q1 + 1 + i) {
                out.replace_range(q1 + 1..q2, display);
            }
        }
    }
    out
}

/// Install a release (`tag`, or the newest when None) of `t` into `target`. `latest`: as `<base>-Latest`, kept
/// on the newest release by [`update_latest`].
pub fn install(
    http: &dyn Http,
    home: &Path,
    data_dir: &Path,
    t: &Tool,
    target: &str,
    tag: Option<&str>,
    latest: bool,
    progress: &mut dyn FnMut(Option<u8>, &str),
) -> Result<Done> {
    let dir = target_dir(home, data_dir, target, t.kind).ok_or_else(|| anyhow!("{} doesn't take {} builds", target, t.name))?;
    progress(Some(1), &format!("Asking GitHub for {} releases", t.name));
    let list = releases(http, data_dir, t, tag.is_none())?;
    let rel = match tag {
        Some(tag) => list.iter().find(|r| r.tag == tag).ok_or_else(|| anyhow!("{} has no release {tag} with a Linux build", t.name))?,
        None => newest(&list).ok_or_else(|| anyhow!("{} has no release with a Linux build", t.name))?,
    }
    .clone();
    let tmp = data_dir.join("tmp/tools");
    std::fs::create_dir_all(&tmp)?;
    let file = safe_child(&tmp, &rel.asset)?;
    let result = (|| -> Result<Done> {
        let mb = |b: u64| b / 1_048_576;
        let total = rel.size;
        let f = std::fs::File::create(&file).with_context(|| format!("creating {}", file.display()))?;
        use sha2::Digest;
        let mut out = Hashing { out: std::io::BufWriter::new(f), sha256: sha2::Sha256::new(), sha512: sha2::Sha512::new() };
        let max = if total > 0 { total + 1024 * 1024 } else { 4 << 30 };
        let mut last = 0u64;
        let bytes = http.download(&rel.url, &mut out, max, &mut |n: u64| {
            if n - last >= 4 * 1_048_576 || n == total {
                last = n;
                let pct = if total > 0 { Some((5 + n * 80 / total).min(85) as u8) } else { None };
                progress(pct, &format!("Downloading {} {} — {} of {} MB", t.name, rel.tag, mb(n), mb(total)));
            }
        })?;
        out.flush()?;
        let sha256: String = out.sha256.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let sha512: String = out.sha512.finalize().iter().map(|b| format!("{b:02x}")).collect();
        drop(out.out);
        let mut verified = false;
        if let Some(curl) = &rel.checksum_url {
            progress(Some(86), "Checking the download against the release's checksum");
            let text = http.get_text(curl, 1 << 20).context("reading the release's checksum")?;
            let want = checksum_for(&text, &rel.asset).ok_or_else(|| anyhow!("the release's checksum file doesn't list {}", rel.asset))?;
            let got = if want.len() == 128 { &sha512 } else { &sha256 };
            if *got != want {
                bail!("{} doesn't match the checksum its release publishes; it was thrown away. Try again later.", rel.asset);
            }
            verified = true;
        }
        progress(Some(88), &format!("Unpacking {}", rel.asset));
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let staging = dir.join(format!(".beacon-staging-{}-{}", t.id, std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        let unpacked = (|| -> Result<PathBuf> {
            untar(&file, &staging)?;
            let entries: Vec<PathBuf> = std::fs::read_dir(&staging)?.flatten().map(|e| e.path()).collect();
            Ok(if entries.len() == 1 && entries[0].is_dir() { entries[0].clone() } else { staging.clone() })
        })();
        let src = match unpacked {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let name = if latest {
            format!("{}-Latest", t.base)
        } else if src != staging {
            src.file_name().unwrap().to_string_lossy().to_string()
        } else {
            rel.tag.clone()
        };
        let dest = safe_child(&dir, &name)?;
        let marker = Marker { tool: t.id.to_string(), tag: rel.tag.clone(), latest, verified };
        std::fs::write(src.join(MARKER), serde_json::to_string_pretty(&marker)?)?;
        if latest && target.starts_with("steam") {
            let vdf = src.join("compatibilitytool.vdf");
            if let Ok(text) = std::fs::read_to_string(&vdf) {
                let _ = std::fs::write(&vdf, rename_vdf(&text, &name, &format!("{} Latest ({})", t.name, rel.tag)));
            }
        }
        progress(Some(96), &format!("Putting {name} in place"));
        let old = dir.join(format!(".beacon-old-{}-{}", t.id, std::process::id()));
        let had = dest.exists();
        if had {
            std::fs::rename(&dest, &old).with_context(|| format!("moving the old {name} aside"))?;
        }
        if let Err(e) = std::fs::rename(&src, &dest) {
            if had {
                let _ = std::fs::rename(&old, &dest);
            }
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e).with_context(|| format!("moving {name} into {}", dir.display()));
        }
        let _ = std::fs::remove_dir_all(&old);
        let _ = std::fs::remove_dir_all(&staging);
        Ok(Done { tool: t.id.to_string(), target: target.to_string(), tag: rel.tag.clone(), path: dest, latest, verified, bytes })
    })();
    let _ = std::fs::remove_file(&file);
    let done = result?;
    progress(Some(100), &format!("{} {} installed{}", t.name, done.tag, if done.verified { " (checksum verified)" } else { "" }));
    Ok(done)
}

/// Remove an installed copy (a folder directly in the target's folder for that kind of tool).
pub fn remove(home: &Path, data_dir: &Path, target: &str, kind: Kind, name: &str) -> Result<PathBuf> {
    let dir = target_dir(home, data_dir, target, kind).ok_or_else(|| anyhow!("{target} doesn't take that kind of tool"))?;
    let p = safe_child(&dir, name)?;
    let meta = std::fs::symlink_metadata(&p).map_err(|_| anyhow!("{name} isn't installed in {}", dir.display()))?;
    if meta.file_type().is_symlink() {
        std::fs::remove_file(&p)?;
    } else if meta.is_dir() {
        std::fs::remove_dir_all(&p).with_context(|| format!("removing {}", p.display()))?;
    } else {
        bail!("{name} isn't a tool folder");
    }
    Ok(p)
}

/// Bring every "Latest" install (in every target) up to the tool's newest release. Returns what was updated.
pub fn update_latest(http: &dyn Http, home: &Path, data_dir: &Path, progress: &mut dyn FnMut(Option<u8>, &str)) -> Result<Vec<Done>> {
    let mut done = Vec::new();
    let mut errors = Vec::new();
    for target in targets(home).into_iter().filter(|t| t.detected) {
        for t in TOOLS {
            let Some(dir) = target_dir(home, data_dir, target.id, t.kind) else { continue };
            let lat: Vec<Installed> = installed_in(&dir, t.kind).into_iter().filter(|i| i.latest && i.tool.as_deref() == Some(t.id)).collect();
            if lat.is_empty() {
                continue;
            }
            progress(None, &format!("Checking {} for {}", t.name, target.name));
            let list = match releases(http, data_dir, t, false) {
                Ok(l) => l,
                Err(e) => {
                    errors.push(format!("{}: {e:#}", t.name));
                    continue;
                }
            };
            let Some(n) = newest(&list) else { continue };
            if lat.iter().all(|i| is_release(i, &n.tag)) {
                continue;
            }
            match install(http, home, data_dir, t, target.id, Some(&n.tag.clone()), true, progress) {
                Ok(d) => done.push(d),
                Err(e) => errors.push(format!("{} for {}: {e:#}", t.name, target.name)),
            }
        }
    }
    if done.is_empty() && !errors.is_empty() {
        bail!("{}", errors.join("; "));
    }
    Ok(done)
}

/// Copy DXVK's or VKD3D-Proton's DLLs into a Wine prefix (64-bit ones into system32, 32-bit ones into syswow64,
/// or system32 in a 32-bit prefix). Returns the DLL names, for WINEDLLOVERRIDES.
pub fn install_dlls(tool_dir: &Path, pfx: &Path) -> Result<Vec<String>> {
    let win = pfx.join("drive_c/windows");
    if !win.is_dir() {
        bail!("the prefix isn't set up yet");
    }
    let wow = win.join("syswow64");
    let pairs: Vec<(PathBuf, PathBuf)> = if wow.is_dir() {
        vec![(tool_dir.join("x64"), win.join("system32")), (tool_dir.join("x32"), wow.clone()), (tool_dir.join("x86"), wow)]
    } else {
        vec![(tool_dir.join("x32"), win.join("system32")), (tool_dir.join("x86"), win.join("system32"))]
    };
    let mut names = Vec::new();
    for (from, to) in pairs {
        let Ok(rd) = std::fs::read_dir(&from) else { continue };
        std::fs::create_dir_all(&to)?;
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if !n.to_ascii_lowercase().ends_with(".dll") {
                continue;
            }
            std::fs::copy(e.path(), to.join(&n)).with_context(|| format!("copying {n}"))?;
            let stem = n[..n.len() - 4].to_string();
            if !names.contains(&stem) {
                names.push(stem);
            }
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub struct FakeHttp {
        pub json: Value,
        pub files: Vec<(String, Vec<u8>)>,
    }
    impl Http for FakeHttp {
        fn get_json(&self, _: &str, _: &[(&str, String)]) -> Result<Value> {
            Ok(self.json.clone())
        }
        fn download(&self, url: &str, out: &mut dyn Write, _: u64, progress: &mut dyn FnMut(u64)) -> Result<u64> {
            let (_, b) = self.files.iter().find(|(u, _)| u == url).ok_or_else(|| anyhow!("no {url}"))?;
            out.write_all(b)?;
            progress(b.len() as u64);
            Ok(b.len() as u64)
        }
    }

    fn rel(tag: &str, assets: &[(&str, u64)]) -> Value {
        json!({"tag_name": tag, "name": tag, "draft": false, "prerelease": false, "published_at": "2025-01-01T00:00:00Z",
               "assets": assets.iter().map(|(n, s)| json!({"name": n, "size": s, "browser_download_url": format!("https://github.com/x/y/releases/download/{tag}/{n}")})).collect::<Vec<_>>()})
    }

    #[test]
    fn picks_the_right_assets() {
        let ge = find("ge-proton").unwrap();
        let r = parse_release(ge, &rel("GE-Proton10-12", &[("GE-Proton10-12.tar.gz", 500), ("GE-Proton10-12.sha512sum", 1)])).unwrap();
        assert_eq!((r.asset.as_str(), r.size), ("GE-Proton10-12.tar.gz", 500));
        assert!(r.checksum_url.unwrap().ends_with("GE-Proton10-12.sha512sum"));
        let cachy = find("proton-cachyos").unwrap();
        let r = parse_release(cachy, &rel("cachyos-10.0-20250807-slr", &[("proton-cachyos-10.0-20250807-slr-x86_64_v3.tar.xz", 1), ("proton-cachyos-10.0-20250807-slr-x86_64.tar.xz", 2), ("proton-cachyos-10.0-20250807-slr-x86_64.sha512sum", 3)])).unwrap();
        assert_eq!(r.asset, "proton-cachyos-10.0-20250807-slr-x86_64.tar.xz");
        assert!(r.checksum_url.is_some());
        let k = find("kron4ek-wine").unwrap();
        let assets = [("wine-10.12-amd64.tar.xz", 1), ("wine-10.12-staging-amd64.tar.xz", 1), ("wine-10.12-amd64-wow64.tar.xz", 1), ("sha256sums.txt", 1)];
        assert_eq!(parse_release(k, &rel("10.12", &assets)).unwrap().asset, "wine-10.12-amd64.tar.xz");
        assert_eq!(parse_release(find("kron4ek-wine-staging").unwrap(), &rel("10.12", &assets)).unwrap().asset, "wine-10.12-staging-amd64.tar.xz");
        assert!(parse_release(k, &rel("10.12", &assets)).unwrap().checksum_url.unwrap().ends_with("sha256sums.txt"));
        assert_eq!(parse_release(find("dxvk").unwrap(), &rel("v2.7.1", &[("dxvk-native-2.7.1-steamrt-sniper.tar.gz", 1), ("dxvk-2.7.1.tar.gz", 1)])).unwrap().asset, "dxvk-2.7.1.tar.gz");
        assert!(parse_release(ge, &rel("x", &[("source.zip", 1)])).is_none());
        let mut draft = rel("GE-Proton10-13", &[("GE-Proton10-13.tar.gz", 1)]);
        draft["draft"] = json!(true);
        assert!(parse_release(ge, &draft).is_none());
    }

    #[test]
    fn checksums_and_names() {
        let h = "a".repeat(128);
        assert_eq!(checksum_for(&format!("{h}  GE-Proton10-12.tar.gz\n"), "GE-Proton10-12.tar.gz").as_deref(), Some(h.as_str()));
        let s = "b".repeat(64);
        let list = format!("{s}  wine-10.12-amd64.tar.xz\n{}  wine-10.12-staging-amd64.tar.xz\n", "c".repeat(64));
        assert_eq!(checksum_for(&list, "wine-10.12-amd64.tar.xz").as_deref(), Some(s.as_str()));
        assert_eq!(checksum_for(&list, "other.tar.xz"), None);
        assert_eq!(checksum_for(&s, "anything").as_deref(), Some(s.as_str()));
        assert_eq!(checksum_for("not a hash  x", "x"), None);
        let i = |name: &str| Installed { name: name.into(), path: PathBuf::new(), tool: None, tag: None, latest: false, by_beacon: false };
        assert!(is_release(&i("GE-Proton10-12"), "GE-Proton10-12") && !is_release(&i("GE-Proton10-1"), "GE-Proton10-12"));
        assert!(is_release(&i("dxvk-2.7.1"), "v2.7.1") && is_release(&i("wine-10.12-amd64"), "10.12"));
        assert!(is_release(&i("proton-cachyos-10.0-20250807-slr-x86_64"), "cachyos-10.0-20250807-slr"));
        assert_eq!(guess_tool("GE-Proton9-20", Kind::Proton).unwrap().id, "ge-proton");
        assert_eq!(guess_tool("dxvk-async-2.0", Kind::Dxvk).unwrap().id, "dxvk-async");
        assert_eq!(guess_tool("dxvk-2.7", Kind::Dxvk).unwrap().id, "dxvk");
        assert_eq!(guess_tool("wine-10.12-staging-amd64", Kind::Wine).unwrap().id, "kron4ek-wine-staging");
        assert_eq!(guess_tool("wine-lutris-GE-Proton8-26-x86_64", Kind::Wine).unwrap().id, "wine-ge");
        let vdf = "\"compatibilitytools\"\n{\n  \"compat_tools\"\n  {\n    \"GE-Proton10-12\" // Internal name\n    {\n      \"install_path\" \".\"\n      \"display_name\" \"GE-Proton10-12\"\n    }\n  }\n}\n";
        let v = rename_vdf(vdf, "GE-Proton-Latest", "GE-Proton Latest (GE-Proton10-12)");
        assert!(v.contains("\"GE-Proton-Latest\" // Internal name") && v.contains("\"display_name\" \"GE-Proton Latest (GE-Proton10-12)\""), "{v}");
    }

    #[test]
    fn targets_and_folders() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let data = t.path().join("data");
        std::fs::create_dir_all(home.join(".local/share/Steam")).unwrap();
        std::fs::create_dir_all(home.join(".var/app/com.usebottles.bottles")).unwrap();
        let ts = targets(&home);
        let on: Vec<&str> = ts.iter().filter(|t| t.detected).map(|t| t.id).collect();
        assert_eq!(on, vec!["beacon", "steam", "bottles-flatpak"]);
        assert_eq!(target_dir(&home, &data, "steam", Kind::Proton).unwrap(), home.join(".local/share/Steam/compatibilitytools.d"));
        assert!(target_dir(&home, &data, "steam", Kind::Wine).is_none());
        assert_eq!(target_dir(&home, &data, "lutris", Kind::Dxvk).unwrap(), home.join(".local/share/lutris/runtime/dxvk"));
        assert_eq!(target_dir(&home, &data, "heroic-flatpak", Kind::Wine).unwrap(), home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic/tools/wine"));
        assert_eq!(target_dir(&home, &data, "beacon", Kind::Proton).unwrap(), crate::runner::beacon_proton_dir(&data));
        assert!(target_dir(&home, &data, "steam", Kind::Umu).is_none() && target_dir(&home, &data, "nope", Kind::Proton).is_none());
    }

    fn tarball(dir: &Path, top: &str, out: &Path) {
        std::fs::create_dir_all(dir.join(top)).unwrap();
        std::fs::write(dir.join(top).join("proton"), "#!/bin/sh\n").unwrap();
        std::fs::write(dir.join(top).join("compatibilitytool.vdf"), format!("\"compatibilitytools\"\n{{\n  \"compat_tools\"\n  {{\n    \"{top}\"\n    {{\n      \"display_name\" \"{top}\"\n    }}\n  }}\n}}\n")).unwrap();
        let st = std::process::Command::new("tar").arg("-czf").arg(out).arg("-C").arg(dir).arg(top).status().unwrap();
        assert!(st.success());
    }

    // Uses the system's tar.
    #[cfg(unix)]
    #[test]
    fn installs_checks_and_removes() {
        use sha2::Digest;
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let data = t.path().join("data");
        std::fs::create_dir_all(home.join(".steam/root")).unwrap();
        let src = t.path().join("src");
        let tgz = t.path().join("GE-Proton10-12.tar.gz");
        tarball(&src, "GE-Proton10-12", &tgz);
        let bytes = std::fs::read(&tgz).unwrap();
        let sum: String = sha2::Sha512::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
        let base = "https://github.com/x/y/releases/download/GE-Proton10-12/";
        let mk = |sum: &str| FakeHttp {
            json: json!([rel("GE-Proton10-12", &[("GE-Proton10-12.tar.gz", bytes.len() as u64), ("GE-Proton10-12.sha512sum", 1)])]),
            files: vec![(format!("{base}GE-Proton10-12.tar.gz"), bytes.clone()), (format!("{base}GE-Proton10-12.sha512sum"), format!("{sum}  GE-Proton10-12.tar.gz\n").into_bytes())],
        };
        let ge = find("ge-proton").unwrap();
        let mut said = Vec::new();
        let d = install(&mk(&sum), &home, &data, ge, "beacon", None, false, &mut |_, s: &str| said.push(s.to_string())).unwrap();
        assert!(d.verified && d.path.join("proton").is_file() && d.path.ends_with("compat/proton/GE-Proton10-12"), "{d:?}");
        assert!(said.iter().any(|s| s.starts_with("Downloading")));
        let st = state(&home, &data, ge, "beacon", Some("GE-Proton10-12"));
        assert!(st.up_to_date && st.installed.len() == 1 && st.installed[0].by_beacon);
        // Beacon's own runners pick it up.
        let set = crate::runner::discover_with(&home, Some(&data), "");
        assert!(set.runners.iter().any(|r| r.id == "proton:GE-Proton10-12"));

        // Latest into Steam: its own folder and name in compatibilitytool.vdf.
        let d = install(&mk(&sum), &home, &data, ge, "steam", None, true, &mut |_, _: &str| {}).unwrap();
        assert!(d.path.ends_with(".steam/root/compatibilitytools.d/GE-Proton-Latest"));
        let vdf = std::fs::read_to_string(d.path.join("compatibilitytool.vdf")).unwrap();
        assert!(vdf.contains("\"GE-Proton-Latest\"") && vdf.contains("GE-Proton Latest (GE-Proton10-12)"), "{vdf}");
        assert!(state(&home, &data, ge, "steam", None).has_latest);
        assert!(update_latest(&mk(&sum), &home, &data, &mut |_, _: &str| {}).unwrap().is_empty(), "already on the newest");

        // A wrong checksum: nothing installed, nothing left behind.
        let bad = "0".repeat(128);
        assert!(install(&mk(&bad), &home, &data, ge, "steam", None, false, &mut |_, _: &str| {}).unwrap_err().to_string().contains("checksum"));
        assert!(!home.join(".steam/root/compatibilitytools.d/GE-Proton10-12").exists());
        assert!(std::fs::read_dir(data.join("tmp/tools")).unwrap().next().is_none());

        remove(&home, &data, "beacon", Kind::Proton, "GE-Proton10-12").unwrap();
        assert!(!crate::runner::beacon_proton_dir(&data).join("GE-Proton10-12").exists());
        assert!(remove(&home, &data, "beacon", Kind::Proton, "../x").is_err());
        assert!(remove(&home, &data, "beacon", Kind::Proton, "GE-Proton10-12").is_err());
    }

    #[test]
    fn dlls_into_a_prefix() {
        let t = tempfile::tempdir().unwrap();
        let dx = t.path().join("dxvk-2.7.1");
        for (d, n) in [("x64", "d3d11.dll"), ("x64", "dxgi.dll"), ("x32", "d3d11.dll"), ("x32", "d3d9.dll")] {
            std::fs::create_dir_all(dx.join(d)).unwrap();
            std::fs::write(dx.join(d).join(n), d).unwrap();
        }
        let pfx = t.path().join("pfx");
        assert!(install_dlls(&dx, &pfx).is_err());
        std::fs::create_dir_all(pfx.join("drive_c/windows/system32")).unwrap();
        std::fs::create_dir_all(pfx.join("drive_c/windows/syswow64")).unwrap();
        let mut n = install_dlls(&dx, &pfx).unwrap();
        n.sort();
        assert_eq!(n, vec!["d3d11", "d3d9", "dxgi"]);
        assert_eq!(std::fs::read_to_string(pfx.join("drive_c/windows/system32/d3d11.dll")).unwrap(), "x64");
        assert_eq!(std::fs::read_to_string(pfx.join("drive_c/windows/syswow64/d3d11.dll")).unwrap(), "x32");
    }
}
