const FACES = {
  bolt: "M13.5 2 5 13.5h5.5L9.5 22 19 10h-5.5z",
  star: "M12 2l2.5 6.6 7 .3-5.5 4.4 1.9 6.8L12 16.2l-5.9 3.9 1.9-6.8-5.5-4.4 7-.3z",
  flame: "M12 2c1 4 6 6.5 6 12a6 6 0 0 1-12 0c0-2.5 1-4 2.5-5.5C9 10 10 11 11 11c0-3-1-5.5 1-9z",
  crown: "M3 8l4.5 4L12 5l4.5 7L21 8l-2 11H5z",
  heart: "M12 21C5 15.5 3 12 3 8.5a4.5 4.5 0 0 1 9-2 4.5 4.5 0 0 1 9 2c0 3.5-2 7-9 12.5z",
  diamond: "M12 2l8 10-8 10-8-10z",
  shield: "M12 2l8 3v6c0 5-3.5 9-8 11-4.5-2-8-6-8-11V5z",
  moon: "M20 14.5A8.5 8.5 0 0 1 9.5 4 8.5 8.5 0 1 0 20 14.5z",
  target: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zm0 4a5 5 0 1 1 0 10 5 5 0 0 1 0-10zm0 3a2 2 0 1 0 0 4 2 2 0 0 0 0-4z",
  ghost: "M12 2a7 7 0 0 0-7 7v12l3.5-2.5L12 21l3.5-2.5L19 21V9a7 7 0 0 0-7-7zM9.5 10a1.25 1.25 0 1 0 2.5 0 1.25 1.25 0 0 0-2.5 0zM14 10a1.25 1.25 0 1 0 2.5 0 1.25 1.25 0 0 0-2.5 0z",
};
const IMAGE_SIDE = 96;
const IMAGE_QUALITY = 0.85;

function initial(name) {
  return (String(name || "?").trim().charAt(0) || "?").toUpperCase();
}

function picture(label, color, image, size, shape) {
  const shown = /^data:image\/(png|jpeg|webp);base64,/.test(image) ? el("img", { src: image, alt: "" }) : FACES[image] ? svg("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" }, svg("path", { d: FACES[image] })) : null;
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
  if (grey) a.style.color = "var(--muted)";
  if (dot) a.append(el("i", { className: dot }));
  return a;
}

const roomIcon = (room, size) => picture(room.name, "var(--raised-3)", room.icon, size, "room");

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
