const LAYOUTS = [
  { id: "grid", name: "Auto grid", min: 1, max: 9 },
  { id: "side", name: "Spotlight + side", min: 2, max: 6 },
  { id: "bottom", name: "Spotlight + bottom", min: 2, max: 6 },
  { id: "row", name: "Row", min: 2, max: 4 },
];
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

function rects(n, id, width, height, gap) {
  const c = cells(n, id);
  const unit = Math.max(0, Math.min((width - (c.cols - 1) * gap) / c.cols, (height - (c.rows - 1) * gap) / c.rows / ASPECT));
  const blockW = c.cols * unit + (c.cols - 1) * gap;
  const blockH = c.rows * unit * ASPECT + (c.rows - 1) * gap;
  const left = (width - blockW) / 2, top = (height - blockH) / 2;
  const px = (r) => ({ x: left + r.x * (unit + gap), y: top + r.y * (unit * ASPECT + gap), w: r.w * unit + (r.w - 1) * gap, h: r.h * unit * ASPECT + (r.h - 1) * gap });
  return { tiles: c.tiles.map(px), free: c.free.map(px) };
}
