//! Play time and last played, per game: `<data>/playtime.json`, `{"<key>": {"seconds": n, "last_played": unix,
//! "sessions": n}}`. Keys are a built-in game's slug ("rotwk") or "custom:<id>" for games added by hand.
//!
//! Kept out of config.json on purpose: a launch that waits for the game writes here when the game closes,
//! hours after it started, and reads the file again right before writing (so a change made meanwhile, in
//! Beacon's window or another `beacon`, is kept).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayStats {
    pub seconds: u64,
    /// Unix time (seconds) the game last closed; 0 = never.
    pub last_played: u64,
    pub sessions: u64,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn load(file: &Path) -> BTreeMap<String, PlayStats> {
    std::fs::read_to_string(file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn get(file: &Path, key: &str) -> PlayStats {
    load(file).get(key).copied().unwrap_or_default()
}

fn save(file: &Path, all: &BTreeMap<String, PlayStats>) -> Result<()> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = file.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, serde_json::to_string_pretty(all)?)?;
    std::fs::rename(&tmp, file)?;
    Ok(())
}

/// Adds a session that just ended (sessions shorter than 5 seconds count as tries, not play: only the time
/// they ended is kept).
pub fn record(file: &Path, key: &str, seconds: u64) -> Result<PlayStats> {
    let mut all = load(file);
    let e = all.entry(key.to_string()).or_default();
    if seconds >= 5 {
        e.seconds += seconds;
        e.sessions += 1;
    }
    e.last_played = now();
    let out = *e;
    save(file, &all)?;
    Ok(out)
}

/// Sets a game's totals (an import from Faugus Launcher or another Beacon), keeping the larger of each.
pub fn merge(file: &Path, key: &str, s: PlayStats) -> Result<()> {
    let mut all = load(file);
    let e = all.entry(key.to_string()).or_default();
    e.seconds = e.seconds.max(s.seconds);
    e.last_played = e.last_played.max(s.last_played);
    e.sessions = e.sessions.max(s.sessions);
    save(file, &all)
}

pub fn forget(file: &Path, key: &str) -> Result<()> {
    let mut all = load(file);
    if all.remove(key).is_some() {
        save(file, &all)?;
    }
    Ok(())
}

/// "2025-07-14T12:34:56.123456" or "2025-07-14 12:34:56" (local time, taken as UTC: close enough for "last
/// played") to Unix seconds.
pub fn parse_iso(s: &str) -> Option<u64> {
    let s = s.trim();
    let (date, time) = s.split_once(['T', ' ']).unwrap_or((s, "00:00:00"));
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || y < 1970 {
        return None;
    }
    let time = time.split(['.', '+', 'Z']).next().unwrap_or("");
    let mut t = time.split(':').map(|x| x.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next().flatten().unwrap_or(0), t.next().flatten().unwrap_or(0), t.next().flatten().unwrap_or(0));
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hh * 3600 + mm * 60 + ss;
    (secs >= 0).then_some(secs as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_merges() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("d/playtime.json");
        assert_eq!(get(&f, "rotwk"), PlayStats::default());
        let s = record(&f, "rotwk", 600).unwrap();
        assert_eq!((s.seconds, s.sessions), (600, 1));
        assert!(s.last_played > 0);
        record(&f, "rotwk", 2).unwrap();
        assert_eq!(get(&f, "rotwk").seconds, 600, "a 2-second try isn't play time");
        merge(&f, "custom:x", PlayStats { seconds: 3600, last_played: 10, sessions: 0 }).unwrap();
        merge(&f, "custom:x", PlayStats { seconds: 100, last_played: 20, sessions: 0 }).unwrap();
        assert_eq!(get(&f, "custom:x"), PlayStats { seconds: 3600, last_played: 20, sessions: 0 });
        forget(&f, "custom:x").unwrap();
        assert_eq!(get(&f, "custom:x").seconds, 0);
        assert_eq!(get(&f, "rotwk").seconds, 600);
    }

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso("1970-01-01T00:00:00"), Some(0));
        assert_eq!(parse_iso("2000-03-01T00:00:00"), Some(951868800));
        assert_eq!(parse_iso("2025-07-14T12:34:56.123456"), Some(1752496496));
        assert_eq!(parse_iso("2025-07-14"), Some(1752451200));
        assert_eq!(parse_iso("yesterday"), None);
        assert_eq!(parse_iso("2025-13-01"), None);
    }
}
