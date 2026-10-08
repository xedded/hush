(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => Array.from(r.querySelectorAll(s));
  const css = (name) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  const tauri = window.__TAURI__;
  const invoke = (cmd, args) => tauri.core.invoke(cmd, args);
  const appWindow = tauri.window.getCurrentWindow();

  const SCOPE_LEN = 160;
  const METER_FLOOR_DB = -60;
  const METER_FALL_DB = 1.2; // per frame, so peaks decay like a VU meter
  const SEND_INTERVAL_MS = 50;

  let ui = null; // last UiState from the engine side
  let enrolled = false; // set by voices.js
  const micName = () => (ui && ui.virtualMicName) || "Hush Microphone";

  // macOS: native traffic-light buttons and the Mac way of writing the shortcut.
  const isMac = /Mac/.test(navigator.userAgent);
  if (isMac) {
    document.documentElement.dataset.platform = "macos";
    $$(".kbd").forEach((k) => { if (k.textContent.trim() === "Ctrl Alt M") k.textContent = "⌃⌥M"; });
  }

  // ---------- Generic controls ----------
  let toastTimer;
  const toast = (msg) => {
    const t = $("#toast"); t.textContent = msg; t.hidden = false;
    clearTimeout(toastTimer); toastTimer = setTimeout(() => (t.hidden = true), 2600);
  };
  const fail = (e) => toast(typeof e === "string" ? e : "Något gick fel. Försök igen.");

  const checked = (el) => el.getAttribute("aria-checked") === "true";
  const setChecked = (el, v) => el.setAttribute("aria-checked", String(!!v));
  $$(".switch").forEach((s) => s.addEventListener("click", () => {
    setChecked(s, !checked(s));
    s.dispatchEvent(new Event("change"));
  }));
  const seg = (el, fn) => $$("button", el).forEach((b) => b.addEventListener("click", () => {
    $$("button", el).forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
    fn(b);
  }));
  const pressSeg = (el, attr, value) => $$("button", el).forEach((b) => b.setAttribute("aria-pressed", String(b.dataset[attr] === value)));

  const paint = (r) => r.style.setProperty("--pct", ((r.value - r.min) / (r.max - r.min)) * 100 + "%");
  const throttle = (fn) => {
    let last = 0, timer = null, pending;
    return (v) => {
      pending = v;
      const run = () => { last = Date.now(); timer = null; fn(pending); };
      if (Date.now() - last >= SEND_INTERVAL_MS) run();
      else if (!timer) timer = setTimeout(run, SEND_INTERVAL_MS);
    };
  };
  const bind = (id, label, send) => {
    const r = $(id);
    const push = send ? throttle(send) : null;
    r.addEventListener("input", () => { paint(r); label(+r.value); if (push) push(+r.value); });
    paint(r); label(+r.value);
    return (v) => { r.value = v; paint(r); label(+r.value); };
  };

  window.HushUI = { $, $$, css, invoke, toast, fail, checked, setChecked, seg, pressSeg };

  // ---------- Window and navigation ----------
  $("#winMin").addEventListener("click", () => appWindow.minimize());
  $("#winMax").addEventListener("click", () => appWindow.toggleMaximize());
  $("#winClose").addEventListener("click", () => appWindow.close());

  $$("nav button[data-view]").forEach((b) => b.addEventListener("click", () => {
    $$("nav button[data-view]").forEach((x) => x.removeAttribute("aria-current"));
    b.setAttribute("aria-current", "page");
    $$("[data-panel]").forEach((p) => (p.hidden = p.dataset.panel !== b.dataset.view));
    resizeCanvases();
  }));

  // ---------- Sound: wired to the engine ----------
  const setSupp = bind("#supp", (v) => ($("#suppOut").textContent = "-" + Math.round(v * 0.45) + " dB"),
    (v) => invoke("set_suppression", { value: v }).catch(fail));
  const setGate = bind("#gate", (v) => ($("#gateOut").textContent = v + " dBFS"),
    (v) => invoke("set_gate", { value: v }).catch(fail));

  $("#masterSwitch").addEventListener("change", (e) => {
    const value = checked(e.target);
    invoke("set_active", { value }).then(() => { ui.active = value; renderRoute(); }).catch(fail);
  });
  $("#muteRow").addEventListener("click", () => {
    const value = !(ui && ui.muted);
    invoke("set_muted", { value }).then(() => { ui.muted = value; renderRoute(); }).catch(fail);
  });
  seg($("#modeSeg"), (b) => {
    invoke("set_mode", { value: b.dataset.mode }).then(() => { ui.mode = b.dataset.mode; renderMode(); }).catch(fail);
  });
  $("#inputDevice").addEventListener("change", (e) => {
    const sel = e.target;
    const name = sel.options[sel.selectedIndex].textContent;
    sel.disabled = true;
    $("#routeText").textContent = "Byter till " + name;
    invoke("select_input", { id: sel.value })
      .then((s) => { render(s); if (!s.error) toast("Mikrofon bytt till " + name); })
      .catch(fail)
      .finally(() => (sel.disabled = false));
  });
  $("#retry").addEventListener("click", () => {
    $("#retry").disabled = true;
    invoke("restart_engine").then(render).catch(fail).finally(() => ($("#retry").disabled = false));
  });

  function renderRoute() {
    if (!ui) return;
    const dot = $("#route .dot"), text = $("#routeText");
    const running = !!ui.engine;
    dot.classList.toggle("idle", !running || !ui.active || ui.muted);
    if (!running) text.textContent = micName() + " är inte igång";
    else if (ui.muted) text.textContent = "Mikrofonen är avstängd";
    else if (!ui.active) text.textContent = "Pausad, " + ui.engine.inputName + " skickas obehandlad";
    else text.textContent = ui.engine.inputName + " till " + micName();
    setChecked($("#masterSwitch"), ui.active);
    $("#muteRow").setAttribute("aria-pressed", String(ui.muted));
    $("#muteLabel").textContent = ui.muted ? "Mikrofonen är avstängd" : "Stäng av mikrofon";
  }

  function renderMode() {
    pressSeg($("#modeSeg"), "mode", ui.mode);
    $("#meHint").hidden = ui.mode !== "me" || enrolled;
  }

  const SETUP = {
    missing: isMac ? {
      title: "Installera BlackHole",
      body: "Hush skickar ditt rena ljud till mötesprogrammen via den kostnadsfria ljuddrivrutinen BlackHole. Ladda ner BlackHole 2ch, öppna paketet och följ installationen. Välj sedan BlackHole 2ch som mikrofon i Teams, Zoom eller Slack.",
      action: "Hämta BlackHole",
    } : {
      title: "Installera VB-CABLE",
      body: "Hush skickar ditt rena ljud till mötesprogrammen via den kostnadsfria drivrutinen VB-CABLE från VB-Audio. Ladda ner den, packa upp filen och kör VBCABLE_Setup_x64.exe som administratör. Starta sedan om datorn.",
      action: "Hämta VB-CABLE",
    },
    rename: {
      title: "Döp om mikrofonen till Hush Microphone",
      body: "VB-CABLE är installerad men heter CABLE Output i Teams, Zoom och Slack. Windows frågar efter administratörsbehörighet.",
      action: "Byt namn",
    },
  };
  let setupStatus = null;
  function renderSetup(status) {
    setupStatus = status;
    const info = SETUP[status];
    $("#setup").hidden = !info;
    if (!info) return;
    $("#setupTitle").textContent = info.title;
    $("#setupBody").textContent = info.body;
    $("#setupAction").textContent = info.action;
  }
  $("#setupAction").addEventListener("click", () => {
    const btn = $("#setupAction");
    if (setupStatus === "missing") {
      invoke("open_vbcable_page").catch(fail);
      return;
    }
    btn.disabled = true;
    invoke("rename_virtual_mic")
      .then((s) => { render(s); toast("Mikrofonen heter nu Hush Microphone"); })
      .catch(fail)
      .finally(() => (btn.disabled = false));
  });

  function render(s) {
    ui = s;
    $$(".mic-name").forEach((el) => (el.textContent = s.virtualMicName));
    const sel = $("#inputDevice");
    sel.textContent = "";
    s.inputs.forEach((d) => {
      const o = document.createElement("option");
      o.value = d.id; o.textContent = d.name;
      sel.appendChild(o);
    });
    if (!s.inputs.length) {
      const o = document.createElement("option"); o.textContent = "Ingen mikrofon hittades"; o.value = "";
      sel.appendChild(o);
    }
    if (s.selectedInput && s.inputs.some((d) => d.id === s.selectedInput)) sel.value = s.selectedInput;

    const current = s.inputs.find((d) => d.id === sel.value);
    $("#btHint").hidden = !((current && current.bluetooth) || (s.engine && s.engine.bluetoothQuality));

    renderSetup(s.virtualMic);
    // The setup panel already explains a missing VB-CABLE.
    $("#engineError").hidden = !s.error || s.virtualMic === "missing";
    $("#engineErrorText").textContent = s.error || "";

    setSupp(Math.round(s.suppression));
    setGate(Math.round(s.gateDbfs));
    renderMode();
    renderRoute();
  }

  // Levels from the engine, about 30 times a second.
  const inHist = new Array(SCOPE_LEN).fill(0), outHist = new Array(SCOPE_LEN).fill(0);
  let peakIn = METER_FLOOR_DB, peakOut = METER_FLOOR_DB, statTick = 0;
  const toUnit = (db) => Math.max(0, Math.min(1, (db - METER_FLOOR_DB) / -METER_FLOOR_DB));

  tauri.event.listen("telemetry", ({ payload: t }) => {
    t.scopeIn.forEach((v) => { inHist.push(toUnit(v)); inHist.shift(); });
    t.scopeOut.forEach((v) => { outHist.push(toUnit(v)); outHist.shift(); });
    peakIn = Math.max(t.peakInDb, peakIn - METER_FALL_DB);
    peakOut = Math.max(t.peakOutDb, peakOut - METER_FALL_DB);
    if (++statTick % 15 === 0) {
      const running = ui && ui.engine;
      $("#cpu").textContent = running ? t.cpuPct.toFixed(1).replace(".", ",") + " %" : "-";
      $("#latency").textContent = running ? Math.round(t.latencyMs) + " ms" : "-";
    }
  });
  tauri.event.listen("state", ({ payload }) => render(payload));

  // ---------- Settings ----------
  const THEME_KEY = "hush.theme";
  const applyTheme = (t) => {
    if (t === "light" || t === "dark") document.documentElement.setAttribute("data-theme", t);
    else document.documentElement.removeAttribute("data-theme");
  };
  let savedTheme = "system";
  try { savedTheme = localStorage.getItem(THEME_KEY) || "system"; } catch (_) { /* storage unavailable */ }
  applyTheme(savedTheme);
  pressSeg($("#themeSeg"), "t", savedTheme);
  seg($("#themeSeg"), (b) => {
    applyTheme(b.dataset.t);
    try { localStorage.setItem(THEME_KEY, b.dataset.t); } catch (_) { /* storage unavailable */ }
  });
  tauri.app.getVersion().then((v) => ($("#version").textContent = "Hush " + v)).catch(() => {});

  // ---------- Voice filter: preview until the feature ships ----------
  const presets = [
    ["Ingen", 0, 0], ["Filmtrailer", -5, -20], ["Rymdskurk", -8, -35], ["Sportkommentator", 2, 10],
    ["Robot", 0, 0], ["Troll", -10, -45], ["Radio 1985", 1, 5], ["Helium", 9, 40],
  ];

  bind("#pitch", (v) => ($("#pitchOut").textContent = (v > 0 ? "+" : "") + v + " halvtoner"));
  bind("#formant", (v) => ($("#formantOut").textContent = (v > 0 ? "+" : "") + v + " %"));
  $("#preview").addEventListener("click", () => toast("Röstfilter kommer i en senare version"));

  const presetBox = $("#presets");
  presets.forEach(([name, p, f]) => {
    const b = document.createElement("button");
    b.className = "preset"; b.setAttribute("aria-pressed", String(name === "Ingen"));
    const desc = name === "Robot" ? "Ringmodulator" : name === "Ingen" ? "Din egen röst" : (p > 0 ? "+" : "") + p + " st, " + (f > 0 ? "+" : "") + f + " %";
    b.innerHTML = "<strong></strong><span></span>";
    b.querySelector("strong").textContent = name; b.querySelector("span").textContent = desc;
    b.addEventListener("click", () => {
      $$(".preset").forEach((x) => x.setAttribute("aria-pressed", String(x === b)));
      const pr = $("#pitch"), fo = $("#formant");
      pr.value = p; fo.value = f; pr.dispatchEvent(new Event("input")); fo.dispatchEvent(new Event("input"));
      setChecked($("#fxSwitch"), name !== "Ingen");
    });
    presetBox.appendChild(b);
  });
  $("#resetFx").addEventListener("click", () => presetBox.firstChild.click());

  // ---------- Drawing ----------
  function fit(c) {
    const r = c.getBoundingClientRect(); const d = devicePixelRatio || 1;
    if (r.width === 0) return;
    c.width = Math.round(r.width * d); c.height = Math.round(r.height * d);
  }
  function resizeCanvases() { $$("canvas").forEach(fit); }
  addEventListener("resize", resizeCanvases);

  function drawScope(c, hist, color) {
    const g = c.getContext("2d"), w = c.width, h = c.height; if (!w) return;
    g.clearRect(0, 0, w, h);
    g.fillStyle = css("--line-soft"); g.fillRect(0, h / 2, w, 1);
    const bw = w / hist.length;
    g.fillStyle = color;
    hist.forEach((a, i) => {
      const bh = Math.max(1, a * h * 0.92);
      g.fillRect(i * bw, (h - bh) / 2, Math.max(1, bw - 1.5 * devicePixelRatio), bh);
    });
  }
  function drawMeter(c, db, label) {
    const g = c.getContext("2d"), w = c.width, h = c.height; if (!w) return;
    g.clearRect(0, 0, w, h);
    const segs = 24, gap = 2 * devicePixelRatio, sh = (h - gap * (segs - 1)) / segs;
    const lit = Math.round(toUnit(db) * segs);
    for (let i = 0; i < segs; i++) {
      const y = h - (i + 1) * sh - i * gap;
      g.fillStyle = i < lit ? (i >= segs - 3 ? css("--warn") : i >= segs - 8 ? css("--accent") : css("--ok")) : css("--line");
      g.fillRect(0, y, w, sh);
    }
    label.textContent = db <= METER_FLOOR_DB ? "-inf" : Math.round(db) + " dB";
  }

  let last = 0;
  function frame(t) {
    if (t - last > 33 && !document.hidden) {
      last = t;
      if (!$('[data-panel="sound"]').hidden) {
        drawScope($("#scopeIn"), inHist, css("--faint"));
        drawScope($("#scopeOut"), outHist, css("--accent"));
        drawMeter($("#meterIn"), peakIn, $("#dbIn"));
        drawMeter($("#meterOut"), peakOut, $("#dbOut"));
      }
    }
    requestAnimationFrame(frame);
  }

  // Shared with voices.js.
  Object.assign(window.HushUI, {
    fit,
    resizeCanvases,
    setEnrolled(v) { enrolled = v; if (ui) renderMode(); },
    mode: () => ui && ui.mode,
    setMode(mode) {
      return invoke("set_mode", { value: mode }).then(() => { ui.mode = mode; renderMode(); });
    },
  });

  resizeCanvases();
  requestAnimationFrame(frame);
  invoke("get_state").then(render).catch(fail);
})();
