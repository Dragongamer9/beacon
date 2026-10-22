//! **Private build only.** Downloading from ModDB without a browser.
//!
//! ModDB's pages chain like this:
//! * a mod or game page (`/mods/<slug>`) → its downloads list (`…/downloads`),
//!   newest first, each entry linking to a download page
//!   (`…/downloads/<file-slug>`, or `moddb.com/downloads/<file-slug>`);
//! * a download page names the file (`Filename`), its size in bytes and its
//!   MD5, and links to `/downloads/start/<id>`;
//! * the start page links to `/downloads/mirror/<id>/<mirror>/<hash>`, which
//!   redirects to the file itself.
//!
//! Beacon follows the same chain. Any of those links works as a starting
//! point (a link pasted from the browser, or a `moddb` source in
//! `sources.json`). The MD5 from the download page checks the file.

use super::text::{self, glob};
use super::{host, Transfer, MAX_TEXT_BYTES};
use anyhow::{anyhow, bail, Result};

/// ModDB answers browsers; Beacon says what it is too.
pub fn headers() -> Vec<(&'static str, String)> {
    vec![
        ("User-Agent", concat!("Mozilla/5.0 (X11; Linux x86_64) Beacon/", env!("CARGO_PKG_VERSION")).to_string()),
        ("Accept", "text/html,application/xhtml+xml".to_string()),
    ]
}

/// What a ModDB link points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// A mod or game page, or its downloads list: the list to pick from.
    List(String),
    /// One file's download page.
    Download(String),
    /// `/downloads/start/<id>`.
    Start(String),
}

/// Is this a moddb.com https link, and which kind?
pub fn classify(url: &str) -> Option<Link> {
    let u = url.trim();
    if !super::is_https(u) {
        return None;
    }
    let h = host(u).to_ascii_lowercase();
    if h != "moddb.com" && h != "www.moddb.com" {
        return None;
    }
    let path = u.split(['?', '#']).next().unwrap_or(u).trim_end_matches('/');
    let after = path.split_once("moddb.com").map(|(_, p)| p).unwrap_or("");
    let segs: Vec<&str> = after.split('/').filter(|s| !s.is_empty()).collect();
    let base = format!("https://www.moddb.com/{}", segs.join("/"));
    match segs.as_slice() {
        ["downloads", "start", _id, ..] => Some(Link::Start(base)),
        ["downloads", _slug] => Some(Link::Download(base)),
        [_kind, _slug, "downloads", "page", ..] | [_kind, _slug, "downloads"] => Some(Link::List(format!("https://www.moddb.com/{}/{}/downloads", segs[0], segs[1]))),
        [_kind, _slug, "downloads", _file] => Some(Link::Download(base)),
        [kind, slug, ..] if ["mods", "games", "addons", "engines"].contains(kind) => Some(Link::List(format!("https://www.moddb.com/{kind}/{slug}/downloads"))),
        _ => None,
    }
}

/// What a download page says about its file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileInfo {
    pub title: Option<String>,
    pub file_name: Option<String>,
    pub size: Option<u64>,
    pub md5: Option<String>,
    pub start: Option<String>,
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => {
                in_tag = true;
                out.push(' ');
            }
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&nbsp;", " ").replace("&#39;", "'").replace("&quot;", "\"")
}

/// The page's text after the first `label` (case-insensitive), tags removed, at most `len` bytes of HTML.
fn text_after(html: &str, label: &str, len: usize) -> Option<String> {
    let i = html.to_ascii_lowercase().find(&label.to_ascii_lowercase())? + label.len();
    let end = (i + len).min(html.len());
    let end = (end..=html.len()).find(|e| html.is_char_boundary(*e)).unwrap_or(html.len());
    Some(strip_tags(&html[i..end]))
}

/// Read a download page.
pub fn parse_download_page(html: &str, page_url: &str) -> FileInfo {
    let mut info = FileInfo::default();
    info.title = text_after(html, "<title>", 300).and_then(|t| t.split(" - ").next().map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))).filter(|t| !t.is_empty());
    info.file_name = text_after(html, ">Filename<", 600).and_then(|t| {
        t.split_whitespace().find(|w| w.contains('.') && w.len() < 200 && !w.contains('/') && !w.contains('\\')).map(str::to_string)
    });
    // "2.67gb (2,861,726,686 bytes)"
    if let Some(b) = html.find("bytes)") {
        if let Some(open) = html[..b].rfind('(') {
            let digits: String = html[open + 1..b].chars().filter(|c| c.is_ascii_digit()).collect();
            info.size = digits.parse().ok().filter(|n: &u64| *n > 0);
        }
    }
    info.md5 = text_after(html, "MD5", 400).and_then(|t| {
        t.split(|c: char| !c.is_ascii_hexdigit()).find(|w| w.len() == 32).map(|w| w.to_ascii_lowercase())
    });
    info.start = text::links(html, page_url).into_iter().map(|l| l.url).find(|u| u.contains("/downloads/start/"));
    info
}

/// The mirror link on a start page (a link, or a script that sends the browser there).
pub fn parse_start_page(html: &str, page_url: &str) -> Option<String> {
    if let Some(u) = text::links(html, page_url).into_iter().map(|l| l.url).find(|u| u.contains("/downloads/mirror/")) {
        return Some(u);
    }
    let i = html.find("/downloads/mirror/")?;
    let start = html[..i].rfind(['"', '\'']).map(|s| s + 1).unwrap_or(i);
    let end = html[i..].find(['"', '\'', '<', ' ']).map(|e| i + e).unwrap_or(html.len());
    text::join(page_url, &html[start..end].replace("\\/", "/"))
}

/// Download pages a downloads list links to, in page order (newest first).
pub fn parse_list(html: &str, list_url: &str) -> Vec<String> {
    let base = list_url.trim_end_matches('/').to_ascii_lowercase();
    let mut out: Vec<String> = Vec::new();
    for l in text::links(html, list_url) {
        let u = l.url.split(['?', '#']).next().unwrap_or(&l.url).trim_end_matches('/').to_string();
        let lu = u.to_ascii_lowercase();
        let child = lu.starts_with(&format!("{base}/")) && !lu[base.len() + 1..].contains('/');
        if child && !lu.ends_with("/downloads/feed") && !out.contains(&u) {
            out.push(u);
        }
    }
    out
}

/// The file a ModDB link leads to now: (mirror link, what the download page said).
/// `file`: a wildcard for the download's slug when starting from a list (first match, newest first);
/// `exclude`: slugs to skip (demos are always skipped unless `file` asks for one).
pub fn resolve(t: &dyn Transfer, start: &str, file: Option<&str>, exclude: &[String]) -> Result<(String, FileInfo)> {
    let h = headers();
    let link = classify(start).ok_or_else(|| anyhow!("{start} isn't a ModDB mod, game or download link"))?;
    let download_page = match link {
        Link::Start(s) => {
            let html = t.get_text(&s, &h, MAX_TEXT_BYTES)?;
            let mirror = parse_start_page(&html, &s).ok_or_else(|| anyhow!("ModDB's start page has no download link (the page changed?)"))?;
            return Ok((mirror, FileInfo::default()));
        }
        Link::Download(d) => d,
        Link::List(l) => {
            let html = t.get_text(&l, &h, MAX_TEXT_BYTES)?;
            let pages = parse_list(&html, &l);
            if pages.is_empty() {
                bail!("{l} lists no downloads");
            }
            let slug = |u: &str| u.rsplit('/').next().unwrap_or("").to_string();
            let wants_demo = file.map(|f| f.to_ascii_lowercase().contains("demo")).unwrap_or(false);
            pages
                .iter()
                .find(|u| {
                    let s = slug(u);
                    file.map(|f| glob(f, &s)).unwrap_or(true)
                        && !exclude.iter().any(|x| glob(x, &s))
                        && (wants_demo || !s.contains("demo"))
                })
                .cloned()
                .ok_or_else(|| anyhow!("no download on {l} matches {}", file.unwrap_or("a full release")))?
        }
    };
    let html = t.get_text(&download_page, &h, MAX_TEXT_BYTES)?;
    let info = parse_download_page(&html, &download_page);
    let start_url = info.start.clone().ok_or_else(|| anyhow!("{download_page} has no download button (the page changed?)"))?;
    let html = t.get_text(&start_url, &h, MAX_TEXT_BYTES)?;
    let mirror = parse_start_page(&html, &start_url).ok_or_else(|| anyhow!("ModDB's start page has no download link (the page changed?)"))?;
    Ok((mirror, info))
}

#[cfg(test)]
mod tests {
    use super::super::testnet::FakeNet;
    use super::*;

    #[test]
    fn links() {
        assert_eq!(classify("https://www.moddb.com/mods/edain-mod"), Some(Link::List("https://www.moddb.com/mods/edain-mod/downloads".into())));
        assert_eq!(classify("https://moddb.com/mods/edain-mod/downloads/"), Some(Link::List("https://www.moddb.com/mods/edain-mod/downloads".into())));
        assert_eq!(classify("https://www.moddb.com/mods/edain-mod/downloads/edain-48?x=1"), Some(Link::Download("https://www.moddb.com/mods/edain-mod/downloads/edain-48".into())));
        assert_eq!(classify("https://www.moddb.com/downloads/armies-of-exigo-fix"), Some(Link::Download("https://www.moddb.com/downloads/armies-of-exigo-fix".into())));
        assert_eq!(classify("https://www.moddb.com/downloads/start/282838"), Some(Link::Start("https://www.moddb.com/downloads/start/282838".into())));
        assert_eq!(classify("https://www.moddb.com/games/lotr-conquest/downloads/conquest-fixes"), Some(Link::Download("https://www.moddb.com/games/lotr-conquest/downloads/conquest-fixes".into())));
        assert_eq!(classify("https://example.com/mods/x"), None);
        assert_eq!(classify("http://www.moddb.com/mods/x"), None);
    }

    const DOWNLOAD_PAGE: &str = r#"<html><head><title>Edain 4.8 file - Edain Mod for Battle for Middle-earth II: The Rise of the Witch-king - ModDB</title></head><body>
        <a href="/downloads/start/282838" class="buttondownload">Download now</a>
        <div class="row clear"><h5>Filename</h5><span class="summary">Edain4.8.zip</span></div>
        <div class="row clear"><h5>Size</h5><span class="summary">2.67gb (2,861,726,686 bytes)</span></div>
        <div class="row clear"><h5>MD5 Hash</h5><span class="summary">B5F6237A9FB72549F5954CE9A442EBDC</span></div></body></html>"#;

    #[test]
    fn reads_a_download_page() {
        let i = parse_download_page(DOWNLOAD_PAGE, "https://www.moddb.com/mods/edain-mod/downloads/edain-48");
        assert_eq!(i.title.as_deref(), Some("Edain 4.8 file"));
        assert_eq!(i.file_name.as_deref(), Some("Edain4.8.zip"));
        assert_eq!(i.size, Some(2_861_726_686));
        assert_eq!(i.md5.as_deref(), Some("b5f6237a9fb72549f5954ce9a442ebdc"));
        assert_eq!(i.start.as_deref(), Some("https://www.moddb.com/downloads/start/282838"));
    }

    #[test]
    fn start_pages() {
        let a = r#"<p>If your download doesn't start, <a href="/downloads/mirror/282838/130/abc123/?referer=x">click here</a></p>"#;
        assert_eq!(parse_start_page(a, "https://www.moddb.com/downloads/start/282838").as_deref(), Some("https://www.moddb.com/downloads/mirror/282838/130/abc123/?referer=x"));
        let b = r#"<script>window.location.href = "\/downloads\/mirror\/1\/2\/ff";</script>"#;
        assert_eq!(parse_start_page(b, "https://www.moddb.com/downloads/start/1").as_deref(), Some("https://www.moddb.com/downloads/mirror/1/2/ff"));
        assert_eq!(parse_start_page("<p>nothing</p>", "https://www.moddb.com/downloads/start/1"), None);
    }

    #[test]
    fn follows_the_chain_from_a_list() {
        let list = r#"<a href="/mods/edain-mod/downloads/unit-plate-submod">Unit Plate Submod</a>
            <a href="/mods/edain-mod/downloads/edain-48">Edain 4.8</a>
            <a href="/mods/edain-mod/downloads/edain-mod-launcher-1200">Launcher</a>
            <a href="/mods/edain-mod/downloads/edain-45-demo">Demo</a>
            <a href="/mods/edain-mod/downloads/page/2">Next</a>"#;
        let net = FakeNet::with(&[
            ("https://www.moddb.com/mods/edain-mod/downloads", list.as_bytes()),
            ("https://www.moddb.com/mods/edain-mod/downloads/edain-48", DOWNLOAD_PAGE.as_bytes()),
            ("https://www.moddb.com/downloads/start/282838", br#"<a href="/downloads/mirror/282838/130/abc/">click here</a>"#),
        ]);
        let ex = vec!["*submod*".to_string(), "*launcher*".to_string()];
        let (mirror, info) = resolve(&net, "https://www.moddb.com/mods/edain-mod", Some("edain-4*"), &ex).unwrap();
        assert_eq!(mirror, "https://www.moddb.com/downloads/mirror/282838/130/abc/");
        assert_eq!(info.md5.as_deref(), Some("b5f6237a9fb72549f5954ce9a442ebdc"));
        // Without a pattern: the newest that isn't excluded or a demo.
        let (_, info) = resolve(&net, "https://www.moddb.com/mods/edain-mod/downloads", None, &ex).unwrap();
        assert_eq!(info.file_name.as_deref(), Some("Edain4.8.zip"));
        assert!(resolve(&net, "https://www.moddb.com/mods/edain-mod", Some("nope-*"), &ex).is_err());
    }
}
