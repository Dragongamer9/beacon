//! `beacon` – headless launcher for BFME, BFME II and RotWK.
//! Handy over SSH, in scripts, and for Hearth to call into.

use anyhow::{anyhow, bail, Result};
use beacon_core::{games::GameId, mods::ModKind, Beacon, Level, Resolution};
use clap::{Parser, Subcommand};
use std::io::BufRead;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "beacon", version, about = "Linux launcher for The Battle for Middle-earth games")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show each game's state (default).
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Look for installed games and fill in missing folders.
    Scan { roots: Vec<PathBuf> },
    /// Point a game at its folder (or its exe).
    SetPath { game: String, path: PathBuf },
    /// Store the CD key from your own copy. Reads stdin if omitted.
    SetKey { game: String, key: Option<String> },
    /// List Wine/Proton/umu runners.
    Runners,
    /// Pick a runner for one game, or `default` for all.
    SetRunner { game: String, runner: String },
    /// Set resolution, e.g. 1920x1080, or `clear`.
    Resolution { game: String, value: String },
    /// Windowed mode on/off.
    Windowed { game: String, value: String },
    /// Extra game arguments (quoted string; empty clears).
    Args { game: String, args: String },
    /// List mods for a game.
    Mods { game: String },
    /// Link a mod from anywhere (.big, folder of .big files, or launcher .exe).
    AddMod {
        game: String,
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
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
    /// Run a Windows exe (patch, mod installer) in a game's prefix.
    Run { game: String, exe: PathBuf },
    /// Show where config, prefixes, mods and logs live.
    Paths,
}

fn game(s: &str) -> Result<GameId> {
    GameId::parse(s).ok_or_else(|| anyhow!("unknown game \"{s}\" (use bfme1, bfme2, rotwk or aotr)"))
}

fn on_off(s: &str) -> Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Ok(true),
        "off" | "false" | "no" | "0" => Ok(false),
        _ => bail!("expected on or off"),
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut b = Beacon::load()?;
    match cli.cmd.unwrap_or(Cmd::Status { json: false }) {
        Cmd::Status { json } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&b.statuses())?);
            } else {
                print_status(&b);
            }
        }
        Cmd::Scan { roots } => {
            let found = b.scan(&roots);
            if found.is_empty() {
                println!("No games found. Use `beacon set-path <game> <folder>` or `beacon scan <folder>`.");
            }
            for f in &found {
                println!("found {} in {}", f.game.short(), f.dir.display());
            }
            b.save()?;
            println!();
            print_status(&b);
        }
        Cmd::SetPath { game: g, path } => {
            let g = game(&g)?;
            b.set_path(g, &path)?;
            b.save()?;
            println!("{} → {}", g.short(), b.game(g).path.unwrap().display());
        }
        Cmd::SetKey { game: g, key } => {
            let g = game(&g)?;
            let key = match key {
                Some(k) => k,
                None => {
                    eprint!("CD key for {}: ", g.short());
                    let mut line = String::new();
                    std::io::stdin().lock().read_line(&mut line)?;
                    line
                }
            };
            let key = key.trim().to_string();
            b.game_mut(g).cd_key = (!key.is_empty()).then_some(key);
            b.save()?;
            println!("Saved. It's applied on the next launch.");
        }
        Cmd::Runners => {
            if b.runners.runners.is_empty() {
                println!("No runners found. Install GE-Proton (e.g. with ProtonUp-Qt), umu-launcher, or wine.");
            }
            let best = b.runners.best().map(|r| r.id.clone());
            for r in &b.runners.runners {
                let star = if Some(&r.id) == best.as_ref() { "*" } else { " " };
                println!("{star} {:<32} {}", r.id, r.path.display());
            }
        }
        Cmd::SetRunner { game: g, runner } => {
            if b.runners.get(&runner).is_none() {
                bail!("no runner {runner}; see `beacon runners`");
            }
            if g == "default" {
                b.config.default_runner = Some(runner);
            } else {
                b.game_mut(game(&g)?).runner = Some(runner);
            }
            b.save()?;
        }
        Cmd::Resolution { game: g, value } => {
            let g = game(&g)?;
            b.game_mut(g).resolution = if value == "clear" {
                None
            } else {
                Some(Resolution::parse(&value).ok_or_else(|| anyhow!("use WIDTHxHEIGHT, e.g. 1920x1080"))?)
            };
            b.save()?;
        }
        Cmd::Windowed { game: g, value } => {
            b.game_mut(game(&g)?).windowed = on_off(&value)?;
            b.save()?;
        }
        Cmd::Args { game: g, args } => {
            b.game_mut(game(&g)?).extra_args = args;
            b.save()?;
        }
        Cmd::Mods { game: g } => {
            let g = game(&g)?;
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
        Cmd::AddMod { game: g, path, name } => {
            let m = b.link_mod(game(&g)?, &path, name.as_deref())?;
            b.save()?;
            println!("added {} ({})", m.name, m.id);
        }
        Cmd::Catalog { game: g } => {
            let g = game(&g)?;
            for item in b.catalog(g) {
                let mark = match &item.installed {
                    Some(id) => format!("installed as {id}"),
                    None => "not installed".into(),
                };
                println!("{} [{}] — {mark}\n    {}\n    {}", item.entry.name, if item.entry.kind == beacon_core::catalog::EntryKind::Patch { "patch" } else { "mod" }, item.entry.blurb, item.entry.url);
            }
        }
        Cmd::FindMods { roots } => {
            let added = b.find_mods(&roots);
            b.save()?;
            if added.is_empty() {
                println!("No new mods found.");
            }
            for m in added {
                println!("added {} for {} ({})", m.name, m.game.short(), m.path.display());
            }
        }
        Cmd::RemoveMod { id } => {
            if !b.unlink_mod(&id) {
                bail!("no linked mod {id} (mods in the mods folder are removed by deleting them)");
            }
            b.save()?;
        }
        Cmd::SelectMod { game: g, id } => {
            let g = game(&g)?;
            b.select_mod(g, (id != "none").then_some(id.as_str()))?;
            b.save()?;
        }
        Cmd::Setup { game: g, force } => {
            let g = game(&g)?;
            println!("Preparing {} prefix (the first time can take a minute)…", g.short());
            if b.setup(g, force)? {
                println!("Done.");
            } else {
                println!("Already set up. Use --force to redo it.");
            }
            if let Some(f) = b.apply_options(g)? {
                println!("Wrote {}", f.display());
            }
        }
        Cmd::Launch { game: g, mod_id, vanilla, dry_run, wait } => {
            let g = game(&g)?;
            let m = if vanilla { Some("") } else { mod_id.as_deref() };
            if dry_run {
                let cmd = b.launch_command(g, m)?;
                if b.needs_setup(g)? {
                    println!("# setup first:\n(cd {} && {})", b.paths.prefix_root(g).display(), b.setup_command(g)?.display());
                }
                println!("(cd {} && {})", cmd.cwd.display(), cmd.display());
                return Ok(());
            }
            let mut child = b.launch(g, m)?;
            println!("Started {} (pid {}). Log: {}", g.short(), child.id(), b.paths.log_file(g).display());
            if wait {
                let st = child.wait()?;
                println!("Exited: {st}");
            }
        }
        Cmd::Run { game: g, exe } => {
            let g = game(&g)?;
            b.setup(g, false)?;
            let cmd = b.run_in_prefix_command(g, &exe)?;
            let st = b.spawn_logged(g, &cmd)?.wait()?;
            println!("Exited: {st}");
            // Pick up anything the installer just put in the prefix.
            for f in b.scan(&[]) {
                println!("found {} in {}", f.game.short(), f.dir.display());
            }
            b.save()?;
        }
        Cmd::Paths => {
            println!("config:   {}", b.paths.config_file.display());
            println!("prefixes: {}", b.paths.data_dir.join("prefixes").display());
            println!("mods:     {}", b.paths.mods_dir().display());
            println!("logs:     {}", b.paths.data_dir.join("logs").display());
        }
    }
    Ok(())
}
