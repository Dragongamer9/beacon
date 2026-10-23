//! Beacon desktop app: a thin Tauri shell over beacon-core.

// Windows release builds: no console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use beacon_core::{
    aio_launcher::LauncherStatus,
    custom::{self, CustomStatus},
    games::GameId,
    media,
    mods::ModEntry,
    runner::{Command, Runner},
    tools, workshop, Beacon, CatalogItem, Community, GameStatus, Resolution,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Downloads.
mod private;

struct AppState {
    beacon: Mutex<Beacon>,
    running: Mutex<BTreeSet<GameId>>,
    /// Games added by hand that are running (or whose installer, tool or program is).
    running_custom: Mutex<BTreeSet<String>>,
}

/// A game added by hand, with its icon as a data: URL for the shelf.
#[derive(Serialize)]
struct CustomView {
    #[serde(flatten)]
    status: CustomStatus,
    icon_url: Option<String>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
struct Snapshot {
    version: &'static str,
    games: Vec<GameStatus>,
    runners: Vec<Runner>,
    default_runner: Option<String>,
    best_runner: Option<String>,
    mods_dir: PathBuf,
    data_dir: PathBuf,
    /// Running games: built-in ids ("rotwk") and "custom:<id>".
    running: Vec<String>,
    /// Games added by hand (Other Games).
    custom: Vec<CustomView>,
    /// Faugus Launcher's library, when there is one to import.
    faugus: Option<PathBuf>,
    flags: Vec<(&'static str, &'static str)>,
    stores: &'static [&'static str],
    wine_tools: &'static [(&'static str, &'static str)],
    tools_auto_update: bool,
    /// Windows: games run directly (no Wine prefixes or runners to pick).
    native: bool,
    /// Where to get the BFME All-in-One launcher.
    aio_url: &'static str,
    /// The All-in-One launcher itself: installed?, where.
    aio_launcher: LauncherStatus,
}

fn snapshot(state: &AppState) -> Snapshot {
    let b = state.beacon.lock().unwrap();
    Snapshot {
        version: env!("CARGO_PKG_VERSION"),
        games: b.statuses(),
        runners: b.runners.runners.clone(),
        default_runner: b.config.default_runner.clone(),
        best_runner: b.runners.best().map(|r| r.id.clone()),
        mods_dir: b.paths.mods_dir(),
        data_dir: b.paths.data_dir.clone(),
        running: state
            .running
            .lock()
            .unwrap()
            .iter()
            .map(|g| g.slug().to_string())
            .chain(state.running_custom.lock().unwrap().iter().map(|id| format!("custom:{id}")))
            .collect(),
        custom: b
            .custom_statuses()
            .into_iter()
            .map(|status| CustomView { icon_url: status.game.icon.as_deref().and_then(custom::data_url), status })
            .collect(),
        faugus: b.faugus_games_file(),
        flags: custom::FLAGS.iter().map(|(k, _, _, label)| (*k, *label)).collect(),
        stores: custom::STORES,
        wine_tools: custom::WINE_TOOLS,
        tools_auto_update: b.config.tools_auto_update,
        native: b.native,
        aio_url: beacon_core::aio::LAUNCHER_URL,
        aio_launcher: b.launcher_status(),
    }
}

/// Change config under the lock, save, return a fresh snapshot.
fn mutate(state: &AppState, f: impl FnOnce(&mut Beacon) -> anyhow::Result<()>) -> CmdResult<Snapshot> {
    {
        let mut b = state.beacon.lock().unwrap();
        f(&mut b).map_err(err)?;
        b.save().map_err(err)?;
    }
    Ok(snapshot(state))
}

#[tauri::command]
fn get_state(state: State<AppState>) -> Snapshot {
    adopt_launcher_games(&state);
    snapshot(&state)
}

/// Games the All-in-One launcher (or a game's community launcher) downloaded since the last look
/// (cheap; no full scan).
fn adopt_launcher_games(state: &AppState) {
    let mut b = state.beacon.lock().unwrap();
    let mut adopted = !b.adopt_launcher_games().is_empty();
    for g in GameId::ALL {
        if b.game(g).community_launcher.is_some() && !b.adopt_community_games(g).is_empty() {
            adopted = true;
        }
    }
    if adopted {
        let _ = b.save();
    }
}

#[tauri::command]
fn scan(state: State<AppState>) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        b.refresh_runners();
        b.scan(&[]);
        Ok(())
    })
}

#[tauri::command]
fn set_game_path(state: State<AppState>, game: GameId, path: String) -> CmdResult<Snapshot> {
    mutate(&state, |b| b.set_path(game, &PathBuf::from(path)))
}

#[tauri::command]
fn set_cd_key(state: State<AppState>, game: GameId, key: String) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        let key = key.trim().to_string();
        b.game_mut(game).cd_key = (!key.is_empty()).then_some(key);
        Ok(())
    })
}

/// `game = None` sets the default runner; `runner = None` means automatic.
#[tauri::command]
fn set_runner(state: State<AppState>, game: Option<GameId>, runner: Option<String>) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        if let Some(id) = &runner {
            if b.runners.get(id).is_none() {
                anyhow::bail!("runner {id} not found");
            }
        }
        match game {
            Some(g) => b.game_mut(g).runner = runner,
            None => b.config.default_runner = runner,
        }
        Ok(())
    })
}

#[tauri::command]
fn set_display(state: State<AppState>, game: GameId, resolution: Option<String>, windowed: bool, extra_args: String) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        let res = match resolution.as_deref() {
            None | Some("") => None,
            Some(s) => Some(Resolution::parse(s).ok_or_else(|| anyhow::anyhow!("bad resolution {s}"))?),
        };
        let cfg = b.game_mut(game);
        cfg.resolution = res;
        cfg.windowed = windowed;
        cfg.extra_args = extra_args;
        Ok(())
    })
}

/// Per-game launch switches for non-BFME games.
#[tauri::command]
fn set_launch_options(state: State<AppState>, game: GameId, virtual_desktop: Option<bool>, via_steam: Option<bool>) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        let cfg = b.game_mut(game);
        if virtual_desktop.is_some() {
            cfg.virtual_desktop = virtual_desktop;
        }
        if let Some(v) = via_steam {
            cfg.via_steam = v;
        }
        Ok(())
    })
}

#[tauri::command]
fn get_community(state: State<AppState>, game: GameId) -> Community {
    state.beacon.lock().unwrap().community(game)
}

#[tauri::command]
fn set_edition(state: State<AppState>, game: GameId, id: Option<String>) -> CmdResult<Snapshot> {
    mutate(&state, |b| b.set_edition(game, id.as_deref()))
}

/// Merge or add a community download for a setup step. Copying can take a
/// while (Expanding Fronts is >1 GB), so it runs on a copy of the state off
/// the main thread, then the step/edition result is folded back in.
#[tauri::command]
async fn add_files(app: AppHandle, game: GameId, step: String, path: String) -> CmdResult<(Snapshot, String)> {
    let mut work = app.state::<AppState>().beacon.lock().unwrap().clone();
    let (work, msg) = tauri::async_runtime::spawn_blocking(move || {
        let r = work.add_files(game, &step, &PathBuf::from(path));
        (work, r)
    })
    .await
    .map_err(err)?;
    let msg = msg.map_err(err)?;
    let state = app.state::<AppState>();
    {
        let mut b = state.beacon.lock().unwrap();
        let done = work.game(game);
        let cfg = b.game_mut(game);
        cfg.done_steps = done.done_steps;
        cfg.edition = done.edition;
        b.save().map_err(err)?;
    }
    Ok((snapshot(&state), msg))
}

#[tauri::command]
fn list_mods(state: State<AppState>, game: GameId) -> Vec<ModEntry> {
    state.beacon.lock().unwrap().mods(game)
}

#[tauri::command]
fn get_catalog(state: State<AppState>, game: GameId) -> Vec<CatalogItem> {
    state.beacon.lock().unwrap().catalog(game)
}

/// Look for known mods already on this PC and add them. Returns how many.
#[tauri::command]
fn find_mods(state: State<AppState>) -> CmdResult<(Snapshot, usize)> {
    let mut added = 0;
    let snap = mutate(&state, |b| {
        added = b.find_mods(&[]).len();
        Ok(())
    })?;
    Ok((snap, added))
}

/// Hand a link or a path to the desktop: xdg-open on Linux, Explorer on Windows.
fn system_open(target: &std::ffi::OsStr) -> std::io::Result<()> {
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(target).spawn().map(|_| ())
}

/// Open a mod's web page in the default browser.
#[tauri::command]
fn open_url(url: String) -> CmdResult<()> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("only https links can be opened".into());
    }
    system_open(std::ffi::OsStr::new(&url)).map_err(err)
}

/// The BFME workshop's fan content for a game (network, so off the main thread).
#[derive(Serialize)]
struct WorkshopPage {
    items: Vec<workshop::Item>,
    hidden: usize,
    source: &'static str,
}

#[tauri::command]
async fn workshop_list(app: AppHandle, game: GameId, search: Option<String>, page: Option<u32>) -> CmdResult<WorkshopPage> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let (items, hidden) = tauri::async_runtime::spawn_blocking(move || {
        b.workshop_list(&workshop::Net::new(), game, search.as_deref().unwrap_or(""), page.unwrap_or(0))
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    Ok(WorkshopPage { items, hidden, source: workshop::SITE })
}

#[derive(Clone, Serialize)]
struct WorkshopProgress {
    game: GameId,
    id: String,
    pct: Option<u8>,
    text: String,
}

/// Download a workshop entry into the mods library; progress arrives as `workshop-progress` events.
#[tauri::command]
async fn workshop_install(app: AppHandle, game: GameId, id: String) -> CmdResult<(Snapshot, String)> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let id2 = id.clone();
    let done = tauri::async_runtime::spawn_blocking(move || {
        b.workshop_install(&workshop::Net::new(), game, &id2, &mut |pct: Option<u8>, text: &str| {
            let _ = app2.emit("workshop-progress", WorkshopProgress { game, id: id2.clone(), pct, text: text.to_string() });
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let state = app.state::<AppState>();
    Ok((snapshot(&state), format!("{} {} installed", done.0.name, done.0.version)))
}

/// Games the BFME All-in-One launcher manages, and where to get it.
#[tauri::command]
fn aio_info(state: State<AppState>) -> serde_json::Value {
    adopt_launcher_games(&state);
    beacon_core::report::aio(&state.beacon.lock().unwrap())
}

#[derive(Clone, Serialize)]
struct LauncherProgress {
    pct: Option<u8>,
    text: String,
}

/// Download the All-in-One launcher from the BFME Foundation and install it (Linux: in its own
/// prefix). Progress arrives as `aio-launcher-progress` events. Never downloads the games.
#[tauri::command]
async fn aio_launcher_install(app: AppHandle) -> CmdResult<(Snapshot, String)> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let done = tauri::async_runtime::spawn_blocking(move || {
        b.install_launcher(&workshop::Net::new(), &mut |pct: Option<u8>, text: &str| {
            let _ = app2.emit("aio-launcher-progress", LauncherProgress { pct, text: text.to_string() });
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let state = app.state::<AppState>();
    let msg = if done.verified {
        "All-in-One launcher installed".to_string()
    } else {
        "All-in-One launcher installed (its published checksum couldn't be read, so it wasn't compared)".to_string()
    };
    Ok((snapshot(&state), msg))
}

/// Start the All-in-One launcher. When it closes, Beacon looks for the games it downloaded and
/// sends `aio-launcher-closed`.
#[tauri::command]
fn aio_launcher_open(app: AppHandle, state: State<AppState>) -> CmdResult<Snapshot> {
    let mut child = state.beacon.lock().unwrap().open_launcher().map_err(|e| format!("{e:#}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
        let state = app.state::<AppState>();
        {
            let mut b = state.beacon.lock().unwrap();
            if b.adopt_launcher_games().is_empty() {
                // Installed somewhere else (another drive letter, say): a full look.
                b.scan(&[]);
            }
            let _ = b.save();
        }
        let _ = app.emit("aio-launcher-closed", ());
    });
    Ok(snapshot(&state))
}

/// Use a community launcher the player downloaded (its .exe, or the .zip it came in). Extracting can
/// take a moment, so it runs on a copy of the state off the main thread; the choice is folded back in.
#[tauri::command]
async fn community_launcher_set(app: AppHandle, game: GameId, path: String) -> CmdResult<(Snapshot, String)> {
    let mut work = app.state::<AppState>().beacon.lock().unwrap().clone();
    let exe = tauri::async_runtime::spawn_blocking(move || work.set_community_launcher(game, &PathBuf::from(path), &mut |_: &str| {}))
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))?;
    let state = app.state::<AppState>();
    {
        let mut b = state.beacon.lock().unwrap();
        b.game_mut(game).community_launcher = Some(exe.clone());
        b.save().map_err(err)?;
    }
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok((snapshot(&state), format!("{} launcher set: {name}", game.short())))
}

#[derive(Clone, Serialize)]
struct CommunityLauncherProgress {
    game: GameId,
    pct: Option<u8>,
    text: String,
}

#[derive(Clone, Serialize)]
struct CommunityLauncherClosed {
    game: GameId,
    /// The game was found after the launcher closed (it had no folder before).
    found: bool,
}

/// Start a game's community launcher (Linux: in the game's prefix, created first when needed; progress as
/// `community-launcher-progress`). When it closes, Beacon looks for the game it downloaded and sends
/// `community-launcher-closed`.
#[tauri::command]
async fn community_launcher_open(app: AppHandle, game: GameId) -> CmdResult<Snapshot> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    let app2 = app.clone();
    let mut child = tauri::async_runtime::spawn_blocking(move || {
        b.open_community_launcher(game, &mut |pct: Option<u8>, text: &str| {
            let _ = app2.emit("community-launcher-progress", CommunityLauncherProgress { game, pct, text: text.to_string() });
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let app3 = app.clone();
    std::thread::spawn(move || {
        let _ = child.wait();
        let state = app3.state::<AppState>();
        let found = {
            let mut b = state.beacon.lock().unwrap();
            let found = !b.rescan_after_community_launcher(game).is_empty();
            let _ = b.save();
            found
        };
        let _ = app3.emit("community-launcher-closed", CommunityLauncherClosed { game, found });
    });
    let state = app.state::<AppState>();
    Ok(snapshot(&state))
}

#[tauri::command]
fn add_mod(state: State<AppState>, game: GameId, path: String, name: Option<String>) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        let m = b.link_mod(game, &PathBuf::from(path), name.as_deref().filter(|n| !n.trim().is_empty()))?;
        b.select_mod(game, Some(&m.id))
    })
}

#[tauri::command]
fn remove_mod(state: State<AppState>, id: String) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        b.unlink_mod(&id);
        Ok(())
    })
}

#[tauri::command]
fn select_mod(state: State<AppState>, game: GameId, id: Option<String>) -> CmdResult<Snapshot> {
    mutate(&state, |b| b.select_mod(game, id.as_deref().filter(|s| !s.is_empty())))
}

#[derive(Clone, Serialize)]
struct GameEvent {
    game: GameId,
    /// preparing | running | exited | error
    phase: &'static str,
    message: Option<String>,
}

fn emit(app: &AppHandle, game: GameId, phase: &'static str, message: Option<String>) {
    let _ = app.emit("game-event", GameEvent { game, phase, message });
}

/// What `start` runs.
enum Job {
    /// The game (or its selected mod's launcher).
    Launch(Option<String>),
    /// An installer or patch exe, for a community step or not.
    Installer(PathBuf, Option<String>),
    /// The player's own copy: an installer, a disc folder or a disc image.
    FromCopy(PathBuf),
}

/// Start a game, an installer, or an install from the player's own copy.
/// Progress arrives as `game-event`s; the call itself returns at once.
fn start(app: AppHandle, state: &AppState, game: GameId, job: Job) -> CmdResult<()> {
    let beacon = state.beacon.lock().unwrap().clone();
    // Fail fast on anything we can check without running Wine (or the installer).
    let cmd: Option<Command> = match &job {
        Job::Installer(exe, _) => Some(beacon.run_in_prefix_command(game, exe).map_err(err)?),
        Job::Launch(mod_id) => Some(beacon.launch_command(game, mod_id.as_deref()).map_err(err)?),
        Job::FromCopy(src) => {
            media::classify(src).map_err(|e| format!("{e:#}"))?;
            None
        }
    };
    let installer = !matches!(job, Job::Launch(_));
    let step = match &job {
        Job::Installer(_, step) => step.clone(),
        _ => None,
    };
    if !state.running.lock().unwrap().insert(game) {
        return Err(format!("{} is already running", game.short()));
    }

    std::thread::spawn(move || {
        emit(&app, game, "preparing", None);
        // A disc image is extracted first, then its setup found.
        let cmd = match (cmd, &job) {
            (Some(c), _) => Ok(c),
            (None, Job::FromCopy(src)) => beacon
                .prepare_install(game, src, &mut |t: &str| emit(&app, game, "preparing", Some(t.to_string())))
                .and_then(|p| beacon.run_in_prefix_command(game, &p.setup)),
            (None, _) => Err(anyhow::anyhow!("nothing to run")),
        };
        // An installer may be installing the game itself: its prefix can't get the game's registry keys yet.
        let started = cmd.and_then(|cmd| {
            (if installer { beacon.prepare_prefix(game) } else { beacon.setup(game, false) })
                .and_then(|_| beacon.apply_options(game))
                .and_then(|_| beacon.spawn_logged(game, &cmd))
        });
        match started {
            Ok(mut child) => {
                emit(&app, game, "running", None);
                let window = app.get_webview_window("main");
                if !installer {
                    if let Some(w) = &window {
                        let _ = w.minimize();
                    }
                }
                let started = std::time::Instant::now();
                let status = child.wait();
                if !installer {
                    let _ = beacon.played(game, started.elapsed().as_secs());
                }
                if let Some(w) = &window {
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
                if installer {
                    // A finished installer may have just put a game in the prefix.
                    let state = app.state::<AppState>();
                    let mut b = state.beacon.lock().unwrap();
                    b.scan(&[]);
                    if let (Some(step), Ok(s)) = (&step, &status) {
                        if s.success() {
                            b.mark_step(game, step);
                        }
                    }
                    let _ = b.save();
                }
                let msg = match status {
                    Ok(s) if s.success() => None,
                    Ok(s) => Some(format!("Exited with {s}. Log: {}", beacon.paths.log_file(game).display())),
                    Err(e) => Some(e.to_string()),
                };
                emit(&app, game, "exited", msg);
            }
            Err(e) => emit(&app, game, "error", Some(format!("{e:#}"))),
        }
        app.state::<AppState>().running.lock().unwrap().remove(&game);
    });
    Ok(())
}

#[tauri::command]
fn launch(app: AppHandle, state: State<AppState>, game: GameId, mod_id: Option<String>) -> CmdResult<()> {
    start(app.clone(), &state, game, Job::Launch(mod_id))
}

#[tauri::command]
fn run_installer(app: AppHandle, state: State<AppState>, game: GameId, path: String, step: Option<String>) -> CmdResult<()> {
    start(app.clone(), &state, game, Job::Installer(PathBuf::from(path), step))
}

/// Install a game from the player's own copy: installer .exe, disc image (.iso) or disc folder.
#[tauri::command]
fn install_from(app: AppHandle, state: State<AppState>, game: GameId, path: String) -> CmdResult<()> {
    start(app.clone(), &state, game, Job::FromCopy(PathBuf::from(path)))
}

#[tauri::command]
fn launch_preview(state: State<AppState>, game: GameId, mod_id: Option<String>) -> CmdResult<String> {
    let b = state.beacon.lock().unwrap();
    Ok(b.launch_command(game, mod_id.as_deref()).map_err(err)?.display())
}

// ---------------------------------------------------------------- games added by hand (Faugus-style)

#[derive(Clone, Serialize)]
struct CustomEvent {
    id: String,
    /// preparing | running | exited | error
    phase: &'static str,
    /// launch | install | run | tool
    job: &'static str,
    message: Option<String>,
}

fn emit_custom(app: &AppHandle, id: &str, job: &'static str, phase: &'static str, message: Option<String>) {
    let _ = app.emit("custom-event", CustomEvent { id: id.to_string(), phase, job, message });
}

#[derive(Clone, Serialize)]
struct InstallDone {
    id: String,
    ok: bool,
    candidates: Vec<String>,
    message: Option<String>,
}

enum CustomJob {
    Launch,
    Install(PathBuf),
    Run(PathBuf),
    Tool(String),
}

/// Start something for a game added by hand on a thread: the game (play time is recorded when it closes), its
/// installer (then the programs it installed are offered), another program in its prefix, or a Wine tool.
fn start_custom(app: AppHandle, state: &AppState, id: String, job: CustomJob) -> CmdResult<()> {
    let beacon = state.beacon.lock().unwrap().clone();
    let kind: &'static str = match &job {
        CustomJob::Launch => "launch",
        CustomJob::Install(_) => "install",
        CustomJob::Run(_) => "run",
        CustomJob::Tool(_) => "tool",
    };
    let cmd = match &job {
        CustomJob::Launch => beacon.custom_launch_command(&id),
        CustomJob::Install(p) | CustomJob::Run(p) => beacon.custom_run_command(&id, p),
        CustomJob::Tool(t) => beacon.custom_tool_command(&id, t, &[]),
    }
    .map_err(|e| format!("{e:#}"))?;
    if !state.running_custom.lock().unwrap().insert(id.clone()) {
        return Err("That game (or something in its prefix) is already running".into());
    }
    std::thread::spawn(move || {
        emit_custom(&app, &id, kind, "preparing", None);
        let started = beacon
            .custom_game(&id)
            .and_then(|g| if matches!(job, CustomJob::Tool(ref t) if t == "kill") { Ok(()) } else { beacon.custom_prepare(&g) })
            .and_then(|_| beacon.custom_spawn(&id, &cmd, kind));
        match started {
            Ok(mut child) => {
                emit_custom(&app, &id, kind, "running", None);
                let window = app.get_webview_window("main");
                if kind == "launch" {
                    if let Some(w) = &window {
                        let _ = w.minimize();
                    }
                }
                let t0 = std::time::Instant::now();
                let status = child.wait();
                if kind == "launch" {
                    let _ = beacon.custom_played(&id, t0.elapsed().as_secs());
                    if let Some(w) = &window {
                        let _ = w.unminimize();
                        let _ = w.set_focus();
                    }
                }
                let msg = match &status {
                    Ok(s) if s.success() => None,
                    // Installers and tools often end with a code; only the game's own failure is shown.
                    Ok(s) if kind == "launch" => Some(format!("Exited with {s}. Log: {}", beacon.custom_log(&id).display())),
                    Ok(_) => None,
                    Err(e) => Some(e.to_string()),
                };
                if let CustomJob::Install(_) = job {
                    let candidates = beacon.custom_game(&id).map(|g| beacon.exe_candidates(&g)).unwrap_or_default();
                    let _ = app.emit(
                        "custom-install-done",
                        InstallDone { id: id.clone(), ok: status.as_ref().map(|s| s.success()).unwrap_or(false), candidates: candidates.iter().map(|p| p.to_string_lossy().into_owned()).collect(), message: msg.clone() },
                    );
                }
                emit_custom(&app, &id, kind, "exited", msg);
            }
            Err(e) => emit_custom(&app, &id, kind, "error", Some(format!("{e:#}"))),
        }
        app.state::<AppState>().running_custom.lock().unwrap().remove(&id);
    });
    Ok(())
}

#[tauri::command]
fn custom_add(state: State<AppState>, path: String, title: Option<String>) -> CmdResult<(Snapshot, String)> {
    let g = state.beacon.lock().unwrap().custom_add(&PathBuf::from(path), title.as_deref(), "", None).map_err(|e| format!("{e:#}"))?;
    Ok((snapshot(&state), g.id))
}

#[tauri::command]
fn custom_set(state: State<AppState>, id: String, key: String, value: String) -> CmdResult<Snapshot> {
    state.beacon.lock().unwrap().custom_set(&id, &key, &value).map_err(|e| format!("{e:#}"))?;
    Ok(snapshot(&state))
}

#[tauri::command]
fn custom_remove(state: State<AppState>, id: String, delete_prefix: bool) -> CmdResult<(Snapshot, String)> {
    if state.running_custom.lock().unwrap().contains(&id) {
        return Err("Close the game first".into());
    }
    let r = state.beacon.lock().unwrap().custom_remove(&id, delete_prefix).map_err(|e| format!("{e:#}"))?;
    let msg = match (&r.deleted_prefix, &r.kept_prefix) {
        (Some(_), _) => "Removed, with its Wine prefix".to_string(),
        (None, Some(why)) if delete_prefix => format!("Removed; its prefix was kept ({why})"),
        _ => "Removed (its Wine prefix was kept)".to_string(),
    };
    Ok((snapshot(&state), msg))
}

#[tauri::command]
fn custom_launch(app: AppHandle, state: State<AppState>, id: String) -> CmdResult<()> {
    start_custom(app.clone(), &state, id, CustomJob::Launch)
}

/// The install flow: a new game with its own prefix, its installer run there; `custom-install-done` then offers the
/// programs it installed.
#[tauri::command]
fn custom_install(app: AppHandle, state: State<AppState>, path: String, title: Option<String>) -> CmdResult<(Snapshot, String)> {
    let p = PathBuf::from(&path);
    let g = state.beacon.lock().unwrap().custom_install_begin(&p, title.as_deref(), None).map_err(|e| format!("{e:#}"))?;
    start_custom(app.clone(), &state, g.id.clone(), CustomJob::Install(p))?;
    Ok((snapshot(&state), g.id))
}

#[tauri::command]
fn custom_run(app: AppHandle, state: State<AppState>, id: String, path: String) -> CmdResult<()> {
    start_custom(app.clone(), &state, id, CustomJob::Run(PathBuf::from(path)))
}

#[tauri::command]
fn custom_tool(app: AppHandle, state: State<AppState>, id: String, tool: String) -> CmdResult<()> {
    start_custom(app.clone(), &state, id, CustomJob::Tool(tool))
}

#[tauri::command]
fn custom_candidates(state: State<AppState>, id: String) -> CmdResult<Vec<String>> {
    let b = state.beacon.lock().unwrap();
    let g = b.custom_game(&id).map_err(|e| format!("{e:#}"))?;
    Ok(b.exe_candidates(&g).into_iter().map(|p| p.to_string_lossy().into_owned()).collect())
}

#[tauri::command]
fn custom_preview(state: State<AppState>, id: String) -> CmdResult<String> {
    Ok(state.beacon.lock().unwrap().custom_launch_command(&id).map_err(|e| format!("{e:#}"))?.display())
}

/// The cover and hero pictures (data: URLs), loaded when a game is selected.
#[tauri::command]
fn custom_art(state: State<AppState>, id: String) -> serde_json::Value {
    let g = state.beacon.lock().unwrap().custom_game(&id).ok();
    let url = |p: Option<&PathBuf>| p.and_then(|p| custom::data_url(p));
    serde_json::json!({"cover": url(g.as_ref().and_then(|g| g.cover.as_ref())), "hero": url(g.as_ref().and_then(|g| g.hero.as_ref()))})
}

#[tauri::command]
async fn custom_export(app: AppHandle) -> CmdResult<Option<String>> {
    let Some(p) = app.dialog().file().set_file_name("beacon-games.json").add_filter("JSON", &["json"]).blocking_save_file().and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    let n = app.state::<AppState>().beacon.lock().unwrap().custom_export(&p).map_err(|e| format!("{e:#}"))?;
    Ok(Some(format!("Saved {n} games to {}", p.display())))
}

#[tauri::command]
fn custom_import(state: State<AppState>, path: Option<String>) -> CmdResult<(Snapshot, String)> {
    let r = {
        let b = state.beacon.lock().unwrap();
        match path {
            Some(p) => b.custom_import(&PathBuf::from(p)),
            None => b.import_faugus(),
        }
        .map_err(|e| format!("{e:#}"))?
    };
    let msg = match (r.added.len(), r.skipped.len()) {
        (0, 0) => "Nothing to import".to_string(),
        (a, 0) => format!("Added {a} game{}", if a == 1 { "" } else { "s" }),
        (a, s) => format!("Added {a} game{}, skipped {s}", if a == 1 { "" } else { "s" }),
    };
    Ok((snapshot(&state), msg))
}

// ---------------------------------------------------------------- compatibility tools (ProtonPlus-style)

#[tauri::command]
async fn tools_list(app: AppHandle, target: String, online: bool) -> CmdResult<serde_json::Value> {
    if !tools::valid_target(&target) {
        return Err(format!("unknown target {target}"));
    }
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || beacon_core::report::tools(&b, &workshop::Net::new(), &target, online)).await.map_err(err)
}

#[tauri::command]
async fn tools_releases(app: AppHandle, tool: String) -> CmdResult<Vec<tools::Release>> {
    let t = tools::find(&tool).ok_or_else(|| format!("unknown tool {tool}"))?;
    let data = app.state::<AppState>().beacon.lock().unwrap().paths.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || tools::releases(&workshop::Net::new(), &data, t, false)).await.map_err(err)?.map_err(|e| format!("{e:#}"))
}

#[derive(Clone, Serialize)]
struct ToolsProgress {
    tool: String,
    pct: Option<u8>,
    text: String,
}

/// Download and install a tool (`version`: a tag, "latest" for the self-updating copy, or none for the newest);
/// progress arrives as `tools-progress`.
#[tauri::command]
async fn tools_install(app: AppHandle, tool: String, target: String, version: Option<String>) -> CmdResult<(Snapshot, String)> {
    let t = tools::find(&tool).ok_or_else(|| format!("unknown tool {tool}"))?;
    if !tools::valid_target(&target) {
        return Err(format!("unknown target {target}"));
    }
    let (home, data) = {
        let b = app.state::<AppState>().beacon.lock().unwrap();
        (b.paths.home.clone(), b.paths.data_dir.clone())
    };
    let app2 = app.clone();
    let tool2 = tool.clone();
    let done = tauri::async_runtime::spawn_blocking(move || {
        let latest = version.as_deref() == Some("latest");
        let tag = version.as_deref().filter(|v| *v != "latest");
        tools::install(&workshop::Net::new(), &home, &data, t, &target, tag, latest, &mut |pct: Option<u8>, text: &str| {
            let _ = app2.emit("tools-progress", ToolsProgress { tool: tool2.clone(), pct, text: text.to_string() });
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let state = app.state::<AppState>();
    state.beacon.lock().unwrap().refresh_runners();
    let msg = format!("{} {} installed{}", t.name, done.tag, if done.verified { " · checksum verified" } else { " · no checksum published" });
    Ok((snapshot(&state), msg))
}

#[tauri::command]
fn tools_remove(state: State<AppState>, tool: String, target: String, name: String) -> CmdResult<Snapshot> {
    let t = tools::find(&tool).ok_or_else(|| format!("unknown tool {tool}"))?;
    {
        let mut b = state.beacon.lock().unwrap();
        tools::remove(&b.paths.home, &b.paths.data_dir, &target, t.kind, &name).map_err(|e| format!("{e:#}"))?;
        b.refresh_runners();
    }
    Ok(snapshot(&state))
}

#[tauri::command]
async fn tools_update(app: AppHandle) -> CmdResult<(Snapshot, String)> {
    let (home, data) = {
        let b = app.state::<AppState>().beacon.lock().unwrap();
        (b.paths.home.clone(), b.paths.data_dir.clone())
    };
    let app2 = app.clone();
    let done = tauri::async_runtime::spawn_blocking(move || {
        tools::update_latest(&workshop::Net::new(), &home, &data, &mut |pct: Option<u8>, text: &str| {
            let _ = app2.emit("tools-progress", ToolsProgress { tool: String::new(), pct, text: text.to_string() });
        })
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let state = app.state::<AppState>();
    state.beacon.lock().unwrap().refresh_runners();
    let msg = if done.is_empty() { "Everything is up to date".to_string() } else { format!("Updated {}", done.iter().map(|d| d.tag.as_str()).collect::<Vec<_>>().join(", ")) };
    Ok((snapshot(&state), msg))
}

#[tauri::command]
fn set_tools_auto(state: State<AppState>, on: bool) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        b.config.tools_auto_update = on;
        Ok(())
    })
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog().file().blocking_pick_folder().and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn pick_file(app: AppHandle, kind: String) -> Option<String> {
    let mut d = app.dialog().file();
    d = match kind.as_str() {
        "mod" => d.add_filter("Mod (.big or launcher .exe)", &["big", "BIG", "exe", "EXE"]),
        "download" => d.add_filter("Download (archive, .exe or .dll)", &["zip", "ZIP", "7z", "rar", "RAR", "tar", "gz", "xz", "exe", "EXE", "dll", "DLL"]),
        "copy" => d.add_filter("Your copy (installer .exe or disc image .iso)", &["exe", "EXE", "msi", "MSI", "iso", "ISO"]),
        "launcher" => d.add_filter("The launcher you downloaded (.exe, .zip, .7z or .rar)", &["exe", "EXE", "zip", "ZIP", "7z", "7Z", "rar", "RAR"]),
        "game" => d.add_filter("Windows game or installer (.exe, .msi)", &["exe", "EXE", "msi", "MSI"]),
        "image" => d.add_filter("Picture (.png, .jpg, .webp) or icon (.ico, .exe)", &["png", "PNG", "jpg", "JPG", "jpeg", "webp", "ico", "ICO", "exe", "EXE"]),
        "json" => d.add_filter("Beacon export or Faugus Launcher games.json", &["json", "JSON"]),
        _ => d.add_filter("Windows program", &["exe", "EXE", "msi", "MSI"]),
    };
    d.blocking_pick_file().and_then(|p| p.into_path().ok()).map(|p| p.to_string_lossy().into_owned())
}

/// Open a folder or file in the desktop's default app.
#[tauri::command]
fn open_path(path: String) -> CmdResult<()> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        std::fs::create_dir_all(&p).map_err(err)?;
    }
    system_open(p.as_os_str()).map_err(err)
}

#[tauri::command]
fn open_log(state: State<AppState>, game: GameId) -> CmdResult<()> {
    let log = state.beacon.lock().unwrap().paths.log_file(game);
    if !log.exists() {
        return Err("No log yet — launch the game first.".into());
    }
    open_path(log.to_string_lossy().into_owned())
}

/// The `beacon` command-line tool through the app's own binary, for the AppImage (which has a single entry
/// point): `Beacon.AppImage --cli <args>`, or the AppImage started through a symlink named `beacon` (the
/// AppImage runtime passes the name it was started as in ARGV0). Runs the `beacon` binary installed next to
/// this one and doesn't return. The .deb and .rpm have /usr/bin/beacon and don't need this, but
/// `beacon-app --cli …` works there too.
#[cfg(unix)]
fn run_cli_if_asked() {
    use std::ffi::{OsStr, OsString};
    use std::os::unix::process::CommandExt;
    use std::path::Path;

    let args: Vec<OsString> = std::env::args_os().collect();
    let Ok(exe) = std::env::current_exe() else { return };
    let named_beacon = |s: &OsStr| Path::new(s).file_name() == Some(OsStr::new("beacon"));
    // ARGV0 is inherited by child processes, so it only counts inside our own AppImage.
    let in_our_appimage = std::env::var_os("APPDIR").is_some_and(|d| exe.starts_with(d));
    let rest: &[OsString] = if args.get(1).is_some_and(|a| a == "--cli") {
        args.get(2..).unwrap_or(&[])
    } else if (in_our_appimage && std::env::var_os("ARGV0").is_some_and(|a| named_beacon(a.as_os_str())))
        || args.first().is_some_and(|a| named_beacon(a.as_os_str()))
    {
        args.get(1..).unwrap_or(&[])
    } else {
        return;
    };
    let Some(cli) = exe.parent().map(|d| d.join("beacon")) else { return };
    let err = std::process::Command::new(&cli).args(rest).exec();
    eprintln!("beacon: couldn't start {}: {err}", cli.display());
    std::process::exit(127);
}

#[derive(Clone, Serialize)]
struct GamesFound {
    games: Vec<GameId>,
}

/// Point games that aren't set up yet at what a scan found (on a copy, so the UI never waits on the
/// disk), save, and tell the UI with `games-found`.
fn adopt_and_announce(app: &AppHandle, found: &[beacon_core::games::FoundGame]) {
    if found.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let set = {
        let mut b = state.beacon.lock().unwrap();
        let set = b.adopt_found(found);
        if !set.is_empty() {
            let _ = b.save();
        }
        set
    };
    if !set.is_empty() {
        let _ = app.emit("games-found", GamesFound { games: set.iter().map(|f| f.game).collect() });
    }
}

/// Finding games without pressing "Find games": a full scan in the background at start-up, then a
/// quick look every couple of minutes (and whenever the window comes back to the front, see
/// `look_for_games`) while any game is still missing, so a game shows up as soon as it's installed:
/// by its own installer, a launcher, or copied in.
fn auto_detect(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let mut work = app.state::<AppState>().beacon.lock().unwrap().clone();
        if !work.missing_games().is_empty() {
            work.refresh_runners();
            let found = work.scan(&[]);
            adopt_and_announce(&app, &found);
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(120));
            let work = app.state::<AppState>().beacon.lock().unwrap().clone();
            let found = work.quick_find();
            adopt_and_announce(&app, &found);
        }
    });
}

/// A quick look for games that aren't found yet (the UI calls this when the window gets focus).
#[tauri::command]
async fn look_for_games(app: AppHandle) -> CmdResult<Vec<GameId>> {
    let work = app.state::<AppState>().beacon.lock().unwrap().clone();
    let found = tauri::async_runtime::spawn_blocking(move || work.quick_find()).await.map_err(err)?;
    let state = app.state::<AppState>();
    let mut b = state.beacon.lock().unwrap();
    let set = b.adopt_found(&found);
    if !set.is_empty() {
        b.save().map_err(err)?;
    }
    Ok(set.into_iter().map(|f| f.game).collect())
}

/// What uninstalling a game would remove.
#[tauri::command]
async fn uninstall_plan(app: AppHandle, game: GameId) -> CmdResult<beacon_core::uninstall::Plan> {
    let b = app.state::<AppState>().beacon.lock().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || b.uninstall_plan(game)).await.map_err(err)
}

/// Uninstall a game (not while it's running). Answers the new snapshot and a message.
#[tauri::command]
async fn uninstall(app: AppHandle, game: GameId, delete_files: bool, remove_prefix: bool) -> CmdResult<(Snapshot, String)> {
    if app.state::<AppState>().running.lock().unwrap().contains(&game) {
        return Err(format!("{} is running; close it first", game.short()));
    }
    let mut work = app.state::<AppState>().beacon.lock().unwrap().clone();
    let done = tauri::async_runtime::spawn_blocking(move || work.uninstall(game, delete_files, remove_prefix).map(|d| (d, work)))
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))?;
    let (done, work) = done;
    let state = app.state::<AppState>();
    {
        let mut b = state.beacon.lock().unwrap();
        b.game_mut(game).path = work.game(game).path;
        b.save().map_err(err)?;
    }
    let freed = done.freed as f64 / 1e9;
    let msg = match (&done.deleted_dir, &done.deleted_prefix) {
        (None, None) => format!("{} removed from Beacon (its files are still there)", game.short()),
        _ => format!("{} uninstalled · {freed:.1} GB freed", game.short()),
    };
    Ok((snapshot(&state), msg))
}

/// Whether an Nvidia GPU is in the system: any `<drm>/*/device/vendor` that says 0x10de.
fn has_nvidia(drm: &std::path::Path) -> bool {
    std::fs::read_dir(drm)
        .map(|rd| rd.flatten().any(|e| std::fs::read_to_string(e.path().join("device/vendor")).map(|v| v.trim().eq_ignore_ascii_case("0x10de")).unwrap_or(false)))
        .unwrap_or(false)
}

/// WebKitGTK's DMA-BUF renderer crashes Beacon's window at start on Nvidia's driver (615 at least): on an Nvidia
/// system, use WebKit's older drawing path, unless WEBKIT_DISABLE_DMABUF_RENDERER is already set either way.
/// Runs before any thread is started.
#[cfg(target_os = "linux")]
fn nvidia_webkit_fix() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() && has_nvidia(std::path::Path::new("/sys/class/drm")) {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

fn main() {
    #[cfg(unix)]
    run_cli_if_asked();
    #[cfg(target_os = "linux")]
    nvidia_webkit_fix();
    // Remove what a self-update left behind.
    beacon_core::downloads::selfupdate::cleanup_old();

    let mut beacon = Beacon::load().expect("loading Beacon config");
    // First run: look for games straight away (later starts look in the background, see `auto_detect`).
    if beacon.config.games.is_empty() {
        beacon.scan(&[]);
        let _ = beacon.save();
    }
    let fullscreen = std::env::args().any(|a| a == "--fullscreen") || std::env::var("BEACON_FULLSCREEN").map(|v| v == "1").unwrap_or(false);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { beacon: Mutex::new(beacon), running: Mutex::new(BTreeSet::new()), running_custom: Mutex::new(BTreeSet::new()) })
        .manage(private::Downloads::default())
        .setup(move |app| {
            if fullscreen {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_fullscreen(true);
                }
            }
            private::start_background_check(app.handle());
            auto_detect(app.handle());
            // "Latest" compatibility tools brought up to date in the background, when that's on.
            let handle = app.handle().clone();
            if handle.state::<AppState>().beacon.lock().unwrap().config.tools_auto_update {
                std::thread::spawn(move || {
                    let (home, data) = {
                        let b = handle.state::<AppState>().beacon.lock().unwrap();
                        (b.paths.home.clone(), b.paths.data_dir.clone())
                    };
                    if let Ok(done) = tools::update_latest(&workshop::Net::new(), &home, &data, &mut |_: Option<u8>, _: &str| {}) {
                        if !done.is_empty() {
                            handle.state::<AppState>().beacon.lock().unwrap().refresh_runners();
                            let _ = handle.emit("tools-updated", done.len());
                        }
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            scan,
            set_game_path,
            set_cd_key,
            set_runner,
            set_display,
            set_launch_options,
            get_community,
            set_edition,
            add_files,
            list_mods,
            add_mod,
            get_catalog,
            find_mods,
            open_url,
            remove_mod,
            select_mod,
            launch,
            run_installer,
            install_from,
            workshop_list,
            workshop_install,
            aio_info,
            aio_launcher_install,
            aio_launcher_open,
            community_launcher_set,
            community_launcher_open,
            launch_preview,
            pick_folder,
            pick_file,
            open_path,
            open_log,
            private::downloads_info,
            private::cdn_get,
            private::sources_check,
            private::sources_update,
            private::set_updates_repo,
            private::set_games_dir,
            private::launcher_get,
            private::app_update_check,
            private::app_update_apply,
            private::mod_downloads,
            private::mod_lookup,
            private::mod_install,
            private::downloads_list,
            private::download_pause,
            private::download_forget,
            private::downloads_storage,
            private::downloads_clear,
            look_for_games,
            uninstall_plan,
            uninstall,
            custom_add,
            custom_set,
            custom_remove,
            custom_launch,
            custom_install,
            custom_run,
            custom_tool,
            custom_candidates,
            custom_preview,
            custom_art,
            custom_export,
            custom_import,
            tools_list,
            tools_releases,
            tools_install,
            tools_remove,
            tools_update,
            set_tools_auto,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Beacon");
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_an_nvidia_gpu() {
        let t = std::env::temp_dir().join(format!("beacon-drm-{}", std::process::id()));
        let card = |n: &str, vendor: &str| {
            std::fs::create_dir_all(t.join(n).join("device")).unwrap();
            std::fs::write(t.join(n).join("device/vendor"), vendor).unwrap();
        };
        card("card0", "0x1002\n");
        assert!(!super::has_nvidia(&t));
        card("card1", "0x10de\n");
        assert!(super::has_nvidia(&t));
        assert!(!super::has_nvidia(&t.join("missing")));
        let _ = std::fs::remove_dir_all(&t);
    }
}
