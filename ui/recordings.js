const THUMB_FACES = 3;

const thumbSrc = (set) => convertFileSrc(`thumb/${set.room}/${set.id}`, "ezh");

function recordingRows() {
  return st.sets.filter((s) => !s.live && !s.of).map((s) => ({ id: s.id, s })).sort((a, b) => (a.id < b.id ? 1 : -1));
}

const debriefsOf = (id) => st.sets.filter((s) => s.of === id && !s.live);

function importedPill(s) {
  const p = pill("gray", "download", t("Imported"));
  p.title = t("Imported by {name}", { name: s.imported_by });
  return p;
}

const noPicture = (s) => s.players.length > 0 && s.players.every((p) => (p.parts || []).every((part) => !part.v));

function thumbBox(set, faces, size, max, live) {
  const box = el("div", { className: "thumb" + (live ? " live" : "") });
  if (set && set.thumb) box.append(el("img", { src: thumbSrc(set), alt: "", draggable: false }));
  else box.append(icon(live ? "rec" : "film"));
  if (faces.length) box.append(el("span", { className: "thumbfaces" }, avatars(faces, size, max)));
  return box;
}

function rowSignature(r) {
  const s = r.s;
  return [r.id, setLength(s), sessionState(s), s.imported_by, s.title, s.thumb, matchCount(s), noteCount(s), s.players.map((p) => p.owner), onThisPc(s), sizeOf(s), s.shared, s.analysis, debriefsOf(s.id).length, st.party.watching[s.id]];
}

function whereLine(s) {
  const here = onThisPc(s);
  return el("div", { className: "l2 where" }, el("span", { textContent: t(here ? "On this PC" : "Not on this PC") }), el("span", { textContent: bytes(sizeOf(s)) }));
}

const KEEP_NEWEST = 3;

function onThisPc(s) {
  return s.progress.have > 0 || !!s.wanted;
}

function sizeOf(s) {
  return s.progress.have > 0 ? s.progress.have : s.progress.total;
}

function canDelete(s) {
  return !s.live && (onThisPc(s) || !s.shared);
}

function deleteForm(s) {
  return s.shared ? "Others still have it" : "This deletes it completely";
}

function deletePreview(sets) {
  return sets.map((s) => ({
    id: s.id,
    bytes: s.progress.have,
    form: deleteForm(s),
    players: s.players.filter((p) => p.have > 0).map((p) => ({ owner: p.owner, bytes: p.have })),
    debriefs: s.debriefs,
  }));
}

function oldSessions(sets, keep = KEEP_NEWEST) {
  return sets.filter((s) => !s.live && s.progress.have > 0).slice(keep).map((s) => s.id);
}
