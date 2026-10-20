//! Finding installs through what installers leave behind in the Windows
//! registry: the EA keys the games' own installers (and the BFME All-in-One
//! launcher) write, `App Paths` entries, and the `Uninstall` entries EA/Origin,
//! Steam, GOG and the disc installers create.
//!
//! The same lookups work in two places:
//! * on Windows, against the real registry ([`windows_registry`], `cfg(windows)`);
//! * on Linux, against a Wine prefix's `system.reg` / `user.reg` text
//!   ([`parse_wine_reg`]), with `C:\…` paths mapped into the prefix ([`win_to_host`]).
//!
//! Every folder found this way is only a hint: the scanner still checks the
//! game's exe is really there before using it.

use crate::games::{self, GameId};
use std::path::{Path, PathBuf};

/// One registry value: the key path (relative to the hive, e.g.
/// `Software\Wow6432Node\Electronic Arts\EA Games\The Battle for Middle-earth`),
/// the value name (`""` for the default value) and its string data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegValue {
    pub key: String,
    pub name: String,
    pub data: String,
}

/// EA key suffixes (under `SOFTWARE\` or `SOFTWARE\WOW6432Node\`) per game.
/// The first spelling is what the original installers and the All-in-One
/// launcher write; the others are older/legacy spellings and Beacon's own.
pub fn ea_keys(g: GameId) -> &'static [&'static str] {
    match g {
        GameId::Bfme1 => &[r"Electronic Arts\EA Games\The Battle for Middle-earth", r"EA GAMES\The Battle for Middle-earth"],
        GameId::Bfme2 => &[
            r"Electronic Arts\Electronic Arts\The Battle for Middle-earth II",
            r"Electronic Arts\The Battle for Middle-earth II",
            r"Electronic Arts\EA Games\The Battle for Middle-earth II",
        ],
        GameId::Rotwk => &[
            r"Electronic Arts\Electronic Arts\The Lord of the Rings, The Rise of the Witch-king",
            r"Electronic Arts\The Lord of the Rings, The Rise of the Witch-king",
            r"Electronic Arts\EA Games\The Lord of the Rings, The Rise of the Witch-king",
        ],
        _ => &[],
    }
}

/// Value names that hold an install folder (or the exe) in the keys we read.
const DIR_VALUES: &[&str] = &["installpath", "install dir", "installdir", "installlocation", "install_dir", "path", "gamedir"];

fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase()
}

/// Which game an `Uninstall` entry's DisplayName (or a Steam/GOG title) is.
pub fn game_for_title(name: &str) -> Option<GameId> {
    let n = squash(name);
    if n.contains("riseofthewitchking") {
        Some(GameId::Rotwk)
    } else if n.contains("ageofthering") {
        Some(GameId::Aotr)
    } else if n.contains("battleformiddleearthii") || n.contains("battleformiddleearth2") {
        Some(GameId::Bfme2)
    } else if n.contains("battleformiddleearth") {
        Some(GameId::Bfme1)
    } else if n.contains("conquest") && (n.contains("lordoftherings") || n.contains("lotr")) {
        Some(GameId::Conquest)
    } else if n.contains("universeatwar") {
        Some(GameId::Uaw)
    } else if n.contains("armiesofexigo") {
        Some(GameId::Exigo)
    } else if n.contains("riseoflegends") {
        Some(GameId::Rol)
    } else if n.contains("galacticbattlegrounds") {
        Some(GameId::Swgb)
    } else {
        None
    }
}

/// Strip the `SOFTWARE\` / `SOFTWARE\WOW6432Node\` prefix, lowercased, `/` → `\`.
fn key_tail(key: &str) -> String {
    let k = key.replace('/', "\\").trim_matches('\\').to_ascii_lowercase();
    let k = k.strip_prefix("software\\").unwrap_or(&k).to_string();
    k.strip_prefix("wow6432node\\").unwrap_or(&k).to_string()
}

/// Turn a registry value into a folder: `"C:\Games\BFME2\lotrbfme2.exe"` →
/// `C:\Games\BFME2`, quotes and trailing separators removed.
fn as_dir(data: &str) -> Option<String> {
    let d = data.trim().trim_matches('"').trim();
    if d.is_empty() {
        return None;
    }
    let lower = d.to_ascii_lowercase();
    let d = if lower.ends_with(".exe") || lower.ends_with(".dat") {
        let cut = d.rfind(['\\', '/'])?;
        &d[..cut]
    } else {
        d
    };
    let d = d.trim_end_matches(['\\', '/']);
    (!d.is_empty()).then(|| d.to_string())
}

/// Install-folder hints from registry values: `(game if known, Windows path)`.
/// Reads EA keys, `App Paths\<exe>` and `Uninstall\*` entries.
pub fn hints(values: &[RegValue]) -> Vec<(Option<GameId>, String)> {
    let mut out: Vec<(Option<GameId>, String)> = Vec::new();
    let mut push = |g: Option<GameId>, dir: Option<String>| {
        if let Some(dir) = dir {
            if !out.iter().any(|(og, od)| *og == g && od.eq_ignore_ascii_case(&dir)) {
                out.push((g, dir));
            }
        }
    };
    // EA keys.
    for g in GameId::ALL {
        let wanted: Vec<String> = ea_keys(g).iter().map(|k| k.to_ascii_lowercase()).collect();
        for v in values.iter().filter(|v| wanted.contains(&key_tail(&v.key))) {
            if DIR_VALUES.contains(&v.name.to_ascii_lowercase().as_str()) {
                push(Some(g), as_dir(&v.data));
            }
        }
    }
    // App Paths\<exe>: the default value is the exe, "Path" its folder.
    for v in values {
        let tail = key_tail(&v.key);
        if let Some(exe) = tail.strip_prefix(r"microsoft\windows\currentversion\app paths\") {
            if let Some(g) = games::game_for_exe(exe) {
                if v.name.is_empty() || v.name.eq_ignore_ascii_case("path") {
                    push(Some(g), as_dir(&v.data));
                }
            }
        }
    }
    // Uninstall\<id>: DisplayName says which game, InstallLocation where.
    let mut uninstall: Vec<&str> = values
        .iter()
        .filter(|v| key_tail(&v.key).starts_with(r"microsoft\windows\currentversion\uninstall\"))
        .map(|v| v.key.as_str())
        .collect();
    uninstall.sort_unstable();
    uninstall.dedup();
    for key in uninstall {
        let of = |name: &str| values.iter().find(|v| v.key == key && v.name.eq_ignore_ascii_case(name)).map(|v| v.data.clone());
        let Some(g) = of("DisplayName").as_deref().and_then(game_for_title) else { continue };
        push(Some(g), of("InstallLocation").as_deref().and_then(as_dir));
        // Some installers leave InstallLocation empty but point DisplayIcon at the exe.
        if let Some(icon) = of("DisplayIcon") {
            let icon = icon.split(',').next().unwrap_or("").to_string();
            if icon.to_ascii_lowercase().trim_matches('"').ends_with(".exe") {
                push(Some(g), as_dir(&icon));
            }
        }
    }
    out
}

/// The UserDataLeafName the game's own registry names (Options.ini lives in
/// `%APPDATA%\<leaf>`); the All-in-One launcher's leaf names differ from EA's.
pub fn user_data_leaf(values: &[RegValue], g: GameId) -> Option<String> {
    let wanted: Vec<String> = ea_keys(g).iter().map(|k| k.to_ascii_lowercase()).collect();
    values
        .iter()
        .find(|v| wanted.contains(&key_tail(&v.key)) && v.name.eq_ignore_ascii_case("UserDataLeafName") && !v.data.trim().is_empty())
        .map(|v| v.data.trim().to_string())
}

// ---------------------------------------------------------------- Wine prefixes

/// Unescape a Wine .reg string body (`\\`, `\"`, `\n`, `\xHHHH`).
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => {}
            Some('x') => {
                let mut hex = String::new();
                while hex.len() < 4 && it.peek().map(|c| c.is_ascii_hexdigit()).unwrap_or(false) {
                    hex.push(it.next().unwrap());
                }
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The end of a quoted string starting at `s[0] == '"'`: (body, rest).
fn quoted(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some((&s[1..i], &s[i + 1..])),
            _ => i += 1,
        }
    }
    None
}

/// Parse the string values of a Wine registry file (`system.reg`,
/// `user.reg`). Keys are relative to the hive, as Wine writes them
/// (`Software\Wow6432Node\…`). Only keys `keep(key)` accepts are kept.
pub fn parse_wine_reg(text: &str, keep: &dyn Fn(&str) -> bool) -> Vec<RegValue> {
    let mut out = Vec::new();
    let mut key: Option<String> = None;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(rest) = line.strip_prefix('[') {
            // [Software\\Wow6432Node\\Electronic Arts\\EA Games\\The Battle for Middle-earth] 1700000000
            key = rest.rfind(']').map(|end| unescape(&rest[..end])).filter(|k| keep(k));
            continue;
        }
        let Some(k) = &key else { continue };
        let (name, rest) = if let Some(rest) = line.strip_prefix("@=") {
            (String::new(), rest)
        } else if let Some((name, rest)) = quoted(line) {
            match rest.strip_prefix('=') {
                Some(rest) => (unescape(name), rest),
                None => continue,
            }
        } else {
            continue;
        };
        // "x"="…", "x"=str(2):"…" (REG_EXPAND_SZ); dwords and hex blobs are skipped.
        let rest = rest.strip_prefix("str(2):").or_else(|| rest.strip_prefix("str(7):")).unwrap_or(rest);
        if let Some((body, _)) = quoted(rest) {
            out.push(RegValue { key: k.clone(), name, data: unescape(body) });
        }
    }
    out
}

/// Keys worth reading for install hints.
pub fn interesting_key(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.contains("electronic arts") || k.contains("ea games") || k.contains(r"\app paths\") || k.contains(r"\uninstall\")
}

/// A Windows path inside a Wine prefix as a host path: `C:\BFME1` →
/// `<pfx>/drive_c/BFME1`, `Z:\home\me\x` → `/home/me/x`, other letters through
/// `<pfx>/dosdevices/<x>:`.
pub fn win_to_host(pfx: &Path, win: &str) -> Option<PathBuf> {
    let w = win.trim().trim_matches('"');
    let b = w.as_bytes();
    if b.len() < 2 || b[1] != b':' || !b[0].is_ascii_alphabetic() {
        return None;
    }
    let letter = (b[0] as char).to_ascii_lowercase();
    let rest: Vec<&str> = w[2..].split(['\\', '/']).filter(|s| !s.is_empty()).collect();
    if rest.iter().any(|s| *s == "..") {
        return None;
    }
    let mut p = match letter {
        'c' => pfx.join("drive_c"),
        'z' => PathBuf::from("/"),
        l => pfx.join("dosdevices").join(format!("{l}:")),
    };
    for s in rest {
        p.push(s);
    }
    Some(p)
}

/// Wine prefixes at or right below `roots` (a folder with `system.reg`, or
/// `pfx/system.reg` as Proton and Beacon lay them out).
pub fn find_prefixes(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut check = |d: &Path| {
        for cand in [d.to_path_buf(), d.join("pfx")] {
            if cand.join("system.reg").is_file() && !out.contains(&cand) {
                out.push(cand);
            }
        }
    };
    for r in roots {
        check(r.as_path());
        if let Ok(rd) = std::fs::read_dir(r) {
            for e in rd.flatten() {
                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    check(e.path().as_path());
                }
            }
        }
    }
    out
}

/// Registry values of a Wine prefix that matter for finding games.
pub fn prefix_registry(pfx: &Path) -> Vec<RegValue> {
    let mut out = Vec::new();
    for f in ["system.reg", "user.reg"] {
        if let Ok(text) = std::fs::read_to_string(pfx.join(f)) {
            out.extend(parse_wine_reg(&text, &interesting_key));
        }
    }
    out
}

/// Install folders a prefix's registry points at, as host paths.
pub fn prefix_hints(pfx: &Path) -> Vec<(Option<GameId>, PathBuf)> {
    hints(&prefix_registry(pfx)).into_iter().filter_map(|(g, w)| win_to_host(pfx, &w).map(|p| (g, p))).collect()
}

// ---------------------------------------------------------------- Steam libraries

/// Library folders from Steam's `libraryfolders.vdf` (`"path"  "D:\\SteamLibrary"`).
pub fn parse_library_folders(vdf: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in vdf.lines() {
        let t = line.trim();
        let Some((k, rest)) = quoted(t) else { continue };
        if !k.eq_ignore_ascii_case("path") {
            continue;
        }
        if let Some((v, _)) = quoted(rest.trim_start()) {
            let p = PathBuf::from(v.replace("\\\\", "\\"));
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// `<library>/steamapps/common` for every Steam library under `steam_root`.
pub fn steam_common_dirs(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    for vdf in [steam_root.join("steamapps").join("libraryfolders.vdf"), steam_root.join("config").join("libraryfolders.vdf")] {
        if let Ok(text) = std::fs::read_to_string(vdf) {
            for p in parse_library_folders(&text) {
                if !libs.contains(&p) {
                    libs.push(p);
                }
            }
        }
    }
    libs.into_iter().map(|l| l.join("steamapps").join("common")).filter(|d| d.is_dir()).collect()
}

// ---------------------------------------------------------------- Windows

/// The registry values on this Windows PC that point at game installs: the
/// EA keys in both views, `App Paths` for the games' exes, and every
/// `Uninstall` entry (machine and user). Also `HKCU\Software\Valve\Steam`.
#[cfg(windows)]
pub fn windows_registry() -> Vec<RegValue> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    fn read_key(hive: &RegKey, path: &str, out: &mut Vec<RegValue>) {
        let Ok(k) = hive.open_subkey(path) else { return };
        for (name, _) in k.enum_values().flatten() {
            if let Ok(data) = k.get_value::<String, _>(&name) {
                out.push(RegValue { key: path.to_string(), name, data });
            }
        }
    }

    let mut out = Vec::new();
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for view in [r"SOFTWARE\WOW6432Node", "SOFTWARE"] {
        for g in GameId::ALL {
            for k in ea_keys(g) {
                read_key(&hklm, format!(r"{view}\{k}").as_str(), &mut out);
            }
            for exe in g.exes() {
                read_key(&hklm, format!(r"{view}\Microsoft\Windows\CurrentVersion\App Paths\{exe}").as_str(), &mut out);
            }
        }
        let uninstall = format!(r"{view}\Microsoft\Windows\CurrentVersion\Uninstall");
        for hive in [&hklm, &hkcu] {
            if let Ok(u) = hive.open_subkey(&uninstall) {
                for sub in u.enum_keys().flatten() {
                    read_key(hive, format!(r"{uninstall}\{sub}").as_str(), &mut out);
                }
            }
        }
    }
    read_key(&hkcu, r"Software\Valve\Steam", &mut out);
    out
}

/// Not Windows: nothing to read (Wine prefixes go through [`prefix_registry`]).
#[cfg(not(windows))]
pub fn windows_registry() -> Vec<RegValue> {
    Vec::new()
}

/// Where Steam lives according to `HKCU\Software\Valve\Steam` (`SteamPath`).
pub fn steam_root_from(values: &[RegValue]) -> Option<PathBuf> {
    values
        .iter()
        .find(|v| key_tail(&v.key) == r"valve\steam" && v.name.eq_ignore_ascii_case("SteamPath"))
        .map(|v| PathBuf::from(v.data.replace('/', "\\")))
}

/// Folders worth scanning on Windows: the usual install folders of the games'
/// publishers and stores, `%USERPROFILE%\Games`, and the All-in-One launcher's
/// `<drive>:\BFME1`, `BFME2`, `RotWK` folders.
pub fn windows_scan_roots(home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
        if let Some(p) = env(var).filter(|p| !p.is_empty()) {
            let p = PathBuf::from(p);
            if !bases.contains(&p) {
                bases.push(p);
            }
        }
    }
    let mut roots = vec![home.join("Games")];
    for b in &bases {
        for sub in [
            "EA Games",
            "Electronic Arts",
            "Origin Games",
            "EA Games\\Origin Games",
            "Steam\\steamapps\\common",
            "GOG Galaxy\\Games",
            "GOG Games",
            "LucasArts",
            "SEGA",
            "Petroglyph",
            "Microsoft Games",
            "Big Huge Games",
            "Age of the Ring",
        ] {
            roots.push(b.join(sub));
        }
    }
    for letter in 'C'..='Z' {
        for sub in ["Games", "GOG Games", "BFME1", "BFME2", "RotWK"] {
            roots.push(PathBuf::from(format!("{letter}:\\{sub}")));
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEM_REG: &str = r#"WINE REGISTRY Version 2
;; All keys relative to \\Machine

#arch=win64

[Software\\Wow6432Node\\Electronic Arts\\EA Games\\The Battle for Middle-earth] 1729000000
#time=1db1234
"InstallPath"="C:\\BFME1\\"
"Language"="english"
"MapPackVersion"=dword:00010000
"UserDataLeafName"="My Battle for Middle-earth Files"

[Software\\Wow6432Node\\Electronic Arts\\EA Games\\The Battle for Middle-earth\\ergc] 1729000000
@="ABCDEFGHIJ0123456789"

[Software\\Wow6432Node\\Electronic Arts\\Electronic Arts\\The Battle for Middle-earth II] 1729000000
"InstallPath"="C:\\BFME2\\"
"UserDataLeafName"="My Battle for Middle-earth II Files"

[Software\\Wow6432Node\\Electronic Arts\\Electronic Arts\\The Lord of the Rings, The Rise of the Witch-king] 1729000000
"InstallPath"="C:\\RotWK\\"

[Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths\\lotrbfme.exe] 1729000000
@="C:\\BFME1\\lotrbfme.exe"
"Path"="C:\\BFME1\\"

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{ABCD-1234}] 1729000000
"DisplayName"="The Lord of the Rings\x2122: Conquest"
"InstallLocation"=str(2):"D:\\Games\\LOTR Conquest"

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Steam App 356500] 1729000000
"DisplayName"="STAR WARS\x2122 Galactic Battlegrounds Saga"
"DisplayIcon"="Z:\\home\\me\\.steam\\steam\\steamapps\\common\\Star Wars Galactic Battlegrounds Saga\\Game\\battlegrounds_x1.exe,0"

[Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\SomethingElse] 1729000000
"DisplayName"="Notepad++"
"InstallLocation"="C:\\Program Files\\Notepad++"

[Software\\Wine\\Fonts] 1729000000
"InstallPath"="C:\\nothere"
"#;

    fn parsed() -> Vec<RegValue> {
        parse_wine_reg(SYSTEM_REG, &interesting_key)
    }

    #[test]
    fn parses_wine_registry_strings() {
        let v = parsed();
        let ip = v.iter().find(|x| x.name == "InstallPath" && x.key.ends_with("The Battle for Middle-earth")).unwrap();
        assert_eq!(ip.key, r"Software\Wow6432Node\Electronic Arts\EA Games\The Battle for Middle-earth");
        assert_eq!(ip.data, r"C:\BFME1\");
        assert!(v.iter().any(|x| x.name.is_empty() && x.data == "ABCDEFGHIJ0123456789"), "default values");
        assert!(!v.iter().any(|x| x.name == "MapPackVersion"), "dwords are skipped");
        assert!(!v.iter().any(|x| x.key.contains("Fonts")), "only interesting keys");
        let conquest = v.iter().find(|x| x.name == "DisplayName" && x.data.contains("Conquest")).unwrap();
        assert_eq!(conquest.data, "The Lord of the Rings\u{2122}: Conquest");
        assert!(v.iter().any(|x| x.name == "InstallLocation" && x.data == r"D:\Games\LOTR Conquest"), "str(2): values");
    }

    #[test]
    fn hints_from_ea_keys_app_paths_and_uninstall() {
        let h = hints(&parsed());
        assert!(h.contains(&(Some(GameId::Bfme1), r"C:\BFME1".to_string())));
        assert!(h.contains(&(Some(GameId::Bfme2), r"C:\BFME2".to_string())));
        assert!(h.contains(&(Some(GameId::Rotwk), r"C:\RotWK".to_string())));
        assert!(h.contains(&(Some(GameId::Conquest), r"D:\Games\LOTR Conquest".to_string())));
        assert!(h.iter().any(|(g, d)| *g == Some(GameId::Swgb) && d.ends_with(r"Saga\Game")), "{h:?}");
        assert!(!h.iter().any(|(_, d)| d.contains("Notepad")));
        assert_eq!(h.iter().filter(|(g, _)| *g == Some(GameId::Bfme1)).count(), 1, "EA key and App Paths agree: {h:?}");
        assert_eq!(user_data_leaf(&parsed(), GameId::Bfme2).as_deref(), Some("My Battle for Middle-earth II Files"));
        assert_eq!(user_data_leaf(&parsed(), GameId::Rotwk), None);
    }

    #[test]
    fn titles() {
        assert_eq!(game_for_title("The Lord of the Rings, The Battle for Middle-earth (tm)"), Some(GameId::Bfme1));
        assert_eq!(game_for_title("The Battle for Middle-earth\u{2122} II"), Some(GameId::Bfme2));
        assert_eq!(game_for_title("The Lord of the Rings, The Rise of the Witch-king"), Some(GameId::Rotwk));
        assert_eq!(game_for_title("Rise of Nations: Rise of Legends"), Some(GameId::Rol));
        assert_eq!(game_for_title("Universe at War: Earth Assault"), Some(GameId::Uaw));
        assert_eq!(game_for_title("Conquest: Frontier Wars"), None);
        assert_eq!(game_for_title("Steam"), None);
    }

    #[test]
    fn windows_paths_in_a_prefix() {
        let pfx = Path::new("/p/pfx");
        assert_eq!(win_to_host(pfx, r"C:\BFME1").unwrap(), Path::new("/p/pfx/drive_c/BFME1"));
        assert_eq!(win_to_host(pfx, r"Z:\home\me\Games\x\").unwrap(), Path::new("/home/me/Games/x"));
        assert_eq!(win_to_host(pfx, r"d:\Games").unwrap(), Path::new("/p/pfx/dosdevices/d:/Games"));
        assert!(win_to_host(pfx, r"C:\..\..\etc").is_none());
        assert!(win_to_host(pfx, "relative\\x").is_none());
    }

    #[test]
    fn prefixes_and_their_hints() {
        let t = tempfile::tempdir().unwrap();
        let pfx = t.path().join("prefixes/bfme1/pfx");
        std::fs::create_dir_all(pfx.join("drive_c/BFME1")).unwrap();
        std::fs::write(pfx.join("system.reg"), SYSTEM_REG).unwrap();
        let wine = t.path().join(".wine");
        std::fs::create_dir_all(&wine).unwrap();
        std::fs::write(wine.join("system.reg"), "WINE REGISTRY Version 2\n").unwrap();
        let found = find_prefixes(&[t.path().join("prefixes"), wine.clone(), t.path().join("missing")]);
        assert_eq!(found, vec![pfx.clone(), wine]);
        let h = prefix_hints(&pfx);
        assert!(h.contains(&(Some(GameId::Bfme1), pfx.join("drive_c/BFME1"))), "{h:?}");
    }

    #[test]
    fn steam_library_folders() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps" { "356500" "123" }
	}
}"#;
        assert_eq!(parse_library_folders(vdf), vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"D:\SteamLibrary")]);
        let v = vec![RegValue { key: r"Software\Valve\Steam".into(), name: "SteamPath".into(), data: "c:/program files (x86)/steam".into() }];
        assert_eq!(steam_root_from(&v).unwrap(), PathBuf::from(r"c:\program files (x86)\steam"));
    }

    #[test]
    fn windows_roots_include_aio_folders() {
        let env = |k: &str| match k {
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".to_string()),
            "ProgramFiles" => Some(r"C:\Program Files".to_string()),
            _ => None,
        };
        let roots = windows_scan_roots(Path::new(r"C:\Users\me"), &env);
        let s: Vec<String> = roots.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        assert!(s.iter().any(|r| r == r"C:\BFME2"), "{s:?}");
        assert!(s.iter().any(|r| r.ends_with(r"Program Files (x86)\EA Games") || r.ends_with(r"Program Files (x86)/EA Games")));
    }
}
