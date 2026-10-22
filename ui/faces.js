const FACES = {
  cat: "M4 3l5 4h6l5-4v10c0 5-3.5 8-8 8s-8-3-8-8zM6.9 12a2.1 2.1 0 1 0 4.2 0a2.1 2.1 0 1 0 -4.2 0zM12.9 12a2.1 2.1 0 1 0 4.2 0a2.1 2.1 0 1 0 -4.2 0zM10.7 15.5h2.6L12 17z",
  fox: "M3 3l6 4h6l6-4-1.2 10L12 22 4.2 13zM6.68 11.5a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0zM13.68 11.5a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0zM10.18 18.3a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0z",
  frog: "M3 15c0-2.5 1-4 3-5a3.6 3.6 0 1 1 5-3h2a3.6 3.6 0 1 1 5 3c2 1 3 2.5 3 5 0 3-4 5-9 5s-9-2-9-5zM5.92 7.2a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM14.72 7.2a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM7 16c2.5 2 7.5 2 10 0l-0.7-0.9c-2 1.6-6.6 1.6-8.6 0z",
  robot: "M11 2h2v2.5h5a2 2 0 0 1 2 2V18a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6.5a2 2 0 0 1 2-2h5zM7.5 9h3v3h-3zM13.5 9h3v3h-3zM8 15h8v1.8H8z",
  alien: "M12 2c5 0 8 3.5 8 8 0 5-4 10-8 12-4-2-8-7-8-12 0-4.5 3-8 8-8zM6 9.5c2 0 4.5 1.5 4.5 3.5-2.5 0-4.5-1.5-4.5-3.5zM18 9.5c-2 0-4.5 1.5-4.5 3.5 2.5 0 4.5-1.5 4.5-3.5z",
  skull: "M12 2.5a8 8 0 0 0-8 8c0 2.6 1.2 4.4 3 5.6V21h3v-2h4v2h3v-4.9c1.8-1.2 3-3 3-5.6a8 8 0 0 0-8-8zM6.7 11a2 2 0 1 0 4 0a2 2 0 1 0 -4 0zM13.3 11a2 2 0 1 0 4 0a2 2 0 1 0 -4 0z",
  ghost: "M12 2a7 7 0 0 0-7 7v12l3.5-2.5L12 21l3.5-2.5L19 21V9a7 7 0 0 0-7-7zM7.48 9.5a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0zM12.88 9.5a1.82 1.82 0 1 0 3.64 0a1.82 1.82 0 1 0 -3.64 0zM9.76 13.8a2.24 2.24 0 1 0 4.48 0a2.24 2.24 0 1 0 -4.48 0z",
  octopus: "M12 2.5c-4.4 0-8 3.2-8 7.5 0 3 1.3 4.5 2 5.5L4 20.5c-0.5 1 0.8 2 1.6 1.1L9 18l1 3.6c0.3 1 1.7 1 2 0l1-3.6 3.4 3.6c0.80.9 2.1-0.1 1.6-1.1l-2.5-5c0.7-1 2.5-2.5 2.5-5.5 0-4.3-3.6-7.5-8-7.5zM6.76 10a2.24 2.24 0 1 0 4.48 0a2.24 2.24 0 1 0 -4.48 0zM12.76 10a2.24 2.24 0 1 0 4.48 0a2.24 2.24 0 1 0 -4.48 0z",
  ninja: "M2.5 12a9.5 9.5 0 1 0 19 0a9.5 9.5 0 1 0 -19 0zM5.5 9.5h13v4h-13zM7.46 11.5a1.54 1.54 0 1 0 3.08 0a1.54 1.54 0 1 0 -3.08 0zM13.46 11.5a1.54 1.54 0 1 0 3.08 0a1.54 1.54 0 1 0 -3.08 0z",
  panda: "M4.5 13.5a7.5 7.5 0 1 0 15 0a7.5 7.5 0 1 0 -15 0zM1.8 5.2a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM16.2 5.2a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM7.3 11.5a1.8 2.4 0 0 1 3.4 0 1.8 2.4 0 0 1-3.4 2.6 1.8 2.4 0 0 1 0-2.6zM13.3 11.5a1.8 2.4 0 0 1 3.4 0 1.8 2.4 0 0 1 0 2.6 1.8 2.4 0 0 1-3.4 0 1.8 2.4 0 0 1 0-2.6zM10.8 15.5h2.4L12 16.8z",
  slime: "M12 2c3 4 7 7 7 12a7 7 0 0 1-14 0c0-5 4-8 7-12zM7.54 14a1.96 1.96 0 1 0 3.92 0a1.96 1.96 0 1 0 -3.92 0zM12.54 14a1.96 1.96 0 1 0 3.92 0a1.96 1.96 0 1 0 -3.92 0zM9.5 17.3c1.5 1.4 3.5 1.4 5 0l-0.6-0.8c-1.2 1-2.6 1-3.8 0z",
  owl: "M4 3l4 2.5h8L20 3v12c0 4-3.5 7-8 7s-8-3-8-7zM5.5 10a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM12.5 10a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM6.96 10a1.54 1.54 0 1 0 3.08 0a1.54 1.54 0 1 0 -3.08 0zM13.96 10a1.54 1.54 0 1 0 3.08 0a1.54 1.54 0 1 0 -3.08 0zM12 12.5l1.5 2.5h-3z",
  crown: "M3 8l4.5 4L12 5l4.5 7L21 8l-2 11H5zM10.04 14a1.96 1.96 0 1 0 3.92 0a1.96 1.96 0 1 0 -3.92 0z",
  flame: "M12 2c1 4 6 6.5 6 12a6 6 0 0 1-12 0c0-2.5 1-4 2.5-5.5C9 10 10 11 11 11c0-3-1-5.5 1-9zM12 19a2.5 2.5 0 0 0 2.5-2.5c0-1.5-1.5-2.5-2.5-4-1 1.5-2.5 2.5-2.5 4A2.5 2.5 0 0 0 12 19z",
  bunny: "M8 2c2 0 2.5 3 2.5 7h3C13.5 5 14 2 16 2c1.5 0 2 3 1.5 7 1.5 1.2 2.5 3 2.5 5.5 0 4-3.5 7.5-8 7.5s-8-3.5-8-7.5c0-2.5 1-4.3 2.5-5.5C6 5 6.5 2 8 2zM7.32 14a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM13.32 14a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM10.8 17h2.4L12 18.2z",
  cyclops: "M5 2l3 4.5h8L19 2l0.8 8.5c0.7 1 1.2 2.3 1.2 3.5 0 4.5-4 8-9 8s-9-3.5-9-8c0-1.20.5-2.5 1.2-3.5zM8.5 12a3.5 3.5 0 1 0 7 0a3.5 3.5 0 1 0 -7 0zM9.9 12a2.1 2.1 0 1 0 4.2 0a2.1 2.1 0 1 0 -4.2 0zM8.5 17.5h7V19h-7z",
};
const PRESETS = Object.keys(FACES);
const LEGACY = { bolt: "robot", star: "alien", heart: "cat", diamond: "slime", shield: "ninja", moon: "owl", target: "fox" };
const IMAGE_SIDE = 96;
const IMAGE_QUALITY = 0.85;

function initial(name) {
  return (String(name || "?").trim().charAt(0) || "?").toUpperCase();
}

function picture(label, color, image, size, shape) {
  const shown = /^data:image\/(png|jpeg|webp);base64,/.test(image) ? el("img", { src: image, alt: "" }) : FACES[LEGACY[image] || image] ? svg("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" }, svg("path", { d: FACES[LEGACY[image] || image] })) : null;
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

const presetTile = (name, size) => {
  const a = picture("", colorOf(PRESETS.indexOf(name)), name, size, "room");
  a.style.color = "var(--on-member)";
  return a;
};
const defaultPreset = (id) => PRESETS[[...id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % PRESETS.length];

function roomIcon(room, size) {
  return room.icon || !room.id ? picture(room.name, "var(--raised-3)", room.icon, size, "room") : presetTile(defaultPreset(room.id), size);
}

function presetImage(name) {
  const css = getComputedStyle(document.documentElement);
  const canvas = el("canvas", { width: IMAGE_SIDE, height: IMAGE_SIDE });
  const g = canvas.getContext("2d");
  g.fillStyle = css.getPropertyValue(`--member-${MEMBER_COLORS[PRESETS.indexOf(name) % MEMBER_COLORS.length]}`).trim();
  g.fillRect(0, 0, IMAGE_SIDE, IMAGE_SIDE);
  const scale = IMAGE_SIDE * 0.62 / 24;
  g.translate(IMAGE_SIDE * 0.19, IMAGE_SIDE * 0.19);
  g.scale(scale, scale);
  g.fillStyle = g.strokeStyle = css.getPropertyValue("--deep").trim();
  g.lineWidth = 0.5;
  g.lineJoin = "round";
  const path = new Path2D(FACES[name]);
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
