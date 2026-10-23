const CHICK_INK = "#3a2a1a";
const CHICK_PARTS = `<g class="ck-hop"><g class="ck-all">
<ellipse cx="37" cy="88" rx="9" ry="4.5" fill="#ff8a2b" stroke="${CHICK_INK}" stroke-width="3"/>
<ellipse cx="59" cy="88" rx="9" ry="4.5" fill="#ff8a2b" stroke="${CHICK_INK}" stroke-width="3"/>
<circle cx="48" cy="54" r="33" fill="#ffd23f" stroke="${CHICK_INK}" stroke-width="3"/>
<ellipse cx="48" cy="64" rx="19" ry="15" fill="#ffe58a"/>
<path d="M43 22 Q45 10 50 20 M51 21 Q58 11 57 24" fill="none" stroke="${CHICK_INK}" stroke-width="3" stroke-linecap="round"/>
<circle cx="30" cy="60" r="5" fill="#ff9a8a" opacity=".75"/><circle cx="66" cy="60" r="5" fill="#ff9a8a" opacity=".75"/>
<g class="ck-eye"><ellipse cx="38" cy="50" rx="3.6" ry="4.8" fill="${CHICK_INK}"/><circle cx="39.3" cy="48.3" r="1.3" fill="#fff"/></g>
<g class="ck-eye"><ellipse cx="58" cy="50" rx="3.6" ry="4.8" fill="${CHICK_INK}"/><circle cx="59.3" cy="48.3" r="1.3" fill="#fff"/></g>
<path d="M43 57 L53 57 L48 64 Z" fill="#ff8a2b" stroke="${CHICK_INK}" stroke-width="2.4" stroke-linejoin="round"/>
<path d="M17 52 Q17 14 48 14 Q79 14 79 52" fill="none" stroke="#34414f" stroke-width="5"/>
<rect x="11" y="44" width="11" height="22" rx="5.5" fill="#34414f" stroke="${CHICK_INK}" stroke-width="2"/>
<rect x="74" y="44" width="11" height="22" rx="5.5" fill="#34414f" stroke="${CHICK_INK}" stroke-width="2"/>
<rect x="14" y="50" width="5" height="10" rx="2.5" fill="#4fd1c5"/><rect x="77" y="50" width="5" height="10" rx="2.5" fill="#4fd1c5"/>
<path d="M16 64 Q20 76 36 73" fill="none" stroke="#34414f" stroke-width="3" stroke-linecap="round"/>
<circle cx="39" cy="73" r="3.6" fill="#34414f" stroke="${CHICK_INK}" stroke-width="1.6"/>
<path class="ck-wing" d="M74 62 Q88 66 83 81 Q72 79 70 68 Z" fill="#ffc21a" stroke="${CHICK_INK}" stroke-width="3" stroke-linejoin="round"/>
<text class="ck-z" x="72" y="22" font-size="20" font-weight="700" fill="#9fb3c8">z</text>
</g></g>`;
const PARTY_FLAGS = 28;
const PARTY_CONFETTI = 16;
const PARTY_CONFETTI_MS = 1500;
const PARTY_WAVE_MS = 1700;
const PARTY_JUMP_MS = 900;
const flagColor = (i) => `var(--member-${MEMBER_COLORS[i % MEMBER_COLORS.length]})`;

function chick() {
  const s = svg("svg", { viewBox: "0 0 96 96", class: "chick", "aria-hidden": "true" });
  s.innerHTML = CHICK_PARTS;
  return s;
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

const partyStage = { others: 0, events: 0, built: false };

function renderStage(on, others, events, playing) {
  const box = $("wpstage");
  box.classList.toggle("on", on);
  if (!on) {
    if (partyStage.built) box.replaceChildren();
    Object.assign(partyStage, { others: 0, events: 0, built: false });
    return;
  }
  if (!partyStage.built) {
    box.replaceChildren(el("span", { className: "flags" }, ...Array.from({ length: PARTY_FLAGS }, (_, i) => el("i", { style: `--c:${flagColor(i)}` }))), el("span", { className: "walker" }, chick()));
    Object.assign(partyStage, { others, events, built: true });
  }
  if (others > partyStage.others) { playClass(box, "wave", PARTY_WAVE_MS); confetti(box); }
  if (events > partyStage.events) playClass(box, "jump", PARTY_JUMP_MS);
  partyStage.others = others;
  partyStage.events = events;
  box.classList.toggle("idle", !playing);
}
