const RECENT_SHOWN = 6;
const THUMB_FACES = 3;
const CARD_FACES = 4;

const thumbSrc = (set) => convertFileSrc(`thumb/${set.room}/${set.id}`, "ezh");

function recordingRows(withGone) {
  const sets = st.sets.filter((s) => !s.live).map((s) => ({ id: s.id, s }));
  const gone = withGone ? st.gone.map((g) => ({ id: g.id, g })) : [];
  return [...sets, ...gone].sort((a, b) => (a.id < b.id ? 1 : -1));
}

function recentRows(rows, keep) {
  const head = rows.slice(0, RECENT_SHOWN);
  const extra = rows.find((r) => r.id === keep && !head.includes(r));
  return extra ? [...head, extra] : head;
}

function recordingWords(text) {
  return text.toLowerCase().split(/\s+/).filter(Boolean);
}

function recordingText(row) {
  const parts = [sessionName(row.id), row.id];
  if (row.s) {
    parts.push(...row.s.players.map((p) => nameOf(p.owner)));
    if (isReview(row.s)) parts.push("debrief", t("Debrief"));
    if (row.s.imported_by) parts.push("imported", t("Imported"), row.s.imported_by);
  } else {
    parts.push(...row.g.players.map((p) => nameOf(p.owner)));
  }
  return parts.join(" ").toLowerCase();
}

function recordingMatches(row, words) {
  const text = recordingText(row);
  return words.every((w) => text.includes(w));
}

function importedPill(s) {
  const p = pill("gray", "download", t("Imported"));
  p.title = t("Imported by {name}", { name: s.imported_by });
  return p;
}

function marksOf(s) {
  return [isReview(s) ? pill("accent", "mic", t("Debrief")) : null, s.imported_by ? importedPill(s) : null].filter(Boolean);
}

function thumbBox(set, faces, size, max, live) {
  const box = el("div", { className: "thumb" + (live ? " live" : "") });
  if (set && set.thumb) box.append(el("img", { src: thumbSrc(set), alt: "", draggable: false }));
  else box.append(icon(live ? "rec" : set && isReview(set) ? "mic" : "film"));
  if (faces.length) box.append(el("span", { className: "thumbfaces" }, avatars(faces, size, max)));
  return box;
}

function detailText(s) {
  const matches = matchCount(s);
  const notes = noteCount(s);
  if (matches) return t(matches === 1 ? "{n} match" : "{n} matches", { n: matches });
  return notes ? t(notes === 1 ? "{n} note" : "{n} notes", { n: notes }) : "";
}

function rowSignature(r) {
  if (!r.s) return [r.id, r.g.end_us, r.g.players.map((p) => p.owner)];
  const s = r.s;
  return [r.id, setLength(s), sessionState(s), isReview(s), s.imported_by, s.title, s.thumb, matchCount(s), noteCount(s), s.players.map((p) => p.owner)];
}
