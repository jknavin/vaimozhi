/* VAIMOZHI UI. Plain JavaScript; talks to the Rust backend through window.__TAURI__ (withGlobalTauri). */
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const dialog = window.__TAURI__.dialog;
const opener = window.__TAURI__.opener;

const $ = (id) => document.getElementById(id);
const STYLES = ["raw", "clean", "professional", "formal", "casual", "email"];
const STYLE_LABELS = {
  en: { raw: "Raw transcription", clean: "Clean transcription", professional: "Professional writing", formal: "Formal office language", casual: "Casual messaging", email: "Email formatting" },
  ta: { raw: "அப்படியே", clean: "சீரான உரை", professional: "தொழில்முறை எழுத்து", formal: "அலுவலக முறை", casual: "சாதாரண செய்தி", email: "மின்னஞ்சல் வடிவம்" },
};
const LANG_HELP = {
  tamil: "Tamil speech, written in Tamil script. Most accurate when you know the language is Tamil.",
  english: "English speech, written in English. Most accurate when you know the language is English.",
  auto: "Whisper picks Tamil or English for each stretch of speech (no other languages). Choose a fixed language when you can: it is more accurate.",
};

let settings = null;
let device = null;
let models = [];
let currentId = null;           // transcript id loaded in the editor (null = unsaved)
let editorMeta = { raw: "", language: "", model: "", style: "", duration: 0 };
let uiBusy = false;
let recStartedHere = false;
const downloading = {};         // model id -> {done,total}

/* ------------------------------------------------------------------ helpers */
function t(key, fallback) {
  const lang = settings ? settings.ui_language : "en";
  return (window.I18N[lang] && window.I18N[lang][key]) || fallback || key;
}
function toast(msg, isError) {
  const el = document.createElement("div");
  el.className = "toast" + (isError ? " error" : "");
  el.textContent = msg;
  $("toasts").appendChild(el);
  setTimeout(() => el.remove(), isError ? 7000 : 3500);
}
async function call(cmd, args) {
  try { return await invoke(cmd, args); }
  catch (e) { toast(String(e), true); throw e; }
}
function fmtBytes(b) {
  if (b >= 1073741824) return (b / 1073741824).toFixed(2) + " GB";
  if (b >= 1048576) return (b / 1048576).toFixed(1) + " MB";
  if (b >= 1024) return (b / 1024).toFixed(0) + " KB";
  return b + " B";
}
function fmtMb(mb) { return mb >= 1024 ? (mb / 1024).toFixed(1) + " GB" : mb + " MB"; }
function fmtDate(ms) { return new Date(ms).toLocaleString(); }
function fmtTime(ms) { const s = Math.floor(ms / 1000); return String(Math.floor(s / 60)).padStart(2, "0") + ":" + String(s % 60).padStart(2, "0"); }
function esc(s) { const d = document.createElement("div"); d.textContent = s; return d.innerHTML; }
function titleOf(tr) { return tr.title || (tr.text.split("\n")[0] || "").slice(0, 60) || "(empty)"; }

function applyI18n() {
  document.documentElement.lang = settings.ui_language;
  document.querySelectorAll("[data-i18n]").forEach((el) => {
    if (!el.dataset.en) el.dataset.en = el.textContent;
    el.textContent = t(el.dataset.i18n, el.dataset.en);
  });
  document.querySelectorAll("[data-i18n-ph]").forEach((el) => {
    if (!el.dataset.en) el.dataset.en = el.placeholder;
    el.placeholder = t(el.dataset.i18nPh, el.dataset.en);
  });
  fillStyleSelects();
  refreshChips();
}
function fillStyleSelects() {
  const labels = STYLE_LABELS[settings.ui_language] || STYLE_LABELS.en;
  for (const id of ["dash-style", "s-style", "ed-style"]) {
    const sel = $(id), cur = sel.value;
    sel.innerHTML = STYLES.map((s) => `<option value="${s}">${labels[s]}</option>`).join("");
    sel.value = cur || settings.style;
  }
}
function applyTheme() {
  let th = settings.theme;
  if (th === "system") th = window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
  document.body.dataset.theme = th;
}
function refreshChips() {
  const langName = t("lang." + settings.language, settings.language.charAt(0).toUpperCase() + settings.language.slice(1));
  $("chip-language").textContent = "🌐 " + langName;
  const m = models.find((x) => x.id === settings.model_id);
  $("chip-model").textContent = "🧠 " + (m ? m.name : "No model selected");
  $("hotkey-hint").textContent = settings.shortcut;
}

/* ------------------------------------------------------------------ navigation */
function show(view) {
  document.querySelectorAll(".view").forEach((v) => v.classList.toggle("active", v.id === "view-" + view));
  document.querySelectorAll("#nav button").forEach((b) => b.classList.toggle("active", b.dataset.view === view));
  if (view === "dashboard") loadDashboard();
  if (view === "history") loadHistory();
  if (view === "models") loadModels();
  if (view === "settings") loadSettingsView();
}
$("nav").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) show(b.dataset.view); });
$("chip-language").onclick = () => show("settings");
$("chip-model").onclick = () => show("models");

/* ------------------------------------------------------------------ settings load/save */
async function persist(patch) {
  const next = Object.assign({}, settings, patch);
  settings = await call("save_settings", { settings: next });
  applyTheme(); applyI18n();
  return settings;
}

async function loadSettingsView() {
  const s = settings;
  $("s-language").value = s.language; updateLangHelp();
  $("s-beam").checked = s.beam_search; $("s-threads").value = s.threads;
  $("s-style").value = s.style; $("s-fillers").checked = s.remove_fillers; $("s-numbers").checked = s.spoken_numbers;
  $("s-insert").value = s.insert_method; $("s-restore").checked = s.restore_clipboard;
  $("s-shortcut").value = s.shortcut; $("s-hotmode").value = s.hotkey_mode;
  $("s-history").checked = s.history_enabled; $("s-keepaudio").checked = s.keep_audio;
  $("s-theme").value = s.theme; $("s-uilang").value = s.ui_language;
  $("s-autostart").checked = s.autostart; $("s-hidden").checked = s.start_hidden;
  $("s-tray").checked = s.minimize_to_tray; $("s-overlay").checked = s.show_overlay; $("s-unload").value = s.unload_after_min;

  const mics = await call("list_microphones");
  $("s-mic").innerHTML = `<option value="">System default</option>` + mics.map((m) => `<option>${esc(m)}</option>`).join("");
  $("s-mic").value = mics.includes(s.microphone) ? s.microphone : "";

  models = await call("list_models");
  $("s-model").innerHTML = `<option value="">(none)</option>` +
    models.filter((m) => m.installed).map((m) => `<option value="${m.id}">${esc(m.name)} (${fmtMb(m.size_mb)})</option>`).join("");
  $("s-model").value = s.model_id;

  device = await call("device_info");
  $("storage-info").innerHTML =
    `Data folder: <code>${esc(device.data_dir)}</code><br>Models: ${fmtBytes(device.models_bytes)} · History database: ${fmtBytes(device.history_bytes)} · Stored audio: ${fmtBytes(device.audio_bytes)}` +
    (device.free_disk_mb != null ? `<br>Free disk space: ${fmtMb(device.free_disk_mb)}` : "");
  $("about-info").textContent = `VAIMOZHI v${device.version} · ${device.cores} CPU threads · ${fmtMb(device.ram_mb)} RAM`;
}
function updateLangHelp() { $("s-language-help").textContent = LANG_HELP[$("s-language").value] || ""; }
$("s-language").onchange = updateLangHelp;

$("save-settings").onclick = async () => {
  const patch = {
    language: $("s-language").value, model_id: $("s-model").value, microphone: $("s-mic").value,
    beam_search: $("s-beam").checked, threads: Math.max(0, parseInt($("s-threads").value || "0", 10)),
    style: $("s-style").value, remove_fillers: $("s-fillers").checked, spoken_numbers: $("s-numbers").checked,
    insert_method: $("s-insert").value, restore_clipboard: $("s-restore").checked,
    shortcut: $("s-shortcut").value || settings.shortcut, hotkey_mode: $("s-hotmode").value,
    history_enabled: $("s-history").checked, keep_audio: $("s-keepaudio").checked,
    theme: $("s-theme").value, ui_language: $("s-uilang").value,
    autostart: $("s-autostart").checked, start_hidden: $("s-hidden").checked,
    minimize_to_tray: $("s-tray").checked, show_overlay: $("s-overlay").checked,
    unload_after_min: Math.max(0, parseInt($("s-unload").value || "0", 10)),
  };
  try { await persist(patch); toast("Settings saved"); } catch (_) { /* toast shown by call() */ }
};

// Shortcut capture
$("s-shortcut").addEventListener("keydown", (e) => {
  e.preventDefault();
  if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
  const mods = [];
  if (e.ctrlKey) mods.push("Ctrl"); if (e.altKey) mods.push("Alt"); if (e.shiftKey) mods.push("Shift"); if (e.metaKey) mods.push("Super");
  if (!mods.length) { toast("Add at least one modifier key (Ctrl, Alt or Shift).", true); return; }
  let key = e.code.replace(/^Key/, "").replace(/^Digit/, "").replace(/^Arrow/, "");
  if (key === "Backquote") key = "`";
  $("s-shortcut").value = [...mods, key].join("+");
});

$("mic-test").onclick = async () => {
  $("mic-test-result").textContent = "Listening… say something";
  // Use the mic chosen in the form even if not yet saved.
  if ($("s-mic").value !== settings.microphone) await persist({ microphone: $("s-mic").value });
  const r = await call("test_microphone");
  $("mic-test-result").textContent = (r.ok ? "✅ " : "⚠️ ") + r.message;
};
$("clear-history").onclick = async () => {
  if (!(await dialog.confirm("Delete ALL saved transcriptions and any stored audio? This cannot be undone.", { title: "VAIMOZHI", kind: "warning" }))) return;
  await call("clear_history"); toast("History deleted"); loadDashboard();
};
$("check-updates").onclick = async () => {
  const url = (window.VAIMOZHI_CONFIG || {}).releasesUrl;
  if (!url) { toast("No update page is configured in this build. Set releasesUrl in src/config.js.", true); return; }
  await opener.openUrl(url);
};

/* ------------------------------------------------------------------ dashboard */
async function loadDashboard() {
  const [stats, list] = await Promise.all([call("get_stats"), call("list_history", { limit: 5 })]);
  $("stat-words").textContent = stats.words.toLocaleString();
  $("stat-count").textContent = stats.count.toLocaleString();
  $("dash-language").value = settings.language; $("dash-style").value = settings.style;
  const box = $("recent-list");
  box.innerHTML = list.length ? "" : `<div class="empty">No transcriptions yet. Press the microphone or ${esc(settings.shortcut)}.</div>`;
  list.forEach((tr) => {
    const d = document.createElement("div"); d.className = "item";
    d.innerHTML = `<div class="meta">${fmtDate(tr.created_at)} · ${tr.words} words</div><div class="text">${esc(tr.text)}</div>`;
    d.querySelector(".text").onclick = () => openInEditor(tr);
    box.appendChild(d);
  });
  models = await call("list_models"); refreshChips();
}
$("dash-language").onchange = (e) => persist({ language: e.target.value });
$("dash-style").onchange = (e) => persist({ style: e.target.value });

/* ------------------------------------------------------------------ recording screen */
const wave = $("wave"), wctx = wave.getContext("2d");
const bars = new Array(64).fill(0.03);
let level = 0, recActive = false, paused = false;
function drawWave() {
  if (recActive) {
    bars.shift(); bars.push(paused ? 0.03 : Math.max(0.03, level * (0.75 + Math.random() * 0.5)));
    const w = wave.width, h = wave.height, bw = w / bars.length;
    wctx.clearRect(0, 0, w, h);
    const g = wctx.createLinearGradient(0, 0, w, 0); g.addColorStop(0, "#6558E8"); g.addColorStop(1, "#20C9B7");
    wctx.fillStyle = g;
    bars.forEach((v, i) => { const bh = Math.max(4, Math.min(1, v) * h); wctx.fillRect(i * bw + 2, (h - bh) / 2, bw - 4, bh); });
  }
  requestAnimationFrame(drawWave);
}
drawWave();

function setRecModal(open, mode) {
  $("rec-modal").classList.toggle("hidden", !open);
  recActive = open && mode === "recording";
  $("rec-actions").classList.toggle("hidden", mode !== "recording");
  $("rec-progress").classList.toggle("hidden", mode === "recording" || !open);
  $("rec-state").className = "rec-state" + (mode === "recording" ? "" : " working");
  const label = { recording: t("status.recording", "Recording"), transcribing: t("status.transcribing", "Transcribing…"), loading: t("status.loading", "Loading model…") }[mode];
  $("rec-state").textContent = label || "";
}
$("mic-btn").onclick = async () => {
  if (uiBusy) return;
  try { await invoke("start_recording"); recStartedHere = true; } catch (e) { setStatus(String(e), true); }
};
$("rec-stop").onclick = () => invoke("stop_recording");
$("rec-cancel").onclick = () => { invoke("cancel_recording"); setRecModal(false); };
$("rec-pause").onclick = async () => {
  paused = !paused; await invoke(paused ? "pause_recording" : "resume_recording");
  $("rec-pause").textContent = paused ? t("rec.resume", "Resume") : t("rec.pause", "Pause");
};
document.addEventListener("keydown", (e) => { if (e.key === "Escape" && !$("rec-modal").classList.contains("hidden") && recActive) $("rec-cancel").click(); });

function setStatus(text, isError) {
  const el = $("status-line"); el.textContent = text; el.classList.toggle("error", !!isError);
}

listen("audio-level", (e) => { level = e.payload.level; $("rec-timer").textContent = fmtTime(e.payload.elapsed_ms); });
listen("transcribe-progress", (e) => {
  const bar = $("rec-progress"); bar.classList.remove("indeterminate"); bar.firstElementChild.style.width = Math.round(e.payload * 100) + "%";
});
listen("status", (e) => {
  const { state, message, target } = e.payload;
  const mic = $("mic-btn");
  mic.classList.toggle("recording", state === "recording" || state === "paused");
  uiBusy = state === "transcribing" || state === "loading-model";
  mic.classList.toggle("busy", uiBusy);
  mic.setAttribute("aria-label", state === "recording" ? "Stop recording" : "Start recording");
  if (state === "recording") {
    paused = false; $("rec-pause").textContent = t("rec.pause", "Pause"); $("rec-timer").textContent = "00:00";
    setStatus(t("status.recording", "Recording…"));
    if (target === "editor") setRecModal(true, "recording");
    mic.onclick = () => invoke("stop_recording");
  } else if (state === "transcribing" || state === "loading-model") {
    setStatus(state === "loading-model" ? t("status.loading", "Loading model…") : t("status.transcribing", "Transcribing…"));
    mic.onclick = null;
    if (target === "editor") { const p = $("rec-progress"); p.classList.add("indeterminate"); p.firstElementChild.style.width = ""; setRecModal(true, state === "loading-model" ? "loading" : "transcribing"); }
  } else if (state === "idle") {
    setStatus(t("status.ready", "Ready")); setRecModal(false);
    mic.onclick = async () => { if (!uiBusy) { try { await invoke("start_recording"); } catch (err) { setStatus(String(err), true); } } };
  } else if (state === "error" || state === "no-model") {
    setStatus(message || "Error", true); setRecModal(false); toast(message || "Error", true);
    mic.onclick = async () => { if (!uiBusy) { try { await invoke("start_recording"); } catch (err) { setStatus(String(err), true); } } };
    if (state === "no-model") show("models");
  }
});
listen("dictation-result", (e) => {
  const r = e.payload;
  if (r.target === "editor") {
    currentId = r.id;
    editorMeta = { raw: r.raw_text, language: r.language, model: r.model, style: r.style, duration: r.duration_ms };
    setEditorText(r.text); show("editor"); updateEditorMeta();
  } else {
    toast(`Inserted ${r.text.split(/\s+/).filter(Boolean).length} words`);
    if ($("view-dashboard").classList.contains("active")) loadDashboard();
  }
});

/* ------------------------------------------------------------------ editor (undo/redo kept in JS so it survives programmatic changes) */
const ed = $("editor-text");
let undoStack = [], redoStack = [], lastVal = "";
function setEditorText(v) { ed.value = v; undoStack = []; redoStack = []; lastVal = v; }
ed.addEventListener("input", () => {
  if (ed.value !== lastVal) { undoStack.push(lastVal); if (undoStack.length > 200) undoStack.shift(); redoStack = []; lastVal = ed.value; }
});
function applyEditorChange(v) { undoStack.push(lastVal); redoStack = []; ed.value = v; lastVal = v; }
$("ed-undo").onclick = () => { if (!undoStack.length) return; redoStack.push(lastVal); lastVal = undoStack.pop(); ed.value = lastVal; };
$("ed-redo").onclick = () => { if (!redoStack.length) return; undoStack.push(lastVal); lastVal = redoStack.pop(); ed.value = lastVal; };
function updateEditorMeta() {
  const bits = [];
  if (currentId) bits.push("Saved #" + currentId); else bits.push("Not saved yet");
  if (editorMeta.language) bits.push(editorMeta.language);
  if (editorMeta.model) bits.push(editorMeta.model);
  $("editor-meta").textContent = bits.join(" · ");
}
function openInEditor(tr) {
  currentId = tr.id;
  editorMeta = { raw: tr.raw_text, language: tr.language, model: tr.model, style: tr.style, duration: tr.duration_ms };
  setEditorText(tr.text); $("ed-language").value = tr.language || settings.language; $("ed-style").value = settings.style;
  $("retry-note").textContent = tr.has_audio ? "" : "Retry needs stored audio. Turn on Settings → Privacy → Keep audio before dictating.";
  show("editor"); updateEditorMeta();
}
$("ed-copy").onclick = async () => { try { await navigator.clipboard.writeText(ed.value); toast("Copied"); } catch (_) { ed.select(); document.execCommand("copy"); toast("Copied"); } };
$("ed-share").onclick = async () => {
  if (navigator.share) { try { await navigator.share({ text: ed.value }); return; } catch (_) { return; } }
  try { await navigator.clipboard.writeText(ed.value); toast("Sharing is not available on this PC; text copied instead."); } catch (_) { toast("Sharing is not available.", true); }
};
$("ed-save").onclick = async () => {
  if (!ed.value.trim()) { toast("Nothing to save.", true); return; }
  if (currentId) await call("update_transcript", { id: currentId, text: ed.value });
  else currentId = await call("save_transcript", { text: ed.value, rawText: editorMeta.raw || ed.value, language: editorMeta.language || settings.language, model: editorMeta.model || settings.model_id, style: editorMeta.style || settings.style, durationMs: editorMeta.duration || 0 });
  updateEditorMeta(); toast("Saved");
};
$("ed-delete").onclick = async () => {
  if (currentId && !(await dialog.confirm("Delete this transcription?", { title: "VAIMOZHI", kind: "warning" }))) return;
  if (currentId) await call("delete_transcript", { id: currentId });
  currentId = null; setEditorText(""); editorMeta = { raw: "", language: "", model: "", style: "", duration: 0 }; updateEditorMeta(); toast("Deleted");
};
$("ed-clean").onclick = async () => {
  const out = await call("clean_text", { text: ed.value, style: $("ed-style").value });
  applyEditorChange(out);
};
$("ed-retry").onclick = async () => {
  if (!currentId) { toast("Save or open a stored transcription first.", true); return; }
  try { await invoke("retranscribe", { id: currentId, language: $("ed-language").value }); } catch (e) { toast(String(e), true); }
};

/* ------------------------------------------------------------------ history */
let hTimer = null;
["h-search", "h-from", "h-to", "h-fav"].forEach((id) => $(id).addEventListener("input", () => { clearTimeout(hTimer); hTimer = setTimeout(loadHistory, 200); }));
async function loadHistory() {
  const from = $("h-from").value ? new Date($("h-from").value + "T00:00:00").getTime() : null;
  const to = $("h-to").value ? new Date($("h-to").value + "T23:59:59").getTime() : null;
  const list = await call("list_history", { query: $("h-search").value, fromMs: from, toMs: to, favoritesOnly: $("h-fav").checked, limit: 300 });
  const box = $("history-list");
  box.innerHTML = list.length ? "" : `<div class="empty">Nothing found.</div>`;
  list.forEach((tr) => {
    const d = document.createElement("div"); d.className = "item";
    d.innerHTML = `
      <div class="top"><button class="star ${tr.favorite ? "on" : ""}" title="Favourite">${tr.favorite ? "★" : "☆"}</button>
        <div class="title">${esc(titleOf(tr))}</div><span class="badge">${esc(tr.language || "")}</span></div>
      <div class="meta">${fmtDate(tr.created_at)} · ${tr.words} words · ${esc(tr.model)}</div>
      <div class="text">${esc(tr.text)}</div>
      <div class="actions">
        <button class="btn small" data-a="open">Open</button><button class="btn small" data-a="copy">Copy</button>
        <button class="btn small" data-a="rename">Rename</button><button class="btn small" data-a="txt">Export TXT</button>
        <button class="btn small" data-a="docx">Export DOCX</button><button class="btn small" data-a="pdf">Export PDF</button>
        <button class="btn small danger" data-a="del">Delete</button></div>`;
    d.querySelector(".text").onclick = () => openInEditor(tr);
    d.querySelector(".star").onclick = async () => { await call("set_favorite", { id: tr.id, favorite: !tr.favorite }); loadHistory(); };
    d.querySelector(".actions").onclick = async (e) => {
      const a = e.target.dataset.a; if (!a) return;
      if (a === "open") openInEditor(tr);
      else if (a === "copy") { await navigator.clipboard.writeText(tr.text); toast("Copied"); }
      else if (a === "rename") {
        const name = window.prompt("Title for this transcription:", tr.title || "");
        if (name !== null) { await call("rename_transcript", { id: tr.id, title: name.trim() }); loadHistory(); }
      } else if (a === "del") {
        if (await dialog.confirm("Delete this transcription?", { title: "VAIMOZHI", kind: "warning" })) { await call("delete_transcript", { id: tr.id }); loadHistory(); }
      } else if (a === "txt" || a === "docx") {
        const path = await dialog.save({ defaultPath: (titleOf(tr).replace(/[\\/:*?"<>|]/g, "").slice(0, 40) || "transcription") + "." + a, filters: [{ name: a.toUpperCase(), extensions: [a] }] });
        if (path) { await call("export_transcript", { id: tr.id, format: a, path }); toast("Exported"); }
      } else if (a === "pdf") exportPdf(tr);
    };
    box.appendChild(d);
  });
}
// PDF: the browser engine shapes Tamil correctly, so use the print dialog's "Save as PDF".
function exportPdf(tr) {
  $("print-area").innerHTML = (tr.title ? `<h1>${esc(tr.title)}</h1>` : "") + esc(tr.text);
  toast("In the print window choose “Save as PDF”.");
  setTimeout(() => window.print(), 300);
}

/* ------------------------------------------------------------------ models */
async function loadModels() {
  [device, models] = await Promise.all([call("device_info"), call("list_models")]);
  const rec = models.find((m) => m.recommended);
  $("device-card").innerHTML =
    `<h2>This PC</h2><div>${fmtMb(device.ram_mb)} RAM · ${device.cores} CPU threads${device.free_disk_mb != null ? " · " + fmtMb(device.free_disk_mb) + " free disk" : ""}</div>
     <div class="muted small" style="margin-top:6px">Recommended: <b>${rec ? esc(rec.name) : ""}</b>. For accurate Tamil use Medium or larger; Large v3 is the most accurate but slow on CPU. Tiny, Base and Small make many Tamil mistakes. Larger models need the RAM shown.</div>`;
  renderModels();
}
function renderModels() {
  const box = $("model-list"); box.innerHTML = "";
  models.forEach((m) => {
    const dl = downloading[m.id];
    const lowRam = device && device.ram_mb && m.ram_mb > device.ram_mb * 0.7;
    const d = document.createElement("div"); d.className = "item";
    const active = settings.model_id === m.id;
    d.innerHTML = `
      <div class="top"><div class="title">${esc(m.name)}</div>
        ${m.recommended ? '<span class="badge">Recommended</span>' : ""}${active ? '<span class="badge">Active</span>' : ""}${lowRam ? '<span class="badge warn">May be too large</span>' : ""}</div>
      <div class="meta">Download ${fmtMb(m.size_mb)} · needs about ${fmtMb(m.ram_mb)} RAM while transcribing</div>
      <div>${esc(m.note)}</div>
      ${dl ? `<div class="bar"><i style="width:${dl.total ? Math.round((dl.done / dl.total) * 100) : 0}%"></i></div><div class="meta">${fmtBytes(dl.done)} / ${dl.total ? fmtBytes(dl.total) : "?"}</div>` : ""}
      <div class="actions"></div>`;
    const act = d.querySelector(".actions");
    const add = (label, cls, fn) => { const b = document.createElement("button"); b.className = "btn small " + cls; b.textContent = label; b.onclick = fn; act.appendChild(b); };
    if (dl) add("Pause download", "", () => invoke("cancel_download", { id: m.id }));
    else if (m.installed) {
      if (!active) add("Use this model", "primary", async () => { await persist({ model_id: m.id }); renderModels(); loadDashboard(); toast("Active model: " + m.name); });
      add("Verify", "", async () => { const r = await call("verify_model", { id: m.id }); toast((r.ok ? "✅ " : "❌ ") + r.message, !r.ok); });
      add("Delete", "danger", async () => {
        if (!(await dialog.confirm(`Delete ${m.name} from this PC? You can download it again later.`, { title: "VAIMOZHI", kind: "warning" }))) return;
        await call("delete_model", { id: m.id });
        if (settings.model_id === m.id) await persist({ model_id: "" });
        loadModels();
      });
    } else {
      const label = m.has_partial ? "Resume download" : `Download (${fmtMb(m.size_mb)})`;
      add(label, "primary", async () => {
        if (!(await dialog.confirm(`Download ${m.name}?\n\nSize: ${fmtMb(m.size_mb)}. It will be stored on this PC and used offline.`, { title: "VAIMOZHI" }))) return;
        downloading[m.id] = { done: 0, total: 0 }; renderModels();
        try { await invoke("download_model", { id: m.id }); } catch (e) { delete downloading[m.id]; toast(String(e), true); renderModels(); }
      });
    }
    box.appendChild(d);
  });
}
listen("model-progress", async (e) => {
  const p = e.payload;
  if (p.state === "downloading") { downloading[p.id] = { done: p.done, total: p.total }; if ($("view-models").classList.contains("active")) renderModels(); return; }
  delete downloading[p.id];
  toast(p.message || p.state, p.state === "error");
  if (p.state === "done" && !settings.model_id) await persist({ model_id: p.id });
  if ($("view-models").classList.contains("active")) loadModels();
});

/* ------------------------------------------------------------------ init */
(async function init() {
  settings = await invoke("get_settings");
  if (!["tamil", "english", "auto"].includes(settings.language)) settings.language = "auto";
  models = await invoke("list_models");
  applyTheme(); applyI18n();
  window.matchMedia("(prefers-color-scheme: light)").addEventListener("change", applyTheme);
  $("mic-btn").onclick = async () => { if (!uiBusy) { try { await invoke("start_recording"); } catch (err) { setStatus(String(err), true); } } };
  setStatus(t("status.ready", "Ready"));
  updateEditorMeta();
  show(models.some((m) => m.installed) ? "dashboard" : "models");
})();
