// Voice gate calibration on the Sound view: a quiet phase, a speaking phase,
// then the backend places the threshold just below the user's voice.
(() => {
  const { $, invoke, toast, fail } = window.HushUI;
  const ui = window.HushUI;

  const QUIET_S = 3;
  const SPEAK_S = 5;
  const TICK_MS = 100;

  let run = 0; // bumped by Avbryt, so a running measurement is ignored

  function show(step, text) {
    $("#calibStep").textContent = step;
    $("#calibText").textContent = text;
  }

  function measure(seconds, id) {
    const bar = $("#calibBar");
    bar.style.width = "0%";
    const started = Date.now();
    const timer = setInterval(() => {
      bar.style.width = Math.min(100, ((Date.now() - started) / (seconds * 1000)) * 100) + "%";
    }, TICK_MS);
    return invoke("measure_levels", { seconds }).finally(() => clearInterval(timer)).then((levels) => {
      if (id !== run) throw new Error("cancelled");
      return levels;
    });
  }

  function setBusy(busy) {
    $("#calib").hidden = !busy;
    $("#gateHelp").hidden = busy;
    $("#calibrate").disabled = busy;
    $("#gate").disabled = busy;
  }

  async function calibrate() {
    const id = ++run;
    setBusy(true);
    try {
      show("1 av 2: Var tyst", "Sitt still och var tyst medan Hush lyssnar på rummet.");
      const quiet = await measure(QUIET_S, id);
      show("2 av 2: Prata", "Prata som i ett möte tills mätaren är full, till exempel om vad du gjorde i går.");
      const speech = await measure(SPEAK_S, id);
      const threshold = await invoke("apply_gate_calibration", { quiet, speech });
      ui.showGate(threshold);
      toast("Röstgrinden är inställd på " + Math.round(threshold) + " dBFS för " + ui.inputName());
    } catch (e) {
      if (id === run && !(e instanceof Error && e.message === "cancelled")) fail(e);
    } finally {
      if (id === run) setBusy(false);
    }
  }

  $("#calibrate").addEventListener("click", calibrate);
  $("#calibCancel").addEventListener("click", () => { run++; setBusy(false); });
})();
