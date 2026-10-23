//! downloads in the app. BFME, BFME II and RotWK from the BFME
//! Foundation's CDN (`cdn_get`), and the download sources (check, update,
//! games folder). Progress arrives as `cdn-progress` events; the
//! start-up check of the sources as `sources-checked`.
//!
//! Every download is a *job* in the Downloads menu (`downloads_list`, and a
//! `downloads-changed` event with the job on every change). A job can be
//! paused (`download_pause`: the transfer stops, its `.part` file stays) and
//! run again from the menu, which resumes where it stopped.

use crate::{err, snapshot, AppState, CmdResult};
use beacon_core::downloads::{self, archive, basegame, launcher, modget, selfupdate, sources, Cancellable, Client};
use beacon_core::games::GameId;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

// ---------------------------------------------------------------- download jobs

/// One download in the Downloads menu.
#[derive(Clone, Serialize)]
pub struct Job {
    id: u64,
    /// game, launcher, mod or update.
    kind: &'static str,
    title: String,
    game: Option<GameId>,
    /// running, paused, waiting (needs a yes), done or failed.
    state: &'static str,
    pct: Option<u8>,
    text: String,
    /// Seconds since 1970.
    started: u64,
    ended: Option<u64>,
    /// The result, or why it failed.
    message: Option<String>,
    /// How to run it again: `{cmd, args}`.
    retry: Value,
}

/// The download jobs of this session (managed state).
#[derive(Default)]
pub struct Downloads {
    jobs: Mutex<Vec<Job>>,
    flags: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next: AtomicU64,
}

/// Finished jobs kept in the list.
const KEEP_FINISHED: usize = 25;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn emit_job(app: &AppHandle, job: &Job) {
    let _ = app.emit("downloads-changed", job);
}

/// Start a job; returns its id and the flag that pauses it.
fn job_start(app: &AppHandle, kind: &'static str, title: String, game: Option<GameId>, retry: Value) -> (u64, Arc<AtomicBool>) {
    let d = app.state::<Downloads>();
    let id = d.next.fetch_add(1, Ordering::Relaxed) + 1;
    let flag = Arc::new(AtomicBool::new(false));
    d.flags.lock().unwrap().insert(id, flag.clone());
    let job = Job { id, kind, title, game, state: "running", pct: None, text: "Starting".into(), started: now(), ended: None, message: None, retry };
    {
        let mut jobs = d.jobs.lock().unwrap();
        // The same download run again replaces its old entry.
        jobs.retain(|j| !(j.retry == job.retry && j.state != "running"));
        jobs.push(job.clone());
        let finished: Vec<u64> = jobs.iter().filter(|j| j.state != "running").map(|j| j.id).collect();
        if finished.len() > KEEP_FINISHED {
            let drop: Vec<u64> = finished[..finished.len() - KEEP_FINISHED].to_vec();
            jobs.retain(|j| !drop.contains(&j.id));
        }
    }
    emit_job(app, &job);
    (id, flag)
}

fn job_progress(app: &AppHandle, id: u64, pct: Option<u8>, text: &str) {
    let d = app.state::<Downloads>();
    let job = {
        let mut jobs = d.jobs.lock().unwrap();
        let Some(j) = jobs.iter_mut().find(|j| j.id == id) else { return };
        if pct.is_some() {
            j.pct = pct;
        }
        j.text = text.to_string();
        j.clone()
    };
    emit_job(app, &job);
}

/// Finish a job from its command's result.
fn job_end(app: &AppHandle, id: u64, r: &CmdResult<Value>) {
    let d = app.state::<Downloads>();
    let paused = d.flags.lock().unwrap().remove(&id).map(|f| f.load(Ordering::Relaxed)).unwrap_or(false);
    let job = {
        let mut jobs = d.jobs.lock().unwrap();
        let Some(j) = jobs.iter_mut().find(|j| j.id == id) else { return };
        j.ended = Some(now());
        match r {
            Ok(v) if v.get("result").and_then(Value::as_str) == Some("needs-confirm") => {
                j.state = "waiting";
                j.message = v.get("note").and_then(Value::as_str).map(|n| format!("Its usual download changed: {n}"));
            }
            Ok(v) => {
                j.state = "done";
                j.pct = Some(100);
                j.message = v.get("message").and_then(Value::as_str).map(str::to_string);
            }
            Err(e) if paused || e.ends_with(downloads::PAUSED) => {
                j.state = "paused";
                j.message = Some("Paused — resume to carry on where it stopped".into());
            }
            Err(e) => {
                j.state = "failed";
                j.message = Some(e.clone());
            }
        }
        j.clone()
    };
    emit_job(app, &job);
}

/// A progress callback for a job: at most ~8 updates a second (plus every change of
/// percentage), sent both as the job and as `event` with `{game, pct, text}`.
fn progress_fn(app: AppHandle, id: u64, event: &'static str, game: Option<GameId>) -> impl FnMut(Option<u8>, &str) {
    let mut last = std::time::Instant::now() - Duration::from_secs(1);
    let mut last_pct = None;
    move |pct: Option<u8>, text: &str| {
        if pct != last_pct || last.elapsed() >= Duration::from_millis(120) {
            last = std::time::Instant::now();
            last_pct = pct;
            job_progress(&app, id, pct, text);
            let _ = app.emit(event, json!({"game": game, "pct": pct, "text": text}));
        }
    }
}

/// The downloads of this session, newest last.
#[tauri::command]
pub fn downloads_list(d: State<Downloads>) -> Vec<Job> {
    d.jobs.lock().unwrap().clone()
}

/// Pause a running download (its partial file stays; running it again resumes).
#[tauri::command]
pub fn download_pause(d: State<Downloads>, id: u64) -> CmdResult<()> {
    let flags = d.flags.lock().unwrap();
    let f = flags.get(&id).ok_or("that download isn't running")?;
    f.store(true, Ordering::Relaxed);
    Ok(())
}

/// Take a finished, failed or paused download off the list.
#[tauri::command]
pub fn download_forget(d: State<Downloads>, id: u64) {
    d.jobs.lock().unwrap().retain(|j| j.id != id || j.state == "running");
}

/// What the download folders hold: kept downloads (installers kept to run again, partial downloads) and update packages.
#[tauri::command]
pub fn downloads_storage(state: State<AppState>) -> Value {
    let store = state.beacon.lock().unwrap().download_store();
    let (bytes, files) = downloads::kept_bytes(&store.dir);
    json!({"bytes": bytes, "files": files})
}

/// Remove kept and partial downloads (not while something downloads).
#[tauri::command]
pub fn downloads_clear(state: State<AppState>, d: State<Downloads>) -> CmdResult<u64> {
    if d.jobs.lock().unwrap().iter().any(|j| j.state == "running") {
        return Err("wait for the running downloads to finish (or pause them) first".into());
    }
    let store = state.beacon.lock().unwrap().download_store();
    downloads::clear_kept(&store.dir).map_err(|e| format!("{e:#}"))
}

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
    let what = if verify { "Checking" } else if repair { "Repairing" } else { "" };
    let title = if what.is_empty() { game.title().to_string() } else { format!("{what} {}", game.short()) };
    let retry = json!({"cmd": "cdn_get", "args": {"game": game, "repair": repair, "verify": verify, "acceptGuess": accept_guess}});
    let (id, flag) = job_start(&app, "game", title, Some(game), retry);
    let r = if basegame::supported(game) { cdn_get_inner(app.clone(), game, repair, verify, accept_guess, id, flag).await } else { archive_get(app.clone(), game, accept_guess, id, flag).await };
    job_end(&app, id, &r);
    r
}

async fn cdn_get_inner(app: AppHandle, game: GameId, repair: bool, verify: bool, accept_guess: bool, id: u64, flag: Arc<AtomicBool>) -> CmdResult<Value> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let opts = basegame::Options { thorough: verify, accept_guess, reinstall: repair || verify };
    let out = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = progress_fn(app2, id, "cdn-progress", Some(game));
        b.cdn_download(&Cancellable::new(Client::new(), flag), game, opts, &mut progress)
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
    job_progress(&app, id, Some(100), "Writing the registry keys");
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
    let retry = json!({"cmd": "launcher_get", "args": {"game": game, "force": force, "acceptGuess": accept_guess}});
    let (id, flag) = job_start(&app, "launcher", format!("{} launcher", game.short()), Some(game), retry);
    let r = launcher_get_inner(app.clone(), game, force, accept_guess, id, flag).await;
    job_end(&app, id, &r);
    r
}

async fn launcher_get_inner(app: AppHandle, game: GameId, force: bool, accept_guess: bool, id: u64, flag: Arc<AtomicBool>) -> CmdResult<Value> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let out = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = progress_fn(app2, id, "community-launcher-progress", Some(game));
        b.launcher_download(&Cancellable::new(Client::new(), flag), game, accept_guess, force, &mut progress)
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
    job_progress(&app, id, Some(100), "Setting up the launcher");
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
async fn archive_get(app: AppHandle, game: GameId, accept_guess: bool, id: u64, flag: Arc<AtomicBool>) -> CmdResult<Value> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    if !b.has_archive_source(game) {
        return Err(format!("Beacon has no download for {}", game.short()));
    }
    let app2 = app.clone();
    let out = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Result<(archive::Fetched, archive::Unpacked), String>> {
        let mut progress = progress_fn(app2, id, "cdn-progress", Some(game));
        match b.archive_download(&Cancellable::new(Client::new(), flag), game, accept_guess, &mut progress)? {
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
    let (id, flag) = job_start(&app, "update", "Beacon update".into(), None, json!({"cmd": "app_update_apply", "args": {}}));
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let applied = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = progress_fn(app2.clone(), id, "app-update-progress", None);
        b.update_apply(&Cancellable::new(Client::new(), flag), &mut progress)
    })
    .await
    .map_err(err)
    .and_then(|r| r.map_err(|e| format!("{e:#}")));
    let applied = match applied {
        Ok(a) => {
            job_end(&app, id, &Ok(json!({"message": "Installed — restarting Beacon"})));
            a
        }
        Err(e) => {
            let r: CmdResult<Value> = Err(e.clone());
            job_end(&app, id, &r);
            return Err(e);
        }
    };
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

/// Mod downloads for a game's catalog: `{id, title, catalog_id}`.
#[tauri::command]
pub fn mod_downloads(state: State<AppState>, game: GameId) -> Vec<Value> {
    state.beacon.lock().unwrap().mod_sources(game).into_iter().map(|s| json!({"id": s.id, "title": s.title, "catalog_id": s.catalog_id})).collect()
}

/// What a mod source (`id`) or a ModDB link (`link`) leads to now; nothing is downloaded.
#[tauri::command]
pub async fn mod_lookup(app: AppHandle, game: GameId, id: Option<String>, link: Option<String>) -> CmdResult<modget::Found> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let r = match (id, link) {
        (Some(id), _) => modget::ModRef::Source(id),
        (None, Some(l)) => modget::ModRef::Link(l.trim().to_string()),
        _ => return Err("nothing to look up".into()),
    };
    tauri::async_runtime::spawn_blocking(move || b.mod_lookup(&Client::new(), game, &r)).await.map_err(err)?.map_err(|e| format!("{e:#}"))
}

#[derive(Clone, Serialize)]
struct ModProgress {
    game: GameId,
    pct: Option<u8>,
    text: String,
}

/// Download and install what `mod_lookup` found. A library mod is selected for the game; an
/// installer comes back as `{kind: "installer", exe}` for the UI to run in the game's prefix.
#[tauri::command]
pub async fn mod_install(app: AppHandle, found: modget::Found) -> CmdResult<Value> {
    let retry = json!({"cmd": "mod_install", "args": {"found": found}});
    let (id, flag) = job_start(&app, "mod", found.name.clone(), Some(found.game), retry);
    let r = mod_install_inner(app.clone(), found, id, flag).await;
    job_end(&app, id, &r);
    r
}

async fn mod_install_inner(app: AppHandle, found: modget::Found, id: u64, flag: Arc<AtomicBool>) -> CmdResult<Value> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let game = found.game;
    let f2 = found.clone();
    let done = tauri::async_runtime::spawn_blocking(move || {
        let mut progress = progress_fn(app2, id, "mod-progress", Some(game));
        b.mod_install(&Cancellable::new(Client::new(), flag), &f2, &mut progress)
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let state = app.state::<AppState>();
    let message = {
        let mut b = state.beacon.lock().unwrap();
        match &done {
            modget::Installed::Library { mod_id, .. } => {
                if let Some(id) = mod_id {
                    let _ = b.select_mod(game, Some(id));
                    let _ = b.save();
                }
                format!("{} installed and selected", found.name)
            }
            modget::Installed::GameFolder { files, .. } => format!("{} copied into {}'s folder ({files} files; replaced ones are backed up)", found.name, game.short()),
            modget::Installed::Installer { .. } => format!("{} downloaded — starting its setup in {}'s prefix", found.name, game.short()),
        }
    };
    Ok(json!({"installed": done, "snapshot": snapshot(&state), "message": message}))
}
