# Beacon (private build)

This repo is a private fork of [Dragongamer9/beacon](https://github.com/Dragongamer9/beacon). Its only addition is
Beacon's own downloads. Don't publish its builds.

## What it adds

- **One-press BFME, BFME II and RotWK.** Downloads come straight from the BFME Foundation's CDN. These are the same
  base packages the All-in-One launcher installs (`original-BFME1`, `original-BFME2`, `original-RotWK`).
  - Installing RotWK also installs BFME II when it's missing.
  - Each game gets a made-up CD key, the same way the launcher does it, plus its registry keys (on Linux in the
    game's prefix; on Windows through `reg import`, with a UAC prompt).
  - Downloads resume where they stopped. Every file is checked against the package's size and MD5.
- **Universe at War's community launcher, downloaded for you.** Beacon fetches the newest launcher from the
  community's Google Drive folder and sets it up the same way as one you picked by hand; open it from there to
  download the game.
  - Beacon lists the folder each time, so new versions get picked up on their own. If the launchers move into
    subfolders, Beacon searches the newest one.
  - "Update the UaW launcher" downloads only when a newer file is in the folder.
- **Links that fix themselves.** Nothing is a hard-coded download link. `sources.json` says how to *find* each file,
  and Beacon looks it up at download time:
  - `workshop`: the Foundation's API. If its server moves, Beacon reads the All-in-One launcher's source on GitHub for
    the new address, remembers it, and carries on. If a package's id changes, Beacon searches the workshop for the
    game's base package and shows you the guess before downloading.
  - `page`: a web page scanned for a link whose file name matches a pattern, newest version first, optionally
    through intermediate pages. If nothing matches, the closest link is offered, never downloaded without a yes.
  - `github`: the latest release asset matching a pattern.
  - `drive-folder`: the newest file matching a pattern in a public Google Drive folder. Beacon gets past Drive's
    virus-scan warning page that large files get.
  - `direct`: a plain https URL.
- **Fixes without rebuilding.** Beacon fetches `sources.json` from this repo, at most every 6 hours and on demand. The
  higher `version` wins over the copy built in. So when a site changes in a way the scanner can't follow, edit
  `sources.json` here, bump `version`, and push. Every install of this build picks it up.
- **Check sources** (on start-up, in Launcher settings, or `beacon sources check`) looks up every source and touches
  its file. A broken link shows up before you need it.

## Setting it up

1. **Token** (for `sources.json` updates):
   - On GitHub, go to Settings → Developer settings → Fine-grained tokens.
   - Create a token with access to `Dragongamer9/beacon-private` only and **Contents: read-only**.
   - Paste it into Launcher settings → Downloads → GitHub token, or run `beacon sources token` (it reads from stdin).
   - It's stored in `~/.config/beacon/private.json`, readable only by you.
2. **Games folder** (optional): the default is `~/.local/share/beacon/games/<BFME1|BFME2|RotWK>`. Change it in
   Launcher settings → Downloads, or with `beacon sources games-dir /path` (`default` resets it).

## Command line

```sh
beacon get rotwk                 # download RotWK (+ BFME II if missing), set up, ready to launch
beacon get bfme1 --repair        # download anything missing (size check)
beacon get bfme1 --verify        # check every file's MD5 and replace damaged ones
beacon get bfme1 --yes           # accept a source Beacon only guessed after its link changed
beacon get uaw                   # newest UaW community launcher from its Drive folder, set up
beacon get uaw --repair          # download it again even if it's the one in use
beacon sources                   # list version, sources, token, games folder
beacon sources check             # look every source up now
beacon sources update            # fetch sources.json from this repo now
beacon sources token [--clear]   # store the GitHub token (stdin)
beacon sources games-dir <path|default>
```

## Keeping up with public Beacon

```sh
git fetch upstream
git merge upstream/main          # private code lives in new files, so conflicts stay small
```

Private code lives in:
- `crates/beacon-core/src/downloads/`
- `crates/beacon-cli/src/private.rs`
- `src-tauri/src/private.rs`
- `sources.json`
- the "private build" block in `ui/app.js` and the mock handlers in `ui/api.js`

The rest of the tree only gets small hooks:
- `pub mod downloads` and a games-folder scan root in `lib.rs`
- two subcommands in the CLI
- six commands and a start-up check in the app
- a few rows in the game and launcher sheets
