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
  flame: "M12 1C13 5 19 8 19 14.5A7 7 0 0 1 5 14.5C5 11 7 9.5 8 7.5C8.5 9 9.5 10 10.5 10C10 6 10.5 3 12 1zM12 21a3.2 3.2 0 0 0 3.2-3.2C15.2 15.5 13.5 14.5 12 12.5C10.5 14.5 8.8 15.5 8.8 17.8A3.2 3.2 0 0 0 12 21z",
  bolt: "M14 1L4 13.5h6.5L9 23l11-13.5h-6.8z",
  crown: "M2 8l5 4 5-8 5 8 5-4-2 13H4zM5.6 17h12.8v2H5.6z",
  skull: "M12 1.5C6.5 1.5 3.5 5.5 3.5 10c0 3 1.3 4.8 3 5.8V20h3v-2h1.5v2h2v-2h1.5v2h3v-4.2c1.7-1 3-2.8 3-5.8 0-4.5-3-8.5-8.5-8.5zM8.5 8.5a2.3 2.3 0 1 0 0 4.6a2.3 2.3 0 1 0 0-4.6zM15.5 8.5a2.3 2.3 0 1 0 0 4.6a2.3 2.3 0 1 0 0-4.6zM12 13l-1.3 2.5h2.6z",
  swords: "M20.13 3.87L19.35 6.63L10.87 15.11L8.89 13.13L17.37 4.65zM6.34 10.59L13.41 17.66L12.14 18.93L5.07 11.86zM7.97 14.76L9.24 16.03L6.77 18.51L5.49 17.23zM5 16.74L7.26 19L5.71 20.56L3.44 18.29zM3.87 3.87L6.63 4.65L15.11 13.13L13.13 15.11L4.65 6.63zM10.59 17.66L17.66 10.59L18.93 11.86L11.86 18.93zM14.76 16.03L16.03 14.76L18.51 17.23L17.23 18.51zM16.74 19L19 16.74L20.56 18.29L18.29 20.56z",
  shield: "M12 1.5l9 3v7c0 5.5-3.8 9.5-9 11.5C6.8 21 3 17 3 11.5v-7zM12 5L6 7v4.5c0 3.6 2.3 6.3 6 8 3.7-1.7 6-4.4 6-8V7zM12 8.5l1.4 3 3.2.4-2.4 2.2.7 3.2L12 15.7l-2.9 1.6.7-3.2-2.4-2.2 3.2-.4z",
  crosshair: "M12 3.5a8.5 8.5 0 1 0 0 17a8.5 8.5 0 1 0 0-17zM12 5.5a6.5 6.5 0 1 1 0 13a6.5 6.5 0 1 1 0-13zM11 0.5h2v8h-2zM11 15.5h2v8h-2zM0.5 11h8v2h-8zM15.5 11h8v2h-8zM12 10.3a1.7 1.7 0 1 0 0 3.4a1.7 1.7 0 1 0 0-3.4z",
  rocket: "M12 1c4 3 5.5 7 5 12l-2 3H9l-2-3C6.5 8 8 4 12 1zM12 7.5a2 2 0 1 0 0 4a2 2 0 1 0 0-4zM7 13l-3.5 4.5V21L8 18.5zM17 13l3.5 4.5V21L16 18.5zM10 17.5h4L12 23z",
  wings: "M11 19L11 8L8 4L3 1.5L5 5.5L0.8 6L4.5 9L1 12L5.2 13L2.5 17L7.5 16.5L6.5 20zM13 19L13 8L16 4L21 1.5L19 5.5L23.2 6L19.5 9L23 12L18.8 13L21.5 17L16.5 16.5L17.5 20z",
  trophy: "M6 2h12v6c0 3.5-2.5 6-6 6S6 11.5 6 8zM6 4H2v3c0 2.5 2 4 4.4 4.4l.3-1.8C5.3 9.3 4.2 8.4 4.2 7V6.2H6zM18 4h4v3c0 2.5-2 4-4.4 4.4l-.3-1.8c1.4-.3 2.5-1.2 2.5-2.6V6.2H18zM10.8 14h2.4v4h-2.4zM7 19h10v3H7z",
  gem: "M6 3h12l4 6-10 13L2 9zM8.2 9L12 16.8 15.8 9z",
  comet: "M16.5 12a4.5 4.5 0 1 0 0 9a4.5 4.5 0 1 0 0-9zM2 2L16.3 12.1L12.1 16.3zM2 10L7 15L8.5 17.5zM10 2L15 7L17.5 8.5z",
  peak: "M1 21L9 5l4.5 8 2.5-4L23 21zM9 5l-2.6 4.8 2-1.3 1.3 1.6 1.3-1.6 2 1.3z",
  mask: "M3 4h18v7c0 6-4 10-9 10S3 17 3 11zM5.5 9l5 2.5-1 1.5-4.5-1zM18.5 9l-5 2.5 1 1.5 4.5-1zM9.5 16.5h5v1.5h-5z",
  star: "M12 1l3.1 7 7.4.7-5.6 5 1.7 7.3L12 17l-6.6 4 1.7-7.3-5.6-5 7.4-.7z",
  chevrons: "M12 2l9 8-2.3 2.6L12 6.7 5.3 12.6 3 10zM12 9.5l9 8-2.3 2.6L12 14.2 5.3 20.1 3 17.5z",
};
const ROOM_PRESETS = Object.keys(ROOM_FACES);
const LEGACY = { bolt: "tiger", star: "penguin", flame: "fox", crown: "wolf", heart: "bear", diamond: "shark", shield: "crab", moon: "owl", target: "raccoon", ghost: "bunny", robot: "koala", alien: "octopus", skull: "panda", ninja: "whale", slime: "frog", cyclops: "tiger" };
const IMAGE_SIDE = 96;
const IMAGE_QUALITY = 0.85;

function initial(name) {
  return ([...String(name || "?").trim()][0] || "?").toUpperCase();
}

function picture(label, color, image, size, shape) {
  const glyph = (shape === "room" && ROOM_FACES[image]) || FACES[LEGACY[image] || image];
  const shown = /^data:image\/(png|jpeg|webp);base64,/.test(image) ? el("img", { src: image, alt: "" }) : glyph ? svg("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" }, svg("path", { d: glyph })) : null;
  const a = el("span", { className: ["av", size, shape].filter(Boolean).join(" "), title: label }, shown || initial(label));
  a.style.background = color || "var(--muted)";
  return a;
}

const stamp = (image) => [image.length, image.slice(-16)];
const look = (id) => [colorIndex(id), ...stamp(st.avatars[id] || "")];
const looks = () => Object.keys(st.colors).map(look);

function avatar(id, size, dot, grey) {
  const a = picture(id === st.id ? st.name : st.names[id], grey ? "var(--raised-3)" : colorOf(colorIndex(id)), st.avatars[id], size);
  a.title = nameOf(id);
  if (grey) {
    a.style.color = "var(--muted)";
    a.classList.add("dim");
  }
  if (dot) a.append(el("i", { className: dot }));
  return a;
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
  return room.icon || !room.id ? picture(room.name, "var(--raised-3)", room.icon, size, "room") : presetTile(defaultPreset(room.id), size);
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
  g.fill(path, "evenodd");
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
