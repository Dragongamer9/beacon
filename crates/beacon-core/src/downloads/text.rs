//! Small text helpers for finding download links (std only): wildcard
//! patterns, links on a web page, relative URLs, version-aware sorting.

use std::cmp::Ordering;

/// Case-insensitive wildcard match: `*` is any run of characters, `?` one.
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// `%20` and friends decoded (invalid escapes are kept as they are).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The HTML entities that show up in links.
fn unescape_html(s: &str) -> String {
    s.replace("&amp;", "&").replace("&#38;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&#x2F;", "/").replace("&#47;", "/")
}

/// The last path segment of a URL, decoded (no query or fragment).
pub fn file_name(url: &str) -> String {
    let no_q = url.split(['?', '#']).next().unwrap_or(url);
    let seg = no_q.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    percent_decode(seg)
}

/// A link found on a page: where it goes, and the text it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    pub text: String,
}

/// Links on a page: `href`, `src` and `data-*` URL attributes, plus bare
/// `https://` URLs in the text (scripts often carry the real download link).
/// Relative links are resolved against `base`. Duplicates are dropped.
pub fn links(html: &str, base: &str) -> Vec<Link> {
    let mut out: Vec<Link> = Vec::new();
    let mut push = |url: String, text: String| {
        if let Some(abs) = join(base, &url) {
            if !out.iter().any(|l| l.url == abs) {
                out.push(Link { url: abs, text });
            }
        }
    };
    let lower = html.to_ascii_lowercase();
    // Attributes.
    for attr in ["href=", "src=", "data-href=", "data-url=", "data-download="] {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(attr) {
            let start = from + pos + attr.len();
            from = start;
            // The attribute name must start a word (not "xhref=").
            let before = lower[..start - attr.len()].chars().last();
            if before.map(|c| c.is_ascii_alphanumeric() || c == '-').unwrap_or(false) {
                continue;
            }
            let rest = &html[start..];
            let (val, used) = match rest.chars().next() {
                Some(q @ ('"' | '\'')) => match rest[1..].find(q) {
                    Some(end) => (&rest[1..1 + end], end + 2),
                    None => continue,
                },
                Some(_) => {
                    let end = rest.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(rest.len());
                    (&rest[..end], end)
                }
                None => continue,
            };
            let val = unescape_html(val.trim());
            if val.is_empty() || val.starts_with('#') || val.starts_with("javascript:") || val.starts_with("mailto:") {
                continue;
            }
            // Link text: up to the closing </a>, tags stripped, for <a href>.
            let mut text = String::new();
            if attr == "href=" {
                let after = start + used;
                if let Some(gt) = html[after..].find('>') {
                    let body_start = after + gt + 1;
                    let body_end = lower[body_start..].find("</a").map(|e| body_start + e).unwrap_or(body_start);
                    text = strip_tags(&html[body_start..body_end.min(body_start + 400).max(body_start)]);
                }
            }
            push(val, text);
        }
    }
    // Bare URLs.
    let mut from = 0;
    while let Some(pos) = lower[from..].find("https://") {
        let start = from + pos;
        let end = html[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | ')' | '\\'))
            .map(|e| start + e)
            .unwrap_or(html.len());
        from = end.max(start + 8);
        push(unescape_html(&html[start..end]), String::new());
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    unescape_html(&out.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// `href` as an absolute https URL against `base`; `None` for other schemes.
pub fn join(base: &str, href: &str) -> Option<String> {
    let h = href.trim();
    if h.len() > 4096 || h.chars().any(|c| c.is_control()) {
        return None;
    }
    let lower = h.to_ascii_lowercase();
    if lower.starts_with("https://") {
        return Some(h.to_string());
    }
    // Any other scheme (plain http included) isn't followed.
    if let Some(i) = lower.find([':', '/', '?', '#']) {
        if lower.as_bytes()[i] == b':' && i > 0 && lower[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) {
            return None;
        }
    }
    let b = base.trim();
    if !b.to_ascii_lowercase().starts_with("https://") {
        return None;
    }
    let after_scheme = &b[8..];
    let host_end = after_scheme.find('/').map(|i| 8 + i).unwrap_or(b.len());
    let origin = &b[..host_end];
    if let Some(rest) = h.strip_prefix("//") {
        return Some(format!("https://{rest}"));
    }
    if h.starts_with('/') {
        return Some(format!("{origin}{h}"));
    }
    // Relative to the base's folder.
    let path = b[host_end..].split(['?', '#']).next().unwrap_or("");
    let dir = match path.rfind('/') {
        Some(i) => &path[..=i],
        None => "/",
    };
    let mut segs: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    let (rel_path, tail) = match h.find(['?', '#']) {
        Some(i) => (&h[..i], &h[i..]),
        None => (h, ""),
    };
    let rel_segs: Vec<&str> = rel_path.split('/').collect();
    for (i, s) in rel_segs.iter().enumerate() {
        match *s {
            "." => {}
            ".." => {
                segs.pop();
            }
            "" if i + 1 < rel_segs.len() => {}
            s => segs.push(s),
        }
    }
    let trailing = rel_path.ends_with('/') && !segs.is_empty();
    Some(format!("{origin}/{}{}{tail}", segs.join("/"), if trailing { "/" } else { "" }))
}

/// Compare names so `v10` sorts after `v9` (runs of digits compare as numbers).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(char::is_ascii_digit) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(char::is_ascii_digit) {
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

/// The literal words of a wildcard pattern (`AotR_Standalone_*_Installer.zip`
/// → aotr, standalone, installer, zip), for judging near misses.
pub fn pattern_words(pattern: &str) -> Vec<String> {
    words(&pattern.replace(['*', '?'], " "))
}

/// Lowercase words (letters and digits) of a name.
pub fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// How much of the pattern's words a name carries, 0.0 to 1.0.
pub fn similarity(pattern: &str, name: &str) -> f32 {
    let want = pattern_words(pattern);
    if want.is_empty() {
        return 0.0;
    }
    let have = words(name);
    let hits = want.iter().filter(|w| have.iter().any(|h| h == *w)).count();
    hits as f32 / want.len() as f32
}

/// A pattern's file extension (`*.zip` → `zip`), if it ends in a plain one.
pub fn extension(pattern: &str) -> Option<String> {
    let name = pattern.rsplit('/').next().unwrap_or(pattern);
    let (_, ext) = name.rsplit_once('.')?;
    (!ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric())).then(|| ext.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob("AotR_Standalone_*_Installer.zip", "aotr_standalone_9.2_installer.zip"));
        assert!(glob("*.zip", "a.zip"));
        assert!(!glob("*.zip", "a.zip.part"));
        assert!(glob("file?.7z", "file1.7z"));
        assert!(!glob("file?.7z", "file12.7z"));
        assert!(glob("*", ""));
        assert!(glob("*/downloads/start/*", "https://www.moddb.com/downloads/start/123"));
    }

    #[test]
    fn decode() {
        assert_eq!(percent_decode("My%20Mod%2B1.zip"), "My Mod+1.zip");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("a%zz"), "a%zz");
        assert_eq!(file_name("https://x.com/a/b/My%20File.zip?x=1#y"), "My File.zip");
    }

    #[test]
    fn joins() {
        let base = "https://www.moddb.com/mods/edain/downloads?page=2";
        assert_eq!(join(base, "/downloads/start/1").unwrap(), "https://www.moddb.com/downloads/start/1");
        assert_eq!(join(base, "x.zip").unwrap(), "https://www.moddb.com/mods/edain/x.zip");
        assert_eq!(join(base, "../a/b.zip?q=1").unwrap(), "https://www.moddb.com/mods/a/b.zip?q=1");
        assert_eq!(join(base, "//cdn.example.com/f.zip").unwrap(), "https://cdn.example.com/f.zip");
        assert_eq!(join(base, "https://other.org/z").unwrap(), "https://other.org/z");
        assert_eq!(join(base, "http://insecure.org/z"), None);
        assert_eq!(join(base, "ftp://a/b"), None);
        assert_eq!(join("https://a.org", "f.zip").unwrap(), "https://a.org/f.zip");
    }

    #[test]
    fn finds_links() {
        let html = r##"<a class="x" href="/downloads/start/281"><span>Download</span> now</a>
            <img src='img/logo.png'><a href=mirror/1.zip>m</a>
            <script>var u = "https://cdn.example.com/AotR_Standalone_9.2_Installer.zip";</script>
            <a xhref="/no">n</a><a href="#top">t</a><a href="/downloads/start/281">dup</a>"##;
        let l = links(html, "https://www.moddb.com/mods/aotr/downloads/");
        let urls: Vec<&str> = l.iter().map(|x| x.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://www.moddb.com/downloads/start/281",
                "https://www.moddb.com/mods/aotr/downloads/mirror/1.zip",
                "https://www.moddb.com/mods/aotr/downloads/img/logo.png",
                "https://cdn.example.com/AotR_Standalone_9.2_Installer.zip",
            ]
        );
        assert_eq!(l[0].text, "Download now");
    }

    #[test]
    fn natural() {
        let mut v = vec!["AotR_9.10.zip", "AotR_9.2.zip", "AotR_10.0.zip", "AotR_9.9.zip"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["AotR_9.2.zip", "AotR_9.9.zip", "AotR_9.10.zip", "AotR_10.0.zip"]);
    }

    #[test]
    fn near_misses() {
        assert!(similarity("AotR_Standalone_*_Installer.zip", "AotR-Standalone-Setup-9.3.zip") >= 0.6);
        assert!(similarity("AotR_Standalone_*_Installer.zip", "edain_4.7.zip") < 0.3);
        assert_eq!(extension("AotR_*_Installer.zip").as_deref(), Some("zip"));
        assert_eq!(extension("Installer*"), None);
    }
}
