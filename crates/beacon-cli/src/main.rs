//! `beacon` – headless launcher for BFME, BFME II and RotWK (Linux and Windows).
//! Handy over SSH, in scripts, and for Hearth to call into.
//!
//! Every command takes `--json` for machine-readable output (see `beacon_core::report` and the README's
//! "For Hearth / scripting"). Without it, the output is for people, as before.

use anyhow::Result;
use beacon_core::{aio, community_launcher, editions, games::GameId, media, mods::ModKind, report, workshop, Beacon, Level, Resolution};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// Private build: downloads.
mod private;

#[derive(Parser)]
#[command(name = "beacon", version, about = "Launcher for BFME, AotR, LOTR: Conquest and other classic games (Linux and Windows)")]
struct Cli {
    /// Machine-readable output: JSON, with progress lines for long jobs.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show each game's state (default).
    Status,
    /// Look for installed games and fill in missing folders.
    Scan { roots: Vec<PathBuf> },
    /// Point a game at its folder (or its exe).
    SetPath { game: String, path: PathBuf },
    /// Store the CD key from your own copy, read from stdin (never from the command line).
    SetKey {
        game: String,
        /// Remove the saved key instead.
        #[arg(long)]
        clear: bool,
    },
    /// List Wine/Proton/umu runners.
    Runners,
    /// Pick a runner for one game, or `default` for all; `auto` goes back to automatic.
    SetRunner { game: String, runner: String },
    /// Set resolution, e.g. 1920x1080, or `clear`.
    Resolution { game: String, value: String },
    /// Windowed mode on/off.
    Windowed { game: String, value: String },
    /// Community versions for a game: editions and setup steps.
    Community { game: String },
    /// Pick the edition to launch (`auto` = community edition if installed).
    Edition { game: String, id: String },
    /// Add a community download (archive, folder or file) for a setup step.
    AddFiles { game: String, step: String, path: PathBuf },
    /// Run a non-BFME game inside a Wine virtual desktop: on, off or default.
    VirtualDesktop { game: String, value: String },
    /// For Steam copies (UaW, SWGB): launch through Steam on/off.
    ViaSteam { game: String, value: String },
    /// Extra game arguments (quoted string; empty clears). Put `--` before arguments that start with a dash.
    Args {
        game: String,
        #[arg(allow_hyphen_values = true)]
        args: String,
    },
    /// List mods for a game.
    Mods { game: String },
    /// Link a mod from anywhere (.big, folder of .big files, or launcher .exe).
    AddMod {
        game: String,
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
        /// Also make it the mod the game starts with.
        #[arg(long)]
        select: bool,
    },
    /// Known mods for a game, with links and what's installed.
    Catalog { game: String },
    /// Search this PC for known mods and add them.
    FindMods { roots: Vec<PathBuf> },
    /// Remove a linked mod by id.
    RemoveMod { id: String },
    /// Choose the mod to launch by default (`none` for vanilla).
    SelectMod { game: String, id: String },
    /// Create the game's prefix and import its registry keys.
    Setup {
        game: String,
        #[arg(long)]
        force: bool,
    },
    /// Start a game.
    Launch {
        game: String,
        /// Use this mod instead of the saved choice.
        #[arg(long = "mod", conflicts_with = "vanilla")]
        mod_id: Option<String>,
        /// Ignore the saved mod and run the base game.
        #[arg(long)]
        vanilla: bool,
        /// Print the command instead of running it.
        #[arg(long)]
        dry_run: bool,
        /// Wait for the game to exit.
        #[arg(long)]
        wait: bool,
    },
    /// Run a Windows exe (patch, mod installer, a game's own installer) in a game's prefix.
    Run {
        game: String,
        #[arg(required_unless_present = "tool", conflicts_with = "tool")]
        exe: Option<PathBuf>,
        /// Run the game's own extra launcher instead (AotR's, for updates).
        #[arg(long)]
        tool: bool,
    },
    /// Show the end of a game's log.
    Log {
        game: String,
        /// How many lines, from the end.
        #[arg(long, default_value_t = 200)]
        lines: usize,
    },
    /// Uninstall a game: shows what would be removed; --yes does it. Saves and settings in its Wine prefix stay
    /// unless --remove-prefix. Without --delete-files the game only leaves Beacon (its files stay).
    Uninstall {
        game: String,
        /// Delete the game's folder (when Beacon can tell it's the game's alone).
        #[arg(long)]
        delete_files: bool,
        /// Also delete its Wine prefix: saves, settings and anything installed in it.
        #[arg(long)]
        remove_prefix: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Show where config, prefixes, mods and logs live.
    Paths,
    /// Install a game from your own copy: its installer (.exe), a disc image (.iso) or the disc's folder.
    InstallFrom { game: String, path: PathBuf },
    /// Fan-made mods, maps and patches from the BFME workshop (BFME, BFME II, RotWK).
    Workshop {
        game: String,
        /// Only entries matching these words.
        #[arg(long, default_value = "")]
        search: String,
        /// Page of results, from 0.
        #[arg(long, default_value_t = 0)]
        page: u32,
    },
    /// Download a workshop entry (fan content only) into Beacon's mods.
    WorkshopInstall {
        game: String,
        id: String,
        /// Also make it the mod the game starts with.
        #[arg(long)]
        select: bool,
    },
    /// Games the BFME All-in-One launcher installed, and where to get the launcher.
    Aio,
    /// The BFME All-in-One launcher itself: status, install (from the BFME Foundation's official download), open.
    AioLauncher {
        #[command(subcommand)]
        action: LauncherAction,
    },
    /// A game's community launcher (Universe at War): status, set (a launcher you downloaded), open.
    CommunityLauncher {
        game: String,
        #[command(subcommand)]
        action: CommunityAction,
    },
    /// Private build: download BFME, BFME II or RotWK from the BFME Foundation's CDN and set it up
    /// (RotWK brings BFME II along when it's missing), or UaW's newest community launcher from its
    /// Google Drive folder. Resumes where it stopped.
    Get {
        game: String,
        /// Download it again over an existing copy, keeping files that are already fine.
        #[arg(long)]
        repair: bool,
        /// Like --repair, but compare every file's MD5 (slow, thorough).
        #[arg(long)]
        verify: bool,
        /// Use a source Beacon only guessed at after its link changed.
        #[arg(long)]
        yes: bool,
    },
    /// Private build: download a mod or patch inside Beacon (a catalog id with a Download, or any ModDB link) and
    /// put it where it belongs for the game: Beacon's mods, the game's folder, or its installer run in the prefix.
    ModGet {
        game: String,
        /// A catalog id (e.g. edain) or a ModDB link.
        what: String,
        /// Download without showing what was found first.
        #[arg(long)]
        yes: bool,
    },
    /// Private build: update Beacon itself from the private repo's newest release.
    SelfUpdate {
        /// Only say whether there's a newer version.
        #[arg(long)]
        check: bool,
    },
    /// Private build: the download sources (list, check, update, token, games-dir).
    Sources {
        #[command(subcommand)]
        action: Option<private::SourcesAction>,
    },
}

#[derive(Subcommand)]
enum CommunityAction {
    /// Which launcher is chosen, where it's downloaded from.
    Status,
    /// Use a launcher you downloaded: its .exe (copied) or the .zip, .7z or .rar it came in (extracted). Beacon never downloads it.
    Set { path: PathBuf },
    /// Start it (Linux: in the game's own Wine prefix). Download the game there; `status` and `scan` find it.
    Open,
}

#[derive(Subcommand)]
enum LauncherAction {
    /// Is it installed, and where.
    Status,
    /// Download the launcher from the BFME Foundation and install it (Linux: in its own Wine prefix). Never the games.
    Install,
    /// Start it (Linux: in its prefix, through the runner). Download the games there; `status` and `scan` find them.
    Open,
    /// Install a game through it: install the launcher if needed, open it, wait while you install the game there,
    /// then close it (Linux) and add the game. The launcher downloads the game, never Beacon.
    Get {
        /// bfme1, bfme2 or rotwk.
        game: String,
    },
}

// ---------------------------------------------------------------- errors and output

/// Bad arguments (exit code 2): an unknown game, mod, runner or step, or a malformed value.
#[derive(Debug)]
struct Usage(String);

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Usage {}

fn usage(msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Usage(msg.into()))
}

/// How a command's output goes out: for people, or as JSON.
struct Ctx {
    json: bool,
    /// The CD key for `set-key`, instead of stdin (tests).
    key_input: Option<String>,
}

impl Ctx {
    /// A line for people (nothing in JSON mode).
    fn say(&self, s: impl AsRef<str>) {
        if !self.json {
            println!("{}", s.as_ref());
        }
    }

    /// A progress line for long jobs (JSON mode only).
    fn progress(&self, pct: Option<u8>, text: &str) {
        if self.json {
            println!("{}", report::progress(pct, text));
            let _ = std::io::stdout().flush();
        }
    }

    fn read_key(&self, g: GameId) -> Result<String> {
        if let Some(k) = &self.key_input {
            return Ok(k.clone());
        }
        let stdin = std::io::stdin();
        if stdin.is_terminal() && !self.json {
            eprint!("CD key for {}: ", g.short());
        }
        let mut line = String::new();
        stdin.lock().read_line(&mut line)?;
        Ok(line)
    }
}

/// The exit code for a command's outcome.
fn exit_code(r: &Result<Value>) -> i32 {
    match r {
        Ok(v) if v.get("ok") == Some(&Value::Bool(false)) => report::EXIT_PROGRAM_FAILED,
        Ok(_) => report::EXIT_OK,
        Err(e) if e.downcast_ref::<Usage>().is_some() => report::EXIT_USAGE,
        Err(_) => report::EXIT_FAILED,
    }
}

/// The JSON line for a failed command.
fn error_json(e: &anyhow::Error) -> Value {
    report::error(if e.downcast_ref::<Usage>().is_some() { "usage" } else { "failed" }, &format!("{e:#}"))
}

fn game(s: &str) -> Result<GameId> {
    GameId::parse(s).ok_or_else(|| usage(format!("unknown game \"{s}\" (use bfme1, bfme2, rotwk, aotr, conquest, uaw, exigo, rol or swgb)")))
}

fn on_off(s: &str) -> Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => Err(usage("expected on or off")),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

/// The result of running a Windows program: ok when it exited cleanly, otherwise ok:false (exit code 3).
fn program_result(g: GameId, st: &std::process::ExitStatus, what: &str, extra: Value) -> Value {
    let code = st.code();
    let mut extra = extra;
    if let Some(o) = extra.as_object_mut() {
        o.insert("exit_code".into(), json!(code));
    }
    if st.success() {
        report::ok(Some(g), format!("{what} finished"), extra)
    } else {
        let how = code.map(|c| format!("code {c}")).unwrap_or_else(|| "it was stopped".into());
        report::program_failed(g, format!("{what} closed with an error ({how}); see the log"), extra)
    }
}

fn print_status(b: &Beacon) {
    for st in b.statuses() {
        let mark = if st.ready { "ready" } else if st.installed { "needs attention" } else { "not found" };
        println!("{} — {}  [{}]", st.short, st.title, mark);
        if let Some(p) = &st.path {
            println!("    folder:  {}", p.display());
        }
        if let Some(r) = &st.runner {
            println!("    runner:  {} ({})", r.name, r.id);
        }
        if let Some(r) = st.resolution {
            println!("    res:     {r}{}", if st.windowed { " windowed" } else { "" });
        }
        if let Some(m) = &st.selected_mod {
            println!("    mod:     {m}");
        }
        for i in &st.issues {
            println!("    {}: {}", if i.level == Level::Error { "error" } else { "note" }, i.message);
        }
    }
}

fn main() {
    let json_wanted = std::env::args_os().any(|a| a.to_str() == Some("--json"));
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            let shows_text = matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand);
            if json_wanted && !shows_text {
                let text = e.to_string();
                let first = text.lines().next().unwrap_or("bad arguments").trim_start_matches("error: ").to_string();
                println!("{}", report::error("usage", &first));
                std::process::exit(report::EXIT_USAGE);
            }
            e.exit();
        }
    };
    let ctx = Ctx { json: cli.json, key_input: None };
    let cmd = cli.cmd.unwrap_or(Cmd::Status);
    let pretty = matches!(cmd, Cmd::Status);
    let result = Beacon::load().and_then(|mut b| run(&ctx, &mut b, cmd));
    match &result {
        Ok(v) if ctx.json => {
            let text = if pretty { serde_json::to_string_pretty(v) } else { serde_json::to_string(v) };
            println!("{}", text.unwrap_or_else(|_| "null".into()));
        }
        Ok(_) => {}
        Err(e) if ctx.json => println!("{}", error_json(e)),
        Err(e) => eprintln!("Error: {e:#}"),
    }
    let _ = std::io::stdout().flush();
    std::process::exit(exit_code(&result));
}

// ---------------------------------------------------------------- the commands

/// Runs one command. Returns the JSON result (printed by `main` in JSON mode); people's output is printed here.
fn run(ctx: &Ctx, b: &mut Beacon, cmd: Cmd) -> Result<Value> {
    Ok(match cmd {
        Cmd::Status => {
            // Games the All-in-One launcher (or a game's community launcher) downloaded since the last look.
            let mut adopted = !b.adopt_launcher_games().is_empty();
            for g in GameId::ALL {
                if b.game(g).community_launcher.is_some() && !b.adopt_community_games(g).is_empty() {
                    adopted = true;
                }
            }
            if adopted {
                b.save()?;
            }
            if !ctx.json {
                print_status(b);
            }
            report::status(b)
        }
        Cmd::Scan { roots } => {
            let found = b.scan_with(&roots, &mut |i: usize, n: usize, root: &Path| {
                ctx.progress(Some(report::pct(i, n, 0, 95)), &format!("Looking in {}", root.display()));
            });
            b.save()?;
            if found.is_empty() {
                ctx.say("No games found. Use `beacon set-path <game> <folder>` or `beacon scan <folder>`.");
            }
            for f in &found {
                ctx.say(format!("found {} in {}", f.game.short(), f.dir.display()));
            }
            if !ctx.json {
                println!();
                print_status(b);
            }
            let installed: Vec<&str> = b.statuses().iter().filter(|s| s.installed).map(|s| s.id.slug()).collect();
            let msg = match found.len() {
                0 => "No games found".to_string(),
                1 => "Found 1 game".to_string(),
                n => format!("Found {n} games"),
            };
            report::ok(None, msg, json!({"found": report::found(&found), "installed": installed}))
        }
        Cmd::SetPath { game: g, path } => {
            let g = game(&g)?;
            b.set_path(g, &path)?;
            b.save()?;
            let dir = b.game(g).path.unwrap_or_default();
            ctx.say(format!("{} → {}", g.short(), dir.display()));
            report::ok(Some(g), format!("{} folder set", g.short()), json!({"path": report::path(&dir)}))
        }
        Cmd::SetKey { game: g, clear } => {
            let g = game(&g)?;
            let key = if clear { String::new() } else { ctx.read_key(g)? };
            let key = key.trim().to_string();
            if !clear && key.is_empty() {
                return Err(usage("no CD key on stdin (use --clear to remove the saved one)"));
            }
            if key.len() > 64 || key.chars().any(|c| c.is_control()) {
                return Err(usage("that doesn't look like a CD key"));
            }
            b.game_mut(g).cd_key = (!key.is_empty()).then_some(key);
            b.save()?;
            let has = b.status(g).has_cd_key;
            ctx.say(if has { "Saved. It's applied on the next launch." } else { "CD key removed." });
            report::ok(Some(g), if has { "CD key saved" } else { "CD key removed" }, json!({"has_cd_key": has}))
        }
        Cmd::Runners => {
            if !ctx.json {
                if b.runners.runners.is_empty() {
                    println!("No runners found. Install GE-Proton (e.g. with ProtonUp-Qt), umu-launcher, or wine.");
                }
                let best = b.runners.best().map(|r| r.id.clone());
                for r in &b.runners.runners {
                    let star = if Some(&r.id) == best.as_ref() { "*" } else { " " };
                    println!("{star} {:<32} {}", r.id, r.path.display());
                }
            }
            report::runners(b)
        }
        Cmd::SetRunner { game: g, runner } => {
            let choice = if runner == "auto" {
                None
            } else if b.runners.get(&runner).is_some() {
                Some(runner)
            } else {
                return Err(usage(format!("no runner {runner}; see `beacon runners`")));
            };
            let target = if g == "default" { None } else { Some(game(&g)?) };
            match target {
                None => b.config.default_runner = choice.clone(),
                Some(gg) => b.game_mut(gg).runner = choice.clone(),
            }
            b.save()?;
            let name = choice.as_deref().and_then(|id| b.runners.get(id)).map(|r| r.name.clone()).unwrap_or_else(|| "Automatic".into());
            report::ok(target, format!("Runner: {name}"), json!({"runner": choice, "default": target.is_none()}))
        }
        Cmd::Resolution { game: g, value } => {
            let g = game(&g)?;
            let res = if value == "clear" {
                None
            } else {
                Some(Resolution::parse(&value).ok_or_else(|| usage("use WIDTHxHEIGHT, e.g. 1920x1080, or clear"))?)
            };
            b.game_mut(g).resolution = res;
            b.save()?;
            let msg = match res {
                Some(r) => format!("Resolution: {r}"),
                None => "Resolution: the game's own setting".to_string(),
            };
            report::ok(Some(g), msg, json!({"resolution": res.map(|r| r.to_string())}))
        }
        Cmd::Windowed { game: g, value } => {
            let g = game(&g)?;
            let on = on_off(&value)?;
            b.game_mut(g).windowed = on;
            b.save()?;
            report::ok(Some(g), if on { "Windowed on" } else { "Windowed off" }, json!({"windowed": on}))
        }
        Cmd::Community { game: g } => {
            let g = game(&g)?;
            if !ctx.json {
                let c = b.community(g);
                if c.editions.is_empty() && c.steps.is_empty() {
                    println!("{} has no community version setup.", g.short());
                }
                for e in &c.editions {
                    let mark = if e.active { "*" } else if e.installed { "+" } else { " " };
                    println!("{mark} {:<20} {}{}", e.edition.id, e.edition.name, if e.edition.community { " (community)" } else { "" });
                }
                if !c.editions.is_empty() {
                    println!("  (* launches, + installed)\n");
                }
                for st in &c.steps {
                    println!("[{}] {:<18} {}\n    {}", if st.done { "x" } else { " " }, st.step.id, st.step.title, st.step.detail);
                    if let Some(u) = st.step.url {
                        println!("    {u}");
                    }
                }
            }
            report::community(b, g)
        }
        Cmd::Edition { game: g, id } => {
            let g = game(&g)?;
            if id != "auto" && !editions::editions(g).iter().any(|e| e.id == id) {
                return Err(usage(format!("{} has no edition {id}; see `beacon community {}`", g.short(), g.slug())));
            }
            b.set_edition(g, (id != "auto").then_some(id.as_str()))?;
            b.save()?;
            let st = b.status(g);
            let msg = format!("Version: {}", st.edition.as_deref().unwrap_or("automatic"));
            report::ok(Some(g), msg, json!({"chosen": b.game(g).edition, "edition": st.edition}))
        }
        Cmd::AddFiles { game: g, step, path } => {
            let g = game(&g)?;
            let step_def = editions::steps(g)
                .into_iter()
                .find(|s| s.id == step)
                .ok_or_else(|| usage(format!("no step {step}; see `beacon community {}`", g.slug())))?;
            match step_def.action {
                editions::Action::Link => return Err(usage(format!("\"{}\" is only a link: open its page", step_def.title))),
                editions::Action::Installer => {
                    ctx.progress(Some(5), "Preparing the game's Wine prefix");
                    b.prepare_prefix(g)?;
                    let cmd = b.run_in_prefix_command(g, &path)?;
                    ctx.progress(Some(15), &format!("Running {} — follow its window", file_name(&path)));
                    let st = b.spawn_logged(g, &cmd)?.wait()?;
                    if st.success() {
                        b.mark_step(g, &step);
                    }
                    b.save()?;
                    ctx.say(format!("Installer exited: {st}"));
                    program_result(g, &st, &step_def.title.to_string(), json!({"step": step, "done": st.success()}))
                }
                _ => {
                    let msg = b.add_files_with(g, &step, &path, &mut |t: &str| ctx.progress(None, t))?;
                    b.save()?;
                    ctx.say(&msg);
                    report::ok(Some(g), msg, json!({"step": step, "done": true, "edition": b.status(g).edition}))
                }
            }
        }
        Cmd::VirtualDesktop { game: g, value } => {
            let g = game(&g)?;
            if g.is_sage() {
                return Err(usage(format!("{} doesn't use a virtual desktop", g.short())));
            }
            let choice = if value == "default" { None } else { Some(on_off(&value)?) };
            b.game_mut(g).virtual_desktop = choice;
            b.save()?;
            let on = b.virtual_desktop(g);
            report::ok(Some(g), if on { "Virtual desktop on" } else { "Virtual desktop off" }, json!({"virtual_desktop": on, "choice": choice}))
        }
        Cmd::ViaSteam { game: g, value } => {
            let g = game(&g)?;
            if g.steam_appid().is_none() {
                return Err(usage(format!("{} has no Steam version Beacon knows about", g.short())));
            }
            let on = on_off(&value)?;
            b.game_mut(g).via_steam = on;
            b.save()?;
            let msg = if on { "Launching through Steam" } else { "Launching with Beacon" };
            report::ok(Some(g), msg, json!({"via_steam": b.via_steam(g), "steam_copy": b.steam_appid(g).is_some()}))
        }
        Cmd::Args { game: g, args } => {
            let g = game(&g)?;
            if args.chars().any(|c| c.is_control()) {
                return Err(usage("arguments can't contain control characters"));
            }
            b.game_mut(g).extra_args = args.clone();
            b.save()?;
            report::ok(Some(g), if args.trim().is_empty() { "No extra arguments" } else { "Extra arguments saved" }, json!({"extra_args": args}))
        }
        Cmd::Mods { game: g } => {
            let g = game(&g)?;
            if !ctx.json {
                let sel = b.status(g).selected_mod;
                let mods = b.mods(g);
                if mods.is_empty() {
                    println!("No mods. Drop them in {} or use `beacon add-mod`.", b.paths.mods_dir().join(g.slug()).display());
                }
                for m in mods {
                    let star = if Some(&m.id) == sel.as_ref() { "*" } else { " " };
                    let kind = if m.kind == ModKind::Launcher { "launcher" } else { "mod" };
                    println!("{star} {:<30} {:<9} {}", m.id, kind, m.path.display());
                }
            }
            report::mods(b, g)
        }
        Cmd::AddMod { game: g, path, name, select } => {
            let g = game(&g)?;
            let m = b.link_mod(g, &path, name.as_deref())?;
            if select {
                b.select_mod(g, Some(&m.id))?;
            }
            b.save()?;
            ctx.say(format!("added {} ({})", m.name, m.id));
            report::ok(Some(g), format!("Added {}", m.name), json!({"mod": report::value(&m), "selected": select}))
        }
        Cmd::Catalog { game: g } => {
            let g = game(&g)?;
            if !ctx.json {
                for item in b.catalog(g) {
                    let mark = match &item.installed {
                        Some(id) => format!("installed as {id}"),
                        None => "not installed".into(),
                    };
                    println!("{} [{}] — {mark}\n    {}\n    {}", item.entry.name, if item.entry.kind == beacon_core::catalog::EntryKind::Patch { "patch" } else { "mod" }, item.entry.blurb, item.entry.url);
                }
            }
            report::catalog(b, g)
        }
        Cmd::FindMods { roots } => {
            let added = b.find_mods_with(&roots, &mut |i: usize, n: usize, root: &Path| {
                ctx.progress(Some(report::pct(i, n, 0, 95)), &format!("Looking in {}", root.display()));
            });
            b.save()?;
            if added.is_empty() {
                ctx.say("No new mods found.");
            }
            for m in &added {
                ctx.say(format!("added {} for {} ({})", m.name, m.game.short(), m.path.display()));
            }
            let msg = match added.len() {
                0 => "No new mods found".to_string(),
                1 => "Found 1 mod".to_string(),
                n => format!("Found {n} mods"),
            };
            report::ok(None, msg, json!({"added": report::value(&added)}))
        }
        Cmd::RemoveMod { id } => {
            if !b.unlink_mod(&id) {
                return Err(usage(format!("no linked mod {id} (mods in the mods folder are removed by deleting them)")));
            }
            b.save()?;
            report::ok(GameId::parse(id.split(':').next().unwrap_or("")), "Mod removed", json!({"removed": id}))
        }
        Cmd::SelectMod { game: g, id } => {
            let g = game(&g)?;
            let choice = (id != "none").then_some(id.as_str());
            if let Some(m) = choice {
                if b.find_mod(g, m).is_none() {
                    return Err(usage(format!("no mod {m} for {}; see `beacon mods {}`", g.short(), g.slug())));
                }
            }
            b.select_mod(g, choice)?;
            b.save()?;
            report::ok(Some(g), if choice.is_some() { "Mod chosen" } else { "Playing without mods" }, json!({"selected": choice}))
        }
        Cmd::Setup { game: g, force } => {
            let g = game(&g)?;
            ctx.say(format!("Preparing {} prefix (the first time can take a minute)…", g.short()));
            ctx.progress(Some(5), "Checking the game's setup");
            let needed = !b.via_steam(g) && (force || b.needs_setup(g)?);
            if needed {
                ctx.progress(Some(20), "Creating the Wine prefix and adding the game's registry keys (the first time can take a minute)");
            }
            let did = b.setup(g, force)?;
            if did {
                ctx.say("Done.");
            } else {
                ctx.say("Already set up. Use --force to redo it.");
            }
            ctx.progress(Some(90), "Writing the game's settings");
            let written = b.apply_options(g)?;
            if let Some(f) = &written {
                ctx.say(format!("Wrote {}", f.display()));
            }
            report::ok(Some(g), if did { "Ready to play" } else { "Already set up" }, json!({"did_work": did, "options_file": written.as_deref().map(report::path)}))
        }
        Cmd::Launch { game: g, mod_id, vanilla, dry_run, wait } => {
            let g = game(&g)?;
            let m = if vanilla { Some("") } else { mod_id.as_deref() };
            if dry_run {
                let cmd = b.launch_command(g, m)?;
                let setup_first = b.needs_setup(g).unwrap_or(false);
                if !ctx.json {
                    if b.needs_setup(g)? {
                        println!("# setup first:\n(cd {} && {})", b.paths.prefix_root(g).display(), b.setup_command(g)?.display());
                    }
                    println!("(cd {} && {})", cmd.cwd.display(), cmd.display());
                }
                return Ok(report::ok(Some(g), "What Beacon would run", json!({"command": report::command(&cmd), "setup_first": setup_first})));
            }
            let mut child = b.launch(g, m)?;
            let log = b.paths.log_file(g);
            ctx.say(format!("Started {} (pid {}). Log: {}", g.short(), child.id(), log.display()));
            let mut extra = json!({"pid": child.id(), "log": report::path(&log)});
            if wait {
                let st = child.wait()?;
                ctx.say(format!("Exited: {st}"));
                extra["exit_code"] = json!(st.code());
            }
            report::ok(Some(g), if wait { "The game closed" } else { "Started" }, extra)
        }
        Cmd::Run { game: g, exe, tool } => {
            let g = game(&g)?;
            let exe = if tool {
                b.status(g).tool_launcher.ok_or_else(|| usage(format!("{} has no launcher of its own", g.short())))?
            } else {
                exe.ok_or_else(|| usage("give the .exe to run"))?
            };
            ctx.progress(Some(5), "Preparing the game's Wine prefix");
            b.prepare_prefix(g)?;
            let cmd = b.run_in_prefix_command(g, &exe)?;
            ctx.progress(Some(15), &format!("Running {} — follow its window. Some installers work for a long time without showing progress.", file_name(&exe)));
            let st = b.spawn_logged(g, &cmd)?.wait()?;
            ctx.say(format!("Exited: {st}"));
            ctx.progress(Some(90), "Looking for newly installed games");
            // Pick up anything the installer just put in the prefix.
            let found = b.scan(&[]);
            for f in &found {
                ctx.say(format!("found {} in {}", f.game.short(), f.dir.display()));
            }
            b.save()?;
            let what = if tool { "The launcher".to_string() } else { file_name(&exe) };
            program_result(g, &st, &what, json!({"found": report::found(&found), "installed": b.status(g).installed}))
        }
        Cmd::Log { game: g, lines } => {
            let g = game(&g)?;
            let v = report::log(b, g, lines);
            if !ctx.json {
                if v["exists"] == true {
                    println!("== {}", b.paths.log_file(g).display());
                    for l in v["lines"].as_array().into_iter().flatten() {
                        println!("{}", l.as_str().unwrap_or(""));
                    }
                } else {
                    println!("No log yet for {} — launch or set up the game first.", g.short());
                }
            }
            v
        }
        Cmd::InstallFrom { game: g, path } => {
            let g = game(&g)?;
            media::classify(&path).map_err(|e| usage(format!("{e:#}")))?;
            ctx.progress(Some(2), "Checking what you picked");
            let prep = b.prepare_install(g, &path, &mut |t: &str| ctx.progress(None, t))?;
            ctx.progress(Some(10), if b.native { "Getting the installer ready" } else { "Preparing the game's Wine prefix" });
            b.prepare_prefix(g)?;
            let cmd = b.run_in_prefix_command(g, &prep.setup)?;
            ctx.say(format!("Running {} — follow its window.", prep.setup.display()));
            ctx.progress(Some(15), &format!("Running {} — follow its window. Some installers work for a long time without showing progress.", file_name(&prep.setup)));
            let st = b.spawn_logged(g, &cmd)?.wait()?;
            ctx.say(format!("Exited: {st}"));
            ctx.progress(Some(90), "Looking for the installed game");
            let found = b.scan(&[]);
            for f in &found {
                ctx.say(format!("found {} in {}", f.game.short(), f.dir.display()));
            }
            b.save()?;
            let installed = b.status(g).installed;
            if let Some(x) = &prep.extracted {
                ctx.say(format!("The disc image was extracted to {} (removed by the next install-from for {}).", x.display(), g.slug()));
            }
            program_result(
                g,
                &st,
                &file_name(&prep.setup),
                json!({
                    "setup": report::path(&prep.setup),
                    "extracted": prep.extracted.as_deref().map(report::path),
                    "found": report::found(&found),
                    "installed": installed,
                }),
            )
        }
        Cmd::Workshop { game: g, search, page } => {
            let g = game(&g)?;
            workshop::game_number(g).map_err(|e| usage(e.to_string()))?;
            let (items, hidden) = b.workshop_list(&workshop::Net::new(), g, &search, page)?;
            if !ctx.json {
                if items.is_empty() {
                    println!("Nothing found on this page of the {} workshop.", g.short());
                }
                for it in &items {
                    let mark = match &it.installed {
                        Some(id) => format!("installed as {id}"),
                        None => "not installed".into(),
                    };
                    let mb = it.size as f64 / 1_048_576.0;
                    println!("{} {} [{}] by {} — {mb:.0} MB, {mark}\n    id: {}", it.name, it.version, it.kind, it.author, it.id);
                }
                println!("\nFan content from {} (base games and EA's own patches aren't listed). Install with `beacon workshop-install {} <id>`.", workshop::SITE, g.slug());
            }
            report::workshop(g, page, &items, hidden)
        }
        Cmd::WorkshopInstall { game: g, id, select } => {
            let g = game(&g)?;
            workshop::game_number(g).map_err(|e| usage(e.to_string()))?;
            let (done, m) = b.workshop_install(&workshop::Net::new(), g, &id, &mut |p: Option<u8>, t: &str| ctx.progress(p, t))?;
            let selected = select && m.is_some();
            if let Some(m) = m.as_ref().filter(|_| select) {
                b.select_mod(g, Some(&m.id))?;
                b.save()?;
            }
            ctx.say(format!("Installed {} {} into {}", done.name, done.version, done.dir.display()));
            report::ok(
                Some(g),
                format!("Installed {}", done.name),
                json!({
                    "id": done.id,
                    "name": done.name,
                    "version": done.version,
                    "kind": done.kind,
                    "dir": report::path(&done.dir),
                    "files": done.files,
                    "bytes": done.bytes,
                    "skipped": done.skipped,
                    "mod": report::value(&m),
                    "selected": selected,
                }),
            )
        }
        Cmd::Aio => {
            if !b.adopt_launcher_games().is_empty() {
                b.save()?;
            }
            if !ctx.json {
                let games = b.aio_games();
                let launcher = b.launcher_status();
                match &launcher.path {
                    Some(p) => println!("All-in-One launcher: {}", p.display()),
                    None => println!("All-in-One launcher: not installed — `beacon aio-launcher install` gets it from the BFME Foundation ({}).", aio::LAUNCHER_URL),
                }
                if games.is_empty() {
                    println!("No games from the BFME All-in-One launcher found yet. Download them in the launcher (`beacon aio-launcher open`); Beacon finds them afterwards.");
                }
                for a in &games {
                    println!("{} — {}", a.game.short(), a.dir.display());
                    if let Some(p) = &a.active_patch {
                        println!("    patch: {p}");
                    }
                    if let Some(m) = &a.active_mod {
                        println!("    mod:   {m}");
                    }
                }
            }
            report::aio(b)
        }
        Cmd::AioLauncher { action } => match action {
            LauncherAction::Status => {
                let st = b.launcher_status();
                if !ctx.json {
                    match &st.path {
                        Some(p) => {
                            println!("Installed: {}", p.display());
                            if let Some(pfx) = &st.prefix {
                                println!("Prefix:    {}", pfx.display());
                            }
                        }
                        None if st.can_install => println!("Not installed. `beacon aio-launcher install` downloads it from the BFME Foundation ({}).", st.site),
                        None => println!("Not installed. It runs through Wine here: install GE-Proton, umu-launcher or Wine first."),
                    }
                }
                report::launcher(b)
            }
            LauncherAction::Install => {
                let done = b.install_launcher(&workshop::Net::new(), &mut |p: Option<u8>, t: &str| ctx.progress(p, t))?;
                ctx.say(format!("Installed the All-in-One launcher at {}{}", done.path.display(), if done.verified { "" } else { " (the published checksum couldn't be read, so it wasn't compared)" }));
                ctx.say("Open it with `beacon aio-launcher open` and download the games there; Beacon finds them afterwards.");
                report::ok(
                    None,
                    "All-in-One launcher installed",
                    json!({
                        "path": report::path(&done.path),
                        "prefix": done.prefix.as_deref().map(report::path),
                        "bytes": done.bytes,
                        "md5": done.md5,
                        "verified": done.verified,
                        "launcher": report::launcher(b),
                    }),
                )
            }
            LauncherAction::Open => {
                let child = b.open_launcher()?;
                let log = b.launcher_log();
                ctx.say(format!("Started the All-in-One launcher (pid {}). Download the games there; `beacon status` finds them afterwards. Log: {}", child.id(), log.display()));
                let path = b.launcher_exe();
                report::ok(None, "Started the All-in-One launcher", json!({"pid": child.id(), "log": report::path(&log), "path": path.as_deref().map(report::path)}))
            }
            LauncherAction::Get { game: name } => {
                let g = game(&name)?;
                if aio::game_index(g).is_none() {
                    return Err(usage(format!("{} doesn't come from the All-in-One launcher (only bfme1, bfme2 and rotwk)", g.short())));
                }
                ctx.say(format!("Opening the All-in-One launcher: install {} there. Beacon waits and adds it when it's done.", g.title()));
                let out = b.get_with_launcher(g, &workshop::Net::new(), std::time::Duration::from_secs(6 * 3600), &mut |p: Option<u8>, t: &str| ctx.progress(p, t))?;
                b.save()?;
                let msg = if out.ready { format!("{} is installed", g.title()) } else { format!("{} wasn't installed (the launcher was closed first)", g.title()) };
                ctx.say(&msg);
                report::ok(Some(g), msg, json!({
                    "ready": out.ready,
                    "dir": out.dir.as_deref().map(report::path),
                    "closed_launcher": out.closed_launcher,
                    "installed_launcher": out.installed_launcher,
                }))
            }
        },
        Cmd::CommunityLauncher { game: g, action } => {
            let g = game(&g)?;
            let url = community_launcher::launcher_url(g).ok_or_else(|| usage(format!("{} has no community launcher (only uaw has one)", g.short())))?;
            match action {
                CommunityAction::Status => {
                    if b.game(g).community_launcher.is_some() && !b.adopt_community_games(g).is_empty() {
                        b.save()?;
                    }
                    let st = b.community_launcher_status(g).ok_or_else(|| usage(format!("{} has no community launcher", g.short())))?;
                    if !ctx.json {
                        match (&st.path, st.missing) {
                            (Some(p), _) => println!("{} launcher: {}", g.short(), p.display()),
                            (None, true) => println!("The {} launcher you chose is gone; choose it again with `beacon community-launcher {} set <path>`.", g.short(), g.slug()),
                            (None, false) => println!("No {} launcher chosen yet.", g.short()),
                        }
                        println!("Get it: {url}");
                        println!("{}", community_launcher::HOW_TO);
                        if let Some(pfx) = &st.prefix {
                            println!("Runs in: {}", pfx.display());
                        }
                    }
                    report::community_launcher(b, g).unwrap_or(Value::Null)
                }
                CommunityAction::Set { path } => {
                    let abs = std::path::absolute(&path).map_err(|e| usage(format!("{} isn't a usable path: {e}", path.display())))?;
                    community_launcher::validate_source(&abs).map_err(|e| usage(format!("{e:#}")))?;
                    let exe = b.set_community_launcher(g, &abs, &mut |t: &str| ctx.progress(None, t))?;
                    b.save()?;
                    ctx.say(format!("{} launcher: {}", g.short(), exe.display()));
                    ctx.say(format!("Open it with `beacon community-launcher {} open` and download the game there; Beacon finds it afterwards.", g.slug()));
                    report::ok(
                        Some(g),
                        format!("{} launcher chosen", g.short()),
                        json!({"path": report::path(&exe), "community_launcher": report::community_launcher(b, g)}),
                    )
                }
                CommunityAction::Open => {
                    let child = b.open_community_launcher(g, &mut |p: Option<u8>, t: &str| ctx.progress(p, t))?;
                    let log = b.community_launcher_log(g);
                    let path = b.community_launcher_exe(g);
                    ctx.say(format!(
                        "Started the {} launcher (pid {}). Download the game there; `beacon status` finds it afterwards. Log: {}",
                        g.short(),
                        child.id(),
                        log.display()
                    ));
                    report::ok(
                        Some(g),
                        format!("Started the {} launcher", g.short()),
                        json!({
                            "pid": child.id(),
                            "log": report::path(&log),
                            "path": path.as_deref().map(report::path),
                            "prefix": (!b.native).then(|| report::path(&b.run_root(g).join("pfx"))),
                        }),
                    )
                }
            }
        }
        Cmd::Get { game: g, repair, verify, yes } => private::get(ctx, b, &g, repair, verify, yes)?,
        Cmd::Sources { action } => private::sources_cmd(ctx, b, action)?,
        Cmd::SelfUpdate { check } => private::self_update(ctx, b, check)?,
        Cmd::ModGet { game: g, what, yes } => private::mod_get(ctx, b, &g, &what, yes)?,
        Cmd::Uninstall { game: g, delete_files, remove_prefix, yes } => {
            let g = game(&g)?;
            let plan = b.uninstall_plan(g);
            let size = |n: u64| format!("{:.2} GB", n as f64 / 1e9);
            if !yes {
                match &plan.dir {
                    Some(d) => ctx.say(format!("{}: {} ({})", g.short(), d.display(), size(plan.bytes))),
                    None => ctx.say(format!("{} has no game folder set.", g.short())),
                }
                if let Some(why) = &plan.why_not {
                    ctx.say(format!("Beacon won't delete its files: {why}."));
                }
                if let Some(u) = &plan.uninstaller {
                    ctx.say(format!("It has its own uninstaller: beacon run {} \"{}\"", g.slug(), u.display()));
                }
                if let Some(p) = &plan.prefix {
                    ctx.say(format!("Its Wine prefix (saves and settings): {} ({})", p.display(), size(plan.prefix_bytes)));
                }
                if !plan.needed_by.is_empty() {
                    ctx.say(format!("Needed by: {}", plan.needed_by.iter().map(|x| x.short()).collect::<Vec<_>>().join(", ")));
                }
                ctx.say(format!("Run `beacon uninstall {} --yes` with --delete-files and/or --remove-prefix to go ahead.", g.slug()));
                report::ok(Some(g), "Uninstall plan", json!({"plan": report::value(&plan)}))
            } else {
                let done = b.uninstall(g, delete_files, remove_prefix)?;
                b.save()?;
                ctx.say(format!("{} uninstalled ({} freed).", g.short(), size(done.freed)));
                report::ok(Some(g), format!("{} uninstalled", g.short()), json!({"done": report::value(&done)}))
            }
        }
        Cmd::Paths => {
            if !ctx.json {
                println!("config:   {}", b.paths.config_file.display());
                println!("prefixes: {}", b.paths.data_dir.join("prefixes").display());
                println!("mods:     {}", b.paths.mods_dir().display());
                println!("logs:     {}", b.paths.data_dir.join("logs").display());
            }
            report::paths(b)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_core::config::Paths;
    use clap::CommandFactory;
    use std::fs;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn json_goes_anywhere() {
        // Hearth's existing call, and the flag before the command.
        assert!(parse(&["beacon", "status", "--json"]).json);
        assert!(parse(&["beacon", "--json", "status"]).json);
        assert!(!parse(&["beacon", "status"]).json);
        let c = parse(&["beacon", "--json", "args", "rotwk", "--", "-noshellmap -win"]);
        assert!(matches!(c.cmd, Some(Cmd::Args { ref args, .. }) if args == "-noshellmap -win"));
    }

    #[test]
    fn cd_key_never_on_the_command_line() {
        assert!(Cli::try_parse_from(["beacon", "set-key", "bfme2", "ABCD-EFGH"]).is_err());
        assert!(matches!(parse(&["beacon", "set-key", "bfme2"]).cmd, Some(Cmd::SetKey { clear: false, .. })));
        assert!(matches!(parse(&["beacon", "set-key", "bfme2", "--clear"]).cmd, Some(Cmd::SetKey { clear: true, .. })));
    }

    #[test]
    fn run_takes_an_exe_or_the_tool() {
        assert!(Cli::try_parse_from(["beacon", "run", "aotr"]).is_err());
        assert!(Cli::try_parse_from(["beacon", "run", "aotr", "/x.exe", "--tool"]).is_err());
        assert!(matches!(parse(&["beacon", "run", "aotr", "--tool"]).cmd, Some(Cmd::Run { tool: true, exe: None, .. })));
        assert!(matches!(parse(&["beacon", "log", "aotr"]).cmd, Some(Cmd::Log { lines: 200, .. })));
    }

    #[test]
    fn new_commands_parse() {
        assert!(matches!(parse(&["beacon", "uninstall", "bfme2", "--delete-files", "--yes"]).cmd, Some(Cmd::Uninstall { delete_files: true, remove_prefix: false, yes: true, .. })));
        assert!(matches!(parse(&["beacon", "install-from", "bfme2", "/media/BFME2.iso", "--json"]).cmd, Some(Cmd::InstallFrom { .. })));
        assert!(matches!(parse(&["beacon", "workshop", "rotwk"]).cmd, Some(Cmd::Workshop { page: 0, ref search, .. }) if search.is_empty()));
        assert!(matches!(parse(&["beacon", "workshop", "rotwk", "--search", "edain", "--page", "2"]).cmd, Some(Cmd::Workshop { page: 2, .. })));
        assert!(matches!(parse(&["beacon", "workshop-install", "rotwk", "abc", "--select"]).cmd, Some(Cmd::WorkshopInstall { select: true, .. })));
        assert!(matches!(parse(&["beacon", "aio", "--json"]).cmd, Some(Cmd::Aio)));
        assert!(matches!(parse(&["beacon", "aio-launcher", "status", "--json"]).cmd, Some(Cmd::AioLauncher { action: LauncherAction::Status })));
        assert!(matches!(parse(&["beacon", "--json", "aio-launcher", "install"]).cmd, Some(Cmd::AioLauncher { action: LauncherAction::Install })));
        assert!(matches!(parse(&["beacon", "aio-launcher", "open"]).cmd, Some(Cmd::AioLauncher { action: LauncherAction::Open })));
        assert!(Cli::try_parse_from(["beacon", "aio-launcher"]).is_err());
        assert!(Cli::try_parse_from(["beacon", "workshop-install", "rotwk"]).is_err());
        assert!(matches!(parse(&["beacon", "community-launcher", "uaw", "status", "--json"]).cmd, Some(Cmd::CommunityLauncher { action: CommunityAction::Status, ref game }) if game == "uaw"));
        assert!(matches!(parse(&["beacon", "--json", "community-launcher", "uaw", "open"]).cmd, Some(Cmd::CommunityLauncher { action: CommunityAction::Open, .. })));
        assert!(matches!(parse(&["beacon", "community-launcher", "uaw", "set", "/d/UaW Launcher.zip", "--json"]).cmd, Some(Cmd::CommunityLauncher { action: CommunityAction::Set { ref path }, .. }) if path == Path::new("/d/UaW Launcher.zip")));
        assert!(Cli::try_parse_from(["beacon", "community-launcher", "uaw"]).is_err());
        assert!(Cli::try_parse_from(["beacon", "community-launcher", "uaw", "set"]).is_err());
        assert!(matches!(parse(&["beacon", "get", "rotwk", "--repair"]).cmd, Some(Cmd::Get { repair: true, verify: false, yes: false, .. })));
        assert!(matches!(parse(&["beacon", "sources"]).cmd, Some(Cmd::Sources { action: None })));
        assert!(matches!(parse(&["beacon", "sources", "check", "--json"]).cmd, Some(Cmd::Sources { action: Some(private::SourcesAction::Check) })));
        assert!(Cli::try_parse_from(["beacon", "sources", "token"]).is_err());
        assert!(matches!(parse(&["beacon", "sources", "repo", "Dragongamer9/beacon"]).cmd, Some(Cmd::Sources { action: Some(private::SourcesAction::Repo { .. }) })));
        assert!(matches!(parse(&["beacon", "self-update", "--check"]).cmd, Some(Cmd::SelfUpdate { check: true })));
        assert!(matches!(parse(&["beacon", "mod-get", "rotwk", "edain", "--yes"]).cmd, Some(Cmd::ModGet { yes: true, .. })));
    }

    #[test]
    fn new_commands_refuse_before_the_network() {
        let mut t = setup();
        // The workshop only has BFME, BFME II and RotWK.
        assert_eq!(exit_code(&go(&mut t, &["workshop", "swgb"])), 2);
        assert_eq!(exit_code(&go(&mut t, &["workshop-install", "conquest", "x"])), 2);
        // Private build: only the BFME games come from the CDN.
        assert_eq!(exit_code(&go(&mut t, &["get", "swgb"])), 2);
        // Base games and EA's patches are refused without asking the server.
        let r = go(&mut t, &["workshop-install", "bfme1", "original-BFME1"]);
        assert_eq!(exit_code(&r), 1);
        assert!(error_json(r.as_ref().unwrap_err())["error"].as_str().unwrap().contains("never downloads"));
        assert_eq!(exit_code(&go(&mut t, &["workshop-install", "bfme2", "official-1"])), 1);
        // install-from wants an installer, an image or a folder.
        let txt = t.games.join("notes.txt");
        fs::write(&txt, b"").unwrap();
        assert_eq!(exit_code(&go(&mut t, &["install-from", "bfme2", txt.to_str().unwrap()])), 2);
        let gone = t.games.join("gone.iso");
        assert_eq!(exit_code(&go(&mut t, &["install-from", "bfme2", gone.to_str().unwrap()])), 2);
        let v = go(&mut t, &["aio"]).unwrap();
        assert_eq!(v["launcher_url"], "https://bfmeladder.com");
        assert!(v["games"].as_array().unwrap().is_empty());
        assert_eq!(v["launcher"]["installed"], false);
        assert!(v["launcher"]["path"].is_null());
        let l = go(&mut t, &["aio-launcher", "status"]).unwrap();
        assert_eq!(l, v["launcher"]);
        // Not installed: open fails (exit 1) without starting anything.
        let r = go(&mut t, &["aio-launcher", "open"]);
        assert_eq!(exit_code(&r), 1);
        assert!(error_json(r.as_ref().unwrap_err())["error"].as_str().unwrap().contains("isn't installed"));
        let s = go(&mut t, &["status"]).unwrap();
        assert_eq!(s[8]["store_url"], "https://store.steampowered.com/app/356500/");
    }

    #[test]
    fn community_launcher_commands() {
        let mut t = setup();
        // Only UaW has one: a usage error for the others, an unknown game too.
        assert_eq!(exit_code(&go(&mut t, &["community-launcher", "swgb", "status"])), 2);
        assert_eq!(exit_code(&go(&mut t, &["community-launcher", "doom", "status"])), 2);
        let v = go(&mut t, &["community-launcher", "uaw", "status"]).unwrap();
        assert_eq!(v["url"], "https://drive.google.com/drive/folders/167UET84ZWXrMd9zpxEYiQWTQZexCGZfT");
        assert!(v["path"].is_null());
        // Nothing chosen: open fails (exit 1) without starting anything.
        let r = go(&mut t, &["community-launcher", "uaw", "open"]);
        assert_eq!(exit_code(&r), 1);
        assert!(error_json(r.as_ref().unwrap_err())["error"].as_str().unwrap().contains("no UaW launcher chosen"));
        // Only an existing .exe or .zip.
        let txt = t.games.join("notes.txt");
        fs::write(&txt, b"").unwrap();
        assert_eq!(exit_code(&go(&mut t, &["community-launcher", "uaw", "set", txt.to_str().unwrap()])), 2);
        let gone = t.games.join("gone.exe");
        assert_eq!(exit_code(&go(&mut t, &["community-launcher", "uaw", "set", gone.to_str().unwrap()])), 2);
        let exe = t.games.join("UaW Launcher.exe");
        fs::write(&exe, b"MZ").unwrap();
        let v = go(&mut t, &["community-launcher", "uaw", "set", exe.to_str().unwrap()]).unwrap();
        assert_eq!((v["ok"].as_bool(), v["game"].as_str()), (Some(true), Some("uaw")));
        let chosen = PathBuf::from(v["path"].as_str().unwrap());
        assert!(chosen.starts_with(t.b.community_launcher_dir(GameId::Uaw)) && chosen.is_file());
        assert_eq!(v["community_launcher"]["path"], v["path"]);
        assert_eq!(t.b.game(GameId::Uaw).community_launcher.as_deref(), Some(chosen.as_path()));
        let s = go(&mut t, &["status"]).unwrap();
        assert_eq!(s[5]["id"], "uaw");
        assert_eq!(s[5]["community_launcher"]["path"], v["path"]);
        assert!(s[5]["community_launcher"]["url"].as_str().unwrap().starts_with("https://drive.google.com/"));
        assert!(s[1].get("community_launcher").is_none());
    }

    struct T {
        _t: tempfile::TempDir,
        b: Beacon,
        games: PathBuf,
    }

    fn setup() -> T {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let games = home.join("Games");
        for (d, exe) in [("BFME2", "lotrbfme2.exe"), ("RotWK", "lotrbfme2ep1.exe")] {
            fs::create_dir_all(games.join(d)).unwrap();
            fs::write(games.join(d).join(exe), b"").unwrap();
        }
        let bin = t.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        fs::write(&wine, "#!/bin/sh\nmkdir -p \"$WINEPREFIX/drive_c/users/tester\"\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let b = Beacon::load_with(Paths::under(&t.path().join("beacon"), &home), &bin.to_string_lossy()).unwrap();
        T { _t: t, b, games }
    }

    fn json_ctx() -> Ctx {
        Ctx { json: true, key_input: None }
    }

    fn go(t: &mut T, args: &[&str]) -> Result<Value> {
        let mut all = vec!["beacon", "--json"];
        all.extend_from_slice(args);
        let cli = Cli::try_parse_from(all).unwrap();
        run(&json_ctx(), &mut t.b, cli.cmd.unwrap())
    }

    // Uses the stub `wine` (a shell script) and Wine runner ids.
    #[cfg(unix)]
    #[test]
    fn write_results_and_exit_codes() {
        let mut t = setup();
        let r = go(&mut t, &["scan"]);
        assert_eq!(exit_code(&r), 0);
        let v = r.unwrap();
        assert_eq!((v["ok"].as_bool(), v["game"].is_null()), (Some(true), true));
        assert_eq!(v["found"].as_array().unwrap().len(), 2);
        assert!(v["installed"].as_array().unwrap().iter().any(|g| g == "rotwk"));

        let v = go(&mut t, &["resolution", "rotwk", "1280x800"]).unwrap();
        assert_eq!(v, json!({"ok": true, "game": "rotwk", "message": "Resolution: 1280x800", "resolution": "1280x800"}));
        assert_eq!(t.b.game(GameId::Rotwk).resolution.unwrap().to_string(), "1280x800");

        // Bad input: exit code 2, kind "usage".
        let r = go(&mut t, &["resolution", "rotwk", "huge"]);
        assert_eq!(exit_code(&r), 2);
        assert_eq!(error_json(r.as_ref().unwrap_err())["kind"], "usage");
        assert_eq!(exit_code(&go(&mut t, &["windowed", "nope", "on"])), 2);
        assert_eq!(exit_code(&go(&mut t, &["set-runner", "rotwk", "proton:missing"])), 2);
        assert_eq!(exit_code(&go(&mut t, &["select-mod", "rotwk", "rotwk:missing"])), 2);
        assert_eq!(exit_code(&go(&mut t, &["virtual-desktop", "rotwk", "on"])), 2);
        assert_eq!(exit_code(&go(&mut t, &["add-files", "swgb", "nope", "/tmp"])), 2);
        // Something that fails for another reason: exit code 1, kind "failed".
        let r = go(&mut t, &["set-path", "bfme1", "/nonexistent-folder"]);
        assert_eq!(exit_code(&r), 1);
        assert_eq!(error_json(r.as_ref().unwrap_err())["kind"], "failed");

        let v = go(&mut t, &["set-runner", "rotwk", "wine:system"]).unwrap();
        assert_eq!((v["runner"].as_str(), v["default"].as_bool()), (Some("wine:system"), Some(false)));
        let v = go(&mut t, &["set-runner", "default", "auto"]).unwrap();
        assert!(v["runner"].is_null() && v["game"].is_null() && v["default"] == true);

        let v = go(&mut t, &["args", "rotwk", "--", "-noshellmap"]).unwrap();
        assert_eq!(v["extra_args"], "-noshellmap");
        assert_eq!(go(&mut t, &["windowed", "rotwk", "on"]).unwrap()["windowed"], true);
        let v = go(&mut t, &["virtual-desktop", "swgb", "off"]).unwrap();
        assert_eq!((v["virtual_desktop"].as_bool(), v["choice"].as_bool()), (Some(false), Some(false)));
        assert!(go(&mut t, &["via-steam", "swgb", "on"]).unwrap()["steam_copy"] == false);
    }

    #[test]
    fn cd_key_comes_from_stdin_and_is_never_echoed() {
        let mut t = setup();
        let cli = Cli::try_parse_from(["beacon", "--json", "set-key", "bfme2"]).unwrap();
        let ctx = Ctx { json: true, key_input: Some("SECRET-KEY-1234\n".into()) };
        let v = run(&ctx, &mut t.b, cli.cmd.unwrap()).unwrap();
        assert_eq!(v, json!({"ok": true, "game": "bfme2", "message": "CD key saved", "has_cd_key": true}));
        assert!(!v.to_string().contains("SECRET"));
        assert_eq!(t.b.game(GameId::Bfme2).cd_key.as_deref(), Some("SECRET-KEY-1234"));
        let ctx = Ctx { json: true, key_input: Some("\n".into()) };
        let cli = Cli::try_parse_from(["beacon", "--json", "set-key", "bfme2"]).unwrap();
        assert_eq!(exit_code(&run(&ctx, &mut t.b, cli.cmd.unwrap())), 2, "an empty key isn't a silent clear");
        let v = go(&mut t, &["set-key", "bfme2", "--clear"]).unwrap();
        assert_eq!(v["has_cd_key"], false);
        assert!(t.b.game(GameId::Bfme2).cd_key.is_none());
    }

    // Uses the stub `wine` (a shell script) and Wine runner ids.
    #[cfg(unix)]
    #[test]
    fn mods_and_setup() {
        let mut t = setup();
        go(&mut t, &["scan"]).unwrap();
        let big = t.games.join("widescreen.big");
        fs::write(&big, b"").unwrap();
        let v = go(&mut t, &["add-mod", "rotwk", big.to_str().unwrap(), "--name", "Widescreen", "--select"]).unwrap();
        assert_eq!((v["mod"]["id"].as_str(), v["selected"].as_bool()), (Some("rotwk:widescreen"), Some(true)));
        let m = go(&mut t, &["mods", "rotwk"]).unwrap();
        assert_eq!(m["selected"], "rotwk:widescreen");
        assert_eq!(go(&mut t, &["select-mod", "rotwk", "none"]).unwrap()["selected"], Value::Null);
        assert_eq!(go(&mut t, &["remove-mod", "rotwk:widescreen"]).unwrap()["removed"], "rotwk:widescreen");
        assert_eq!(exit_code(&go(&mut t, &["remove-mod", "rotwk:widescreen"])), 2);
        assert!(go(&mut t, &["catalog", "rotwk"]).unwrap()["items"].as_array().unwrap().len() > 2);
        assert_eq!(go(&mut t, &["find-mods"]).unwrap()["added"], json!([]));

        let v = go(&mut t, &["setup", "bfme2"]).unwrap();
        assert_eq!((v["ok"].as_bool(), v["did_work"].as_bool()), (Some(true), Some(true)));
        assert_eq!(go(&mut t, &["setup", "bfme2"]).unwrap()["did_work"], false);
        let l = go(&mut t, &["log", "bfme2", "--lines", "5"]).unwrap();
        assert_eq!(l["exists"], true);
        assert!(l["lines"][0].as_str().unwrap().starts_with("== beacon setup"));
        let d = go(&mut t, &["launch", "bfme2", "--dry-run"]).unwrap();
        assert!(d["command"]["display"].as_str().unwrap().contains("lotrbfme2.exe"));
        assert_eq!(go(&mut t, &["runners"]).unwrap()["best"], "wine:system");
        assert!(go(&mut t, &["paths"]).unwrap()["games"]["rotwk"]["log"].as_str().unwrap().ends_with("rotwk.log"));
        assert_eq!(go(&mut t, &["community", "uaw"]).unwrap()["steps"][0]["id"], "patch3");
        let s = go(&mut t, &["status"]).unwrap();
        assert_eq!(s[1]["id"], "bfme2");
        assert_eq!(s[1]["needs_setup"], false);
    }

    // Uses the stub `wine` (a shell script) and Wine runner ids.
    #[cfg(unix)]
    #[test]
    fn installers_report_their_exit() {
        let mut t = setup();
        go(&mut t, &["scan"]).unwrap();
        let exe = t.games.join("patch.exe");
        fs::write(&exe, b"").unwrap();
        let r = go(&mut t, &["run", "rotwk", exe.to_str().unwrap()]);
        assert_eq!(exit_code(&r), 0);
        let v = r.unwrap();
        assert_eq!((v["ok"].as_bool(), v["exit_code"].as_i64(), v["installed"].as_bool()), (Some(true), Some(0), Some(true)));
        assert_eq!(exit_code(&go(&mut t, &["run", "rotwk", "--tool"])), 2, "RotWK has no launcher of its own");

        // A failing installer: ok false, exit code 3.
        let bin = t.b.runners.get("wine:system").unwrap().path.clone();
        fs::write(&bin, "#!/bin/sh\nexit 4\n").unwrap();
        let r = go(&mut t, &["run", "conquest", exe.to_str().unwrap()]);
        assert_eq!(exit_code(&r), 3);
        let v = r.unwrap();
        assert_eq!((v["ok"].as_bool(), v["kind"].as_str(), v["exit_code"].as_i64()), (Some(false), Some("program"), Some(4)));
        assert!(t.b.paths.prefix_root(GameId::Conquest).is_dir(), "an installer gets a prefix before its game exists");
    }
}
