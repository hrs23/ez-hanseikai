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

function inline(text, onTag = filterTag, onLink = openLink) {
  const out = [];
  const pattern = /(\*\*[^*]+\*\*|==[^=]+==|\[[^\]]+\]\(ezh:\/\/moment\/[^)\s]+\)|ezh:\/\/moment\/[^\s)]+|#[\p{L}\p{N}_-]+)/gu;
  let last = 0;
  for (const m of text.matchAll(pattern)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const s = m[0];
    if (s.startsWith("**")) out.push(el("b", { textContent: s.slice(2, -2) }));
    else if (s.startsWith("==")) out.push(el("mark", { textContent: s.slice(2, -2) }));
    else if (s.startsWith("[")) {
      const label = s.slice(1, s.indexOf("]("));
      const url = s.slice(s.indexOf("](") + 2, -1);
      out.push(linkButton(label, url, onLink));
    } else if (s.startsWith("ezh://")) out.push(linkButton(t("Moment"), s, onLink));
    else {
      const tag = s.slice(1);
      out.push(el("button", { className: "tag", textContent: tagName(tag), onclick: (e) => { e.stopPropagation(); onTag(tag); } }));
    }
    last = m.index + s.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

function linkButton(label, url, onLink) {
  const link = parseLink(url);
  return el("button", { className: "link", textContent: link ? `${label} ${fmt(link.m)}` : label, onclick: (e) => { e.stopPropagation(); if (link) onLink(link); } });
}

function tagName(tag) {
  const names = (jobInfo && jobInfo.tags) || {};
  return "#" + (names[tag] || tag);
}

function markdown(text, onTag = filterTag, onLink = openLink) {
  const blocks = [];
  let list = null;
  for (const line of text.split("\n")) {
    const bullet = line.match(/^\s*[-*]\s+(.*)$/);
    if (bullet) {
      if (!list) blocks.push((list = el("ul")));
      list.append(el("li", {}, ...inline(bullet[1], onTag, onLink)));
      continue;
    }
    list = null;
    const head = line.match(/^(#{1,3})\s+(.*)$/);
    if (head) blocks.push(el(`h${head[1].length + 3}`, {}, ...inline(head[2], onTag, onLink)));
    else if (line.trim()) blocks.push(el("p", {}, ...inline(line, onTag, onLink)));
  }
  return blocks;
}

function chaptersOf(list) {
  return list.filter((e) => e.role === "chapter").sort((a, b) => a.m - b.m);
}

function activeAt(list, m) {
  return list.filter((e) => e.m !== null && m >= e.m && m <= (e.end !== null ? e.end : e.m + NOTE_SHOW_S));
}
