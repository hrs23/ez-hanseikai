// Member and room profiles, and the checks of a member's PC and connection.
const CHECK_FRESH_MS = 60000;

let profileFor = null;
let speed = null;
const checks = {};

function fetchChecks(id, again) {
  const c = checks[id];
  if (c && (c.busy || (!again && Date.now() - c.at < CHECK_FRESH_MS))) return;
  checks[id] = { busy: true, at: Date.now(), data: c ? c.data : null };
  invoke("check", id === st.id ? {} : { id })
    .then((data) => data, () => null)
    .then((data) => {
      checks[id] = { busy: false, at: Date.now(), data, failed: !data };
      delete sigs.memberprof;
      delete sigs.diag;
      renderMemberProfile();
      renderDiagnostics();
    });
}
function checkLine(l) {
  return el("div", { className: "ok-line lv-" + l.level }, icon(LEVEL_ICONS[l.level] || "info", "sm"), el("span", { textContent: t(l.text, l.args) }));
}
function checkBox(lines) {
  return el("div", { className: "okbox" }, ...lines.map(checkLine));
}
// The two groups of one member's checks, under headings, or a line that says why there are none.
function checkGroups(id, action) {
  const c = checks[id];
  const note = (iconName, text, cls = "") => el("div", { className: "okbox" }, el("div", { className: "ok-line " + cls }, icon(iconName, "sm" + (iconName === "spinner" ? " spin" : "")), el("span", { textContent: text })));
  if (!c || (c.busy && !c.data)) return [note("spinner", t("Checking…"))];
  if (!c.data) return [note("alert", t("No answer. Their app may be older."), "lv-warn")];
  return [el("div", { className: "row" }, el("div", { className: "h3 grow", textContent: t("PC") }), action || ""), checkBox(c.data.pc), el("div", { className: "h3", textContent: t("Connection") }), checkBox(c.data.net)];
}
const checkSig = (id) => checks[id] && [checks[id].busy, checks[id].data];

function openProfile(id) {
  if (!st || !people().includes(id)) return;
  if (profileFor !== id) speed = null;
  profileFor = id;
  delete sigs.memberprof;
  if ($("roomprofdlg").open) $("roomprofdlg").close();
  if (!$("profiledlg").open) $("profiledlg").showModal();
  renderMemberProfile();
}
const speedLine = (r) => [t("Up {n} Mbit/s", { n: r.up_mbps.toFixed(1) }), t("Down {n} Mbit/s", { n: r.down_mbps.toFixed(1) }), `${r.rtt_ms} ms`, r.path === "relay" ? t("Relay") : t("Direct")].join(" · ");
function measureSpeed(id) {
  speed = { id, line: null };
  delete sigs.memberprof;
  renderMemberProfile();
  invoke("measure_speed", { id }).then(speedLine, () => t("No answer")).then((line) => { if (speed && speed.id === id) speed.line = line; delete sigs.memberprof; renderMemberProfile(); });
}
function renderMemberProfile() {
  const box = $("profiledlg");
  if (!box.open) return;
  const id = profileFor;
  if (!inRoom() || !people().includes(id)) { box.close(); return; }
  const me = id === st.id;
  const p = me ? null : peerOf(id);
  const online = me || !!(p && p.connected);
  if (online) fetchChecks(id);
  const state = memberOf(id).status;
  const receivingFrom = st.sets.filter((s) => !s.live).flatMap((s) => s.players.filter((x) => x.owner === id && x.total && x.have < x.total).map((x) => ({ s, x })));
  const sig = [id, state, p && [p.connected, p.path, p.relay, p.rtt_ms, p.sent, p.received, p.offset_us, p.version], speed, checkSig(id), receivingFrom.map((r) => [r.s.id, r.x.have]), st.names, look(id), lang];
  if (!changed("memberprof", sig)) return;
  const kv = [];
  const add = (k, v) => kv.push(el("dt", { textContent: k }), el("dd", { textContent: v }));
  if (p && p.connected) {
    add(t("Path"), p.path === "relay" ? [t("Via relay server"), p.relay].filter(Boolean).join(" · ") : t("Direct"));
    add(t("Round trip"), `${p.rtt_ms} ms`);
    if (p.sent != null) add(t("Sent"), bytes(p.sent));
    if (p.received != null) add(t("Received"), bytes(p.received));
    if (p.offset_us != null) add(t("Clock offset"), `${p.offset_us >= 0 ? "+" : "−"}${(Math.abs(p.offset_us) / 1e6).toFixed(2)} s`);
  }
  add(t("Version"), (me ? st.version : p && p.version) || "—");
  const measuring = !!speed && speed.id === id && !speed.line;
  if (speed && speed.id === id && speed.line) add(t("Speed"), speed.line);
  box.replaceChildren(
    el("div", { className: "pband", style: `--mc:${colorOf(colorIndex(id))}` }, iconButton("x", t("Close"), () => box.close(), "icon sm close")),
    el("div", { className: "phead" },
      avatar(id, "xxl", online ? "on" : null, false, true),
      me ? el("button", { className: "sm", onclick: () => { box.close(); openSettings("general"); } }, icon("edit", "sm"), t("Edit profile")) : ""),
    el("div", { className: "pname" },
      el("h2", { textContent: nameOf(id) }),
      el("div", { className: "row" }, idText(id, "small"), iconButton("copy", t("Copy ID"), () => navigator.clipboard.writeText(id).then(() => flash(t("ID copied"))).catch(fail))),
      el("div", { className: "state " + state.kind, title: state.title || "" }, el("i", { className: "sdot" }), state.text)),
    el("div", { className: "pbody" },
      el("dl", { className: "kv" }, ...kv),
      receivingFrom.length ? el("div", { className: "h3", textContent: t("Receiving") }) : "",
      ...receivingFrom.map(({ s, x }) => el("div", { className: "recvrow" }, el("div", { className: "row" }, el("span", { textContent: sessionName(s.id) }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: `${Math.floor((x.have / x.total) * 100)}%` })), el("div", { className: "bar" }, el("i", { style: `width:${(x.have / x.total) * 100}%` })))),
      ...(online ? checkGroups(id, iconButton("refresh", t("Check again"), () => fetchChecks(id, true))) : [])),
    el("div", { className: "row foot" },
      me ? "" : el("button", { className: "sm", disabled: measuring || !online, onclick: () => measureSpeed(id) }, icon("activity", "sm"), measuring ? t("Measuring…") : t("Measure speed")),
      el("span", { className: "sp" }),
      me ? "" : el("button", { className: "danger sm", onclick: () => removeMember(id), textContent: t("Remove from room…") })),
  );
}

function openRoomProfile() {
  if (!inRoom()) return;
  delete sigs.roomprof;
  if (!$("roomprofdlg").open) $("roomprofdlg").showModal();
  renderRoomProfile();
}
function renderRoomProfile() {
  const box = $("roomprofdlg");
  if (!box.open) return;
  if (!inRoom()) { box.close(); return; }
  const ids = people();
  const states = ids.map((id) => memberOf(id).status);
  if (!changed("roomprof", [st.room.name, stamp(st.room.icon), ids, states, st.invites, st.names, looks(), lang])) return;
  const online = states.filter((s) => s.kind !== "off").length;
  const rows = ids.map((id, i) => {
    const row = el("button", { className: "memrow" + (states[i].kind === "off" ? " dim" : "") }, avatar(id, "lg", states[i].kind === "off" ? null : "on", false, true), el("b", { textContent: nameOf(id) }), id === st.id ? el("span", { className: "pill gray", textContent: t("You") }) : "", el("span", { className: "sp" }), el("span", { className: "state " + states[i].kind }, el("i", { className: "sdot" }), states[i].text));
    row.onclick = () => openProfile(id);
    return row;
  });
  box.replaceChildren(
    el("div", { className: "pband room" }, iconButton("x", t("Close"), () => box.close(), "icon sm close")),
    el("div", { className: "phead" }, roomIcon(st.room, "xxl"), el("button", { className: "sm", onclick: () => { box.close(); openRoomSettings(); } }, icon("edit", "sm"), t("Edit"))),
    el("div", { className: "pname" }, el("h2", { textContent: st.room.name }), el("div", { className: "muted", textContent: t("{n} members · {m} online", { n: ids.length, m: online }) })),
    el("div", { className: "pbody" }, el("div", { className: "plist" }, ...rows)),
    el("div", { className: "row foot" }, el("span", { className: "sp" }), el("button", { className: "sm primary", onclick: copyInvite }, icon("user-plus", "sm"), t("Copy invite code"))),
  );
}
