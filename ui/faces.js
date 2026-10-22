const FACES = {
  cat: "M4 3l5 4h6l5-4v10c0 5-3.5 8-8 8s-8-3-8-8zM6.9 12a2.1 2.1 0 1 0 4.2 0a2.1 2.1 0 1 0 -4.2 0zM12.9 12a2.1 2.1 0 1 0 4.2 0a2.1 2.1 0 1 0 -4.2 0zM10.7 15.5h2.6L12 17z",
  fox: "M3 3l6 4h6l6-4-1.2 10L12 22 4.2 13zM6.68 11.5a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0zM13.68 11.5a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0zM10.18 18.3a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0z",
  frog: "M3 15c0-2.5 1-4 3-5a3.6 3.6 0 1 1 5-3h2a3.6 3.6 0 1 1 5 3c2 1 3 2.5 3 5 0 3-4 5-9 5s-9-2-9-5zM5.92 7.2a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM14.72 7.2a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM7 16c2.5 2 7.5 2 10 0l-0.7-0.9c-2 1.6-6.6 1.6-8.6 0z",
  panda: "M4.5 13.5a7.5 7.5 0 1 0 15 0a7.5 7.5 0 1 0 -15 0zM1.8 5.2a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM16.2 5.2a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM7.3 11.5a1.8 2.4 0 0 1 3.4 0 1.8 2.4 0 0 1-3.4 2.6 1.8 2.4 0 0 1 0-2.6zM13.3 11.5a1.8 2.4 0 0 1 3.4 0 1.8 2.4 0 0 1 0 2.6 1.8 2.4 0 0 1-3.4 0 1.8 2.4 0 0 1 0-2.6zM10.8 15.5h2.4L12 16.8z",
  owl: "M4 3l4 2.5h8L20 3v12c0 4-3.5 7-8 7s-8-3-8-7zM5.5 10a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM12.5 10a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM6.96 10a1.54 1.54 0 1 0 3.08 0a1.54 1.54 0 1 0 -3.08 0zM13.96 10a1.54 1.54 0 1 0 3.08 0a1.54 1.54 0 1 0 -3.08 0zM12 12.5l1.5 2.5h-3z",
  bunny: "M8 2c2 0 2.5 3 2.5 7h3C13.5 5 14 2 16 2c1.5 0 2 3 1.5 7 1.5 1.2 2.5 3 2.5 5.5 0 4-3.5 7.5-8 7.5s-8-3.5-8-7.5c0-2.5 1-4.3 2.5-5.5C6 5 6.5 2 8 2zM7.32 14a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM13.32 14a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM10.8 17h2.4L12 18.2z",
  octopus: "M12 2.5c-4.4 0-8 3.2-8 7.5 0 3 1.3 4.5 2 5.5L4 20.5c-0.5 1 0.8 2 1.6 1.1L9 18l1 3.6c0.3 1 1.7 1 2 0l1-3.6 3.4 3.6c0.80.9 2.1-0.1 1.6-1.1l-2.5-5c0.7-1 2.5-2.5 2.5-5.5 0-4.3-3.6-7.5-8-7.5zM6.76 10a2.24 2.24 0 1 0 4.48 0a2.24 2.24 0 1 0 -4.48 0zM12.76 10a2.24 2.24 0 1 0 4.48 0a2.24 2.24 0 1 0 -4.48 0z",
  bear: "M3 9 C0 7 1 1 5 1 Q9 1 9 5 Q12 4 15 5 Q15 1 19 1 C23 1 24 7 21 9 Q23 12 23 16 Q23 23 12 23 Q1 23 1 16 Q1 12 3 9 Z M7 9 A1 1 0 1 0 9 9 A1 1 0 1 0 7 9 Z M15 9 A1 1 0 1 0 17 9 A1 1 0 1 0 15 9 Z M12 12 C19 12 19 21 12 21 C5 21 5 12 12 12 Z M11 15 A1 1 0 1 0 13 15 A1 1 0 1 0 11 15 Z",
  penguin: "M12 1 C6 1 4 6 4 11 Q1 13 1 18 Q1 20 5 18 Q6 23 12 23 Q18 23 19 18 Q23 20 23 18 Q23 13 20 11 C20 6 18 1 12 1 Z M7 9 A1 1 0 1 0 9 9 A1 1 0 1 0 7 9 Z M15 9 A1 1 0 1 0 17 9 A1 1 0 1 0 15 9 Z M4 13 A1 1 0 1 0 6 13 A1 1 0 1 0 4 13 Z M18 13 A1 1 0 1 0 20 13 A1 1 0 1 0 18 13 Z M9 13 H15 V16 Q12 20 9 16 Z",
  koala: "M7 6 C5 0 0 2 1 8 Q0 14 5 15 Q5 23 12 23 Q19 23 19 15 Q24 14 23 8 C24 2 19 0 17 6 Q12 3 7 6 Z M6 9 A1 1 0 1 0 8 9 A1 1 0 1 0 6 9 Z M16 9 A1 1 0 1 0 18 9 A1 1 0 1 0 16 9 Z M12 10 Q14 10 14 12 V14 Q14 16 12 16 Q10 16 10 14 V12 Q10 10 12 10 Z M5 14 A1 1 0 1 0 7 14 A1 1 0 1 0 5 14 Z M17 14 A1 1 0 1 0 19 14 A1 1 0 1 0 17 14 Z M9 18 Q12 19 15 18 V20 Q12 21 9 20 Z",
  wolf: "M3 1L8 5H16L21 1V11L23 14L19 15L18 19L12 23L6 19L5 15L1 14L3 11ZM6 9L10 10V12H6ZM18 9L14 10V12H18ZM10 16H14V18L12 20L10 18Z",
  tiger: "M3 8C0 5 2 1 5 1Q8 1 9 4Q12 3 15 4Q16 1 19 1C22 1 24 5 21 8Q23 11 22 16Q20 23 12 23Q4 23 2 16Q1 11 3 8ZM6 5H8V8H6ZM11 5H13V8H11ZM16 5H18V8H16ZM5 10H9V12H5ZM15 10H19V12H15ZM10 15H14V18L12 20L10 18Z",
  raccoon: "M2 2Q7 1 8 5Q12 3 16 5Q17 1 22 2V10Q23 13 22 17Q20 22 12 23Q4 22 2 17Q1 13 2 10ZM4 8H10V14H4ZM6 10H8V12H6ZM14 8H20V14H14ZM16 10H18V12H16ZM10 17H14V19L12 21L10 19Z",
  shark: "M1 5L6 10L9 8L10 1H12L16 7Q21 8 23 13Q23 18 17 20H11L8 23H6V19L5 17L1 21V15L3 13L1 11ZM18 10A1 1 0 1 0 18 12A1 1 0 1 0 18 10ZM10 10H12V14H10ZM14 15H20V17H14Z",
  whale: "M1 11 C1 6 5 3 10 3 C15 3 18 7 18 12 Q20 12 20 9 L17 6 H21 L23 3 V12 Q23 18 17 20 C13 23 6 22 3 18 Q1 15 1 11 Z M7.5 10 A1.5 1.5 0 1 0 4.5 10 A1.5 1.5 0 1 0 7.5 10 Z",
  crab: "M7 11 L4 9 H1 V3 L3 1 V5 H5 V1 L7 3 V7 L9 10 H15 L17 7 V3 L19 1 V5 H21 V1 L23 3 V9 H20 L17 11 L20 13 L23 12 V15 L20 16 L23 19 V22 L18 19 Q12 23 6 19 L1 22 V19 L4 16 L1 15 V12 L4 13 Z M9 13 H15 V16 H9 Z",
};
const PRESETS = Object.keys(FACES);
const ROOM_FACES = {
  flame: "M13 1.5 L14 6 L17 9 L18 5.5 L21 10.5 L22 14.5 L21 18.5 L17.5 21.5 L12 22.5 L6.5 21 L3 17.5 L2 13.5 L3.5 8 L6 10.5 L7 6.5 L10 3.5 Z M11.5 10 L8 14.5 L8 17 L10 19 L13 19.5 L16 17.5 L16 15 L13.5 12 L13 15 L11.5 16 L10.5 15 Z",
  bolt: "M11 1.5 L21 1.5 L15 9 L21.5 9 L8.5 22.5 L11 13.5 L2.5 13.5 Z M11 5 L8 10.5 L9.8 10.5 L12.8 5 Z",
  crown: "M2 5 L7.5 9 L12 2 L16.5 9 L22 5 L19.5 20.5 L4.5 20.5 Z M6.7 17.5 L17.3 17.5 L17.5 15.5 L6.5 15.5 Z",
  skull: "M7 2 L17 2 L21 6 L22 13 L18.5 16 L18.5 20 L16 22 L13 22 L13 19.5 L11 19.5 L11 22 L8 22 L5.5 20 L5.5 16 L2 13 L3 6 Z M5.5 11 L6.5 12.5 L10 12.5 L10.5 9 L5.5 8 Z M13.5 9 L14 12.5 L17.5 12.5 L18.5 11 L18.5 8 Z M10 16.5 L14 16.5 L12.7 14 L11.3 14 Z",
  swords: "M19.161 4.321 L18.553 7.172 L11.733 14.486 L13.561 16.191 L12.197 17.653 L10.15 15.744 L7.763 18.304 L8.007 19.215 L6.643 20.678 L3.717 17.95 L5.081 16.487 L6.007 16.667 L8.394 14.107 L6.347 12.197 L7.711 10.735 L9.539 12.44 L16.359 5.126 Z M4.839 4.321 L7.641 5.126 L14.461 12.44 L16.289 10.735 L17.653 12.197 L15.606 14.107 L17.993 16.667 L18.919 16.487 L20.283 17.95 L17.357 20.678 L15.993 19.215 L16.237 18.304 L13.85 15.744 L11.803 17.653 L10.439 16.191 L12.267 14.486 L5.447 7.172 Z",
  shield: "M12 1.5 L21.5 5 L20.5 15.5 L17 19.5 L12 22.5 L7 19.5 L3.5 15.5 L2.5 5 Z M12 5 L6 7 L6.8 14 L9 16.8 L12 18.7 L15 16.8 L17.2 14 L18 7 Z M8.5 9 L12 11 L15.5 9 L15 12 L12 14 L9 12 Z",
  crosshair: "M8.85 4.5 L15.15 4.5 L19.5 8.85 L19.5 15.15 L15.15 19.5 L8.85 19.5 L4.5 15.15 L4.5 8.85 Z M10.5 1.5 L13.5 1.5 L13.5 7 L10.5 7 Z M10.5 17 L13.5 17 L13.5 22.5 L10.5 22.5 Z M1.5 10.5 L7 10.5 L7 13.5 L1.5 13.5 Z M17 10.5 L22.5 10.5 L22.5 13.5 L17 13.5 Z M7.5 10.11 L7.5 13.89 L10.11 16.5 L13.89 16.5 L16.5 13.89 L16.5 10.11 L13.89 7.5 L10.11 7.5 Z M12 10 L14 12 L12 14 L10 12 Z",
  rocket: "M12 1.5 L16 6 L16 11.5 L20 16 L20 19.5 L15.5 17.5 L15.5 16 L8.5 16 L8.5 17.5 L4 19.5 L4 16 L8 11.5 L8 6 Z M9.5 18 L14.5 18 L13.7 20.5 L12 22.5 L10.3 20.5 Z M10 8 L10 10 L12 11.5 L14 10 L14 8 L12 6 Z",
  wings: "M1.5 3 L8 6 L12 13 L16 6 L22.5 3 L21.5 8 L17.5 10.5 L21 9.5 L19.5 13 L15.5 15.5 L18.5 14.5 L16 18 L12 21 L8 18 L5.5 14.5 L8.5 15.5 L4.5 13 L3 9.5 L6.5 10.5 L2.5 8 Z",
  trophy: "M6 2.5 L18 2.5 L18 5 L22 5 L22 10 L19 13 L16.5 13 L14 15 L14 18 L18 18 L19 21.5 L5 21.5 L6 18 L10 18 L10 15 L7.5 13 L5 13 L2 10 L2 5 L6 5 Z M9.5 9.5 L12 11.5 L14.5 9.5 L15.5 5 L8.5 5 Z M4 9.2 L5.4 10.5 L6.7 10.5 L6 7 L4 7 Z M18 7 L17.3 10.5 L18.6 10.5 L20 9.2 L20 7 Z",
  gem: "M6 2.5 L18 2.5 L22.5 9 L12 22 L1.5 9 Z M7 7.5 L17 7.5 L15.5 5 L8.5 5 Z M11.3 17 L12.7 17 L15.5 10 L8.5 10 Z",
  comet: "M1.5 1.5 L16 5.5 L20 7.5 L22 11 L22.5 15 L20.5 19.5 L17 22 L12 22 L8 19.5 L6 16 L3 7.5 L8.5 11 L2.5 4.5 L13 8.5 Z M11.5 11.5 L10 14.5 L11.5 17.5 L14.5 19 L17.5 17.5 L19 14.5 L18 11.5 L15 10 Z",
  peak: "M1.5 21 L8 9 L10 12 L15 2 L22.5 21 Z M12 15.5 L15 13.5 L18.1 15.5 L15 8 Z",
  mask: "M1.5 4 L7 5.5 L12 4 L17 5.5 L22.5 4 L21 14 L17.5 19 L12 22 L6.5 19 L3 14 Z M5 11 L6 12 L9 13 L10 10 L5 8.5 Z M14 10 L15 13 L18 12 L19 11 L19 8.5 Z M9 17 L12 18.5 L15 17 L13 15.5 L11 15.5 Z",
  star: "M12 1.5 L15.2 8.2 L22.5 9.1 L17.2 14.3 L18.6 21.8 L12 18.2 L5.4 21.8 L6.8 14.3 L1.5 9.1 L8.8 8.2 Z M9.5 12 L12 14.5 L14.5 12 L12 9.5 Z",
  chevrons: "M2 2.5 L12 7.5 L22 2.5 L22 8 L12 13 L2 8 Z M2 12 L12 17 L22 12 L22 17.5 L12 22.5 L2 17.5 Z",
};
const ROOM_PRESETS = Object.keys(ROOM_FACES);
const IMAGE_SIDE = 96;
const IMAGE_QUALITY = 0.85;

function initial(name) {
  return ([...String(name || "?").trim()][0] || "?").toUpperCase();
}

const IMAGE_URL = /^data:image\/(png|jpeg|webp);base64,/;
const own = (table, name) => typeof name === "string" && Object.hasOwn(table, name);
const knownIcon = (icon) => IMAGE_URL.test(icon) || own(ROOM_FACES, icon) || own(FACES, icon);

function picture(label, color, image, size, shape) {
  const glyph = (shape === "room" && own(ROOM_FACES, image) && ROOM_FACES[image]) || (own(FACES, image) && FACES[image]);
  const shown = IMAGE_URL.test(image) ? el("img", { src: image, alt: "" }) : glyph ? svg("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" }, svg("path", { d: glyph })) : null;
  const a = el("span", { className: ["av", size, shape].filter(Boolean).join(" "), title: label }, shown || initial(label));
  a.style.background = color || "var(--muted)";
  return a;
}

const stamp = (image) => [image.length, image.slice(-16)];
const look = (id) => [colorIndex(id), ...stamp(st.avatars[id] || "")];
const looks = () => Object.keys(st.colors).map(look);

function avatar(id, size, dot, grey, still) {
  const a = picture(id === st.id ? st.name : st.names[id], grey ? "var(--raised-3)" : colorOf(colorIndex(id)), st.avatars[id], size);
  a.title = nameOf(id);
  if (!still) {
    a.dataset.speak = id;
    a.style.setProperty("--mc", colorOf(colorIndex(id)));
    markSpeak([a], speaking, st.id, st.capture.mix.mic.muted);
  }
  if (grey) {
    a.style.color = "var(--muted)";
    a.classList.add("dim");
  }
  if (dot) a.append(el("i", { className: dot }));
  return a;
}

function nameChip(pic, name, ...after) {
  return el("span", { className: "chip" }, pic, el("b", { textContent: name }), ...after);
}
function stack(title, sub, large) {
  return el("span", { className: large ? "t lg" : "t" }, el("b", { textContent: title }), typeof sub === "string" ? (sub ? el("small", { textContent: sub }) : null) : sub);
}

const CREST_BOTTOM = "#14171d";
const presetTile = (name, size) => {
  const color = colorOf(ROOM_PRESETS.indexOf(name));
  const a = picture("", `linear-gradient(var(--raised-3), ${CREST_BOTTOM})`, name, size, "room");
  a.style.color = color;
  a.style.border = `1px solid ${color}`;
  return a;
};
const defaultPreset = (id) => ROOM_PRESETS[[...id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % ROOM_PRESETS.length];

function roomIcon(room, size) {
  return knownIcon(room.icon) || !room.id ? picture(room.name, "var(--raised-3)", room.icon, size, "room") : presetTile(defaultPreset(room.id), size);
}

function presetImage(name) {
  const css = getComputedStyle(document.documentElement);
  const canvas = el("canvas", { width: IMAGE_SIDE, height: IMAGE_SIDE });
  const g = canvas.getContext("2d");
  const color = css.getPropertyValue(`--member-${MEMBER_COLORS[ROOM_PRESETS.indexOf(name) % MEMBER_COLORS.length]}`).trim();
  const fade = g.createLinearGradient(0, 0, 0, IMAGE_SIDE);
  fade.addColorStop(0, css.getPropertyValue("--raised-3").trim());
  fade.addColorStop(1, CREST_BOTTOM);
  const edge = IMAGE_SIDE * 0.22;
  g.beginPath();
  g.roundRect(0, 0, IMAGE_SIDE, IMAGE_SIDE, edge);
  g.fillStyle = fade;
  g.fill();
  g.strokeStyle = color;
  g.lineWidth = 4;
  g.beginPath();
  g.roundRect(2, 2, IMAGE_SIDE - 4, IMAGE_SIDE - 4, edge - 2);
  g.stroke();
  const scale = IMAGE_SIDE * 0.8 / 24;
  g.translate(IMAGE_SIDE * 0.1, IMAGE_SIDE * 0.1);
  g.scale(scale, scale);
  g.fillStyle = g.strokeStyle = color;
  g.lineWidth = 0.5;
  g.lineJoin = "round";
  const path = new Path2D(ROOM_FACES[name]);
  g.fill(path);
  g.stroke(path);
  return canvas.toDataURL("image/png");
}

async function squareImage(file) {
  const bitmap = await createImageBitmap(file);
  const side = Math.min(bitmap.width, bitmap.height);
  const canvas = el("canvas", { width: IMAGE_SIDE, height: IMAGE_SIDE });
  canvas.getContext("2d").drawImage(bitmap, (bitmap.width - side) / 2, (bitmap.height - side) / 2, side, side, 0, 0, IMAGE_SIDE, IMAGE_SIDE);
  bitmap.close();
  return canvas.toDataURL("image/webp", IMAGE_QUALITY);
}

function pickImage(apply) {
  const input = $("imagefile");
  input.onchange = () => {
    const file = input.files[0];
    input.value = "";
    if (file) squareImage(file).then(apply).catch(() => fail("That image is not valid."));
  };
  input.click();
}
