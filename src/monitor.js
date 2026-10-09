// "Lyssna på dig själv" in the sidebar: plays what the meeting hears on the
// default playback device, so noise suppression, enhancement and voice filter
// can all be judged by ear from any page.
(() => {
  const { $, invoke, toast, fail } = window.HushUI;

  // Telemetry sent before a change landed can still be on its way.
  const SETTLE_MS = 400;

  let monitoring = false;
  let changed = 0;

  function render() {
    $("#listen").setAttribute("aria-pressed", String(monitoring));
    $("#listenLabel").textContent = monitoring ? "Sluta lyssna" : "Lyssna på dig själv";
  }

  function setMonitor(value) {
    const btn = $("#listen");
    btn.disabled = true;
    return invoke("set_monitor", { value })
      .then(() => {
        monitoring = value;
        changed = Date.now();
        render();
        if (value) toast("Använd hörlurar, annars når ljudet mikrofonen och det blir rundgång");
      })
      .catch(fail)
      .finally(() => (btn.disabled = false));
  }
  $("#listen").addEventListener("click", () => setMonitor(!monitoring));

  // Never leave it playing unseen, e.g. after minimising to the tray.
  document.addEventListener("visibilitychange", () => {
    if (document.hidden && monitoring) setMonitor(false);
  });

  // The engine switches listening off by itself if the headphones go away.
  window.__TAURI__.event.listen("telemetry", ({ payload: t }) => {
    if (t.monitoring === monitoring || Date.now() - changed < SETTLE_MS) return;
    if (monitoring) toast("Uppspelningen stoppades. Kontrollera hörlurarna.");
    monitoring = t.monitoring;
    render();
  });

  const apply = (s) => {
    if (Date.now() - changed > SETTLE_MS) monitoring = s.monitor;
    render();
  };
  window.__TAURI__.event.listen("state", ({ payload }) => apply(payload));
  invoke("get_state").then(apply).catch(() => render());
})();
