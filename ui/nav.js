// Controller + keyboard navigation.
//
// Focusable things carry the `nav` class. Layers (main screen, sheets,
// overlay) stack; only the top one takes input. D-pad / left stick /
// arrow keys move focus spatially; A/Enter activates; B/Escape goes back.
// Rows with `data-cycle` use left/right to change their value instead.

const stack = [];
let focused = null;
let buttonsDown = [];
let axisHeld = null;
let axisNext = 0;
let usingPad = false;
const handlers = { back: null, shortcut: null, inputChange: null };

export function onBack(fn) { handlers.back = fn; }
/** fn(name) for: lb, rb, x, y, start, select */
export function onShortcut(fn) { handlers.shortcut = fn; }
/** fn(usingPad) when the input device switches, to swap button hints. */
export function onInputChange(fn) { handlers.inputChange = fn; }

function setPad(v) {
  if (usingPad !== v) {
    usingPad = v;
    handlers.inputChange?.(v);
  }
}

export function pushLayer(el, focusTarget) {
  stack.push({ el, restore: focused });
  el.hidden = false;
  requestAnimationFrame(() => focus(focusTarget || firstIn(el)));
}

export function popLayer() {
  const top = stack.pop();
  if (!top) return;
  top.el.hidden = true;
  const restore = top.restore && document.body.contains(top.restore) && !top.restore.closest("[hidden]") ? top.restore : firstIn(topLayer());
  focus(restore);
}

export function topLayer() { return stack.length ? stack[stack.length - 1].el : document.getElementById("main-layer"); }
export function layerDepth() { return stack.length; }

function visible(el) {
  if (el.disabled || el.closest("[hidden]")) return false;
  const r = el.getBoundingClientRect();
  return r.width > 0 && r.height > 0;
}

function candidates() { return [...topLayer().querySelectorAll(".nav")].filter(visible); }
function firstIn(layer) { return [...layer.querySelectorAll(".nav")].find(visible) || null; }

export function focus(el) {
  if (focused && focused !== el) focused.classList.remove("focused");
  focused = el || null;
  if (!focused) return;
  focused.classList.add("focused");
  if (focused.matches("button, [tabindex]")) focused.focus({ preventScroll: true });
  focused.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
}

export function current() { return focused; }

/** Re-attach focus after a re-render replaced the focused element. */
export function refocus(selector) {
  const el = selector ? topLayer().querySelector(selector) : null;
  if (el && visible(el)) return focus(el);
  if (!focused || !document.body.contains(focused) || !visible(focused) || !topLayer().contains(focused)) focus(firstIn(topLayer()));
}

function move(dir) {
  if (focused?.dataset.cycle !== undefined && (dir === "left" || dir === "right")) {
    focused.dispatchEvent(new CustomEvent("cycle", { detail: dir === "left" ? -1 : 1 }));
    return;
  }
  const items = candidates();
  if (!focused || !items.includes(focused)) return focus(items[0]);
  const from = focused.getBoundingClientRect();
  const fx = from.left + from.width / 2, fy = from.top + from.height / 2;
  let best = null, bestScore = Infinity;
  for (const el of items) {
    if (el === focused) continue;
    const r = el.getBoundingClientRect();
    const x = r.left + r.width / 2, y = r.top + r.height / 2;
    let main, cross;
    if (dir === "left") { main = from.left - r.right; cross = Math.abs(y - fy); if (x >= fx - 1) continue; }
    if (dir === "right") { main = r.left - from.right; cross = Math.abs(y - fy); if (x <= fx + 1) continue; }
    if (dir === "up") { main = from.top - r.bottom; cross = Math.abs(x - fx); if (y >= fy - 1) continue; }
    if (dir === "down") { main = r.top - from.bottom; cross = Math.abs(x - fx); if (y <= fy + 1) continue; }
    const score = Math.max(main, 0) + cross * 2.2;
    if (score < bestScore) { bestScore = score; best = el; }
  }
  // Entering a group (e.g. the game shelf) lands on its selected member.
  const group = best?.dataset.group;
  if (group && focused.dataset.group !== group) {
    best = topLayer().querySelector(`[data-group="${group}"].selected`) || best;
  }
  if (best) focus(best);
}

function activate() {
  if (!focused) return;
  if (focused.matches(".field")) {
    focused.querySelector("input")?.focus();
    return;
  }
  focused.click();
}

function back() {
  const ae = document.activeElement;
  if (ae && ae.tagName === "INPUT") { ae.blur(); return; }
  handlers.back?.();
}

// ---- keyboard ----
window.addEventListener("keydown", (e) => {
  setPad(false);
  const typing = document.activeElement?.tagName === "INPUT";
  if (typing) {
    if (e.key === "Escape" || e.key === "Enter") { e.preventDefault(); document.activeElement.blur(); }
    return;
  }
  const dirs = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };
  if (dirs[e.key]) { e.preventDefault(); move(dirs[e.key]); }
  else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); activate(); }
  else if (e.key === "Escape" || e.key === "Backspace") { e.preventDefault(); back(); }
  else if (e.key === "q" || e.key === "PageUp") handlers.shortcut?.("lb");
  else if (e.key === "e" || e.key === "PageDown") handlers.shortcut?.("rb");
  else if (e.key === "m") handlers.shortcut?.("y");
  else if (e.key === "s") handlers.shortcut?.("x");
});

// Mouse/touch: hovering moves focus too, so the two never disagree.
document.addEventListener("pointermove", (e) => {
  setPad(false);
  const el = e.target.closest?.(".nav");
  if (el && el !== focused && topLayer().contains(el)) focus(el);
});

// ---- gamepad (standard mapping) ----
const MAP = { 0: "a", 1: "b", 2: "x", 3: "y", 4: "lb", 5: "rb", 8: "select", 9: "start", 12: "up", 13: "down", 14: "left", 15: "right" };

function poll(t) {
  const pads = navigator.getGamepads ? [...navigator.getGamepads()].filter(Boolean) : [];
  const pressed = new Set();
  let ax = 0, ay = 0;
  for (const p of pads) {
    p.buttons.forEach((b, i) => { if (b.pressed && MAP[i]) pressed.add(MAP[i]); });
    if (Math.abs(p.axes[0] || 0) > Math.abs(ax)) ax = p.axes[0];
    if (Math.abs(p.axes[1] || 0) > Math.abs(ay)) ay = p.axes[1];
  }
  const now = [...pressed];
  const fresh = now.filter((b) => !buttonsDown.includes(b));
  if (fresh.length) setPad(true);
  buttonsDown = now;
  const typing = document.activeElement?.tagName === "INPUT";
  for (const b of fresh) {
    if (typing && b !== "b") continue;
    if (b === "a") activate();
    else if (b === "b") back();
    else if (["up", "down", "left", "right"].includes(b)) { move(b); axisHeld = b; axisNext = t + 380; }
    else handlers.shortcut?.(b);
  }
  // Held d-pad or stick repeats.
  let stick = null;
  if (Math.abs(ax) > 0.55 || Math.abs(ay) > 0.55) stick = Math.abs(ax) > Math.abs(ay) ? (ax < 0 ? "left" : "right") : (ay < 0 ? "up" : "down");
  const held = stick || ["up", "down", "left", "right"].find((d) => pressed.has(d)) || null;
  if (!typing) {
    if (held && held !== axisHeld) { setPad(true); move(held); axisHeld = held; axisNext = t + 380; }
    else if (held && t >= axisNext) { move(held); axisNext = t + 120; }
  }
  if (!held) axisHeld = null;
  requestAnimationFrame(poll);
}
requestAnimationFrame(poll);
window.addEventListener("gamepadconnected", () => setPad(true));
