# Beacon

A Linux launcher for Lord of the Rings games. The shelf has two sections:

- **Battle for Middle-earth:** BFME, BFME II, The Rise of the Witch-king, and the fan-made **Age of the Ring Standalone**.
- **Lord of the Rings games:** **The Lord of the Rings: Conquest** (Pandemic, 2009). More games can join this section. It is built for controllers, so it works on handhelds, on a TV, and inside Hearth's Game Mode.

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
