// Talks to the Rust side through Tauri. Opened in a plain browser (no
// Tauri), it falls back to a mock so the UI can be designed and tested.

const tauri = window.__TAURI__;

function realApi() {
  return {
    mock: false,
    call: (cmd, args = {}) => tauri.core.invoke(cmd, args),
    on: (event, cb) => tauri.event.listen(event, (e) => cb(e.payload)),
  };
}

function mockApi() {
  const listeners = {};
  const emit = (ev, payload) => (listeners[ev] || []).forEach((cb) => cb(payload));
  const runners = [
    { id: "proton:GE-Proton10-12", name: "GE-Proton10-12", kind: "proton", path: "~/.steam/root/compatibilitytools.d/GE-Proton10-12" },
    { id: "proton:Proton 9.0 (Beta)", name: "Proton 9.0 (Beta)", kind: "proton", path: "~/.steam/steam/steamapps/common/Proton 9.0 (Beta)" },
    { id: "umu", name: "umu-launcher (UMU-Proton)", kind: "umu", path: "/usr/bin/umu-run" },
    { id: "wine:system", name: "System Wine", kind: "wine", path: "/usr/bin/wine" },
  ];
  const mk = (id, title, short, path, extra = {}) => ({
    id, title, short, path, family: "bfme", virtual_desktop: false, steam_appid: null, via_steam: false, installed: !!path, runner: runners[0], has_cd_key: true,
    resolution: { width: 1920, height: 1080 }, windowed: false, extra_args: "", prefix_ready: true,
    runner_choice: null, selected_mod: null, ready: !!path, issues: path ? [] : [{ level: "error", message: "Game folder not set — scan or pick its folder." }], store_url: null,
    ...extra,
  });
  const state = {
    version: "0.0.7 (preview)",
    games: [
      mk("bfme1", "The Battle for Middle-earth", "BFME", "/home/you/Games/BFME1"),
      mk("bfme2", "The Battle for Middle-earth II", "BFME II", "/home/you/Games/BFME2", { has_cd_key: false, issues: [{ level: "warn", message: "No CD key set. Add the key from your copy if the game asks you to reinstall." }] }),
      mk("rotwk", "The Rise of the Witch-king", "RotWK", "/home/you/Games/RotWK", { selected_mod: "rotwk:age-of-the-ring" }),
      mk("aotr", "Age of the Ring", "AotR", null, { has_cd_key: false, tool_launcher: null }),
      mk("conquest", "The Lord of the Rings: Conquest", "Conquest", "/home/you/Games/LOTR Conquest", { family: "lotr", has_cd_key: false, resolution: null }),
      mk("uaw", "Universe at War: Earth Assault", "UaW", "/home/you/Games/Universe at War Earth Assault", { family: "other", resolution: null, issues: [{ level: "warn", message: "Games for Windows Live is gone: put the community xliveless xlive.dll in the game folder (campaign and skirmish only)." }], community_launcher: { path: null, url: "https://drive.google.com/drive/folders/167UET84ZWXrMd9zpxEYiQWTQZexCGZfT" } }),
      mk("exigo", "Armies of Exigo", "Exigo", null, { family: "other", resolution: null }),
      mk("rol", "Rise of Nations: Rise of Legends", "RoL", "/home/you/Games/Rise of Legends", { family: "other", resolution: null }),
      mk("swgb", "Star Wars: Galactic Battlegrounds Saga", "SWGB", "/home/you/.local/share/Steam/steamapps/common/Star Wars Galactic Battlegrounds Saga/Game", { family: "other", resolution: null, virtual_desktop: true, steam_appid: 356500, store_url: "https://store.steampowered.com/app/356500/", edition: "Clone Campaigns", issues: [{ level: "warn", message: "Community version not set up yet — Game settings → Community version walks you through it." }] }),
    ],
    runners, default_runner: null, best_runner: runners[0].id,
    mods_dir: "/home/you/.local/share/beacon/mods", data_dir: "/home/you/.local/share/beacon", running: [],
    native: false, aio_url: "https://bfmeladder.com",
    aio_launcher: {
      installed: false, path: null, prefix: "/home/you/.local/share/beacon/prefixes/aio-launcher/pfx", version: null, by_beacon: false, can_install: true, runner: runners[0].id,
      download_url: "https://arena-files.bfmeladder.com/application-builds/all-in-one-launcher-main", site: "https://bfmeladder.com", log: "/home/you/.local/share/beacon/logs/aio-launcher.log",
    },
  };
  const workshopItems = {
    rotwk: [
      { id: "6f1c2a7e-0001", name: "Edain Mod", version: "4.8.1", author: "Edain Team", description: "Lore-focused overhaul.", kind: "mod", game: "rotwk", size: 2500000000, downloads: 1234, artwork_url: null, installed: null },
      { id: "6f1c2a7e-0002", name: "Rhûn Map Pack", version: "1.2", author: "Mapper", description: "Eight new skirmish maps.", kind: "maps", game: "rotwk", size: 42000000, downloads: 310, artwork_url: null, installed: null },
    ],
    bfme2: [{ id: "6f1c2a7e-0003", name: "Widescreen HUD", version: "2.0", author: "Someone", description: "HUD fixes for 16:9.", kind: "enhancement", game: "bfme2", size: 3000000, downloads: 99, artwork_url: null, installed: null }],
    bfme1: [],
  };
  const mods = {
    bfme1: [],
    bfme2: [{ id: "bfme2:widescreen", name: "Widescreen", game: "bfme2", kind: "mod", path: "/mods/bfme2/widescreen.big", args: [], in_library: true }],
    aotr: [],
    conquest: [], uaw: [], exigo: [], rol: [], swgb: [],
    rotwk: [
      { id: "rotwk:age-of-the-ring", name: "Age of the Ring", game: "rotwk", kind: "mod", path: "/mods/rotwk/Age of the Ring", args: [], in_library: true },
      { id: "rotwk:edain", name: "Edain", game: "rotwk", kind: "launcher", path: "/home/you/Mods/Edain/Edain Launcher.exe", args: [], in_library: false },
    ],
  };
  const game = (id) => state.games.find((g) => g.id === id);
  const ed = (id, name, blurb, community, installed, active) => ({ id, name, blurb, community, installed, active });
  const step = (id, title, detail, action, url = "https://example.org") => ({ id, title, detail, action, url, done: false });
  const community = {
    swgb: {
      editions: [ed("expanding-fronts", "Expanding Fronts", "The community expansion.", true, false, false), ed("clone-campaigns", "Clone Campaigns", "The Saga edition's expansion, unmodded.", false, true, true), ed("original", "Original game", "The base game.", false, false, false)],
      steps: [step("expanding-fronts", "Expanding Fronts", "Download the FULL Expanding Fronts zip from ModDB (1.5.5, about 1.15 GB), then pick it here. Beacon merges its Game files into your install and backs up anything it replaces.", "merge")],
    },
    uaw: {
      editions: [ed("community-patch", "Community Patch", "Fan-voted balance and bug-fix patch.", true, false, false), ed("human-resistance", "Human Resistance Reborn", "Adds a Human faction.", true, false, false), ed("vanilla", "Patch 3 (vanilla)", "The last official version.", false, true, true)],
      steps: [step("patch3", "Official Patch 3", "Steam copies already have it.", "installer"), step("community-patch", "Community Patch", "Pick the archive; added under Mods.", "mods"), step("human-resistance", "Human Resistance Reborn (optional)", "Added under Mods.", "mods"), step("xliveless", "xliveless", "Put xlive.dll in the game folder.", "merge")],
    },
    exigo: { editions: [], steps: [step("patch14", "Patch 1.4", "The version the community plays.", "installer"), step("intel-fix", "Integrated graphics fix", "Only for the minimum-requirements error.", "merge"), step("forum", "Armies of Exigo community", "Forum with downloads and guides.", "link")] },
    rol: { editions: [], steps: [step("patch", "Latest official patch", "From Rise of Legends Heaven.", "installer"), step("crash-fix", "Long-game crash fix", "Community fix for long sessions.", "merge"), step("user-patch", "User Patch Project", "Community patch in progress.", "link")] },
  };
  const snap = () => structuredClone(state);
  // Download jobs (private build), like the Rust side: downloads-changed on every change.
  const jobs = [];
  let jobId = 0;
  const pausedJobs = new Set();
  const jobRun = async (kind, title, game, retry, steps) => {
    const job = { id: ++jobId, kind, title, game, state: "running", pct: null, text: "Starting", started: Date.now() / 1000, ended: null, message: null, retry };
    jobs.push(job);
    emit("downloads-changed", structuredClone(job));
    for (const [pct, text] of steps) {
      await delay(700);
      if (pausedJobs.has(job.id)) {
        Object.assign(job, { state: "paused", message: "Paused — resume to carry on where it stopped", ended: Date.now() / 1000 });
        emit("downloads-changed", structuredClone(job));
        throw "BFME: INI.big: downloading from workshop-files.bfmeladder.com: paused";
      }
      Object.assign(job, { pct, text });
      emit("downloads-changed", structuredClone(job));
    }
    return job;
  };
  const jobDone = (job, message) => {
    Object.assign(job, { state: "done", pct: 100, message, ended: Date.now() / 1000 });
    emit("downloads-changed", structuredClone(job));
  };
  const dl = { version: 1, built_in: true, sources_repo: "Dragongamer9/beacon-private", games_dir: "/home/you/.local/share/beacon/games", custom_games_dir: false, learned_api: null, checks: null, games: ["bfme1", "bfme2", "rotwk", "rol"], launchers: ["uaw"], archive_launchers: {} };
  const delay = (ms) => new Promise((r) => setTimeout(r, ms));

  const handlers = {
    get_state: () => snap(),
    scan: async () => { await delay(400); return snap(); },
    set_game_path: ({ game: g, path }) => { Object.assign(game(g), { path, installed: true, ready: true, issues: [] }); return snap(); },
    set_cd_key: ({ game: g, key }) => { Object.assign(game(g), { has_cd_key: !!key.trim(), issues: [] }); return snap(); },
    set_runner: ({ game: g, runner }) => {
      if (g) Object.assign(game(g), { runner_choice: runner, runner: runners.find((r) => r.id === runner) || runners[0] });
      else state.default_runner = runner;
      return snap();
    },
    get_community: ({ game: g }) => structuredClone(community[g] || { editions: [], steps: [] }),
    set_edition: ({ game: g, id }) => {
      community[g].editions.forEach((e) => (e.active = e.id === id));
      game(g).edition = community[g].editions.find((e) => e.active)?.name;
      return snap();
    },
    add_files: async ({ game: g, step }) => {
      await delay(1200);
      const c = community[g];
      c.steps.find((s) => s.id === step).done = true;
      const ed = c.editions.find((e) => e.id === step);
      if (ed) { ed.installed = true; c.editions.forEach((e) => (e.active = e === ed)); Object.assign(game(g), { edition: ed.name, issues: [] }); }
      return [snap(), "Merged 1,204 files into the game folder"];
    },
    set_launch_options: ({ game: g, virtualDesktop, viaSteam }) => {
      if (virtualDesktop !== null) game(g).virtual_desktop = virtualDesktop;
      if (viaSteam !== null) game(g).via_steam = viaSteam;
      return snap();
    },
    set_display: ({ game: g, resolution, windowed, extraArgs }) => {
      const [w, h] = (resolution || "").split("x").map(Number);
      Object.assign(game(g), { resolution: resolution ? { width: w, height: h } : null, windowed, extra_args: extraArgs });
      return snap();
    },
    list_mods: ({ game: g }) => structuredClone(mods[g]),
    add_mod: ({ game: g, path, name }) => {
      const n = name || path.split("/").pop().replace(/\.(big|exe)$/i, "");
      const m = { id: `${g}:${n.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`, name: n, game: g, kind: /\.exe$/i.test(path) ? "launcher" : "mod", path, args: [], in_library: false };
      mods[g].push(m); game(g).selected_mod = m.id; return snap();
    },
    remove_mod: ({ id }) => {
      for (const g of Object.keys(mods)) mods[g] = mods[g].filter((m) => m.id !== id);
      state.games.forEach((g) => { if (g.selected_mod === id) g.selected_mod = null; });
      return snap();
    },
    select_mod: ({ game: g, id }) => { game(g).selected_mod = id || null; return snap(); },
    launch: async ({ game: g }) => {
      state.running.push(g);
      setTimeout(() => emit("game-event", { game: g, phase: "preparing" }), 50);
      setTimeout(() => emit("game-event", { game: g, phase: "running" }), 1600);
      setTimeout(() => { state.running = state.running.filter((x) => x !== g); emit("game-event", { game: g, phase: "exited" }); }, 4200);
    },
    run_installer: async (a) => {
      await handlers.launch(a);
      if (a.game === "aotr") setTimeout(() => Object.assign(game("aotr"), { path: "/home/you/.local/share/beacon/prefixes/aotr/pfx/drive_c/AgeoftheRing/aotr", installed: true, ready: true, issues: [], tool_launcher: "/…/AgeoftheRing/AotR Launcher.exe" }), 4000);
    },
    launch_preview: ({ game: g, modId }) => {
      const gm = game(g);
      const mod = mods[g].find((m) => m.id === (modId ?? gm.selected_mod));
      return `WINEPREFIX=~/.local/share/beacon/prefixes/${g}/pfx PROTONPATH=${gm.runner.path} umu-run '${gm.path}/lotrbfme…exe'${gm.windowed ? " -win" : ""}${mod ? ` -mod 'Z:${mod.path.replaceAll("/", "\\")}'` : ""}`;
    },
    get_catalog: ({ game: g }) => {
      const cat = {
        rotwk: [["edain","Edain Mod","mod","Large lore-focused overhaul with its own launcher and submods.","rotwk:edain"],["aotr","Age of the Ring","mod","Major overhaul with new factions, heroes and a reworked campaign.","rotwk:age-of-the-ring"],["elven-alliance","Elven Alliance","mod","Expands the Elven factions with new units and structures.",null],["rotwk-202","RotWK Patch 2.02","patch","Community balance patch many RotWK mods expect.",null]],
        bfme2: [["patch-222","BFME Patch 2.22 / All-in-One","patch","Community patch for BFME II.",null],["divide-conquer","Divide and Conquer","mod","Splits the base factions into more distinct sub-factions.",null]],
        conquest: [["mordorwide","MordorWide (online multiplayer)","patch","Community re-implementation of EA's old Conquest servers. Needs patch 1.1 and a patched exe.",null],["conquest-fixes","Conquest Fixes","mod","Bug fixes and restored cut levels, without changing balance.",null]],
        uaw: [["uaw-xliveless","xliveless (removes Games for Windows Live)","patch","Put xliveless's xlive.dll in the game folder.",null]],
        exigo: [["exigo-intel-fix","Integrated graphics fix","patch","Replacement exe for the minimum-requirements error on Intel graphics.",null]],
        rol: [["rol-heaven","Rise of Legends Heaven","patch","Community site with patches, guides and fixes.",null]],
        swgb: [["swgb-steam","Galactic Battlegrounds Saga on Steam","game","Still sold.",null],["swgb-fix","Crash & stuck-key fix","patch","Fixes the crash after ~20 minutes and stuck arrow keys.",null]],
        aotr: [["aotr-standalone","Age of the Ring Standalone","game","The fan remake, with both base games bundled.",null]],
        bfme1: [["bfme1-108","BFME Patch 1.08","patch","Community balance and bug-fix patch.",null],["mee","Middle-earth Extended Edition","mod","Extends the original factions.",null],["return-of-shadow","Return of Shadow","mod","Long-running overhaul adding new factions and heroes.",null]],
      }[g];
      return cat.map(([id, name, kind, blurb, installed]) => ({ id, name, game: g, kind, blurb, url: "https://www.moddb.com/search?q=" + name, installed }));
    },
    find_mods: async () => { await delay(500); return [snap(), 0]; },
    install_from: async (a) => {
      setTimeout(() => emit("game-event", { game: a.game, phase: "preparing", message: "Extracting BFME2.iso (this can take a few minutes)" }), 300);
      await handlers.launch(a);
    },
    workshop_list: async ({ game: g, search, page }) => {
      await delay(500);
      const all = (workshopItems[g] || []).filter((i) => !search || (i.name + i.description).toLowerCase().includes(search.toLowerCase()));
      return { items: page ? [] : structuredClone(all), hidden: page ? 0 : 3, source: "https://bfmeladder.com" };
    },
    workshop_install: async ({ game: g, id }) => {
      const it = workshopItems[g].find((i) => i.id === id);
      for (const pct of [10, 40, 75, 95]) {
        emit("workshop-progress", { game: g, id, pct, text: `Downloading ${it.name.toLowerCase().replace(/\W+/g, "_")}.big` });
        await delay(350);
      }
      const m = { id: `${g}:${it.name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`, name: it.name, game: g, kind: "mod", path: `/home/you/.local/share/beacon/mods/${g}/${it.name}`, args: [], in_library: true };
      mods[g].push(m);
      it.installed = m.id;
      return [snap(), `${it.name} ${it.version} installed`];
    },
    aio_info: () => ({ launcher_url: "https://bfmeladder.com", games: [], appdata: [], launcher: structuredClone(state.aio_launcher) }),
    aio_launcher_install: async () => {
      for (const [pct, text] of [[5, "Creating the launcher's Wine prefix (the first time can take a few minutes)"], [null, "Downloading the All-in-One launcher from the BFME Foundation… 40 MB"], [null, "Downloading the All-in-One launcher from the BFME Foundation… 180 MB"], [96, "Installing the launcher"]]) {
        emit("aio-launcher-progress", { pct, text });
        await delay(500);
      }
      Object.assign(state.aio_launcher, { installed: true, by_beacon: true, path: "/home/you/.local/share/beacon/prefixes/aio-launcher/pfx/drive_c/users/steamuser/AppData/Roaming/BFME All In One Launcher/AllInOneLauncher.exe", version: "0123456789abcdef0123456789abcdef" });
      return [snap(), "All-in-One launcher installed"];
    },
    aio_launcher_open: () => {
      setTimeout(() => emit("aio-launcher-closed", null), 5000);
      return snap();
    },
    community_launcher_set: async ({ game: g, path }) => {
      await delay(600);
      const name = path.split("/").pop().replace(/\.(zip|exe)$/i, "");
      game(g).community_launcher.path = `/home/you/.local/share/beacon/tools/${g}-launcher/${name}/${name}.exe`;
      return [snap(), `${game(g).short} launcher set: ${name}.exe`];
    },
    community_launcher_open: async ({ game: g }) => {
      emit("community-launcher-progress", { game: g, pct: 10, text: "Creating UaW's Wine prefix (the first time can take a few minutes)" });
      await delay(800);
      setTimeout(() => emit("community-launcher-closed", { game: g, found: false }), 5000);
      return snap();
    },
    // Private build: downloads.
    downloads_info: () => structuredClone(dl),
    downloads_list: () => structuredClone(jobs),
    download_pause: ({ id }) => { pausedJobs.add(id); },
    download_forget: ({ id }) => { const i = jobs.findIndex((j) => j.id === id); if (i >= 0) jobs.splice(i, 1); },
    downloads_storage: () => ({ bytes: 734003200, files: 3 }),
    downloads_clear: () => 734003200,
    cdn_get: async (args) => {
      const job = await jobRun("game", game(args.game).title, args.game, { cmd: "cdn_get", args }, [[5, "Looking up"], [32, "INI.big"], [61, "Textures.big"], [88, "Maps.big"]]);
      const r = await handlers.cdn_get_inner(args);
      jobDone(job, r.message);
      return r;
    },
    cdn_get_inner: async ({ game: g, repair, acceptGuess }) => {
      if (g === "bfme1" && !acceptGuess && !dl.guessed) {
        dl.guessed = true;
        return { result: "needs-confirm", game: g, note: "original-BFME1 is gone; the closest package is original-BFME1-v2 (Vanilla (1.03))" };
      }
      if (g === "rol") {
        for (const pct of [null, 20, 60, 89, 90, 97]) {
          emit("cdn-progress", { game: g, pct, text: pct === 90 ? "Extracting Rise Of Legends Game Files.zip" : pct === 97 ? "Putting the game in place" : "Downloading Rise Of Legends Game Files.zip" });
          await delay(300);
        }
        Object.assign(game(g), { path: `${dl.games_dir}/Rise of Legends`, installed: true, ready: true, issues: [] });
        return { result: "done", snapshot: snap(), message: "RoL downloaded and ready", launcher: null };
      }
      const order = g === "rotwk" && !game("bfme2").installed ? ["bfme2", "rotwk"] : [g];
      for (const id of order) {
        for (const pct of [0, 12, 37, 64, 88, 99]) {
          emit("cdn-progress", { game: g, pct, text: `${game(id).short} (${Math.round(pct * 2.4)}/240): ${pct < 50 ? "data\\movies\\intro.vp6" : "Textures.big"}` });
          await delay(300);
        }
        Object.assign(game(id), { path: `${dl.games_dir}/${{ bfme1: "BFME1", bfme2: "BFME2", rotwk: "RotWK" }[id]}`, installed: true, ready: true, has_cd_key: true, issues: [] });
      }
      return { result: "done", snapshot: snap(), message: `${order.map((id) => game(id).short).join(" and ")} ${repair ? "repaired" : "downloaded and ready"}`, notes: [] };
    },
    sources_check: async () => {
      await delay(900);
      dl.checks = [
        { id: "bfme1", title: "The Battle for Middle-earth", state: "ok", detail: "Vanilla (1.03): 183 files, 3.9 GB" },
        { id: "bfme2", title: "The Battle for Middle-earth II", state: "ok", detail: "Vanilla (1.06): 240 files, 5.6 GB" },
        { id: "rotwk", title: "The Rise of the Witch-king", state: "moved", detail: "Vanilla (2.01): 151 files, 3.1 GB (the workshop moved to api.example.org; Beacon found it in the All-in-One launcher's source)" },
      ];
      return structuredClone(dl.checks);
    },
    launcher_get: async ({ game: g }) => {
      const cl = game(g).community_launcher;
      if (cl.path?.includes("UaW Launcher 1.4")) return { result: "up-to-date", snapshot: snap(), message: "The UaW launcher is already the newest (UaW Launcher 1.4.zip)" };
      for (const pct of [null, 20, 55, 90]) {
        emit("community-launcher-progress", { game: g, pct, text: pct == null ? "Looking for the newest UaW launcher" : "Downloading UaW Launcher 1.4.zip" });
        await delay(350);
      }
      cl.path = "/home/you/.local/share/beacon/tools/uaw-launcher/UaW Launcher 1.4/UaW Launcher.exe";
      return { result: "done", snapshot: snap(), message: "UaW Launcher 1.4.zip ready — open it to download the game" };
    },
    app_update_check: async () => { await delay(500); return { current: "0.0.9", latest: { version: "0.0.10", tag: "v0.0.10", url: "", notes: "", assets: [] }, newer: true, problem: null, method: { kind: "app-image" } }; },
    app_update_apply: async () => {
      for (const [pct, text] of [[null, "Looking for the newest Beacon"], [40, "Downloading Beacon 0.0.10"], [96, "Checking the download"], [98, "Putting the new version in place"]]) {
        emit("app-update-progress", { pct, text });
        await delay(400);
      }
      return { next: "restart", program: "/home/you/Beacon.AppImage" };
    },
    mod_downloads: ({ game: g }) => (g === "rotwk" ? [{ id: "mod-edain", title: "Edain Mod", catalog_id: "edain" }, { id: "mod-edain-launcher", title: "Edain Mod Launcher", catalog_id: null }] : []),
    mod_lookup: async ({ game: g, id, link }) => { await delay(500); return { game: g, name: id === "mod-edain-launcher" ? "Edain Mod Launcher" : link ? "Some Mod" : "Edain Mod", file: "Edain4.8.zip", size: 2861726686, url: "https://www.moddb.com/downloads/mirror/1/2/3", md5: null, install: "library", catalog_id: "edain", note: null }; },
    mod_install: async (args) => {
      const job = await jobRun("mod", args.found.name, args.found.game, { cmd: "mod_install", args }, [[12, `Downloading ${args.found.file}`], [55, `Downloading ${args.found.file}`]]);
      const r = await handlers.mod_install_inner(args);
      jobDone(job, r.message);
      return r;
    },
    mod_install_inner: async ({ found }) => {
      for (const pct of [10, 45, 80, 90, 96]) { emit("mod-progress", { game: found.game, pct, text: pct >= 90 ? "Extracting Edain4.8.zip" : "Downloading Edain4.8.zip" }); await delay(300); }
      const m = { id: `${found.game}:edain-mod`, name: found.name, game: found.game, kind: "mod", path: `/home/you/.local/share/beacon/mods/${found.game}/${found.name}`, args: [], in_library: true };
      mods[found.game].push(m); game(found.game).selected_mod = m.id;
      return { installed: { kind: "library", game: found.game, dir: m.path, mod_id: m.id }, snapshot: snap(), message: `${found.name} installed and selected` };
    },
    sources_update: async () => { await delay(600); return { from: dl.version, to: ++dl.version, skipped: null }; },
    set_updates_repo: ({ repo }) => { if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw `"${repo}" isn't a GitHub repo (owner/repo)`; dl.sources_repo = repo; return repo; },
    set_games_dir: ({ path }) => { dl.games_dir = path || "/home/you/.local/share/beacon/games"; dl.custom_games_dir = !!path; },
    look_for_games: async () => { await delay(300); return []; },
    open_url: () => null,
    pick_folder: async () => { await delay(200); return "/home/you/Games/Picked"; },
    pick_file: async ({ kind }) => { await delay(200); return kind === "mod" ? "/home/you/Downloads/Some Mod.big" : kind === "launcher" ? "/home/you/Downloads/UaW Launcher 1.3.zip" : "/home/you/Downloads/patch.exe"; },
    open_path: () => null,
    open_log: () => null,
  };

  return {
    mock: true,
    call: async (cmd, args = {}) => {
      const h = handlers[cmd];
      if (!h) throw `mock: no handler for ${cmd}`;
      return h(args);
    },
    on: async (event, cb) => { (listeners[event] ||= []).push(cb); },
  };
}

export const api = tauri?.core ? realApi() : mockApi();
