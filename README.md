# Beacon

A Linux launcher for Lord of the Rings games. The shelf has two sections:

- **Battle for Middle-earth:** BFME, BFME II, The Rise of the Witch-king, and the fan-made **Age of the Ring Standalone**.
- **Lord of the Rings games:** **The Lord of the Rings: Conquest** (Pandemic, 2009).
- **Other Games:** **Universe at War: Earth Assault**, **Armies of Exigo**, **Rise of Nations: Rise of Legends** and **Star Wars: Galactic Battlegrounds Saga**.

The shelf scrolls sideways and follows your selection. New games are added in `crates/beacon-core/src/games.rs`. It is built for controllers, so it works on handhelds, on a TV, and inside Hearth's Game Mode.

Beacon runs the games directly. It does not wrap the Windows All-in-One launcher. Each game gets its own Wine/Proton prefix, holding the registry keys the original installer used to write. Beacon also sets your resolution and manages mods.

> Beacon doesn't include or download the games. Use your own install files and your own CD key.

## What it does

- **Finds your installs.** It scans `~/Games`, Wine and Bottles prefixes, Steam compatdata, Beacon's own prefixes and removable drives for `lotrbfme.exe`, `lotrbfme2.exe` and `lotrbfme2ep1.exe`, and tells AotR Standalone apart from RotWK.
- **Finds a runner.** In order of preference: GE-Proton, then any Proton, then umu-launcher's UMU-Proton, then system Wine or Lutris Wine. You can override this per game or globally. Proton builds run through `umu-run` when it's installed, so they get the Steam Linux Runtime without Steam.
- **Builds one prefix per game** in `~/.local/share/beacon/prefixes/<game>/pfx`. On first launch, and whenever the folder, CD key or runner changes, it imports the EA registry keys with Wine's `regedit`:
  - `InstallPath`, `Language`, `UserDataLeafName` and your `ergc` CD key.
  - The keys go into both the native and `WOW6432Node` views.
  - RotWK's prefix also gets BFME II's keys, because the expansion reads the base game.
- **Sets the resolution.** It writes `Resolution = W H` into the game's `Options.ini` before each launch, so widescreen and handheld resolutions work.
- **Handles mods** (see below), including mods that ship their own Windows launcher.
- **Runs patches and installers inside a game's prefix,** so community patches and mod installers see the game.
- **Logs** each game to `~/.local/share/beacon/logs/<game>.log`.

## Age of the Ring Standalone

Age of the Ring is a fan remake built on RotWK's engine. Since 9.0 it ships as a standalone game: the installer bundles both base games, so you don't need RotWK or a CD key. Beacon gives it its own tile and its own prefix.

**Installing.** On the AotR tile, press *Install Age of the Ring*:
1. **Get the installer.** This opens the ModDB page. Download `AotR_Standalone_*_Installer.zip` (about 14 GB) and extract it.
2. **Run the installer.** Pick its `.exe`. It runs inside AotR's prefix and can sit silently for 15–20 minutes with no progress bar.
3. When it finishes, Beacon rescans and finds `…/AgeoftheRing/aotr/lotrbfme2ep1.exe` by itself.

If AotR is already installed somewhere, for example in a Lutris prefix, use *Already installed? Locate it* or *Find games*. You can point at either the `AgeoftheRing` folder or its `aotr` folder.

**How Beacon runs it.**
- **The game exe directly.** AotR's own launcher handles updates, but it may not work under Wine.
- **Registry keys.** The launcher normally writes them only while it runs, so Beacon writes the equivalent: the BFME II and RotWK keys point at the bundled copies, and the user-data folder is `Age of the Ring`.
- **Settings and saves.** They go to `AppData/Roaming/Age of the Ring/`, which is also where Beacon writes the resolution.
- **Detection.** AotR's exe has the same name as RotWK's. The scanner tells them apart by the `AgeoftheRing`/`aotr` folders, and it ignores the bundled base-game copies, so they never replace your real RotWK install.

**Updating.** Game settings → *Open the AotR Launcher for updates* runs AotR's launcher in the prefix. If it won't start under Wine, download the new Standalone installer and run it the same way.

**If it crashes on start:**
- AotR needs the Visual C++ 2019 runtime and DirectX 9. Recent GE-Proton covers these.
- With plain Wine, run `WINEPREFIX=~/.local/share/beacon/prefixes/aotr/pfx winetricks vcrun2019 d3dx9`.
- Or try another Proton build in Game settings → Runner.

## The Lord of the Rings: Conquest

Conquest isn't sold anywhere now, so Beacon works with your own disc or setup files.

**Getting it in.** On the Conquest tile, press *Install or locate Conquest*:
- **Run the game's installer.** Pick the `setup.exe` from your disc or files. It installs into Conquest's own prefix and writes its own registry entries, which is why Beacon doesn't add any for Conquest. Beacon rescans when the installer closes.
- **Already installed?** Point Beacon at the folder holding `Conquest.exe`. *Find games* also detects it, as long as a nearby folder name says Conquest or LOTR (`Conquest.exe` is too generic a name to trust on its own).

**Running it.**
- Beacon starts `Conquest.exe` in its prefix with your chosen runner.
- Resolution, window mode and controls are set in Conquest's own options menu. The BFME-only settings (Options.ini resolution, `-win`, `-mod`, CD key) don't apply to it.

**Online with MordorWide.** MordorWide re-implements EA's old Conquest servers. Game settings → *Play online with MordorWide* opens their guide. In short:
1. Patch the game to **1.1**.
2. Patch `Conquest.exe` the way their guide describes.
3. Create an EA Nation account in-game, using a CD key ending in `-MORDORWIDE`.

Their exe patch turns off SSL checking on the login, so use a password you don't use anywhere else. You can run patches and installers with *Run a patch or installer in this game's prefix*.

**Mods.** Conquest mods such as *Conquest Fixes* install into the game folder rather than loading with `-mod`. Run their installer in the prefix, or add their launcher in Mods. The catalog lists MordorWide and Conquest Fixes.

## Other Games

These all work the same way as Conquest:
- Each game gets its own prefix.
- "Install or locate" runs the game's own installer inside that prefix, or points Beacon at an existing folder.
- Resolution and controls are set in each game's own options menu.
- Two extra launch options sit under Game settings → Launch:

| Option | What it does | Default |
|---|---|---|
| **Virtual desktop** | Runs the game inside a fixed-size Wine desktop (`explorer /desktop=Beacon,WxH`). This fixes click offsets and resolution switching in older games. *Desktop size* sets it (default 1280x1024); set the game's own resolution to that size or smaller. | On for SWGB |
| **Launch through Steam** | For copies in a Steam library (UaW, SWGB): Steam launches the game with its own Proton setup, and Beacon's prefix isn't used. | Off |

| Game | Exe Beacon runs | Known fixes |
|---|---|---|
| **Universe at War: Earth Assault** (Petroglyph, 2007; delisted 2014) | `UAWEA.exe` | Games for Windows Live is gone and blocks start-up. Put the community **xliveless** `xlive.dll` in the game folder; Beacon warns until it's there. Only campaign and skirmish work. The DX10 renderer crashes under Wine, and Beacon sets `DX10Enabled` to 0 before each launch. The first launch may still crash once while the game creates that setting. |
| **Armies of Exigo** (Black Hole Entertainment, 2004) | `Exigo.exe` (in `Exigo Files`) | On integrated graphics the game refuses to start ("video card does not meet minimum requirements"). The catalog links a community replacement exe for this. |
| **Rise of Nations: Rise of Legends** (Big Huge Games, 2006) | `RiseOfLegends.exe` | Lutris reports a missing cursor at times and laggy music under Wine. Rise of Legends Heaven has the official patches. |
| **Star Wars: Galactic Battlegrounds Saga** (LucasArts, 2001; on Steam) | `battlegrounds_x1.exe` (Clone Campaigns), falling back to `Battlegrounds.exe` | Steam's `player.exe` launcher doesn't work under Proton, so Beacon skips it. Virtual desktop is on by default to fix fullscreen click offset. A community-patched exe (linked in the catalog) fixes a crash after about 20 minutes and stuck arrow keys. |

Of these, only Galactic Battlegrounds Saga is still sold. For the others, use your own discs or installers.

### Community versions

Each Other Game is meant to be played on its community version. Game settings opens with a **Community version** section: a **Version** picker (for games that have editions) and numbered setup steps.

Each step works the same way:
- **Page** opens the download page.
- The main action (**Add**, or **Run** for installers) takes what you downloaded:
  - an archive (`.zip`, `.7z`, `.rar`…), extracted with 7-Zip, bsdtar or unzip, whichever is installed;
  - a folder you already extracted (**Folder**);
  - a single file;
  - or an installer, which runs in the game's prefix.
- Beacon merges files into the game folder the way Windows would, ignoring case, so `DATA` lands in `Data`.
- Any file it replaces is first copied to `~/.local/share/beacon/backups/<game>/<time>/`.
- Finished steps get a tick.

| Game | Community version | Setup steps |
|---|---|---|
| **Galactic Battlegrounds** | **Expanding Fronts** (1.5.5, Sept 2025): new factions, campaign and balance, with online play through Voobly. It runs from its own `battlegrounds_x2.exe`. | Add the FULL Expanding Fronts zip; Beacon merges its `GAME` folder into your `Game` folder. |
| **Universe at War** | **Community Patch** (fan-voted fixes and balance), or **Human Resistance Reborn** (adds a Human faction). Both load with the engine's `MODPATH=Mods\…` switch, so the base game stays untouched. | 1. Official Patch 3 (Steam copies already have it). 2. Community Patch, added under `Mods\CommunityPatch`. 3. Human Resistance Reborn (optional), added under `Mods\HumanResistanceReborn`. 4. xliveless `xlive.dll`. |
| **Armies of Exigo** | **Patch 1.4**, the version the community plays. | 1. Patch 1.4 installer. 2. Integrated graphics fix, if needed. 3. Community forum. |
| **Rise of Legends** | Latest official patch plus the community crash fix. The **User Patch Project** is still in progress on the RoL Discord. | 1. Official patch installer. 2. Long-game crash fix. 3. User Patch Project page. |

The **Version** picker defaults to the community edition whenever one is installed; you can switch back to the original any time. Until a community version is set up, the game shows a note on its page.

With **Launch through Steam** on, Steam starts its own `player.exe` and the Version picker has no effect. Leave it off to play Expanding Fronts.

## Mods

The **Mods** sheet (Y on a controller, M on a keyboard) lists everything Beacon knows about for the selected game. Picking one makes it that game's default.

Beacon handles two kinds of mod:

| Kind | Examples | How Beacon starts it |
|---|---|---|
| `mod` | a `.big` file, or a folder of `.big` files | the game exe with `-mod "<path>"` |
| `launcher` | Edain's launcher, other mod launchers | that exe, inside the game's prefix |

There are three ways to add mods:

1. **Drop them in the library** at `~/.local/share/beacon/mods/<bfme1|bfme2|rotwk>/`. Each mod is a folder or a single `.big` file.
   - To control the details, add a `beacon-mod.json`:
     ```json
     { "name": "My Mod", "kind": "mod", "entry": "data", "args": ["-noshellmap"] }
     ```
2. **Add a mod from anywhere** with "Add mod file" or "Add mod folder".
3. **Use "Find mods on this PC".** It searches `~/Games`, `~/Downloads`, `~/Mods`, `~/.wine` and the folders around your games for the known mods below, and links any it finds.

The **Mod catalog** lists the major mods and community patches for all three games:

- **AotR:** the Standalone installer.
- **Conquest:** MordorWide (online), Conquest Fixes.
- **Other Games:**
  - UaW: xliveless.
  - Exigo: integrated graphics fix, ModDB downloads.
  - RoL: Rise of Legends Heaven.
  - SWGB: Steam store page, crash and stuck-key fix.
- **RotWK:** Edain, Age of the Ring 8.x (the older mod version), The Dwarf Holds, Elven Alliance, Patch 2.02.
- **BFME II:** Patch 2.22, Divide and Conquer, Helm's Deep: Last Hope.
- **BFME:** Patch 1.08, Middle-earth Extended Edition, Return of Shadow, Shadow and Flame, King of the West, Legends of the 3rd Age.

In the catalog:

- **Installed** mods are marked, and **Use** selects one.
- **Get** opens the mod's ModDB page. Beacon doesn't redistribute mods.
- Patches and mods that come as an installer `.exe` go through *Game settings → Run a patch or installer in this game's prefix*.

To add more catalog entries, edit `crates/beacon-core/src/catalog.rs`.

## Controls

| Controller | Keyboard | Action |
|---|---|---|
| D-pad / left stick | Arrows | Move |
| A | Enter | Select |
| B | Esc | Back |
| X | S | Game settings |
| Y | M | Mods |
| LB / RB | Q / E | Switch game |
| Menu (☰) | | Play |
| View | | Launcher settings |

On rows with ◂ ▸ (runner, resolution), use left and right to change the value.

## Install

Download a package from [Releases](https://github.com/Dragongamer9/beacon/releases). Every package has Beacon's window (`beacon-app`) and the `beacon` command-line tool. You need WebKitGTK 4.1, which most desktops already have, and Wine or Proton to run the games.

| Package | Latest download |
|---|---|
| AppImage (any distro) | [Beacon-x86_64.AppImage](https://github.com/Dragongamer9/beacon/releases/latest/download/Beacon-x86_64.AppImage) |
| Debian, Ubuntu, Mint, Pop!_OS | [beacon_amd64.deb](https://github.com/Dragongamer9/beacon/releases/latest/download/beacon_amd64.deb) |
| Fedora, Bazzite, openSUSE | [beacon.x86_64.rpm](https://github.com/Dragongamer9/beacon/releases/latest/download/beacon.x86_64.rpm) |
| Arch, CachyOS, anything else | [beacon-x86_64-linux.tar.gz](https://github.com/Dragongamer9/beacon/releases/latest/download/beacon-x86_64-linux.tar.gz) |

Each release also has versioned copies (`Beacon-<version>-x86_64.AppImage`, `beacon_<version>_amd64.deb`, `beacon-<version>-1.x86_64.rpm`) and a `SHA256SUMS` file.

**AppImage.** Make it executable and run it:
```sh
chmod +x Beacon-x86_64.AppImage
./Beacon-x86_64.AppImage
```
For the CLI, run `./Beacon-x86_64.AppImage --cli status`, or link it under the name `beacon`; the AppImage runs the CLI when it's started as `beacon`. Linking it as `beacon-app` too lets Hearth find both:
```sh
ln -s "$PWD/Beacon-x86_64.AppImage" ~/.local/bin/beacon
ln -s "$PWD/Beacon-x86_64.AppImage" ~/.local/bin/beacon-app
```

**.deb:**
```sh
sudo apt install ./beacon_*_amd64.deb
```

**.rpm:**
```sh
sudo dnf install ./beacon-*.x86_64.rpm
```
On Bazzite and other atomic Fedoras, use `rpm-ostree install ./beacon-*.x86_64.rpm`, or use the AppImage.

**tar.gz.** It holds `bin/beacon`, `bin/beacon-app`, the desktop file and the icons. Extract it into `~/.local` (or `/usr/local` with sudo):
```sh
tar -xzf beacon-x86_64-linux.tar.gz -C ~/.local
```
On Arch and CachyOS, install `webkit2gtk-4.1` first. `~/.local/bin` must be on your `PATH`.

**CLI.** The .deb and .rpm install it as `/usr/bin/beacon`, and the tar.gz as `bin/beacon`. Hearth uses it (`beacon status --json`, `beacon launch <game> --wait`). See [Command line](#command-line).

## Building

You need Rust (stable), plus the Tauri 2 system packages.

On Fedora, Bazzite or Anatase:
```sh
sudo dnf install webkit2gtk4.1-devel openssl-devel libappindicator-gtk3-devel librsvg2-devel
```
On Debian or Ubuntu:
```sh
sudo apt install libwebkit2gtk-4.1-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Then build:
```sh
./scripts/fetch-fonts.sh            # once; UI fonts (Cinzel, Alegreya Sans)
cargo test -p beacon-core            # core tests
cargo build --release                # CLI → target/release/beacon
cargo install tauri-cli --version '^2'
cd src-tauri && cargo tauri build    # app + AppImage/deb/rpm in target/release/bundle
```

`cargo tauri build` builds the `beacon` CLI first (`beforeBuildCommand`) and puts it in each package as `/usr/bin/beacon`. The Build workflow (`.github/workflows/build.yml`) does all of this on every push and pull request, and makes a release for each `v*` tag.

During development, `cargo tauri dev` runs the app. You can also open `ui/index.html` in a browser: without Tauri, the UI runs against a built-in mock.

## Command line

The `beacon` CLI does everything the app does. It's useful over SSH, in scripts, or for calling from Hearth.

```sh
beacon scan                          # find games
beacon status [--json]               # what's ready and what's missing
beacon set-key bfme2                 # prompts for your CD key
beacon resolution rotwk 1280x800
beacon catalog rotwk                 # known mods + links, what's installed
beacon find-mods                     # link known mods already on this PC
beacon add-mod rotwk ~/Mods/AotR --name "Age of the Ring"
beacon select-mod rotwk rotwk:age-of-the-ring
beacon run rotwk ~/Downloads/patch_installer.exe
beacon launch rotwk [--mod <id> | --vanilla] [--dry-run] [--wait]
beacon run aotr ~/Downloads/AotR/AotR_Standalone_Installer.exe   # install AotR, then:
beacon launch aotr
beacon run conquest /run/media/$USER/CONQUEST/setup.exe   # install Conquest from disc
beacon launch conquest
beacon virtual-desktop swgb on && beacon resolution swgb 1280x1024
beacon via-steam swgb on                 # let Steam launch your Steam copy
beacon community swgb                    # community version: editions + steps
beacon add-files swgb expanding-fronts ~/Downloads/EF_1.5.5_Full.zip
beacon add-files uaw community-patch ~/Downloads/Community_Patch_2.0.7z
beacon edition uaw human-resistance      # or: auto
```

## Hearth integration

- `beacon-app --fullscreen` (or `BEACON_FULLSCREEN=1`) opens it full-screen for Game Mode.
- Beacon minimizes while a game runs and comes back when the game exits.
- `beacon status --json` and `beacon launch <game>` let Hearth list the three games and start them directly.
- `BEACON_CONFIG` and `BEACON_DATA_DIR` move the config file and data folder.

## Good to know

- **In-game controls.** The BFME games have no controller support. On a handheld, map the controller to mouse and keyboard with Steam Input or a Handheld Daemon (HHD) profile.
- **LAN** works through Wine's normal networking. Online play is out of scope for now.
- **The registry key set is the minimum** the games are known to read. If a particular game version wants more, there's one place to add it: `registry.rs`.
- **Per-game environment variables** (for example `DXVK_HUD=fps` or `WINEDEBUG`) go in the `env` map of that game's entry in `~/.config/beacon/config.json`.
