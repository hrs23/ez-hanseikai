const apiUrl = (address) => `http://${address}`;

function apiBadge(api) {
  if (!api.on) return null;
  return api.lan ? { lan: true, title: t("Other devices on your network can control the app") } : { lan: false, title: t("The local API is on") };
}

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
    { id: "notes", icon: "note", label: t("Write notes from the transcript"), task: t("Task: read the transcript, chapters and events of the recording. Then add a short note, with a tag such as #callout or #mistake, at each moment that matters: a decision, a good call or a mistake. Do not repeat notes that already exist.") },
    { id: "plan", icon: "board", label: t("Summarize a recording into the game plan"), task: t("Task: read the events, chapters and transcript of the recording and the current game plan. Then replace the game plan with a short, clear one: a bold first line with the goal and a few bullet points of what to do in the next match. Keep what the team already wrote that still applies.") },
    { id: "mistakes", icon: "flag", label: t("Find the mistakes in a match"), task: t("Task: go through the matches of the recording one by one. Read the transcript and events of each match, find the mistakes the team made, and add a note with the tag #mistake at the moment of each, saying what went wrong and what to do instead.") },
  ];
}

function aiPrompt(purpose, info, room, recording) {
  const address = apiUrl(info.address);
  return [
    t("Help my team review its game with GGI Hanseikai, using the app's local API."),
    t("First read {address}/llms.txt: it explains the API.", { address }),
    t("Address: {address}", { address }),
    t("Token (send it as: Authorization: Bearer <token>): {token}", { token: info.token }),
    t("Room: {id} ({name})", { id: room.id, name: room.name }),
    recording ? t("Recording: {id} (in a URL, write spaces as %20)", { id: recording }) : t("Recording: the newest one in the room (list the recordings to find it)"),
    "",
    purpose.task,
    t("Keep the token secret: never write it into a note or the game plan."),
  ].join("\n");
}

const AI_MASK = "••••••••";
const aiPreview = (text, token) => (token ? text.replaceAll(token, AI_MASK) : text);

function aiMenuItems(info, room, recording, actions) {
  if (!info.on) return [{ label: t("Turn on the local API"), icon: "terminal", run: actions.enable }];
  return aiPurposes().map((p) => ({ label: p.label, icon: p.icon, run: () => actions.show(p.label, aiPrompt(p, info, room, recording), info.token) }));
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

let promptFull = "";
function showPrompt(title, text, token) {
  promptFull = text;
  $("prompttitle").textContent = title;
  $("prompttext").value = aiPreview(text, token);
  $("promptdlg").showModal();
  $("promptcopy").focus();
}

const aiDebrief = () => ({ task: t("Task: this is a debrief the team recorded while watching another recording. Read its JSON: of is the recording it is about and watch is what was on screen and when. Read each member's voice at the voices endpoint, transcribe it, then add short notes with a tag such as #callout or #mistake to the recording it is about, at the moments that were discussed. Do not repeat notes that already exist.") });

async function summarizeDebrief(id) {
  const info = await invoke("api_info").catch(fail);
  if (!info || !st.room) return;
  if (!info.on) { flash(t("Turn on the local API")); return; }
  navigator.clipboard.writeText(aiPrompt(aiDebrief(), info, st.room, id)).then(() => flash(t("Prompt copied"))).catch(fail);
}

const newestRecording = () => { const s = st.sets.find((x) => !x.live); return s ? s.id : null; };
