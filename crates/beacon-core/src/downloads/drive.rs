//! Public Google Drive folders and files, without an
//! API key: list a shared folder (its embedded view, or the data the normal
//! folder page carries), and turn a file id into a link that downloads it
//! (getting past the "can't scan this file for viruses" page large files get).

use super::text::{self, glob, natural_cmp};
use super::{host, Transfer, MAX_TEXT_BYTES};
use anyhow::{anyhow, bail, Result};
use serde_json::Value;

/// One entry in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub folder: bool,
}

/// Is this a Drive id (letters, digits, `-`, `_`; 10 to 100 of them)?
pub fn is_id(s: &str) -> bool {
    (10..=100).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A folder id from an id or a `…/folders/<id>` link.
pub fn folder_id(s: &str) -> Option<String> {
    let s = s.trim();
    let id = s.split("/folders/").nth(1).map(|r| r.split(['?', '/', '#']).next().unwrap_or("")).unwrap_or(s);
    is_id(id).then(|| id.to_string())
}

/// A file id from an id or a `…/file/d/<id>/…` (or `…?id=<id>`) link.
pub fn file_id(s: &str) -> Option<String> {
    let s = s.trim();
    let id = if let Some(r) = s.split("/file/d/").nth(1) {
        r.split(['?', '/', '#']).next().unwrap_or("")
    } else if let Some(r) = s.split("id=").nth(1).filter(|_| s.contains("drive.google.com") || s.contains("googleusercontent.com")) {
        r.split(['&', '#']).next().unwrap_or("")
    } else {
        s
    };
    is_id(id).then(|| id.to_string())
}

/// List a public folder: the embedded view first, the folder page's data second.
pub fn list(t: &dyn Transfer, folder: &str) -> Result<Vec<Entry>> {
    if !is_id(folder) {
        bail!("{folder:?} isn't a Google Drive folder id");
    }
    let mut errs = Vec::new();
    match t.get_text(&format!("https://drive.google.com/embeddedfolderview?id={folder}"), &[], MAX_TEXT_BYTES) {
        Ok(html) => {
            let e = parse_embedded(&html);
            if !e.is_empty() {
                return Ok(e);
            }
            errs.push("the embedded view listed nothing".to_string());
        }
        Err(e) => errs.push(format!("{e:#}")),
    }
    match t.get_text(&format!("https://drive.google.com/drive/folders/{folder}"), &[], MAX_TEXT_BYTES) {
        Ok(html) => {
            let e = parse_folder_page(&html, folder);
            if !e.is_empty() {
                return Ok(e);
            }
            errs.push("the folder page listed nothing (is it still shared publicly?)".to_string());
        }
        Err(e) => errs.push(format!("{e:#}")),
    }
    bail!("couldn't list the Google Drive folder: {}", errs.join("; "))
}

/// `embeddedfolderview`: `<div class="flip-entry" id="entry-<id>">… <a href=".../file/d/<id>/…" or ".../folders/<id>">
/// … <div class="flip-entry-title">Name</div>`.
pub fn parse_embedded(html: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let marker = "id=\"entry-";
    let mut from = 0;
    while let Some(p) = html[from..].find(marker) {
        let start = from + p + marker.len();
        let Some(end) = html[start..].find('"') else { break };
        let id = &html[start..start + end];
        // The entry runs to the next one.
        let block_end = html[start..].find(marker).map(|n| start + n).unwrap_or(html.len());
        let block = &html[start..block_end];
        from = block_end;
        if !is_id(id) {
            continue;
        }
        let name = block
            .find("flip-entry-title\">")
            .map(|i| &block[i + "flip-entry-title\">".len()..])
            .and_then(|r| r.find('<').map(|j| &r[..j]))
            .map(unescape)
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let folder = block.contains("/folders/") || block.contains("folderview?");
        out.push(Entry { id: id.to_string(), name, folder });
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").trim().to_string()
}

/// The normal folder page: `window['_DRIVE_ivd'] = '<JS string with JSON>'`; entries are arrays
/// `[id, [parent id], name, mime type, …]`.
pub fn parse_folder_page(html: &str, folder: &str) -> Vec<Entry> {
    let marker = "_DRIVE_ivd'] = '";
    let Some(p) = html.find(marker) else { return vec![] };
    let rest = &html[p + marker.len()..];
    let Some(lit) = js_string(rest) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(&lit) else { return vec![] };
    let mut out = Vec::new();
    walk(&v, folder, &mut out);
    out
}

fn walk(v: &Value, folder: &str, out: &mut Vec<Entry>) {
    let Some(a) = v.as_array() else { return };
    let is_entry = a.len() >= 4
        && a[0].as_str().map(is_id).unwrap_or(false)
        && a[1].as_array().map(|p| p.iter().any(|x| x.as_str() == Some(folder))).unwrap_or(false)
        && a[2].is_string()
        && a[3].is_string();
    if is_entry {
        let id = a[0].as_str().unwrap_or_default().to_string();
        if !out.iter().any(|e| e.id == id) {
            out.push(Entry {
                id,
                name: a[2].as_str().unwrap_or_default().to_string(),
                folder: a[3].as_str() == Some("application/vnd.google-apps.folder"),
            });
        }
        return;
    }
    for x in a {
        walk(x, folder, out);
    }
}

/// A single-quoted JS string literal's contents (up to the closing quote), unescaped.
fn js_string(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => return Some(out),
            '\\' => match chars.next()? {
                'x' => {
                    let h: String = chars.by_ref().take(2).collect();
                    out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?)?);
                }
                'u' => {
                    let h: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&h, 16).ok()?).unwrap_or('\u{fffd}'));
                }
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// The newest file matching any of `patterns` (newest = highest version in its name), names
/// with `prefer` in them first. Looks into subfolders (newest first) when the folder itself has
/// none, up to `depth` levels. Returns the file and the folder path it was found in.
pub fn pick(t: &dyn Transfer, folder: &str, patterns: &[String], prefer: Option<&str>, depth: u8) -> Result<Option<(Entry, Vec<String>)>> {
    let entries = list(t, folder)?;
    let mut files: Vec<&Entry> = entries.iter().filter(|e| !e.folder && patterns.iter().any(|p| glob(p, &e.name))).collect();
    let pref = |e: &Entry| prefer.map(|w| !e.name.to_lowercase().contains(&w.to_lowercase())).unwrap_or(false);
    files.sort_by(|a, b| pref(*a).cmp(&pref(*b)).then_with(|| natural_cmp(&b.name, &a.name)));
    if let Some(f) = files.first() {
        return Ok(Some(((*f).clone(), vec![])));
    }
    if depth > 0 {
        let mut subs: Vec<&Entry> = entries.iter().filter(|e| e.folder).collect();
        subs.sort_by(|a, b| natural_cmp(&b.name, &a.name));
        for s in subs.into_iter().take(5) {
            if let Some((f, mut path)) = pick(t, &s.id, patterns, prefer, depth - 1)? {
                path.insert(0, s.name.clone());
                return Ok(Some((f, path)));
            }
        }
    }
    Ok(None)
}

/// The closest file when nothing matches (same extension as a pattern, most words in common).
pub fn near_miss(t: &dyn Transfer, folder: &str, patterns: &[String]) -> Result<Option<Entry>> {
    let entries = list(t, folder)?;
    let exts: Vec<String> = patterns.iter().filter_map(|p| text::extension(p)).collect();
    let mut near: Vec<(f32, &Entry)> = entries
        .iter()
        .filter(|e| !e.folder)
        .filter(|e| exts.is_empty() || text::extension(&e.name).map(|x| exts.contains(&x)).unwrap_or(false))
        .map(|e| (patterns.iter().map(|p| text::similarity(p, &e.name)).fold(0.0f32, f32::max), e))
        .filter(|(s, _)| *s >= 0.5)
        .collect();
    near.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| natural_cmp(&b.1.name, &a.1.name)));
    Ok(near.first().map(|(_, e)| (*e).clone()))
}

/// A link that downloads the file itself, and its size when the server says. Large files get a
/// "can't scan for viruses" page first; its form leads to the real download.
pub fn download_url(t: &dyn Transfer, id: &str) -> Result<(String, Option<u64>)> {
    if !is_id(id) {
        bail!("{id:?} isn't a Google Drive file id");
    }
    let mut url = format!("https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t");
    for _ in 0..2 {
        let (opened, mut r) = t.open(&url, 0, &[])?;
        let html = opened.content_type.as_deref().map(|c| c.to_ascii_lowercase().starts_with("text/html")).unwrap_or(false);
        if !html {
            return Ok((url, opened.len));
        }
        let mut body = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(&mut r, 1 << 20), &mut body)?;
        let page = String::from_utf8_lossy(&body);
        match confirm_form(&page) {
            Some(next) if next != url => url = next,
            _ => {
                let why = if page.contains("Quota exceeded") || page.contains("quota") {
                    "Google Drive's download quota for this file is used up for now; try again later"
                } else if page.contains("accounts.google.com") {
                    "Google Drive wants a sign-in for this file (it's no longer shared publicly?)"
                } else {
                    "Google Drive answered with a page instead of the file"
                };
                bail!("{why} ({})", host(&url));
            }
        }
    }
    Err(anyhow!("Google Drive kept answering with a page instead of the file"))
}

/// The virus-scan warning's form as a URL: its `action` plus every hidden input.
pub fn confirm_form(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let f = lower.find("<form")?;
    let form_end = lower[f..].find("</form>").map(|e| f + e).unwrap_or(html.len());
    let form = &html[f..form_end];
    let action = attr(form, "action").map(|a| a.replace("&amp;", "&"))?;
    let action = text::join("https://drive.usercontent.google.com/", &action)?;
    let mut params: Vec<(String, String)> = Vec::new();
    let mut from = 0;
    let fl = form.to_ascii_lowercase();
    while let Some(p) = fl[from..].find("<input") {
        let start = from + p;
        let end = fl[start..].find('>').map(|e| start + e).unwrap_or(form.len());
        let tag = &form[start..end];
        from = end;
        if let (Some(n), Some(v)) = (attr(tag, "name"), attr(tag, "value")) {
            params.push((n, v));
        }
    }
    if params.is_empty() {
        return None;
    }
    let q: Vec<String> = params.iter().map(|(n, v)| format!("{}={}", enc(n), enc(v))).collect();
    let sep = if action.contains('?') { '&' } else { '?' };
    Some(format!("{action}{sep}{}", q.join("&")))
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(p) = lower[from..].find(&format!("{name}=")) {
        let i = from + p;
        from = i + name.len() + 1;
        if i > 0 && lower.as_bytes()[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        let rest = &tag[i + name.len() + 1..];
        let q = rest.chars().next()?;
        return if q == '"' || q == '\'' {
            rest[1..].find(q).map(|e| unescape(&rest[1..1 + e]))
        } else {
            Some(rest.split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or("").to_string())
        };
    }
    None
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[cfg(test)]
mod tests {
    use super::super::testnet::FakeNet;
    use super::super::{Opened, Transfer};
    use super::*;

    const FOLDER: &str = "167UET84ZWXrMd9zpxEYiQWTQZexCGZfT";

    fn embedded(entries: &[(&str, &str, bool)]) -> String {
        let mut s = String::from("<html><body><div class=\"flip-entries\">");
        for (id, name, folder) in entries {
            let href = if *folder { format!("https://drive.google.com/drive/folders/{id}") } else { format!("https://drive.google.com/file/d/{id}/view?usp=drive_web") };
            s.push_str(&format!(
                "<div class=\"flip-entry\" id=\"entry-{id}\" tabindex=\"0\" role=\"link\"><div class=\"flip-entry-info\"><a href=\"{href}\" target=\"_blank\"><div class=\"flip-entry-thumb\"></div><div class=\"flip-entry-title\">{name}</div></a></div><div class=\"flip-entry-last-modified\"><div>Mar 3</div></div></div>"
            ));
        }
        s + "</div></body></html>"
    }

    #[test]
    fn parses_the_embedded_view() {
        let html = embedded(&[("1aaaaaaaaaaaaaaaaaaaaaaa", "UaW Launcher 1.2.zip", false), ("1bbbbbbbbbbbbbbbbbbbbbbb", "Old &amp; broken", true)]);
        let e = parse_embedded(&html);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0], Entry { id: "1aaaaaaaaaaaaaaaaaaaaaaa".into(), name: "UaW Launcher 1.2.zip".into(), folder: false });
        assert_eq!(e[1].name, "Old & broken");
        assert!(e[1].folder);
    }

    #[test]
    fn parses_the_folder_page_data() {
        let json = format!(
            r#"[[["1ccccccccccccccccccccccc",["{FOLDER}"],"UaW Launcher 1.10.zip","application/zip",1],["1ddddddddddddddddddddddd",["{FOLDER}"],"Older","application/vnd.google-apps.folder"]],null]"#
        );
        // As the page has it: a single-quoted JS string with \x escapes.
        let lit: String = json.chars().map(|c| if c == '"' || c == '[' || c == ']' { format!("\\x{:02x}", c as u32) } else { c.to_string() }).collect();
        let html = format!("<script>window['_DRIVE_ivd'] = '{lit}';</script>");
        let e = parse_folder_page(&html, FOLDER);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].name, "UaW Launcher 1.10.zip");
        assert!(e[1].folder);
    }

    #[test]
    fn picks_the_newest_launcher_and_looks_in_subfolders() {
        let net = FakeNet::default();
        net.put(
            &format!("https://drive.google.com/embeddedfolderview?id={FOLDER}"),
            embedded(&[
                ("1aaaaaaaaaaaaaaaaaaaaaaa", "UaW Launcher 1.2.zip", false),
                ("1eeeeeeeeeeeeeeeeeeeeeee", "UaW Launcher 1.10.zip", false),
                ("1fffffffffffffffffffffff", "readme.zip", false),
            ])
            .as_bytes(),
        );
        let pats = vec!["*.zip".to_string(), "*.exe".to_string()];
        let (f, path) = pick(&net, FOLDER, &pats, Some("launcher"), 1).unwrap().unwrap();
        assert_eq!(f.name, "UaW Launcher 1.10.zip");
        assert!(path.is_empty());
        // Only subfolders: the newest one is searched.
        net.put(
            &format!("https://drive.google.com/embeddedfolderview?id={FOLDER}"),
            embedded(&[("1ggggggggggggggggggggggg", "v1.3", true), ("1hhhhhhhhhhhhhhhhhhhhhhh", "v1.4", true)]).as_bytes(),
        );
        net.put("https://drive.google.com/embeddedfolderview?id=1hhhhhhhhhhhhhhhhhhhhhhh", embedded(&[("1iiiiiiiiiiiiiiiiiiiiiii", "UaWLauncher.exe", false)]).as_bytes());
        let (f, path) = pick(&net, FOLDER, &pats, Some("launcher"), 1).unwrap().unwrap();
        assert_eq!(f.name, "UaWLauncher.exe");
        assert_eq!(path, vec!["v1.4"]);
    }

    struct Html {
        pages: Vec<(&'static str, &'static str, &'static str)>, // url, content type, body
    }

    impl Transfer for Html {
        fn open(&self, url: &str, _offset: u64, _h: &[(&str, String)]) -> Result<(Opened, Box<dyn std::io::Read + Send>)> {
            let (_, ct, body) = self.pages.iter().find(|(u, _, _)| *u == url).ok_or_else(|| anyhow!("{url}: HTTP 404"))?;
            let o = Opened { partial: false, len: Some(body.len() as u64), final_url: url.into(), content_type: Some(ct.to_string()) };
            Ok((o, Box::new(std::io::Cursor::new(body.as_bytes().to_vec()))))
        }
    }

    #[test]
    fn gets_past_the_virus_scan_page() {
        let warn = r#"<html><form id="download-form" action="https://drive.usercontent.google.com/download" method="get">
            <input type="submit" id="uc-download-link" value="Download anyway"/>
            <input type="hidden" name="id" value="1aaaaaaaaaaaaaaaaaaaaaaa"><input type="hidden" name="export" value="download">
            <input type="hidden" name="confirm" value="t"><input type="hidden" name="uuid" value="ab-12"></form></html>"#;
        let net = Html {
            pages: vec![
                ("https://drive.usercontent.google.com/download?id=1aaaaaaaaaaaaaaaaaaaaaaa&export=download&confirm=t", "text/html; charset=utf-8", warn),
                ("https://drive.usercontent.google.com/download?id=1aaaaaaaaaaaaaaaaaaaaaaa&export=download&confirm=t&uuid=ab-12", "application/zip", "PKzip"),
            ],
        };
        let (url, size) = download_url(&net, "1aaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        assert!(url.ends_with("&uuid=ab-12"));
        assert_eq!(size, Some(5));
        // A page with no way through is an error, not a file.
        let net = Html { pages: vec![("https://drive.usercontent.google.com/download?id=1aaaaaaaaaaaaaaaaaaaaaaa&export=download&confirm=t", "text/html", "<p>Quota exceeded</p>")] };
        assert!(format!("{:#}", download_url(&net, "1aaaaaaaaaaaaaaaaaaaaaaa").unwrap_err()).contains("quota"));
    }

    #[test]
    fn ids() {
        assert_eq!(folder_id("https://drive.google.com/drive/folders/167UET84ZWXrMd9zpxEYiQWTQZexCGZfT?usp=sharing").as_deref(), Some(FOLDER));
        assert_eq!(folder_id(FOLDER).as_deref(), Some(FOLDER));
        assert_eq!(folder_id("../etc"), None);
        assert_eq!(file_id("https://drive.google.com/file/d/1P7a2FcJaQnyNMtI9Tfa9OTXvVkfB9nrx/edit").as_deref(), Some("1P7a2FcJaQnyNMtI9Tfa9OTXvVkfB9nrx"));
        assert_eq!(file_id("https://drive.google.com/open?id=1P7a2FcJaQnyNMtI9Tfa9OTXvVkfB9nrx").as_deref(), Some("1P7a2FcJaQnyNMtI9Tfa9OTXvVkfB9nrx"));
        assert_eq!(file_id("not an id"), None);
    }
}
