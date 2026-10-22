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

function thumbBox(set, live) {
  const box = el("div", { className: "thumb" + (live ? " live" : "") });
  if (set && set.thumb) box.append(el("img", { src: thumbSrc(set), alt: "", draggable: false }));
  else box.append(icon(live ? "rec" : "film"));
  return box;
}

function rowSignature(r) {
  const s = r.s;
  return [r.id, setLength(s), sessionState(s), runningMark(s), s.imported_by, s.title, s.thumb, matchCount(s), noteCount(s), s.players.map((p) => p.owner), onThisPc(s), sizeOf(s), s.shared, debriefsOf(s.id).length, st.party.watching[s.id]];
}

function downloadButton(s) {
  return onThisPc(s) ? null : el("button", { className: "sm", textContent: t("Download"), onclick: (e) => { e.stopPropagation(); run("download_set", { room: s.room, set: s.id }); } });
}

function whereLine(s) {
  return el("div", { className: "l2 where" }, el("span", { textContent: bytes(sizeOf(s)) }), downloadButton(s));
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

function fileCount(n) {
  return t(n === 1 ? "1 file" : "{n} files", { n });
}

const CLEANUP_GROUPS = { temp: "Temporary files", orphan: "Leftovers of deleted recordings", old: "Files of older formats" };

function itemCount(n) {
  return t(n === 1 ? "1 item" : "{n} items", { n });
}

function cleanupPicked(plan) {
  return { groups: new Set(plan.groups.map((g) => g.id)), folders: new Set(plan.folders.filter((f) => !f.key).map((f) => f.path)) };
}

function cleanupTotal(plan, picked) {
  const groups = plan.groups.filter((g) => picked.groups.has(g.id));
  const folders = plan.folders.filter((f) => picked.folders.has(f.path));
  const n = groups.reduce((s, g) => s + g.count, 0) + folders.length;
  const size = groups.reduce((s, g) => s + g.bytes, 0) + folders.reduce((s, f) => s + f.bytes, 0);
  return { n, text: t("Move {items} ({size}) to the Recycle Bin", { items: itemCount(n), size: bytes(size) }) };
}

function cleanupHead(label, count, size) {
  return `${t(label)} · ${itemCount(count)} · ${bytes(size)}`;
}

function cleanupDone(r) {
  const parts = [r.moved ? t("Moved {files} to the Recycle Bin", { files: fileCount(r.moved) }) : "", r.deleted ? t("Deleted {files}", { files: fileCount(r.deleted) }) : "", r.skipped ? t("{files} could not be moved", { files: fileCount(r.skipped) }) : ""];
  return parts.filter(Boolean).join(" · ") || t("Nothing to clean up");
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

const EXPORT_RANGES = ["whole", "matches"];
const EXPORT_QUALITIES = ["original", "standard", "light"];

function exportChoice(saved, owners) {
  const v = saved && typeof saved === "object" ? saved : {};
  return {
    range: EXPORT_RANGES.includes(v.range) ? v.range : "whole",
    quality: EXPORT_QUALITIES.includes(v.quality) ? v.quality : "standard",
    member: owners.includes(v.member) ? v.member : "",
    notes: v.notes !== false,
    marks: v.marks !== false,
    matches: v.matches !== false,
  };
}

const exportView = (set, folder, video) => ({ options: video || !noPicture(set), video: !!video, path: folder || "", ready: !!folder });
const exportArgs = (choice, video) => video
  ? { kind: "debrief_video", quality: choice.quality, member: choice.member || null, notes: choice.notes, marks: choice.marks, matches: choice.matches }
  : { range: choice.range, quality: choice.quality, member: choice.member || null };

function oldSessions(sets, keep = KEEP_NEWEST) {
  return sets.filter((s) => !s.live && s.progress.have > 0).slice(keep).map((s) => s.id);
}
