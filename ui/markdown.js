const MD_HEADING_MAX = 6;
const MD_DEPTH_MAX = 8;
const BOARD_HEADING_BASE = 3;
const MD_FENCE = /^\s*(```+|~~~+)\s*([\w+-]*)\s*$/;
const MD_RULE = /^\s*([-*_])(\s*\1){2,}\s*$/;
const MD_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const MD_TASK = /^\[( |x|X)\]\s+(.*)$/;
const MD_TABLE_ROW = /\|/;
const MD_TABLE_RULE = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;
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
  if (!mdVouched(url, label) && !(await ask(t("Open {host}?", { host: new URL(url).host }), t("Open"), false, url))) return;
  invoke("open_link", { url }).catch(warn);
}
const mdHooks = (over = {}) => ({ onTag: filterTag, onLink: openLink, onOpen: openWeb, onTask: null, ...over });

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

function mdCells(row) {
  const cells = row.trim().replace(/^\|/, "").replace(/\|$/, "").split(/(?<!\\)\|/);
  return cells.map((c) => c.replaceAll("\\|", "|").trim());
}

function mdAlign(cell) {
  const c = cell.trim();
  if (c.startsWith(":") && c.endsWith(":")) return "center";
  return c.endsWith(":") ? "right" : "";
}

function mdBlocks(lines, depth = 0) {
  const blocks = [];
  let i = 0;
  const nest = depth < MD_DEPTH_MAX;
  const quote = (s) => nest && /^\s*>/.test(s);
  const item = (s) => nest && MD_ITEM.test(s);
  const starts = (line) => MD_FENCE.test(line.s) || /^\s{0,3}#{1,6}\s/.test(line.s) || MD_RULE.test(line.s) || quote(line.s) || item(line.s);
  while (i < lines.length) {
    const line = lines[i];
    if (!line.s.trim()) { i++; continue; }
    const fence = line.s.match(MD_FENCE);
    if (fence) {
      const body = [];
      i++;
      while (i < lines.length && !(MD_FENCE.test(lines[i].s) && lines[i].s.trim().startsWith(fence[1][0]))) body.push(lines[i++].s);
      i++;
      blocks.push({ type: "code", lang: fence[2], text: body.join("\n") });
      continue;
    }
    const head = line.s.match(/^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/);
    if (head) { blocks.push({ type: "heading", level: head[1].length, inline: mdInline(head[2]) }); i++; continue; }
    if (MD_RULE.test(line.s)) { blocks.push({ type: "rule" }); i++; continue; }
    if (quote(line.s)) {
      const inner = [];
      while (i < lines.length && /^\s*>/.test(lines[i].s)) { inner.push({ s: lines[i].s.replace(/^\s*>\s?/, ""), n: lines[i].n }); i++; }
      blocks.push({ type: "quote", blocks: mdBlocks(inner, depth + 1) });
      continue;
    }
    if (MD_TABLE_ROW.test(line.s) && i + 1 < lines.length && MD_TABLE_RULE.test(lines[i + 1].s) && lines[i + 1].s.includes("-")) {
      const head = mdCells(line.s);
      const rule = mdCells(lines[i + 1].s);
      if (head.length === rule.length) {
        i += 2;
        const rows = [];
        while (i < lines.length && lines[i].s.trim() && MD_TABLE_ROW.test(lines[i].s)) rows.push(mdCells(lines[i++].s));
        blocks.push({ type: "table", align: rule.map(mdAlign), head: head.map(mdInline), rows: rows.map((r) => r.slice(0, head.length).map(mdInline)) });
        continue;
      }
    }
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
  const list = { type: "list", ordered: /\d/.test(first[2]), items: [] };
  let item = null;
  let rest = [];
  const close = () => {
    if (item && rest.length) item.blocks = mdBlocks(rest.map((l) => ({ s: l.s.slice(Math.min(mdIndent(l.s), mdIndent(rest[0].s))), n: l.n })), depth + 1);
    rest = [];
  };
  for (const line of lines) {
    const m = line.s.match(MD_ITEM);
    if (m && mdIndent(line.s) <= base) {
      close();
      const task = m[3].match(MD_TASK);
      item = { inline: mdInline(task ? task[2] : m[3]), task: task ? task[1] !== " " : null, n: line.n, blocks: [] };
      list.items.push(item);
    } else if (item) rest.push(line);
  }
  close();
  return list;
}

function mdParse(text, firstLine = 0) {
  const lines = String(text).replace(/\r\n?/g, "\n").replaceAll("\t", "    ").split("\n").map((s, k) => ({ s, n: firstLine + k }));
  return mdBlocks(lines);
}

function toggleTask(text, n) {
  const lines = text.split("\n");
  if (n < 0 || n >= lines.length) return text;
  lines[n] = lines[n].replace(/^((?:\s*>\s?)*\s*(?:[-*+]|\d{1,9}[.)])\s+)\[( |x|X)\]/, (all, lead, mark) => `${lead}[${mark === " " ? "x" : " "}]`);
  return lines.join("\n");
}

const mdText = (nodes) => nodes.map((n) => n.text ?? n.code ?? mdText(n.strong || n.em || n.strike || n.mark || n.children || [])).join("");

function mdSpans(nodes, hooks) {
  const wrap = (tag, children, props = {}) => el(tag, props, ...mdSpans(children, hooks));
  return nodes.flatMap((n) => {
    if (n.text !== undefined) {
      const parts = n.text.split("\n");
      return parts.flatMap((p, k) => (k ? [el("br"), p] : [p]));
    }
    if (n.code !== undefined) return [el("code", { textContent: n.code })];
    if (n.strong) return [wrap("strong", n.strong)];
    if (n.em) return [wrap("em", n.em)];
    if (n.strike) return [wrap("del", n.strike)];
    if (n.mark) return [wrap("mark", n.mark)];
    if (n.tag !== undefined) return [el("button", { className: "tag", textContent: tagName(n.tag), onclick: (e) => { e.stopPropagation(); hooks.onTag(n.tag); } })];
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
    if (b.type === "rule") return el("hr");
    if (b.type === "code") return el("pre", {}, el("code", { textContent: b.text }));
    if (b.type === "quote") return el("blockquote", {}, ...mdNodes(b.blocks, hooks, base));
    if (b.type === "table") {
      const cell = (tag, content, c) => el(tag, { style: b.align[c] ? `text-align:${b.align[c]}` : "" }, ...mdSpans(content, hooks));
      return el("div", { className: "mdtable" }, el("table", {}, el("thead", {}, el("tr", {}, ...b.head.map((h, c) => cell("th", h, c)))), el("tbody", {}, ...b.rows.map((r) => el("tr", {}, ...r.map((x, c) => cell("td", x, c)))))));
    }
    const items = b.items.map((it) => {
      const box = it.task === null ? null : el("input", { type: "checkbox", checked: it.task, disabled: !hooks.onTask, ariaLabel: t("Done"), onchange: () => hooks.onTask(it.n) });
      return el("li", { className: it.task === null ? "" : "task" + (it.task ? " done" : "") }, box, el("span", {}, ...mdSpans(it.inline, hooks)), ...mdNodes(it.blocks, hooks, base));
    });
    return el(b.ordered ? "ol" : "ul", {}, ...items);
  });
}

function markdownDom(text, hooks, base, firstLine) {
  return mdNodes(mdParse(text, firstLine), hooks, base);
}

function boardParts(text) {
  const lines = String(text).replace(/\r\n?/g, "\n").split("\n");
  const out = { title: "", lead: { text: "", line: 0 }, sections: [] };
  let fenced = null;
  let cur = out.lead;
  let body = [];
  const flush = () => { cur.text = body.join("\n"); body = []; };
  lines.forEach((line, n) => {
    const fence = line.match(MD_FENCE);
    if (fence) fenced = fenced && line.trim().startsWith(fenced) ? null : fenced || fence[1][0];
    const h1 = !fenced && line.match(/^#\s+(.*)$/);
    const h2 = !fenced && line.match(/^##\s+(.*)$/);
    if (h1 && !out.title && !out.sections.length) { out.title = h1[1]; body.push(""); return; }
    if (h2) { flush(); cur = { title: h2[1], text: "", line: n + 1 }; out.sections.push(cur); return; }
    body.push(line);
  });
  flush();
  return out;
}

function boardDom(text, hooks) {
  const b = boardParts(text);
  const card = el("div", { className: "bcard" });
  if (b.title) card.append(el("h2", { className: "title" }, ...mdSpans(mdInline(b.title), hooks)));
  if (b.lead.text.trim()) card.append(el("div", { className: "lead md" }, ...markdownDom(b.lead.text, hooks, BOARD_HEADING_BASE, b.lead.line)));
  if (b.sections.length) card.append(el("div", { className: "bgrid" }, ...b.sections.map((x) => el("section", { className: "bsec md" + (x.text.includes("ezh://") ? " wide" : "") }, el("h3", {}, ...mdSpans(mdInline(x.title), hooks)), ...markdownDom(x.text, hooks, BOARD_HEADING_BASE, x.line)))));
  return card;
}
