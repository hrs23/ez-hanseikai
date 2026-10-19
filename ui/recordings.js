const THUMB_FACES = 3;
const CARD_FACES = 4;

const thumbSrc = (set) => convertFileSrc(`thumb/${set.room}/${set.id}`, "ezh");

function recordingRows() {
  return st.sets.filter((s) => !s.live).map((s) => ({ id: s.id, s })).sort((a, b) => (a.id < b.id ? 1 : -1));
}

function recordingWords(text) {
  return text.toLowerCase().split(/\s+/).filter(Boolean);
}

function recordingText(row) {
  const parts = [sessionName(row.id), row.id, ...row.s.players.map((p) => nameOf(p.owner))];
  if (isReview(row.s)) parts.push("debrief", t("Debrief"));
  if (row.s.imported_by) parts.push("imported", t("Imported"), row.s.imported_by);
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
  const s = r.s;
  return [r.id, setLength(s), sessionState(s), isReview(s), s.imported_by, s.title, s.thumb, matchCount(s), noteCount(s), s.players.map((p) => p.owner), onThisPc(s), sizeOf(s), s.shared];
}

function whereLine(s) {
  const here = onThisPc(s);
  const parts = [el("span", { textContent: t(here ? "On this PC" : "Not on this PC") }), el("span", { textContent: bytes(sizeOf(s)) })];
  if (!here) parts.push(el("span", { className: "sp" }), el("button", { className: "sm", onclick: (e) => { e.stopPropagation(); run("download_set", { room: s.room, set: s.id }); } }, icon("download", "sm"), t("Download")));
  return el("div", { className: "l2 where" }, ...parts);
}
