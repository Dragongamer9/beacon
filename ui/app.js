import { api } from "./api.js";
import * as nav from "./nav.js";

const $ = (s, r = document) => r.querySelector(s);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

let snap = null;
let sel = 0;
const sheets = []; // stack of open sheets: "mods" | "catalog" | "game" | "launcher"
let openSheet = null; // top of the stack
let overlayState = null; // { game, phase }
let installing = null; // game id while an installer we started is running
const AOTR_URL = "https://www.moddb.com/mods/the-horse-lords-a-total-modification-for-bfme/downloads/age-of-the-ring-version-90";
const modsCache = {};

const RESOLUTIONS = ["1280x720", "1280x800", "1366x768", "1600x900", "1920x1080", "1920x1200", "2560x1080", "2560x1440", "2560x1600", "3440x1440", "3840x2160"];
(() => {
  const native = `${Math.round(screen.width * devicePixelRatio)}x${Math.round(screen.height * devicePixelRatio)}`;
  if (!RESOLUTIONS.includes(native)) RESOLUTIONS.push(native);
  RESOLUTIONS.sort((a, b) => a.split("x")[0] * a.split("x")[1] - b.split("x")[0] * b.split("x")[1]);
  RESOLUTIONS.native = native;
})();

const game = () => snap.games[sel];
const isRunning = (id) => snap.running.includes(id);
const resText = (r) => (r ? `${r.width}x${r.height}` : "");

// ---------------------------------------------------------------- helpers

let toastTimer;
function toast(msg, error = false) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = "toast" + (error ? " error" : "");
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), error ? 6000 : 2600);
}

async function act(fn, okMsg) {
  try {
    const result = await fn();
    if (result && result.games) snap = result;
    render();
    if (okMsg) toast(okMsg);
    return true;
  } catch (e) {
    toast(String(e?.message ?? e), true);
    return false;
  }
}

async function loadMods(id, force = false) {
  if (force || !modsCache[id]) modsCache[id] = await api.call("list_mods", { game: id });
  return modsCache[id];
}

function modName(g) {
  if (!g.selected_mod) return "Vanilla";
  return modsCache[g.id]?.find((m) => m.id === g.selected_mod)?.name ?? g.selected_mod.split(":")[1];
}

// ---------------------------------------------------------------- main screen

const FAMILIES = [
  ["bfme", "Battle for Middle-earth"],
  ["lotr", "Lord of the Rings games"],
  ["other", "Other Games"],
];
const KICKERS = {
  aotr: "Fan-made standalone",
  conquest: "Pandemic Studios · 2009",
  uaw: "Petroglyph · 2007",
  exigo: "Black Hole Entertainment · 2004",
  rol: "Big Huge Games · 2006",
  swgb: "LucasArts · 2001",
};
const STORE = { swgb: "https://store.steampowered.com/app/356500/" };
const communityCache = {};
async function loadCommunity(id) {
  communityCache[id] = await api.call("get_community", { game: id });
  return communityCache[id];
}
const isSage = (g) => g.family === "bfme";

function buildShelf() {
  const shelf = $("#shelf");
  const tile = (g, i) => `
      <button class="nav tile" data-group="shelf" data-game="${g.id}" data-index="${i}">
        <svg class="emblem" viewBox="0 0 120 120"><use href="#emblem-${g.id}"/></svg>
        <span style="min-width:0"><span class="t-short">${esc(g.short)}</span><span class="t-state"><span class="dot"></span><span class="t-text"></span></span></span>
      </button>`;
  shelf.innerHTML = FAMILIES.map(([fam, label]) => {
    const members = snap.games.map((g, i) => [g, i]).filter(([g]) => g.family === fam);
    if (!members.length) return "";
    return `<div class="shelf-group" style="flex:${members.length}"><p class="shelf-label">${esc(label)}</p><div class="shelf-tiles">${members.map(([g, i]) => tile(g, i)).join("")}</div></div>`;
  }).join("");
  shelf.querySelectorAll(".tile").forEach((tile) => {
    const i = Number(tile.dataset.index);
    tile.addEventListener("focus", () => select(i));
    tile.addEventListener("click", () => (sel === i ? nav.focus($("#btn-play")) : select(i)));
  });
}

function select(i) {
  if (i === sel && document.body.dataset.accent === game().id) return;
  sel = (i + snap.games.length) % snap.games.length;
  render();
  loadMods(game().id).then(renderHero).catch(() => {});
}

function renderShelf() {
  const current = document.querySelector(`.tile[data-index="${sel}"]`);
  if (current && renderShelf.last !== sel) {
    renderShelf.last = sel;
    current.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }
  document.querySelectorAll(".tile").forEach((tile) => {
    const g = snap.games[Number(tile.dataset.index)];
    tile.classList.toggle("selected", g === game());
    tile.classList.toggle("is-ready", g.ready);
    tile.classList.toggle("is-missing", !g.installed);
    tile.querySelector(".t-text").textContent = isRunning(g.id) ? "Running" : g.ready ? "Ready" : g.installed ? "Needs attention" : "Not found";
  });
}

function renderHero() {
  const g = game();
  document.body.dataset.accent = g.id;
  $("#hero-title").textContent = g.title;
  $("#hero-kicker").textContent = KICKERS[g.id] ?? "The Lord of the Rings";
  $("#hero-emblem").setAttribute("href", `#emblem-${g.id}`);
  const running = isRunning(g.id);
  const chips = [];
  if (running) chips.push(`<span class="chip run"><span class="dot"></span>Running</span>`);
  else if (g.ready) chips.push(`<span class="chip ok"><span class="dot"></span>Ready</span>`);
  else chips.push(`<span class="chip bad"><span class="dot"></span>${g.installed ? "Needs attention" : "Not found"}</span>`);
  if (g.runner) chips.push(`<span class="chip">${esc(g.runner.name)}</span>`);
  if (g.edition) chips.push(`<span class="chip">${esc(g.edition)}</span>`);
  if (isSage(g)) chips.push(`<span class="chip">${g.resolution ? resText(g.resolution) : "Game's resolution"}${g.windowed ? " · windowed" : ""}</span>`);
  $("#hero-chips").innerHTML = chips.join("");

  const err = g.issues.find((i) => i.level === "error");
  const warn = g.issues.find((i) => i.level === "warn");
  const issue = $("#hero-issue");
  issue.hidden = !(err || warn);
  issue.innerHTML = err ? `<strong>Can't start yet.</strong> ${esc(err.message)}` : warn ? esc(warn.message) : "";

  const play = $("#btn-play");
  const label = $("#play-label");
  if (!g.installed) {
    label.textContent = g.id === "aotr" ? "Install Age of the Ring" : !isSage(g) ? `Install or locate ${g.short}` : "Locate game folder";
    play.disabled = false;
  } else {
    label.textContent = running ? "Running…" : "Play";
    play.disabled = running || !g.ready;
  }
  $("#mods-label").textContent = modName(g);
}

function renderHints(pad = renderHints.pad) {
  renderHints.pad = pad;
  const h = (glyph, cls, text) => `<span class="hint"><span class="glyph ${cls}">${glyph}</span>${text}</span>`;
  const rows = pad
    ? [h("A", "a", "Select"), h("B", "b", "Back"), h("X", "x", "Game settings"), h("Y", "y", "Mods"), `<span class="hint"><span class="glyph">LB</span><span class="glyph">RB</span>Switch game</span>`, h("☰", "", "Play")]
    : [h("Enter", "key", "Select"), h("Esc", "key", "Back"), h("S", "key", "Game settings"), h("M", "key", "Mods"), `<span class="hint"><span class="glyph key">Q</span><span class="glyph key">E</span>Switch game</span>`];
  $("#hints").innerHTML = rows.join("");
}

function render() {
  if (!snap) return;
  renderShelf();
  renderHero();
  // Only the top sheet re-renders; ones underneath refresh when uncovered.
  if (openSheet === "mods") renderModsSheet();
  if (openSheet === "catalog") renderCatalogSheet();
  if (openSheet === "game") renderGameSheet();
  if (openSheet === "launcher") renderLauncherSheet();
}

// ---------------------------------------------------------------- actions

async function play() {
  const g = game();
  if (!g.installed) return g.id === "aotr" || !isSage(g) ? openGameSheet() : locate(g);
  if (!g.ready || isRunning(g.id)) return;
  try {
    await api.call("launch", { game: g.id, modId: null });
  } catch (e) {
    toast(String(e?.message ?? e), true);
  }
}

async function locate(g) {
  const path = await api.call("pick_folder");
  if (path) await act(() => api.call("set_game_path", { game: g.id, path }), `${g.short} folder set`);
}

// ---------------------------------------------------------------- sheets

function rowHTML({ key, label, sub = "", value = "", cycle = false, selected = false, tick = false, badge = "", extra = "" }) {
  return `<button class="nav row${selected ? " selected" : ""}" data-key="${esc(key)}"${cycle ? " data-cycle" : ""}>
    <span style="min-width:0"><span class="label">${esc(label)}</span>${sub ? `<span class="sub">${esc(sub)}</span>` : ""}</span>
    <span class="value">${badge ? `<span class="badge">${esc(badge)}</span>` : ""}${cycle ? `<span class="arrows">◂</span>${esc(value)}<span class="arrows">▸</span>` : esc(value)}${tick ? `<svg class="tick" viewBox="0 0 24 24"><use href="#i-check"/></svg>` : ""}${extra}</span>
  </button>`;
}

function fieldHTML({ key, label, placeholder = "", value = "", help = "" }) {
  return `<div class="nav field" data-key="${esc(key)}" tabindex="-1">
    <label for="f-${esc(key)}">${esc(label)}</label>
    <input id="f-${esc(key)}" type="text" autocomplete="off" spellcheck="false" placeholder="${esc(placeholder)}" value="${esc(value)}">
    ${help ? `<span class="help">${esc(help)}</span>` : ""}
  </div>`;
}

function open(name, el) {
  sheets.push(name);
  openSheet = name;
  render();
  nav.pushLayer(el);
}

// Mods
function renderModsSheet() {
  const g = game();
  const sheet = $("#sheet-mods");
  sheet.querySelector("[data-game-short]").textContent = g.title;
  $("#btn-add-mod-folder").hidden = !isSage(g);
  $("#btn-add-mod-file").querySelector("svg").nextSibling.textContent = isSage(g) ? "Add mod file (.big or launcher)" : "Add a mod's launcher (.exe)";
  const mods = modsCache[g.id] || [];
  const keep = nav.current()?.dataset.key;
  let html = rowHTML({ key: "vanilla", label: "Vanilla", sub: "The game as it shipped", selected: !g.selected_mod, tick: !g.selected_mod });
  for (const m of mods) {
    const row = rowHTML({ key: m.id, label: m.name, sub: m.path, badge: m.kind === "launcher" ? "Launcher" : "", selected: g.selected_mod === m.id, tick: g.selected_mod === m.id });
    html += m.in_library
      ? row
      : `<div class="mod-line">${row}<button class="nav remove" data-key="rm:${esc(m.id)}" aria-label="Remove ${esc(m.name)}"><svg viewBox="0 0 24 24"><use href="#i-x"/></svg></button></div>`;
  }
  if (!mods.length)
    html += isSage(g)
      ? `<p class="section-label">No mods yet — add a .big file, a mod folder, or a mod's own launcher.</p>`
      : `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">${esc(g.short)} mods usually install into the game folder — run their installer from Game settings. Browse the catalog for known ones.</p>`;
  const rows = $("#mods-rows");
  rows.innerHTML = html;
  rows.querySelectorAll(".row").forEach((r) =>
    r.addEventListener("click", async () => {
      const id = r.dataset.key === "vanilla" ? null : r.dataset.key;
      if (await act(() => api.call("select_mod", { game: g.id, id }))) {
        closeAllSheets();
        toast(id ? `${modName(game())} selected` : "Playing vanilla");
      }
    })
  );
  rows.querySelectorAll(".remove").forEach((b) =>
    b.addEventListener("click", async () => {
      const id = b.dataset.key.slice(3);
      await act(() => api.call("remove_mod", { id }), "Mod removed");
      await loadMods(g.id, true);
      renderModsSheet();
    })
  );
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

// Catalog of known mods
let catalogCache = {};
async function openCatalog() {
  catalogCache[game().id] = await api.call("get_catalog", { game: game().id });
  open("catalog", $("#sheet-catalog"));
}

function renderCatalogSheet() {
  const g = game();
  $("#sheet-catalog").querySelector("[data-game-short]").textContent = g.title;
  const items = catalogCache[g.id] || [];
  const keep = nav.current()?.dataset.key;
  const section = (title, list) =>
    list.length
      ? `<p class="section-label">${title}</p>` +
        list
          .map((it) =>
            rowHTML({
              key: it.id,
              label: it.name,
              sub: it.blurb,
              badge: it.installed ? "Installed" : it.kind === "patch" ? "Patch" : "",
              value: it.installed ? (g.selected_mod === it.installed ? "" : "Use") : "Get",
              tick: g.selected_mod === it.installed && !!it.installed,
              selected: !!it.installed,
            })
          )
          .join("")
      : "";
  const rows = $("#catalog-rows");
  rows.innerHTML =
    section("Standalone game", items.filter((i) => i.kind === "game")) +
    section("Mods", items.filter((i) => i.kind === "mod")) +
    section("Community patches", items.filter((i) => i.kind === "patch")) +
    `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">“Get” opens the mod's page. Once it's downloaded, add it from Mods — or run its installer from Game settings.</p>`;
  rows.querySelectorAll(".row").forEach((r) =>
    r.addEventListener("click", async () => {
      const it = items.find((i) => i.id === r.dataset.key);
      if (it.installed) {
        if (await act(() => api.call("select_mod", { game: g.id, id: it.installed }))) {
          closeAllSheets();
          toast(`${it.name} selected`);
        }
      } else if (await act(() => api.call("open_url", { url: it.url }))) {
        toast(`Opened ${it.name} in your browser`);
      }
    })
  );
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function findMods() {
  try {
    const [next, added] = await api.call("find_mods");
    snap = next;
    await Promise.all(snap.games.map((x) => loadMods(x.id, true)));
    if (openSheet === "catalog") catalogCache[game().id] = await api.call("get_catalog", { game: game().id });
    render();
    toast(added ? `Found ${added} mod${added > 1 ? "s" : ""}` : "No new mods found");
  } catch (e) {
    toast(String(e?.message ?? e), true);
  }
}

async function addMod(kind) {
  const g = game();
  const path = kind === "folder" ? await api.call("pick_folder") : await api.call("pick_file", { kind: "mod" });
  if (!path) return;
  if (await act(() => api.call("add_mod", { game: g.id, path, name: null }), "Mod added and selected")) {
    await loadMods(g.id, true);
    render();
  }
}

// Game settings
function renderGameSheet() {
  const g = game();
  const sheet = $("#sheet-game");
  sheet.querySelector("[data-game-title]").textContent = g.title;
  const keep = nav.current()?.dataset.key;
  const runnerName = g.runner ? (g.runner_choice ? g.runner.name : `Automatic (${g.runner.name})`) : "None found";
  const res = resText(g.resolution);
  const rows = $("#game-rows");
  const aotr = g.id === "aotr";
  const sage = isSage(g);
  const comm = communityCache[g.id] || { editions: [], steps: [] };
  const installedEds = comm.editions.filter((e) => e.installed);
  const activeEd = comm.editions.find((e) => e.active);
  const stepVerb = { installer: "Run", merge: "Add", mods: "Add", link: "Open" };
  const communityRows =
    g.installed && (comm.editions.length || comm.steps.length)
      ? [
          `<p class="section-label">Community version</p>`,
          ...(comm.editions.length
            ? [rowHTML({ key: "edition", label: "Version", sub: activeEd?.blurb || "", value: activeEd ? activeEd.name + (activeEd.community ? " · community" : "") : "—", cycle: installedEds.length > 1 })]
            : []),
          ...comm.steps.map((st, i) => {
            const row = rowHTML({ key: `step:${st.id}`, label: st.title, sub: st.detail, value: stepVerb[st.action] })
              .replace('class="nav row', `class="nav row step${st.done ? " done" : ""}`)
              .replace('<span class="label">', `<span class="label"><span class="num">${st.done ? "✓" : i + 1}</span>`);
            const extras = [
              ...(st.action === "merge" || st.action === "mods" ? [`<button class="nav side" data-key="folder:${st.id}">Folder</button>`] : []),
              ...(st.url && st.action !== "link" ? [`<button class="nav side" data-key="page:${st.id}">Page</button>`] : []),
            ];
            return extras.length ? `<div class="mod-line">${row}${extras.join("")}</div>` : row;
          }),
        ]
      : [];
  const conquest = g.id === "conquest";
  rows.innerHTML = [
    ...communityRows,
    `<p class="section-label">Install</p>`,
    ...(aotr && !g.installed
      ? [
          rowHTML({ key: "aotr-get", label: "1 · Get the installer", sub: "Opens ModDB: download the Standalone zip (about 14 GB) and extract it", value: "Open" }),
          rowHTML({ key: "aotr-install", label: "2 · Run the installer", sub: "Pick the installer .exe from the extracted folder", value: "Choose" }),
        ]
      : []),
    ...(!sage && !g.installed
      ? [
          ...(STORE[g.id] ? [rowHTML({ key: "store", label: "Get it", sub: "Still sold — opens the store page", value: "Open" })] : []),
          rowHTML({ key: "install", label: "Run the game's installer", sub: `Your disc or setup files — it installs into ${g.short}'s own Wine prefix`, value: "Choose" }),
        ]
      : []),
    rowHTML({ key: "folder", label: (aotr || !sage) && !g.installed ? "Already installed? Locate it" : "Game folder", sub: g.path || (aotr ? "The Age of the Ring folder, e.g. …/drive_c/AgeoftheRing" : "Not set"), value: "Change" }),
    !sage || aotr ? "" : fieldHTML({
      key: "cdkey",
      label: "CD key",
      placeholder: g.has_cd_key ? "Saved — type a new one to replace it" : "Key from your copy of the game",
      help: "Stored in this game's own Wine prefix, the way the original installer did.",
    }),
    `<p class="section-label">Compatibility</p>`,
    rowHTML({ key: "runner", label: "Runner", sub: "Wine or Proton build used for this game", value: runnerName, cycle: true }),
    ...(conquest
      ? [`<p class="section-label">Online</p>`, rowHTML({ key: "mordorwide", label: "Play online with MordorWide", sub: "Community servers; needs patch 1.1 and their exe patch", value: "Guide" })]
      : []),
    `<p class="section-label">${sage ? "Display" : "Launch"}</p>`,
    ...(sage
      ? []
      : [
          ...(g.steam_appid
            ? [rowHTML({ key: "steam", label: "Launch through Steam", sub: "Use Steam's own Proton setup for this Steam copy", extra: `<span class="toggle${g.via_steam ? " on" : ""}"></span>` })]
            : []),
          ...(g.via_steam
            ? []
            : [
                rowHTML({ key: "vdesk", label: "Virtual desktop", sub: "Runs the game in a fixed-size Wine desktop — fixes click offsets and resolution switching", extra: `<span class="toggle${g.virtual_desktop ? " on" : ""}"></span>` }),
                g.virtual_desktop && rowHTML({ key: "res", label: "Desktop size", sub: "Set the game to this resolution or smaller", value: res || "1280x1024", cycle: true }),
              ]),
          `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">${esc(g.short)}'s resolution and controls are set in its own options menu.</p>`,
        ]),
    sage && rowHTML({ key: "res", label: "Resolution", sub: res === RESOLUTIONS.native ? "Matches this screen" : "Written to Options.ini before each launch", value: res || "Game's setting", cycle: true }),
    sage && rowHTML({ key: "windowed", label: "Windowed", sub: "Adds -win", extra: `<span class="toggle${g.windowed ? " on" : ""}"></span>` }),
    fieldHTML({ key: "args", label: "Extra arguments", placeholder: sage ? "-noshellmap" : "Optional", value: g.extra_args }),
    `<p class="section-label">Tools</p>`,
    ...(aotr && g.tool_launcher
      ? [`<button class="nav row-btn" data-key="aotr-launcher"><svg viewBox="0 0 24 24"><use href="#i-open"/></svg>Open the AotR Launcher for updates (may not work under Wine)</button>`]
      : []),
    `<button class="nav row-btn" data-key="installer"><svg viewBox="0 0 24 24"><use href="#i-plus"/></svg>Run a patch or installer in this game's prefix</button>`,
    `<button class="nav row-btn" data-key="log"><svg viewBox="0 0 24 24"><use href="#i-scroll"/></svg>Open log</button>`,
    `<button class="nav row-btn" data-key="openfolder"><svg viewBox="0 0 24 24"><use href="#i-open"/></svg>Open game folder</button>`,
    `<div class="cmd" id="cmd-preview">…</div>`,
  ].filter(Boolean).join("");

  const on = (key, ev, fn) => rows.querySelector(`[data-key="${key}"]`)?.addEventListener(ev, fn);
  on("folder", "click", () => locate(g));
  on("runner", "cycle", (e) => {
    const ids = [null, ...snap.runners.map((r) => r.id)];
    const cur = Math.max(0, ids.indexOf(g.runner_choice ?? null));
    const next = ids[(cur + e.detail + ids.length) % ids.length];
    act(() => api.call("set_runner", { game: g.id, runner: next }));
  });
  on("runner", "click", (e) => e.currentTarget.dispatchEvent(new CustomEvent("cycle", { detail: 1 })));
  on("res", "cycle", (e) => {
    const opts = ["", ...RESOLUTIONS];
    const i = (opts.indexOf(res) + e.detail + opts.length) % opts.length;
    saveDisplay({ resolution: opts[i] });
  });
  on("res", "click", (e) => e.currentTarget.dispatchEvent(new CustomEvent("cycle", { detail: 1 })));
  on("windowed", "click", () => saveDisplay({ windowed: !g.windowed }));
  on("aotr-get", "click", () => act(() => api.call("open_url", { url: AOTR_URL }), "Opened ModDB in your browser"));
  on("aotr-install", "click", () => runInstaller(g));
  on("install", "click", () => runInstaller(g));
  on("edition", "cycle", (e) => {
    if (installedEds.length < 2) return;
    const i = Math.max(0, installedEds.findIndex((x) => x.active));
    const next = installedEds[(i + e.detail + installedEds.length) % installedEds.length];
    act(() => api.call("set_edition", { game: g.id, id: next.id })).then(() => loadCommunity(g.id)).then(render);
  });
  on("edition", "click", (e) => e.currentTarget.dispatchEvent(new CustomEvent("cycle", { detail: 1 })));
  for (const st of comm.steps) {
    const page = () => act(() => api.call("open_url", { url: st.url }), `Opened ${st.title} in your browser`);
    on(`page:${st.id}`, "click", page);
    on(`folder:${st.id}`, "click", async () => {
      const path = await api.call("pick_folder");
      if (path) addCommunityFiles(g, st, path);
    });
    on(`step:${st.id}`, "click", async () => {
      if (st.action === "link") return page();
      const path = await api.call("pick_file", { kind: st.action === "installer" ? "exe" : "download" });
      if (!path) return;
      if (st.action === "installer") return runInstaller(g, path, st.id);
      addCommunityFiles(g, st, path);
    });
  }
  on("store", "click", () => act(() => api.call("open_url", { url: STORE[g.id] }), "Opened the store page"));
  on("vdesk", "click", () => act(() => api.call("set_launch_options", { game: g.id, virtualDesktop: !g.virtual_desktop, viaSteam: null })));
  on("steam", "click", () => act(() => api.call("set_launch_options", { game: g.id, virtualDesktop: null, viaSteam: !g.via_steam })));
  on("mordorwide", "click", () => act(() => api.call("open_url", { url: "https://mordorwi.de/howtoplay" }), "Opened the MordorWide guide"));
  on("aotr-launcher", "click", () => runInstaller(g, g.tool_launcher));
  const key = rows.querySelector("#f-cdkey");
  key?.addEventListener("change", () => act(() => api.call("set_cd_key", { game: g.id, key: key.value }), key.value.trim() ? "CD key saved" : "CD key cleared"));
  const args = rows.querySelector("#f-args");
  args.addEventListener("change", () => saveDisplay({ extraArgs: args.value }));
  on("installer", "click", () => runInstaller(g));
  on("log", "click", () => act(() => api.call("open_log", { game: g.id })));
  on("openfolder", "click", () => g.path && act(() => api.call("open_path", { path: g.path })));

  api
    .call("launch_preview", { game: g.id, modId: null })
    .then((s) => ($("#cmd-preview").textContent = s))
    .catch((e) => ($("#cmd-preview").textContent = String(e?.message ?? e)));
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function runInstaller(g, path, step = null) {
  path ||= await api.call("pick_file", { kind: "exe" });
  if (!path) return;
  installing = g.id;
  if (!(await act(() => api.call("run_installer", { game: g.id, path, step })))) installing = null;
}

async function addCommunityFiles(g, st, path) {
  showOverlay({ title: `Adding ${st.title}…`, text: "Extracting and copying files. Anything replaced is backed up first." });
  try {
    const [next, msg] = await api.call("add_files", { game: g.id, step: st.id, path });
    snap = next;
    await loadCommunity(g.id);
    hideOverlay();
    render();
    toast(msg);
  } catch (e) {
    showOverlay({ title: `Couldn't add ${st.title}`, text: String(e?.message ?? e), error: true, ok: true });
  }
}

function saveDisplay(change) {
  const g = game();
  const next = { resolution: resText(g.resolution), windowed: g.windowed, extraArgs: g.extra_args, ...change };
  return act(() => api.call("set_display", { game: g.id, ...next }));
}

// Launcher settings
function renderLauncherSheet() {
  const keep = nav.current()?.dataset.key;
  const def = snap.runners.find((r) => r.id === snap.default_runner);
  const best = snap.runners.find((r) => r.id === snap.best_runner);
  const rows = $("#launcher-rows");
  rows.innerHTML = [
    `<p class="section-label">Games</p>`,
    `<button class="nav row-btn" data-key="scan"><svg viewBox="0 0 24 24"><use href="#i-scan"/></svg>Find installed games</button>`,
    `<p class="section-label">Default runner</p>`,
    rowHTML({ key: "defrunner", label: "Runner for all games", sub: "Each game can override this", value: def ? def.name : `Automatic${best ? ` (${best.name})` : ""}`, cycle: true }),
    `<p class="section-label">Runners found</p>`,
    ...(snap.runners.length
      ? snap.runners.map((r) => `<div class="row" style="cursor:default"><span style="min-width:0"><span class="label">${esc(r.name)}</span><span class="sub">${esc(r.path)}</span></span><span class="badge">${esc(r.kind)}</span></div>`)
      : [`<p class="section-label">None — install GE-Proton (ProtonPlus or ProtonUp-Qt), umu-launcher, or Wine.</p>`]),
    `<p class="section-label">Storage</p>`,
    `<button class="nav row-btn" data-key="data"><svg viewBox="0 0 24 24"><use href="#i-folder"/></svg>Open Beacon's data folder (prefixes, mods, logs)</button>`,
    `<p class="section-label">About</p>`,
    `<div class="row" style="cursor:default"><span><span class="label">Beacon ${esc(snap.version)}</span><span class="sub">Not affiliated with EA or the Tolkien Estate. Bring your own copy of the games.</span></span></div>`,
  ].join("");
  const on = (key, ev, fn) => rows.querySelector(`[data-key="${key}"]`)?.addEventListener(ev, fn);
  on("scan", "click", scan);
  on("defrunner", "cycle", (e) => {
    const ids = [null, ...snap.runners.map((r) => r.id)];
    const next = ids[(ids.indexOf(snap.default_runner) + e.detail + ids.length) % ids.length];
    act(() => api.call("set_runner", { game: null, runner: next }));
  });
  on("defrunner", "click", (e) => e.currentTarget.dispatchEvent(new CustomEvent("cycle", { detail: 1 })));
  on("data", "click", () => act(() => api.call("open_path", { path: snap.data_dir })));
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function scan() {
  const before = snap.games.filter((g) => g.installed).length;
  if (await act(() => api.call("scan"))) {
    const after = snap.games.filter((g) => g.installed).length;
    toast(after > before ? `Found ${after - before} more game${after - before > 1 ? "s" : ""}` : after ? "Up to date" : "No games found — set each folder in Game settings");
  }
}

function closeAllSheets() {
  while (openSheet) closeSheet();
}

function closeSheet() {
  if (!openSheet) return;
  sheets.pop();
  openSheet = sheets.at(-1) ?? null;
  nav.popLayer();
  render();
}

// ---------------------------------------------------------------- launch overlay

function showOverlay({ title, text, error = false, ok = false }) {
  const o = $("#overlay");
  $("#overlay-title").textContent = title;
  $("#overlay-text").textContent = text || "";
  o.classList.toggle("error", error);
  $("#overlay-ok").hidden = !ok;
  if (o.hidden) nav.pushLayer(o, ok ? $("#overlay-ok") : null);
  else if (ok) nav.focus($("#overlay-ok"));
}

function hideOverlay() {
  if ($("#overlay").hidden) return;
  overlayState = null;
  nav.popLayer();
}

async function onGameEvent({ game: id, phase, message }) {
  const g = snap.games.find((x) => x.id === id);
  const short = g?.short ?? id;
  overlayState = { game: id, phase };
  const inst = installing === id;
  if (phase === "preparing") {
    snap.running = [...new Set([...snap.running, id])];
    showOverlay({ title: "Lighting the beacon…", text: `Getting ${short}'s Wine prefix ready. The first time can take a minute.` });
  } else if (phase === "running") {
    showOverlay(
      inst
        ? { title: "Installer running", text: id === "aotr" ? "The Age of the Ring installer can run silently for 15–20 minutes with no progress bar. Leave it be — Beacon finds the game when it finishes." : "Follow the installer's window. Beacon picks up anything it installs when it closes." }
        : { title: `${short} is running`, text: "Beacon will be right here when you're done." }
    );
  } else {
    installing = null;
    const wasInstalled = g?.installed;
    snap = await api.call("get_state");
    await loadMods(id, true).catch(() => {});
    await loadCommunity(id).catch(() => {});
    if (inst && !wasInstalled && snap.games.find((x) => x.id === id)?.installed) toast(`${short} is installed and ready`);
    if (phase === "error" || message) showOverlay({ title: phase === "error" ? `${short} couldn't start` : `${short} closed`, text: message || "", error: true, ok: true });
    else hideOverlay();
  }
  render();
}

// ---------------------------------------------------------------- wiring

nav.onBack(() => {
  if (!$("#overlay").hidden) {
    if (!$("#overlay-ok").hidden) hideOverlay();
    return;
  }
  closeSheet();
});

nav.onShortcut((b) => {
  if (!$("#overlay").hidden) return;
  if (b === "lb" || b === "rb") {
    if (openSheet) return;
    select(sel + (b === "lb" ? -1 : 1));
    const tile = document.querySelector(`.tile[data-index="${sel}"]`);
    if (nav.current()?.classList.contains("tile")) nav.focus(tile);
  } else if (b === "x") {
    if (openSheet === "game") closeSheet();
    else if (!openSheet) openGameSheet();
  } else if (b === "y") {
    if (openSheet === "mods") closeSheet();
    else if (!openSheet) loadMods(game().id).then(() => open("mods", $("#sheet-mods")));
  } else if (b === "start" && !openSheet) play();
  else if (b === "select") {
    if (openSheet === "launcher") closeSheet();
    else if (!openSheet) open("launcher", $("#sheet-launcher"));
  }
});

nav.onInputChange(renderHints);

$("#btn-play").addEventListener("click", play);
$("#btn-mods").addEventListener("click", () => loadMods(game().id).then(() => open("mods", $("#sheet-mods"))));
const openGameSheet = () => loadCommunity(game().id).catch(() => {}).then(() => open("game", $("#sheet-game")));
$("#btn-game-settings").addEventListener("click", openGameSheet);
$("#btn-launcher-settings").addEventListener("click", () => open("launcher", $("#sheet-launcher")));
$("#btn-scan").addEventListener("click", scan);
$("#btn-add-mod-file").addEventListener("click", () => addMod("file"));
$("#btn-add-mod-folder").addEventListener("click", () => addMod("folder"));
$("#btn-catalog").addEventListener("click", openCatalog);
$("#btn-find-mods").addEventListener("click", findMods);
$("#btn-catalog-add").addEventListener("click", () => addMod("file"));
$("#btn-catalog-find").addEventListener("click", findMods);
$("#btn-open-mods").addEventListener("click", () => act(() => api.call("open_path", { path: `${snap.mods_dir}/${game().id}` })));
$("#overlay-ok").addEventListener("click", hideOverlay);
// Clicking the dimmed area left of a sheet closes it.
document.addEventListener("pointerdown", (e) => {
  if (openSheet && !e.target.closest(".sheet") && !e.target.closest("#overlay")) closeSheet();
});

function tick() {
  $("#clock").textContent = new Date().toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

(async function init() {
  snap = await api.call("get_state");
  // Start on the first installed game.
  sel = Math.max(0, snap.games.findIndex((g) => g.installed));
  buildShelf();
  renderHints(false);
  render();
  await Promise.all(snap.games.map((g) => loadMods(g.id).catch(() => [])));
  render();
  nav.focus($("#btn-play"));
  api.on("game-event", onGameEvent);
  tick();
  setInterval(tick, 15000);
})();
