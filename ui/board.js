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
  const box = el("textarea", { value: c.text, maxLength: LIMITS.board, spellcheck: false, ariaLabel: cardTitle(c) });
  const save = () => {
    const input = $("canvas").querySelector(".bhead input");
    const title = input ? cleanText(input.value, LIMITS.card) : c.title || "";
    editing = null;
    putCard(c.card, title === (c.title || "") ? { text: box.value } : { text: box.value, title });
  };
  box.onkeydown = (e) => {
    e.stopPropagation();
    if (composing(e)) return;
    if (e.key === "Enter" && e.ctrlKey) { e.preventDefault(); save(); }
    if (e.key === "Escape") { e.preventDefault(); stopEditing(); }
  };
  return el("div", { className: "cedit" }, box, el("div", { className: "row end" }, el("button", { className: "sm", textContent: t("Cancel"), onclick: stopEditing }), el("button", { className: "primary sm", onclick: save }, t("Save"), el("kbd", { textContent: "Ctrl+Enter" }))));
}
function titleInput(c) {
  const input = el("input", { value: c.title || "", placeholder: cardTitle({ ...c, title: null }), maxLength: LIMITS.card, ariaLabel: t("Name") });
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (composing(e)) return;
    if (e.key === "Enter") { e.preventDefault(); const box = $("canvas").querySelector(".cedit textarea"); if (box) box.focus(); }
    if (e.key === "Escape") { e.preventDefault(); stopEditing(); }
  };
  return input;
}
function nameInput(value, done) {
  const input = el("input", { value, maxLength: LIMITS.card, ariaLabel: t("Name") });
  let finished = false;
  const close = (save) => {
    if (finished) return;
    finished = true;
    done(save ? cleanText(input.value, LIMITS.card) : null);
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
    { label: t("Delete…"), icon: "trash", danger: true, run: async () => { if (await ask(t("Delete this card?"), t("Delete"), true)) putCard(c.card, { gone: true }); } },
  ], anchor);
}
function track(grab, down, move, end) {
  const stop = (e) => {
    grab.removeEventListener("pointermove", move);
    grab.removeEventListener("pointerup", stop);
    grab.removeEventListener("pointercancel", stop);
    grab.releasePointerCapture(down.pointerId);
    end(e);
  };
  grab.setPointerCapture(down.pointerId);
  grab.addEventListener("pointermove", move);
  grab.addEventListener("pointerup", stop);
  grab.addEventListener("pointercancel", stop);
}
function dragCard(c, down) {
  if (down.button !== 0 || c.card === "team" || down.target.closest("button, input")) return;
  const box = down.currentTarget.parentElement;
  const tiles = [...$("canvas").querySelectorAll(".brow > .bcard:not(.team):not(.new)")].filter((x) => x !== box);
  const rects = tiles.map((x) => x.getBoundingClientRect());
  const cards = st.board.filter((x) => x.card !== "team");
  const from = cards.findIndex((x) => x.card === c.card);
  let index = null;
  const mark = (i) => tiles.forEach((x, n) => { x.classList.toggle("dropto", n === i); x.classList.toggle("dropend", i === tiles.length && n === tiles.length - 1); });
  track(down.currentTarget, down, (e) => {
    if (index === null && Math.hypot(e.clientX - down.clientX, e.clientY - down.clientY) < DRAG_START_PX) return;
    box.classList.add("drag");
    index = dropSlot(rects, e.clientX, e.clientY);
    mark(index === from ? -1 : index);
  }, (e) => {
    box.classList.remove("drag");
    mark(-1);
    if (index === null || index === from || e.type !== "pointerup") return;
    putCard(c.card, { pos: dropPos(cards.filter((x) => x.card !== c.card).map((x) => x.pos), index) });
  });
}
function spanCard(box, size) {
  box.style.gridColumn = `span ${size.w}`;
  box.style.gridRow = `span ${size.h}`;
}
function resizeCard(c, down) {
  if (down.button !== 0) return;
  const box = down.currentTarget.parentElement;
  const grid = box.parentElement;
  const css = getComputedStyle(grid);
  const scale = grid.getBoundingClientRect().width / grid.offsetWidth;
  const col = ((grid.offsetWidth + parseFloat(css.columnGap)) / BOARD_COLS) * scale;
  const row = (parseFloat(css.gridAutoRows) + parseFloat(css.rowGap)) * scale;
  let size = { w: c.w, h: c.h };
  box.classList.add("sizing");
  track(down.currentTarget, down, (e) => {
    size = boardSpan(c, e.clientX - down.clientX, e.clientY - down.clientY, col, row);
    spanCard(box, size);
  }, (e) => {
    box.classList.remove("sizing");
    if (e.type === "pointerup" && (size.w !== c.w || size.h !== c.h)) putCard(c.card, size);
    else spanCard(box, c);
  });
}
function cardEl(c) {
  const open = editing && editing.card === c.card;
  const title = open ? titleInput(c) : el("b", { textContent: cardTitle(c) });
  const face = c.card.startsWith("m:") ? avatar(c.card.slice(2), "sm") : null;
  const edit = iconButton("edit", t("Edit"), () => editCard(c));
  const more = c.card === "team" ? null : iconButton("more", t("More"), (e) => cardMenu(c, e.currentTarget));
  const head = el("div", { className: "bhead" }, face, title, el("span", { className: "sp" }), edit, more);
  head.onpointerdown = (e) => dragCard(c, e);
  if (more) head.oncontextmenu = (e) => { e.preventDefault(); cardMenu(c, e); };
  const body = open ? cardEditor(c) : c.text.trim() ? cardDom(c.text, mdHooks()) : el("button", { className: "sm write", textContent: t("Write"), onclick: () => editCard(c) });
  const grip = open ? null : el("div", { className: "bsize", ariaHidden: "true", onpointerdown: (e) => resizeCard(c, e) });
  const box = el("div", { className: "bcard" + (c.card === "team" ? " team" : "") }, head, body, grip);
  spanCard(box, open ? { w: Math.max(c.w, 4), h: Math.max(c.h, 7) } : c);
  return box;
}
function addCard() {
  editing = { adding: true };
  renderBoard();
  const input = $("canvas").querySelector(".bcard.new input");
  if (input) input.focus();
}
function newCardEl() {
  const input = nameInput("", (name) => { editing = null; if (name) putCard("", { title: name }); else renderBoard(); });
  const box = el("div", { className: "bcard new" }, el("div", { className: "bhead" }, input));
  spanCard(box, { w: 4, h: 2 });
  return box;
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
    $("canvas").replaceChildren(el("div", { className: "brow" }, ...st.board.map(cardEl), editing && editing.adding ? newCardEl() : null));
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
