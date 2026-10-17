const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const REFRESH_MS = 2000;
const mb = (bytes) => (bytes / 1e6).toFixed(1) + " MB";
const gb = (bytes) => (bytes / 1e9).toFixed(1) + " GB";
let st = null;

async function refresh() {
  st = await invoke("state");
  const lines = [`version ${st.version}${st.previous_version ? ` (previous run ${st.previous_version})` : ""}`, `id ${st.id}`, `recording codec ${st.codec}`];
  for (const p of st.peers) lines.push(`peer ${p.name ? `${p.name} (${p.short})` : p.short}: ${p.connected ? p.path + ", " + p.rtt_ms + " ms" : "not connected"}${p.version ? ", v" + p.version : ""}${p.offset_us != null ? ", clock offset " + (p.offset_us / 1000).toFixed(1) + " ms" : ""}`);
  $("info").textContent = lines.join("\n");
  $("network").textContent = await invoke("debug_text");
  const info = await invoke("data_info");
  $("datainfo").textContent = `${info.dir}\n${mb(info.used_bytes)} used, ${gb(info.free_bytes)} free`;
}
async function runCheck() {
  $("checks").textContent = "Checking...";
  const lines = await invoke("check", { audioProcess: st ? st.audio_process : "" });
  const bad = lines.filter((l) => l.level === "fail").length;
  const warn = lines.filter((l) => l.level === "warn").length;
  $("checksummary").textContent = bad ? `Check: ${bad} problem(s)` : warn ? `Check: ${warn} warning(s)` : "Check: OK";
  $("checks").replaceChildren(...lines.map((l) => Object.assign(document.createElement("div"), { className: l.level, textContent: l.text })));
}
const level = () => document.querySelector("input[name=level]:checked").value;
$("report").onclick = () => invoke("save_report", { includeAddresses: $("addr").checked, audioProcess: st ? st.audio_process : "" }).then((p) => { $("reportpath").textContent = p; }).catch((e) => { $("reportpath").textContent = String(e); });
$("runcheck").onclick = runCheck;
$("openfolder").onclick = () => invoke("open_data_folder");
$("openfolder2").onclick = () => invoke("open_data_folder");
$("resetopen").onclick = () => { $("resetpanel").hidden = false; };
$("resetcancel").onclick = () => { $("resetpanel").hidden = true; $("reseterr").textContent = ""; };
document.querySelectorAll("input[name=level]").forEach((r) => { r.onchange = () => { $("confirm").hidden = level() !== "all"; }; });
$("doreset").onclick = () => invoke("reset", { everything: level() === "all", confirm: $("confirm").value }).catch((e) => { $("reseterr").textContent = String(e); });
setInterval(() => refresh().catch(() => {}), REFRESH_MS);
refresh().then(runCheck);
