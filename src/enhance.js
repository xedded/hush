// Voice enhancement on the Sound view: on/off, profile and strength.
(() => {
  const { $, invoke, fail, checked, setChecked, seg, pressSeg, bind } = window.HushUI;

  const DESCRIPTIONS = {
    natural: "Tar bort muller och håller jämn nivå. Rösten låter som vanligt.",
    clear: "Mindre burkigt, mer närvaro, dämpade s-ljud och jämnare nivå. Bäst för att höras i möten.",
    warm: "Mer botten och mjukare diskant, som en poddröst.",
  };
  // A state broadcast can predate the change just sent.
  const SETTLE_MS = 400;

  let enh = { enabled: false, preset: "clear", amount: 50 };
  let lastSent = 0;

  function update(change) {
    enh = { ...enh, ...change };
    lastSent = Date.now();
    render();
    invoke("set_voice_enhance", { value: enh }).catch(fail);
  }

  // Read when the throttled send fires, so a later click is not overwritten.
  const setAmount = bind("#enhAmount", (v) => ($("#enhAmountOut").textContent = v + " %"),
    () => update({ amount: +$("#enhAmount").value }));

  $("#enhSwitch").addEventListener("change", (e) => update({ enabled: checked(e.target) }));
  seg($("#enhSeg"), (b) => update({ preset: b.dataset.preset, enabled: true }));

  function render() {
    setChecked($("#enhSwitch"), enh.enabled);
    pressSeg($("#enhSeg"), "preset", enh.preset);
    $("#enhDesc").textContent = DESCRIPTIONS[enh.preset] || "";
    setAmount(Math.round(enh.amount));
    // Naturlig has nothing to scale.
    const fixed = enh.preset === "natural";
    $("#enhAmount").disabled = fixed;
    $(".enh-amount").setAttribute("aria-disabled", String(fixed));
  }

  const apply = (s) => {
    if (s.voiceEnhance && Date.now() - lastSent > SETTLE_MS) enh = s.voiceEnhance;
    render();
  };
  window.__TAURI__.event.listen("state", ({ payload }) => apply(payload));
  invoke("get_state").then(apply).catch(() => render());
})();
