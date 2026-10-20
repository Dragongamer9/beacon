//! Editing the game's Options.ini (resolution lives here; the games have
//! no widescreen picker of their own for most modern resolutions).

use std::path::{Path, PathBuf};

/// Set (or add) `key = value`, keeping every other line as it was.
pub fn set_value(contents: &str, key: &str, value: &str) -> String {
    let mut found = false;
    let mut lines: Vec<String> = contents
        .lines()
        .map(|line| {
            let k = line.split('=').next().unwrap_or("").trim();
            if !found && line.contains('=') && k.eq_ignore_ascii_case(key) {
                found = true;
                format!("{key} = {value}")
            } else {
                line.to_string()
            }
        })
        .collect();
    if !found {
        lines.push(format!("{key} = {value}"));
    }
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

pub fn get_value(contents: &str, key: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        k.trim().eq_ignore_ascii_case(key).then(|| v.trim().to_string())
    })
}

/// Find `%APPDATA%` for the prefix's user. Proton uses `steamuser`, plain
/// Wine uses the login name; we take whichever real user folder exists.
pub fn appdata_dir(pfx: &Path) -> PathBuf {
    let users = pfx.join("drive_c/users");
    let found = std::fs::read_dir(&users).ok().and_then(|rd| {
        let mut names: Vec<String> = rd
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| !n.eq_ignore_ascii_case("public"))
            .collect();
        names.sort();
        names.iter().find(|n| *n == "steamuser").cloned().or_else(|| names.into_iter().next())
    });
    let user = found.unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "steamuser".into()));
    users.join(user).join("AppData/Roaming")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_existing_key() {
        let ini = "AudioLOD = High\r\nResolution = 800 600\r\nGamma = 50\r\n";
        let out = set_value(ini, "Resolution", "1920 1080");
        assert_eq!(out, "AudioLOD = High\r\nResolution = 1920 1080\r\nGamma = 50\r\n");
        assert_eq!(get_value(&out, "resolution").as_deref(), Some("1920 1080"));
    }

    #[test]
    fn appends_missing_key() {
        assert_eq!(set_value("", "Resolution", "2560 1440"), "Resolution = 2560 1440\r\n");
    }

    #[test]
    fn prefers_steamuser() {
        let t = tempfile::tempdir().unwrap();
        for u in ["Public", "alice", "steamuser"] {
            std::fs::create_dir_all(t.path().join("drive_c/users").join(u)).unwrap();
        }
        assert!(appdata_dir(t.path()).ends_with("drive_c/users/steamuser/AppData/Roaming"));
    }
}
