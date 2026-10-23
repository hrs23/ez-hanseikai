const LAYOUTS = [
  { id: "grid", name: "Auto grid", min: 1, max: 9 },
  { id: "side", name: "Spotlight + side", min: 2, max: 6 },
  { id: "bottom", name: "Spotlight + bottom", min: 2, max: 6 },
  { id: "row", name: "Row", min: 2, max: 4 },
];
const MAX_SLOTS = 6;
const SPOTLIGHT_SPAN = 3;
const ASPECT = 9 / 16;

function layoutFits(id, n) {
  const l = LAYOUTS.find((x) => x.id === id);
  return !!l && n >= l.min && n <= l.max;
}

function cells(n, id) {
  if (!layoutFits(id, n)) id = "grid";
  if (id === "row") return { cols: n, rows: 1, tiles: [...Array(n).keys()].map((i) => ({ x: i, y: 0, w: 1, h: 1 })), free: [] };
  if (id === "side" || id === "bottom") {
    const k = Math.max(n - 1, SPOTLIGHT_SPAN);
    const small = [...Array(n - 1).keys()].map((i) => (id === "side" ? { x: k, y: i, w: 1, h: 1 } : { x: i, y: k, w: 1, h: 1 }));
    return { cols: id === "side" ? k + 1 : k, rows: id === "side" ? k : k + 1, tiles: [{ x: 0, y: 0, w: k, h: k }, ...small], free: [] };
  }
  const cols = Math.ceil(Math.sqrt(Math.max(1, n)));
  const rows = Math.ceil(Math.max(1, n) / cols);
  const at = (i) => ({ x: i % cols, y: Math.floor(i / cols), w: 1, h: 1 });
  return { cols, rows, tiles: [...Array(n).keys()].map(at), free: [...Array(cols * rows - n).keys()].map((i) => at(n + i)) };
}

function rects(n, id, width, height, gap, slots = n, strip = 0) {
  const c = cells(Math.max(n, slots), id);
  const unit = Math.max(0, Math.min((width - (c.cols - 1) * gap) / c.cols, (height - (c.rows - 1) * gap - c.rows * strip) / c.rows / ASPECT));
  const blockW = c.cols * unit + (c.cols - 1) * gap;
  const rowH = unit * ASPECT + strip;
  const blockH = c.rows * rowH + (c.rows - 1) * gap;
  const left = (width - blockW) / 2, top = (height - blockH) / 2;
  const px = (r) => ({ x: left + r.x * (unit + gap), y: top + r.y * (rowH + gap), w: r.w * unit + (r.w - 1) * gap, h: r.h * rowH + (r.h - 1) * gap });
  const all = [...c.tiles, ...c.free].map(px);
  return { tiles: all.slice(0, n), free: all.slice(n) };
}

function soundKey(view, keys, me, chosen) {
  if (view > 0) return keys[view - 1] || null;
  return [chosen, me].find((k) => keys.includes(k)) || keys[0] || null;
}

const MINIMAP = { at: 0.04, size: 0.235 };

function minimapRect(width, height) {
  if (!(width > 0 && height > 0)) return null;
  const at = MINIMAP.at * height;
  const side = Math.min(MINIMAP.size * height, width - at);
  return side > 0 ? { x: at / width, y: MINIMAP.at, w: side / width, h: side / height } : null;
}

// Checks on a small copy of the minimap crop (MINIMAP_LOOK.size square): the tile is hidden only when the crop is clearly not a minimap.
// flat: neighbouring pixels nearly equal (a menu panel, a loading screen); dark or bright: nearly all near black or near white (a black screen, a document).
const MINIMAP_LOOK = { size: 24, flatStep: 4, flatShare: 0.7, dark: 24, darkShare: 0.9, bright: 225, brightShare: 0.6, hideAfter: 3, showAfter: 2, everyMs: 500 };

function minimapLook(rgba, size = MINIMAP_LOOK.size) {
  const n = size * size;
  if (!rgba || rgba.length < n * 4) return null;
  const luma = new Float32Array(n);
  for (let i = 0; i < n; i++) luma[i] = 0.299 * rgba[i * 4] + 0.587 * rgba[i * 4 + 1] + 0.114 * rgba[i * 4 + 2];
  let flat = 0, pairs = 0, dark = 0, bright = 0;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const v = luma[y * size + x];
      if (v < MINIMAP_LOOK.dark) dark++;
      if (v > MINIMAP_LOOK.bright) bright++;
      if (x + 1 < size) { pairs++; if (Math.abs(v - luma[y * size + x + 1]) < MINIMAP_LOOK.flatStep) flat++; }
      if (y + 1 < size) { pairs++; if (Math.abs(v - luma[(y + 1) * size + x]) < MINIMAP_LOOK.flatStep) flat++; }
    }
  }
  const look = { flat: flat / pairs, dark: dark / n, bright: bright / n };
  look.not = look.flat > MINIMAP_LOOK.flatShare ? "flat" : look.dark > MINIMAP_LOOK.darkShare ? "dark" : look.bright > MINIMAP_LOOK.brightShare ? "bright" : "";
  return look;
}

// Hysteresis for the minimap tile: hide after a few checks in a row say "not a minimap", show again after a few say it is.
// A first check (state.shown null) or a still picture (now) decides at once.
function minimapShown(state, isMap, now = false) {
  if (state.shown === null || now) return { shown: isMap, run: 0 };
  if (isMap === state.shown) return { shown: state.shown, run: 0 };
  const run = state.run + 1;
  return run >= (state.shown ? MINIMAP_LOOK.hideAfter : MINIMAP_LOOK.showAfter) ? { shown: isMap, run: 0 } : { shown: state.shown, run };
}

const osdSpeed = (rate) => `${rate}×`;
const osdSeek = (run) => t(run < 0 ? "−{n} s" : "+{n} s", { n: Math.abs(run) });
const osdVolume = (volume, muted) => (muted || volume === 0 ? t("Muted") : t("Volume {n}%", { n: Math.round(volume * 100) }));

const seekSum = (run, delta) => (Math.sign(run) === Math.sign(delta) ? run + delta : delta);

const BOARD_COLS = 12;
const BOARD_ROWS = 16;
const BOARD_ROW_EM = 1.5;
const BOARD_WIDTH = 1800;
const BOARD_GROW = 1.15;
const BOARD_FONT_MIN = 10;

function boardSpan(start, dx, dy, colStep, rowStep) {
  const span = (n, max) => Math.min(max, Math.max(2, n));
  return { w: span(start.w + Math.round(dx / colStep), BOARD_COLS), h: span(start.h + Math.round(dy / rowStep), BOARD_ROWS) };
}

const boardFont = (base, width) => Math.round(base * Math.min(BOARD_GROW, Math.max(1, width / BOARD_WIDTH)) * 2) / 2;

const cellSpan = (px, unit, gap) => Math.max(1, Math.ceil((px + gap) / unit));

function fitFont(fits, high, low = BOARD_FONT_MIN) {
  let lo = 0;
  let hi = Math.max(0, Math.round((high - low) * 2));
  while (lo < hi) {
    const mid = Math.ceil((lo + hi) / 2);
    if (fits(low + mid / 2)) lo = mid;
    else hi = mid - 1;
  }
  return low + lo / 2;
}

function dropSlot(rects, x, y) {
  const hit = rects.findIndex((r) => x >= r.left && x <= r.right && y >= r.top && y <= r.bottom);
  if (hit >= 0) return x < (rects[hit].left + rects[hit].right) / 2 ? hit : hit + 1;
  return rects.filter((r) => r.bottom < y || (r.top <= y && r.right < x)).length;
}

function dropPos(positions, index) {
  const before = positions[index - 1];
  const after = positions[index];
  if (before === undefined) return after === undefined ? 0 : after - 1;
  return after === undefined ? before + 1 : (before + after) / 2;
}
