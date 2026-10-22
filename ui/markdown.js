const MD_HEADING_MAX = 6;
const MD_DEPTH_MAX = 8;
const BOARD_HEADING_BASE = 3;
const MD_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const MD_WEB = /^https?:\/\/[^\s"<>]+$/i;
const MD_INLINE = new RegExp([
  "\\\\([\\\\`*_{}\\[\\]()#+\\-.!~=|<>])",
  "(?<!`)(`+)(?!`)([^]*?[^`])\\2(?!`)",
  "!?\\[([^\\]]*)\\]\\(((?:[^()\\s]|\\([^()\\s]*\\))+)(?:\\s+\"[^\"]*\")?\\)",
  "<(https?:\\/\\/[^>\\s]+)>",
  "(https?:\\/\\/[^\\s<>]+)",
  "(ezh:\\/\\/moment\\/[^\\s)]+)",
  "\\*\\*(.+?)\\*\\*",
  "__(.+?)__",
  "~~(.+?)~~",
  "==(.+?)==",
  "\\*(?!\\s)(.+?)(?<!\\s)\\*",
  "(?<![\\p{L}\\p{N}_])_(?!\\s)(.+?)(?<!\\s)_(?![\\p{L}\\p{N}_])",
  "(?<![\\p{L}\\p{N}_&])#([\\p{L}\\p{N}_-]+)",
].join("|"), "gu");

function mdHosts(label) {
  return label.split(/\s+/).flatMap((w) => {
    try {
      const u = new URL(/^[a-z][\w+.-]*:\/\//i.test(w) ? w : `http://${w}`);
      return u.username || u.password ? [] : [u.host.replace(/^www\./, "")];
    } catch { return []; }
  });
}

function mdVouched(url, label) {
  const host = new URL(url).host;
  return !/(^|\.)xn--/i.test(host) && mdHosts(label).includes(host.replace(/^www\./, ""));
}

async function openWeb(url, label) {
  if (!mdVouched(url, label) && !(await ask(t("Open {host}?", { host: new URL(url).host }), t("Open"), false))) return;
  dialog("open_link", { url }).catch(warn);
}
const mdHooks = (over = {}) => ({ onLink: openLink, onOpen: openWeb, names: memberNames(), who: (name) => mentionWho(name, null), ...over });

function mdHref(url) {
  if (/^ezh:\/\/moment\//i.test(url)) {
    const link = parseLink(url);
    return link ? { kind: "moment", link } : null;
  }
  if (!MD_WEB.test(url)) return null;
  try { return { kind: "web", url: new URL(url).href }; } catch { return null; }
}

function mdLink(url, label, raw) {
  const href = mdHref(url);
  return href ? { link: href, children: label } : { text: raw };
}

function urlEnd(url) {
  let end = url.length;
  const count = (c) => url.slice(0, end).split(c).length - 1;
  while (end > 0 && (".,;:!?'\"".includes(url[end - 1]) || (url[end - 1] === ")" && count(")") > count("(")))) end--;
  return url.slice(0, end);
}

function mdInline(text) {
  const out = [];
  let last = 0;
  for (const m of text.matchAll(MD_INLINE)) {
    if (m.index > last) out.push({ text: text.slice(last, m.index) });
    last = m.index + m[0].length;
    if (m[1] !== undefined) out.push({ text: m[1] });
    else if (m[3] !== undefined) out.push({ code: m[3].trim() });
    else if (m[5] !== undefined) out.push(m[0].startsWith("!") ? { text: m[0] } : mdLink(m[5], mdInline(m[4]), m[0]));
    else if (m[6] !== undefined) out.push(mdLink(m[6], [{ text: m[6] }], m[0]));
    else if (m[7] !== undefined) {
      const url = urlEnd(m[7]);
      last = m.index + url.length;
      out.push(mdLink(url, [{ text: url }], url));
    }
    else if (m[8] !== undefined) out.push(mdLink(m[8], [], m[0]));
    else if (m[9] !== undefined) out.push({ strong: mdInline(m[9]) });
    else if (m[10] !== undefined) out.push({ strong: mdInline(m[10]) });
    else if (m[11] !== undefined) out.push({ strike: mdInline(m[11]) });
    else if (m[12] !== undefined) out.push({ mark: mdInline(m[12]) });
    else if (m[13] !== undefined) out.push({ em: mdInline(m[13]) });
    else if (m[14] !== undefined) out.push({ em: mdInline(m[14]) });
    else out.push({ tag: m[15] });
  }
  if (last < text.length) out.push({ text: text.slice(last) });
  return out;
}

const mdIndent = (s) => s.match(/^\s*/)[0].length;

function mdBlocks(lines, depth = 0) {
  const blocks = [];
  let i = 0;
  const item = (s) => depth < MD_DEPTH_MAX && MD_ITEM.test(s);
  const starts = (line) => /^\s{0,3}#{1,6}\s/.test(line.s) || item(line.s);
  while (i < lines.length) {
    const line = lines[i];
    if (!line.s.trim()) { i++; continue; }
    const head = line.s.match(/^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/);
    if (head) { blocks.push({ type: "heading", level: head[1].length, inline: mdInline(head[2]) }); i++; continue; }
    if (item(line.s)) {
      const run = [];
      const kind = (s) => /\d/.test(s.match(MD_ITEM)[2]);
      const base = mdIndent(line.s);
      while (i < lines.length) {
        const s = lines[i].s;
        if (run.length && MD_ITEM.test(s) && mdIndent(s) <= base && kind(s) !== kind(run[0].s)) break;
        if (MD_ITEM.test(s) || (s.trim() && mdIndent(s) > 0 && run.length)) { run.push(lines[i++]); continue; }
        if (!s.trim() && i + 1 < lines.length && (MD_ITEM.test(lines[i + 1].s) || mdIndent(lines[i + 1].s) > 0) && lines[i + 1].s.trim()) { i++; continue; }
        break;
      }
      blocks.push(mdList(run, depth));
      continue;
    }
    const para = [];
    while (i < lines.length && lines[i].s.trim() && !(para.length && starts(lines[i]))) para.push(lines[i++].s.trim());
    if (para.length) blocks.push({ type: "paragraph", inline: mdInline(para.join("\n")) });
    else i++;
  }
  return blocks;
}

function mdList(lines, depth) {
  const base = mdIndent(lines[0].s);
  const first = lines[0].s.match(MD_ITEM);
  const ordered = /\d/.test(first[2]);
  const list = { type: "list", ordered, start: ordered ? parseInt(first[2], 10) : null, items: [] };
  let item = null;
  let rest = [];
  const close = () => {
    if (item && rest.length) item.blocks = mdBlocks(rest.map((l) => ({ s: l.s.slice(Math.min(mdIndent(l.s), mdIndent(rest[0].s))) })), depth + 1);
    rest = [];
  };
  for (const line of lines) {
    const m = line.s.match(MD_ITEM);
    if (m && mdIndent(line.s) <= base) {
      close();
      item = { inline: mdInline(m[3]), blocks: [] };
      list.items.push(item);
    } else if (item) rest.push(line);
  }
  close();
  return list;
}

function mdParse(text) {
  const lines = String(text).replace(/\r\n?/g, "\n").replaceAll("\t", "    ").split("\n").map((s) => ({ s }));
  return mdBlocks(lines);
}

const mdText = (nodes) => nodes.map((n) => n.text ?? n.code ?? mdText(n.strong || n.em || n.strike || n.mark || n.children || [])).join("");

function chipEl(kind, value, label, ids = [], face = null) {
  const b = el("button", { type: "button", className: "tag", onclick: (e) => { e.stopPropagation(); pickChip(kind, value, ids); } });
  if (face) b.append(avatar(face, "xs"));
  b.append(label);
  b.dataset.kind = kind;
  b.dataset.value = value;
  return b;
}

function mentionEl(name, hooks) {
  const who = hooks.who(name);
  const face = who.ids.length === 1 && memberNow()[who.ids[0]] ? who.ids[0] : null;
  return chipEl("mention", who.value, "@" + who.value, who.ids, face);
}

function mdSpans(nodes, hooks) {
  const wrap = (tag, children, props = {}) => el(tag, props, ...mdSpans(children, hooks));
  return nodes.flatMap((n) => {
    if (n.text !== undefined) {
      const parts = n.text.split("\n");
      return parts.flatMap((p, k) => (k ? [el("br"), p] : [p])).flatMap((p) => (typeof p === "string" ? mentionParts(p, hooks.names).map((q) => (q.mention ? mentionEl(q.mention, hooks) : q)) : [p]));
    }
    if (n.code !== undefined) return [el("code", { textContent: n.code })];
    if (n.strong) return [wrap("strong", n.strong)];
    if (n.em) return [wrap("em", n.em)];
    if (n.strike) return [wrap("del", n.strike)];
    if (n.mark) return [wrap("mark", n.mark)];
    if (n.tag !== undefined) return [chipEl("tag", n.tag, tagName(n.tag))];
    if (n.link.kind === "moment") {
      const label = mdText(n.children) || t("Moment");
      return [el("button", { className: "link", onclick: (e) => { e.stopPropagation(); hooks.onLink(n.link.link); } }, icon("play", "sm"), `${label} ${fmt(n.link.link.m)}`)];
    }
    const open = (e) => { e.preventDefault(); e.stopPropagation(); hooks.onOpen(n.link.url, mdText(n.children)); };
    const a = el("a", { className: "ext", role: "link", tabIndex: 0, title: n.link.url, onclick: open, onauxclick: (e) => { if (e.button === 1) open(e); }, onkeydown: (e) => { if (e.key === "Enter") open(e); } }, ...(n.children.length ? mdSpans(n.children, hooks) : [n.link.url]));
    return [a];
  });
}

function mdNodes(blocks, hooks, base) {
  return blocks.map((b) => {
    if (b.type === "heading") return el(`h${Math.min(MD_HEADING_MAX, b.level + base)}`, {}, ...mdSpans(b.inline, hooks));
    if (b.type === "paragraph") return el("p", {}, ...mdSpans(b.inline, hooks));
    const items = b.items.map((it) => el("li", {}, el("span", {}, ...mdSpans(it.inline, hooks)), ...mdNodes(it.blocks, hooks, base)));
    return el(b.ordered ? "ol" : "ul", b.ordered ? { start: b.start } : {}, ...items);
  });
}

function cardParts(text) {
  const [first, ...rest] = String(text).replace(/\r\n?/g, "\n").split("\n");
  return { head: mdInline(first.replace(/^\s*#{1,6}\s+/, "")), body: mdParse(rest.join("\n")) };
}

function cardDom(text, hooks) {
  const c = cardParts(text);
  return el("div", { className: "cardmd" }, c.head.length ? el("p", { className: "chead" }, ...mdSpans(c.head, hooks)) : null, ...mdNodes(c.body, hooks, BOARD_HEADING_BASE));
}
