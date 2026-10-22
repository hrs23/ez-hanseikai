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

function rects(n, id, width, height, gap, slots = n) {
  const c = cells(Math.max(n, slots), id);
  const unit = Math.max(0, Math.min((width - (c.cols - 1) * gap) / c.cols, (height - (c.rows - 1) * gap) / c.rows / ASPECT));
  const blockW = c.cols * unit + (c.cols - 1) * gap;
  const blockH = c.rows * unit * ASPECT + (c.rows - 1) * gap;
  const left = (width - blockW) / 2, top = (height - blockH) / 2;
  const px = (r) => ({ x: left + r.x * (unit + gap), y: top + r.y * (unit * ASPECT + gap), w: r.w * unit + (r.w - 1) * gap, h: r.h * unit * ASPECT + (r.h - 1) * gap });
  const all = [...c.tiles, ...c.free].map(px);
  return { tiles: all.slice(0, n), free: all.slice(n) };
}

function soundKey(view, keys, me, chosen) {
  if (view > 0) return keys[view - 1] || null;
  return [chosen, me].find((k) => keys.includes(k)) || keys[0] || null;
}

const seekSum = (run, delta) => (Math.sign(run) === Math.sign(delta) ? run + delta : delta);

const BOARD_COLS = 12;
const BOARD_ROWS = 16;

function boardSpan(start, dx, dy, colStep, rowStep) {
  const span = (n, max) => Math.min(max, Math.max(2, n));
  return { w: span(start.w + Math.round(dx / colStep), BOARD_COLS), h: span(start.h + Math.round(dy / rowStep), BOARD_ROWS) };
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
