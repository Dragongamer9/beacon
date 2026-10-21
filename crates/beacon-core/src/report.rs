//! Machine-readable output for `beacon … --json`, for Hearth and scripts.
//!
//! The shapes are documented in the README ("For Hearth / scripting") and are
//! kept stable: fields may be added, never renamed or removed.
//!
//! * Reading commands print one JSON document (`status`: the `GameStatus`
//!   list, as before; the others: an object).
//! * Changing commands print one line per progress update while they work,
//!   `{"event":"progress","pct":40,"text":"…"}` (`pct` may be null), then one
//!   result line: `{"ok":true,"game":…,"message":…, …}`, or
//!   `{"ok":false,"kind":…,"error":…}`.
//! * Exit codes: [`EXIT_OK`], [`EXIT_FAILED`], [`EXIT_USAGE`], [`EXIT_PROGRAM_FAILED`].

use crate::games::{FoundGame, GameId};
use crate::runner::Command;
use crate::Beacon;
use serde_json::{json, Map, Value};
use std::path::Path;

/// Done.
pub const EXIT_OK: i32 = 0;
/// It didn't work (the message says why).
pub const EXIT_FAILED: i32 = 1;
/// Bad arguments: an unknown game, mod, runner or step, or a malformed value.
pub const EXIT_USAGE: i32 = 2;
/// The Windows program Beacon ran (installer, patch) closed with an error.
pub const EXIT_PROGRAM_FAILED: i32 = 3;

/// Lines read from the end of a log by `beacon log`, at most.
pub const LOG_LINES_MAX: usize = 2000;

/// Any serializable value; `null` if it can't be (a path that isn't UTF-8).
pub fn value<T: serde::Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// A path as a string (lossy for paths that aren't UTF-8).
pub fn path(p: &Path) -> Value {
    Value::String(p.to_string_lossy().into_owned())
}

/// `status`: every game's `GameStatus`.
pub fn status(b: &Beacon) -> Value {
    value(&b.statuses())
}

/// `mods <game>`: `{game, selected, mods_dir, mods: [ModEntry]}`.
pub fn mods(b: &Beacon, g: GameId) -> Value {
    let selected = b.config.selected_mod.get(&g).cloned().filter(|id| b.find_mod(g, id).is_some());
    json!({
        "game": g.slug(),
        "selected": selected,
        "mods_dir": path(&b.paths.mods_dir().join(g.slug())),
        "mods": value(&b.mods(g)),
    })
}

/// `catalog <game>`: `{game, items: [{id, name, game, kind, blurb, url, installed}]}`.
pub fn catalog(b: &Beacon, g: GameId) -> Value {
    json!({"game": g.slug(), "items": value(&b.catalog(g))})
}

/// `runners`: `{runners: [Runner], best, default, umu}`.
pub fn runners(b: &Beacon) -> Value {
    json!({
        "runners": value(&b.runners.runners),
        "best": b.runners.best().map(|r| r.id.clone()),
        "default": b.config.default_runner.clone().filter(|id| b.runners.get(id).is_some()),
        "umu": b.runners.umu.as_deref().map(path),
    })
}

/// `community <game>`: `{game, chosen, editions: [...], steps: [...]}`.
pub fn community(b: &Beacon, g: GameId) -> Value {
    let c = b.community(g);
    json!({
        "game": g.slug(),
        "chosen": b.game(g).edition,
        "editions": value(&c.editions),
        "steps": value(&c.steps),
    })
}

/// `paths`: Beacon's folders, and each game's prefix, log and mods folder.
pub fn paths(b: &Beacon) -> Value {
    let games: Map<String, Value> = GameId::ALL
        .into_iter()
        .map(|g| {
            let v = json!({
                "prefix": path(&b.run_root(g)),
                "log": path(&b.paths.log_file(g)),
                "mods": path(&b.paths.mods_dir().join(g.slug())),
            });
            (g.slug().to_string(), v)
        })
        .collect();
    json!({
        "config": path(&b.paths.config_file),
        "data": path(&b.paths.data_dir),
        "prefixes": path(&b.paths.data_dir.join("prefixes")),
        "mods": path(&b.paths.mods_dir()),
        "logs": path(&b.paths.data_dir.join("logs")),
        "backups": path(&b.paths.data_dir.join("backups")),
        "games": games,
    })
}

/// `workshop <game>`: `{game, page, source, items: [workshop::Item], hidden}`. `hidden` counts the
/// entries left out (base games, EA's official patches, snapshots, other games).
pub fn workshop(g: GameId, page: u32, items: &[crate::workshop::Item], hidden: usize) -> Value {
    json!({
        "game": g.slug(),
        "page": page,
        "source": crate::workshop::SITE,
        "items": value(&items),
        "hidden": hidden,
    })
}

/// `aio`: `{launcher_url, games: [{game, dir, active_patch, active_mod}], appdata: [...], launcher}`
/// (`launcher` since 0.0.7: as [`launcher`]).
pub fn aio(b: &Beacon) -> Value {
    json!({
        "launcher_url": crate::aio::LAUNCHER_URL,
        "games": value(&b.aio_games()),
        "appdata": b.aio_appdatas().iter().map(|p| path(p)).collect::<Vec<_>>(),
        "launcher": launcher(b),
    })
}

/// `aio-launcher status`: `{installed, path, prefix, version, by_beacon, can_install, runner,
/// download_url, site, log}` (since 0.0.7).
pub fn launcher(b: &Beacon) -> Value {
    value(&b.launcher_status())
}

/// `community-launcher <game> status`: `{game, url, path, missing, can_open, prefix, runner, tools_dir, log,
/// game_installed}` (since 0.0.7); `None` for games without a community launcher.
pub fn community_launcher(b: &Beacon, g: GameId) -> Option<Value> {
    b.community_launcher_status(g).map(|s| value(&s))
}

/// The end of a text file: at most `max_bytes`, starting at a whole line.
fn tail(file: &Path, max_bytes: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(file).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    // One byte early, to see whether `start` begins a line.
    f.seek(SeekFrom::Start(start.saturating_sub(1))).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let mut s = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        match s.find('\n') {
            Some(i) => s = s[i + 1..].to_string(),
            None => s.clear(),
        }
    }
    Some(s)
}

/// `log <game>`: `{game, path, exists, lines}`, the last `lines` lines.
pub fn log(b: &Beacon, g: GameId, lines: usize) -> Value {
    let file = b.paths.log_file(g);
    let text = tail(&file, 512 * 1024);
    let lines = lines.clamp(1, LOG_LINES_MAX);
    let out: Vec<String> = text
        .as_deref()
        .map(|t| {
            let all: Vec<&str> = t.lines().collect();
            all[all.len().saturating_sub(lines)..].iter().map(|l| l.to_string()).collect()
        })
        .unwrap_or_default();
    json!({"game": g.slug(), "path": path(&file), "exists": text.is_some(), "lines": out})
}

/// Games a scan found: `[{game, dir}]`.
pub fn found(list: &[FoundGame]) -> Value {
    value(&list)
}

/// A command Beacon would run: `{program, args, env: [[k, v]], cwd, display}`.
pub fn command(c: &Command) -> Value {
    json!({
        "program": path(&c.program),
        "args": c.args,
        "env": c.env.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "cwd": path(&c.cwd),
        "display": c.display(),
    })
}

/// One progress line for a long job. `pct` is 0–100, or `None` when unknown.
pub fn progress(pct: Option<u8>, text: &str) -> String {
    json!({"event": "progress", "pct": pct.map(|p| p.min(100)), "text": text}).to_string()
}

/// Percent done after `i` of `n` steps, within `from..=to`.
pub fn pct(i: usize, n: usize, from: u8, to: u8) -> u8 {
    let span = to.saturating_sub(from) as usize;
    (from as usize + span * i / n.max(1)).min(100) as u8
}

fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(o), Value::Object(e)) = (base.as_object_mut(), extra) {
        for (k, x) in e {
            o.insert(k, x);
        }
    }
    base
}

/// A changing command worked: `{ok: true, game, message, ...extra}`.
pub fn ok(game: Option<GameId>, message: impl Into<String>, extra: Value) -> Value {
    merge(json!({"ok": true, "game": game.map(|g| g.slug()), "message": message.into()}), extra)
}

/// The Windows program Beacon ran closed with an error:
/// `{ok: false, kind: "program", game, error, ...extra}` (exit code 3).
pub fn program_failed(game: GameId, message: impl Into<String>, extra: Value) -> Value {
    merge(json!({"ok": false, "kind": "program", "game": game.slug(), "error": message.into()}), extra)
}

/// A command failed: `{ok: false, kind, error}`. `kind` is `usage` (exit 2) or
/// `failed` (exit 1).
pub fn error(kind: &str, message: &str) -> Value {
    json!({"ok": false, "kind": kind, "error": message})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use std::fs;

    fn beacon() -> (tempfile::TempDir, Beacon) {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let rotwk = home.join("Games/RotWK");
        let bfme2 = home.join("Games/BFME2");
        fs::create_dir_all(&rotwk).unwrap();
        fs::create_dir_all(&bfme2).unwrap();
        fs::write(rotwk.join("lotrbfme2ep1.exe"), b"").unwrap();
        fs::write(bfme2.join("lotrbfme2.exe"), b"").unwrap();
        let bin = t.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("wine"), b"#!/bin/sh\n").unwrap();
        let mut b = Beacon::load_with(Paths::under(&t.path().join("beacon"), &home), &bin.to_string_lossy()).unwrap();
        b.scan(&[]);
        (t, b)
    }

    fn keys(v: &Value) -> Vec<&str> {
        let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        k.sort();
        k
    }

    #[test]
    fn mods_shape() {
        let (_t, mut b) = beacon();
        let dir = b.paths.mods_dir().join("rotwk/Edain");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("Edain Launcher.exe"), b"").unwrap();
        b.select_mod(GameId::Rotwk, Some("rotwk:edain")).unwrap();
        let v = mods(&b, GameId::Rotwk);
        assert_eq!(keys(&v), ["game", "mods", "mods_dir", "selected"]);
        assert_eq!(v["game"], "rotwk");
        assert_eq!(v["selected"], "rotwk:edain");
        let m = &v["mods"][0];
        assert_eq!(keys(m), ["args", "game", "id", "in_library", "kind", "name", "path"]);
        assert_eq!((m["id"].as_str(), m["kind"].as_str(), m["in_library"].as_bool()), (Some("rotwk:edain"), Some("launcher"), Some(true)));
        assert!(m["path"].as_str().unwrap().ends_with("Edain Launcher.exe"));
        assert_eq!(mods(&b, GameId::Bfme1)["mods"], json!([]));
        assert!(mods(&b, GameId::Bfme1)["selected"].is_null());
    }

    #[test]
    fn catalog_shape() {
        let (_t, b) = beacon();
        let v = catalog(&b, GameId::Rotwk);
        assert_eq!(keys(&v), ["game", "items"]);
        let edain = v["items"].as_array().unwrap().iter().find(|i| i["id"] == "edain").unwrap();
        assert_eq!(keys(edain), ["blurb", "game", "id", "installed", "kind", "name", "url"]);
        assert_eq!(edain["kind"], "mod");
        assert!(edain["installed"].is_null());
        assert!(edain["url"].as_str().unwrap().starts_with("https://"));
    }

    // Wine runner ids, Wine command lines and `/` in path strings.
    #[cfg(unix)]
    #[test]
    fn runners_shape() {
        let (_t, mut b) = beacon();
        b.config.default_runner = Some("wine:system".into());
        let v = runners(&b);
        assert_eq!(keys(&v), ["best", "default", "runners", "umu"]);
        assert_eq!(v["best"], "wine:system");
        assert_eq!(v["default"], "wine:system");
        assert!(v["umu"].is_null());
        assert_eq!(keys(&v["runners"][0]), ["id", "kind", "name", "path"]);
        assert_eq!(v["runners"][0]["kind"], "wine");
        b.config.default_runner = Some("gone".into());
        assert!(runners(&b)["default"].is_null(), "a runner that's gone isn't reported");
    }

    #[test]
    fn community_shape() {
        let (_t, b) = beacon();
        let v = community(&b, GameId::Uaw);
        assert_eq!(keys(&v), ["chosen", "editions", "game", "steps"]);
        assert!(v["chosen"].is_null());
        assert_eq!(keys(&v["editions"][0]), ["active", "blurb", "community", "id", "installed", "name"]);
        assert_eq!(keys(&v["steps"][0]), ["action", "detail", "done", "id", "title", "url"]);
        assert_eq!(v["steps"][0]["action"], "installer");
        let none = community(&b, GameId::Bfme1);
        assert_eq!((none["editions"].clone(), none["steps"].clone()), (json!([]), json!([])));
    }

    // Wine runner ids, Wine command lines and `/` in path strings.
    #[cfg(unix)]
    #[test]
    fn status_paths_and_log_shapes() {
        let (_t, b) = beacon();
        let s = status(&b);
        assert_eq!(s.as_array().unwrap().len(), GameId::ALL.len());
        assert_eq!(s[2]["id"], "rotwk");
        assert_eq!(s[2]["requires"], "bfme2");

        let p = paths(&b);
        assert_eq!(keys(&p), ["backups", "config", "data", "games", "logs", "mods", "prefixes"]);
        assert_eq!(keys(&p["games"]["aotr"]), ["log", "mods", "prefix"]);
        assert!(p["games"]["aotr"]["prefix"].as_str().unwrap().ends_with("prefixes/aotr"));

        let l = log(&b, GameId::Rotwk, 2);
        assert_eq!(keys(&l), ["exists", "game", "lines", "path"]);
        assert_eq!((l["exists"].as_bool(), l["lines"].clone()), (Some(false), json!([])));
        let f = b.paths.log_file(GameId::Rotwk);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "one\ntwo\nthree\n").unwrap();
        assert_eq!(log(&b, GameId::Rotwk, 2)["lines"], json!(["two", "three"]));
        assert_eq!(tail(&f, 6).unwrap(), "three\n", "starts at a whole line");
    }

    // Wine runner ids, Wine command lines and `/` in path strings.
    #[cfg(unix)]
    #[test]
    fn command_and_found_shapes() {
        let (_t, b) = beacon();
        let c = b.launch_command(GameId::Bfme2, None).unwrap();
        let v = command(&c);
        assert_eq!(keys(&v), ["args", "cwd", "display", "env", "program"]);
        assert!(v["env"].as_array().unwrap().iter().any(|e| e[0] == "WINEPREFIX"));
        let f = found(&[FoundGame { game: GameId::Bfme2, dir: "/g/BFME2".into() }]);
        assert_eq!(f, json!([{"game": "bfme2", "dir": "/g/BFME2"}]));
    }

    #[test]
    fn workshop_and_aio_shapes() {
        let (_t, b) = beacon();
        let item = crate::workshop::parse_item(&json!({"Guid": "g-1", "Name": "Edain", "Game": 2, "Type": 1})).unwrap();
        let v = workshop(GameId::Rotwk, 0, &[item], 3);
        assert_eq!(keys(&v), ["game", "hidden", "items", "page", "source"]);
        assert_eq!((v["items"][0]["kind"].as_str(), v["hidden"].as_u64()), (Some("mod"), Some(3)));
        let a = aio(&b);
        assert_eq!(keys(&a), ["appdata", "games", "launcher", "launcher_url"]);
        assert_eq!(a["launcher_url"], "https://bfmeladder.com");
        assert_eq!(a["games"], json!([]));
        assert_eq!(
            keys(&a["launcher"]),
            ["by_beacon", "can_install", "download_url", "installed", "log", "path", "prefix", "runner", "site", "version"]
        );
        assert_eq!((a["launcher"]["installed"].as_bool(), a["launcher"]["path"].is_null()), (Some(false), true));
        assert!(a["launcher"]["download_url"].as_str().unwrap().starts_with("https://"));
        assert_eq!(launcher(&b), a["launcher"]);
        // store_url is in every GameStatus.
        let s = status(&b);
        assert_eq!(s[8]["id"], "swgb");
        assert_eq!(s[8]["store_url"], "https://store.steampowered.com/app/356500/");
        assert!(s[0]["store_url"].is_null());
    }

    #[test]
    fn community_launcher_shapes() {
        let (_t, b) = beacon();
        let v = community_launcher(&b, GameId::Uaw).unwrap();
        assert_eq!(keys(&v), ["can_open", "game", "game_installed", "log", "missing", "path", "prefix", "runner", "tools_dir", "url"]);
        assert_eq!(v["game"], "uaw");
        assert!(v["url"].as_str().unwrap().starts_with("https://drive.google.com/"));
        assert!(v["path"].is_null() && v["can_open"] == false && v["missing"] == false);
        assert!(community_launcher(&b, GameId::Rotwk).is_none());
        // In `status`: `{path, url}` for UaW only; left out for the others.
        let s = status(&b);
        assert_eq!(s[5]["id"], "uaw");
        assert_eq!(keys(&s[5]["community_launcher"]), ["path", "url"]);
        assert_eq!(s[5]["community_launcher"]["url"], v["url"]);
        assert!(s[5]["community_launcher"]["path"].is_null());
        assert!(s[2].get("community_launcher").is_none());
    }

    #[test]
    fn result_lines() {
        let p: Value = serde_json::from_str(&progress(Some(140), "Looking in ~/Games")).unwrap();
        assert_eq!(p, json!({"event": "progress", "pct": 100, "text": "Looking in ~/Games"}));
        let p: Value = serde_json::from_str(&progress(None, "x")).unwrap();
        assert!(p["pct"].is_null());
        assert!(!progress(None, "a\nb").contains('\n'), "one line per update");
        assert_eq!(pct(0, 4, 0, 100), 0);
        assert_eq!(pct(2, 4, 10, 90), 50);
        assert_eq!(pct(3, 0, 0, 100), 100);

        assert_eq!(ok(Some(GameId::Rotwk), "Done", json!({"x": 1})), json!({"ok": true, "game": "rotwk", "message": "Done", "x": 1}));
        assert_eq!(ok(None, "Done", Value::Null), json!({"ok": true, "game": null, "message": "Done"}));
        assert_eq!(program_failed(GameId::Aotr, "Closed with an error (1)", json!({"exit_code": 1}))["kind"], "program");
        assert_eq!(error("usage", "unknown game"), json!({"ok": false, "kind": "usage", "error": "unknown game"}));
    }
}
