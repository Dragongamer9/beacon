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
- **Rise of Legends from its Google Drive zip.** Beacon downloads it, extracts it and checks what's inside:
  - The game itself (`RiseOfLegends.exe`): it goes to `<games>/Rise of Legends` and is ready to play. The zip is
    deleted afterwards to save space.
  - A launcher or installer: it's kept in Beacon's tools folder and run in RoL's prefix, and Beacon finds the game
    when it closes. "Run the RoL launcher again" stays on the game's page until the game is installed.
- **Mods without the browser.** In Mods → Browse known mods & patches, mods with a download get a **Download** button.
  You can also paste any ModDB link under "Download from a link". Beacon follows ModDB's pages to the file, shows its
  name and size, and downloads it once you say yes. The file is checked against ModDB's MD5, and downloads resume
  where they stopped. Then it puts the mod in the right place for that game, wherever the game is installed:
  - `.big` files or a mod's launcher go into Beacon's mods for the game, and get selected.
  - Files that replace the game's own (a fixed exe, a patch's data) are copied into the game's folder. Anything
    replaced is backed up under `backups/<game>/`.
  - Setup programs run in the game's own prefix, where the registry already points at the game.
  - Built in: Edain 4.x and its launcher (RotWK), The Dwarf Holds and Return of Shadow plus its patch (BFME),
    Conquest Fixes, and the Armies of Exigo integrated-graphics fix. Add more as `moddb` sources in `sources.json`:
    `mod_for`, an optional `catalog_id`, and `install` set to `auto`, `library`, `game-folder` or `installer`.
- **Downloads menu.** The **Downloads** button under the game list (its badge counts what's running) lists every
  download, whether game, launcher, mod or Beacon update, with a live progress bar. Downloads run in the background,
  so you can keep using Beacon.
  - **Pause** stops a download and keeps the partial file; **Resume** carries on from there.
  - A failed download gets **Try again**. A download whose link changed waits for **Use what Beacon found**.
  - **Kept downloads** shows the space taken by partial downloads and kept setups. **Clear kept downloads** frees it.
- **Reinstall downloads it again.** Game settings → Uninstall or reinstall works as in public Beacon. In this build,
  **Reinstall** for a game Beacon can download (BFME, BFME II, RotWK, Rise of Legends) deletes its files, keeps saves
  and settings, and downloads it again straight away, with progress in Downloads.
- **.7z and .rar too.** Launchers and game archives can be .zip, .7z or .rar, both for downloads and for "Use a
  launcher you downloaded". They're extracted with bsdtar, 7-Zip, or unrar/unar for RAR files that the others can't
  open.
- **Updates itself.** Launcher settings → About → **Check for updates** (also checked on start-up, or run
  `beacon self-update`) looks at the newest release of the *updates repo*. That's a public GitHub repo, no login,
  `Dragongamer9/beacon-private` unless you change it in Launcher settings → Downloads or with
  `beacon sources repo owner/repo`. The download is checked against the release's `SHA256SUMS` before anything is
  replaced. How it installs depends on how Beacon was installed:
  - AppImage: the file is replaced, and Beacon restarts.
  - tar.gz, or any folder Beacon can write to: `beacon`/`beacon-app` are replaced, and Beacon restarts.
  - .deb/.rpm: installed with `pkexec` (asks for your password). On Bazzite and other atomic systems, the package
    is downloaded for `rpm-ostree install`.
  - Windows installer or portable zip: the exes are swapped in place, and Beacon restarts.
  - Windows MSI in Program Files: the new MSI runs with the UAC prompt.
  - To ship an update, bump the version in `Cargo.toml` and `src-tauri/tauri.conf.json` and push to `main`. CI
    builds the release.
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
  - `drive-file`: one public Google Drive file (its link or id).
  - `moddb`: a ModDB mod, game or download page. With a downloads list, `file` picks the newest download whose slug
    matches, and `exclude` skips some (demos are always skipped).
  - `direct`: a plain https URL.
- **Fixes without rebuilding.** Beacon fetches `sources.json` from the updates repo (public, no login), at most
  every 6 hours and on demand. The
  higher `version` wins over the copy built in. So when a site changes in a way the scanner can't follow, edit
  `sources.json` here, bump `version`, and push. Every install of this build picks it up.
- **Check sources** (on start-up, in Launcher settings, or `beacon sources check`) looks up every source and touches
  its file. A broken link shows up before you need it.

## Setting it up

1. **Updates repo:** Beacon reads its updates and `sources.json` from a public repo, `Dragongamer9/beacon-private` by
   default, so that repo has to be public for updates to work. To use another repo, set it in Launcher settings →
   Downloads → "Updates come from", or run `beacon sources repo owner/repo`. GitHub allows about 60 checks an hour
   without a login, which is plenty.
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
beacon get rol                   # Rise of Legends from its Drive zip: the game, or its launcher to run
beacon mod-get rotwk edain [--yes]   # a catalog mod: shows what it found, --yes downloads and installs it
beacon mod-get bfme1 https://www.moddb.com/mods/… --yes   # any ModDB link
beacon self-update [--check]     # update Beacon itself from the updates repo's newest release
beacon sources                   # list version, sources, updates repo, games folder
beacon sources check             # look every source up now
beacon sources update            # fetch sources.json from the updates repo now
beacon sources repo <owner/repo> # the public repo updates come from
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
