const NOTE_SHOW_S = 8;
const OVERLAY_MAX = 4;

function role(e) {
  if (e.kind === "board") return "board";
  if (e.kind === "drawing") return "drawing";
  if (e.end_us !== null && e.end_us !== undefined) return e.file && e.text ? "speech" : "chapter";
  return "moment";
}

function kindLabel(kind) {
  const words = String(kind || "").replaceAll("_", " ");
  return t(words.charAt(0).toUpperCase() + words.slice(1));
}

function scalars(values) {
  return Object.entries(values || {}).filter(([, v]) => v !== null && typeof v !== "object").map(([k, v]) => `${k} ${v}`).join(" ");
}

function eventText(e) {
  if (role(e) === "drawing") return t("Drawing");
  return [e.text, scalars(e.values)].filter(Boolean).join(" ") || kindLabel(e.kind);
}

function momentLink(set, m, view) {
  return `ezh://moment/${encodeURIComponent(set)}/${Math.round(m * 10) / 10}/${view || "all"}`;
}

function parseLink(text) {
  const parts = text.replace("ezh://moment/", "").split("/");
  const m = Number(parts[1]);
  if (parts.length < 2 || !Number.isFinite(m) || m < 0) return null;
  let set;
  try { set = decodeURIComponent(parts[0]); } catch { return null; }
  return { set, m, view: parts[2] && parts[2] !== "all" ? parts[2] : null };
}

function inline(text) {
  return mdSpans(mdInline(text), mdHooks());
}

function tagName(tag) {
  const names = (st && st.tag_names) || {};
  return "#" + (names[tag] || tag);
}

function chaptersOf(list) {
  return list.filter((e) => e.role === "chapter").sort((a, b) => a.m - b.m);
}

function activeAt(list, m) {
  return list.filter((e) => e.m !== null && m >= e.m && m <= (e.end !== null ? e.end : e.m + NOTE_SHOW_S));
}

const MATCH_TAG = "match";
const MATCH_LEAD_S = 60;
const DRAWING_GROUP_S = 1;

function matchEnds(list) {
  return list.filter((n) => n.role === "moment" && n.plugin && n.m !== null && (n.tags || []).includes(MATCH_TAG)).map((n) => n.m).sort((a, b) => a - b);
}

function matchChapters(ends, total, live) {
  const out = [];
  let from = 0;
  ends.forEach((end, i) => { out.push({ m: from, end, n: i + 1 }); from = end; });
  if (live && ends.length && total > from) out.push({ m: from, end: total, n: ends.length + 1 });
  return out;
}

function stepTarget(times, m, dir, gap) {
  const sorted = [...times].sort((a, b) => a - b);
  return dir > 0 ? sorted.find((x) => x > m + gap) : sorted.reverse().find((x) => x < m - gap);
}

const matchTargets = (ends) => ends.map((e) => Math.max(0, e - MATCH_LEAD_S));

function groupDrawings(list) {
  const out = [];
  for (const n of list) {
    const last = out[out.length - 1];
    if (n.role === "drawing" && last && last.role === "drawing" && last.author === n.author && last.setId === n.setId && Math.abs((n.m ?? 0) - (last.m ?? 0)) <= DRAWING_GROUP_S) last.count += 1;
    else out.push(n.role === "drawing" ? { ...n, count: 1 } : n);
  }
  return out;
}

