// Statistics view: talk time, share and loudness per voice from the library.
(() => {
  const { $, $$, css, invoke, toast, fail, seg } = window.HushUI;
  const ui = window.HushUI;

  const REFRESH_MS = 5_000;
  // Too little speech for "loudest" or "longest turns" to mean anything.
  const MIN_SECONDS_FOR_RANKING = 10;

  let stats = null;
  let sortBy = "seconds";

  const duration = (s) => {
    if (s < 60) return Math.round(s) + " s";
    const min = Math.round(s / 60);
    if (min < 60) return min + " min";
    return Math.floor(min / 60) + " h " + String(min % 60).padStart(2, "0") + " min";
  };
  const db = (v) => (v == null ? "-" : Math.round(v) + " dB");
  const pct = (v) => Math.round(v * 100) + " %";

  function card(label, value, detail) {
    const c = document.createElement("div");
    c.className = "panel card";
    c.innerHTML = '<div class="label"></div><strong></strong><span></span>';
    c.querySelector(".label").textContent = label;
    c.querySelector("strong").textContent = value;
    c.querySelector("span").textContent = detail;
    return c;
  }

  const best = (rows, key) => rows
    .filter((r) => r.seconds >= MIN_SECONDS_FOR_RANKING && r[key] != null)
    .reduce((a, r) => (!a || r[key] > a[key] ? r : a), null);

  function renderCards() {
    const box = $("#statCards"); box.textContent = "";
    const rows = stats.rows;
    const top = rows[0];
    const loud = best(rows, "meanDb");
    const long = best(rows, "turnSeconds");
    box.append(
      card("Total taltid", duration(stats.totalSeconds), stats.since ? "sedan " + ui.dateLabel(stats.since) : ""),
      card("Pratar mest", top.name, pct(top.share) + " av taltiden"),
      card("Starkast röst", loud ? loud.name : "-", loud ? db(loud.meanDb) + " i snitt" : "behöver mer tal"),
      card("Längst inlägg", long ? long.name : "-", long ? duration(long.turnSeconds) + " i snitt" : "behöver mer tal"),
    );
  }

  function drawChart(canvas, values, color) {
    ui.fit(canvas);
    const g = canvas.getContext("2d"), w = canvas.width, h = canvas.height;
    if (!w) return;
    g.clearRect(0, 0, w, h);
    const max = Math.max(...stats.rows.flatMap((r) => r.chart), 1);
    const bw = w / values.length;
    values.forEach((v, i) => {
      const bh = v > 0 ? Math.max(2, (v / max) * (h - 1)) : 1;
      g.fillStyle = v > 0 ? color : css("--line-soft");
      g.fillRect(i * bw + 1, h - bh, Math.max(1, bw - 2), bh);
    });
  }

  function sorted() {
    const key = { seconds: (r) => r.seconds, level: (r) => r.meanDb ?? -999, turns: (r) => r.turns }[sortBy];
    return [...stats.rows].sort((a, b) => key(b) - key(a));
  }

  function renderRows() {
    const box = $("#statRows"); box.textContent = "";
    const maxShare = Math.max(...stats.rows.map((r) => r.share), 0.01);
    sorted().forEach((r) => {
      const color = ui.colorFor ? ui.colorFor(r.id) : css("--accent");
      const row = document.createElement("div");
      row.className = "stat-row";
      row.innerHTML = `
        <div class="stat-name"><i class="swatch"></i><span></span></div>
        <div class="share"><i></i><em></em></div>
        <span class="t"></span><span class="m"></span><span class="p"></span><span class="n"></span>
        <canvas height="24"></canvas>`;
      row.querySelector(".swatch").style.background = color;
      row.querySelector(".stat-name span").textContent = r.name;
      const bar = row.querySelector(".share i");
      bar.style.width = (r.share / maxShare) * 80 + "%";
      bar.style.background = color;
      row.querySelector(".share em").textContent = pct(r.share);
      row.querySelector(".t").textContent = duration(r.seconds);
      row.querySelector(".m").textContent = db(r.meanDb);
      row.querySelector(".p").textContent = db(r.peakDb);
      const turns = row.querySelector(".n");
      turns.textContent = r.turns;
      if (r.turnSeconds != null) {
        const avg = document.createElement("small"); avg.textContent = duration(r.turnSeconds) + " i snitt";
        turns.appendChild(avg);
      }
      row.title = "Hörd " + r.daysHeard + (r.daysHeard === 1 ? " dag" : " dagar") +
        (r.lastHeard ? ", senast " + ui.dateLabel(r.lastHeard) : "");
      box.appendChild(row);
      drawChart(row.querySelector("canvas"), r.chart, color);
    });
  }

  function render(v) {
    stats = v;
    const empty = !v.enrolled || !v.rows.length;
    $("#statsEmpty").hidden = !empty;
    $("#statCards").hidden = empty;
    $("#statsTable").hidden = empty;
    $("#resetStats").hidden = empty;
    if (empty) {
      $("#statsEmptyTitle").textContent = v.enrolled ? "Ingen statistik än" : "Spela in din röstprofil först";
      $("#statsEmptyText").textContent = v.enrolled
        ? "Statistiken fylls på när Hush känner igen röster, i lägena Dämpa bakgrundsljud och Bara min röst."
        : "Statistiken bygger på röstigenkänningen, som startar när du har spelat in din röstprofil under Röster.";
      return;
    }
    renderCards();
    renderRows();
    $("#statsNote").textContent = "Nivån visar hur starkt rösten når din mikrofon, så den beror mest på hur nära personen sitter. " +
      "Taltiden räknas från att rösten känns igen. Dagarna sparas i 90 dagar, totalerna tills du nollställer.";
  }

  const visible = () => !$('[data-panel="stats"]').hidden && !document.hidden;
  const load = () => invoke("get_stats").then(render).catch(fail);

  document.addEventListener("hush:view", (e) => { if (e.detail === "stats") load(); });
  setInterval(() => { if (visible()) load(); }, REFRESH_MS);
  window.addEventListener("resize", () => { if (visible() && stats) renderRows(); });

  seg($("#statsSort"), (b) => { sortBy = b.dataset.sort; if (stats) renderRows(); });

  // Wiping everyone's statistics is permanent, so it takes a second click.
  const reset = $("#resetStats");
  let armed = false, disarm;
  reset.addEventListener("click", () => {
    if (!armed) {
      armed = true; reset.classList.add("armed"); reset.textContent = "Klicka igen för att nollställa";
      disarm = setTimeout(() => { armed = false; reset.classList.remove("armed"); reset.textContent = "Nollställ statistiken"; }, 3000);
      return;
    }
    clearTimeout(disarm);
    armed = false; reset.classList.remove("armed"); reset.textContent = "Nollställ statistiken";
    invoke("reset_stats").then((v) => { render(v); toast("Statistiken är nollställd"); }).catch(fail);
  });
})();
