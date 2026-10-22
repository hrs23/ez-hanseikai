const VOICE_QUIET_S = 60;
const PICTURE_LAG_S = 10;

function memberStatus(m, now) {
  if (!m.connected) return m.seen_us ? { kind: "off", text: t("Offline · last seen {time}", { time: seenLabel(m.seen_us, now) }), title: fullWhen(new Date(m.seen_us / 1000)) } : { kind: "off", text: t("Offline") };
  if (!m.sharing) return { kind: "idle", text: t("Not sharing") };
  return m.relayed ? { kind: "idle", text: t("No picture via relay") } : { kind: "standby", text: t("Standby") };
}

const sendsPicture = (p) => !!p && p.connected && p.path !== "relay";

const seenLabel = (us, now) => formatWhen(new Date(us / 1000), now, language);

const noScreen = (p) => (p.parts || []).length > 0 && p.parts.every((part) => !part.v);

function playerLine(set, p, ctx) {
  if (p.recording || !p.end_us || p.verified_by.includes(ctx.me)) return null;
  const name = ctx.name(p.owner);
  const holders = p.verified_by.filter((id) => id !== ctx.me);
  if (!holders.some(ctx.online)) return { kind: "away", text: t("{name}'s video hasn't arrived", { name }) };
  if (set.wanted || p.have > 0) {
    return { kind: "receiving", text: t("Receiving {name}'s video", { name }) };
  }
  return { kind: "missing", text: t("{name}'s video missing", { name }), action: t("Download") };
}

function viewLine(set, ctx, key) {
  if (set.live) return null;
  return set.players.filter((p) => !key || p.owner === key).map((p) => playerLine(set, p, ctx)).find(Boolean) || null;
}

const ageOf = (now, ...ends) => {
  const at = ends.find((e) => e !== null && e !== undefined);
  return at === undefined ? 0 : (now - at) / 1e6;
};

function tileProblem(m) {
  if (!m.recording) return null;
  if (ageOf(m.now, m.voiceEnd, m.start) > VOICE_QUIET_S) return t("No voice for 1 min");
  const lag = ageOf(m.now, m.pictureEnd, m.start);
  return lag > PICTURE_LAG_S ? t("{n} s behind", { n: Math.round(lag) }) : null;
}
