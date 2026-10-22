//! Private build: `beacon get` (games from the BFME Foundation's CDN) and
//! `beacon sources` (the download list: show, check, update, token, folder).

use super::{game, usage, Ctx};
use anyhow::Result;
use beacon_core::downloads::{archive, basegame, launcher, sources, Client};
use beacon_core::{report, Beacon};
use clap::Subcommand;
use serde_json::{json, Value};
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum SourcesAction {
    /// The sources in use and their list version (default).
    List,
    /// Look every source up and check that its file answers.
    Check,
    /// Fetch a newer sources.json from the private repo now.
    Update,
    /// Store the GitHub token used for updates, read from stdin (Contents: read on the private repo).
    Token {
        /// Remove the saved token instead.
        #[arg(long)]
        clear: bool,
    },
    /// Where downloaded games go (`default` = Beacon's data folder).
    GamesDir { path: String },
}

/// Progress for people on a terminal: one line, rewritten (stderr).
fn human_progress(ctx: &Ctx) -> impl FnMut(Option<u8>, &str) + '_ {
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let tty = std::io::stderr().is_terminal();
    move |pct: Option<u8>, text: &str| {
        if ctx.json {
            ctx.progress(pct, text);
            return;
        }
        if !tty || last.elapsed().as_millis() < 150 {
            return;
        }
        last = std::time::Instant::now();
        let line: String = text.chars().take(70).collect();
        match pct {
            Some(p) => eprint!("\r\x1b[2K[{p:>3}%] {line}"),
            None => eprint!("\r\x1b[2K{line}"),
        }
        let _ = std::io::stderr().flush();
    }
}

pub fn get(ctx: &Ctx, b: &mut Beacon, g: &str, repair: bool, verify: bool, yes: bool) -> Result<Value> {
    let g = game(g)?;
    if !basegame::supported(g) {
        if b.has_archive_source(g) {
            return get_archive(ctx, b, g, yes);
        }
        if b.has_launcher_source(g) {
            return get_launcher(ctx, b, g, repair || verify, yes);
        }
        return Err(usage(format!("Beacon can't download {} (bfme1, bfme2 and rotwk come from the BFME Foundation's CDN; uaw's launcher and rol from Google Drive)", g.short())));
    }
    let opts = basegame::Options { thorough: verify, accept_guess: yes, reinstall: repair || verify };
    let net = Client::new();
    let mut progress = human_progress(ctx);
    let out = b.cdn_download(&net, g, opts, &mut progress)?;
    if !ctx.json && std::io::stderr().is_terminal() {
        eprintln!();
    }
    match out {
        basegame::Outcome::NeedsConfirm { game: which, note } => {
            ctx.say(format!("Not downloaded: {note}.\nIf that looks right, run `beacon get {} --yes`.", g.slug()));
            Ok(json!({"ok": false, "kind": "needs-confirm", "game": which.slug(), "error": note}))
        }
        basegame::Outcome::Done { games } if games.is_empty() => {
            ctx.say(format!("{} is already installed. `beacon get {} --repair` checks its files.", g.short(), g.slug()));
            Ok(report::ok(Some(g), "Already installed", json!({"games": []})))
        }
        basegame::Outcome::Done { games } => {
            b.cdn_adopt(&games)?;
            b.save()?;
            for i in &games {
                if let Some(n) = &i.note {
                    ctx.say(format!("Note: {n}"));
                }
                ctx.say(format!(
                    "{}: {} {} in {} ({} files, {:.2} GB downloaded, {} already fine)",
                    i.game.short(),
                    i.name,
                    i.version,
                    i.dir.display(),
                    i.files,
                    i.downloaded_bytes as f64 / 1e9,
                    i.kept
                ));
            }
            ctx.say("Writing registry keys…");
            b.cdn_register(&games)?;
            ctx.say(format!("Ready: `beacon launch {}`", g.slug()));
            Ok(report::ok(Some(g), format!("{} installed", g.short()), json!({"games": report::value(&games)})))
        }
    }
}

pub fn sources_cmd(ctx: &Ctx, b: &mut Beacon, action: Option<SourcesAction>) -> Result<Value> {
    let store = b.download_store();
    match action.unwrap_or(SourcesAction::List) {
        SourcesAction::List => {
            let list = store.list();
            let s = store.settings();
            let learned = store.learned();
            if !ctx.json {
                println!("sources.json version {} ({})", list.version, if list == sources::SourceList::bundled() { "built in" } else { "downloaded" });
                println!("updates from {} ({}), token {}", s.sources_repo, s.sources_branch, if s.github_token.is_some() { "set" } else { "not set" });
                println!("games go to {}", b.games_root().display());
                if let Some(api) = &learned.workshop_api {
                    println!("workshop found at {api} (learned)");
                }
                for src in &list.sources {
                    println!("  {:<10} {}", src.id, src.title);
                }
            }
            Ok(json!({
                "ok": true,
                "version": list.version,
                "sources": report::value(&list.sources),
                "sources_repo": s.sources_repo,
                "token_set": s.github_token.is_some(),
                "games_dir": report::path(&b.games_root()),
                "learned": report::value(&learned),
            }))
        }
        SourcesAction::Check => {
            let net = Client::new();
            let list = store.list();
            let mut learned = store.learned();
            let checks = sources::check_all(&net, &list, &mut learned);
            let _ = store.save_learned(&learned);
            let _ = std::fs::create_dir_all(&store.dir);
            let _ = std::fs::write(store.status_file(), serde_json::to_string_pretty(&checks)?);
            let broken = checks.iter().filter(|c| c.state == sources::State::Broken).count();
            if !ctx.json {
                for c in &checks {
                    let tag = match c.state {
                        sources::State::Ok => "ok",
                        sources::State::Moved => "moved",
                        sources::State::NeedsConfirm => "guess",
                        sources::State::Broken => "BROKEN",
                    };
                    println!("{tag:>6}  {:<10} {}", c.id, c.detail);
                }
            }
            Ok(json!({"ok": broken == 0, "checks": report::value(&checks)}))
        }
        SourcesAction::Update => {
            let u = store.update(&Client::new(), true)?;
            match (&u.skipped, u.to > u.from) {
                (Some(why), _) => ctx.say(format!("Not updated: {why}")),
                (None, true) => ctx.say(format!("sources.json updated: version {} → {}", u.from, u.to)),
                (None, false) => ctx.say(format!("sources.json is up to date (version {})", u.from)),
            }
            Ok(json!({"ok": u.skipped.is_none() || u.skipped.as_deref() == Some("checked recently"), "update": report::value(&u)}))
        }
        SourcesAction::Token { clear } => {
            let mut s = store.settings();
            if clear {
                s.github_token = None;
            } else {
                let stdin = std::io::stdin();
                if stdin.is_terminal() && !ctx.json {
                    eprint!("GitHub token: ");
                }
                let mut line = String::new();
                stdin.lock().read_line(&mut line)?;
                let tok = line.trim().to_string();
                if tok.is_empty() || tok.chars().any(|c| c.is_whitespace() || c.is_control()) {
                    return Err(usage("that doesn't look like a GitHub token"));
                }
                s.github_token = Some(tok);
            }
            store.save_settings(&s)?;
            ctx.say(if clear { "Token removed." } else { "Token saved (readable only by you). `beacon sources update` fetches the list now." });
            Ok(json!({"ok": true, "token_set": s.github_token.is_some()}))
        }
        SourcesAction::GamesDir { path } => {
            let mut s = store.settings();
            s.games_dir = if path.eq_ignore_ascii_case("default") { None } else { Some(PathBuf::from(path)) };
            if let Some(p) = &s.games_dir {
                if !p.is_absolute() {
                    return Err(usage("use an absolute folder"));
                }
            }
            store.save_settings(&s)?;
            ctx.say(format!("Downloaded games go to {}", b.games_root().display()));
            Ok(json!({"ok": true, "games_dir": report::path(&b.games_root())}))
        }
    }
}

/// `beacon get uaw`: the newest community launcher, set up for the game.
fn get_launcher(ctx: &Ctx, b: &mut Beacon, g: beacon_core::games::GameId, force: bool, yes: bool) -> Result<Value> {
    let mut progress = human_progress(ctx);
    let out = b.launcher_download(&Client::new(), g, yes, force, &mut progress)?;
    if !ctx.json && std::io::stderr().is_terminal() {
        eprintln!();
    }
    match out {
        launcher::Outcome::NeedsConfirm { note, .. } => {
            ctx.say(format!("Not downloaded: {note}.\nIf that looks right, run `beacon get {} --yes`.", g.slug()));
            Ok(json!({"ok": false, "kind": "needs-confirm", "game": g.slug(), "error": note}))
        }
        launcher::Outcome::Ready(f) if f.up_to_date => {
            ctx.say(format!("The {} launcher in use is already the newest ({}). `--repair` downloads it again.", g.short(), f.name));
            Ok(report::ok(Some(g), "Up to date", json!({"launcher": report::value(&f)})))
        }
        launcher::Outcome::Ready(f) => {
            if let Some(n) = &f.note {
                ctx.say(format!("Note: {n}"));
            }
            let exe = b.set_community_launcher(g, &f.file, &mut |t: &str| ctx.progress(None, t))?;
            b.save()?;
            ctx.say(format!("{} launcher ready: {}\nOpen it with `beacon community-launcher {} open` and download the game there.", g.short(), exe.display(), g.slug()));
            Ok(report::ok(Some(g), format!("{} launcher ready", g.short()), json!({"launcher": report::value(&f), "exe": report::path(&exe)})))
        }
    }
}

/// `beacon get rol`: the game's archive, extracted; the game itself, or a launcher to run.
fn get_archive(ctx: &Ctx, b: &mut Beacon, g: beacon_core::games::GameId, yes: bool) -> Result<Value> {
    let mut progress = human_progress(ctx);
    let out = b.archive_download(&Client::new(), g, yes, &mut progress)?;
    let f = match out {
        archive::Outcome::NeedsConfirm { note, .. } => {
            ctx.say(format!("Not downloaded: {note}.\nIf that looks right, run `beacon get {} --yes`.", g.slug()));
            return Ok(json!({"ok": false, "kind": "needs-confirm", "game": g.slug(), "error": note}));
        }
        archive::Outcome::Ready(f) => f,
    };
    let u = b.archive_unpack(g, &f.file, &mut progress)?;
    if !ctx.json && std::io::stderr().is_terminal() {
        eprintln!();
    }
    b.archive_adopt(&u)?;
    b.save()?;
    match &u {
        archive::Unpacked::Game { dir, .. } => ctx.say(format!("{} is ready in {}: `beacon launch {}`", g.short(), dir.display(), g.slug())),
        archive::Unpacked::Launcher { exe, .. } => ctx.say(format!(
            "{} held a launcher/installer, not the game itself: {}\nRun it in {}'s prefix with `beacon run {} \"{}\"`; Beacon finds the game when it closes.",
            f.name,
            exe.display(),
            g.short(),
            g.slug(),
            exe.display()
        )),
    }
    Ok(report::ok(Some(g), format!("{} downloaded", g.short()), json!({"download": report::value(&f), "unpacked": report::value(&u)})))
}
