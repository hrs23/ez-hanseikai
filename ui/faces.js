const FACES = {
  wolf: "M4 2 9 5 12 4 15 5 20 2 19 10 22 14.0H19.0L20 17.0L16 18.0L12 22.0L8 18.0L4 17.0L5 14.0H2.0L5 10.0L4 2.0Z M6 10 7 12 10 13.0V11.0Z M14 11.0V13.0L17 12.0L18 10.0Z M10 16 12 18 14 16.0Z",
  fox: "M3 2 10 7.0H14.0L21 2.0L19 12.0L21 13.0L17 17.0L12 22.0L7 17.0L3 13.0L5 12.0L3 2.0Z M5 5 6 9 8 8.0Z M19 5 16 8 18 9.0Z M6 11 7 13 9 14 10 12.0Z M14 12 15 14 17 13 18 11.0Z M10 17 12 19 14 17.0Z",
  hawk: "M4 3 12 4.0Q17 4 18 8.0L22 10.0V14.0L19 12.0L17 14.0L16 18.0L9 22.0L10 18.0L4 19.0L6 15.0L2 16.0L5 11.0L4 3.0Z M12 7.0V9.0L15 10.0L16 8.0Z",
  raven: "M5 3 12 5.0Q16 4 18 8.0L22 12.0H17.0V16.0L19 21.0L12 19.0L8 22.0V18.0H3.0L5 14.0L2 12.0L6 10.0L5 3.0Z M12 8.0V10.0L14 11.0L16 9.0Z",
  owl: "M3 3 9 6.0Q12 5 15 6.0L21 3.0L20 10.0Q22 18 16 20.0L12 22.0L8 20.0Q2 18 4 10.0L3 3.0Z M5 9.0V11.0Q5 15 9 14.0L10 11.0Q8 8 5 9.0Z M19 9.0Q16 8 14 11.0L15 14.0Q19 15 19 11.0Z M10.5 15 12 18 13.5 15.0Z",
  cobra: "M12 2.0Q16 2 17 6.0Q23 10 21 17.0L17 20.0L14 18.0L15 22.0H9.0L10 18.0L7 20.0L3 17.0Q1 10 7 6.0Q8 2 12 2.0Z M8.3 6.3V8.0L10 9.0L11 7.3Z M13 7.3 14 9 15.7 8.0V6.3Z M10 12 12 15 14 12.0Z M5 12 6 16 8 17 7 13.0Z M17 13 16 17 18 16 19 12.0Z",
  viper: "M2 5 8 3.0H16.0L22 5.0L20 13.0L16 17.0L15 21.0H9.0L8 17.0L4 13.0L2 5.0Z M5 7 6 10 9 11 10 9.0Z M14 9 15 11 18 10 19 7.0Z M9 14.0V16.0L11 17.0V14.0Z M13 14.0V17.0L15 16.0V14.0Z",
  scorpion: "M8 13.0H16.0L17 15.0L20 12.0L19 10.0H17.0V7.0L19 8.0L21 6.0L20 4.0L23 6.0V10.0L21 14.0L18 17.0L17 21.0H7.0L6 17.0L3 14.0L1 10.0V6.0L4 4.0L3 6.0L5 8.0L7 7.0V10.0H5.0L4 12.0L7 15.0L8 13.0Z M10 14 9 8.0Q8 2 13 2.0Q18 2 18 6.0L16 9.0L14 7.0L15 5.0Q11 3 11 7.0L13 14.0H10.0Z M9 16.0V18.0L10 19.0L11 17.0Z M13 17 14 19 15 18.0V16.0Z",
  shark: "M2 12.0Q5 8 10 7.0L13 2.0L15 7.0Q19 8 22 12.0L20 18.0L14 20.0L8 22.0L9 18.0L3 17.0L7 15.0H3.0L2 12.0Z M15 10.0V12.0L16 13.0L18 11.0Z M10 14.0V17.0L12 16.0L13 18.0L15 16.0L17 17.0L19 14.0Z M7 10 6 13.0H8.0L9 10.0Z",
  orca: "M2 14.0Q3 5 12 4.0Q19 3 21 9.0L22 13.0L19 17.0L15 18.0L12 22.0L9 19.0H4.0L6 16.0L2 14.0Z M13 8.0Q11 11 13 12.0H17.0L18 9.0Q16 6 13 8.0Z M7 15 11 18 17 16 19 14.0Q12 14 7 15.0Z",
  bear: "M4 8.0Q0 4 4 2.0Q7 1 9 5.0Q12 4 15 5.0Q17 1 20 2.0Q24 4 20 8.0L21 14.0L19 19.0L15 21.0H9.0L5 19.0L3 14.0L4 8.0Z M6 9.0V11.0L9 12.0L10 10.0Z M14 10 15 12 18 11.0V9.0Z M9 15 10 18.0H14.0L15 15.0Q12 13 9 15.0Z",
  tiger: "M4 7.0Q2 3 5 2.0Q8 2 9 5.0H10.0L12 9.0L14 5.0H15.0Q16 2 19 2.0Q22 3 20 7.0L20.5 8.0L16 9.0L17 11.0L20.8 10.0L21 12.0L20.5 15.0L17 16.0L18 18.0L20.1 17.4L20 18.0L16 21.0H8.0L4 18.0L3.9 17.4L6 18.0L7 16.0L3.5 15.0L3 12.0L3.2 10.0L7 11.0L8 9.0L3.5 8.0L4 7.0Z M6 12 7 14.0H9.0L10 12.0Z M14 12 15 14.0H17.0L18 12.0Z M10 17 12 20 14 17.0Z",
  panther: "M4 8.0Q2 3 6 3.0Q8 3 9 6.0Q12 5 15 6.0Q16 3 18 3.0Q22 3 20 8.0L21 13.0L19 18.0L15 21.0H9.0L5 18.0L3 13.0L4 8.0Z M6 10.0V12.0L9 13.0L10 11.0Z M14 11 15 13 18 12.0V10.0Z M10 16 12 18 14 16.0Z",
  lynx: "M4 1 9 6.0H15.0L20 1.0V9.0L22 12.0L20 14.0L22 16.0L18 17.0L16 20.0L12 22.0L8 20.0L6 17.0L2 16.0L4 14.0L2 12.0L4 9.0V1.0Z M6 10 7 12 9 13 10 11.0Z M14 11 15 13 17 12 18 10.0Z M10 16 12 18 14 16.0Z",
  jackal: "M4 2 9 5 13 2 14 9 18 11 22 12 21 15 17 17 16 21.0H7.0L8 17.0L4 14.0L5 9.0L4 2.0Z M11 11.0V13.0L13 14.0L15 12.0Z M17.5 12.8V14.5H19.0L20 12.8Z",
  boar: "M3 3 9 6.0H15.0L21 3.0L20 11.0L18 14.0V17.0Q21 16 22 12.0V19.0L17 22.0H7.0L2 19.0V12.0Q3 16 6 17.0V14.0L4 11.0L3 3.0Z M6 9.0V11.0L9 12.0L10 10.0Z M14 10 15 12 18 11.0V9.0Z M8 14 7 18.0Q12 22 17 18.0L16 14.0Q12 12 8 14.0Z M9 14.5Q11 14 11 16.0Q11 18 9 17.5V14.5Z M13 16.0Q13 14 15 14.5V17.5Q13 18 13 16.0Z",
  stag: "M3 2 5 5.0H7.0V2.0L9 4.0V8.0L11 11.0H13.0L15 8.0V4.0L17 2.0V5.0H19.0L21 2.0V7.0L18 9.0L17 12.0L22 10.0L20 15.0H16.0L15 20.0L12 22.0L9 20.0L8 15.0H4.0L2 10.0L7 12.0L6 9.0L3 7.0V2.0Z M9.2 13.0V15.0L11 16.0V14.0Z M13 14.0V16.0L14.8 15.0V13.0Z M10.5 18 12 20 13.5 18.0Z",
  rhino: "M3 5 8 6 11 4 17 5 19 10 20 5 22 13 20 18 16 21.0H6.0L7 17.0L3 14.0L4 10.0L3 5.0Z M12 9.0V11.0L15 12.0L16 10.0Z M16 16.0V18.0L19 17.0L20 15.0Z",
  bull: "M2 2 5 7 9 8 10 6.0H14.0L15 8.0L19 7.0L22 2.0V9.0L19 12.0L17 13.0V19.0L14 22.0H10.0L7 19.0V13.0L5 12.0L2 9.0V2.0Z M8 11.0V13.0L10 14.0L11 12.0Z M13 12 14 14 16 13.0V11.0Z M9 17 10 20.0H14.0L15 17.0Z",
  mantis: "M3 2.0H5.0L9 8.0H15.0L19 2.0H21.0L17 9.0L22 11.0L17 18.0L14 19.0L12 22.0L10 19.0L7 18.0L2 11.0L7 9.0L3 2.0Z M5 11 7 15 10 14 9 11.0Z M15 11 14 14 17 15 19 11.0Z M10.5 17 12 19 13.5 17.0Z",
  wasp: "M4 2.0H8.0L11 7.0H13.0L16 2.0H20.0V4.0H17.0L15 8.0L19 10.0L21 15.0L18 19.0L15 20.0L14 22.0L12 20.0L10 22.0L9 20.0L6 19.0L3 15.0L5 10.0L9 8.0L7 4.0H4.0V2.0Z M6 11.0V15.0L8 17.0L10 14.0L9 10.0Z M15 10 14 14 16 17 18 15.0V11.0Z M11 10.0V14.0H13.0V10.0Z",
  kraken: "M7 10.0Q5 2 12 2.0Q19 2 17 10.0L18 14.0Q20 16 20 12.0H22.0Q23 19 18 18.0L16 16.0L17 20.0L20 19.0L21 21.0Q15 24 14 18.0L13 16.0V21.0H11.0V16.0L10 18.0Q9 24 3 21.0L4 19.0L7 20.0L8 16.0L6 18.0Q1 19 2 12.0H4.0Q4 16 6 14.0L7 10.0Z M8 9.0V11.0L10 12.0L11 10.0Z M13 10 14 12 16 11.0V9.0Z",
  dragon: "M3 2 9 5 12 2.0V6.0H15.0L16 9.0H21.0L22 13.0L20 16.0H16.0L15 21.0L6 22.0L8 17.0L3 18.0L5 14.0L2 12.0L6 9.0L3 2.0Z M10 8.0V10.0L13 11.0L14 9.0Z M15 12.0V14.0L17 13.0V14.0H19.0L20 12.0Z",
  phoenix: "M10 2 14 5 17 4 16 7 20 8 22 11.0H19.0L18 14.0L15 16.0L18 17.0L15 22.0L11 20.0L7 21.0L8 18.0H3.0L5 15.0L2 12.0H5.0L2 6.0L7 9.0L6 3.0L10 6.0V2.0Z M12 8.0V10.0L15 11.0L16 9.0Z M7 13 9 16 8 17 10 19 11 15.0Z",
};
const LEGACY_FACES = {
  frog: "M3 15c0-2.5 1-4 3-5a3.6 3.6 0 1 1 5-3h2a3.6 3.6 0 1 1 5 3c2 1 3 2.5 3 5 0 3-4 5-9 5s-9-2-9-5zM5.92 7.2a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM14.72 7.2a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM7 16c2.5 2 7.5 2 10 0l-0.7-0.9c-2 1.6-6.6 1.6-8.6 0z",
  panda: "M4.5 13.5a7.5 7.5 0 1 0 15 0a7.5 7.5 0 1 0 -15 0zM1.8 5.2a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM16.2 5.2a3 3 0 1 0 6 0a3 3 0 1 0 -6 0zM7.3 11.5a1.8 2.4 0 0 1 3.4 0 1.8 2.4 0 0 1-3.4 2.6 1.8 2.4 0 0 1 0-2.6zM13.3 11.5a1.8 2.4 0 0 1 3.4 0 1.8 2.4 0 0 1 0 2.6 1.8 2.4 0 0 1-3.4 0 1.8 2.4 0 0 1 0-2.6zM10.8 15.5h2.4L12 16.8z",
  bunny: "M8 2c2 0 2.5 3 2.5 7h3C13.5 5 14 2 16 2c1.5 0 2 3 1.5 7 1.5 1.2 2.5 3 2.5 5.5 0 4-3.5 7.5-8 7.5s-8-3.5-8-7.5c0-2.5 1-4.3 2.5-5.5C6 5 6.5 2 8 2zM7.32 14a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM13.32 14a1.68 1.68 0 1 0 3.36 0a1.68 1.68 0 1 0 -3.36 0zM10.8 17h2.4L12 18.2z",
  penguin: "M12 1 C6 1 4 6 4 11 Q1 13 1 18 Q1 20 5 18 Q6 23 12 23 Q18 23 19 18 Q23 20 23 18 Q23 13 20 11 C20 6 18 1 12 1 Z M7 9 A1 1 0 1 0 9 9 A1 1 0 1 0 7 9 Z M15 9 A1 1 0 1 0 17 9 A1 1 0 1 0 15 9 Z M4 13 A1 1 0 1 0 6 13 A1 1 0 1 0 4 13 Z M18 13 A1 1 0 1 0 20 13 A1 1 0 1 0 18 13 Z M9 13 H15 V16 Q12 20 9 16 Z",
  koala: "M7 6 C5 0 0 2 1 8 Q0 14 5 15 Q5 23 12 23 Q19 23 19 15 Q24 14 23 8 C24 2 19 0 17 6 Q12 3 7 6 Z M6 9 A1 1 0 1 0 8 9 A1 1 0 1 0 6 9 Z M16 9 A1 1 0 1 0 18 9 A1 1 0 1 0 16 9 Z M12 10 Q14 10 14 12 V14 Q14 16 12 16 Q10 16 10 14 V12 Q10 10 12 10 Z M5 14 A1 1 0 1 0 7 14 A1 1 0 1 0 5 14 Z M17 14 A1 1 0 1 0 19 14 A1 1 0 1 0 17 14 Z M9 18 Q12 19 15 18 V20 Q12 21 9 20 Z",
  raccoon: "M2 2Q7 1 8 5Q12 3 16 5Q17 1 22 2V10Q23 13 22 17Q20 22 12 23Q4 22 2 17Q1 13 2 10ZM4 8H10V14H4ZM6 10H8V12H6ZM14 8H20V14H14ZM16 10H18V12H16ZM10 17H14V19L12 21L10 19Z",
  crab: "M7 11 L4 9 H1 V3 L3 1 V5 H5 V1 L7 3 V7 L9 10 H15 L17 7 V3 L19 1 V5 H21 V1 L23 3 V9 H20 L17 11 L20 13 L23 12 V15 L20 16 L23 19 V22 L18 19 Q12 23 6 19 L1 22 V19 L4 16 L1 15 V12 L4 13 Z M9 13 H15 V16 H9 Z",
};
const LEGACY_ROOM_FACES = {
  rocket: "M12 1.5 L16 6 L16 11.5 L20 16 L20 19.5 L15.5 17.5 L15.5 16 L8.5 16 L8.5 17.5 L4 19.5 L4 16 L8 11.5 L8 6 Z M9.5 18 L14.5 18 L13.7 20.5 L12 22.5 L10.3 20.5 Z M10 8 L10 10 L12 11.5 L14 10 L14 8 L12 6 Z",
  trophy: "M6 2.5 L18 2.5 L18 5 L22 5 L22 10 L19 13 L16.5 13 L14 15 L14 18 L18 18 L19 21.5 L5 21.5 L6 18 L10 18 L10 15 L7.5 13 L5 13 L2 10 L2 5 L6 5 Z M9.5 9.5 L12 11.5 L14.5 9.5 L15.5 5 L8.5 5 Z M4 9.2 L5.4 10.5 L6.7 10.5 L6 7 L4 7 Z M18 7 L17.3 10.5 L18.6 10.5 L20 9.2 L20 7 Z",
  mask: "M1.5 4 L7 5.5 L12 4 L17 5.5 L22.5 4 L21 14 L17.5 19 L12 22 L6.5 19 L3 14 Z M5 11 L6 12 L9 13 L10 10 L5 8.5 Z M14 10 L15 13 L18 12 L19 11 L19 8.5 Z M9 17 L12 18.5 L15 17 L13 15.5 L11 15.5 Z",
  peak: "M1.5 21 L8 9 L10 12 L15 2 L22.5 21 Z M12 15.5 L15 13.5 L18.1 15.5 L15 8 Z",
};
const FACE_ALIASES = {
  cat: "lynx",
  octopus: "kraken",
  whale: "orca",
  bolt: "lightning-bolt",
  swords: "crossed-swords",
  shield: "heater-shield",
  wings: "wing",
  chevrons: "chevron-stack",
  gem: "diamond",
};
const LEGACY_PATHS = new Set([...Object.values(LEGACY_FACES), ...Object.values(LEGACY_ROOM_FACES)]);
const fillRule = (d) => (LEGACY_PATHS.has(d) ? "evenodd" : "nonzero");
const PERSON = "person";
const PERSON_GLYPH = "M12 4.5a5 5 0 1 0 0 10a5 5 0 1 0 0-10zM-1 25c0-6 5-9.5 13-9.5s13 3.5 13 9.5z";
const PRESETS = Object.keys(FACES);
const ROOM_FACES = {
  "crossed-swords": "M2 2 7 3 17 15 19 13 21 15 19 17 22 20 20 22 17 19 15 21 13 19 15 17 3 7 2 2.0ZM22 2 21 7 9 17 11 19 9 21 7 19 4 22 2 20 5 17 3 15 5 13 7 15 17 3 22 2.0Z",
  crown: "M2 5 7.5 9 12 2 16.5 9 22 5 20 18.0H4.0L2 5.0ZM4 19.5H20.0V22.0H4.0V19.5Z",
  skull: "M5 3 9 1.5H15.0L19 3.0L21.5 8.0V14.0L18 17.0V21.0L15 22.5H9.0L6 21.0V17.0L2.5 14.0V8.0L5 3.0ZM5 9.0V13.0H9.0L10 10.0L5 9.0ZM14 10 15 13.0H19.0V9.0L14 10.0ZM12 14 10.5 17.0H13.5L12 14.0ZM9 19.0V22.5H10.5V19.0H9.0ZM13.5 19.0V22.5H15.0V19.0H13.5Z",
  "horned-helm": "M1.5 2 5 4.0V8.0L8 9.0L9 5.0L12 3.0L15 5.0L16 9.0L19 8.0V4.0L22.5 2.0L22 10.0L18 14.0V19.0L12 22.5L6 19.0V14.0L2 10.0L1.5 2.0ZM8.5 12.0V15.0L11 16.0V13.0L8.5 12.0ZM13 13.0V16.0L15.5 15.0V12.0L13 13.0Z",
  "lightning-bolt": "M12 1.5H21.0L15 9.0H21.0L8 22.5L10 14.0H3.0L12 1.5Z",
  flame: "M12 1.5C13 7 19 7 18 13.0L21 9.0C24 17 18 22.5 12 22.5C5 22.5 1 18 3 12.0L6 6.0L7 12.0C10 9 8 6 12 1.5ZM12 13.0C12 16 9 16 9 18.0C9 20 11 21 12 21.0C14 21 16 19 15 17.0L14 15.0L13 17.0L12 13.0Z",
  star: "M12 1.5 15 8 22.5 9 17 14.5 18.5 22 12 18.5 5.5 22 7 14.5 1.5 9 9 8 12 1.5Z",
  trident: "M12 1.5 15.5 7.0H13.5V13.0L17 10.5V6.0H15.5L19 2.5L22.5 6.0H20.5V12.5L13.5 17.0V22.5H10.5V17.0L3.5 12.5V6.0H1.5L5 2.5L8.5 6.0H7.0V10.5L10.5 13.0V7.0H8.5L12 1.5Z",
  "battle-axe": "M10.5 2.0H13.5V22.0H10.5V2.0ZM3 3 8 6.0H16.0L21 3.0L22 8.0L20 14.0L15 12.0H9.0L4 14.0L2 8.0L3 3.0Z",
  arrowhead: "M12 1.5 22.5 22 12 17 1.5 22 12 1.5ZM12 9 9.5 16 12 14.5 14.5 16 12 9.0Z",
  crosshair: "M20 12.0C20 16.418278 16.418278 20 12 20.0C7.581722 20 4 16.418278 4 12.0C4 7.581722 7.581722 4 12 4.0C16.418278 4 20 7.581722 20 12.0ZM16.5 12.0C16.5 9.514718625 14.485281375 7.5 12 7.5C9.514718625 7.5 7.5 9.514718625 7.5 12.0C7.5 14.485281375 9.514718625 16.5 12 16.5C14.485281375 16.5 16.5 14.485281375 16.5 12.0ZM10.5 1.5H13.5V7.0H10.5V1.5ZM10.5 17.0H13.5V22.5H10.5V17.0ZM1.5 10.5H7.0V13.5H1.5V10.5ZM17 10.5H22.5V13.5H17.0V10.5ZM10.5 10.5H13.5V13.5H10.5V10.5Z",
  wing: "M2 21.5V12.0L8 8.0L22.5 2.0L20 8.0L9 13.0L8.5 15.0L19 10.5L16.5 16.0L8 19.0L7.5 21.5H2.0Z",
  fang: "M6 1.5H19.0L18 8.0C17 14 14 19 8 22.5C10 17 11 13 9 9.0L6 7.0V1.5Z",
  "claw-marks": "M4 2 8 3 6 16 2 21 3 12 4 2.0ZM11 1.5 15 2.5 13 17.5 9 22.5 10 12 11 1.5ZM18 3 22 4 20 16 16 21 17 12 18 3.0Z",
  anvil: "M6 3.0H22.5V9.0H17.0L14 13.0V17.0L19 19.0L20 22.0H4.0L5 19.0L10 17.0V13.0L7 10.0L3 8.0L1.5 5.0H6.0V3.0Z",
  comet: "M21.5 1.5 16 12 13 18 9 21.5H5.0L2.5 19.0L1.5 15.0L3 11.0L8 8.0L17 3.0L12 10.0L14 9.0L21.5 1.5ZM6 12 4 15 5 18 8 19 11 17.0V14.0L9 12.0H6.0Z",
  eclipse: "M16 5.0C12 5 9 8 11 11.0C13 14 17 15 21 12.0C20.5 16 18 19 14 19.5C9 20 5 17 5.5 11.0C6 6 11 3 16 5.0ZM11.5 1.5H14.5V3.0H11.5V1.5ZM1.5 10.5H3.5V13.5H1.5V10.5ZM11.5 21.0H14.5V22.5H11.5V21.0ZM2.5 3.5 4 2 7 4.5 5 6.5 2.5 3.5ZM2.5 20 5 17.5 7 19.5 4 22 2.5 20.0Z",
  spade: "M12 1.5 21 11.0C25 17 18 22 13.5 17.0V19.0L17 22.5H7.0L10.5 19.0V17.0C6 22 -1 17 3 11.0L12 1.5Z",
  tower: "M5 2.0H8.0V5.0H10.5V2.0H13.5V5.0H16.0V2.0H19.0V10.0L17 12.0L18 19.0H21.0V22.0H3.0V19.0H6.0L7 12.0L5 10.0V2.0ZM10 22.0H14.0V16.0L12 14.0L10 16.0Z",
  banner: "M3 1.5H6.0V22.5H3.0V1.5ZM7.5 3.0H22.5L19 9.0L22.5 15.0H7.5V3.0Z",
  "chevron-stack": "M2 6.5 12 1.5 22 6.5V11.0L12 6.0L2 11.0V6.5ZM2 13 12 8 22 13.0V17.5L12 12.5L2 17.5V13.0ZM5 19 12 15.5 19 19.0V22.5L12 19.0L5 22.5V19.0Z",
  diamond: "M12 1.5 22.5 12 12 22.5 1.5 12 12 1.5ZM12 7 7 12 12 17 17 12 12 7.0Z",
  "sun-burst": "M12 1.5 14.5 5 18 3 18.5 7.5 22.5 8 20.5 12 22.5 16 18.5 16.5 18 21 14.5 19 12 22.5 9.5 19 6 21 5.5 16.5 1.5 16 3.5 12 1.5 8 5.5 7.5 6 3 9.5 5 12 1.5ZM15.5 12.0C15.5 10.067003375 13.932996625 8.5 12 8.5C10.067003375 8.5 8.5 10.067003375 8.5 12.0C8.5 13.932996625 10.067003375 15.5 12 15.5C13.932996625 15.5 15.5 13.932996625 15.5 12.0Z",
  "heater-shield": "M3 2.0H21.0V12.0L19 17.0L12 22.5L5 17.0L3 12.0V2.0ZM6 5.0V11.5L7.5 15.0L12 18.5L16.5 15.0L18 11.5V5.0H6.0Z",
};
const ROOM_PRESETS = Object.keys(ROOM_FACES);
const glyphOf = (current, legacy, name) => {
  if (!own(current, name) && !own(legacy, name) && own(FACE_ALIASES, name)) name = FACE_ALIASES[name];
  return (own(current, name) && current[name]) || (own(legacy, name) && legacy[name]) || null;
};
const faceGlyph = (name) => glyphOf(FACES, LEGACY_FACES, name);
const roomGlyph = (name) => glyphOf(ROOM_FACES, LEGACY_ROOM_FACES, name);
const IMAGE_SIDE = 96;
const IMAGE_QUALITY = 0.85;

function initial(name) {
  return ([...String(name || "?").trim()][0] || "?").toUpperCase();
}

const IMAGE_URL = /^data:image\/(png|jpeg|webp);base64,/;
const own = (table, name) => typeof name === "string" && Object.hasOwn(table, name);
const knownIcon = (icon) => IMAGE_URL.test(icon) || !!(roomGlyph(icon) || faceGlyph(icon));

function picture(label, color, image, size, shape) {
  const room = shape === "room";
  const photo = IMAGE_URL.test(image);
  const glyph = (room && roomGlyph(image)) || faceGlyph(image) || (!room && !photo && PERSON_GLYPH);
  const shown = photo ? el("img", { src: image, alt: "" }) : glyph ? svg("svg", { viewBox: "0 0 24 24", "aria-hidden": "true" }, svg("path", { d: glyph, "fill-rule": fillRule(glyph) })) : null;
  const a = el("span", { className: ["av", size, shape, glyph === PERSON_GLYPH && "person"].filter(Boolean).join(" "), title: label }, shown || initial(label));
  const base = color || "var(--muted)";
  a.style.background = glyph === PERSON_GLYPH ? `color-mix(in srgb, ${base} 35%, var(--raised-3))` : base;
  if (glyph === PERSON_GLYPH) a.style.color = base;
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
  g.fill(path, "nonzero");
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
