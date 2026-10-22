const apiUrl = (address) => `http://${address}`;

function apiReferenceBlocks(lines) {
  return lines.map((text) => {
    try {
      const value = JSON.parse(text);
      if (value && typeof value === "object") return { code: true, text: JSON.stringify(value, null, 2) };
    } catch {}
    return { code: false, text };
  });
}

function aiPurposes() {
  return [
    { id: "notes", icon: "note", label: t("Write notes from the voices"), task: t("Transcribe the voices and read the chapters and events of the recording, then add short tagged notes (#callout, #mistake) at the key moments. Skip notes that already exist.") },
    { id: "plan", icon: "board", label: t("Summarize a recording into the game plan"), task: t("Read the recording and the game plan, then rewrite the game plan: a bold goal, then a few bullets for the next match.") },
    { id: "mistakes", icon: "flag", label: t("Find the mistakes in a match"), task: t("Go through the matches of the recording and add a #mistake note at each team mistake: what went wrong and what to do instead.") },
  ];
}

const aiTaskKey = (p) => `aiTask/${p.id}`;
const aiTask = (p) => stored(aiTaskKey(p), null) || p.task;

function aiFacts(info, room, recording) {
  return [
    t("API: {address}/llms.txt", { address: apiUrl(info.address) }),
    t("Token: {token}", { token: info.token }),
    t("Room: {id} ({name})", { id: room.id, name: room.name }),
    recording ? t("Recording: {id}", { id: recording }) : t("Recording: the newest one"),
    t("Keep the token out of notes."),
  ].join("\n");
}

const aiPrompt = (purpose, info, room, recording) => `${purpose.task}\n\n${aiFacts(info, room, recording)}`;

const AI_MASK = "••••••••";
const aiPreview = (text, token) => (token ? text.replaceAll(token, AI_MASK) : text);

function aiMenuItems(info, room, recording, actions) {
  if (!info.on) return [{ label: t("Turn on the local API"), icon: "terminal", run: actions.enable }];
  return aiPurposes().map((p) => ({ label: p.label, icon: p.icon, run: () => actions.show(p, aiFacts(info, room, recording), info.token) }));
}

async function openAiMenu(anchor, recording) {
  const info = await invoke("api_info").catch(fail);
  if (!info || !st.room) return;
  const actions = {
    enable: () => run("set_api", { on: true }, () => refresh().then(() => openAiMenu(anchor, recording))),
    show: showPrompt,
  };
  openMenu(aiMenuItems(info, st.room, recording, actions), anchor);
}

let promptPurpose = null;
let promptFacts = "";
let promptToken = "";
const aiTaskOf = (text) => text.split("\n\n")[0].trim();
const aiCopyText = (text, token) => text.replaceAll(AI_MASK, token);
const promptBody = (task) => `${task}\n\n${aiPreview(promptFacts, promptToken)}`;
function promptReset() { $("promptreset").disabled = aiTaskOf($("prompttext").value) === promptPurpose.task; }
function showPrompt(purpose, facts, token) {
  promptPurpose = purpose;
  promptFacts = facts;
  promptToken = token;
  $("prompttitle").textContent = purpose.label;
  $("prompttext").value = promptBody(aiTask(purpose));
  promptReset();
  $("promptdlg").showModal();
  $("promptcopy").focus();
}
function editPrompt(reset) {
  if (reset) $("prompttext").value = promptBody(promptPurpose.task);
  const task = aiTaskOf($("prompttext").value);
  store(aiTaskKey(promptPurpose), task && task !== promptPurpose.task ? task : null);
  promptReset();
}

const aiDebrief = () => ({ task: t("This recording is a debrief of another one (of is that recording, watch is what was on screen). Add short tagged notes (#callout, #mistake) to that recording at the moments discussed. Skip notes that already exist.") });

async function summarizeDebrief(id) {
  const info = await invoke("api_info").catch(fail);
  if (!info || !st.room) return;
  if (!info.on) { flash(t("Turn on the local API")); return; }
  navigator.clipboard.writeText(aiPrompt(aiDebrief(), info, st.room, id)).then(() => flash(t("Prompt copied"))).catch(fail);
}

const newestRecording = () => { const s = st.sets.find((x) => !x.live); return s ? s.id : null; };
