# Beacon

A launcher for Lord of the Rings games, for Linux and Windows. The shelf has three sections:

- **Battle for Middle-earth:** BFME, BFME II, The Rise of the Witch-king, and the fan-made **Age of the Ring Standalone**.
- **Lord of the Rings games:** **The Lord of the Rings: Conquest** (Pandemic, 2009).
- **Other Games:** **Universe at War: Earth Assault**, **Armies of Exigo**, **Rise of Nations: Rise of Legends** and **Star Wars: Galactic Battlegrounds Saga**.

The shelf scrolls sideways and follows your selection. New games are added in `crates/beacon-core/src/games.rs`. It is built for controllers, so it works on handhelds, on a TV, and inside Hearth's Game Mode.

Beacon runs the games directly. It does not wrap the Windows All-in-One launcher (it finds the games that launcher installed, see [below](#bfme-all-in-one-launcher)). On Linux each game gets its own Wine/Proton prefix, holding the registry keys the original installer used to write; on Windows the games run as they are (see [Windows](#windows)). Beacon also sets your resolution and manages mods, including fan content from the [BFME workshop](#workshop).

> Beacon doesn't include or download the games. Use your own discs, disc images or install files (see [Install from your own copy](#install-from-your-own-copy)) and your own CD key.

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

## Windows

Beacon also runs on Windows 10 and 11 (`beacon-app.exe`, with the `beacon.exe` command-line tool next to it). There's
no Wine there, so:

- **The games run directly**: the game's exe with its own folder as the working directory, `-win` / `-mod` / the
  community editions' `MODPATH=` with ordinary Windows paths. Mods, the catalog, the workshop and community versions
  work the same way.
- **No prefixes and no runners.** The runner list is a single "Windows (native)" entry (`kind: "native"`); `setup`
  has nothing to do (`needs_setup` is `false`), the virtual desktop doesn't apply, and "Launch through Steam" opens
  `steam://rungameid/…`.
- **Finding games** uses what the installers wrote in the registry (the EA keys of the BFME games, which the BFME
  All-in-One launcher writes too; `App Paths`; the `Uninstall` entries of the disc installers, EA/Origin, Steam and
  GOG) plus the usual folders: `Program Files (x86)\EA Games`, `Electronic Arts`, `Origin Games`, `SEGA`,
  `LucasArts`, `Microsoft Games`, Steam libraries, `%USERPROFILE%\Games` and the All-in-One launcher's
  `<drive>:\BFME1`, `BFME2`, `RotWK`.
- **Resolution** goes into `%APPDATA%\<the game's user data folder>\Options.ini`, the folder named by the game's
  `UserDataLeafName` registry value (the All-in-One launcher's names differ from EA's).
- **Installers that need administrator rights** (most disc installers) get the UAC prompt.
- **Settings and data:** `%APPDATA%\beacon\config.json` and `%LOCALAPPDATA%\beacon\` (mods, logs, downloads).
  `BEACON_CONFIG` and `BEACON_DATA_DIR` work as on Linux.
- Universe at War still needs xliveless's `xlive.dll` on Windows: Games for Windows Live is gone there too.

## Install from your own copy

Every game's page has **Install from your own copy** while the game isn't installed (`beacon install-from <game>
<path>` on the command line). Pick one of:

- **the installer** (`setup.exe`, AotR's installer…);
- **a disc image** (`.iso`): Beacon extracts it into `~/.local/share/beacon/tmp/install-<game>-<time>/` with
  `bsdtar` or `7z` (Windows: its built-in `tar.exe`, or 7-Zip), runs nothing through a shell, removes any links the
  image carries, and keeps the files until the next install for that game (installers often start helpers that keep
  reading them). Or mount the image yourself and pick its folder;
- **a disc's folder** (**Disc folder**): Beacon finds the setup program (`setup.exe` at the top, the one
  `autorun.inf` opens when it's a setup, or a `*setup*`/`*install*` exe two levels down).

The setup runs in the game's own prefix on Linux (directly on Windows); when it closes, Beacon scans for the game.
Multi-disc installs ask for the next disc as usual: mount or extract it and point the installer at it.

## BFME All-in-One launcher

The [BFME Foundation's All-in-One launcher](https://bfmeladder.com) installs BFME, BFME II and RotWK (Windows). Beacon
**never downloads the games itself**; it finds the copies the launcher installed and uses them.

**Get the All-in-One launcher** (on the BFME games' pages and in Launcher settings, or `beacon aio-launcher install`)
installs the launcher for you, and **Open the All-in-One launcher** (`beacon aio-launcher open`) starts it. You download
the games inside the launcher; Beacon picks them up by itself (when the launcher closes, when you come back to Beacon,
on `beacon status`, or with **Find games**). The bfmeladder.com link is still there.

- **Where it comes from:** the launcher has no separate installer or GitHub release: it's one self-contained exe that
  installs and updates itself. Beacon downloads it at the moment you ask from the same official URL the launcher's own
  updater uses (`https://arena-files.bfmeladder.com/application-builds/all-in-one-launcher-main`, from its source,
  `Data/Consts.cs`), https only (redirects too), at most 1 GB, and checks it is a Windows program and matches the MD5
  the Foundation publishes for the current build (`https://bfmeladder.com/api/applications/versionHash?name=all-in-one-launcher&version=main`;
  when that answer can't be read, the install says it wasn't compared). Beacon never ships or hosts the launcher.
- **Where it goes:** where the launcher installs itself, `%APPDATA%\BFME All In One Launcher\AllInOneLauncher.exe`. On
  Linux that's inside a Wine prefix of its own, `~/.local/share/beacon/prefixes/aio-launcher/pfx` (created with
  `wineboot` through your default runner, or the best one found; the exe carries its own .NET runtime, so nothing else
  is installed into the prefix). Games you install there with the launcher's default `C:` drive land in that prefix's
  `drive_c` (`BFME1`, `BFME2`, `RotWK`). On Windows the launcher asks for administrator rights when it starts (UAC).
- On its first start the launcher makes its own desktop/Start-menu shortcuts and uninstall entry, and from then on
  updates itself.

What Beacon reads (from the launcher's source):

- **Game folders:** `<drive>:\BFME1`, `<drive>:\BFME2`, `<drive>:\RotWK`, and the `InstallPath` the launcher writes
  under `HKLM\SOFTWARE\WOW6432Node\Electronic Arts\EA Games\The Battle for Middle-earth`,
  `…\Electronic Arts\Electronic Arts\The Battle for Middle-earth II` and
  `…\Electronic Arts\Electronic Arts\The Lord of the Rings, The Rise of the Witch-king`.
- **On Linux**, the same keys inside Wine prefixes: Beacon reads `system.reg` in its own prefixes, `~/.wine`, Bottles,
  `~/Games/<prefix>`, and any folder you give `beacon scan <folder>` (a prefix, or a folder of prefixes), and maps
  `C:\…` into the prefix. The launcher's own folders (`drive_c/BFME1`…) are found by the normal scan too.
- **The launcher's state:** `%APPDATA%\BFME Workshop\Config\active_patch_<n>.json` (the patch a game is synced to),
  `<game>\mod.txt` (its active mod), and its workshop mods in `<drive>:\BFME Workshop\Mods\<Name>-<id>`, which
  *Find mods on this PC* links into Beacon.

`beacon aio` lists the games it manages and whether the launcher is installed; `beacon aio-launcher status` shows the
launcher alone.

## Universe at War community launcher

The Universe at War community has a launcher that downloads the game. It's distributed only from
[a Google Drive folder](https://drive.google.com/drive/folders/167UET84ZWXrMd9zpxEYiQWTQZexCGZfT) with several versions,
no stable direct-download link and no checksums, so **Beacon never downloads it** (nor the game). On UaW's page and in
Launcher settings:

- **Get the UaW launcher** opens that folder in your browser. Download the launcher version you want, then choose it
  below. Games are downloaded inside it; Beacon finds them afterwards.
- **Use a launcher you downloaded** takes its `.exe` (copied) or the `.zip` it came in (extracted the same safe way as
  disc images: bsdtar or 7-Zip, no shell, links removed, every entry checked to stay inside). Beacon keeps it in
  `~/.local/share/beacon/tools/uaw-launcher/<name>/` (`%LOCALAPPDATA%\beacon\tools\…` on Windows) and remembers it
  for UaW. In a `.zip`, the exe with "launcher" in its name is picked first; uninstallers, runtime installers and
  crash reporters are skipped. It must be a Windows program (`MZ`).
- **Open the UaW launcher** runs it: on Linux in UaW's own Wine prefix (`prefixes/uaw/pfx`, created first with
  `wineboot` when needed) through UaW's runner, detached, logged to `logs/uaw-launcher.log`; on Windows directly
  (through the UAC prompt when it asks for administrator rights). When it closes, Beacon looks for `UAWEA.exe` in
  UaW's prefix (`drive_c`, and the folders its registry names) and the launcher's folder, then does a full scan if
  that finds nothing. `beacon status` also picks it up, and the normal scan always walks Beacon's prefixes.

Other games can get a community launcher the same way (one URL per game in `community_launcher.rs`).

## Workshop

The **Mods** sheet of BFME, BFME II and RotWK has **Workshop**: fan-made mods, maps, add-ons and patches from the BFME
Foundation's workshop, through the same public API the All-in-One launcher uses (`bfmeladder.com/api/workshop/query`
and `/workshop/download`). `beacon workshop <game>` and `beacon workshop-install <game> <id>` do the same.

- **Fan content only.** Base game packages (`original-…`), EA's official patches (`official-…`) and snapshots of
  someone's install are never listed and are refused by `workshop-install`. Inside a package, EA's executables,
  `game.dat` and logo movies are skipped, and a package carrying the base game's own archives is refused.
- **Downloads** are https only (redirects too), at most 4 GB per file and 8 GB per package, checked against the
  workshop's MD5 and size, with every path checked (no `..`, no absolute paths or drive letters). They're written as
  new files into a fresh folder, which then becomes `~/.local/share/beacon/mods/<game>/<Name>/` with a
  `beacon-mod.json`. Files for other languages than Beacon's `language` setting are skipped.
- After that it's a mod like any other: pick it in Mods, and the game starts with `-mod`. Map packs land under the
  mod's `maps\` folder.

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

## Your own games (Other Games)

Besides the games it knows, Beacon runs **any Windows game or installer you add**: the last tile under Other Games
(*Add a game*) does it. This is Faugus Launcher's feature set, rebuilt inside Beacon (nothing of Faugus is bundled).

- **Add a Windows program** (.exe). Beacon takes its icon out of the exe and names it after the program (or its folder).
- **Install from an installer** (setup.exe or .msi). It runs in a new Wine prefix of its own; when it closes, Beacon
  lists the programs it installed (biggest first, uninstallers and redistributables left out) and you pick the game.
- **Per game**: title, program, launch arguments, working folder, icon, cover and background pictures; the runner
  (any of Beacon's: GE-Proton, UMU-Proton and other Proton builds, Steam's Proton, umu-launcher, Wine); its own
  prefix or one you choose; **GAMEID** and **STORE** for umu-launcher's protonfixes; DXVK and VKD3D-Proton on or off;
  MangoHud, GameMode and gamescope (with its options; skipped inside a gamescope session such as Hearth's Game Mode);
  Proton options (NVAPI/DLSS, Wayland, HDR, WoW64, FSR, SDL controllers, esync/fsync off, hide the NVIDIA GPU, large
  address aware, Proton log); environment variables.
- **Wine tools** for each prefix: winecfg, winetricks (umu's own when umu-launcher is there, else the system's with the
  runner's Wine), regedit, a command prompt, task manager, explorer, the uninstaller, control panel, and *stop
  everything in the prefix*; run another program (an installer, a patch, an .msi) in it.
- **Play time and last played** are kept for every launch Beacon waits for (`<data>/playtime.json`; Beacon's own games
  too).
- **Library**: export everything (with play time) to a file and import it on another PC; import Faugus Launcher's
  library (`~/.local/share/faugus-launcher/games.json`, or `~/.config/faugus-launcher/` from older versions, or its
  Flatpak) with its icons, covers, banners and play time; remove a game, optionally deleting its Wine prefix (only
  when it is one, isn't shared with another game and isn't a top-level folder).
- **Wine runners and DXVK**: Proton brings DXVK and VKD3D-Proton itself. For plain Wine runners Beacon copies the newest
  DXVK / VKD3D-Proton you got under Compatibility tools into the prefix and prefers them (`WINEDLLOVERRIDES`).

The list is `custom-games.json` next to `config.json`; Beacon's window and the `beacon` tool read it again before every
change, so neither undoes the other's. In Hearth these games show in the Library under Beacon's Other Games.

## Compatibility tools

*Launcher → Compatibility tools* (or *Get more runners* in a game's settings) downloads, updates and removes runners,
like ProtonPlus does, for **Beacon and the other launchers on this PC**: Steam (and its Flatpak), Lutris, Heroic and
Bottles (native or Flatpak; only the ones that are installed are offered).

| Tool | Kind | Goes to |
|---|---|---|
| GE-Proton, UMU-Proton, Proton-CachyOS, Proton-EM, Proton-Sarek | Proton | Beacon, Steam, Lutris, Heroic, Bottles |
| umu-launcher (zipapp) | runs Proton outside Steam | Beacon |
| Wine (Kron4ek), Wine Staging (Kron4ek), Wine-GE (archived) | Wine | Beacon, Lutris, Heroic, Bottles |
| DXVK, DXVK-async (archived) | Direct3D 8–11 | Beacon (Wine runners), Lutris, Bottles |
| VKD3D-Proton | Direct3D 12 | Beacon (Wine runners), Lutris, Bottles |

- Only each project's **own GitHub releases** are used. A download is checked against the **SHA-256/SHA-512 checksum**
  its release publishes (GE-Proton, UMU-Proton, Proton-CachyOS, Kron4ek's Wine, …) and thrown away on a mismatch;
  releases that publish none say so.
- Each tool shows the version(s) installed in the chosen launcher and the newest release; any release can be installed
  or removed, with download progress.
- **Latest**: one `<Tool>-Latest` copy that Beacon keeps on the newest release (*Update “Latest” copies now*, or
  automatically when Beacon opens). For Steam, its compatibilitytool.vdf gets its own name ("GE-Proton Latest").
- Everything installed for Beacon (`<data>/compat/`) is a runner straight away, in every game's runner choice, and
  umu-launcher installed this way is used when there's none on PATH.

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

Download a package from [Releases](https://github.com/Dragongamer9/beacon/releases). Every package has Beacon's window (`beacon-app`) and the `beacon` command-line tool. On Linux you need WebKitGTK 4.1, which most desktops already have, and Wine or Proton to run the games. On Windows you need WebView2 (Windows 11 has it; the installers fetch it on Windows 10).

| Package | Latest download |
|---|---|
| AppImage (any distro) | [Beacon-x86_64.AppImage](https://github.com/Dragongamer9/beacon/releases/latest/download/Beacon-x86_64.AppImage) |
| Debian, Ubuntu, Mint, Pop!_OS | [beacon_amd64.deb](https://github.com/Dragongamer9/beacon/releases/latest/download/beacon_amd64.deb) |
| Fedora, Bazzite, openSUSE | [beacon.x86_64.rpm](https://github.com/Dragongamer9/beacon/releases/latest/download/beacon.x86_64.rpm) |
| Arch, CachyOS, anything else | [beacon-x86_64-linux.tar.gz](https://github.com/Dragongamer9/beacon/releases/latest/download/beacon-x86_64-linux.tar.gz) |
| Windows (installer) | [Beacon-x64-setup.exe](https://github.com/Dragongamer9/beacon/releases/latest/download/Beacon-x64-setup.exe) |
| Windows (MSI) | [Beacon-x64.msi](https://github.com/Dragongamer9/beacon/releases/latest/download/Beacon-x64.msi) |
| Windows (portable) | [beacon-windows-x64.zip](https://github.com/Dragongamer9/beacon/releases/latest/download/beacon-windows-x64.zip) |

Each release also has versioned copies (`Beacon-<version>-x86_64.AppImage`, `beacon_<version>_amd64.deb`, `beacon-<version>-1.x86_64.rpm`, `Beacon-<version>-x64-setup.exe`, `Beacon-<version>-x64.msi`) and a `SHA256SUMS` file.

**Windows.** The installer (per user, no administrator rights needed) and the MSI put `beacon-app.exe` and
`beacon.exe` in the same folder. The portable zip holds just those two exes. Hearth's Windows edition looks for exactly
these names.

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

`cargo tauri build` builds the `beacon` CLI first (`beforeBuildCommand`) and puts it in each package as `/usr/bin/beacon`.

On Windows (Rust stable with the MSVC toolchain, Node for the Tauri CLI): `cargo build --release -p beacon-cli` gives
`target\release\beacon.exe`, and `cargo tauri build --bundles nsis,msi` (in `src-tauri`) the installers, with
`windows-bundle.conf.json` (passed with `--config` at packaging time) adding `beacon.exe` next to `beacon-app.exe`.

The Build workflow (`.github/workflows/build.yml`) does all of this on every push and pull request: `build-linux`
(Ubuntu 22.04) and `build-windows` (windows-latest) each test and package; the `release` job makes one GitHub Release
with both platforms' packages for a new version on main or a `v*` tag.

During development, `cargo tauri dev` runs the app. You can also open `ui/index.html` in a browser: without Tauri, the UI runs against a built-in mock.

## Command line

The `beacon` CLI does everything the app does. It's useful over SSH, in scripts, or for calling from Hearth.

```sh
beacon scan                          # find games
beacon status [--json]               # what's ready and what's missing
beacon set-key bfme2                 # prompts for your CD key (or reads it from stdin)
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
beacon set-runner rotwk auto             # back to the automatic runner
beacon run aotr --tool                   # AotR's own launcher, for updates
beacon log rotwk --lines 50              # the end of a game's log
beacon install-from bfme2 ~/Downloads/BFME2.iso          # install from your own disc image
beacon install-from conquest /run/media/$USER/CONQUEST   # …or the disc's folder, or its setup.exe
beacon workshop rotwk --search edain     # fan content from the BFME workshop
beacon workshop-install rotwk <id> --select
beacon aio                               # games the BFME All-in-One launcher installed
beacon aio-launcher install              # get the All-in-One launcher (from the BFME Foundation; never the games)
beacon aio-launcher open                 # start it, download the games there; `beacon status` finds them
beacon aio-launcher get bfme1            # install a game through it: opens it, waits while you install the game there,
                                         # then closes it (Linux) and adds the game (since 0.0.8)
beacon community-launcher uaw status     # UaW's community launcher: where to get it, which one is chosen
beacon community-launcher uaw set ~/Downloads/UaW_Launcher.zip   # a launcher you downloaded (.exe or .zip)
beacon community-launcher uaw open       # start it (Linux: in UaW's prefix); `beacon status` finds the game

# Your own games (Other Games)
beacon custom add ~/Games/Witcher3/bin/x64/witcher3.exe --title "The Witcher 3"
beacon custom install ~/Downloads/setup_game.exe     # runs it in a new prefix, then lists what it installed
beacon custom set the-witcher-3 exe ~/.local/share/beacon/prefixes/custom/the-witcher-3/pfx/drive_c/…/game.exe
beacon custom set the-witcher-3 runner proton:GE-Proton10-25     # or: auto
beacon custom set the-witcher-3 umu-id umu-292030 && beacon custom set the-witcher-3 store gog
beacon custom set the-witcher-3 flag +nvapi          # Proton options: +key / -key
beacon custom set the-witcher-3 env DXVK_HUD=fps     # KEY= removes it
beacon custom set the-witcher-3 gamescope on && beacon custom set the-witcher-3 gamescope-args "-W 1920 -H 1080 -f"
beacon custom set the-witcher-3 args -- -skipintro
beacon custom tool the-witcher-3 winetricks vcrun2019   # also winecfg, regedit, cmd, taskmgr, explorer, uninstaller, control, kill
beacon custom run the-witcher-3 ~/Downloads/patch.exe
beacon custom launch the-witcher-3 [--dry-run] [--wait]   # --wait records the play time
beacon custom list                       # with play time, runner, prefix, issues
beacon custom export ~/beacon-games.json && beacon custom import ~/beacon-games.json
beacon custom import-faugus              # everything from Faugus Launcher
beacon custom remove the-witcher-3 [--delete-prefix]

# Compatibility tools (from the projects' GitHub releases, checksums checked)
beacon tools list [--target steam]       # beacon, steam, steam-flatpak, lutris(-flatpak), heroic(-flatpak), bottles(-flatpak)
beacon tools install ge-proton           # newest release, for Beacon
beacon tools install ge-proton --version latest --target steam   # GE-Proton-Latest in Steam, kept up to date
beacon tools install dxvk --target lutris
beacon tools remove ge-proton GE-Proton10-20 --target steam
beacon tools update                      # every "Latest" copy to its newest release
beacon tools auto-update on              # …each time Beacon's window opens
```

## For Hearth / scripting

Every command takes `--json` (before or after the command: `beacon --json mods rotwk` or `beacon mods rotwk --json`).
Without it the output is for people, as above. Hearth uses only the JSON forms, and these shapes stay stable: new
fields can appear, existing ones aren't renamed or removed. The code is `crates/beacon-core/src/report.rs`.

### Reading

Each prints one JSON document.

| Command | Prints |
|---|---|
| `status` | The list of every game's `GameStatus` (pretty-printed, as since 0.0.1) |
| `mods <game>` | `{game, selected, mods_dir, mods: [{id, name, game, kind, path, args, in_library}]}` |
| `catalog <game>` | `{game, items: [{id, name, game, kind, blurb, url, installed}]}` |
| `runners` | `{runners: [{id, name, kind, path}], best, default, umu}` |
| `community <game>` | `{game, chosen, editions: [{id, name, blurb, community, installed, active}], steps: [{id, title, detail, url, action, done}]}` |
| `paths` | `{config, data, prefixes, mods, logs, backups, games: {<game>: {prefix, log, mods}}}` |
| `log <game> [--lines N]` | `{game, path, exists, lines: [...]}`: the last N lines (default 200, at most 2000) |
| `launch <game> --dry-run` | `{ok, game, message, command: {program, args, env: [[k, v]], cwd, display}, setup_first}` |
| `workshop <game> [--search S] [--page N]` | `{game, page, source, items: [{id, name, version, author, description, kind, game, size, downloads, artwork_url, installed}], hidden}` (since 0.0.6; BFME, BFME II and RotWK only) |
| `aio` | `{launcher_url, games: [{game, dir, active_patch, active_mod}], appdata: [...], launcher}` (since 0.0.6; `launcher` since 0.0.7, as in `aio-launcher status`) |
| `aio-launcher status` | `{installed, path, prefix, version, by_beacon, can_install, runner, download_url, site, log}` (since 0.0.7) |
| `community-launcher <game> status` | `{game, url, path, missing, can_open, prefix, runner, tools_dir, log, game_installed}` (since 0.0.7; only `uaw` has one, other games are a `usage` error) |

- **`GameStatus`:** `id, family, title, short, path, installed, runner, runner_choice, has_cd_key, resolution, windowed,
  extra_args, prefix_ready, virtual_desktop, steam_appid, via_steam, edition, selected_mod, tool_launcher, ready,
  issues: [{level, message}]`. Since 0.0.6 also:
  - `sage`: a BFME game, where the resolution, windowed (`-win`) and `-mod` apply;
  - `wants_cd_key`: the game's registry carries a CD key;
  - `requires`: another game that must be set up too (RotWK: `bfme2`);
  - `virtual_desktop_choice` (`null` = the game's default) and `virtual_desktop_default`;
  - `needs_setup`: whether `setup` has work to do, `null` when it can't tell yet;
  - `log_file`, `has_log`;
  - `has_community`;
  - `store_url`: the game's Steam store page when it's still sold there, else `null`. Only Galactic Battlegrounds
    Saga has one (`https://store.steampowered.com/app/356500/`): Universe at War (Steam app 10430) was delisted in
    2014, and Armies of Exigo and Rise of Legends were never on Steam. `steam_appid` is separate: it's set for copies
    found in a Steam library.
  Since 0.0.7 also:
  - `community_launcher`: `{path, url}` for games with a community launcher (only Universe at War): `url` is where
    it's downloaded from (opened in the browser, never downloaded by Beacon), `path` the launcher exe chosen with
    `community-launcher <game> set` (`null` until then, or when it's gone). Left out for other games.
  The CD key itself is never printed, only `has_cd_key`.
- **Values:** `kind` (mods) is `mod` or `launcher`. `kind` (catalog) is `mod`, `patch` or `game`; `installed` is the
  id of the installed mod it matches, or `null`. `kind` (runners) is `umu`, `proton` or `wine`, and on Windows
  `native` (the single "Windows (native)" runner, id `native`, empty `path`). `action` (steps) is `link`,
  `installer`, `merge` or `mods`. `chosen` is the edition picked (`null`: automatic). `level` is `error` or `warn`.
  `kind` (workshop) is `patch`, `mod`, `enhancement` or `maps`; `installed` is the Beacon mod id it's installed as,
  or `null`; `size` is bytes; `hidden` counts the entries left out (base games, EA's official patches, snapshots,
  other games).
- **`aio-launcher status`:** `path` is the launcher's exe (`null` when not installed); `prefix` the Wine prefix it runs
  in on Linux (`null` on Windows); `version` the MD5 of the build Beacon installed (the launcher's own version id;
  `null` when Beacon didn't install it or it has updated itself since); `by_beacon` whether Beacon installed it;
  `can_install` whether `aio-launcher install` can run (Linux needs a runner); `runner` the runner id it uses on
  Linux; `download_url` where `install` downloads it from; `log` Beacon's log for it.
- **`community-launcher <game> status`:** `url` where it's downloaded from; `path` the chosen launcher exe (`null`
  when none is chosen or it's gone); `missing` one was chosen but its exe is gone; `can_open` whether `open` can run
  (a launcher is chosen and, on Linux, there's a runner); `prefix` the game's Wine prefix it runs in on Linux (`null`
  on Windows); `runner` the game's runner id on Linux; `tools_dir` where chosen launchers are kept; `log` Beacon's log
  for it; `game_installed` whether the game itself is found.
- `status` and `aio` also pick up BFME games the All-in-One launcher installed since the last look (its prefix on
  Linux, the registry on Windows) and save their folders, so Hearth needn't `scan` after the launcher. `status`
  (and `community-launcher <game> status`) does the same for a game downloaded by its community launcher (its prefix
  and the launcher's folder), once a launcher is chosen.

### Changing things

Long jobs print one progress line for each step while they work, then every command prints one result line. Each line
is one JSON object:

```json
{"event":"progress","pct":40,"text":"Looking in /home/me/Games"}
{"ok":true,"game":"rotwk","message":"Found 2 games","found":[{"game":"rotwk","dir":"/home/me/Games/RotWK"}]}
{"ok":false,"kind":"usage","error":"unknown game \"doom\" (use bfme1, bfme2, …)"}
```

`pct` is 0–100, or `null` when Beacon can't tell. `game` is `null` for commands that aren't about one game.

| Command | Long job | Result fields besides `ok`, `game`, `message` |
|---|---|---|
| `scan [folders]` | yes | `found: [{game, dir}]`, `installed: [<game>]` |
| `set-path <game> <folder or exe>` | | `path` |
| `set-key <game> [--clear]` | | `has_cd_key`. The key is read from **stdin**, one line, never from the command line |
| `set-runner <game\|default> <runner id\|auto>` | | `runner` (`null`: automatic), `default` |
| `resolution <game> <WxH\|clear>` | | `resolution` |
| `windowed <game> <on\|off>` | | `windowed` |
| `edition <game> <id\|auto>` | | `chosen`, `edition` (the name of the one that launches) |
| `add-files <game> <step> <path>` | yes | `step`, `done`, and `edition` (or `exit_code` for an installer step) |
| `virtual-desktop <game> <on\|off\|default>` | | `virtual_desktop`, `choice` |
| `via-steam <game> <on\|off>` | | `via_steam`, `steam_copy` |
| `args <game> -- <arguments>` | | `extra_args` (put `--` first, so arguments starting with `-` aren't read as Beacon's) |
| `add-mod <game> <path> [--name N] [--select]` | | `mod` (as in `mods`), `selected` |
| `find-mods [folders]` | yes | `added: [mod]` |
| `remove-mod <id>` | | `removed` |
| `select-mod <game> <id\|none>` | | `selected` |
| `setup <game> [--force]` | yes | `did_work`, `options_file` |
| `run <game> <exe>` / `run <game> --tool` | yes | `exit_code`, `found`, `installed` |
| `launch <game> [--wait]` | | `pid`, `log`, and `exit_code` with `--wait` |
| `install-from <game> <installer.exe \| image.iso \| folder>` | yes | `exit_code`, `setup`, `extracted` (where an image was extracted, or `null`), `found`, `installed` (since 0.0.6) |
| `workshop-install <game> <id> [--select]` | yes | `id`, `name`, `version`, `kind`, `dir`, `files`, `bytes`, `skipped`, `mod` (as in `mods`), `selected` (since 0.0.6) |
| `aio-launcher install` | yes | `path`, `prefix` (Linux; else `null`), `bytes`, `md5`, `verified` (the MD5 matched the published one), `launcher` (as in `aio-launcher status`) (since 0.0.7) |
| `aio-launcher open` | | `pid`, `log`, `path` (since 0.0.7). It starts detached; download the games there, then `status` finds them |
| `aio-launcher get <game>` | yes | `ready`, `dir`, `closed_launcher`, `installed_launcher` (since 0.0.8). Installs the launcher if needed, opens it and waits (up to 6 hours) until the launcher has synced that game (its `BFME Workshop\Config\active_patch_<n>` files), then closes it (Linux) and adds the game. The launcher downloads the game, never Beacon |
| `community-launcher <game> set <path>` | yes | `path` (Beacon's copy of the launcher exe), `community_launcher` (as in `community-launcher <game> status`) (since 0.0.7) |
| `community-launcher <game> open` | yes | `pid`, `log`, `path`, `prefix` (Linux; else `null`) (since 0.0.7). Progress lines while the game's prefix is prepared; then it starts detached. Download the game there, then `status` finds it |

`run` runs an installer or patch inside the game's prefix (the game doesn't have to be installed yet: that's how a
game's own installer is run), then scans for what it installed. `--tool` runs the game's `tool_launcher` (AotR's own
launcher). When the program closes with an error, the result is `{"ok":false,"kind":"program","exit_code":N,…}`.
`install-from` works the same way, after turning the player's own copy into a setup program (see
[Install from your own copy](#install-from-your-own-copy)); a path that isn't an installer, image or folder is a
`usage` error. `workshop` and `workshop-install` for a game the workshop doesn't cover are `usage` errors; asking
`workshop-install` for a base game or EA patch fails (`failed`) without contacting the server. `aio-launcher install`
fails (`failed`) without a runner on Linux, when the download isn't the launcher or doesn't match the published MD5
(nothing is installed then); `aio-launcher open` fails when the launcher isn't installed. `community-launcher` for a
game without one is a `usage` error, as is `set` with a path that isn't an existing `.exe` or `.zip` (a relative path
is taken from the current folder); `set` fails (`failed`) when the file isn't a Windows program or a `.zip` has no
launcher exe (nothing is changed then); `open` fails when no launcher is chosen, its copy is gone, or (Linux) there's
no runner. Beacon never downloads a community launcher.

**Windows.** The CLI is `beacon.exe` with the same commands and JSON. `runners` answers in the same shape with the
`native` runner; `setup` reports `did_work: false`; `paths` still lists the (unused) prefix folders.

### Your own games and compatibility tools (0.0.8)

| Command | Answer |
|---|---|
| `custom list` | `{games: [CustomStatus], file, faugus, flags: [{key, env, label}], stores, tools: [{key, label}]}`. Each game: its settings (`id`, `title`, `exe`, `args`, `cwd`, `prefix`, `runner`, `icon`, `cover`, `hero`, `env`, `no_dxvk`, `no_vkd3d`, `mangohud`, `gamemode`, `gamescope`, `gamescope_args`, `flags`, `umu_id`, `store`, `added`, `installer`, `source`) plus `key` (`custom:<id>`), `family` (`other`), `installed`, `ready`, `issues`, `resolved_runner`, `wineprefix`, `prefix_ready`, `own_prefix`, `play_seconds`, `last_played` (Unix time), `sessions`, `log_file`, `has_log`. `icon`/`cover`/`hero` are absolute paths to PNG/JPG/WebP files. |
| `custom add`, `custom set`, `custom remove`, `custom import`, `custom import-faugus`, `custom export` | `{ok, game: null, id, message, …}`; `add` and `set` include `custom` (the game's CustomStatus). |
| `custom launch <id> --wait` | Like `launch`: started, then (with `--wait`) `{exit_code, play_seconds, last_played}` when the game closes. Beacon stays the parent of the game's processes, so an environment variable Hearth sets (`HEARTH_GAME_ID`) reaches them. |
| `custom install`, `custom run`, `custom tool` | Long jobs with progress lines; `{ok, id, exit_code, …}` (`install` adds `candidates`), exit code 3 when the program failed. |
| `tools list [--target T] [--offline]` | `{target, targets: [{id, name, detected}], tools: [{id, name, kind, repo, blurb, archived, supported, dir, installed: [{name, path, tool, tag, latest, by_beacon}], newest, up_to_date, has_latest}], errors, auto_update}` |
| `tools releases <tool>` | `{tool, releases: [{tag, name, published, prerelease, asset, url, size, checksum_url}]}` |
| `tools install`, `tools update` | Progress lines, then `{ok, installed: {tool, target, tag, path, latest, verified, bytes}}` / `{ok, updated: […]}`. |

`status` also has `play_seconds` and `last_played` for each built-in game (from `launch --wait`).

### Exit codes

| Code | Meaning | `kind` |
|---|---|---|
| 0 | Done | |
| 1 | It didn't work; `error` says why | `failed` |
| 2 | Bad arguments: an unknown game, mod, runner, edition or step, or a malformed value | `usage` |
| 3 | The Windows program Beacon ran (installer, patch, launcher) closed with an error | `program` |

## Hearth integration

- `beacon-app --fullscreen` (or `BEACON_FULLSCREEN=1`) opens it full-screen for Game Mode.
- Beacon minimizes while a game runs and comes back when the game exits.
- `beacon status --json` and `beacon launch <game>` let Hearth list the games and start them directly. Since 0.0.6
  Hearth also sets games up, changes their settings and manages mods through the JSON commands above, so everything
  Beacon does is in Hearth's Library; Beacon's own window is for outside Hearth.
- Games you add yourself are listed by `beacon custom list --json` and started with `beacon custom launch <id> --wait`;
  Hearth shows them in its Library (Beacon › Other Games) with their cover or icon and play time.
- On an Nvidia GPU, Beacon's window sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` for itself (unless it's already set):
  WebKitGTK's DMA-BUF renderer crashes the window at start on the 615 driver.
- `BEACON_CONFIG` and `BEACON_DATA_DIR` move the config file and data folder.
- Windows (Ember): `beacon.exe` and `beacon-app.exe` sit in the same folder in every Windows package.

## Good to know

- **In-game controls.** The BFME games have no controller support. On a handheld, map the controller to mouse and keyboard with Steam Input or a Handheld Daemon (HHD) profile.
- **LAN** works through Wine's normal networking. Online play is out of scope for now.
- **The registry key set is the minimum** the games are known to read. If a particular game version wants more, there's one place to add it: `registry.rs`.
- **Per-game environment variables** (for example `DXVK_HUD=fps` or `WINEDEBUG`) go in the `env` map of that game's entry in `~/.config/beacon/config.json`.

## License

Beacon is free software, licensed under the **GNU General Public License v3.0 or later** ([LICENSE](LICENSE)).
Ideas from Faugus Launcher (MIT) and ProtonPlus (GPL-3.0) are credited in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). The games it launches, and the community launchers it can set up, belong to their owners and keep their own terms: Beacon never ships or downloads the games themselves.
