const VOICE_QUIET_S = 60;
const PICTURE_LAG_S = 10;

function memberStatus(m, now) {
  if (!m.connected) return { kind: "off", text: m.seen_us ? t("Offline · last seen {time}", { time: seenLabel(m.seen_us, now) }) : t("Offline") };
  if (!m.sharing) return { kind: "idle", text: t("Not sharing") };
  return m.relayed ? { kind: "idle", text: t("No picture via relay") } : { kind: "standby", text: t("Standby") };
}

const sendsPicture = (p) => !!p && p.connected && p.path !== "relay";

function seenLabel(us, now) {
  const at = new Date(us / 1000);
  return `${dayLabel(dayOf(at), now, language)} ${at.toTimeString().slice(0, 5)}`;
}

const noScreen = (p) => (p.parts || []).length > 0 && p.parts.every((part) => !part.v);

function playerLine(set, p, ctx) {
  if (p.recording || !p.end_us) return null;
  const name = ctx.name(p.owner);
  if (p.verified_by.includes(ctx.me)) return noScreen(p) ? { kind: "nopicture", text: t("{name} didn't record a screen", { name }) } : null;
  const holders = p.verified_by.filter((id) => id !== ctx.me);
  if (!holders.some(ctx.online)) return { kind: "away", text: t("{name}'s video hasn't arrived", { name }) };
  if (set.wanted || p.have > 0) {
    return { kind: "receiving", text: t("Receiving {name}'s video", { name }) };
  }
  return { kind: "missing", text: t("{name}'s video missing", { name }), action: t("Download") };
}

function viewLine(set, ctx, key) {
  if (set.live) return null;
  const lines = set.players.filter((p) => !key || p.owner === key).map((p) => playerLine(set, p, ctx));
  return lines.find((line) => line && (key || line.kind !== "nopicture")) || null;
}

const ageOf = (now, ...ends) => {
  const at = ends.find((e) => e !== null && e !== undefined);
  return at === undefined ? 0 : (now - at) / 1e6;
};

function tileProblem(m) {
  if (!m.recording) return null;
  if (!m.sharing) return t("Not recording screen");
  if (ageOf(m.now, m.voiceEnd, m.start) > VOICE_QUIET_S) return t("No voice for 1 min");
  const lag = ageOf(m.now, m.pictureEnd, m.start);
  return lag > PICTURE_LAG_S ? t("{n} s behind", { n: Math.round(lag) }) : null;
}
