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

// The list's sections: the Battle for Middle-earth games and Conquest are all Lord of the Rings games.
const SECTIONS = [
  ["Lord of the Rings", ["bfme", "lotr"]],
  ["Other Games", ["other"]],
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

function buildShelf() {
  const list = $("#shelf");
  const row = (g, i) => `
      <button class="nav game-row" data-group="shelf" data-game="${g.id}" data-index="${i}">
        <span class="swatch" aria-hidden="true"></span>
        <span class="r-title">${esc(g.title.replace(/^The Lord of the Rings: /, "").replace(/^Star Wars: /, "").replace(/^Rise of Nations: /, ""))}</span>
        <span class="r-state"></span>
      </button>`;
  list.innerHTML = SECTIONS.map(([label, fams]) => {
    const members = snap.games.map((g, i) => [g, i]).filter(([g]) => fams.includes(g.family));
    if (!members.length) return "";
    return `<div class="list-group"><p class="list-label">${esc(label)}</p>${members.map(([g, i]) => row(g, i)).join("")}</div>`;
  }).join("");
  list.querySelectorAll(".game-row").forEach((el) => {
    const i = Number(el.dataset.index);
    el.addEventListener("focus", () => select(i));
    el.addEventListener("click", () => (sel === i ? nav.focus($("#btn-play")) : select(i)));
  });
}

function select(i) {
  if (i === sel && document.body.dataset.accent === game().id) return;
  sel = (i + snap.games.length) % snap.games.length;
  render();
  loadMods(game().id).then(renderHero).catch(() => {});
}

function renderShelf() {
  const current = document.querySelector(`.game-row[data-index="${sel}"]`);
  if (current && renderShelf.last !== sel) {
    renderShelf.last = sel;
    current.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }
  document.querySelectorAll(".game-row").forEach((el) => {
    const g = snap.games[Number(el.dataset.index)];
    const running = isRunning(g.id);
    el.classList.toggle("selected", g === game());
    el.classList.toggle("is-ready", g.ready && !running);
    el.classList.toggle("is-running", running);
    el.classList.toggle("is-missing", !g.installed);
    el.querySelector(".r-state").textContent = running ? "Running" : g.ready ? "Ready" : g.installed ? "Needs attention" : "Not installed";
  });
}

// The cards under the buttons: what this game is set up with, at a glance.
function heroCards(g) {
  const card = (label, value, key) =>
    key
      ? `<button class="nav card" data-card="${key}"><span class="c-label">${esc(label)}</span><span class="c-value">${esc(value)}</span></button>`
      : `<div class="card"><span class="c-label">${esc(label)}</span><span class="c-value">${esc(value)}</span></div>`;
  const out = [];
  if (!g.installed) return "";
  out.push(card("Mod", modName(g), "mods"));
  const req = g.requires && snap.games.find((x) => x.id === g.requires);
  if (req) out.push(card("Needs", `${req.short} · ${req.installed ? "installed" : "not installed"}`));
  else if (g.edition) out.push(card("Version", g.edition, "settings"));
  else if (!snap.native && g.runner) out.push(card("Runner", g.runner.name, "settings"));
  if (WORKSHOP_GAMES.includes(g.id)) out.push(card("Workshop", "Browse fan mods", "workshop"));
  else if (g.has_community) out.push(card("Community version", g.edition ? "Set up" : "Set it up", "settings"));
  return out.join("");
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
  else chips.push(`<span class="chip bad"><span class="dot"></span>${g.installed ? "Needs attention" : "Not installed"}</span>`);
  if (g.runner && !snap.native) chips.push(`<span class="chip">${esc(g.runner.name)}</span>`);
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

  const cards = $("#hero-cards");
  const keep = nav.current()?.dataset.card;
  cards.innerHTML = heroCards(g);
  cards.querySelectorAll("[data-card]").forEach((el) => {
    const k = el.dataset.card;
    el.addEventListener("click", () => {
      if (k === "mods") loadMods(game().id).then(() => open("mods", $("#sheet-mods")));
      else if (k === "workshop") openWorkshop();
      else openGameSheet();
    });
  });
  if (keep && !openSheet) nav.refocus(`[data-card="${CSS.escape(keep)}"]`);
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
  if (openSheet === "workshop") renderWorkshopSheet();
  if (openSheet === "game") renderGameSheet();
  if (openSheet === "launcher") renderLauncherSheet();
  if (openSheet === "downloads") renderDownloadsSheet();
  if (openSheet === "uninstall") renderUninstallSheet();
}

// ---------------------------------------------------------------- actions

async function play() {
  const g = game();
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
  modDownloads[game().id] = await api.call("mod_downloads", { game: game().id }).catch(() => []);
  open("catalog", $("#sheet-catalog"));
}
const modDownloads = {}; // game id → [{id, title, catalog_id}] (private build)

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
              value: it.installed ? (g.selected_mod === it.installed ? "" : "Use") : dlFor(g, it) ? "Download" : "Get",
              tick: g.selected_mod === it.installed && !!it.installed,
              selected: !!it.installed,
            })
          )
          .join("")
      : "";
  const rows = $("#catalog-rows");
  const extras = (modDownloads[g.id] || []).filter((d) => !d.catalog_id);
  rows.innerHTML =
    section("Standalone game", items.filter((i) => i.kind === "game")) +
    section("Mods", items.filter((i) => i.kind === "mod")) +
    section("Community patches", items.filter((i) => i.kind === "patch")) +
    (extras.length ? `<p class="section-label">More downloads</p>` + extras.map((d) => rowHTML({ key: `dl:${d.id}`, label: d.title, sub: "Downloaded and set up inside Beacon", value: "Download" })).join("") : "") +
    `<p class="section-label">Download from a link</p>` +
    fieldHTML({ key: "moddb-link", label: "ModDB link", placeholder: "https://www.moddb.com/mods/…", help: "Paste a mod's ModDB page or one of its downloads. Beacon downloads it and puts it in the right place for this game." }) +
    `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">“Download” gets the mod inside Beacon and sets it up for ${esc(g.short)}. “Get” opens the mod's page.</p>`;
  const linkField = rows.querySelector("#f-moddb-link");
  linkField?.addEventListener("change", () => {
    const link = linkField.value.trim();
    linkField.value = "";
    if (link) modGet(g, { link });
  });
  rows.querySelectorAll(".row").forEach((r) =>
    r.addEventListener("click", async () => {
      if (r.dataset.key.startsWith("dl:")) return modGet(g, { id: r.dataset.key.slice(3) });
      const it = items.find((i) => i.id === r.dataset.key);
      if (!it.installed && dlFor(g, it)) return modGet(g, { id: dlFor(g, it).id });
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
          // Private build: Beacon downloads the BFME games itself, from the BFME Foundation's CDN.
          ...(dlInfo?.games?.includes(g.id) ? cdnRows(g) : []),
          // Or the BFME Foundation's launcher downloads them, Beacon finds them.
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
    ...(g.installed && WORKSHOP_GAMES.includes(g.id) && ownDownload(g)
      ? [
          `<button class="nav row-btn" data-key="cdn-repair"><svg viewBox="0 0 24 24"><use href="#i-scan"/></svg>Repair game files (downloads anything missing)</button>`,
          `<button class="nav row-btn" data-key="cdn-verify"><svg viewBox="0 0 24 24"><use href="#i-scan"/></svg>Verify every file (slow: checks each checksum)</button>`,
        ]
      : []),
    ...(g.installed && WORKSHOP_GAMES.includes(g.id) && snap.aio_launcher?.installed
      ? [`<button class="nav row-btn" data-key="aio-open"><svg viewBox="0 0 24 24"><use href="#i-open"/></svg>Open the All-in-One launcher (patches, workshop, online)</button>`]
      : []),
    ...(aotr && g.tool_launcher
      ? [`<button class="nav row-btn" data-key="aotr-launcher"><svg viewBox="0 0 24 24"><use href="#i-open"/></svg>Open the AotR Launcher for updates (may not work under Wine)</button>`]
      : []),
    `<button class="nav row-btn" data-key="installer"><svg viewBox="0 0 24 24"><use href="#i-plus"/></svg>${snap.native ? "Run a patch or installer for this game" : "Run a patch or installer in this game's prefix"}</button>`,
    `<button class="nav row-btn" data-key="log"><svg viewBox="0 0 24 24"><use href="#i-scroll"/></svg>Open log</button>`,
    `<button class="nav row-btn" data-key="openfolder"><svg viewBox="0 0 24 24"><use href="#i-open"/></svg>Open game folder</button>`,
    ...(g.installed ? [`<button class="nav row-btn danger" data-key="uninstall"><svg viewBox="0 0 24 24"><use href="#i-x"/></svg>Uninstall or reinstall ${esc(g.short)}…</button>`] : []),
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
  wireCdn(on, g);
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
  on("uninstall", "click", () => openUninstall(g));

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
    ...launcherDownloadRows(g),
    rowHTML({ key: `cl-get:${g.id}`, label: `Get the ${g.short} launcher`, sub: CL_HOW_TO, value: "Open" }),
    rowHTML({ key: `cl-set:${g.id}`, label: "Use a launcher you downloaded", sub: cl.path || "Its .exe, or the .zip, .7z or .rar it came in", value: cl.path ? "Change" : "Choose" }),
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
  on(`cl-dl:${g.id}`, "click", () => launcherGet(g));
  on(`cl-get:${g.id}`, "click", () => act(() => api.call("open_url", { url: g.community_launcher.url }), "Opened the download folder in your browser"));
  on(`cl-set:${g.id}`, "click", () => chooseCommunityLauncher(g));
  on(`cl-open:${g.id}`, "click", () => openCommunityLauncher(g));
}

async function chooseCommunityLauncher(g) {
  if (clBusy) return;
  const path = await api.call("pick_file", { kind: "launcher" });
  if (!path) return;
  clBusy = g.id;
  showOverlay({ title: `Setting up the ${g.short} launcher…`, text: /\.(zip|7z|rar)$/i.test(path) ? "Extracting it into Beacon's tools folder." : "Copying it into Beacon's tools folder." });
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
          `<p class="section-label">Runners found</p>`,
          ...(snap.runners.length
            ? snap.runners.map((r) => `<div class="row" style="cursor:default"><span style="min-width:0"><span class="label">${esc(r.name)}</span><span class="sub">${esc(r.path)}</span></span><span class="badge">${esc(r.kind)}</span></div>`)
            : [`<p class="section-label">None — install GE-Proton (ProtonPlus or ProtonUp-Qt), umu-launcher, or Wine.</p>`]),
        ]),
    `<p class="section-label">BFME All-in-One launcher</p>`,
    ...aio.map((a) => {
      const gg = snap.games.find((x) => x.id === a.game);
      return `<div class="row" style="cursor:default"><span style="min-width:0"><span class="label">${esc(gg?.short ?? a.game)}${a.active_patch ? ` · ${esc(a.active_patch)}` : ""}</span><span class="sub">${esc(a.dir)}</span></span><span class="badge">Found</span></div>`;
    }),
    ...aioRows(),
    ...downloadsRows(),
    ...snap.games.filter((x) => x.community_launcher).flatMap((x) => [`<p class="section-label">${esc(x.title)} launcher</p>`, ...communityLauncherRows(x)]),
    `<p class="section-label">Storage</p>`,
    `<button class="nav row-btn" data-key="data"><svg viewBox="0 0 24 24"><use href="#i-folder"/></svg>Open Beacon's data folder (prefixes, mods, logs)</button>`,
    `<p class="section-label">About</p>`,
    `<div class="row" style="cursor:default"><span><span class="label">Beacon ${esc(snap.version)}</span><span class="sub">Not affiliated with EA or the Tolkien Estate. Bring your own copy of the games.</span></span></div>`,
    appUpdateRow(),
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
  wireAio(on);
  wireDownloads(on, rows);
  on("app-update", "click", appUpdate);
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

// ---------------------------------------------------------------- uninstall / reinstall
// The plan comes first (what would go, how big, whether Beacon will delete it); nothing happens until
// "Uninstall" or "Reinstall". Saves and settings in the game's prefix stay unless asked.
let unPlan = null;
const unOpts = { files: true, prefix: false };

async function openUninstall(g) {
  try {
    unPlan = await api.call("uninstall_plan", { game: g.id });
  } catch (e) {
    return toast(String(e?.message ?? e), true);
  }
  Object.assign(unOpts, { files: unPlan.can_delete, prefix: false });
  open("uninstall", $("#sheet-uninstall"));
}

// How a game gets back after a reinstall: hook for builds that can download it (see the private build).
const reinstallers = [];

function renderUninstallSheet() {
  const g = game();
  const p = unPlan;
  if (!p || p.game !== g.id) return closeSheet();
  const sheet = $("#sheet-uninstall");
  sheet.querySelector("[data-game-short]").textContent = g.title;
  const keep = nav.current()?.dataset.key;
  const size = (n) => (n ? fmtSize(n) : "empty");
  const rows = $("#uninstall-rows");
  rows.innerHTML = [
    `<div class="row" style="cursor:default"><span style="min-width:0"><span class="label">Game folder</span><span class="sub">${esc(p.dir ?? "Not set")}</span></span><span class="value">${p.dir ? size(p.bytes) : ""}</span></div>`,
    p.why_not ? `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">Beacon won't delete these files: ${esc(p.why_not)}.</p>` : "",
    p.needed_by.length ? `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400;color:var(--danger)">${esc(p.needed_by.map((x) => snap.games.find((y) => y.id === x)?.short ?? x).join(", "))} needs ${esc(g.short)} to run.</p>` : "",
    `<p class="section-label">What to remove</p>`,
    p.can_delete ? rowHTML({ key: "un-files", label: "Delete the game's files", sub: `Frees ${size(p.bytes)}. Off: ${g.short} only leaves Beacon, its files stay.`, extra: `<span class="toggle${unOpts.files ? " on" : ""}"></span>` }) : "",
    p.prefix ? rowHTML({ key: "un-prefix", label: "Also delete saves and settings", sub: `${g.short}'s Wine prefix · ${size(p.prefix_bytes)}${p.inside_prefix ? " · the game is installed inside it" : ""}. Off: kept for when you reinstall.`, extra: `<span class="toggle${unOpts.prefix ? " on" : ""}"></span>` }) : "",
    `<p class="section-label">Do it</p>`,
    rowHTML({ key: "un-go", label: unOpts.files || unOpts.prefix ? `Uninstall ${g.short}` : `Remove ${g.short} from Beacon`, sub: unOpts.files || unOpts.prefix ? "Can't be undone" : "Its files stay where they are; Find games brings it back", value: "Uninstall" }),
    p.can_delete ? rowHTML({ key: "un-reinstall", label: `Reinstall ${g.short}`, sub: reinstallers.some((r) => r.can(g)) ? "Deletes the game's files (saves and settings stay), then downloads it again" : "Deletes the game's files (saves and settings stay), then you install it again from your copy", value: "Reinstall" }) : "",
    p.uninstaller ? rowHTML({ key: "un-own", label: "Run the game's own uninstaller", sub: p.uninstaller, value: "Run" }) : "",
    p.steam ? `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">This is a Steam copy: uninstall it in Steam (Library → ${esc(g.short)} → Manage → Uninstall).</p>` : "",
  ].filter(Boolean).join("");
  const on = (key, fn) => rows.querySelector(`[data-key="${key}"]`)?.addEventListener("click", fn);
  on("un-files", () => { unOpts.files = !unOpts.files; renderUninstallSheet(); });
  on("un-prefix", () => { unOpts.prefix = !unOpts.prefix; renderUninstallSheet(); });
  on("un-go", () => doUninstall(g, unOpts.files, unOpts.prefix, false));
  on("un-reinstall", () => doUninstall(g, true, false, true));
  on("un-own", () => {
    closeAllSheets();
    runInstaller(g, p.uninstaller);
  });
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

async function doUninstall(g, files, prefix, reinstall) {
  try {
    const [next, msg] = await api.call("uninstall", { game: g.id, deleteFiles: files, removePrefix: prefix });
    snap = next;
    unPlan = null;
    closeAllSheets();
    render();
    toast(msg);
    if (reinstall) {
      const r = reinstallers.find((x) => x.can(g));
      if (r) return r.run(g);
      openGameSheet();
      toast(`Now install ${g.short} again: pick its installer or disc under Install`);
    }
  } catch (e) {
    toast(String(e?.message ?? e), true);
  }
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
  // A mod download that wasn't confirmed is dropped (private build).
  modPending = null;
  $("#overlay-ok").textContent = "OK";
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

// ---------------------------------------------------------------- private build: downloads
// BFME, BFME II and RotWK straight from the BFME Foundation's CDN (the packages the All-in-One launcher installs),
// resolved through sources.json so a moved link is found again. RotWK brings BFME II along when it's missing.

let dlInfo = null; // downloads_info: sources version, updates repo, games folder, last check
let cdnBusy = null; // game id while downloading
const cdnGuess = {}; // game id → note, when the source only turned up a guess

const ownDownload = (g) => !!(dlInfo && g.path && g.path.startsWith(dlInfo.games_dir));

function cdnRows(g) {
  if (cdnGuess[g.id]) {
    return [
      rowHTML({ key: "cdn-guess", label: `Download ${g.short} from the package Beacon found`, sub: `Its usual link changed: ${cdnGuess[g.id]}`, value: "Use it" }),
    ];
  }
  const withB2 = g.id === "rotwk" && !snap.games.find((x) => x.id === "bfme2")?.installed;
  const bfme = WORKSHOP_GAMES.includes(g.id);
  const launcherExe = dlInfo?.archive_launchers?.[g.id];
  const check = dlInfo?.checks?.find((c) => c.id === g.id);
  const broken = check?.state === "broken";
  return [
    rowHTML({
      key: "cdn-get",
      label: `Download ${g.short}`,
      sub: broken
        ? `The source didn't answer at the last check: ${check.detail}`
        : `${bfme ? "Straight from the BFME Foundation's servers" : "From its Google Drive download, extracted and set up"}${withB2 ? " · BFME II comes along" : ""} · resumes if it stops${check?.state === "ok" || check?.state === "moved" ? ` · ${check.detail.split(" (")[0]}` : ""}`,
      value: "Download",
    }),
    ...(launcherExe
      ? [rowHTML({ key: "cdn-run-launcher", label: `Run the ${g.short} launcher again`, sub: `${launcherExe} · in ${g.short}'s own prefix; Beacon finds the game when it closes`, value: "Run" })]
      : []),
  ];
}

function wireCdn(on, g) {
  on("cdn-get", "click", () => cdnGet(g, {}));
  on("cdn-guess", "click", () => cdnGet(g, { acceptGuess: true }));
  on("cdn-repair", "click", () => cdnGet(g, { repair: true }));
  on("cdn-verify", "click", () => cdnGet(g, { verify: true }));
  on("cdn-run-launcher", "click", () => runInstaller(g, dlInfo.archive_launchers[g.id]));
}

async function cdnGet(g, { repair = false, verify = false, acceptGuess = false }) {
  return runDownload("cdn_get", { game: g.id, repair, verify, acceptGuess }, g, repair || verify ? `${g.short} repair` : g.short);
}

function onCdnProgress({ pct, text }) {
  if (!cdnBusy || $("#overlay").hidden) return;
  $("#overlay-text").textContent = pct == null ? text : `${pct}% · ${text}`;
}

const STATE_BADGE = { ok: "Working", moved: "Moved · found", "needs-confirm": "Changed · guess", broken: "Broken" };

function downloadsRows() {
  const d = dlInfo;
  if (!d) return [];
  const checks = d.checks || [];
  const bad = checks.filter((c) => c.state === "broken" || c.state === "needs-confirm");
  const summary = !checks.length ? "Not checked yet" : bad.length ? `${bad.length} of ${checks.length} need a look` : `All ${checks.length} working`;
  return [
    `<p class="section-label">Downloads (private build)</p>`,
    rowHTML({ key: "src-check", label: "Check download sources", sub: `${summary} · also runs when Beacon starts`, value: "Check" }),
    ...checks.map(
      (c) =>
        `<div class="row" style="cursor:default"><span style="min-width:0"><span class="label">${esc(c.title)}</span><span class="sub">${esc(c.detail)}</span></span><span class="badge${c.state === "broken" ? " bad" : ""}">${esc(STATE_BADGE[c.state] ?? c.state)}</span></div>`
    ),
    rowHTML({ key: "src-update", label: "Update the sources list", sub: `Version ${d.version} (${d.built_in ? "built in" : "downloaded"}) · from ${d.sources_repo}`, value: "Update" }),
    fieldHTML({
      key: "updates-repo",
      label: "Updates come from",
      placeholder: d.sources_repo,
      help: "A public GitHub repo (owner/repo): Beacon's own updates and fixes to the download list come from its releases and its sources.json.",
    }),
    `<div class="mod-line">${rowHTML({ key: "games-dir", label: "Downloaded games go to", sub: d.games_dir, value: "Change" })}${d.custom_games_dir ? `<button class="nav side" data-key="games-dir-default">Default</button>` : ""}</div>`,
  ];
}

function wireDownloads(on, rows) {
  on("src-check", "click", async () => {
    toast("Checking the download sources…");
    try {
      const checks = await api.call("sources_check");
      dlInfo = { ...dlInfo, checks };
      render();
      const bad = checks.filter((c) => c.state === "broken");
      toast(bad.length ? `Broken: ${bad.map((c) => c.title).join(", ")}` : "All download sources work", !!bad.length);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  });
  on("src-update", "click", async () => {
    try {
      const u = await api.call("sources_update");
      dlInfo = await api.call("downloads_info").catch(() => dlInfo);
      render();
      toast(u.skipped ? `Not updated: ${u.skipped}` : u.to > u.from ? `Sources list updated to version ${u.to}` : `Sources list is up to date (version ${u.from})`, !!u.skipped);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  });
  const repo = rows.querySelector("#f-updates-repo");
  repo?.addEventListener("change", async () => {
    const value = repo.value.trim();
    repo.value = "";
    if (!value) return;
    try {
      const r = await api.call("set_updates_repo", { repo: value });
      dlInfo = await api.call("downloads_info").catch(() => dlInfo);
      appUpd = null;
      render();
      toast(`Updates come from ${r} now`);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  });
  const setDir = async (path) => {
    if (await act(() => api.call("set_games_dir", { path }), path ? `Downloaded games go to ${path}` : "Downloaded games go to Beacon's data folder")) {
      dlInfo = await api.call("downloads_info").catch(() => dlInfo);
      render();
    }
  };
  on("games-dir", "click", async () => {
    const path = await api.call("pick_folder");
    if (path) setDir(path);
  });
  on("games-dir-default", "click", () => setDir(null));
}

// Private build: the newest community launcher (UaW's) straight from its Google Drive folder.
const clGuess = {}; // game id → note, when the folder only turned up a guess

function launcherDownloadRows(g) {
  if (!dlInfo?.launchers?.includes(g.id)) return [];
  const have = !!g.community_launcher?.path;
  const check = dlInfo.checks?.find((c) => c.id === `${g.id}-launcher`);
  if (clGuess[g.id]) return [rowHTML({ key: `cl-dl:${g.id}`, label: `Download the ${g.short} launcher Beacon found`, sub: `Its usual file changed: ${clGuess[g.id]}`, value: "Use it" })];
  return [
    rowHTML({
      key: `cl-dl:${g.id}`,
      label: have ? `Update the ${g.short} launcher` : `Download the ${g.short} launcher`,
      sub: check?.state === "broken" ? `The folder didn't answer at the last check: ${check.detail}` : `The newest version from the community's Google Drive folder, set up for you${check && check.state !== "broken" ? ` · ${check.detail.split(" (")[0]}` : ""}`,
      value: have ? "Check" : "Download",
    }),
  ];
}

async function launcherGet(g) {
  return runDownload("launcher_get", { game: g.id, force: false, acceptGuess: !!clGuess[g.id] }, g, `${g.short} launcher`);
}

// Private build: Beacon updates itself from the private repo's releases.
let appUpd = null; // last update check
let appUpdBusy = false;

function appUpdateRow() {
  const c = appUpd;
  const label = c?.newer ? `Update to Beacon ${c.latest.version}` : "Check for updates";
  const sub = !c
    ? `From ${dlInfo?.sources_repo ?? "the updates repo"}'s releases`
    : c.problem
      ? c.problem
      : c.newer
        ? `This is ${c.current} · downloads, checks and restarts`
        : `Up to date (newest release: ${c.latest?.version ?? c.current})`;
  return rowHTML({ key: "app-update", label, sub, value: c?.newer ? "Update" : "Check" });
}

async function appUpdate() {
  if (appUpdBusy) return;
  if (!appUpd?.newer) {
    try {
      appUpd = await api.call("app_update_check");
      render();
      toast(appUpd.problem ? appUpd.problem : appUpd.newer ? `Beacon ${appUpd.latest.version} is available` : "Beacon is up to date", !!appUpd.problem);
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
    return;
  }
  appUpdBusy = true;
  showOverlay({ title: `Updating to Beacon ${appUpd.latest.version}…`, text: "Downloading" });
  try {
    const r = await api.call("app_update_apply");
    // Restart or quit: this window closes on its own. A manual install comes back here.
    if (r?.next === "manual") showOverlay({ title: `Beacon ${appUpd.latest.version} downloaded`, text: `Install it with: ${r.how}`, ok: true });
    else showOverlay({ title: "Restarting Beacon…", text: "" });
  } catch (e) {
    showOverlay({ title: "Couldn't update Beacon", text: String(e?.message ?? e), error: true, ok: true });
  }
  appUpdBusy = false;
}

// Private build: mods downloaded inside Beacon. Look the file up, say what it is, then download and
// install it where it belongs for the game (its mods, its folder, or its setup run in its prefix).
const dlFor = (g, it) => (modDownloads[g.id] || []).find((d) => d.catalog_id === it.id);
let modBusy = false;
let modPending = null; // a lookup waiting for "Download"

async function modGet(g, what) {
  if (modBusy) return;
  modBusy = true;
  showOverlay({ title: "Looking it up…", text: "Following the download to its file" });
  try {
    const found = await api.call("mod_lookup", { game: g.id, id: what.id ?? null, link: what.link ?? null });
    const size = found.size ? ` · ${fmtSize(found.size)}` : "";
    modPending = found;
    showOverlay({ title: `Download ${found.name}?`, text: `${found.file}${size}${found.note ? ` · ${found.note}` : ""}`, ok: true });
    $("#overlay-ok").textContent = "Download";
  } catch (e) {
    showOverlay({ title: "Couldn't find the download", text: String(e?.message ?? e), error: true, ok: true });
  }
  modBusy = false;
}

async function modInstall() {
  const found = modPending;
  hideOverlay();
  if (!found) return;
  const g = snap.games.find((x) => x.id === found.game);
  return runDownload("mod_install", { found }, g, found.name);
}

// ---------------------------------------------------------------- downloads menu (private build)
// Every download runs in the background as a job; the Downloads menu shows them all with their progress,
// and can pause (the partial file stays), resume, retry or remove them.
const jobs = new Map(); // id → job (from downloads_list and downloads-changed)
const running = new Set(); // "cmd:game:name" of downloads this window started
let dlStorage = null;

function jobKey(cmd, args) {
  return `${cmd}:${args.game ?? args.found?.game ?? ""}:${args.found?.name ?? ""}`;
}

async function runDownload(cmd, args, g, label) {
  const key = jobKey(cmd, args);
  if (running.has(key)) return toast(`${label} is already downloading — see Downloads`, true);
  running.add(key);
  toast(`Downloading ${label} — progress is in Downloads`);
  try {
    const r = await api.call(cmd, args);
    await afterDownload(cmd, g, r);
  } catch (e) {
    const m = String(e?.message ?? e);
    if (/paused$/.test(m)) toast(`${label} paused — resume it from Downloads`);
    else toast(`${label}: ${m}`, true);
  }
  running.delete(key);
  if (openSheet === "downloads") loadDlStorage();
}

// What a finished download means for the screen: new state, a message, maybe a setup to run.
async function afterDownload(cmd, g, r) {
  if (r?.result === "needs-confirm") {
    if (cmd === "cdn_get") cdnGuess[g.id] = r.note;
    if (cmd === "launcher_get") clGuess[g.id] = r.note;
    render();
    return toast(`${g.short}'s usual download changed — check what Beacon found (Downloads or Game settings)`, true);
  }
  if (cmd === "cdn_get") delete cdnGuess[g.id];
  if (cmd === "launcher_get") delete clGuess[g.id];
  if (r?.snapshot) snap = r.snapshot;
  if (cmd === "cdn_get") dlInfo = await api.call("downloads_info").catch(() => dlInfo);
  if (cmd === "mod_install") {
    await loadMods(g.id, true).catch(() => {});
    if (catalogCache[g.id]) catalogCache[g.id] = await api.call("get_catalog", { game: g.id }).catch(() => catalogCache[g.id]);
  }
  render();
  if (r?.message) toast(r.notes?.length ? `${r.message} (${r.notes.join("; ")})` : r.message);
  // The download held a setup program, not the game or mod itself: run it in the game's prefix.
  const setup = r?.launcher || (r?.installed?.kind === "installer" ? r.installed.exe : null);
  if (setup) return runInstaller(g, setup);
}

function updateDlBadge() {
  const n = [...jobs.values()].filter((j) => j.state === "running").length;
  const c = $("#dl-count");
  if (!c) return;
  c.hidden = !n;
  c.textContent = n;
}

async function loadDlStorage() {
  dlStorage = await api.call("downloads_storage").catch(() => null);
  if (openSheet === "downloads") renderDownloadsSheet();
}

async function openDownloads() {
  for (const j of (await api.call("downloads_list").catch(() => [])) || []) jobs.set(j.id, j);
  updateDlBadge();
  open("downloads", $("#sheet-downloads"));
  loadDlStorage();
}

const JOB_STATE = { running: ["Downloading", "run"], paused: ["Paused", ""], waiting: ["Needs a yes", "run"], done: ["Done", "ok"], failed: ["Failed", "bad"] };
const KIND_LABEL = { game: "Game", launcher: "Launcher", mod: "Mod", update: "Beacon" };

function renderDownloadsSheet() {
  const keep = nav.current()?.dataset.key;
  const list = [...jobs.values()].sort((a, b) => (a.state === "running") - (b.state === "running") || a.id - b.id).reverse();
  const rows = $("#downloads-rows");
  rows.innerHTML = list.length
    ? list
        .map((j) => {
          const [stateLabel, cls] = JOB_STATE[j.state] ?? [j.state, ""];
          const g = j.game && snap.games.find((x) => x.id === j.game);
          const sub = j.state === "running" ? `${j.pct != null ? `${j.pct}% · ` : ""}${j.text}` : j.message || j.text;
          const actions = [
            j.state === "running" && `<button class="nav" data-key="pause:${j.id}">Pause</button>`,
            j.state === "paused" && `<button class="nav" data-key="retry:${j.id}">Resume</button>`,
            j.state === "failed" && `<button class="nav" data-key="retry:${j.id}">Try again</button>`,
            j.state === "waiting" && `<button class="nav" data-key="accept:${j.id}">Use what Beacon found</button>`,
            j.state !== "running" && `<button class="nav" data-key="forget:${j.id}">Remove</button>`,
          ].filter(Boolean);
          return `<div class="dl-job ${j.state}">
            <div class="dl-head"><span class="label">${esc(j.title)}</span><span class="badge ${cls}">${esc(stateLabel)}</span></div>
            ${j.state === "running" ? `<div class="bar${j.pct == null ? " unknown" : ""}"><span style="width:${j.pct ?? 0}%"></span></div>` : ""}
            <span class="sub">${esc(KIND_LABEL[j.kind] ?? "")}${g ? ` · ${esc(g.short)}` : ""} · ${esc(sub || "")}</span>
            ${actions.length ? `<div class="dl-actions">${actions.join("")}</div>` : ""}
          </div>`;
        })
        .join("")
    : `<p class="section-label" style="text-transform:none;letter-spacing:0;font-weight:400">Nothing downloading. Games, launchers, mods and Beacon updates you download show up here, with their progress.</p>`;
  const kept = dlStorage?.bytes ? `${fmtSize(dlStorage.bytes)} in ${dlStorage.files} file${dlStorage.files === 1 ? "" : "s"}` : "None";
  $("#downloads-foot").innerHTML =
    rowHTML({ key: "dl-folder", label: "Kept downloads", sub: `${kept} · partial downloads and setups kept to run again`, value: "Open" }) +
    (dlStorage?.bytes ? `<button class="nav row-btn danger" data-key="dl-clear"><svg viewBox="0 0 24 24"><use href="#i-x"/></svg>Clear kept downloads</button>` : "");
  const sheet = $("#sheet-downloads");
  const on = (key, fn) => sheet.querySelector(`[data-key="${key}"]`)?.addEventListener("click", fn);
  for (const j of list) {
    const g = j.game ? snap.games.find((x) => x.id === j.game) : null;
    on(`pause:${j.id}`, () => act(() => api.call("download_pause", { id: j.id })));
    on(`retry:${j.id}`, () => (j.retry.cmd === "app_update_apply" ? appUpdate() : runDownload(j.retry.cmd, j.retry.args, g, j.title)));
    on(`accept:${j.id}`, () => runDownload(j.retry.cmd, { ...j.retry.args, acceptGuess: true }, g, j.title));
    on(`forget:${j.id}`, async () => {
      await api.call("download_forget", { id: j.id }).catch(() => {});
      jobs.delete(j.id);
      renderDownloadsSheet();
    });
  }
  on("dl-folder", () => act(() => api.call("open_path", { path: `${snap.data_dir}/downloads` })));
  on("dl-clear", async () => {
    try {
      const freed = await api.call("downloads_clear");
      toast(`Cleared ${fmtSize(freed)}`);
      loadDlStorage();
    } catch (e) {
      toast(String(e?.message ?? e), true);
    }
  });
  nav.refocus(keep ? `[data-key="${CSS.escape(keep)}"]` : null);
}

// Reinstall: games Beacon downloads come straight back (saves and settings were kept).
reinstallers.push({ can: (g) => !!dlInfo?.games?.includes(g.id), run: (g) => cdnGet(g, {}) });

function onSourcesChecked({ checks, updated }) {
  if (dlInfo) dlInfo = { ...dlInfo, checks, ...(updated ? { version: updated.to, built_in: false } : {}) };
  if (updated) toast(`Download sources list updated to version ${updated.to}`);
  const bad = (checks || []).filter((c) => c.state === "broken");
  if (bad.length) toast(`A download source isn't answering: ${bad.map((c) => c.title).join(", ")}`, true);
  if (openSheet === "launcher" || openSheet === "game") render();
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
    const row = document.querySelector(`.game-row[data-index="${sel}"]`);
    if (nav.current()?.classList.contains("game-row")) nav.focus(row);
  } else if (b === "x") {
    if (openSheet === "game") closeSheet();
    else if (!openSheet) openGameSheet();
  } else if (b === "y") {
    if (openSheet === "mods") closeSheet();
    else if (!openSheet) loadMods(game().id).then(() => open("mods", $("#sheet-mods")));
  } else if (b === "start" && !openSheet) play();
  else if (b === "select") {
    if (openSheet === "launcher") closeSheet();
    else if (!openSheet) openLauncherSheet();
  }
});

nav.onInputChange(renderHints);

$("#btn-play").addEventListener("click", play);
$("#btn-mods").addEventListener("click", () => loadMods(game().id).then(() => open("mods", $("#sheet-mods"))));
const openGameSheet = () => loadCommunity(game().id).catch(() => {}).then(() => open("game", $("#sheet-game")));
$("#btn-game-settings").addEventListener("click", openGameSheet);
$("#btn-launcher-settings").addEventListener("click", openLauncherSheet);
$("#btn-downloads").addEventListener("click", openDownloads);
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
$("#overlay-ok").addEventListener("click", () => (modPending ? modInstall() : hideOverlay()));
// Coming back to Beacon while the All-in-One launcher is open: pick up games it finished downloading.
window.addEventListener("focus", () => {
  if (!snap || !aioOpenedAt || Date.now() - (refreshAfterAio.last || 0) < 15000) return;
  if (WORKSHOP_GAMES.every((id) => snap.games.find((g) => g.id === id)?.installed)) return;
  refreshAfterAio.last = Date.now();
  refreshAfterAio(false).catch(() => {});
});
// Games found without pressing "Find games" (start-up scan, new installs).
async function onGamesFound(ids) {
  snap = await api.call("get_state");
  render();
  const names = ids.map((id) => snap.games.find((g) => g.id === id)?.short ?? id);
  if (names.length) toast(`Found ${names.join(", ")}`);
}
window.addEventListener("focus", () => {
  if (!snap || Date.now() - (onGamesFound.lastLook || 0) < 20000 || snap.games.every((g) => g.installed)) return;
  onGamesFound.lastLook = Date.now();
  api.call("look_for_games").then((ids) => ids?.length && onGamesFound(ids)).catch(() => {});
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
  sel = Math.max(0, snap.games.findIndex((g) => g.installed));
  buildShelf();
  renderHints(false);
  render();
  await Promise.all(snap.games.map((g) => loadMods(g.id).catch(() => [])));
  render();
  nav.focus($("#btn-play"));
  api.on("game-event", onGameEvent);
  api.on("games-found", ({ games }) => onGamesFound(games).catch(() => {}));
  api.on("workshop-progress", onWorkshopProgress);
  api.on("aio-launcher-progress", onAioProgress);
  api.on("aio-launcher-closed", () => refreshAfterAio(true).catch(() => {}));
  api.on("community-launcher-progress", onCommunityLauncherProgress);
  api.on("cdn-progress", onCdnProgress);
  api.on("downloads-changed", (j) => {
    jobs.set(j.id, j);
    updateDlBadge();
    if (openSheet === "downloads") renderDownloadsSheet();
  });
  for (const j of (await api.call("downloads_list").catch(() => [])) || []) jobs.set(j.id, j);
  updateDlBadge();
  api.on("mod-progress", ({ pct, text }) => {
    if (!modBusy || $("#overlay").hidden) return;
    $("#overlay-text").textContent = pct == null ? text : `${pct}% · ${text}`;
  });
  api.on("sources-checked", onSourcesChecked);
  api.on("app-update", (c) => {
    appUpd = c;
    toast(`Beacon ${c.latest.version} is available — Launcher settings → Update`);
    if (openSheet === "launcher") render();
  });
  api.on("app-update-progress", ({ pct, text }) => {
    if (!appUpdBusy || $("#overlay").hidden) return;
    $("#overlay-text").textContent = pct == null ? text : `${pct}% · ${text}`;
  });
  dlInfo = await api.call("downloads_info").catch(() => null);
  api.on("community-launcher-closed", (e) => onCommunityLauncherClosed(e).catch(() => {}));
  tick();
  setInterval(tick, 15000);
})();
