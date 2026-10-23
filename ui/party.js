const PARTY_FLAGS = 28;
const PARTY_CONFETTI = 16;
const PARTY_CONFETTI_MS = 1500;
const PARTY_WAVE_MS = 1700;
const PARTY_JUMP_MS = 900;
const PARTY_BLINK_EVERY_MS = 5200;
const PARTY_BLINK_MS = 420;
const PARTY_NAP_AFTER_MS = 45000;
const flagColor = (i) => `var(--member-${MEMBER_COLORS[i % MEMBER_COLORS.length]})`;

function chick() {
  return el("i", { className: "sprite", "aria-hidden": "true" });
}

function playClass(box, name, ms) {
  box.classList.remove(name);
  void box.offsetWidth;
  box.classList.add(name);
  setTimeout(() => box.classList.remove(name), ms);
}

function confetti(box) {
  const pieces = Array.from({ length: PARTY_CONFETTI }, (_, i) => el("i", { className: "cf", style: `--c:${flagColor(i)};--x:${(i + 0.5) * 100 / PARTY_CONFETTI}%;--dx:${(i % 2 ? 1 : -1) * (6 + (i * 7) % 18)}px;--r:${(i % 2 ? 1 : -1) * (120 + (i * 53) % 240)}deg;--d:${(i * 37) % 260}ms` }));
  const group = el("span", { className: "cfs" }, ...pieces);
  box.append(group);
  setTimeout(() => group.remove(), PARTY_CONFETTI_MS + 300);
}

const partyStage = { others: 0, events: 0, built: false, blink: 0, nap: 0 };

function renderStage(on, others, events, playing) {
  const box = $("wpstage");
  box.classList.toggle("on", on);
  if (!on) {
    if (partyStage.built) box.replaceChildren();
    clearInterval(partyStage.blink);
    clearTimeout(partyStage.nap);
    box.classList.remove("blink", "nap", "idle");
    Object.assign(partyStage, { others: 0, events: 0, built: false, blink: 0, nap: 0 });
    return;
  }
  if (!partyStage.built) {
    box.replaceChildren(el("span", { className: "flags" }, ...Array.from({ length: PARTY_FLAGS }, (_, i) => el("i", { style: `--c:${flagColor(i)}` }))), el("span", { className: "walker" }, chick()));
    partyStage.blink = setInterval(() => playClass(box, "blink", PARTY_BLINK_MS), PARTY_BLINK_EVERY_MS);
    Object.assign(partyStage, { others, events, built: true });
  }
  if (others > partyStage.others) { playClass(box, "wave", PARTY_WAVE_MS); confetti(box); }
  if (events > partyStage.events) playClass(box, "jump", PARTY_JUMP_MS);
  partyStage.others = others;
  partyStage.events = events;
  if (playing === box.classList.contains("idle")) {
    box.classList.toggle("idle", !playing);
    clearTimeout(partyStage.nap);
    box.classList.remove("nap");
    if (!playing) partyStage.nap = setTimeout(() => box.classList.add("nap"), PARTY_NAP_AFTER_MS);
  }
}
