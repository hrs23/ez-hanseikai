let feedback = { kind: "bug", shown: null };

function renderFeedback() {
  pick($("fbkind"), feedback.kind, (v) => { feedback.kind = v; renderFeedback(); });
  const shown = feedback.shown;
  const host = shown && shown.to;
  $("fbimage").disabled = !(shown && shown.image);
  if ($("fbimage").disabled) $("fbimage").checked = false;
  $("fbimg").hidden = !$("fbimage").checked;
  if (shown && shown.image) $("fbimg").src = shown.image;
  $("fbtext").textContent = shown ? feedbackPreview(feedback.kind, $("fbcomment").value, $("fbdiag").checked ? shown.diagnostics : null) : t("Preparing…");
  $("fbnote").textContent = host ? t("Sent only to {host}, only when you press Send. Recordings, notes and game screens are never sent, and there is no reply.", { host }) : shown ? t("Feedback is not set up in this build.") : "";
  $("fbsend").disabled = !host || !feedbackReady($("fbcomment").value, $("fbdiag").checked);
}

async function openFeedback(kind = "bug", comment = "") {
  feedback = { kind: Object.hasOwn(FEEDBACK_NAMES, kind) ? kind : "bug", shown: null };
  $("fbcomment").value = comment;
  $("fbdiag").checked = true;
  $("fbimage").checked = false;
  $("fbpreview").open = false;
  renderFeedback();
  const ask = dialog("feedback_preview");
  if (!$("feedbackdlg").open) $("feedbackdlg").showModal();
  $("fbcomment").focus();
  try { feedback.shown = await ask; } catch (e) { feedback.shown = { diagnostics: "", image: null, to: null }; fail(e); }
  renderFeedback();
}

$("fbcomment").maxLength = LIMITS.feedback;
$("fbcomment").oninput = $("fbdiag").onchange = $("fbimage").onchange = renderFeedback;
$("fbsend").onclick = single(async () => {
  $("fbsend").disabled = true;
  try {
    await dialog("send_feedback", { args: { kind: feedback.kind, comment: $("fbcomment").value, diagnostics: $("fbdiag").checked, image: $("fbimage").checked } });
    $("feedbackdlg").close();
    flash(t("Thanks, sent"));
  } catch (e) {
    flash(e, "error");
    renderFeedback();
  }
});
menuActions.feedback = () => openFeedback();
