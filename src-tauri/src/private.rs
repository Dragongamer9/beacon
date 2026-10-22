//! Private build: downloads in the app. BFME, BFME II and RotWK from the BFME
//! Foundation's CDN (`cdn_get`), and the download sources (check, update,
//! token, games folder). Progress arrives as `cdn-progress` events; the
//! start-up check of the sources as `sources-checked`.

use crate::{err, snapshot, AppState, CmdResult};
use beacon_core::downloads::{archive, basegame, launcher, selfupdate, sources, Client};
use beacon_core::games::GameId;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone, Serialize)]
struct CdnProgress {
    game: GameId,
    pct: Option<u8>,
    text: String,
}

/// The sources' state for the Launcher settings sheet.
#[tauri::command]
pub fn downloads_info(state: State<AppState>) -> Value {
    let b = state.beacon.lock().unwrap();
    let store = b.download_store();
    let list = store.list();
    let s = store.settings();
    let games: Vec<String> = ["bfme1", "bfme2", "rotwk"]
        .into_iter()
        .map(String::from)
        .chain(list.sources.iter().filter_map(|s| s.archive_for).map(|g| g.slug().to_string()))
        .collect();
    let checks: Option<Vec<sources::Check>> = std::fs::read_to_string(store.status_file()).ok().and_then(|t| serde_json::from_str(&t).ok());
    json!({
        "version": list.version,
        "built_in": list == sources::SourceList::bundled(),
        "sources_repo": s.sources_repo,
        "games_dir": b.games_root(),
        "custom_games_dir": s.games_dir.is_some(),
        "learned_api": store.learned().workshop_api,
        "checks": checks,
        "games": games,
        "archive_launchers": b.archive_launchers().exes,
        "launchers": list.sources.iter().filter_map(|s| s.launcher_for).collect::<Vec<GameId>>(),
    })
}

/// Download a game from the CDN (and BFME II for RotWK when it's missing), point Beacon at it and
/// write its registry keys. Answers `{result: "done", snapshot, message}` or
/// `{result: "needs-confirm", game, note}` when the source only turned up a guess.
#[tauri::command]
pub async fn cdn_get(app: AppHandle, game: GameId, repair: bool, verify: bool, accept_guess: bool) -> CmdResult<Value> {
    if !basegame::supported(game) {
        return archive_get(app, game, accept_guess).await;
    }
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let opts = basegame::Options { thorough: verify, accept_guess, reinstall: repair || verify };
    let out = tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now() - Duration::from_secs(1);
        let mut last_pct = None;
        b.cdn_download(&Client::new(), game, opts, &mut |pct: Option<u8>, text: &str| {
            // At most ~8 events a second, plus every change of percentage.
            if pct != last_pct || last.elapsed() >= Duration::from_millis(120) {
                last = std::time::Instant::now();
                last_pct = pct;
                let _ = app2.emit("cdn-progress", CdnProgress { game, pct, text: text.to_string() });
            }
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let games = match out {
        basegame::Outcome::NeedsConfirm { game, note } => return Ok(json!({"result": "needs-confirm", "game": game, "note": note})),
        basegame::Outcome::Done { games } => games,
    };
    if games.is_empty() {
        let state = app.state::<AppState>();
        return Ok(json!({"result": "done", "snapshot": snapshot(&state), "message": format!("{} is already installed", game.short())}));
    }
    // Adopt under the lock (quick), then write the registry on a copy (Wine or the UAC prompt can take a while).
    let b = {
        let state = app.state::<AppState>();
        let mut b = state.beacon.lock().unwrap();
        b.cdn_adopt(&games).map_err(|e| format!("{e:#}"))?;
        b.save().map_err(err)?;
        b.clone()
    };
    let _ = app.emit("cdn-progress", CdnProgress { game, pct: Some(100), text: "Writing the registry keys".into() });
    let games2 = games.clone();
    let reg = tauri::async_runtime::spawn_blocking(move || b.cdn_register(&games2)).await.map_err(err)?;
    let state = app.state::<AppState>();
    let notes: Vec<String> = games.iter().filter_map(|g| g.note.clone()).collect();
    let names: Vec<&str> = games.iter().map(|g| g.game.short()).collect();
    let mut message = format!("{} downloaded and ready", names.join(" and "));
    if let Err(e) = reg {
        message = format!("{} downloaded, but the registry keys weren't written: {e:#}", names.join(" and "));
    }
    Ok(json!({"result": "done", "snapshot": snapshot(&state), "message": message, "notes": notes}))
}

/// Look every source up now; also saved for `downloads_info`.
#[tauri::command]
pub async fn sources_check(app: AppHandle) -> CmdResult<Vec<sources::Check>> {
    let store = app.state::<AppState>().beacon.lock().unwrap().download_store();
    tauri::async_runtime::spawn_blocking(move || run_check(&store)).await.map_err(err)
}

fn run_check(store: &sources::Store) -> Vec<sources::Check> {
    let list = store.list();
    let mut learned = store.learned();
    let checks = sources::check_all(&Client::new(), &list, &mut learned);
    let _ = store.save_learned(&learned);
    let _ = std::fs::create_dir_all(&store.dir);
    if let Ok(t) = serde_json::to_string_pretty(&checks) {
        let _ = std::fs::write(store.status_file(), t);
    }
    checks
}

/// Fetch a newer `sources.json` from the private repo now.
#[tauri::command]
pub async fn sources_update(app: AppHandle) -> CmdResult<sources::Update> {
    let store = app.state::<AppState>().beacon.lock().unwrap().download_store();
    tauri::async_runtime::spawn_blocking(move || store.update(&Client::new(), true)).await.map_err(err)?.map_err(|e| format!("{e:#}"))
}

/// The public GitHub repo (owner/repo) Beacon updates itself and `sources.json` from.
#[tauri::command]
pub fn set_updates_repo(state: State<AppState>, repo: String) -> CmdResult<String> {
    let r = sources::check_repo(&repo).map_err(|e| format!("{e:#}"))?;
    let store = state.beacon.lock().unwrap().download_store();
    let mut s = store.settings();
    s.sources_repo = r.clone();
    store.save_settings(&s).map_err(err)?;
    Ok(r)
}

#[tauri::command]
pub fn set_games_dir(state: State<AppState>, path: Option<String>) -> CmdResult<()> {
    let store = state.beacon.lock().unwrap().download_store();
    let mut s = store.settings();
    s.games_dir = path.filter(|p| !p.trim().is_empty()).map(PathBuf::from);
    if s.games_dir.as_ref().is_some_and(|p| !p.is_absolute()) {
        return Err("use an absolute folder".into());
    }
    store.save_settings(&s).map_err(err)
}

/// At start-up, in the background: look for a newer list (at most every few hours), then check the
/// sources and tell the UI (`sources-checked`) so a broken link shows up before you need it.
pub fn start_background_check(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        // Give the window a moment first.
        std::thread::sleep(Duration::from_secs(4));
        let store = app.state::<AppState>().beacon.lock().unwrap().download_store();
        let updated = store.update(&Client::new(), false).ok().filter(|u| u.to > u.from);
        let checks = run_check(&store);
        let _ = app.emit("sources-checked", json!({"checks": checks, "updated": updated}));
        // A newer private release of Beacon itself?
        let b = app.state::<AppState>().beacon.lock().unwrap().clone();
        let c = b.update_check(&Client::new());
        if c.newer {
            let _ = app.emit("app-update", &c);
        }
    });
}

#[derive(Clone, Serialize)]
struct CommunityLauncherProgress {
    game: GameId,
    pct: Option<u8>,
    text: String,
}

/// Download the newest community launcher (UaW's, from its Google Drive folder) and set it up like
/// one picked by hand. Progress arrives as `community-launcher-progress`. Answers
/// `{result: "done" | "up-to-date", snapshot, message}` or `{result: "needs-confirm", note}`.
#[tauri::command]
pub async fn launcher_get(app: AppHandle, game: GameId, force: bool, accept_guess: bool) -> CmdResult<Value> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        let mut last = std::time::Instant::now() - Duration::from_secs(1);
        let mut last_pct = None;
        b.launcher_download(&Client::new(), game, accept_guess, force, &mut |pct: Option<u8>, text: &str| {
            if pct != last_pct || last.elapsed() >= Duration::from_millis(120) {
                last = std::time::Instant::now();
                last_pct = pct;
                let _ = app2.emit("community-launcher-progress", CommunityLauncherProgress { game, pct, text: text.to_string() });
            }
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let f = match out {
        launcher::Outcome::NeedsConfirm { note, .. } => return Ok(json!({"result": "needs-confirm", "game": game, "note": note})),
        launcher::Outcome::Ready(f) => f,
    };
    if f.up_to_date {
        let state = app.state::<AppState>();
        return Ok(json!({"result": "up-to-date", "snapshot": snapshot(&state), "message": format!("The {} launcher is already the newest ({})", game.short(), f.name)}));
    }
    let _ = app.emit("community-launcher-progress", CommunityLauncherProgress { game, pct: Some(100), text: "Setting up the launcher".into() });
    let mut work = app.state::<AppState>().beacon.lock().unwrap().clone();
    let file = f.file.clone();
    let exe = tauri::async_runtime::spawn_blocking(move || work.set_community_launcher(game, &file, &mut |_: &str| {}))
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))?;
    let state = app.state::<AppState>();
    {
        let mut b = state.beacon.lock().unwrap();
        b.game_mut(game).community_launcher = Some(exe);
        b.save().map_err(err)?;
    }
    let mut message = format!("{} ready — open it to download the game", f.name);
    if let Some(n) = &f.note {
        message.push_str(&format!(" ({n})"));
    }
    Ok(json!({"result": "done", "snapshot": snapshot(&state), "message": message}))
}

/// A game delivered as one archive (Rise of Legends): download, extract, then either point Beacon at
/// the game or hand back the launcher/installer it held (`launcher`), which the UI runs in the prefix.
async fn archive_get(app: AppHandle, game: GameId, accept_guess: bool) -> CmdResult<Value> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    if !b.has_archive_source(game) {
        return Err(format!("Beacon has no download for {}", game.short()));
    }
    let app2 = app.clone();
    let out = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Result<(archive::Fetched, archive::Unpacked), String>> {
        let mut last = std::time::Instant::now() - Duration::from_secs(1);
        let mut last_pct = None;
        let mut progress = |pct: Option<u8>, text: &str| {
            if pct != last_pct || last.elapsed() >= Duration::from_millis(120) {
                last = std::time::Instant::now();
                last_pct = pct;
                let _ = app2.emit("cdn-progress", CdnProgress { game, pct, text: text.to_string() });
            }
        };
        match b.archive_download(&Client::new(), game, accept_guess, &mut progress)? {
            archive::Outcome::NeedsConfirm { note, .. } => Ok(Err(note)),
            archive::Outcome::Ready(f) => {
                let u = b.archive_unpack(game, &f.file, &mut progress)?;
                Ok(Ok((f, u)))
            }
        }
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let (f, u) = match out {
        Err(note) => return Ok(json!({"result": "needs-confirm", "game": game, "note": note})),
        Ok(x) => x,
    };
    let state = app.state::<AppState>();
    {
        let mut b = state.beacon.lock().unwrap();
        b.archive_adopt(&u).map_err(|e| format!("{e:#}"))?;
        b.save().map_err(err)?;
    }
    let mut message = match &u {
        archive::Unpacked::Game { .. } => format!("{} downloaded and ready", game.short()),
        archive::Unpacked::Launcher { .. } => format!("{} held a launcher — starting it in {}'s prefix", f.name, game.short()),
    };
    if let Some(n) = &f.note {
        message.push_str(&format!(" ({n})"));
    }
    let launcher = match &u {
        archive::Unpacked::Launcher { exe, .. } => Some(exe.clone()),
        _ => None,
    };
    Ok(json!({"result": "done", "snapshot": snapshot(&state), "message": message, "launcher": launcher}))
}

/// Is there a newer private release of Beacon?
#[tauri::command]
pub async fn app_update_check(app: AppHandle) -> CmdResult<selfupdate::Check> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || b.update_check(&Client::new())).await.map_err(err)
}

#[derive(Clone, Serialize)]
struct AppUpdateProgress {
    pct: Option<u8>,
    text: String,
}

/// Download, check and install the newest private release, then restart into it (or quit for the MSI).
/// Answers `{next: "manual", file, how}` when the package has to be installed by hand.
#[tauri::command]
pub async fn app_update_apply(app: AppHandle) -> CmdResult<Value> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let applied = tauri::async_runtime::spawn_blocking(move || {
        b.update_apply(&Client::new(), &mut |pct: Option<u8>, text: &str| {
            let _ = app2.emit("app-update-progress", AppUpdateProgress { pct, text: text.to_string() });
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    match &applied {
        selfupdate::Applied::Restart { program } => {
            let mut c = std::process::Command::new(program);
            c.args(std::env::args_os().skip(1));
            // The new copy must not inherit the old AppImage's mount.
            c.env_remove("APPDIR").env_remove("APPIMAGE").env_remove("ARGV0").env_remove("OWD");
            c.spawn().map_err(|e| format!("Updated, but couldn't restart Beacon ({e}); start it again yourself"))?;
            app.exit(0);
        }
        selfupdate::Applied::Quit => app.exit(0),
        selfupdate::Applied::Manual { .. } => {}
    }
    Ok(serde_json::to_value(&applied).map_err(err)?)
}
