let openPlugin;
let downloadTimer = 0;

const capital = (c) => c.charAt(0).toUpperCase() + c.slice(1);
function choiceText(key, c) {
  if (key === "language" && c !== "auto") return capital(new Intl.DisplayNames([language], { type: "language" }).of(c) || c);
  return key === "model" ? c : t(capital(c));
}
function pluginProgress(jobs, id) {
  const mine = jobs.filter((j) => j.plugin === id);
  return { done: mine.filter((j) => j.state === "done").length, total: mine.length };
}
function isBusy(jobs, id, who) {
  return jobs.some((j) => j.plugin === id && j.state === "claimed" && j.by === who);
}
function workerState(w, busy) {
  if (!w.on) return "off";
  if (w.ready === false) return "missing";
  return busy ? "busy" : "ready";
}
function downloadStats(d) {
  const n = d.total ? Math.min(100, Math.floor((d.done / d.total) * 100)) : 0;
  return { n, text: [`${n}%`, d.speed ? `${bytes(d.speed)}/s` : null, d.left_s !== null && d.left_s !== undefined ? leftText(d.left_s) : null].filter(Boolean).join(" · ") };
}
function sampleOf(kind) {
  if (kind === "chapters") return el("div", { className: "sample chapters" }, el("i", { className: "fill" }), ...[24, 52, 81].map((x) => el("b", { style: `left:${x}%` })));
  return el("div", { className: "sample subs" }, el("span", { textContent: t("Nice shot!") }));
}
function pluginHead(p, right, onclick) {
  return el("div", { className: "phead" + (onclick ? " click" : ""), onclick }, el("span", { className: "pic" }, icon("puzzle")), el("div", { className: "pt" }, el("b", { textContent: t(p.name) }), el("span", { textContent: t(p.summary) })), sampleOf(p.sample), el("span", { className: "sp" }), ...right);
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
function pluginSettings(p) {
  const settings = { ...p.settings };
  const fields = p.options.map((o) => el("div", { className: "field" }, el("label", { textContent: t(capital(o.key)) }), el("select", { ariaLabel: t(capital(o.key)), onchange: (e) => { settings[o.key] = e.target.value; askRoom(t("Change {name} for the room?", { name: t(p.name) }), t("Change"), () => savePlugin(p, true, settings)); } }, ...o.choices.map((c) => el("option", { value: c, textContent: choiceText(o.key, c), selected: settings[o.key] === c })))));
  return fields.length ? el("div", { className: "psec" }, el("div", { className: "h3" }, t("Settings"), pill("accent", "users", t("Everyone"))), ...fields) : null;
}
function pluginWorkers(p) {
  const jobs = jobInfo ? jobInfo.jobs : [];
  const label = { off: t("Off"), missing: t("Needs download"), busy: t("Processing"), ready: t("Ready") };
  const rows = p.workers.map((w) => {
    const state = workerState(w, isBusy(jobs, p.id, w.id));
    const mine = w.id === st.id;
    const tail = mine ? el("input", { type: "checkbox", className: "switch", role: "switch", checked: w.on, ariaLabel: t("Process on this PC"), onchange: (e) => invoke("set_process_plugin", { id: p.id, on: e.target.checked }).then(pollPlugins).catch(fail) }) : null;
    return el("div", { className: "wrow" }, avatar(w.id, "sm"), el("b", { textContent: mine ? t("{name} (you)", { name: nameOf(w.id) }) : nameOf(w.id) }), mine ? el("span", { className: "muted", textContent: t("Process on this PC") }) : null, el("span", { className: "sp" }), el("span", { className: "pill " + { off: "gray", missing: "warn", busy: "accent", ready: "ok" }[state], textContent: label[state] }), tail);
  });
  return el("div", { className: "psec" }, el("div", { className: "h3", textContent: t("Processing") }), ...rows);
}
function pluginProgressBox(p) {
  const { done, total } = pluginProgress(jobInfo ? jobInfo.jobs : [], p.id);
  if (!total) return null;
  return el("div", { className: "psec" }, el("div", { className: "h3", textContent: t("Progress") }), el("div", { className: "row" }, el("span", { textContent: t("{done} of {total} recordings", { done, total }) }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: `${Math.floor((done / total) * 100)}%` })), el("div", { className: "bar" }, el("i", { style: `width:${(done / total) * 100}%` })));
}
function pluginHere(p) {
  let body;
  if (p.download) {
    const d = downloadStats(p.download);
    body = el("div", { className: "dl" }, el("div", { className: "row" }, icon("spinner", "sm spin"), el("span", { textContent: t("Downloading to this PC…") }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: d.text })), el("div", { className: "bar" }, el("i", { style: `width:${d.n}%` })));
  } else if (p.ready) body = el("div", { className: "row" }, icon("ok", "sm ok"), el("span", { className: "ok", textContent: t("Ready on this PC") }));
  else body = el("div", { className: "row" }, icon("download", "sm"), el("span", { textContent: t("Not on this PC") }), el("span", { className: "sp" }), el("button", { className: "sm", textContent: t("Download {size}", { size: bytes(p.download_bytes) }), onclick: () => downloadPlugin(p) }));
  return el("div", { className: "psec" }, el("div", { className: "h3", textContent: t("This PC") }), body);
}
function enabledCard(p) {
  const open = openPlugin === p.id;
  const { done, total } = pluginProgress(jobInfo ? jobInfo.jobs : [], p.id);
  const right = [total ? el("span", { className: "pill " + (done === total ? "ok" : "gray"), textContent: `${done}/${total}` }) : null, icon(open ? "chevron-up" : "chevron-down", "sm")];
  const body = open ? el("div", { className: "pbody" }, pluginSettings(p), pluginWorkers(p), pluginProgressBox(p), pluginHere(p), el("div", { className: "psec" }, el("button", { className: "ghost", textContent: t("Turn off for the room…"), onclick: () => askRoom(t("Turn off {name} for the room?", { name: t(p.name) }), t("Turn off"), () => savePlugin(p, false, p.settings)) }))) : null;
  return el("div", { className: "plug" + (open ? " open" : "") }, pluginHead(p, right, () => { openPlugin = open ? null : p.id; delete sigs.roomtab; renderRoom(); }), body);
}
function availableCard(p) {
  return el("div", { className: "plug" }, pluginHead(p, [el("button", { className: "sm", textContent: t("Turn on"), onclick: () => askRoom(t("Turn on {name} for the room?", { name: t(p.name) }), t("Turn on"), () => savePlugin(p, true, p.settings)) })], null));
}
function renderPlugins() {
  if (openPlugin === undefined) { const first = plugins.find((p) => p.on); if (first) openPlugin = first.id; }
  if (!changed("roomtab", ["plugins", plugins, openPlugin, jobInfo && jobInfo.jobs.map((j) => [j.plugin, j.state, j.by]), st.names, lang])) return;
  const enabled = plugins.filter((p) => p.on);
  const available = plugins.filter((p) => !p.on);
  $("roomtab").replaceChildren(
    el("div", { className: "h3 first", textContent: t("Enabled in this room") }),
    el("div", { className: "plugins" }, ...(enabled.length ? enabled.map(enabledCard) : [el("div", { className: "empty", textContent: t("No plugins are on") })])),
    available.length ? el("div", { className: "h3", textContent: t("Available plugins") }) : null,
    available.length ? el("div", { className: "plugins" }, ...available.map(availableCard)) : null,
  );
}
function watchDownloads() {
  clearTimeout(downloadTimer);
  if (plugins.some((p) => p.download)) downloadTimer = setTimeout(pollPlugins, DOWNLOAD_POLL_MS);
}
