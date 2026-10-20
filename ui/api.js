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
    id, title, short, path, family: "bfme", installed: !!path, runner: runners[0], has_cd_key: true,
    resolution: { width: 1920, height: 1080 }, windowed: false, extra_args: "", prefix_ready: true,
    runner_choice: null, selected_mod: null, ready: !!path, issues: path ? [] : [{ level: "error", message: "Game folder not set — scan or pick its folder." }],
    ...extra,
  });
  const state = {
    version: "0.0.1 (preview)",
    games: [
      mk("bfme1", "The Battle for Middle-earth", "BFME", "/home/you/Games/BFME1"),
      mk("bfme2", "The Battle for Middle-earth II", "BFME II", "/home/you/Games/BFME2", { has_cd_key: false, issues: [{ level: "warn", message: "No CD key set. Add the key from your copy if the game asks you to reinstall." }] }),
      mk("rotwk", "The Rise of the Witch-king", "RotWK", "/home/you/Games/RotWK", { selected_mod: "rotwk:age-of-the-ring" }),
      mk("aotr", "Age of the Ring", "AotR", null, { has_cd_key: false, tool_launcher: null }),
      mk("conquest", "The Lord of the Rings: Conquest", "Conquest", "/home/you/Games/LOTR Conquest", { family: "lotr", has_cd_key: false, resolution: null }),
    ],
    runners, default_runner: null, best_runner: runners[0].id,
    mods_dir: "/home/you/.local/share/beacon/mods", data_dir: "/home/you/.local/share/beacon", running: [],
  };
  const mods = {
    bfme1: [],
    bfme2: [{ id: "bfme2:widescreen", name: "Widescreen", game: "bfme2", kind: "mod", path: "/mods/bfme2/widescreen.big", args: [], in_library: true }],
    aotr: [],
    conquest: [],
    rotwk: [
      { id: "rotwk:age-of-the-ring", name: "Age of the Ring", game: "rotwk", kind: "mod", path: "/mods/rotwk/Age of the Ring", args: [], in_library: true },
      { id: "rotwk:edain", name: "Edain", game: "rotwk", kind: "launcher", path: "/home/you/Mods/Edain/Edain Launcher.exe", args: [], in_library: false },
    ],
  };
  const game = (id) => state.games.find((g) => g.id === id);
  const snap = () => structuredClone(state);
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
        rotwk: [["edain","Edain Mod","mod","Large lore-focused overhaul with its own launcher and submods.","rotwk:edain"],["aotr","Age of the Ring","mod","Major overhaul with new factions, heroes and a reworked campaign.","rotwk:age-of-the-ring"],["dwarf-holds","The Dwarf Holds","mod","Adds the Dwarven kingdoms as separate playable factions.",null],["elven-alliance","Elven Alliance","mod","Expands the Elven factions with new units and structures.",null],["rotwk-202","RotWK Patch 2.02","patch","Community balance patch many RotWK mods expect.",null]],
        bfme2: [["patch-222","BFME Patch 2.22 / All-in-One","patch","Community patch for BFME II.",null],["divide-conquer","Divide and Conquer","mod","Splits the base factions into more distinct sub-factions.",null]],
        conquest: [["mordorwide","MordorWide (online multiplayer)","patch","Community re-implementation of EA's old Conquest servers. Needs patch 1.1 and a patched exe.",null],["conquest-fixes","Conquest Fixes","mod","Bug fixes and restored cut levels, without changing balance.",null]],
        aotr: [["aotr-standalone","Age of the Ring Standalone","game","The fan remake, with both base games bundled.",null]],
        bfme1: [["bfme1-108","BFME Patch 1.08","patch","Community balance and bug-fix patch.",null],["mee","Middle-earth Extended Edition","mod","Extends the original factions.",null],["return-of-shadow","Return of Shadow","mod","Long-running overhaul adding new factions and heroes.",null]],
      }[g];
      return cat.map(([id, name, kind, blurb, installed]) => ({ id, name, game: g, kind, blurb, url: "https://www.moddb.com/search?q=" + name, installed }));
    },
    find_mods: async () => { await delay(500); return [snap(), 0]; },
    open_url: () => null,
    pick_folder: async () => { await delay(200); return "/home/you/Games/Picked"; },
    pick_file: async ({ kind }) => { await delay(200); return kind === "mod" ? "/home/you/Downloads/Some Mod.big" : "/home/you/Downloads/patch.exe"; },
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
