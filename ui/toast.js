const note = new URLSearchParams(location.search);
const joiner = note.get("id");
document.body.style.animationDuration = `${note.get("ms")}ms`;
$("text").textContent = note.get("text");
invoke("state").then((s) => $("face").replaceChildren(picture(s.names[joiner], colorOf(s.colors[joiner]), s.avatars[joiner], "xl"))).catch(warn);
document.body.onclick = () => dialog("show_main").catch(warn);
