// Voices view: the user's voice profile and one list of known voices, each with
// a single heard/muted choice. All data comes from the speaker service.
(() => {
  const { $, $$, css, invoke, toast, fail, setChecked, seg, pressSeg } = window.HushUI;
  const ui = window.HushUI;

  const LANE_HISTORY = 600; // 20 s of telemetry ticks at about 30 per second
  const LANE_FLOOR_DB = -60;
  const MAX_NAME = 40;
  const ME_COLOR = "#e0a458";
  const PALETTE = ["#7fa7c9", "#a593c4", "#8fb996", "#c99a7f", "#7fbfbf", "#c27f9f", "#b5b06a", "#8c9bb5"];
  const PENCIL = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"><path d="M10.5 2.5l3 3L6 13H3v-3l7.5-7.5z"/></svg>';
  const TRASH = '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5"/></svg>';
  const MONTHS = ["jan", "feb", "mars", "apr", "maj", "juni", "juli", "aug", "sep", "okt", "nov", "dec"];

  let view = null;
  const lanes = new Map(); // voice id -> { hist, canvas, on, color }

  // Stable colour per voice id, so a voice keeps its colour between meetings.
  const colorFor = (id) => {
    if (id === "me") return ME_COLOR;
    let h = 0;
    for (const ch of id) h = (h * 31 + ch.charCodeAt(0)) >>> 0;
    return PALETTE[h % PALETTE.length];
  };

  // The statistics page uses the same colours.
  ui.colorFor = colorFor;
  ui.dateLabel = (iso) => heardLabel(iso);

  const heardLabel = (iso) => {
    const today = new Date().toISOString().slice(0, 10);
    if (iso === today) return "i dag";
    const [y, m, d] = iso.split("-").map(Number);
    if (!y) return iso;
    const label = d + " " + MONTHS[m - 1];
    return y === new Date().getFullYear() ? label : label + " " + y;
  };

  const apply = (p) => p.then(render).catch(fail);

  // Editable name: click the name or the pencil, Enter saves, Escape cancels.
  function nameField(v) {
    const wrap = document.createElement("span"); wrap.className = "name-field";
    const input = document.createElement("input");
    input.value = v.name; input.maxLength = MAX_NAME;
    input.setAttribute("aria-label", "Namn på " + v.name);
    const pen = document.createElement("button");
    pen.className = "name-edit"; pen.type = "button"; pen.innerHTML = PENCIL;
    pen.setAttribute("aria-label", "Byt namn på " + v.name); pen.title = "Byt namn";
    pen.addEventListener("click", () => { input.focus(); input.select(); });
    let cancelled = false;
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") input.blur();
      if (e.key === "Escape") { cancelled = true; input.value = v.name; input.blur(); }
    });
    input.addEventListener("blur", () => {
      const next = input.value.trim().slice(0, MAX_NAME);
      if (cancelled || !next || next === v.name) { cancelled = false; input.value = v.name; return; }
      invoke("rename_voice", { id: v.id, name: next })
        .then((s) => { render(s); toast("Rösten heter nu " + next); })
        .catch((e) => { input.value = v.name; fail(e); });
    });
    wrap.append(input, pen);
    return wrap;
  }

  function renderEnrollment() {
    const e = view.enrollment;
    $("#enrollPanel").hidden = !e;
    $("#enrollIntro").hidden = !!e || view.enrolled;
    $("#enroll").hidden = !!e || !view.enrolled;
    if (!e) return;
    const pct = Math.min(100, (e.seconds / e.needed) * 100);
    $("#enrollBar").style.width = pct + "%";
    $(".progress").setAttribute("aria-valuenow", String(Math.round(pct)));
    $("#enrollCount").textContent = Math.floor(e.seconds) + " av " + e.needed + " s";
  }

  function renderOnlyMe() {
    $("#onlyMeNote").hidden = !view || !view.enrolled || !!view.enrollment || ui.mode() !== "me";
  }

  // Deleting a voiceprint is permanent, so it takes a second click to confirm.
  function deleteButton(v) {
    const del = document.createElement("button");
    del.className = "btn icon"; del.innerHTML = TRASH;
    del.setAttribute("aria-label", "Ta bort " + v.name); del.title = "Ta bort";
    let armed = false, disarm;
    del.addEventListener("click", () => {
      if (!armed) {
        armed = true; del.classList.add("armed"); del.setAttribute("aria-label", "Bekräfta borttagning");
        toast("Klicka igen för att ta bort " + v.name + " permanent");
        disarm = setTimeout(() => { armed = false; del.classList.remove("armed"); del.setAttribute("aria-label", "Ta bort " + v.name); }, 3000);
        return;
      }
      clearTimeout(disarm);
      invoke("delete_voice", { id: v.id })
        .then((s) => { render(s); toast(v.name + " borttagen. Röstavtrycket är raderat."); })
        .catch(fail);
    });
    return del;
  }

  function voiceRow(v) {
    const me = v.id === "me";
    const row = document.createElement("div");
    row.className = "voice" + (me || v.heard ? "" : " muted");
    row.innerHTML = `
      <div class="who"><i class="swatch"></i><div class="who-text">
        <div class="who-name"></div><div class="who-meta"></div></div></div>
      <canvas height="28"></canvas>
      <div class="match"></div>
      <div class="cell-switch"></div>
      <div class="cell-delete"></div>`;
    row.querySelector(".swatch").style.background = colorFor(v.id);
    const nameBox = row.querySelector(".who-name");
    if (me) {
      nameBox.textContent = "Din röst";
      const tag = document.createElement("span"); tag.className = "tag"; tag.textContent = "Profil"; nameBox.appendChild(tag);
    } else {
      nameBox.appendChild(nameField(v));
    }
    const status = () => (v.recent ? "Hörd i det här mötet" : "Senast hörd " + heardLabel(v.lastHeard));
    row.querySelector(".who-meta").textContent = me ? "Släpps alltid igenom" : (v.named ? status() : "Ny röst, ge den ett namn");
    row.querySelector(".match").textContent = !me && v.score != null ? Math.round(v.score * 100) + " %" : "";
    if (!me) {
      const sw = document.createElement("button");
      sw.className = "switch"; sw.setAttribute("role", "switch");
      setChecked(sw, v.heard);
      sw.setAttribute("aria-label", v.name + " hörs i mötet");
      sw.addEventListener("click", () => {
        const heard = !v.heard;
        // Letting someone through means little while every other voice is muted.
        const leaveOnlyMe = heard && ui.mode() === "me"
          ? ui.setMode("noise").then(() => toast("Läget är nu Dämpa bakgrundsljud, så att " + v.name + " hörs"))
          : Promise.resolve();
        apply(leaveOnlyMe.then(() => invoke("set_voice_default", { id: v.id, policy: heard ? "pass" : "mute" })));
      });
      row.querySelector(".cell-switch").appendChild(sw);
      row.querySelector(".cell-delete").appendChild(deleteButton(v));
    }
    const lane = lanes.get(v.id) || { hist: new Array(LANE_HISTORY).fill(0) };
    Object.assign(lane, { canvas: row.querySelector("canvas"), on: me || v.heard, color: colorFor(v.id) });
    lanes.set(v.id, lane);
    return row;
  }

  function renderVoices() {
    const show = view.enrolled && !view.enrollment;
    $("#voicePanel").hidden = !show;
    $("#voicesPrivacy").hidden = !show;
    pressSeg($("#unknownSeg"), "u", view.unknown);
    const list = $("#voiceList"); list.textContent = "";
    if (!show) return;
    list.appendChild(voiceRow({ id: "me", name: "Din röst" }));
    view.voices.forEach((v) => list.appendChild(voiceRow(v)));
    $("#voicesEmpty").hidden = view.voices.length > 0;
    const ids = new Set(["me", ...view.voices.map((v) => v.id)]);
    for (const id of lanes.keys()) if (!ids.has(id)) lanes.delete(id);
    ui.resizeCanvases();
  }

  function render(v) {
    // Keep typing undisturbed: skip a re-render while a name is being edited.
    const editing = document.activeElement && document.activeElement.closest && document.activeElement.closest(".name-field");
    view = v;
    ui.setEnrolled(v.enrolled);
    $("#voicesError").hidden = !v.error;
    $("#voicesErrorText").textContent = v.error || "";
    renderEnrollment();
    renderOnlyMe();
    if (editing) return;
    renderVoices();
  }

  // Controls
  seg($("#unknownSeg"), (b) => apply(invoke("set_unknown_policy", { policy: b.dataset.u })));
  $("#leaveOnlyMe").addEventListener("click", () => ui.setMode("noise").catch(fail));
  document.addEventListener("hush:mode", renderOnlyMe);
  const startEnrollment = () => apply(invoke("start_enrollment"));
  $("#enroll").addEventListener("click", startEnrollment);
  $("#enrollStart").addEventListener("click", startEnrollment);
  $("#enrollCancel").addEventListener("click", () => apply(invoke("cancel_enrollment")));

  window.__TAURI__.event.listen("voices", ({ payload }) => render(payload));
  window.__TAURI__.event.listen("enrollment-done", ({ payload }) => toast(payload.message));

  // Activity lanes: who is speaking, at the input level, about 30 times a second.
  window.__TAURI__.event.listen("telemetry", ({ payload: t }) => {
    const level = t.scopeIn.length ? t.scopeIn[t.scopeIn.length - 1] : LANE_FLOOR_DB;
    const amount = Math.max(0, Math.min(1, (level - LANE_FLOOR_DB) / -LANE_FLOOR_DB));
    lanes.forEach((lane, id) => {
      lane.hist.push(t.speaking === id ? amount : 0);
      lane.hist.shift();
    });
  });

  function drawLane(lane) {
    const c = lane.canvas; if (!c) return;
    const g = c.getContext("2d"), w = c.width, h = c.height; if (!w) return;
    g.clearRect(0, 0, w, h);
    g.fillStyle = css("--line-soft"); g.fillRect(0, h - 1, w, 1);
    const bw = w / LANE_HISTORY;
    g.fillStyle = lane.on ? lane.color : css("--off");
    lane.hist.forEach((a, i) => {
      if (a <= 0) return;
      const bh = Math.max(2, a * (h - 2));
      g.fillRect(i * bw, h - 1 - bh, Math.ceil(bw), bh);
    });
  }

  let last = 0;
  function frame(t) {
    if (t - last > 50 && !document.hidden && !$('[data-panel="voices"]').hidden) {
      last = t;
      lanes.forEach(drawLane);
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  invoke("get_voices").then(render).catch(fail);
})();
