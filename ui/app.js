"use strict";

const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const fmtNum = (n) => Number(n || 0).toLocaleString();

function fmtBytes(n) {
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  n = Number(n || 0);
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${n.toFixed(n < 10 && i ? 1 : 0)} ${u[i]}`;
}
/** Lengths: sound effects are often well under a second, so short ones get decimals. */
function fmtDur(s) {
  s = Math.max(0, s || 0);
  if (s < 10) return `${s.toFixed(2)} s`;
  if (s < 60) return `${s.toFixed(1)} s`;
  s = Math.round(s);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}` : `${m}:${String(sec).padStart(2, "0")}`;
}
function fmtLong(s) {
  s = Math.round(s || 0);
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60);
  return h ? `${h} h ${m} min` : m ? `${m} min` : `${s} s`;
}

function toast(text, kind = "") {
  const el = document.createElement("div");
  el.className = `toast ${kind}`;
  el.textContent = text;
  $("toasts").appendChild(el);
  setTimeout(() => { el.classList.add("out"); setTimeout(() => el.remove(), 300); }, kind === "err" ? 7000 : 4000);
}

/* ---------- bridge to Rust ---------- */
const pending = new Map();
let seq = 1;
function call(cmd, args = {}) {
  return new Promise((resolve, reject) => {
    const id = seq++;
    pending.set(id, { resolve, reject });
    window.ipc.postMessage(JSON.stringify({ id, cmd, args }));
  });
}
window.__rx = (msg) => {
  if (msg.kind === "reply") {
    const p = pending.get(msg.id);
    if (!p) return;
    pending.delete(msg.id);
    msg.ok ? p.resolve(msg.data) : p.reject(new Error(msg.error || "Something went wrong"));
  } else if (msg.kind === "event") {
    onEvent(msg.data);
  }
};

/* ---------- title bar ---------- */
$("tbDrag").addEventListener("mousedown", (e) => {
  if (e.button === 0 && e.detail === 1) call("win_drag");
});
$("tbDrag").addEventListener("dblclick", () => call("win_maximize"));
$("winMin").addEventListener("click", () => call("win_minimize"));
$("winMax").addEventListener("click", () => call("win_maximize"));
$("winClose").addEventListener("click", () => call("win_close"));
function setMaximized(on) {
  $("winMax").title = on ? "Restore" : "Maximize";
  $("maxIcon").innerHTML = on
    ? '<rect x="0.5" y="2.5" width="7" height="7"/><path d="M2.5 2.5v-2h7v7h-2"/>'
    : '<rect x="0.5" y="0.5" width="9" height="9"/>';
}

/* ---------- spinning disc ---------- */
// Spins slowly, and speeds up (smoothly) while hovered or while something is dragged over.
const disc = $("disc").animate([{ transform: "rotate(0deg)" }, { transform: "rotate(360deg)" }], {
  duration: 6000,
  iterations: Infinity,
});
let discRate = 1, discTarget = 1, discEasing = false;
function spinDisc(rate) {
  discTarget = rate;
  if (discEasing) return;
  discEasing = true;
  const step = () => {
    discRate += (discTarget - discRate) * 0.08;
    if (Math.abs(discTarget - discRate) < 0.01) discRate = discTarget;
    disc.playbackRate = discRate;
    if (discRate !== discTarget) requestAnimationFrame(step);
    else discEasing = false;
  };
  requestAnimationFrame(step);
}
$("drop").addEventListener("mouseenter", () => spinDisc(4));
$("drop").addEventListener("mouseleave", () => spinDisc(1));

/* ---------- state ---------- */
const S = {
  game: null,
  tracks: [],
  files: [],
  filter: "all",
  query: "",
  selected: new Set(),
  output: "",
  busy: false,
  playing: null,
  lastFolder: "",
};
try { S.output = localStorage.getItem("output") || ""; } catch {}
const prefs = { raw: true, rawRate: "22050", loops: "once" };
try { Object.assign(prefs, JSON.parse(localStorage.getItem("prefs") || "{}")); } catch {}
$("optRaw").checked = prefs.raw;
$("optRawRate").value = prefs.rawRate;
$("optLoops").value = prefs.loops;
function savePrefs() {
  prefs.raw = $("optRaw").checked;
  prefs.rawRate = $("optRawRate").value;
  prefs.loops = $("optLoops").value;
  try { localStorage.setItem("prefs", JSON.stringify(prefs)); } catch {}
}
// Changing what counts as a sound means searching again.
$("optRaw").addEventListener("change", () => { savePrefs(); startScan(); });
$("optRawRate").addEventListener("change", () => { savePrefs(); if (prefs.raw) startScan(); });
$("optLoops").addEventListener("change", savePrefs);

/* ---------- opening a game ---------- */
async function openWith(promise) {
  try {
    const info = await promise;
    if (info) showGame(info);
  } catch (e) {
    toast(e.message, "err");
  }
}
$("openFile").addEventListener("click", () => openWith(call("pick_source", { folder: false })));
$("openFolder").addEventListener("click", () => openWith(call("pick_source", { folder: true })));
$("change").addEventListener("click", () => openWith(call("pick_source", { folder: false })));
$("rescan").addEventListener("click", startScan);

function showGame(info) {
  stopPreview();
  S.game = info;
  S.tracks = [];
  S.selected.clear();
  S.filter = "all";
  $("search").value = S.query = "";
  if (!S.output) S.output = info.default_output;
  $("start").classList.add("hidden");
  $("work").classList.remove("hidden");
  $("footer").classList.remove("hidden");
  $("results").classList.add("hidden");
  $("openOut").classList.add("hidden");
  $("gameTitle").textContent = info.title;
  $("gameSerial").textContent = info.serial || "";
  $("gameMeta").textContent = `${fmtNum(info.files)} ${info.files === 1 ? "file" : "files"} · ${fmtBytes(info.size)} · ${info.path}`;
  $("tbSub").textContent = info.title;
  updateOutput();
  updateFooter();
  startScan();
}

/* ---------- scanning ---------- */
async function startScan() {
  if (!S.game || S.busy) return;
  stopPreview();
  S.busy = true;
  $("scan").classList.remove("hidden");
  $("results").classList.add("hidden");
  $("scanText").textContent = "Searching the game's files…";
  $("scanPct").textContent = "";
  $("scanBar").style.width = "0";
  updateFooter();
  try {
    await call("scan", { headerless: $("optRaw").checked, headerless_rate: +$("optRawRate").value });
  } catch (e) {
    S.busy = false;
    $("scan").classList.add("hidden");
    updateFooter();
    toast(e.message, "err");
  }
}
$("scanCancel").addEventListener("click", () => call("cancel"));

function onScanProgress(ev) {
  const pct = ev.total ? (ev.done / ev.total) * 100 : 0;
  $("scanBar").style.width = pct.toFixed(1) + "%";
  $("scanPct").textContent = `${Math.floor(pct)}%`;
  if (ev.file) $("scanText").textContent = ev.file;
}

function onScanDone(ev) {
  S.busy = false;
  $("scan").classList.add("hidden");
  if (ev.error) {
    if (!ev.cancelled) toast(`Couldn't finish searching: ${ev.error}`, "err");
    updateFooter();
    return;
  }
  S.tracks = ev.tracks;
  S.files = ev.files;
  S.selected = new Set(S.tracks.filter((t) => !t.note).map((t) => t.id));
  $("results").classList.remove("hidden");
  renderChips();
  renderRows();
  updateFooter();
  const total = S.tracks.reduce((s, t) => s + t.samples / t.sample_rate, 0);
  if (S.tracks.length) {
    const took = ev.seconds >= 1 ? ` in ${ev.seconds.toFixed(1)} s` : "";
    toast(`Found ${fmtNum(S.tracks.length)} ${S.tracks.length === 1 ? "sound" : "sounds"} (${fmtLong(total)} of audio)${took}`, "ok");
  }
}

/* ---------- results ---------- */
function renderChips() {
  const counts = {};
  for (const t of S.tracks) counts[t.format] = (counts[t.format] || 0) + 1;
  // Most common formats first; headerless ("RAW") last.
  const formats = Object.keys(counts).sort((a, b) => (a === "RAW") - (b === "RAW") || counts[b] - counts[a] || a.localeCompare(b));
  const chips = [["all", "All", S.tracks.length], ...formats.map((f) => [f, f === "RAW" ? "Headerless" : f, counts[f]])];
  $("chips").innerHTML = chips.map(([k, label, n]) =>
    `<button class="chip ${S.filter === k ? "on" : ""}" data-f="${esc(k)}">${esc(label)}<span class="n">${fmtNum(n)}</span></button>`).join("");
}
$("chips").addEventListener("click", (e) => {
  const c = e.target.closest("[data-f]");
  if (!c) return;
  S.filter = c.dataset.f;
  renderChips();
  renderRows();
});
$("search").addEventListener("input", (e) => {
  S.query = e.target.value.trim().toLowerCase();
  renderRows();
});

function visibleTracks() {
  return S.tracks.filter((t) =>
    (S.filter === "all" || t.format === S.filter) &&
    (!S.query || t.path.toLowerCase().includes(S.query) || S.files[t.entry].toLowerCase().includes(S.query)));
}

const PLAY = '<svg viewBox="0 0 10 10"><path d="M2 1l7 4-7 4z"/></svg>';
const PAUSE = '<svg viewBox="0 0 10 10"><path d="M2 1h2v8H2zM6 1h2v8H6z"/></svg>';

function renderRows() {
  const list = visibleTracks();
  const html = list.map((t) => {
    const slash = t.path.lastIndexOf("/");
    const dir = slash >= 0 ? t.path.slice(0, slash + 1) : "";
    const file = t.path.slice(slash + 1);
    const src = S.files[t.entry] + (t.offset ? ` @ 0x${t.offset.toString(16).toUpperCase()}` : "");
    const playing = S.playing === t.id;
    return `<tr data-id="${t.id}" class="${t.note ? "bad" : ""} ${playing ? "playing" : ""}">
      <td class="c-check"><input type="checkbox" ${S.selected.has(t.id) ? "checked" : ""} ${t.note ? "disabled" : ""}></td>
      <td class="c-play">${t.note ? "" : `<button class="play" title="Preview">${playing ? PAUSE : PLAY}</button>`}</td>
      <td class="name" title="${esc(t.path)}"><span class="dir">${esc(dir)}</span>${esc(file)}${t.note ? `<span class="note">${esc(t.note)}</span>` : ""}</td>
      <td class="src" title="${esc(src)}">${esc(src)}</td>
      <td class="c-fmt"><span class="fmt" ${t.format === "RAW" ? `title="No header: the sample rate is the one chosen above"` : ""}>${esc(t.format)}</span></td>
      <td class="c-num">${t.channels === 1 ? "Mono" : t.channels === 2 ? "Stereo" : t.channels}</td>
      <td class="c-num">${fmtNum(t.sample_rate)} Hz</td>
      <td class="c-num">${t.loop_end != null ? `<span class="loop" title="Loops from ${fmtDur(t.loop_start / t.sample_rate)} to ${fmtDur(t.loop_end / t.sample_rate)}">↻</span>` : ""}${fmtDur(t.samples / t.sample_rate)}</td>
    </tr>`;
  }).join("");
  $("rows").innerHTML = html;
  const empty = $("empty");
  if (!S.tracks.length) {
    empty.innerHTML = "<b>No audio found</b>This game may keep its sound in a format this tool doesn't read yet (compressed or headerless archives).";
  } else if (!list.length) {
    empty.innerHTML = "<b>Nothing matches</b>Try another filter.";
  }
  empty.classList.toggle("hidden", list.length > 0);
  updateCheckAll();
}

function updateCheckAll() {
  const list = visibleTracks().filter((t) => !t.note);
  const on = list.filter((t) => S.selected.has(t.id)).length;
  $("checkAll").checked = list.length > 0 && on === list.length;
  $("checkAll").indeterminate = on > 0 && on < list.length;
}
$("checkAll").addEventListener("change", (e) => {
  for (const t of visibleTracks()) {
    if (t.note) continue;
    e.target.checked ? S.selected.add(t.id) : S.selected.delete(t.id);
  }
  renderRows();
  updateFooter();
});
$("rows").addEventListener("change", (e) => {
  const tr = e.target.closest("tr");
  if (!tr || e.target.type !== "checkbox") return;
  const id = +tr.dataset.id;
  e.target.checked ? S.selected.add(id) : S.selected.delete(id);
  updateCheckAll();
  updateFooter();
});
$("rows").addEventListener("click", (e) => {
  const btn = e.target.closest(".play");
  if (btn) togglePreview(+btn.closest("tr").dataset.id);
});

/* ---------- previews ---------- */
const audio = $("audio");
function togglePreview(id) {
  if (S.playing === id && !audio.paused) return stopPreview();
  S.playing = id;
  audio.src = `/preview?id=${id}`;
  audio.play().catch((e) => { if (e.name !== "AbortError") toast("Couldn't play this sound", "err"); stopPreview(); });
  markPlaying();
}
function stopPreview() {
  audio.pause();
  audio.removeAttribute("src");
  S.playing = null;
  markPlaying();
}
function markPlaying() {
  for (const tr of $("rows").querySelectorAll("tr")) {
    const on = +tr.dataset.id === S.playing;
    tr.classList.toggle("playing", on);
    const b = tr.querySelector(".play");
    if (b) b.innerHTML = on ? PAUSE : PLAY;
  }
}
audio.addEventListener("ended", stopPreview);
audio.addEventListener("error", () => { if (S.playing !== null && audio.src) { toast("Couldn't play this sound", "err"); stopPreview(); } });

/* ---------- extracting ---------- */
function updateOutput() {
  $("outPath").textContent = S.output || "Choose a folder…";
  $("outPath").title = S.output ? `${S.output}\n(click to change)` : "Choose where to save the audio";
}
$("outPath").addEventListener("click", async () => {
  try {
    const p = await call("pick_output");
    if (p) {
      S.output = p;
      try { localStorage.setItem("output", p); } catch {}
      updateOutput();
    }
  } catch (e) { toast(e.message, "err"); }
});

function updateFooter() {
  const n = S.tracks.filter((t) => S.selected.has(t.id)).length;
  const btn = $("extract");
  btn.disabled = S.busy || n === 0;
  btn.textContent = n ? `Extract ${fmtNum(n)} ${n === 1 ? "sound" : "sounds"}` : "Extract";
  $("rescan").disabled = S.busy;
  $("change").disabled = S.busy;
}

$("extract").addEventListener("click", async () => {
  if (S.busy) return;
  stopPreview();
  const ids = S.tracks.filter((t) => S.selected.has(t.id)).map((t) => t.id);
  S.busy = true;
  setExtracting(true);
  $("exText").textContent = "Starting…";
  $("exBar").style.width = "0";
  updateFooter();
  try {
    await call("extract", { output: S.output, ids: ids.length === S.tracks.length ? null : ids, loops: $("optLoops").value });
  } catch (e) {
    S.busy = false;
    setExtracting(false);
    updateFooter();
    toast(e.message, "err");
  }
});
$("exCancel").addEventListener("click", () => call("cancel"));
$("openOut").addEventListener("click", () => S.lastFolder && call("open_path", { path: S.lastFolder }).catch((e) => toast(e.message, "err")));

function setExtracting(on) {
  $("footer").classList.toggle("running", on);
  $("exProgress").classList.toggle("hidden", !on);
  $("exCancel").classList.toggle("hidden", !on);
  $("extract").classList.toggle("hidden", on);
  if (on) $("openOut").classList.add("hidden");
}

function onExtractProgress(ev) {
  const pct = ev.total ? (ev.done / ev.total) * 100 : 0;
  $("exBar").style.width = pct.toFixed(1) + "%";
  $("exText").textContent = `${fmtNum(ev.done)} of ${fmtNum(ev.total)}${ev.file ? " · " + ev.file : ""}`;
}

function onExtractDone(ev) {
  S.busy = false;
  setExtracting(false);
  updateFooter();
  if (ev.error) {
    if (ev.cancelled) toast("Stopped. What was already saved is kept.");
    else toast(`Couldn't finish: ${ev.error}`, "err");
    return;
  }
  S.lastFolder = ev.folder;
  $("openOut").classList.remove("hidden");
  const extra = [];
  if (ev.skipped.length) extra.push(`${ev.skipped.length} skipped`);
  if (ev.failed.length) extra.push(`${ev.failed.length} failed`);
  toast(`Saved ${fmtNum(ev.written)} WAV ${ev.written === 1 ? "file" : "files"}${extra.length ? ` (${extra.join(", ")})` : ""}`, ev.failed.length ? "err" : "ok");
  for (const f of ev.failed.slice(0, 3)) toast(f, "err");
}

/* ---------- events from Rust ---------- */
function onEvent(ev) {
  switch (ev.name) {
    case "scan_progress": return onScanProgress(ev);
    case "scan_done": return onScanDone(ev);
    case "extract_progress": return onExtractProgress(ev);
    case "extract_done": return onExtractDone(ev);
    case "window": return setMaximized(ev.maximized);
    case "drag":
      spinDisc(ev.over ? 8 : 1);
      return $("dropOverlay").classList.toggle("hidden", !ev.over || S.busy);
    case "dropped":
      if (S.busy) return toast("Wait for the current job to finish", "err");
      return openWith(call("open_source", { path: ev.path }));
  }
}

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && S.playing !== null) stopPreview();
  if ((e.ctrlKey && (e.key === "r" || e.key === "p")) || e.key === "F5") e.preventDefault();
});
document.addEventListener("contextmenu", (e) => { if (!e.target.closest("input, .name, .source-meta")) e.preventDefault(); });

call("init").catch(() => {});
