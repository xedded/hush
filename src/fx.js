// Voice filter view: pitch, voice character and style.
(() => {
  const { $, invoke, fail, checked, setChecked, seg, pressSeg, bind } = window.HushUI;

  const PRESETS = [
    { name: "Ingen", desc: "Din egen röst", off: true },
    { name: "Filmtrailer", pitch: -5, formant: -20 },
    { name: "Rymdskurk", pitch: -8, formant: -35 },
    { name: "Sportkommentator", pitch: 2, formant: 10 },
    { name: "Robot", pitch: 0, formant: 0, style: "robot", desc: "Monoton och metallisk" },
    { name: "Troll", pitch: -10, formant: -45 },
    { name: "Radio 1985", pitch: 0, formant: 10, style: "radio", desc: "Liten högtalare" },
    { name: "Helium", pitch: 9, formant: 40 },
  ];
  // A state broadcast can predate the change just sent.
  const SETTLE_MS = 400;

  let fx = { enabled: false, pitch: 0, formant: 0, style: "natural" };

  const signed = (v) => (v > 0 ? "+" : "") + v;
  const describe = (p) => p.desc || signed(p.pitch) + " st, " + signed(p.formant) + " %";
  const matches = (p) => p.off
    ? !fx.enabled
    : fx.enabled && fx.pitch === p.pitch && fx.formant === p.formant && fx.style === (p.style || "natural");

  let lastSent = 0;
  function update(change) {
    fx = { ...fx, ...change };
    lastSent = Date.now();
    render();
    invoke("set_voice_fx", { value: fx }).catch(fail);
  }

  // Sliders send throttled; the value is read when the send fires, so a preset
  // clicked in between is not overwritten by an older slider position.
  const setPitch = bind("#pitch", (v) => ($("#pitchOut").textContent = signed(v) + " halvtoner"),
    () => update({ pitch: +$("#pitch").value }));
  const setFormant = bind("#formant", (v) => ($("#formantOut").textContent = signed(v) + " %"),
    () => update({ formant: +$("#formant").value }));
  // Moving a slider is a clear sign the user wants to hear it, so it switches the filter on.
  // Done at once rather than in the delayed send, which could undo a later "Ingen".
  ["#pitch", "#formant"].forEach((id) => $(id).addEventListener("input", () => {
    if (fx.enabled) return;
    fx = { ...fx, enabled: true };
    setChecked($("#fxSwitch"), true);
  }));

  const presetBox = $("#presets");
  const presetButtons = PRESETS.map((p) => {
    const b = document.createElement("button");
    b.className = "preset";
    b.innerHTML = "<strong></strong><span></span>";
    b.querySelector("strong").textContent = p.name;
    b.querySelector("span").textContent = describe(p);
    b.addEventListener("click", () => update(p.off
      ? { enabled: false }
      : { enabled: true, pitch: p.pitch, formant: p.formant, style: p.style || "natural" }));
    presetBox.appendChild(b);
    return [b, p];
  });

  $("#fxSwitch").addEventListener("change", (e) => update({ enabled: checked(e.target) }));
  seg($("#styleSeg"), (b) => update({ style: b.dataset.style, enabled: true }));
  $("#resetFx").addEventListener("click", () => update({ enabled: false, pitch: 0, formant: 0, style: "natural" }));

  function render() {
    setChecked($("#fxSwitch"), fx.enabled);
    setPitch(fx.pitch);
    setFormant(fx.formant);
    pressSeg($("#styleSeg"), "style", fx.style);
    presetButtons.forEach(([b, p]) => b.setAttribute("aria-pressed", String(matches(p))));
  }

  const apply = (s) => {
    if (Date.now() - lastSent > SETTLE_MS) fx = s.voiceFx;
    render();
  };
  window.__TAURI__.event.listen("state", ({ payload }) => apply(payload));
  invoke("get_state").then(apply).catch(() => render());
})();
