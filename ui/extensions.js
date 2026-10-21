let downloadTimer = 0;
let openExtensions = new Set(stored("ext-open", []));

const capital = (c) => c.charAt(0).toUpperCase() + c.slice(1);
function choiceText(key, c) {
  if (key === "language" && c !== "auto") return capital(new Intl.DisplayNames([language], { type: "language" }).of(c) || c);
  return key === "model" ? c : t(capital(c));
}
function downloadStats(d) {
  const n = d.total ? Math.min(100, Math.floor((d.done / d.total) * 100)) : 0;
  return { n, text: [`${n}%`, d.speed ? `${bytes(d.speed)}/s` : null, d.left_s !== null && d.left_s !== undefined ? leftText(d.left_s) : null].filter(Boolean).join(" · ") };
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
  return a.waiting_for ? t("Waiting for {name}'s PC", { name: name(a.waiting_for) }) : t("Waiting");
}
function recordingAnalysis(s) {
  const mine = (s.analysis || []).filter((a) => a.state !== "done");
  const failed = mine.find((a) => a.state === "failed");
  const going = mine.find((a) => a.state === "running");
  if (failed) return { kind: "line", icon: "alert", cls: "warn", text: t("Analysis failed") };
  if (going) return { kind: "tail", icon: "cpu", text: analyzing(going) };
  return null;
}
function sampleOf(kind) {
  if (kind === "chapters") return el("div", { className: "sample chapters" }, el("i", { className: "fill" }), ...[24, 52, 81].map((x) => el("b", { style: `left:${x}%` })));
  return el("div", { className: "sample subs" }, el("span", { textContent: t("Nice shot!") }));
}
function askRoom(title, action, then) {
  ask(title, action, false, t("Applies to everyone in the room.")).then((yes) => { if (yes) then(); else { delete sigs.roomtab; renderRoom(); } });
}
function savePlugin(p, on, settings) {
  return invoke("set_room_plugin", { id: p.id, on, settings }).then(pollPlugins).catch(fail);
}
function downloadPlugin(p) {
  invoke("prepare_plugin", { id: p.id }).catch(fail).finally(pollPlugins);
  pollPlugins();
}
function retryAnalysis(set, plugin) {
  invoke("retry_job", { room: set.room, set: set.id, plugin }).then(refresh).then(pollPlugins).catch(fail);
}
function extensionSettings(p) {
  const settings = { ...p.settings };
  return p.options.map((o) => el("div", { className: "field" }, el("label", { textContent: t(capital(o.key)) }), el("select", { ariaLabel: t(capital(o.key)), onchange: (e) => { settings[o.key] = e.target.value; askRoom(t("Change {name} for the room?", { name: t(p.name) }), t("Change"), () => savePlugin(p, true, settings)); } }, ...o.choices.map((c) => el("option", { value: c, textContent: choiceText(o.key, c), selected: settings[o.key] === c })))));
}
function extensionRunner(p) {
  const members = people();
  const change = (e) => askRoom(t("Change who runs {name}?", { name: t(p.name) }), t("Change"), () => invoke("set_plugin_runner", { id: p.id, runner: e.target.value }).then(pollPlugins).catch(fail));
  const none = members.includes(p.runner) ? [] : [el("option", { value: "", textContent: "—", selected: true, disabled: true })];
  return el("div", { className: "field" }, el("label", { textContent: t("Runner") }), el("select", { ariaLabel: t("Runner"), onchange: change }, ...none, ...members.map((id) => el("option", { value: id, textContent: nameOf(id), selected: p.runner === id }))));
}
function extensionHere(p) {
  if (p.download) {
    const d = downloadStats(p.download);
    return el("div", { className: "dl" }, el("div", { className: "row" }, icon("spinner", "sm spin"), el("span", { textContent: t("Downloading to this PC…") }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: d.text })), el("div", { className: "bar" }, el("i", { style: `width:${d.n}%` })));
  }
  if (p.ready) return el("div", { className: "row" }, icon("ok", "sm ok"), el("span", { className: "ok", textContent: t("Ready on this PC") }));
  return el("div", { className: "row" }, icon("download", "sm"), el("span", { textContent: t("Not on this PC") }), el("span", { className: "sp" }), el("button", { className: "sm", textContent: t("Download {size}", { size: bytes(p.download_bytes) }), onclick: () => downloadPlugin(p) }));
}
function extensionRecordings(p) {
  const rows = st.sets.filter((s) => !s.live).flatMap((s) => (s.analysis || []).filter((a) => a.plugin === p.id && (a.state === "left" || a.state === "running" || a.state === "failed")).map((a) => ({ s, a })));
  if (!rows.length) return null;
  return el("div", { className: "psec" }, el("div", { className: "h3", textContent: t("Recordings") }), ...rows.map(({ s, a }) => el("div", { className: "wrow" }, el("b", { textContent: sessionName(s.id) }), el("span", { className: a.state === "failed" ? "warn" : "muted", textContent: analysisReason(a, nameOf) }), el("span", { className: "sp" }), a.state === "failed" ? el("button", { className: "sm", textContent: t("Retry"), onclick: () => retryAnalysis(s, p.id) }) : null)));
}
function extensionRow(p) {
  const open = openExtensions.has(p.id);
  const line = extensionLine(p, nameOf);
  const toggle = el("input", { type: "checkbox", className: "switch", role: "switch", checked: p.on, ariaLabel: t(p.name), onclick: (e) => e.stopPropagation(), onchange: (e) => { const on = e.target.checked; askRoom(t(on ? "Turn on {name} for the room?" : "Turn off {name} for the room?", { name: t(p.name) }), t(on ? "Turn on" : "Turn off"), () => savePlugin(p, on, p.settings)); } });
  const head = el("div", { className: "phead click", onclick: () => { if (open) openExtensions.delete(p.id); else openExtensions.add(p.id); store("ext-open", [...openExtensions]); delete sigs.roomtab; renderRoom(); } },
    el("span", { className: "pic" }, icon("puzzle")), el("div", { className: "pt" }, el("b", { textContent: t(p.name) }), el("span", { textContent: t(p.summary) })), sampleOf(p.sample), el("span", { className: "sp" }),
    line ? el("span", { className: p.on && (!p.runner || p.failed) ? "warn" : "muted", textContent: line }) : null, toggle, icon(open ? "chevron-up" : "chevron-down", "sm"));
  const body = open ? el("div", { className: "pbody" }, el("div", { className: "psec" }, ...extensionSettings(p), p.on ? extensionRunner(p) : null), el("div", { className: "psec" }, extensionHere(p)), p.on ? extensionRecordings(p) : null) : null;
  return el("div", { className: "plug" + (open ? " open" : "") }, head, body);
}
function renderExtensions() {
  if (!changed("roomtab", ["extensions", plugins, [...openExtensions], st.sets.map((s) => [s.id, s.analysis]), st.names, lang])) return;
  $("roomtab").replaceChildren(el("div", { className: "plugins" }, ...plugins.map(extensionRow)));
}
function watchDownloads() {
  clearTimeout(downloadTimer);
  if (plugins.some((p) => p.download)) downloadTimer = setTimeout(pollPlugins, DOWNLOAD_POLL_MS);
}
function pollPlugins() {
  if (!st || !inRoom() || document.hidden) return;
  const gen = ++pluginsGen;
  invoke("room_plugins").then((list) => {
    if (gen !== pluginsGen) return;
    plugins = list;
    watchDownloads();
    renderRoom();
    renderSessions();
  }).catch(warn);
}
