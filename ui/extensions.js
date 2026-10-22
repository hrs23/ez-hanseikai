let openExtensions = new Set(stored("ext-open", []));

function downloadStats(d) {
  const n = d.total ? Math.min(100, Math.floor((d.done / d.total) * 100)) : 0;
  return { n, text: progressText({ n, speed: d.speed, left: d.left_s }) };
}
function extensionLine(p, name) {
  if (!p.on) return "";
  if (!p.runner) return t("No runner");
  if (p.failed) return t("{n} failed · Retry", { n: p.failed });
  if (p.left && p.waiting_for) return t("Waiting for {name}'s PC", { name: name(p.waiting_for) });
  return p.left ? t("{n} left · {name}'s PC", { n: p.left, name: name(p.runner) }) : "";
}
const analyzing = (a) => (typeof a.progress === "number" ? t("Analyzing {n}%", { n: Math.floor(a.progress * 100) }) : t("Analyzing"));
function analysisReason(a, name) {
  if (a.state === "failed") return a.reason === "missing" ? t("File missing") : t("Failed");
  if (a.state === "running") return analyzing(a);
  if (a.reason === "receiving") return t("Receiving");
  if (a.reason === "no_runner") return t("No runner");
  if (a.reason === "recording") return t("Waiting for the recording to end");
  if (a.reason === "ffmpeg") return t("Waiting for the video tools");
  return a.waiting_for ? t("Waiting for {name}'s PC", { name: name(a.waiting_for) }) : t("Waiting");
}
const runningMark = (s) => { const a = (s.analysis || []).find((x) => x.state === "running"); return a ? analyzing(a) : null; };
function analysisState(a, runner, name) {
  if (a.state === "done") return t("Done");
  if (a.state === "idle") return t("Not analyzed");
  const who = a.state === "running" && runner ? t("{name}'s PC", { name: name(runner) }) : null;
  return [analysisReason(a, name), who].filter(Boolean).join(" · ");
}
function sampleOf(kind) {
  return kind === "chapters" ? el("div", { className: "sample chapters" }, el("i", { className: "fill" }), ...[24, 52, 81].map((x) => el("b", { style: `left:${x}%` }))) : null;
}
function askRoom(title, action, then) {
  ask(title, action).then((yes) => { if (yes) then(); else { delete sigs.roomtab; renderRoom(); } });
}
function savePlugin(p, on) {
  return invoke("set_room_plugin", { id: p.id, on }).then(pollPlugins).catch(fail);
}
function retryAnalysis(set, plugin) {
  invoke("retry_job", { room: set.room, set: set.id, plugin }).then(refresh).then(pollPlugins).catch(fail);
}
function extensionRunner(p) {
  const members = people();
  const change = (e) => askRoom(t("Change who runs {name}?", { name: t(p.name) }), t("Change"), () => invoke("set_plugin_runner", { id: p.id, runner: e.target.value }).then(pollPlugins).catch(fail));
  const none = members.includes(p.runner) ? [] : [el("option", { value: "", textContent: "—", selected: true, disabled: true })];
  return el("div", { className: "field" }, el("label", { textContent: t("Runner") }), el("select", { ariaLabel: t("Runner"), onchange: change }, ...none, ...members.map((id) => el("option", { value: id, textContent: nameOf(id), selected: p.runner === id }))));
}
function extensionRecordings(p) {
  const rows = st.sets.filter((s) => !s.live).flatMap((s) => (s.analysis || []).filter((a) => a.plugin === p.id && (a.state === "left" || a.state === "running" || a.state === "failed")).map((a) => ({ s, a })));
  if (!rows.length) return null;
  return el("div", { className: "psec" }, el("div", { className: "h3", textContent: t("Recordings") }), ...rows.map(({ s, a }) => el("div", { className: "wrow" }, el("b", { textContent: sessionName(s.id) }), el("span", { className: a.state === "failed" ? "warn" : "muted", textContent: analysisReason(a, nameOf) }), el("span", { className: "sp" }), a.state === "failed" ? el("button", { className: "sm", textContent: t("Retry"), onclick: () => retryAnalysis(s, p.id) }) : null)));
}
function extensionRow(p) {
  const more = p.on && !p.view;
  const open = more && openExtensions.has(p.id);
  const line = p.view ? "" : extensionLine(p, nameOf);
  const toggle = el("input", { type: "checkbox", className: "switch", role: "switch", checked: p.on, ariaLabel: t(p.name), onclick: (e) => e.stopPropagation(), onchange: (e) => { const on = e.target.checked; askRoom(t(on ? "Turn on {name} for the room?" : "Turn off {name} for the room?", { name: t(p.name) }), t(on ? "Turn on" : "Turn off"), () => savePlugin(p, on)); } });
  const head = el("div", { className: "phead" + (more ? " click" : ""), onclick: more ? () => { if (open) openExtensions.delete(p.id); else openExtensions.add(p.id); store("ext-open", [...openExtensions]); delete sigs.roomtab; renderRoom(); } : null },
    el("span", { className: "pic" }, icon("puzzle")), withClass(stack(t(p.name), t(p.summary), true), "pt"), sampleOf(p.sample), el("span", { className: "sp" }),
    line ? el("span", { className: p.on && (!p.runner || p.failed) ? "warn" : "muted", textContent: line }) : null, toggle, more ? icon(open ? "chevron-up" : "chevron-down", "sm") : null);
  const body = open ? el("div", { className: "pbody" }, el("div", { className: "psec" }, extensionRunner(p)), extensionRecordings(p)) : null;
  return el("div", { className: "plug" + (open ? " open" : "") }, head, body);
}
function renderExtensions() {
  if (!changed("roomtab", ["extensions", plugins, [...openExtensions], st.sets.map((s) => [s.id, s.analysis]), st.names, lang])) return;
  $("roomtab").replaceChildren(el("div", { className: "plugins" }, ...plugins.map(extensionRow)));
}
function pollPlugins() {
  if (!st || !inRoom() || document.hidden) return;
  const gen = ++pluginsGen;
  invoke("room_plugins").then((list) => {
    if (gen !== pluginsGen) return;
    plugins = list;
    renderRoom();
    renderSessions();
  }).catch(warn);
}
