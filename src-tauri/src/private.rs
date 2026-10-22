//! Private build: downloads in the app. BFME, BFME II and RotWK from the BFME
//! Foundation's CDN (`cdn_get`), and the download sources (check, update,
//! token, games folder). Progress arrives as `cdn-progress` events; the
//! start-up check of the sources as `sources-checked`.

use crate::{err, snapshot, AppState, CmdResult};
use beacon_core::downloads::{basegame, sources, Client};
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
    let checks: Option<Vec<sources::Check>> = std::fs::read_to_string(store.status_file()).ok().and_then(|t| serde_json::from_str(&t).ok());
    json!({
        "version": list.version,
        "built_in": list == sources::SourceList::bundled(),
        "sources_repo": s.sources_repo,
        "token_set": s.github_token.as_deref().map(|t| !t.is_empty()).unwrap_or(false),
        "games_dir": b.games_root(),
        "custom_games_dir": s.games_dir.is_some(),
        "learned_api": store.learned().workshop_api,
        "checks": checks,
        "games": ["bfme1", "bfme2", "rotwk"],
    })
}

/// Download a game from the CDN (and BFME II for RotWK when it's missing), point Beacon at it and
/// write its registry keys. Answers `{result: "done", snapshot, message}` or
/// `{result: "needs-confirm", game, note}` when the source only turned up a guess.
#[tauri::command]
pub async fn cdn_get(app: AppHandle, game: GameId, repair: bool, verify: bool, accept_guess: bool) -> CmdResult<Value> {
    if !basegame::supported(game) {
        return Err(format!("{} isn't on the BFME Foundation's CDN", game.short()));
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

#[tauri::command]
pub fn set_github_token(state: State<AppState>, token: String) -> CmdResult<()> {
    let store = state.beacon.lock().unwrap().download_store();
    let mut s = store.settings();
    let t = token.trim();
    if t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("that doesn't look like a GitHub token".into());
    }
    s.github_token = (!t.is_empty()).then(|| t.to_string());
    store.save_settings(&s).map_err(err)
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
    });
}
