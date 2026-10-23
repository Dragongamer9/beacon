//! Beacon's own download engine: the base games from
//! the BFME Foundation's CDN (the same packages the All-in-One launcher
//! installs), and later mods and other games, all resolved through
//! `sources.json` so a moved link is found again instead of breaking.
//!
//! * [`sources`]: what can be downloaded and how to find it right now
//!   (workshop API, web pages scanned for a matching link, GitHub releases,
//!   direct URLs), the bundled list, the copy updated from the private repo,
//!   and "Check sources".
//! * [`basegame`]: BFME, BFME II and RotWK installed straight from the CDN.
//! * This file: the transfer layer: https only, resumable `.part` files,
//!   size and MD5 checks, size limits.

pub mod archive;
pub mod basegame;
pub mod drive;
pub mod launcher;
pub mod moddb;
pub mod modget;
pub mod selfupdate;
pub mod sources;
pub mod text;

use anyhow::{anyhow, bail, Context, Result};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Largest single file the engine downloads.
pub const MAX_FILE_BYTES: u64 = 16 << 30;
/// Largest page or API answer read.
pub const MAX_TEXT_BYTES: u64 = 16 << 20;

/// What a server said about a URL before any bytes are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    /// The server honoured the `Range` request (HTTP 206).
    pub partial: bool,
    /// Bytes in this answer (`Content-Length`), when given.
    pub len: Option<u64>,
    /// Where the request ended up after redirects.
    pub final_url: String,
    pub content_type: Option<String>,
}

/// The network, so the engine can be tested without it.
pub trait Transfer: Send + Sync {
    /// GET `url` from byte `offset` on (a `Range` request when `offset > 0`),
    /// with extra request headers.
    fn open(&self, url: &str, offset: u64, headers: &[(&str, String)]) -> Result<(Opened, Box<dyn Read + Send>)>;

    /// Where `url` redirects to, without following it (so headers like a GitHub token
    /// never reach the other host). `None` when it doesn't redirect.
    fn redirect_target(&self, _url: &str, _headers: &[(&str, String)]) -> Result<Option<String>> {
        Ok(None)
    }

    /// GET a text answer, at most `max` bytes.
    fn get_text(&self, url: &str, headers: &[(&str, String)], max: u64) -> Result<String> {
        let (_, mut r) = self.open(url, 0, headers)?;
        let mut buf = Vec::new();
        r.by_ref().take(max + 1).read_to_end(&mut buf).with_context(|| format!("reading {url}"))?;
        if buf.len() as u64 > max {
            bail!("the answer from {} is larger than expected", host(url));
        }
        String::from_utf8(buf).map_err(|_| anyhow!("the answer from {} isn't text", host(url)))
    }

    /// GET a JSON answer.
    fn get_json(&self, url: &str, headers: &[(&str, String)]) -> Result<serde_json::Value> {
        let t = self.get_text(url, headers, MAX_TEXT_BYTES)?;
        serde_json::from_str(&t).with_context(|| format!("the answer from {} isn't JSON", host(url)))
    }
}

/// `https://host/…` → `host`, for messages.
pub fn host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or(rest).to_string()
}

/// Is this an https URL with nothing odd in it?
pub fn is_https(url: &str) -> bool {
    let u = url.trim();
    u.len() > 8 && u[..8].eq_ignore_ascii_case("https://") && !u.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// The real network: ureq over rustls, https only (redirects too), timeouts.
pub struct Client {
    agent: ureq::Agent,
    /// The same, without following redirects.
    no_redirects: ureq::Agent,
}

impl Client {
    pub fn new() -> Client {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(90))
            .timeout_write(Duration::from_secs(60))
            .https_only(true)
            .redirects(10)
            .try_proxy_from_env(true)
            .user_agent(concat!("beacon/", env!("CARGO_PKG_VERSION")))
            .build();
        let no_redirects = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .https_only(true)
            .redirects(0)
            .try_proxy_from_env(true)
            .user_agent(concat!("beacon/", env!("CARGO_PKG_VERSION")))
            .build();
        Client { agent, no_redirects }
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

impl Transfer for Client {
    fn redirect_target(&self, url: &str, headers: &[(&str, String)]) -> Result<Option<String>> {
        if !is_https(url) {
            bail!("not an https link: {url}");
        }
        let mut req = self.no_redirects.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => bail!("{} answered HTTP {code}", host(url)),
            Err(ureq::Error::Transport(t)) => bail!("couldn't reach {}: {t}", host(url)),
        };
        if (300..400).contains(&resp.status()) {
            let to = resp.header("Location").map(str::to_string).ok_or_else(|| anyhow!("{} redirected nowhere", host(url)))?;
            let to = text::join(url, &to).ok_or_else(|| anyhow!("{} redirected to a link that isn't https", host(url)))?;
            return Ok(Some(to));
        }
        Ok(None)
    }

    fn open(&self, url: &str, offset: u64, headers: &[(&str, String)]) -> Result<(Opened, Box<dyn Read + Send>)> {
        if !is_https(url) {
            bail!("not an https link: {url}");
        }
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        if offset > 0 {
            req = req.set("Range", &format!("bytes={offset}-"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            // Asking past the end of a finished file.
            Err(ureq::Error::Status(416, r)) if offset > 0 => {
                let opened = Opened { partial: true, len: Some(0), final_url: r.get_url().to_string(), content_type: None };
                return Ok((opened, Box::new(std::io::empty())));
            }
            Err(ureq::Error::Status(code, _)) => bail!("{} answered HTTP {code}", host(url)),
            Err(ureq::Error::Transport(t)) => bail!("couldn't reach {}: {t}", host(url)),
        };
        let opened = Opened {
            partial: resp.status() == 206,
            len: resp.header("Content-Length").and_then(|l| l.trim().parse().ok()),
            final_url: resp.get_url().to_string(),
            content_type: resp.header("Content-Type").map(str::to_string),
        };
        if !is_https(&opened.final_url) {
            bail!("{} redirected to a link that isn't https", host(url));
        }
        Ok((opened, Box::new(resp.into_reader())))
    }
}

// ---------------------------------------------------------------- pausing

/// A transfer that stops when `flag` is set: the next read fails with "paused", leaving
/// the `.part` file for a later run to resume from.
pub struct Cancellable<T: Transfer> {
    pub inner: T,
    pub flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// The error a stopped transfer gives.
pub const PAUSED: &str = "paused";

impl<T: Transfer> Cancellable<T> {
    pub fn new(inner: T, flag: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        Cancellable { inner, flag }
    }
    fn stopped(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Relaxed)
    }
}

struct StopReader {
    inner: Box<dyn Read + Send>,
    flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Read for StopReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.flag.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, PAUSED));
        }
        self.inner.read(buf)
    }
}

impl<T: Transfer> Transfer for Cancellable<T> {
    fn open(&self, url: &str, offset: u64, headers: &[(&str, String)]) -> Result<(Opened, Box<dyn Read + Send>)> {
        if self.stopped() {
            bail!(PAUSED);
        }
        let (o, r) = self.inner.open(url, offset, headers)?;
        Ok((o, Box::new(StopReader { inner: r, flag: self.flag.clone() })))
    }
    fn redirect_target(&self, url: &str, headers: &[(&str, String)]) -> Result<Option<String>> {
        if self.stopped() {
            bail!(PAUSED);
        }
        self.inner.redirect_target(url, headers)
    }
}

/// Was this error a pause (not a failure)?
pub fn is_paused(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.to_string() == PAUSED)
}

/// Bytes kept in the download folders (finished downloads waiting to be used, partial ones, update packages).
pub fn kept_bytes(store_dir: &Path) -> (u64, usize) {
    let mut bytes = 0;
    let mut n = 0;
    for sub in ["files", "self-update"] {
        for e in walkdir::WalkDir::new(store_dir.join(sub)).into_iter().flatten() {
            if e.file_type().is_file() {
                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
                n += 1;
            }
        }
    }
    (bytes, n)
}

/// Remove kept downloads (finished and partial) and old update packages.
pub fn clear_kept(store_dir: &Path) -> Result<u64> {
    let (bytes, _) = kept_bytes(store_dir);
    for sub in ["files", "self-update"] {
        let d = store_dir.join(sub);
        if d.exists() {
            std::fs::remove_dir_all(&d).with_context(|| format!("removing {}", d.display()))?;
        }
    }
    Ok(bytes)
}

// ---------------------------------------------------------------- files

/// One file to fetch.
#[derive(Debug, Clone, PartialEq)]
pub struct FileJob {
    pub url: String,
    pub dest: PathBuf,
    /// Expected size, when the source says.
    pub size: Option<u64>,
    /// Expected MD5 (lowercase hex), when the source says.
    pub md5: Option<String>,
}

/// What `fetch_file` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched {
    /// The file was already there and matched.
    Already,
    /// Downloaded (`bytes` this time; less than the size when it resumed).
    Downloaded { bytes: u64, resumed: bool },
}

/// The `.part` file a download goes to before it's checked.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

/// MD5 of a file, lowercase hex.
pub fn md5_file(path: &Path) -> Result<String> {
    use md5::Digest;
    let mut f = std::fs::File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut h = md5::Md5::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Does the file at `path` match the job? `thorough` also compares the MD5
/// (otherwise the size is enough, like the All-in-One launcher's fast check).
pub fn matches(job: &FileJob, path: &Path, thorough: bool) -> bool {
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if !meta.is_file() {
        return false;
    }
    let size_ok = job.size.map(|s| meta.len() == s);
    match (&job.md5, size_ok) {
        // The size matches: that's enough for a quick check.
        (Some(want), Some(true)) => !thorough || md5_file(path).map(|h| &h == want).unwrap_or(false),
        // No size, or one that doesn't match (sources list sizes wrong now and then): the checksum decides.
        (Some(want), _) => md5_file(path).map(|h| &h == want).unwrap_or(false),
        (None, Some(ok)) => ok,
        (None, None) => true,
    }
}

/// Get one file: skip it when it's already there and matches; otherwise
/// download into `<dest>.part` (resuming one left by an earlier try), check
/// size and MD5, and move it into place. A bad `.part` is thrown away and
/// downloaded once more from the start. `progress(bytes of this file so far)`.
pub fn fetch_file(t: &dyn Transfer, job: &FileJob, thorough: bool, progress: &mut dyn FnMut(u64)) -> Result<Fetched> {
    if matches(job, &job.dest, thorough) {
        return Ok(Fetched::Already);
    }
    if let Some(dir) = job.dest.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let part = part_path(&job.dest);
    let mut last_err = None;
    for attempt in 0..2 {
        if attempt > 0 {
            let _ = std::fs::remove_file(&part);
        }
        match download_part(t, job, &part, progress) {
            Ok((bytes, resumed)) => match check_part(job, &part) {
                Ok(()) => {
                    // Replace whatever was there (an older or damaged copy).
                    if job.dest.exists() {
                        std::fs::remove_file(&job.dest).with_context(|| format!("replacing {}", job.dest.display()))?;
                    }
                    std::fs::rename(&part, &job.dest).with_context(|| format!("moving the download to {}", job.dest.display()))?;
                    return Ok(Fetched::Downloaded { bytes, resumed });
                }
                Err(e) => last_err = Some(e),
            },
            Err(e) => {
                // A network error keeps the .part for the next try (resume);
                // only a size or checksum problem starts it over.
                return Err(e);
            }
        }
    }
    let _ = std::fs::remove_file(&part);
    Err(last_err.unwrap_or_else(|| anyhow!("download failed")))
}

fn download_part(t: &dyn Transfer, job: &FileJob, part: &Path, progress: &mut dyn FnMut(u64)) -> Result<(u64, bool)> {
    // With an MD5 to check against, the listed size is only a hint (sources get it wrong now and
    // then; the checksum is what counts). Without one, the size is the limit.
    let max = if job.md5.is_some() { MAX_FILE_BYTES } else { job.size.unwrap_or(MAX_FILE_BYTES).min(MAX_FILE_BYTES) };
    let mut have = std::fs::symlink_metadata(part).ok().filter(|m| m.is_file()).map(|m| m.len()).unwrap_or(0);
    if have > max {
        std::fs::remove_file(part)?;
        have = 0;
    }
    if job.size == Some(have) && have > 0 {
        return Ok((0, true));
    }
    let (opened, mut r) = t.open(&job.url, have, &[])?;
    let resumed = have > 0 && opened.partial;
    let mut file = if resumed {
        std::fs::OpenOptions::new().append(true).open(part)?
    } else {
        // The server sent the whole file: start over.
        have = 0;
        std::fs::File::create(part).with_context(|| format!("creating {}", part.display()))?
    };
    if let Some(len) = opened.len {
        if have + len > max {
            bail!("{} is {} bytes, more than expected ({max})", label(job), have + len);
        }
    }
    let mut buf = vec![0u8; 256 * 1024];
    let mut total = have;
    let mut got = 0u64;
    loop {
        let n = r.read(&mut buf).with_context(|| format!("downloading from {}", host(&job.url)))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        got += n as u64;
        if total > max {
            bail!("{} is larger than expected ({max} bytes)", label(job));
        }
        file.write_all(&buf[..n])?;
        progress(total);
    }
    file.flush()?;
    file.sync_all().ok();
    Ok((got, resumed))
}

/// The file's name for messages (the destination's, not the CDN's hash-named URL).
fn label(job: &FileJob) -> String {
    job.dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| text::file_name(&job.url))
}

fn check_part(job: &FileJob, part: &Path) -> Result<()> {
    let len = std::fs::metadata(part)?.len();
    let name = label(job);
    if let Some(want) = &job.md5 {
        // The checksum is the real test; a size that differs from the listing is fine when it matches.
        let have = md5_file(part)?;
        if &have != want {
            bail!("{name}: checksum doesn't match (MD5 {have}, expected {want})");
        }
        return Ok(());
    }
    if let Some(size) = job.size {
        if len != size {
            bail!("{name}: got {len} bytes, expected {size}");
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod testnet {
    //! A fake network for tests: URL → bytes, with Range support and a log.
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct FakeNet {
        pub files: Mutex<HashMap<String, Vec<u8>>>,
        pub log: Mutex<Vec<(String, u64)>>,
        /// Ignore Range requests (send the whole file).
        pub no_ranges: bool,
        /// Fail after this many bytes of the next answer (a dropped connection).
        pub cut_after: Mutex<Option<usize>>,
        /// URL → where it redirects (for `redirect_target`).
        pub redirects: Mutex<HashMap<String, String>>,
    }

    impl FakeNet {
        pub fn with(files: &[(&str, &[u8])]) -> FakeNet {
            let f = FakeNet::default();
            for (u, b) in files {
                f.files.lock().unwrap().insert(u.to_string(), b.to_vec());
            }
            f
        }
        pub fn put(&self, url: &str, body: &[u8]) {
            self.files.lock().unwrap().insert(url.to_string(), body.to_vec());
        }
        pub fn requests(&self) -> Vec<(String, u64)> {
            self.log.lock().unwrap().clone()
        }
    }

    struct Cut {
        data: std::io::Cursor<Vec<u8>>,
        left: usize,
    }

    impl Read for Cut {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset"));
            }
            let n = buf.len().min(self.left);
            let got = self.data.read(&mut buf[..n])?;
            if got == 0 {
                return Ok(0);
            }
            self.left -= got;
            Ok(got)
        }
    }

    impl Transfer for FakeNet {
        fn redirect_target(&self, url: &str, _headers: &[(&str, String)]) -> Result<Option<String>> {
            self.log.lock().unwrap().push((url.to_string(), 0));
            Ok(self.redirects.lock().unwrap().get(url).cloned())
        }

        fn open(&self, url: &str, offset: u64, _headers: &[(&str, String)]) -> Result<(Opened, Box<dyn Read + Send>)> {
            self.log.lock().unwrap().push((url.to_string(), offset));
            let body = self.files.lock().unwrap().get(url).cloned().ok_or_else(|| anyhow!("{} answered HTTP 404", host(url)))?;
            let (partial, data) = if offset > 0 && !self.no_ranges { (true, body[(offset as usize).min(body.len())..].to_vec()) } else { (false, body) };
            let opened = Opened { partial, len: Some(data.len() as u64), final_url: url.to_string(), content_type: None };
            let cut = self.cut_after.lock().unwrap().take();
            let r: Box<dyn Read + Send> = match cut {
                Some(n) => Box::new(Cut { data: std::io::Cursor::new(data), left: n }),
                None => Box::new(std::io::Cursor::new(data)),
            };
            Ok((opened, r))
        }
    }

    pub fn md5_hex(data: &[u8]) -> String {
        use md5::Digest;
        md5::Md5::digest(data).iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testnet::*;
    use super::*;

    fn job(dir: &Path, url: &str, body: &[u8]) -> FileJob {
        FileJob { url: url.into(), dest: dir.join("sub/file.big"), size: Some(body.len() as u64), md5: Some(md5_hex(body)) }
    }

    #[test]
    fn downloads_checks_and_skips_next_time() {
        let t = tempfile::tempdir().unwrap();
        let body = vec![7u8; 300_000];
        let net = FakeNet::with(&[("https://cdn.example/f", &body)]);
        let j = job(t.path(), "https://cdn.example/f", &body);
        let r = fetch_file(&net, &j, false, &mut |_| {}).unwrap();
        assert_eq!(r, Fetched::Downloaded { bytes: 300_000, resumed: false });
        assert_eq!(std::fs::read(&j.dest).unwrap(), body);
        assert!(!part_path(&j.dest).exists());
        assert_eq!(fetch_file(&net, &j, true, &mut |_| {}).unwrap(), Fetched::Already);
        assert_eq!(net.requests().len(), 1);
    }

    #[test]
    fn resumes_a_dropped_download() {
        let t = tempfile::tempdir().unwrap();
        let body: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
        let net = FakeNet::with(&[("https://cdn.example/f", &body)]);
        let j = job(t.path(), "https://cdn.example/f", &body);
        *net.cut_after.lock().unwrap() = Some(123_456);
        assert!(fetch_file(&net, &j, false, &mut |_| {}).is_err());
        assert_eq!(std::fs::metadata(part_path(&j.dest)).unwrap().len(), 123_456);
        let r = fetch_file(&net, &j, false, &mut |_| {}).unwrap();
        assert_eq!(r, Fetched::Downloaded { bytes: 500_000 - 123_456, resumed: true });
        assert_eq!(std::fs::read(&j.dest).unwrap(), body);
        assert_eq!(net.requests().last().unwrap().1, 123_456);
    }

    #[test]
    fn server_without_ranges_starts_over() {
        let t = tempfile::tempdir().unwrap();
        let body = vec![1u8; 10_000];
        let mut net = FakeNet::with(&[("https://cdn.example/f", &body)]);
        net.no_ranges = true;
        let j = job(t.path(), "https://cdn.example/f", &body);
        std::fs::create_dir_all(j.dest.parent().unwrap()).unwrap();
        std::fs::write(part_path(&j.dest), vec![1u8; 4_000]).unwrap();
        let r = fetch_file(&net, &j, false, &mut |_| {}).unwrap();
        assert_eq!(r, Fetched::Downloaded { bytes: 10_000, resumed: false });
        assert_eq!(std::fs::read(&j.dest).unwrap(), body);
    }

    #[test]
    fn bad_checksum_is_refused_and_not_kept() {
        let t = tempfile::tempdir().unwrap();
        let body = vec![3u8; 1000];
        let net = FakeNet::with(&[("https://cdn.example/f", &body)]);
        let mut j = job(t.path(), "https://cdn.example/f", &body);
        j.md5 = Some("0".repeat(32));
        let e = fetch_file(&net, &j, false, &mut |_| {}).unwrap_err();
        assert!(format!("{e}").contains("checksum"));
        assert!(!j.dest.exists() && !part_path(&j.dest).exists());
        // Tried twice (once more from the start).
        assert_eq!(net.requests().len(), 2);
    }

    #[test]
    fn replaces_a_damaged_copy() {
        let t = tempfile::tempdir().unwrap();
        let body = vec![9u8; 2048];
        let net = FakeNet::with(&[("https://cdn.example/f", &body)]);
        let j = job(t.path(), "https://cdn.example/f", &body);
        std::fs::create_dir_all(j.dest.parent().unwrap()).unwrap();
        std::fs::write(&j.dest, vec![0u8; 2048]).unwrap();
        // Same size: the fast check passes, the thorough one doesn't.
        assert_eq!(fetch_file(&net, &j, false, &mut |_| {}).unwrap(), Fetched::Already);
        assert!(matches!(fetch_file(&net, &j, true, &mut |_| {}).unwrap(), Fetched::Downloaded { .. }));
        assert_eq!(std::fs::read(&j.dest).unwrap(), body);
    }

    #[test]
    fn a_wrong_listed_size_is_fine_when_the_checksum_matches() {
        let t = tempfile::tempdir().unwrap();
        let body = vec![5u8; 50_000];
        let net = FakeNet::with(&[("https://cdn.example/f", &body)]);
        // Listed smaller (stale) and listed as 0: both download, the MD5 decides.
        for listed in [1_000u64, 0] {
            let mut j = job(t.path(), "https://cdn.example/f", &body);
            j.size = Some(listed);
            let _ = std::fs::remove_file(&j.dest);
            assert!(matches!(fetch_file(&net, &j, false, &mut |_| {}).unwrap(), Fetched::Downloaded { .. }), "listed {listed}");
            assert_eq!(std::fs::read(&j.dest).unwrap(), body);
            // And it's recognised as done afterwards.
            assert_eq!(fetch_file(&net, &j, false, &mut |_| {}).unwrap(), Fetched::Already);
        }
    }

    #[test]
    fn too_large_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let body = vec![3u8; 1000];
        let net = FakeNet::with(&[("https://cdn.example/f", &body)]);
        let mut j = job(t.path(), "https://cdn.example/f", &body);
        j.size = Some(10);
        j.md5 = None;
        assert!(fetch_file(&net, &j, false, &mut |_| {}).is_err());
    }

    #[test]
    fn pausing_keeps_the_part_and_resuming_finishes() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let t = tempfile::tempdir().unwrap();
        let body: Vec<u8> = (0..800_000u32).map(|i| (i % 7) as u8).collect();
        let flag = Arc::new(AtomicBool::new(false));
        let net = Cancellable::new(FakeNet::with(&[("https://cdn.example/f", &body)]), flag.clone());
        let j = job(t.path(), "https://cdn.example/f", &body);
        let f2 = flag.clone();
        let e = fetch_file(&net, &j, false, &mut |b| {
            if b > 300_000 {
                f2.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
        assert!(is_paused(&e), "{e:#}");
        let kept = std::fs::metadata(part_path(&j.dest)).unwrap().len();
        assert!(kept > 300_000 && kept < 800_000);
        flag.store(false, Ordering::Relaxed);
        assert!(matches!(fetch_file(&net, &j, false, &mut |_| {}).unwrap(), Fetched::Downloaded { resumed: true, .. }));
        assert_eq!(std::fs::read(&j.dest).unwrap(), body);
        let (bytes, n) = kept_bytes(t.path());
        assert_eq!((bytes, n), (0, 0)); // job() writes under sub/, not files/
        std::fs::create_dir_all(t.path().join("files/mods")).unwrap();
        std::fs::write(t.path().join("files/mods/x.zip"), b"1234").unwrap();
        assert_eq!(kept_bytes(t.path()), (4, 1));
        assert_eq!(clear_kept(t.path()).unwrap(), 4);
        assert_eq!(kept_bytes(t.path()), (0, 0));
    }

    #[test]
    fn hosts() {
        assert_eq!(host("https://bfmeladder.com/api/x?y"), "bfmeladder.com");
        assert!(is_https("HTTPS://a.b/c"));
        assert!(!is_https("http://a.b/c"));
        assert!(!is_https("https://a b"));
    }
}
