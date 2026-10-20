//! Beacon desktop app: a thin Tauri shell over beacon-core.

use beacon_core::{
    games::GameId,
    mods::ModEntry,
    runner::Runner,
    Beacon, CatalogItem, Community, GameStatus, Resolution,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

struct AppState {
    beacon: Mutex<Beacon>,
    running: Mutex<BTreeSet<GameId>>,
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
    running: Vec<GameId>,
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
        running: state.running.lock().unwrap().iter().copied().collect(),
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
    snapshot(&state)
}

#[tauri::command]
fn scan(state: State<AppState>) -> CmdResult<Snapshot> {
    mutate(&state, |b| {
        let path_env = std::env::var("PATH").unwrap_or_default();
        b.runners = beacon_core::runner::discover(&b.paths.home, &path_env);
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

/// Open a mod's web page in the default browser.
#[tauri::command]
fn open_url(url: String) -> CmdResult<()> {
    if !url.starts_with("https://") {
        return Err("only https links can be opened".into());
    }
    std::process::Command::new("xdg-open").arg(&url).spawn().map_err(err)?;
    Ok(())
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

/// Start a game (or `installer`, an exe to run inside the game's prefix).
/// Progress arrives as `game-event`s; the call itself returns at once.
fn start(app: AppHandle, state: &AppState, game: GameId, mod_id: Option<String>, installer: Option<PathBuf>, step: Option<String>) -> CmdResult<()> {
    let beacon = state.beacon.lock().unwrap().clone();
    // Fail fast on anything we can check without running Wine.
    let cmd = match &installer {
        Some(exe) => beacon.run_in_prefix_command(game, exe),
        None => beacon.launch_command(game, mod_id.as_deref()),
    }
    .map_err(err)?;
    if !state.running.lock().unwrap().insert(game) {
        return Err(format!("{} is already running", game.short()));
    }

    std::thread::spawn(move || {
        emit(&app, game, "preparing", None);
        // An installer may be installing the game itself: its prefix can't get the game's registry keys yet.
        let prepared = if installer.is_some() { beacon.prepare_prefix(game) } else { beacon.setup(game, false) };
        let started = prepared
            .and_then(|_| beacon.apply_options(game))
            .and_then(|_| beacon.spawn_logged(game, &cmd));
        match started {
            Ok(mut child) => {
                emit(&app, game, "running", None);
                let window = app.get_webview_window("main");
                if installer.is_none() {
                    if let Some(w) = &window {
                        let _ = w.minimize();
                    }
                }
                let status = child.wait();
                if let Some(w) = &window {
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
                if installer.is_some() {
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
    start(app.clone(), &state, game, mod_id, None, None)
}

#[tauri::command]
fn run_installer(app: AppHandle, state: State<AppState>, game: GameId, path: String, step: Option<String>) -> CmdResult<()> {
    start(app.clone(), &state, game, None, Some(PathBuf::from(path)), step)
}

#[tauri::command]
fn launch_preview(state: State<AppState>, game: GameId, mod_id: Option<String>) -> CmdResult<String> {
    let b = state.beacon.lock().unwrap();
    Ok(b.launch_command(game, mod_id.as_deref()).map_err(err)?.display())
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
    std::process::Command::new("xdg-open").arg(&p).spawn().map_err(err)?;
    Ok(())
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

fn main() {
    #[cfg(unix)]
    run_cli_if_asked();

    let mut beacon = Beacon::load().expect("loading Beacon config");
    // First run: look for games straight away.
    if beacon.config.games.is_empty() {
        beacon.scan(&[]);
        let _ = beacon.save();
    }
    let fullscreen = std::env::args().any(|a| a == "--fullscreen") || std::env::var("BEACON_FULLSCREEN").map(|v| v == "1").unwrap_or(false);

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { beacon: Mutex::new(beacon), running: Mutex::new(BTreeSet::new()) })
        .setup(move |app| {
            if fullscreen {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_fullscreen(true);
                }
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
            launch_preview,
            pick_folder,
            pick_file,
            open_path,
            open_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Beacon");
}
