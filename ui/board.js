const BOARD_MAX = 4000;
const BOARD_PAD = 28;

let editing = null;
let boardFull = false;

const onBoard = () => boardWindow || (page === "room" && roomTab === "board");
const cardTitle = (c) => c.title || (c.card.startsWith("m:") ? nameOf(c.card.slice(2)) : c.card === "team" ? t("Game plan") : t("Card"));
const putCard = (card, fields) => invoke("put_board_card", { room: st.room.id, card, ...fields }).then(refresh).catch(fail);


function editCard(c) {
  editing = { card: c.card };
  renderBoard();
  const box = $("canvas").querySelector("textarea");
  if (box) box.focus();
}
function stopEditing() {
  editing = null;
  renderBoard();
}
function cardEditor(c) {
  const box = el("textarea", { value: c.text, maxLength: BOARD_MAX, spellcheck: false, ariaLabel: cardTitle(c) });
  const save = () => { editing = null; putCard(c.card, { text: box.value }); };
  box.onkeydown = (e) => {
    e.stopPropagation();
    if (composing(e)) return;
    if (e.key === "Enter" && e.ctrlKey) { e.preventDefault(); save(); }
    if (e.key === "Escape") { e.preventDefault(); stopEditing(); }
  };
  return el("div", { className: "cedit" }, box, el("div", { className: "row end" }, el("button", { className: "sm", textContent: t("Cancel"), onclick: stopEditing }), el("button", { className: "primary sm", onclick: save }, t("Save"), el("kbd", { textContent: "Ctrl+Enter" }))));
}
function nameInput(value, done) {
  const input = el("input", { value, maxLength: 24, ariaLabel: t("Name") });
  let finished = false;
  const close = (save) => {
    if (finished) return;
    finished = true;
    done(save ? input.value.trim() : null);
  };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (composing(e)) return;
    if (e.key === "Enter") close(true);
    if (e.key === "Escape") { e.preventDefault(); close(false); }
  };
  input.onblur = () => close(false);
  return input;
}
function cardMenu(c, anchor) {
  openMenu([
    { label: t("Rename"), icon: "edit", run: () => { editing = { card: c.card, rename: true }; renderBoard(); const input = $("canvas").querySelector(".bhead input"); if (input) input.focus(); } },
    ...(c.card === "team" ? [] : [{ separator: true }, { label: t("Delete…"), icon: "trash", danger: true, run: async () => { if (await ask(t("Delete this card?"), t("Delete"), true)) putCard(c.card, { gone: true }); } }]),
  ], anchor);
}
function dragCard(c, down) {
  if (down.button !== 0 || c.card === "team" || down.target.closest("button, input")) return;
  const grab = down.currentTarget;
  const from = { x: down.clientX, y: down.clientY };
  const tiles = [...$("canvas").querySelectorAll(".brow > .bcard")];
  let moving = false;
  const move = (e) => {
    if (!moving && Math.hypot(e.clientX - from.x, e.clientY - from.y) < DRAG_START_PX) return;
    moving = true;
    grab.parentElement.classList.add("drag");
  };
  const end = (e) => {
    grab.removeEventListener("pointermove", move);
    grab.removeEventListener("pointerup", end);
    grab.removeEventListener("pointercancel", end);
    grab.releasePointerCapture(down.pointerId);
    grab.parentElement.classList.remove("drag");
    if (!moving || e.type !== "pointerup") return;
    const others = st.board.filter((x) => x.card !== "team" && x.card !== c.card);
    const rects = tiles.filter((box) => box !== grab.parentElement).map((box) => box.getBoundingClientRect());
    const index = rects.filter((r) => r.bottom < e.clientY || (r.top <= e.clientY && r.left + r.width / 2 < e.clientX)).length;
    putCard(c.card, { pos: dropPos(others.map((x) => x.pos), index) });
  };
  grab.setPointerCapture(down.pointerId);
  grab.addEventListener("pointermove", move);
  grab.addEventListener("pointerup", end);
  grab.addEventListener("pointercancel", end);
}
function cardEl(c) {
  const renaming = editing && editing.rename && editing.card === c.card;
  const title = renaming ? nameInput(c.title || "", (name) => { editing = null; if (name !== null && name !== (c.title || "")) putCard(c.card, { title: name }); else renderBoard(); }) : el("b", { textContent: cardTitle(c) });
  const face = c.card.startsWith("m:") ? avatar(c.card.slice(2), "sm") : null;
  const edit = iconButton("edit", t("Edit"), () => editCard(c));
  const more = iconButton("more", t("More"), (e) => cardMenu(c, e.currentTarget));
  const head = el("div", { className: "bhead" }, face, title, el("span", { className: "sp" }), edit, more);
  head.onpointerdown = (e) => dragCard(c, e);
  head.oncontextmenu = (e) => { e.preventDefault(); cardMenu(c, e); };
  const body = editing && editing.card === c.card && !editing.rename ? cardEditor(c) : c.text.trim() ? cardDom(c.text, mdHooks()) : el("button", { className: "sm write", textContent: t("Write"), onclick: () => editCard(c) });
  return el("div", { className: "bcard" + (c.card === "team" ? " team" : "") }, head, body);
}
function addCard() {
  editing = { adding: true };
  renderBoard();
  const input = $("canvas").querySelector(".bcard.new input");
  if (input) input.focus();
}
function newCardEl() {
  const input = nameInput("", (name) => { editing = null; if (name) putCard("", { title: name }); else renderBoard(); });
  return el("div", { className: "bcard new" }, el("div", { className: "bhead" }, input));
}
function fitAll() {
  const wrap = $("boardwrap");
  const canvas = $("canvas");
  canvas.style.zoom = "1";
  return Math.min((wrap.clientWidth - 2 * BOARD_PAD) / canvas.offsetWidth, (wrap.clientHeight - 2 * BOARD_PAD) / Math.max(1, canvas.offsetHeight));
}
function placeBoard() {
  $("canvas").style.zoom = String(fitAll());
}
function renderBoard() {
  if (!onBoard() || !inRoom()) return;
  if (changed("board", editing ? [editing, lang] : [st.board, st.names, looks(), lang, st.tag_names])) {
    $("canvas").replaceChildren(...st.board.filter((c) => c.card === "team").map(cardEl), el("div", { className: "brow" }, ...st.board.filter((c) => c.card !== "team").map(cardEl), editing && editing.adding ? newCardEl() : null));
  }
  placeBoard();
}
function toggleBoardFull() {
  boardFull = !boardFull;
  document.body.classList.toggle("boardfull", boardFull);
  setFull(boardFull);
  setTimeout(placeBoard, 50);
}
new ResizeObserver(() => { if (onBoard()) placeBoard(); }).observe($("boardpane"));
$("boardfull").onclick = toggleBoardFull;
$("boardadd").onclick = addCard;
$("boardwin").onclick = () => invoke("open_board_window").catch(fail);
