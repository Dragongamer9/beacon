import { api } from "./api.js";
import * as nav from "./nav.js";

const $ = (s, r = document) => r.querySelector(s);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

let snap = null;
let sel = 0;
const sheets = []; // stack of open sheets: "mods" | "catalog" | "workshop" | "game" | "launcher"
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

// The shelf: Beacon's own games, then the games added by hand (Other Games), then the "Add a game" tile.
const ADD_ITEM = { id: "add", add: true, title: "Add a game", short: "Add a game", family: "other", installed: true, ready: true, issues: [] };
const customItem = (c) => ({ ...c, id: `custom:${c.id}`, cid: c.id, custom: true, short: c.title, family: "other", runner_pick: c.runner, runner: c.resolved_runner, runner_choice: c.runner });
let itemsCache = { snap: null, list: [] };
function items() {
  if (itemsCache.snap !== snap) itemsCache = { snap, list: [...snap.games, ...(snap.custom || []).map(customItem), ADD_ITEM] };
  return itemsCache.list;
}
const game = () => items()[sel] ?? items()[0];
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
// Games the BFME workshop (bfmeladder.com) has fan content for.
const WORKSHOP_GAMES = ["bfme1", "bfme2", "rotwk"];
const fmtSize = (b) => (b >= 1073741824 ? `${(b / 1073741824).toFixed(1)} GB` : b >= 1048576 ? `${Math.max(1, Math.round(b / 1048576))} MB` : b > 0 ? `${Math.max(1, Math.round(b / 1024))} KB` : "");
const communityCache = {};
async function loadCommunity(id) {
  communityCache[id] = await api.call("get_community", { game: id });
  return communityCache[id];
}
const isSage = (g) => g.family === "bfme";

function emblemHTML(g) {
  if (g.custom && g.icon_url) return `<img class="emblem" src="${esc(g.icon_url)}" alt="">`;
  return `<svg class="emblem" viewBox="0 0 120 120"><use href="#emblem-${g.add ? "add" : g.custom ? "custom" : g.id}"/></svg>`;
}

function shelfKey() {
  return items().map((g) => `${g.id}|${g.short}|${g.icon_url ? g.icon_url.length : 0}`).join(",");
}

function buildShelf() {
  const shelf = $("#shelf");
  buildShelf.key = shelfKey();
  const tile = (g, i) => `
      <button class="nav tile" data-group="shelf" data-game="${esc(g.id)}" data-index="${i}">
        ${emblemHTML(g)}
        <span style="min-width:0"><span class="t-short">${esc(g.short)}</span><span class="t-state"><span class="dot"></span><span class="t-text"></span></span></span>
      </button>`;
  shelf.innerHTML = FAMILIES.map(([fam, label]) => {
    const members = items().map((g, i) => [g, i]).filter(([g]) => g.family === fam);
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
  sel = (i + items().length) % items().length;
  render();
  const g = game();
  if (g.custom) loadCustomArt(g).then(renderHero).catch(() => {});
  else if (!g.add) loadMods(g.id).then(renderHero).catch(() => {});
}

function renderShelf() {
  const current = document.querySelector(`.tile[data-index="${sel}"]`);
  if (current && renderShelf.last !== sel) {
    renderShelf.last = sel;
    current.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }
  document.querySelectorAll(".tile").forEach((tile) => {
    const g = items()[Number(tile.dataset.index)];
    if (!g) return;
    tile.classList.toggle("selected", g === game());
    tile.classList.toggle("is-ready", g.ready && !g.add);
    tile.classList.toggle("is-missing", !g.installed);
    tile.querySelector(".t-text").textContent = g.add
      ? "Windows game or installer"
      : isRunning(g.id) ? "Running" : g.ready ? (g.custom && g.play_seconds ? fmtPlay(g.play_seconds) : "Ready") : g.installed ? "Needs attention" : g.custom ? "Not installed" : "Not found";
  });
}

function renderHero() {
  const g = game();
  if (g.custom || g.add) return renderCustomHero(g);
  $("#hero-icon").hidden = true;
  $("#hero-art").classList.remove("on");
  $(".hero-emblem").style.display = "";
  $("#btn-mods").hidden = false;
  $("#btn-game-settings").hidden = false;
  $("#btn-mods").querySelector("use").setAttribute("href", "#i-scroll");
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
    label.textContent = g.id === "aotr" ? "Install Age of the Ring" : `Install or locate ${g.short}`;
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
  if (buildShelf.key !== shelfKey()) {
    sel = Math.min(sel, items().length - 1);
    buildShelf();
  }
  renderShelf();
  renderHero();
  // Only the top sheet re-renders; ones underneath refresh when uncovered.
  if (openSheet === "mods") renderModsSheet();
  if (openSheet === "catalog") renderCatalogSheet();
  if (openSheet === "workshop") renderWorkshopSheet();
  if (openSheet === "game") renderGameSheet();
  if (openSheet === "launcher") renderLauncherSheet();
  if (openSheet === "add") renderAddSheet();
  if (openSheet === "custom") renderCustomSheet();
  if (openSheet === "wine") renderWineSheet();
  if (openSheet === "pick") renderPickSheet();
  if (openSheet === "tools") renderToolsSheet();
  if (openSheet === "tool") renderToolSheet();
}

// ---------------------------------------------------------------- actions

async function play() {
  const g = game();
  if (g.add) return open("add", $("#sheet-add"));
  if (g.custom) return playCustom(g);
  if (!g.installed) return openGameSheet();
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
  $("#btn-workshop").hidden = !WORKSHOP_GAMES.includes(g.id);
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

// Workshop: fan content from the BFME Foundation's workshop (never the base games).
const ws = { game: null, items: [], hidden: 0, page: 0, search: "", loading: false, error: null, installing: null, more: true };

async function openWorkshop() {
  const g = game();
  if (ws.game !== g.id) Object.assign(ws, { game: g.id, items: [], hidden: 0, page: 0, search: "", error: null, more: true });
  open("workshop", $("#sheet-workshop"));
  if (!ws.items.length) await loadWorkshop(0);
}

async function loadWorkshop(page) {
  const id = ws.game;
  ws.loading = true;
  ws.error = null;
  render();
  try {
    const res = await api.call("workshop_list", { game: id, search: ws.search, page });
    if (ws.game !== id) return;
    ws.items = page === 0 ? res.items : [...ws.items, ...res.items.filter((it) => !ws.items.some((x) => x.id === it.id))];
    ws.hidden = res.hidden;
    ws.page = page;
    ws.more = res.items.length + res.hidden > 0;
  } catch (e) {
    ws.error = String(e?.message ?? e);
  }
  ws.loading = false;
  render();
}

function renderWorkshopSheet() {
  const g = game();
  $("#sheet-workshop").querySelector("[data-game-short]").textContent = g.title;
  $("#btn-workshop-more").hidden = !ws.more || ws.loading || !!ws.error;
  const keep = nav.current()?.dataset.key;
  const note = (t) => `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">${t}</p>`;
  const kinds = { mod: "Mod", patch: "Patch", enhancement: "Add-on", maps: "Maps" };
  const rows = $("#workshop-rows");
  rows.innerHTML = [
    fieldHTML({ key: "ws-search", label: "Search", placeholder: "Name or words from the description", value: ws.search }),
    ws.loading ? note("Asking the workshop…") : "",
    ws.error ? note(`Couldn't reach the workshop: ${esc(ws.error)}`) : "",
    !ws.loading && !ws.error && !ws.items.length ? note("Nothing found.") : "",
    ...ws.items.map((it) =>
      rowHTML({
        key: `ws:${it.id}`,
        label: `${it.name}${it.version ? ` ${it.version}` : ""}`,
        sub: [it.author && `by ${it.author}`, fmtSize(it.size), it.description].filter(Boolean).join(" · "),
        badge: it.installed ? "Installed" : kinds[it.kind] ?? it.kind,
        value: it.installed ? (g.selected_mod === it.installed ? "" : "Use") : "Install",
        tick: !!it.installed && g.selected_mod === it.installed,
        selected: !!it.installed,
      })
    ),
    note("Fan-made content from the BFME Foundation's workshop (bfmeladder.com), installed into Beacon's mods. Base games and EA's own patches aren't listed: Beacon never downloads the games."),
  ].join("");
  const search = rows.querySelector("#f-ws-search");
  search.addEventListener("change", () => {
    ws.search = search.value.trim();
    loadWorkshop(0);
  });
  rows.querySelectorAll(".row").forEach((r) =>
    r.addEventListener("click", async () => {
      const it = ws.items.find((x) => `ws:${x.id}` === r.dataset.key);
      if (!it) return;
      if (it.installed) {
        if (await act(() => api.call("select_mod", { game: g.id, id: it.installed }))) {
          closeAllSheets();
          toast(`${it.name} selected`);
        }
      } else workshopInstall(g, it);
    })
  );
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function workshopInstall(g, it) {
  ws.installing = it.id;
  showOverlay({ title: `Installing ${it.name}…`, text: `Downloading ${fmtSize(it.size) || "the files"} from the BFME workshop.` });
  try {
    const [next, msg] = await api.call("workshop_install", { game: g.id, id: it.id });
    snap = next;
    await loadMods(g.id, true);
    ws.installing = null;
    hideOverlay();
    await loadWorkshop(0);
    toast(msg);
  } catch (e) {
    ws.installing = null;
    showOverlay({ title: `Couldn't install ${it.name}`, text: String(e?.message ?? e), error: true, ok: true });
  }
}

function onWorkshopProgress({ id, pct, text }) {
  if (ws.installing !== id || $("#overlay").hidden) return;
  $("#overlay-text").textContent = pct == null ? text : `${text} (${pct}%)`;
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
    ...(g.community_launcher ? [`<p class="section-label">${esc(g.short)} launcher</p>`, ...communityLauncherRows(g)] : []),
    `<p class="section-label">Install</p>`,
    ...(aotr && !g.installed
      ? [
          rowHTML({ key: "aotr-get", label: "1 · Get the installer", sub: "Opens ModDB: download the Standalone zip (about 14 GB) and extract it", value: "Open" }),
          rowHTML({ key: "aotr-install", label: "2 · Run the installer", sub: "Pick the installer .exe from the extracted folder", value: "Choose" }),
        ]
      : []),
    ...(!aotr && !g.installed
      ? [
          // Easiest for the BFME games: the BFME Foundation's launcher downloads them, Beacon finds them.
          ...(WORKSHOP_GAMES.includes(g.id) ? aioRows() : []),
          ...(g.store_url ? [rowHTML({ key: "store", label: "Get it on Steam", sub: "Still sold — opens the store page", value: "Open" })] : []),
          `<div class="mod-line">${rowHTML({
            key: "own-copy",
            label: "Install from your own copy",
            sub: snap.native ? "Your disc's installer, a disc image (.iso) or setup files" : `Your disc's installer, a disc image (.iso) or setup files — it installs into ${g.short}'s own Wine prefix`,
            value: "Choose",
          })}<button class="nav side" data-key="own-copy-folder">Disc folder</button></div>`,
        ]
      : []),
    rowHTML({ key: "folder", label: !g.installed ? "Already installed? Locate it" : "Game folder", sub: g.path || (aotr ? "The Age of the Ring folder, e.g. …/drive_c/AgeoftheRing" : "Not set"), value: "Change" }),
    !sage || aotr ? "" : fieldHTML({
      key: "cdkey",
      label: "CD key",
      placeholder: g.has_cd_key ? "Saved — type a new one to replace it" : "Key from your copy of the game",
      help: "Stored in this game's own Wine prefix, the way the original installer did.",
    }),
    ...(snap.native
      ? []
      : [`<p class="section-label">Compatibility</p>`, rowHTML({ key: "runner", label: "Runner", sub: "Wine or Proton build used for this game", value: runnerName, cycle: true })]),
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
          ...(g.via_steam || snap.native
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
    ...(g.installed && WORKSHOP_GAMES.includes(g.id) && snap.aio_launcher?.installed
      ? [`<button class="nav row-btn" data-key="aio-open"><svg viewBox="0 0 24 24"><use href="#i-open"/></svg>Open the All-in-One launcher (patches, workshop, online)</button>`]
      : []),
    ...(aotr && g.tool_launcher
      ? [`<button class="nav row-btn" data-key="aotr-launcher"><svg viewBox="0 0 24 24"><use href="#i-open"/></svg>Open the AotR Launcher for updates (may not work under Wine)</button>`]
      : []),
    `<button class="nav row-btn" data-key="installer"><svg viewBox="0 0 24 24"><use href="#i-plus"/></svg>${snap.native ? "Run a patch or installer for this game" : "Run a patch or installer in this game's prefix"}</button>`,
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
  on("own-copy", "click", () => installFromCopy(g, false));
  on("own-copy-folder", "click", () => installFromCopy(g, true));
  wireAio(on);
  wireCommunityLauncher(on, g);
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
  on("store", "click", () => act(() => api.call("open_url", { url: g.store_url }), "Opened the store page"));
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

// Install from the player's own copy: installer .exe, disc image (.iso) or disc folder.
async function installFromCopy(g, folder) {
  const path = folder ? await api.call("pick_folder") : await api.call("pick_file", { kind: "copy" });
  if (!path) return;
  installing = g.id;
  if (!(await act(() => api.call("install_from", { game: g.id, path })))) installing = null;
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

// The BFME All-in-One launcher: Beacon installs it from the BFME Foundation's official download and opens it.
// The games are downloaded inside the launcher (never by Beacon); Beacon then finds them.
let aioInstalling = false;
let aioOpenedAt = 0;

function aioRows() {
  const l = snap.aio_launcher || { installed: false, can_install: true };
  const rows = [];
  if (l.installed) {
    rows.push(rowHTML({ key: "aio-open", label: "Open the All-in-One launcher", sub: snap.native ? l.path : `Runs in its own Wine prefix · ${l.path}`, value: "Open" }));
  } else {
    rows.push(
      rowHTML({
        key: "aio-install",
        label: "Get the All-in-One launcher",
        sub: !l.can_install
          ? "It's a Windows program: install GE-Proton, umu-launcher or Wine first"
          : snap.native
            ? "Beacon downloads the BFME Foundation's launcher from bfmeladder.com and installs it"
            : "Beacon downloads the BFME Foundation's launcher from bfmeladder.com and installs it in its own Wine prefix",
        value: "Install",
      })
    );
  }
  rows.push(
    `<div class="mod-line">${rowHTML({ key: "aio-scan", label: "Games are downloaded in the All-in-One launcher", sub: "Beacon picks them up automatically — or look now", value: "Find games" })}<button class="nav side" data-key="aio-site">bfmeladder.com</button></div>`
  );
  return rows;
}

function wireAio(on) {
  on("aio-install", "click", installAioLauncher);
  on("aio-open", "click", openAioLauncher);
  on("aio-scan", "click", scan);
  on("aio-site", "click", () => act(() => api.call("open_url", { url: snap.aio_url }), "Opened bfmeladder.com in your browser"));
}

async function installAioLauncher() {
  if (aioInstalling) return;
  if (snap.aio_launcher && !snap.aio_launcher.can_install) return toast("The All-in-One launcher is a Windows program: install GE-Proton, umu-launcher or Wine first", true);
  aioInstalling = true;
  showOverlay({
    title: "Getting the All-in-One launcher…",
    text: snap.native
      ? "Downloading the BFME Foundation's launcher from bfmeladder.com."
      : "Setting up its Wine prefix and downloading the BFME Foundation's launcher from bfmeladder.com. The first time can take a few minutes.",
  });
  try {
    const [next, msg] = await api.call("aio_launcher_install");
    snap = next;
    if (openSheet === "launcher") aioCache = await api.call("aio_info").catch(() => aioCache);
    hideOverlay();
    render();
    toast(`${msg} — open it to download the games`);
  } catch (e) {
    showOverlay({ title: "Couldn't get the All-in-One launcher", text: String(e?.message ?? e), error: true, ok: true });
  }
  aioInstalling = false;
}

function onAioProgress({ pct, text }) {
  if (!aioInstalling || $("#overlay").hidden) return;
  $("#overlay-text").textContent = pct == null ? text : `${text} (${pct}%)`;
}

async function openAioLauncher() {
  if (await act(() => api.call("aio_launcher_open"), "Started the All-in-One launcher — download the games there; Beacon picks them up")) aioOpenedAt = Date.now();
}

// After the launcher: look for the games it downloaded (cheap; get_state picks them up).
async function refreshAfterAio(announce) {
  const before = snap.games.filter((g) => g.installed).length;
  snap = await api.call("get_state");
  if (openSheet === "launcher") aioCache = await api.call("aio_info").catch(() => aioCache);
  render();
  const after = snap.games.filter((g) => g.installed).length;
  if (after > before) toast(`Found ${after - before} new game${after - before > 1 ? "s" : ""} from the All-in-One launcher`);
  else if (announce) toast("The All-in-One launcher closed");
}

// A game's community launcher (Universe at War's): Beacon only opens the page it's distributed from; the player
// downloads the version they want and picks it, Beacon keeps a copy and runs it (Linux: in the game's own prefix).
// The game is downloaded inside it; Beacon finds it afterwards.
const CL_HOW_TO = "Download the launcher version you want, then choose it below. Games are downloaded inside it; Beacon finds them afterwards.";
let clBusy = null; // game id while choosing or starting its launcher

function communityLauncherRows(g) {
  const cl = g.community_launcher;
  if (!cl) return [];
  return [
    rowHTML({ key: `cl-get:${g.id}`, label: `Get the ${g.short} launcher`, sub: CL_HOW_TO, value: "Open" }),
    rowHTML({ key: `cl-set:${g.id}`, label: "Use a launcher you downloaded", sub: cl.path || "Its .exe, or the .zip it came in", value: cl.path ? "Change" : "Choose" }),
    ...(cl.path
      ? [
          rowHTML({
            key: `cl-open:${g.id}`,
            label: `Open the ${g.short} launcher`,
            sub: snap.native ? "Download the game there; Beacon finds it afterwards" : `Runs in ${g.short}'s own Wine prefix — download the game there; Beacon finds it afterwards`,
            value: "Open",
          }),
        ]
      : []),
  ];
}

function wireCommunityLauncher(on, g) {
  if (!g.community_launcher) return;
  on(`cl-get:${g.id}`, "click", () => act(() => api.call("open_url", { url: g.community_launcher.url }), "Opened the download folder in your browser"));
  on(`cl-set:${g.id}`, "click", () => chooseCommunityLauncher(g));
  on(`cl-open:${g.id}`, "click", () => openCommunityLauncher(g));
}

async function chooseCommunityLauncher(g) {
  if (clBusy) return;
  const path = await api.call("pick_file", { kind: "launcher" });
  if (!path) return;
  clBusy = g.id;
  showOverlay({ title: `Setting up the ${g.short} launcher…`, text: /\.zip$/i.test(path) ? "Extracting it into Beacon's tools folder." : "Copying it into Beacon's tools folder." });
  try {
    const [next, msg] = await api.call("community_launcher_set", { game: g.id, path });
    snap = next;
    hideOverlay();
    render();
    toast(`${msg} — open it to download the game`);
  } catch (e) {
    showOverlay({ title: `Couldn't use that ${g.short} launcher`, text: String(e?.message ?? e), error: true, ok: true });
  }
  clBusy = null;
}

async function openCommunityLauncher(g) {
  if (clBusy) return;
  clBusy = g.id;
  showOverlay({ title: `Starting the ${g.short} launcher…`, text: snap.native ? "" : `Getting ${g.short}'s Wine prefix ready. The first time can take a few minutes.` });
  try {
    snap = await api.call("community_launcher_open", { game: g.id });
    hideOverlay();
    render();
    toast(`Started the ${g.short} launcher — download the game there; Beacon picks it up when it closes`);
  } catch (e) {
    showOverlay({ title: `Couldn't start the ${g.short} launcher`, text: String(e?.message ?? e), error: true, ok: true });
  }
  clBusy = null;
}

function onCommunityLauncherProgress({ game: id, pct, text }) {
  if (clBusy !== id || $("#overlay").hidden) return;
  $("#overlay-text").textContent = pct == null ? text : `${text} (${pct}%)`;
}

async function onCommunityLauncherClosed({ game: id, found }) {
  snap = await api.call("get_state");
  render();
  const g = snap.games.find((x) => x.id === id);
  toast(found ? `Found ${g?.short ?? id} — it's ready in Beacon` : `The ${g?.short ?? id} launcher closed`);
}

// Launcher settings
function renderLauncherSheet() {
  const keep = nav.current()?.dataset.key;
  const def = snap.runners.find((r) => r.id === snap.default_runner);
  const best = snap.runners.find((r) => r.id === snap.best_runner);
  const rows = $("#launcher-rows");
  const aio = aioCache?.games ?? [];
  rows.innerHTML = [
    `<p class="section-label">Games</p>`,
    `<button class="nav row-btn" data-key="scan"><svg viewBox="0 0 24 24"><use href="#i-scan"/></svg>Find installed games</button>`,
    ...(snap.native
      ? []
      : [
          `<p class="section-label">Default runner</p>`,
          rowHTML({ key: "defrunner", label: "Runner for all games", sub: "Each game can override this", value: def ? def.name : `Automatic${best ? ` (${best.name})` : ""}`, cycle: true }),
          `<button class="nav row-btn" data-key="tools"><svg viewBox="0 0 24 24"><use href="#i-download"/></svg>Compatibility tools: get GE-Proton, UMU-Proton, Wine, DXVK… for Beacon, Steam, Lutris, Heroic and Bottles</button>`,
          `<p class="section-label">Runners found</p>`,
          ...(snap.runners.length
            ? snap.runners.map((r) => `<div class="row" style="cursor:default"><span style="min-width:0"><span class="label">${esc(r.name)}</span><span class="sub">${esc(r.path)}</span></span><span class="badge">${esc(r.kind)}</span></div>`)
            : [`<p class="section-label">None yet — get GE-Proton or UMU-Proton under Compatibility tools.</p>`]),
        ]),
    `<p class="section-label">BFME All-in-One launcher</p>`,
    ...aio.map((a) => {
      const gg = snap.games.find((x) => x.id === a.game);
      return `<div class="row" style="cursor:default"><span style="min-width:0"><span class="label">${esc(gg?.short ?? a.game)}${a.active_patch ? ` · ${esc(a.active_patch)}` : ""}</span><span class="sub">${esc(a.dir)}</span></span><span class="badge">Found</span></div>`;
    }),
    ...aioRows(),
    ...snap.games.filter((x) => x.community_launcher).flatMap((x) => [`<p class="section-label">${esc(x.title)} launcher</p>`, ...communityLauncherRows(x)]),
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
  on("tools", "click", openToolsSheet);
  wireAio(on);
  snap.games.filter((x) => x.community_launcher).forEach((x) => wireCommunityLauncher(on, x));
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

let aioCache = null;
async function openLauncherSheet() {
  aioCache = await api.call("aio_info").catch(() => null);
  open("launcher", $("#sheet-launcher"));
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
    showOverlay({
      title: inst ? "Getting the installer ready…" : "Lighting the beacon…",
      text: message || (snap.native ? `Starting ${short}.` : `Getting ${short}'s Wine prefix ready. The first time can take a minute.`),
    });
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

// ---------------------------------------------------------------- games added by hand (Faugus-style)
// Any Windows program or installer, each in its own Wine prefix with its own runner and options. They sit under
// Other Games; the last tile adds one.

const fmtPlay = (s) => (s >= 3600 ? `${(s / 3600).toFixed(s >= 36000 ? 0 : 1)} h played` : s >= 60 ? `${Math.round(s / 60)} min played` : "Played");
const fmtDate = (t) => (t ? new Date(t * 1000).toLocaleDateString([], { day: "numeric", month: "short", year: "numeric" }) : "");
const artCache = {};
const note = (t) => `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">${t}</p>`;
const cg = (cid) => (snap.custom || []).find((c) => c.id === cid);

async function loadCustomArt(g) {
  if (!artCache[g.cid]) artCache[g.cid] = await api.call("custom_art", { id: g.cid }).catch(() => ({}));
  return artCache[g.cid];
}

function renderCustomHero(g) {
  document.body.dataset.accent = g.id;
  const art = g.custom ? artCache[g.cid] || {} : {};
  const heroArt = $("#hero-art");
  heroArt.style.backgroundImage = art.hero ? `url("${art.hero}")` : "";
  heroArt.classList.toggle("on", !!art.hero);
  const icon = $("#hero-icon");
  icon.hidden = !(g.custom && (art.cover || g.icon_url));
  if (!icon.hidden) icon.src = art.cover || g.icon_url;
  $(".hero-emblem").style.display = icon.hidden ? "" : "none";
  $("#hero-emblem").setAttribute("href", g.add ? "#emblem-add" : "#emblem-custom");
  $("#hero-title").textContent = g.title;
  $("#btn-mods").hidden = !g.custom;
  $("#btn-game-settings").hidden = !g.custom;
  const play = $("#btn-play");
  const label = $("#play-label");
  const issue = $("#hero-issue");
  if (g.add) {
    $("#hero-kicker").textContent = "Other Games";
    $("#hero-chips").innerHTML = (snap.custom || []).length ? `<span class="chip">${snap.custom.length} added</span>` : "";
    issue.hidden = false;
    issue.textContent = snap.native
      ? "Add any Windows game or installer. Beacon keeps its icon, arguments and play time."
      : "Add any Windows game or installer. Each one gets its own Wine prefix, with the runner and options you pick — like Faugus Launcher, inside Beacon.";
    label.textContent = "Add a game";
    play.disabled = false;
    return;
  }
  $("#hero-kicker").textContent = g.source === "faugus" ? "From Faugus Launcher" : "Added in Beacon";
  const running = isRunning(g.id);
  const chips = [];
  if (running) chips.push(`<span class="chip run"><span class="dot"></span>Running</span>`);
  else if (g.ready) chips.push(`<span class="chip ok"><span class="dot"></span>Ready</span>`);
  else chips.push(`<span class="chip bad"><span class="dot"></span>${g.installed ? "Needs attention" : "Not installed"}</span>`);
  if (g.runner && !snap.native) chips.push(`<span class="chip">${esc(g.runner.name)}</span>`);
  if (g.play_seconds) chips.push(`<span class="chip">${fmtPlay(g.play_seconds)}</span>`);
  if (g.last_played) chips.push(`<span class="chip">Last played ${esc(fmtDate(g.last_played))}</span>`);
  $("#hero-chips").innerHTML = chips.join("");
  const err = g.issues.find((i) => i.level === "error");
  const warn = g.issues.find((i) => i.level === "warn");
  issue.hidden = !(err || warn);
  issue.innerHTML = err ? `<strong>Can't start yet.</strong> ${esc(err.message)}` : warn ? esc(warn.message) : "";
  if (!g.exe && g.installer) {
    label.textContent = "Choose the game's program";
    play.disabled = running;
  } else {
    label.textContent = running ? "Running…" : "Play";
    play.disabled = running || !g.ready;
  }
  $("#mods-label").textContent = "Wine tools";
  $("#btn-mods").querySelector("use").setAttribute("href", "#i-wine");
}

async function playCustom(g) {
  if (!g.exe && g.installer) return openPick(g);
  if (!g.ready || isRunning(g.id)) return;
  try {
    await api.call("custom_launch", { id: g.cid });
  } catch (e) {
    toast(String(e?.message ?? e), true);
  }
}

let customJob = null; // { id, job } while something we started for a game added by hand runs

async function onCustomEvent({ id, phase, job, message }) {
  const key = `custom:${id}`;
  const c = cg(id);
  const title = c?.title ?? id;
  if (phase === "preparing") {
    customJob = { id, job };
    snap.running = [...new Set([...snap.running, key])];
    showOverlay({
      title: job === "launch" ? "Lighting the beacon…" : job === "install" ? "Getting the installer ready…" : "Getting the prefix ready…",
      text: snap.native ? `Starting ${title}.` : `Getting ${title}'s Wine prefix ready. The first time can take a minute.`,
    });
  } else if (phase === "running") {
    if (job === "tool") {
      hideOverlay();
      toast(`Started in ${title}'s prefix`);
    } else {
      showOverlay(
        job === "launch"
          ? { title: `${title} is running`, text: "Beacon will be right here when you're done." }
          : { title: job === "install" ? "Installer running" : "Program running", text: "Follow its window. Beacon takes over again when it closes." }
      );
    }
  } else {
    customJob = null;
    snap = await api.call("get_state");
    if (job === "install" && phase !== "error") {
      // onInstallDone opens the program picker.
      hideOverlay();
    } else if (phase === "error" || message) {
      showOverlay({ title: phase === "error" ? `${title} couldn't start` : `${title} closed`, text: message || "", error: true, ok: true });
    } else hideOverlay();
  }
  render();
}

let pickFor = null; // { cid, candidates }

function onInstallDone({ id, candidates }) {
  pickFor = { cid: id, candidates };
  api.call("get_state").then((s) => {
    snap = s;
    const i = items().findIndex((x) => x.cid === id);
    if (i >= 0) sel = i;
    render();
    open("pick", $("#sheet-pick"));
  });
}

async function openPick(g) {
  pickFor = { cid: g.cid, candidates: await api.call("custom_candidates", { id: g.cid }).catch(() => []) };
  open("pick", $("#sheet-pick"));
}

function renderPickSheet() {
  const c = cg(pickFor?.cid);
  if (!c) return closeSheet();
  $("#sheet-pick").querySelector("[data-custom-title]").textContent = c.title;
  const rows = $("#pick-rows");
  const base = (p) => p.split("/").pop();
  rows.innerHTML = [
    note(pickFor.candidates.length ? "Programs the installer put in the game's prefix, likeliest first:" : "Beacon didn't find a program in the game's prefix. Choose it yourself — or run the installer again from Wine tools."),
    ...pickFor.candidates.map((p, i) => rowHTML({ key: `pick:${i}`, label: base(p), sub: p, value: "Use" })),
    `<button class="nav row-btn" data-key="browse"><svg viewBox="0 0 24 24"><use href="#i-folder"/></svg>Choose another program…</button>`,
    `<button class="nav row-btn" data-key="later"><svg viewBox="0 0 24 24"><use href="#i-x"/></svg>Later (Play asks again)</button>`,
  ].join("");
  const use = async (path) => {
    if (await act(() => api.call("custom_set", { id: c.id, key: "exe", value: path }))) {
      closeAllSheets();
      toast(`${c.title} is ready to play`);
    }
  };
  pickFor.candidates.forEach((p, i) => rows.querySelector(`[data-key="pick:${i}"]`).addEventListener("click", () => use(p)));
  rows.querySelector('[data-key="browse"]').addEventListener("click", async () => {
    const p = await api.call("pick_file", { kind: "game" });
    if (p) use(p);
  });
  rows.querySelector('[data-key="later"]').addEventListener("click", closeSheet);
  nav.refocus();
}

// Add a game
function renderAddSheet() {
  const rows = $("#add-rows");
  rows.innerHTML = [
    rowHTML({ key: "add-exe", label: "Add a Windows program", sub: "A game's .exe — its icon is taken from it", value: "Choose" }),
    rowHTML({ key: "add-install", label: "Install from an installer", sub: snap.native ? "Run setup.exe or an .msi, then pick the game it installed" : "Runs setup.exe or an .msi in a new Wine prefix, then you pick the game it installed", value: "Choose" }),
    `<p class="section-label">Your library</p>`,
    ...(snap.faugus ? [rowHTML({ key: "import-faugus", label: "Import from Faugus Launcher", sub: snap.faugus, value: "Import" })] : []),
    rowHTML({ key: "import-file", label: "Import from a file", sub: "A Beacon export, or Faugus Launcher's games.json", value: "Choose" }),
    rowHTML({ key: "export", label: "Export your games", sub: "Every game you added and its play time, for another PC or a backup", value: "Save" }),
    ...(snap.native ? [] : [`<p class="section-label">Runners</p>`, `<button class="nav row-btn" data-key="tools"><svg viewBox="0 0 24 24"><use href="#i-download"/></svg>Compatibility tools (GE-Proton, UMU-Proton, Wine, DXVK…)</button>`]),
  ].join("");
  const on = (key, fn) => rows.querySelector(`[data-key="${key}"]`)?.addEventListener("click", fn);
  on("add-exe", async () => {
    const path = await api.call("pick_file", { kind: "game" });
    if (!path) return;
    if (/(^|\/)(setup[^/]*|[^/]*install[^/]*)\.exe$|\.msi$/i.test(path) && !confirmTwice("add-exe", "That looks like an installer — press again to add it as the game anyway, or use “Install from an installer”")) return;
    try {
      const [next, id] = await api.call("custom_add", { path, title: null });
      snap = next;
      closeAllSheets();
      selectCustom(id);
      toast(`Added ${cg(id)?.title ?? id} — set it up under Game settings if it needs it`);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  });
  on("add-install", async () => {
    const path = await api.call("pick_file", { kind: "game" });
    if (!path) return;
    try {
      const [next, id] = await api.call("custom_install", { path, title: null });
      snap = next;
      closeAllSheets();
      selectCustom(id);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  });
  on("import-faugus", () => importGames(null));
  on("import-file", async () => {
    const path = await api.call("pick_file", { kind: "json" });
    if (path) importGames(path);
  });
  on("export", async () => {
    try {
      const msg = await api.call("custom_export");
      if (msg) toast(msg);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  });
  on("tools", openToolsSheet);
  nav.refocus();
}

async function importGames(path) {
  try {
    const [next, msg] = await api.call("custom_import", { path });
    snap = next;
    render();
    toast(msg);
  } catch (e) {
    toast(String(e?.message ?? e), true);
  }
}

function selectCustom(cid) {
  const i = items().findIndex((x) => x.cid === cid);
  if (i >= 0) {
    sel = i;
    render();
    const t = document.querySelector(`.tile[data-index="${sel}"]`);
    if (t) nav.focus(t);
  }
}

// Pressing a dangerous row twice: the first press arms it (and says what the second does).
let armed = null;
function confirmTwice(key, message) {
  if (armed?.key === key && Date.now() - armed.at < 6000) {
    armed = null;
    return true;
  }
  armed = { key, at: Date.now() };
  toast(message, true);
  return false;
}

// Game settings for a game added by hand
const setCustom = (id, key, value, msg) => act(() => api.call("custom_set", { id, key, value: String(value ?? "") }), msg);

function renderCustomSheet() {
  const g = game();
  if (!g?.custom) return closeSheet();
  $("#sheet-custom").querySelector("[data-custom-title]").textContent = g.title;
  const keep = nav.current()?.dataset.key;
  const toggle = (on) => `<span class="toggle${on ? " on" : ""}"></span>`;
  const runnerName = g.runner ? (g.runner_pick ? g.runner.name : `Automatic (${g.runner.name})`) : "None found";
  const protonish = g.runner && (g.runner.kind === "proton" || g.runner.kind === "umu");
  const rows = $("#custom-rows");
  const envRows = Object.entries(g.env || {}).map(
    ([k, v]) => `<div class="mod-line">${rowHTML({ key: `envrow:${k}`, label: k, sub: v })}<button class="nav remove" data-key="envrm:${esc(k)}" aria-label="Remove ${esc(k)}"><svg viewBox="0 0 24 24"><use href="#i-x"/></svg></button></div>`
  );
  rows.innerHTML = [
    `<p class="section-label">Game</p>`,
    fieldHTML({ key: "c-title", label: "Title", value: g.title }),
    rowHTML({ key: "exe", label: "Program", sub: g.exe || "Not chosen yet", value: g.exe ? "Change" : "Choose" }),
    fieldHTML({ key: "c-args", label: "Launch arguments", placeholder: "Optional, e.g. -windowed -skipintro", value: g.args }),
    `<div class="mod-line">${rowHTML({ key: "cwd", label: "Working folder", sub: g.cwd || "The program's own folder", value: "Change" })}${g.cwd ? `<button class="nav side" data-key="cwd-reset">Default</button>` : ""}</div>`,
    `<p class="section-label">Pictures</p>`,
    `<div class="mod-line">${rowHTML({ key: "icon", label: "Icon", sub: g.icon_url ? "From the program, or a picture you chose" : "None — choose a .png, .ico or .exe", value: "Change", extra: g.icon_url ? `<img class="cover-thumb" style="width:40px;height:40px" src="${esc(g.icon_url)}" alt="">` : "" })}${g.exe ? `<button class="nav side" data-key="icon-exe">From program</button>` : ""}</div>`,
    `<div class="mod-line">${rowHTML({ key: "cover", label: "Cover", sub: g.cover ? "Shown in Beacon and Hearth's library" : "A portrait picture (600×900) for Beacon and Hearth", value: g.cover ? "Change" : "Choose" })}${g.cover ? `<button class="nav side" data-key="cover-clear">Clear</button>` : ""}</div>`,
    `<div class="mod-line">${rowHTML({ key: "hero", label: "Background", sub: g.hero ? "Shown behind the game" : "A wide picture (1920×620)", value: g.hero ? "Change" : "Choose" })}${g.hero ? `<button class="nav side" data-key="hero-clear">Clear</button>` : ""}</div>`,
    ...(snap.native
      ? []
      : [
          `<p class="section-label">Compatibility</p>`,
          rowHTML({ key: "runner", label: "Runner", sub: "Proton or Wine build for this game", value: runnerName, cycle: true }),
          `<button class="nav row-btn" data-key="tools"><svg viewBox="0 0 24 24"><use href="#i-download"/></svg>Get more runners (Compatibility tools)</button>`,
          `<div class="mod-line">${rowHTML({ key: "prefix", label: "Wine prefix", sub: g.wineprefix, value: "Change" })}${g.prefix ? `<button class="nav side" data-key="prefix-reset">Own</button>` : ""}</div>`,
          ...(protonish
            ? [
                fieldHTML({ key: "c-gameid", label: "GAMEID (umu protonfixes)", placeholder: "e.g. umu-1091500 or a Steam app id — from umu's database", value: g.umu_id || "", help: "Lets umu apply the game's known fixes." }),
                rowHTML({ key: "store", label: "STORE", sub: "Where the game is from, for protonfixes", value: g.store || "none", cycle: true }),
              ]
            : []),
          `<p class="section-label">Graphics &amp; performance</p>`,
          rowHTML({ key: "dxvk", label: "DXVK", sub: protonish ? "Direct3D 8–11 on Vulkan (off: Wine's OpenGL path)" : "Direct3D 8–11 on Vulkan — copied into the prefix from Compatibility tools", extra: toggle(!g.no_dxvk) }),
          rowHTML({ key: "vkd3d", label: "VKD3D-Proton", sub: "Direct3D 12 on Vulkan", extra: toggle(!g.no_vkd3d) }),
          rowHTML({ key: "mangohud", label: "MangoHud", sub: "Frame rate and system overlay", extra: toggle(g.mangohud) }),
          rowHTML({ key: "gamemode", label: "GameMode", sub: "Feral's performance mode while the game runs", extra: toggle(g.gamemode) }),
          rowHTML({ key: "gamescope", label: "gamescope", sub: "Runs the game in its own compositor (skipped inside Hearth's Game Mode, which is one)", extra: toggle(g.gamescope) }),
          ...(g.gamescope ? [fieldHTML({ key: "c-gsargs", label: "gamescope options", placeholder: "-W 1920 -H 1080 -f", value: g.gamescope_args })] : []),
          `<p class="section-label">Proton options</p>`,
          ...(snap.flags || []).map(([k, label]) => rowHTML({ key: `flag:${k}`, label, extra: toggle((g.flags || []).includes(k)) })),
        ]),
    `<p class="section-label">Environment variables</p>`,
    ...envRows,
    fieldHTML({ key: "c-env", label: "Add a variable", placeholder: "KEY=VALUE, e.g. DXVK_HUD=fps", help: "Press Enter to add it." }),
    `<p class="section-label">Tools</p>`,
    ...(snap.native ? [] : [`<button class="nav row-btn" data-key="wine"><svg viewBox="0 0 24 24"><use href="#i-wine"/></svg>Wine tools: winecfg, winetricks, registry…</button>`]),
    `<button class="nav row-btn" data-key="run"><svg viewBox="0 0 24 24"><use href="#i-plus"/></svg>${snap.native ? "Run another program" : "Run another program in this prefix"}</button>`,
    ...(snap.native ? [] : [`<button class="nav row-btn" data-key="openpfx"><svg viewBox="0 0 24 24"><use href="#i-folder"/></svg>Open the Wine prefix</button>`]),
    `<button class="nav row-btn" data-key="log"><svg viewBox="0 0 24 24"><use href="#i-scroll"/></svg>Open log</button>`,
    `<p class="section-label">Remove</p>`,
    rowHTML({ key: "remove", label: "Remove from Beacon", sub: "Keeps the game's files and its Wine prefix" }),
    ...(!snap.native && (g.prefix_ready || g.own_prefix) ? [rowHTML({ key: "remove-pfx", label: "Remove and delete its Wine prefix", sub: g.wineprefix }).replace('class="nav row', 'class="nav row danger')] : []),
    `<div class="cmd" id="custom-preview">…</div>`,
  ].join("");

  const on = (key, ev, fn) => rows.querySelector(`[data-key="${CSS.escape(key)}"]`)?.addEventListener(ev, fn);
  const field = (key, apply) => {
    const input = rows.querySelector(`#f-${key}`);
    input?.addEventListener("change", () => apply(input.value));
  };
  const cycle = (key, fn) => {
    on(key, "cycle", (e) => fn(e.detail));
    on(key, "click", (e) => e.currentTarget.dispatchEvent(new CustomEvent("cycle", { detail: 1 })));
  };
  field("c-title", (v) => setCustom(g.cid, "title", v, "Title saved"));
  field("c-args", (v) => setCustom(g.cid, "args", v, "Arguments saved"));
  field("c-gameid", (v) => setCustom(g.cid, "umu-id", v.trim(), v.trim() ? "GAMEID saved" : "GAMEID cleared"));
  field("c-gsargs", (v) => setCustom(g.cid, "gamescope-args", v, "gamescope options saved"));
  field("c-env", async (v) => {
    if (!v.includes("=")) return toast("Use KEY=VALUE", true);
    await setCustom(g.cid, "env", v.trim(), "Variable added");
  });
  on("exe", "click", async () => {
    const p = await api.call("pick_file", { kind: "game" });
    if (p) setCustom(g.cid, "exe", p, "Program set");
  });
  on("cwd", "click", async () => {
    const p = await api.call("pick_folder");
    if (p) setCustom(g.cid, "cwd", p, "Working folder set");
  });
  on("cwd-reset", "click", () => setCustom(g.cid, "cwd", "", "Working folder: the program's own"));
  const picture = (key, label) =>
    on(key, "click", async () => {
      const p = await api.call("pick_file", { kind: "image" });
      if (p && (await setCustom(g.cid, key, p, `${label} set`))) {
        delete artCache[g.cid];
        await loadCustomArt(game()).catch(() => {});
        render();
      }
    });
  picture("icon", "Icon");
  picture("cover", "Cover");
  picture("hero", "Background");
  on("icon-exe", "click", () => setCustom(g.cid, "icon", g.exe, "Icon taken from the program"));
  for (const k of ["cover", "hero"])
    on(`${k}-clear`, "click", async () => {
      if (await setCustom(g.cid, k, "", "Picture cleared")) {
        delete artCache[g.cid];
        await loadCustomArt(game()).catch(() => {});
        render();
      }
    });
  cycle("runner", (d) => {
    const ids = [null, ...snap.runners.map((r) => r.id)];
    const cur = Math.max(0, ids.indexOf(g.runner_pick ?? null));
    const next = ids[(cur + d + ids.length) % ids.length];
    setCustom(g.cid, "runner", next ?? "auto");
  });
  on("tools", "click", openToolsSheet);
  on("prefix", "click", async () => {
    const p = await api.call("pick_folder");
    if (p) setCustom(g.cid, "prefix", p, "Wine prefix set");
  });
  on("prefix-reset", "click", () => setCustom(g.cid, "prefix", "", "Using its own prefix in Beacon's data folder"));
  cycle("store", (d) => {
    const opts = snap.stores || ["none"];
    const i = Math.max(0, opts.indexOf(g.store || "none"));
    const next = opts[(i + d + opts.length) % opts.length];
    setCustom(g.cid, "store", next === "none" ? "" : next);
  });
  on("dxvk", "click", () => setCustom(g.cid, "dxvk", g.no_dxvk ? "on" : "off"));
  on("vkd3d", "click", () => setCustom(g.cid, "vkd3d", g.no_vkd3d ? "on" : "off"));
  for (const k of ["mangohud", "gamemode", "gamescope"]) on(k, "click", () => setCustom(g.cid, k, g[k] ? "off" : "on"));
  for (const [k] of snap.flags || []) on(`flag:${k}`, "click", () => setCustom(g.cid, "flag", `${(g.flags || []).includes(k) ? "-" : "+"}${k}`));
  for (const k of Object.keys(g.env || {})) on(`envrm:${k}`, "click", () => setCustom(g.cid, "env", `${k}=`, `${k} removed`));
  on("wine", "click", () => open("wine", $("#sheet-wine")));
  on("run", "click", () => runInCustom(g));
  on("openpfx", "click", () => act(() => api.call("open_path", { path: g.wineprefix })));
  on("log", "click", () => (g.has_log ? act(() => api.call("open_path", { path: g.log_file })) : toast("No log yet — play the game first.", true)));
  const remove = async (deletePrefix) => {
    try {
      const [next, msg] = await api.call("custom_remove", { id: g.cid, deletePrefix });
      snap = next;
      closeAllSheets();
      sel = Math.min(sel, items().length - 1);
      render();
      toast(msg);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  };
  on("remove", "click", () => confirmTwice("remove", `Press again to remove ${g.title} from Beacon (its files and prefix stay)`) && remove(false));
  on("remove-pfx", "click", () => confirmTwice("remove-pfx", `Press again to remove ${g.title} AND delete its Wine prefix (saves inside it go too)`) && remove(true));
  api
    .call("custom_preview", { id: g.cid })
    .then((t) => ($("#custom-preview").textContent = t))
    .catch((e) => ($("#custom-preview").textContent = String(e?.message ?? e)));
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function runInCustom(g) {
  const path = await api.call("pick_file", { kind: "game" });
  if (path) act(() => api.call("custom_run", { id: g.cid, path }));
}

// Wine tools for a game's prefix
function renderWineSheet() {
  const g = game();
  if (!g?.custom) return closeSheet();
  $("#sheet-wine").querySelector("[data-custom-title]").textContent = g.title;
  const rows = $("#wine-rows");
  const subs = {
    winecfg: "Windows version, drives, audio, graphics",
    winetricks: "Install runtimes and fonts (vcrun, d3dx9, dotnet…)",
    regedit: "The prefix's registry",
    cmd: "A Windows command prompt",
    taskmgr: "Programs running in the prefix",
    explorer: "Browse the prefix's drive C:",
    uninstaller: "Remove programs installed in the prefix",
    control: "Windows control panel",
    kill: "Close everything running in this prefix (a stuck game or installer)",
  };
  rows.innerHTML = snap.native
    ? note("Windows runs games directly; there's no Wine prefix.")
    : [
        ...(snap.wine_tools || []).map(([k, label]) => rowHTML({ key: `tool:${k}`, label, sub: subs[k] || "", value: k === "kill" ? "Stop" : "Open" })),
        `<p class="section-label">Programs</p>`,
        `<button class="nav row-btn" data-key="run"><svg viewBox="0 0 24 24"><use href="#i-plus"/></svg>Run another program in this prefix (installer, patch, .msi)</button>`,
        ...(g.installer ? [`<button class="nav row-btn" data-key="reinstall"><svg viewBox="0 0 24 24"><use href="#i-download"/></svg>Run the installer again</button>`] : []),
        `<button class="nav row-btn" data-key="pick"><svg viewBox="0 0 24 24"><use href="#i-scan"/></svg>Find the game's program in the prefix</button>`,
      ].join("");
  for (const [k] of snap.wine_tools || []) rows.querySelector(`[data-key="tool:${k}"]`)?.addEventListener("click", () => act(() => api.call("custom_tool", { id: g.cid, tool: k })));
  rows.querySelector('[data-key="run"]')?.addEventListener("click", () => runInCustom(g));
  rows.querySelector('[data-key="reinstall"]')?.addEventListener("click", () => act(() => api.call("custom_run", { id: g.cid, path: g.installer })));
  rows.querySelector('[data-key="pick"]')?.addEventListener("click", () => openPick(g));
  nav.refocus();
}

// ---------------------------------------------------------------- compatibility tools (ProtonPlus-style)
// Proton, Wine, DXVK and VKD3D-Proton builds from the projects' GitHub releases, for Beacon and the other
// launchers on this PC.

const tl = { target: "beacon", data: null, loading: false, tool: null, releases: null, busy: null, pct: null };

async function openToolsSheet() {
  open("tools", $("#sheet-tools"));
  await loadTools(true);
}

async function loadTools(online) {
  tl.loading = true;
  render();
  try {
    tl.data = await api.call("tools_list", { target: tl.target, online });
  } catch (e) {
    toast(String(e?.message ?? e), true);
  }
  tl.loading = false;
  render();
}

const targetName = (id) => tl.data?.targets?.find((t) => t.id === id)?.name ?? id;

function renderToolsSheet() {
  const rows = $("#tools-rows");
  const keep = nav.current()?.dataset.key;
  const d = tl.data;
  const detected = (d?.targets || []).filter((t) => t.detected);
  const kinds = { proton: "Proton", wine: "Wine", dxvk: "DXVK", vkd3d: "VKD3D", umu: "umu" };
  const groups = [["proton", "Proton"], ["umu", "umu-launcher"], ["wine", "Wine"], ["dxvk", "Direct3D 8–11 (for Wine)"], ["vkd3d", "Direct3D 12 (for Wine)"]];
  const toolRow = (t) => {
    const inst = t.installed.map((i) => i.name + (i.latest ? " (Latest)" : ""));
    const value = t.up_to_date ? "" : t.installed.length ? (t.newest ? `Update to ${t.newest}` : "Installed") : t.newest ? `Get ${t.newest}` : "Get";
    return rowHTML({ key: `tool:${t.id}`, label: t.name, sub: inst.length ? inst.join(", ") : t.blurb, value, tick: t.up_to_date, badge: t.archived ? "No new releases" : kinds[t.kind] ?? "" });
  };
  rows.innerHTML = [
    rowHTML({ key: "target", label: "Install for", sub: "Beacon itself, or another launcher on this PC", value: targetName(tl.target), cycle: detected.length > 1 }),
    rowHTML({ key: "auto", label: "Keep “Latest” copies up to date", sub: "Checked each time Beacon opens", extra: `<span class="toggle${snap.tools_auto_update ? " on" : ""}"></span>` }),
    `<button class="nav row-btn" data-key="update"><svg viewBox="0 0 24 24"><use href="#i-download"/></svg>Update “Latest” copies now</button>`,
    tl.loading && !d ? note("Asking GitHub…") : "",
    ...(d?.errors?.length ? [note(`Couldn't ask GitHub about everything: ${esc(d.errors[0])}`)] : []),
    ...groups.flatMap(([k, label]) => {
      const list = (d?.tools || []).filter((t) => t.kind === k && t.supported);
      return list.length ? [`<p class="section-label">${label}</p>`, ...list.map(toolRow)] : [];
    }),
    note("Downloads come only from each project's own GitHub releases and are checked against the checksum the release publishes, when it has one. The list of tools follows ProtonPlus."),
  ].join("");
  const on = (key, ev, fn) => rows.querySelector(`[data-key="${CSS.escape(key)}"]`)?.addEventListener(ev, fn);
  on("target", "cycle", (e) => {
    if (detected.length < 2) return;
    const i = Math.max(0, detected.findIndex((t) => t.id === tl.target));
    tl.target = detected[(i + e.detail + detected.length) % detected.length].id;
    loadTools(false);
  });
  on("target", "click", (e) => e.currentTarget.dispatchEvent(new CustomEvent("cycle", { detail: 1 })));
  on("auto", "click", () => act(() => api.call("set_tools_auto", { on: !snap.tools_auto_update })));
  on("update", "click", () => toolsJob("Updating “Latest” copies…", () => api.call("tools_update")));
  for (const t of d?.tools || []) on(`tool:${t.id}`, "click", () => openTool(t.id));
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function openTool(id) {
  tl.tool = id;
  tl.releases = null;
  open("tool", $("#sheet-tool"));
  try {
    tl.releases = await api.call("tools_releases", { tool: id });
  } catch (e) {
    tl.releases = [];
    toast(String(e?.message ?? e), true);
  }
  render();
}

function renderToolSheet() {
  const t = tl.data?.tools?.find((x) => x.id === tl.tool);
  if (!t) return closeSheet();
  $("#tool-name").textContent = t.name;
  $("#tool-target").textContent = `For ${targetName(tl.target)}`;
  const keep = nav.current()?.dataset.key;
  const rows = $("#tool-rows");
  const latest = t.installed.find((i) => i.latest);
  const has = (tag) => t.installed.some((i) => (i.tag ? i.tag === tag : i.name.toLowerCase().replace(/[^a-z0-9]/g, "").includes(tag.toLowerCase().replace(/^v/, "").replace(/^cachyos/, "").replace(/[^a-z0-9]/g, ""))));
  rows.innerHTML = [
    note(esc(t.blurb) + (t.archived ? " Its project makes no new releases; the last one still installs." : "")),
    rowHTML({ key: "latest", label: `${t.name} Latest`, sub: latest ? `On ${latest.tag || latest.name} — kept on the newest release` : "One copy that Beacon keeps on the newest release", value: latest ? (t.up_to_date ? "" : "Update") : "Install", tick: !!latest && t.up_to_date }),
    ...(t.installed.length
      ? [
          `<p class="section-label">Installed</p>`,
          ...t.installed.map(
            (i) => `<div class="mod-line">${rowHTML({ key: `inst:${i.name}`, label: i.name, sub: i.by_beacon ? `Installed by Beacon${i.latest ? " · Latest" : ""}` : "Installed outside Beacon" })}<button class="nav remove" data-key="rm:${esc(i.name)}" aria-label="Remove ${esc(i.name)}"><svg viewBox="0 0 24 24"><use href="#i-trash"/></svg></button></div>`
          ),
        ]
      : []),
    `<p class="section-label">Releases</p>`,
    tl.releases == null ? note("Asking GitHub…") : !tl.releases.length ? note("No releases with a Linux build found.") : "",
    ...(tl.releases || []).slice(0, 30).map((r) => rowHTML({ key: `rel:${r.tag}`, label: r.tag, sub: [r.published?.slice(0, 10), fmtSize(r.size), r.checksum_url ? "checksum published" : "", r.prerelease ? "pre-release" : ""].filter(Boolean).join(" · "), value: has(r.tag) ? "" : "Install", tick: has(r.tag) })),
  ].join("");
  const on = (key, fn) => rows.querySelector(`[data-key="${CSS.escape(key)}"]`)?.addEventListener("click", fn);
  on("latest", () => toolsJob(`Installing ${t.name} Latest…`, () => api.call("tools_install", { tool: t.id, target: tl.target, version: "latest" })));
  for (const r of tl.releases || []) on(`rel:${r.tag}`, () => !has(r.tag) && toolsJob(`Installing ${t.name} ${r.tag}…`, () => api.call("tools_install", { tool: t.id, target: tl.target, version: r.tag })));
  for (const i of t.installed)
    on(`rm:${i.name}`, async () => {
      if (!confirmTwice(`rm:${i.name}`, `Press again to delete ${i.name} from ${targetName(tl.target)}`)) return;
      if (await act(() => api.call("tools_remove", { tool: t.id, target: tl.target, name: i.name }), `Removed ${i.name}`)) await loadTools(false);
    });
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function toolsJob(title, fn) {
  if (tl.busy) return;
  tl.busy = title;
  showOverlay({ title, text: "Asking GitHub for the release" });
  try {
    const [next, msg] = await fn();
    snap = next;
    hideOverlay();
    await loadTools(false);
    toast(msg);
  } catch (e) {
    showOverlay({ title: "That didn't work", text: String(e?.message ?? e), error: true, ok: true });
  }
  tl.busy = null;
}

function onToolsProgress({ pct, text }) {
  if (!tl.busy || $("#overlay").hidden) return;
  $("#overlay-text").textContent = pct == null ? text : `${text} (${pct}%)`;
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
    if (openSheet === "game" || openSheet === "custom" || openSheet === "add") closeSheet();
    else if (!openSheet) openGameSheet();
  } else if (b === "y") {
    if (openSheet === "mods" || openSheet === "wine") closeSheet();
    else if (!openSheet) openModsSheet();
  } else if (b === "start" && !openSheet) play();
  else if (b === "select") {
    if (openSheet === "launcher") closeSheet();
    else if (!openSheet) openLauncherSheet();
  }
});

nav.onInputChange(renderHints);

$("#btn-play").addEventListener("click", play);
function openModsSheet() {
  const g = game();
  if (g.add) return;
  if (g.custom) return open("wine", $("#sheet-wine"));
  loadMods(g.id).then(() => open("mods", $("#sheet-mods")));
}
$("#btn-mods").addEventListener("click", openModsSheet);
const openGameSheet = () => {
  const g = game();
  if (g.add) return open("add", $("#sheet-add"));
  if (g.custom) return open("custom", $("#sheet-custom"));
  return loadCommunity(g.id).catch(() => {}).then(() => open("game", $("#sheet-game")));
};
$("#btn-game-settings").addEventListener("click", openGameSheet);
$("#btn-launcher-settings").addEventListener("click", openLauncherSheet);
$("#btn-scan").addEventListener("click", scan);
$("#btn-add-mod-file").addEventListener("click", () => addMod("file"));
$("#btn-add-mod-folder").addEventListener("click", () => addMod("folder"));
$("#btn-catalog").addEventListener("click", openCatalog);
$("#btn-workshop").addEventListener("click", openWorkshop);
$("#btn-workshop-more").addEventListener("click", () => loadWorkshop(ws.page + 1));
$("#btn-workshop-site").addEventListener("click", () => act(() => api.call("open_url", { url: "https://bfmeladder.com" }), "Opened bfmeladder.com in your browser"));
$("#btn-find-mods").addEventListener("click", findMods);
$("#btn-catalog-add").addEventListener("click", () => addMod("file"));
$("#btn-catalog-find").addEventListener("click", findMods);
$("#btn-open-mods").addEventListener("click", () => act(() => api.call("open_path", { path: `${snap.mods_dir}/${game().id}` })));
$("#overlay-ok").addEventListener("click", hideOverlay);
// Coming back to Beacon while the All-in-One launcher is open: pick up games it finished downloading.
window.addEventListener("focus", () => {
  if (!snap || !aioOpenedAt || Date.now() - (refreshAfterAio.last || 0) < 15000) return;
  if (WORKSHOP_GAMES.every((id) => snap.games.find((g) => g.id === id)?.installed)) return;
  refreshAfterAio.last = Date.now();
  refreshAfterAio(false).catch(() => {});
});
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
  sel = Math.max(0, items().findIndex((g) => g.installed && !g.add));
  buildShelf();
  renderHints(false);
  render();
  await Promise.all(snap.games.map((g) => loadMods(g.id).catch(() => [])));
  render();
  nav.focus($("#btn-play"));
  api.on("game-event", onGameEvent);
  api.on("workshop-progress", onWorkshopProgress);
  api.on("aio-launcher-progress", onAioProgress);
  api.on("aio-launcher-closed", () => refreshAfterAio(true).catch(() => {}));
  api.on("community-launcher-progress", onCommunityLauncherProgress);
  api.on("community-launcher-closed", (e) => onCommunityLauncherClosed(e).catch(() => {}));
  api.on("custom-event", (e) => onCustomEvent(e).catch(() => {}));
  api.on("custom-install-done", onInstallDone);
  api.on("tools-progress", onToolsProgress);
  api.on("tools-updated", (n) => api.call("get_state").then((s) => { snap = s; render(); toast(`Updated ${n} compatibility tool${n > 1 ? "s" : ""}`); }).catch(() => {}));
  tick();
  setInterval(tick, 15000);
})();
