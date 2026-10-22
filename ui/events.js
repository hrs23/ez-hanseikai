const NOTE_SHOW_S = 8;
const MENTION_QUERY_MAX = 40;
const OVERLAY_MAX = 4;

function role(e) {
  if (e.kind === "board") return "board";
  if (e.kind === "drawing") return "drawing";
  if (e.end_us !== null && e.end_us !== undefined) return "chapter";
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
  return [e.text, scalars(e.values)].filter(Boolean).join(" ") || kindLabel(e.kind);
}

function noteCount(s, byKey = idx.byKey) {
  return s.events.filter((k) => { const e = byKey.get(k); return e && e.kind === "note"; }).length;
}

function matchCount(s, byKey = idx.byKey) {
  return s.events.filter((k) => { const e = byKey.get(k); return e && e.plugin && (e.tags || []).includes(MATCH_TAG); }).length;
}

function rowActions(n) {
  const own = !!n.mine && n.kind !== "mark";
  return [n.seekable && n.m !== null && "go", n.text && "copy", n.m !== null && "link", own && "edit", own && "delete"].filter(Boolean);
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

function inline(text, stored) {
  const names = stored ? [...new Set(stored.map((m) => m.name))].sort((a, b) => b.length - a.length) : memberNames();
  return mdSpans(mdInline(text), mdHooks({ names, who: (name) => mentionWho(name, stored) }));
}

function memberNow() {
  return { ...((st && st.names) || {}), ...(st && st.id && st.name ? { [st.id]: st.name } : {}) };
}

function memberNames() {
  const all = [...Object.values((st && st.names) || {}), st && st.name].filter((n) => typeof n === "string" && n.trim());
  return [...new Set(all)].sort((a, b) => b.length - a.length);
}

function mentionWho(name, stored) {
  const key = name.toLowerCase();
  const now = memberNow();
  const ids = stored ? stored.filter((m) => m.name.toLowerCase() === key).map((m) => m.id) : Object.keys(now).filter((id) => String(now[id]).toLowerCase() === key);
  return { value: ids.length === 1 && now[ids[0]] ? now[ids[0]] : name, ids };
}

function mentionParts(text, names) {
  const out = [];
  let last = 0;
  for (let i = text.indexOf("@"); i >= 0; i = text.indexOf("@", i + 1)) {
    if (i > 0 && /[\p{L}\p{N}_]/u.test(text[i - 1])) continue;
    const rest = text.slice(i + 1).toLowerCase();
    const name = names.find((n) => rest.startsWith(n.toLowerCase()) && !(/\w$/.test(n) && /\w/.test(text[i + 1 + n.length] || "")));
    if (!name) continue;
    if (i > last) out.push(text.slice(last, i));
    out.push({ mention: text.slice(i + 1, i + 1 + name.length) });
    last = i + 1 + name.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

const chipIs = (chip, kind, value) => !!chip && chip.kind === kind && chip.value.toLowerCase() === value.toLowerCase();

function nextChip(chip, kind, value, ids, set, here) {
  return here && chipIs(chip, kind, value) ? null : { kind, value, ids, set };
}

function chipMatches(n, chip) {
  if (!chip) return true;
  const want = chip.value.toLowerCase();
  if (chip.kind === "tag") return (n.tags || []).some((t) => t.toLowerCase() === want);
  if (Array.isArray(n.mentions)) return n.mentions.some((m) => (chip.ids || []).includes(m.id));
  return mentionParts(n.body || n.text || "", memberNames()).some((p) => p.mention && p.mention.toLowerCase() === want);
}

function hintAt(text, caret, tags, names, limit) {
  const before = text.slice(0, caret);
  const tag = before.match(/#([\p{L}\p{N}_-]*)$/u);
  if (tag) {
    const q = tag[1].toLowerCase();
    return { trigger: "#", start: caret - tag[0].length, items: tags.filter((x) => x.toLowerCase().startsWith(q) && x.toLowerCase() !== q).slice(0, limit) };
  }
  const at = before.lastIndexOf("@");
  const q = before.slice(at + 1).toLowerCase();
  if (at < 0 || (at > 0 && /[\p{L}\p{N}_]/u.test(before[at - 1])) || /[#@\n]/.test(q) || q.length > MENTION_QUERY_MAX) return { trigger: "", start: caret, items: [] };
  return { trigger: "@", start: at, items: names.filter((x) => x.toLowerCase().startsWith(q) && x.toLowerCase() !== q).slice(0, limit) };
}

function applyHint(text, caret, hint, value) {
  const head = text.slice(0, hint.start) + hint.trigger + value + " ";
  return { text: head + text.slice(caret), caret: head.length };
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

const isMatchEnd = (n) => n.role === "moment" && !!n.plugin && n.m !== null && (n.tags || []).includes(MATCH_TAG);
const matchLabel = (n) => t("Match {n}", { n });
const shownTags = (n) => (n.tags || []).filter((tg) => !(n.plugin && tg === MATCH_TAG));

function matchEnds(list) {
  return list.filter(isMatchEnd).map((n) => n.m).sort((a, b) => a - b);
}

function labelMatchEnds(list) {
  const order = list.filter(isMatchEnd).sort((a, b) => a.m - b.m);
  return list.map((n) => {
    const i = order.indexOf(n);
    if (i < 0) return n;
    const text = t("Match {n} end", { n: i + 1 });
    return { ...n, body: text, text };
  });
}

const noteMoments = (list) => list.filter((n) => (n.kind === "note" || n.kind === "mark") && n.role === "moment" && n.m !== null).sort((a, b) => a.m - b.m);

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

