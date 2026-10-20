//! The EA registry keys the games look for, written as a .reg file and
//! imported with Wine's regedit. Without these the games refuse to start
//! ("please reinstall") because they can't find their install path.

use crate::games::GameId;
use std::path::Path;

/// `/home/me/Games/BFME2` → `Z:\home\me\Games\BFME2\` (Wine maps Z: to /).
pub fn wine_path(p: &Path, trailing_slash: bool) -> String {
    let mut s = String::from("Z:");
    s.push_str(&p.to_string_lossy().replace('/', "\\"));
    if trailing_slash && !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub struct RegGame<'a> {
    pub game: GameId,
    pub install_dir: &'a Path,
    pub cd_key: Option<&'a str>,
}

/// Build a REGEDIT4 file covering `games`, written to both the native and
/// the WOW6432Node views so 32-bit games find it in a 64-bit prefix.
pub fn build_reg(games: &[RegGame], language: &str) -> String {
    let mut out = String::from("REGEDIT4\r\n");
    for view in ["SOFTWARE", "SOFTWARE\\WOW6432Node"] {
        for g in games {
            let key = format!("HKEY_LOCAL_MACHINE\\{view}\\Electronic Arts\\EA Games\\{}", g.game.registry_name());
            out.push_str(&format!("\r\n[{key}]\r\n"));
            out.push_str(&format!("\"InstallPath\"=\"{}\"\r\n", esc(&wine_path(g.install_dir, true))));
            out.push_str(&format!("\"Language\"=\"{}\"\r\n", esc(language)));
            out.push_str(&format!("\"UserDataLeafName\"=\"{}\"\r\n", esc(g.game.user_data_leaf())));
            if let Some(k) = g.cd_key.map(str::trim).filter(|k| !k.is_empty()) {
                out.push_str(&format!("\r\n[{key}\\ergc]\r\n@=\"{}\"\r\n", esc(k)));
            }
        }
    }
    out
}

/// Small stable fingerprint so we only re-import when something changed.
pub fn fingerprint(s: &str) -> String {
    // FNV-1a, 64-bit.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wine_paths() {
        assert_eq!(wine_path(Path::new("/home/me/Games/BFME 2"), true), "Z:\\home\\me\\Games\\BFME 2\\");
        assert_eq!(wine_path(Path::new("/a/b.big"), false), "Z:\\a\\b.big");
    }

    #[test]
    fn reg_contents() {
        let dir = Path::new("/g/BFME2");
        let reg = build_reg(&[RegGame { game: GameId::Bfme2, install_dir: dir, cd_key: Some(" ABCD ") }], "english");
        assert!(reg.starts_with("REGEDIT4\r\n"));
        assert!(reg.contains("[HKEY_LOCAL_MACHINE\\SOFTWARE\\Electronic Arts\\EA Games\\The Battle for Middle-earth II]"));
        assert!(reg.contains("[HKEY_LOCAL_MACHINE\\SOFTWARE\\WOW6432Node\\Electronic Arts\\EA Games\\The Battle for Middle-earth II\\ergc]"));
        assert!(reg.contains("\"InstallPath\"=\"Z:\\\\g\\\\BFME2\\\\\""));
        assert!(reg.contains("@=\"ABCD\""));
    }

    #[test]
    fn no_key_no_ergc() {
        let reg = build_reg(&[RegGame { game: GameId::Bfme1, install_dir: Path::new("/x"), cd_key: None }], "english");
        assert!(!reg.contains("ergc"));
    }
}
