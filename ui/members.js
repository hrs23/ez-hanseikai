const RECEIVE_RECENT_US = 12 * 3600 * 1e6;

function receivingPercent(sets, id, now, me) {
  let have = 0;
  let total = 0;
  for (const s of sets) {
    if (s.live || !s.end_us || now - s.end_us > RECEIVE_RECENT_US) continue;
    if (!(s.lacking || []).some((l) => l.id === id)) continue;
    if (id === me) {
      have += s.progress.have;
      total += s.progress.total;
    } else {
      total += s.players.length;
      have += s.players.filter((p) => p.verified_by.includes(id)).length;
    }
  }
  return total ? Math.min(99, Math.floor((have / total) * 100)) : null;
}

function memberStatus(m, now) {
  if (!m.connected) {
    const seen = m.seen_us ? t("Offline · last seen {time}", { time: seenLabel(m.seen_us, now) }) : t("Offline");
    return { kind: "off", text: seen };
  }
  if (m.recording) return { kind: "rec", text: t("Recording") };
  if (m.receiving !== null && m.receiving !== undefined) return { kind: "recv", text: t("Receiving {n}%", { n: m.receiving }) };
  if (m.sharing) return { kind: "standby", text: t("Standby") };
  return { kind: "idle", text: t("Not sharing a screen") };
}

function seenLabel(us, now) {
  const at = new Date(us / 1000);
  const clock = `${String(at.getHours()).padStart(2, "0")}:${String(at.getMinutes()).padStart(2, "0")}`;
  return `${dayLabel(dayOf(at), now, language)} ${clock}`;
}
