/* sensors-to-mqtt web dashboard.
 *
 * Data: GET api/v1/{status,live,channels,config} and the api/v1/stream SSE
 * feed (read with fetch so the bearer token travels in a header). Dynamic
 * text is set with textContent only. Only the visible mode is redrawn.
 */
"use strict";
(() => {
  const $ = (id) => document.getElementById(id);
  const store = {
    get(k, d) { try { return localStorage.getItem(k) ?? d; } catch { return d; } },
    set(k, v) { try { localStorage.setItem(k, v); } catch { /* blocked */ } },
  };
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  // Canvas colours follow the active theme's CSS variables.
  const C = { font: '"Chakra Petch", ui-sans-serif, system-ui, sans-serif', num: '"Doto", "JetBrains Mono", ui-monospace, monospace' };
  function themeColours() {
    const css = getComputedStyle(document.documentElement);
    for (const k of ["display", "cyan", "amber", "green", "red", "text", "muted", "faint"]) C[k] = css.getPropertyValue(`--${k}`).trim();
    C.grid = css.getPropertyValue("--line").trim();
  }
  const MODES = ["dash", "sensors", "charts", "diag", "config"];
  const STALE_MS = 2000;
  const CHART_DEFAULT = ["lateralG", "longitudinalG", "yawRateDegS", "ambientC"];
  const BALL_TRAIL_MS = 3000;

  const S = {
    mode: MODES.includes(location.hash.slice(1)) ? location.hash.slice(1) : store.get("mode", "dash"),
    fahrenheit: store.get("temp") === "F",
    theme: store.get("theme") || (matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark"),
    token: store.get("token", ""),
    catalog: {}, config: null, status: null,
    channels: {}, fields: {}, lastAt: 0, streamUp: false,
    history: {}, window: Number(store.get("chartWindow", 60)) || 60, paused: false,
    charted: (store.get("charted") || CHART_DEFAULT.join(",")).split(",").filter(Boolean),
    trail: [], peakG: 0,
    prevCounters: null, prevAt: 0, rates: {},
  };

  // ── formatting ────────────────────────────────────────────────────────
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const fmt = (v, d = 1) => (isNum(v) ? v.toFixed(d) : "—");
  const isTemp = (name) => (S.catalog[name]?.unit ?? "") === "°C";
  const tempOut = (name, v) => (isTemp(name) && S.fahrenheit && isNum(v) ? v * 9 / 5 + 32 : v);
  const unitOf = (name) => (isTemp(name) ? (S.fahrenheit ? "°F" : "°C") : S.catalog[name]?.unit ?? "");
  const labelOf = (name) => S.catalog[name]?.label ?? name;
  const decimals = (name) => {
    const u = S.catalog[name]?.unit ?? "";
    return u === "g" ? 2 : u === "°/s" || u === "°" || u === "°C" || u === "%" ? 1 : u === "lx" || u === "µg/m³" || u === "" ? 0 : 2;
  };
  const show = (name, v) => fmt(tempOut(name, v), decimals(name));
  const age = (name) => { const c = S.channels[name]; return c ? performance.now() - c.at : Infinity; };
  const val = (name) => (age(name) <= STALE_MS ? S.channels[name].value : null);

  // ── DOM helpers ───────────────────────────────────────────────────────
  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  }
  function setText(id, v) { const e = $(id); if (e && e.textContent !== v) e.textContent = v; }
  function kv(id, rows) {
    const frag = document.createDocumentFragment();
    for (const [k, v, cls, small] of rows) {
      frag.append(el("dt", null, k));
      const dd = el("dd", cls || null, v);
      if (small) dd.append(el("small", null, small));
      frag.append(dd);
    }
    $(id).replaceChildren(frag);
  }
  function meterInto(box, frac, n, zone) {
    if (box.children.length !== n) box.replaceChildren(...Array.from({ length: n }, () => el("i")));
    const lit = Math.round(Math.max(0, Math.min(1, frac)) * n);
    for (let i = 0; i < n; i++) box.children[i].className = i < lit ? `on ${zone(i / n)}` : "";
  }
  // Signed ranges fill outwards from zero, so -1.7 g looks as big as +1.7 g.
  function bipolarInto(box, v, lo, hi, n, zone) {
    if (box.children.length !== n) box.replaceChildren(...Array.from({ length: n }, () => el("i")));
    const zero = Math.round(((0 - lo) / (hi - lo)) * n), at = Math.round(((Math.max(lo, Math.min(hi, v)) - lo) / (hi - lo)) * n);
    const [a, b] = at < zero ? [at, zero] : [zero, at];
    for (let i = 0; i < n; i++) {
      box.children[i].className = i >= a && i < b ? `on ${zone(Math.abs(i - zero) / n)}` : i === zero || i === zero - 1 ? "mid" : "";
    }
  }
  const ageText = (ms) => (ms == null ? "—" : ms < 10000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.round(ms / 1000)} s`);

  // ── API ───────────────────────────────────────────────────────────────
  const headers = () => (S.token ? { Authorization: `Bearer ${S.token}` } : {});
  async function api(path) {
    const res = await fetch(path, { headers: headers(), cache: "no-store" });
    if (res.status === 401) { askToken(); throw new Error("unauthorized"); }
    return res;
  }
  function askToken() { const d = $("auth"); if (!d.open) d.showModal(); }
  $("auth").addEventListener("close", () => {
    const t = $("auth-token").value.trim();
    if (!t) return;
    S.token = t; store.set("token", t); $("auth-token").value = "";
    restartStream(); loadOnce(); pollStatus();
  });

  let streamCtl = null;
  async function stream() {
    let delay = 1000;
    for (;;) {
      const ctl = new AbortController();
      streamCtl = ctl;
      try {
        const res = await fetch("api/v1/stream", { headers: { ...headers(), Accept: "text/event-stream" }, signal: ctl.signal, cache: "no-store" });
        if (res.status === 401) { setStream(false); askToken(); return; }
        if (!res.ok || !res.body) throw new Error(`HTTP ${res.status}`);
        setStream(true); delay = 1000;
        const reader = res.body.getReader(), dec = new TextDecoder();
        let buf = "";
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          buf += dec.decode(value, { stream: true }).replace(/\r\n/g, "\n");
          let i;
          while ((i = buf.indexOf("\n\n")) >= 0) { dispatch(buf.slice(0, i)); buf = buf.slice(i + 2); }
        }
      } catch {
        if (ctl.signal.aborted) return;
      }
      setStream(false);
      await sleep(delay); delay = Math.min(delay * 2, 15000);
    }
  }
  function restartStream() { if (streamCtl) streamCtl.abort(); stream(); }
  function setStream(up) { S.streamUp = up; schedule(); }
  function dispatch(block) {
    let ev = "message", data = "";
    for (const line of block.split("\n")) {
      if (line.startsWith("event:")) ev = line.slice(6).trim();
      else if (line.startsWith("data:")) data += line.slice(5).trimStart();
    }
    if (ev !== "sample" || !data) return;
    try { onSample(JSON.parse(data)); } catch { /* malformed */ }
  }

  function onSample(s) {
    const now = performance.now();
    for (const [k, v] of Object.entries(s.channels || {})) {
      S.channels[k] = { value: v, at: now, sensor: s.sensor };
      if (!S.paused) (S.history[k] ||= []).push([now, v]);
    }
    if (s.sensor) S.fields[s.sensor] = { fields: s.fields || {}, at: now, driver: s.driver };
    const lat = s.channels?.lateralG, lon = s.channels?.longitudinalG;
    if (isNum(lat) && isNum(lon)) {
      S.trail.push([now, lat, lon]);
      S.peakG = Math.max(S.peakG, Math.hypot(lat, lon));
    }
    S.lastAt = now;
    schedule();
  }

  function trimHistory() {
    const cutoff = performance.now() - 125000;
    for (const h of Object.values(S.history)) {
      let i = 0;
      while (i < h.length && h[i][0] < cutoff) i++;
      if (i) h.splice(0, i);
    }
  }

  async function loadOnce() {
    try {
      const [cat, cfg, live] = await Promise.all([api("api/v1/channels"), api("api/v1/config"), api("api/v1/live")]);
      if (cat.ok) for (const c of await cat.json()) S.catalog[c.name] = c;
      if (cfg.ok) S.config = await cfg.json();
      if (live.ok) {
        const l = await live.json(), now = performance.now();
        for (const [k, c] of Object.entries(l.channels || {})) S.channels[k] = { value: c.value, at: now - c.ageMs, sensor: c.sensor };
        for (const [name, f] of Object.entries(l.fields || {})) S.fields[name] = { fields: f, at: now };
      }
    } catch { /* retried by pollStatus */ }
    configRendered = false;
    schedule();
  }

  async function pollStatus() {
    try {
      const r = await api("api/v1/status");
      if (r.ok) {
        const st = await r.json(), now = performance.now();
        if (S.prevCounters) {
          const dt = (now - S.prevAt) / 1000;
          for (const [k, v] of Object.entries(st.counters)) S.rates[k] = (v - (S.prevCounters[k] ?? v)) / dt;
        }
        S.prevCounters = st.counters; S.prevAt = now; S.status = st;
        if (!S.config) loadOnce();
      }
    } catch { /* lamps show it */ }
    schedule();
  }

  // ── rendering ─────────────────────────────────────────────────────────
  let queued = false;
  function schedule() { if (!queued) { queued = true; requestAnimationFrame(render); } }
  function render() {
    queued = false;
    renderBar(); renderReadout();
    ({ dash: renderDash, sensors: renderSensors, charts: renderCharts, diag: renderDiag, config: renderConfig })[S.mode]();
  }

  function lamp(id, state) { $(id).className = `lamp ${state}`; }
  function renderBar() {
    const st = S.status, h = st?.health;
    setText("version", st?.info?.version ? `v${st.info.version}` : "");
    lamp("lamp-sensors", !h ? "" : h.status === "ok" ? "ok" : h.status === "degraded" ? "warn" : "bad");
    setText("lamp-sensors-t", h ? `Sensors ${h.sensors_live}/${h.sensors}` : "Sensors");
    lamp("lamp-mqtt", !h || h.mqtt === "disabled" ? "" : h.mqtt === "connected" ? "ok" : h.mqtt === "connecting" ? "warn" : "bad");
    lamp("lamp-link", S.streamUp ? "ok" : "bad");
    setText("health", h ? h.status.replace("_", " ") : "—");
    setText("foot-source", st ? (st.info.mqtt ? `mqtt ${st.info.mqtt}` : "mqtt disabled") : "connecting…");
    if (h) setText("foot-uptime", `up ${Math.floor(h.uptime_s / 3600)}h ${String(Math.floor(h.uptime_s / 60) % 60).padStart(2, "0")}m`);
  }

  function renderReadout() {
    document.querySelector(".readout").classList.toggle("stale", performance.now() - S.lastAt > STALE_MS);
    setText("r-lat", show("lateralG", val("lateralG")));
    setText("r-lon", show("longitudinalG", val("longitudinalG")));
    const lat = val("lateralG"), lon = val("longitudinalG");
    const comb = isNum(val("combinedG")) ? val("combinedG") : isNum(lat) && isNum(lon) ? Math.hypot(lat, lon) : null;
    setText("r-comb", fmt(comb, 2));
    setText("r-comb-sub", S.peakG > 0 ? `peak ${S.peakG.toFixed(2)} g` : " ");
    setText("r-yaw", show("yawRateDegS", val("yawRateDegS")));
    setText("r-amb", show("ambientC", val("ambientC")));
    setText("r-amb-u", S.fahrenheit ? "°F" : "°C");
    const hum = val("humidityPct"), baro = val("baroKpa");
    setText("r-amb-sub", isNum(hum) ? `${hum.toFixed(0)} % RH${isNum(baro) ? ` · ${baro.toFixed(1)} kPa` : ""}` : isNum(baro) ? `${baro.toFixed(1)} kPa` : " ");
    const h = S.status?.health;
    setText("r-rate", h ? fmt(h.sample_rate_hz, 0) : "—");
    setText("r-rate-sub", h ? `${h.sensors_live} of ${h.sensors} sensors` : " ");
  }

  // Dash ------------------------------------------------------------------
  function zoneFor(name, v) {
    if (!isNum(v)) return () => "";
    const u = S.catalog[name]?.unit;
    if (u === "g") return (f) => (Math.abs(v) > 1.2 ? "r" : Math.abs(v) > 0.8 ? "a" : "");
    if (name === "batteryV" || name === "busV") return () => (v < 11.5 || v > 15.2 ? "r" : v < 12.3 ? "a" : "");
    if (name === "pm25" || name === "pm10") return () => (v > 55 ? "r" : v > 25 ? "a" : "");
    return () => "";
  }
  function drawBall() {
    const cv = $("gball"), dpr = window.devicePixelRatio || 1, w = cv.clientWidth, h = cv.clientHeight;
    if (!w || !h) return;
    if (cv.width !== Math.round(w * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
    const ctx = cv.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, w, h);
    const cx = w / 2, cy = h / 2, r = Math.min(w, h) / 2 - 14, maxG = 1.5, px = r / maxG;
    ctx.strokeStyle = C.grid; ctx.lineWidth = 1;
    for (const g of [0.5, 1.0, 1.5]) { ctx.beginPath(); ctx.arc(cx, cy, g * px, 0, Math.PI * 2); ctx.stroke(); }
    ctx.beginPath(); ctx.moveTo(cx - r, cy); ctx.lineTo(cx + r, cy); ctx.moveTo(cx, cy - r); ctx.lineTo(cx, cy + r); ctx.stroke();
    ctx.fillStyle = C.faint; ctx.font = `10px ${C.num}`;
    for (const g of [0.5, 1.0, 1.5]) ctx.fillText(`${g}`, cx + g * px + 3, cy - 3);
    const now = performance.now();
    while (S.trail.length && S.trail[0][0] < now - BALL_TRAIL_MS) S.trail.shift();
    // longitudinal: acceleration up, braking down; lateral: right is right
    const at = ([, lat, lon]) => [cx + Math.max(-maxG, Math.min(maxG, lat)) * px, cy - Math.max(-maxG, Math.min(maxG, lon)) * px];
    if (S.trail.length > 1) {
      ctx.strokeStyle = C.cyan; ctx.globalAlpha = 0.45; ctx.lineWidth = 1.5; ctx.beginPath();
      S.trail.forEach((p, i) => { const [x, y] = at(p); i ? ctx.lineTo(x, y) : ctx.moveTo(x, y); });
      ctx.stroke(); ctx.globalAlpha = 1;
    }
    const lat = val("lateralG"), lon = val("longitudinalG");
    if (isNum(lat) && isNum(lon)) {
      const [x, y] = at([0, lat, lon]);
      ctx.fillStyle = Math.hypot(lat, lon) > 1.2 ? C.red : Math.hypot(lat, lon) > 0.8 ? C.amber : C.green;
      ctx.beginPath(); ctx.arc(x, y, 7, 0, Math.PI * 2); ctx.fill();
      setText("gball-note", `lateral ${lat.toFixed(2)} · longitudinal ${lon.toFixed(2)} g`);
    } else {
      ctx.fillStyle = C.faint; ctx.font = `12px ${C.font}`; ctx.fillText("no motion sensor data", 12, 20);
      setText("gball-note", "lateral × longitudinal");
    }
  }
  function renderDash() {
    drawBall();
    const present = Object.keys(S.catalog).filter((n) => S.channels[n]);
    const box = $("dash-gauges");
    if (box.dataset.keys !== present.join(",")) {
      box.dataset.keys = present.join(",");
      box.replaceChildren(...present.map((n) => {
        const a = el("article", "panel gauge");
        a.dataset.name = n;
        const v = el("span", "val"); v.append(el("b", null, "—"), el("small", null, unitOf(n)));
        const scale = el("div", "scale"); scale.append(el("span"), el("span"));
        a.append(el("span", "lbl", labelOf(n)), v, el("div", "meter"), scale, el("span", "sub", " "));
        return a;
      }));
    }
    for (const card of box.children) {
      const n = card.dataset.name, c = S.catalog[n], rawV = val(n), v = tempOut(n, rawV);
      card.classList.toggle("stale", rawV == null);
      card.querySelector("b").textContent = show(n, rawV);
      card.querySelector("small").textContent = unitOf(n);
      const lo = tempOut(n, c?.min ?? 0), hi = tempOut(n, c?.max || 100);
      const m = card.querySelector(".meter, .bipolar");
      if (lo < 0 && hi > 0) {
        m.className = "bipolar";
        bipolarInto(m, isNum(v) ? v : 0, lo, hi, 24, zoneFor(n, rawV));
      } else {
        meterInto(m, isNum(v) && hi > lo ? (v - lo) / (hi - lo) : 0, 24, zoneFor(n, rawV));
      }
      const [a, b] = card.querySelectorAll(".scale span");
      a.textContent = fmt(lo, 0); b.textContent = fmt(hi, 0);
      card.querySelector(".sub").textContent = rawV == null && S.channels[n] ? `stale ${(age(n) / 1000).toFixed(1)} s` : S.channels[n]?.sensor ?? " ";
    }
  }

  // Sensors ---------------------------------------------------------------
  function renderSensors() {
    const now = performance.now();
    const names = Object.keys(S.channels).sort();
    $("ch-table").tBodies[0].replaceChildren(...names.map((n) => {
      const c = S.channels[n], a = now - c.at, tr = el("tr", a > STALE_MS ? "stale" : null);
      tr.append(el("td", null, labelOf(n)), el("td", "n v", show(n, c.value)), el("td", null, unitOf(n)),
        el("td", null, c.sensor ?? ""), el("td", "n age", a < 10000 ? `${(a / 1000).toFixed(1)} s` : "—"));
      return tr;
    }));
    const sensors = S.status?.sensors || [];
    $("sensor-cards").replaceChildren(...sensors.map((st) => {
      const a = el("article", "panel"), head = el("div", "sensor-head");
      const ind = el("span", `ind${st.connected ? " on" : st.enabled ? " on r" : ""}`);
      ind.append(el("i"), document.createTextNode(!st.enabled ? "disabled" : st.connected ? `${fmt(st.rate_hz, 0)} Hz` : "offline"));
      head.append(el("h2", null, st.name), ind);
      a.append(head);
      if (st.last_error) a.append(el("p", "err", st.last_error));
      const dl = el("dl", "kv");
      dl.append(el("dt", null, "Driver"), el("dd", null, st.driver), el("dt", null, "Connection"), el("dd", null, st.connection_display));
      const f = S.fields[st.name]?.fields || {};
      for (const k of Object.keys(f).sort()) {
        dl.append(el("dt", null, k), el("dd", null, isNum(f[k]) ? String(Math.round(f[k] * 1000) / 1000) : String(f[k])));
      }
      a.append(dl);
      return a;
    }));
  }

  // Charts ----------------------------------------------------------------
  function renderPicker() {
    const names = Object.keys(S.catalog).filter((n) => S.channels[n]);
    const box = $("chart-picker");
    if (box.dataset.keys === names.join(",")) return;
    box.dataset.keys = names.join(",");
    box.replaceChildren(...names.map((n) => {
      const l = el("label"), cb = el("input");
      cb.type = "checkbox"; cb.checked = S.charted.includes(n);
      cb.addEventListener("change", () => {
        S.charted = cb.checked ? [...S.charted, n] : S.charted.filter((x) => x !== n);
        store.set("charted", S.charted.join(",")); schedule();
      });
      l.append(cb, document.createTextNode(labelOf(n)));
      return l;
    }));
  }
  function renderCharts() {
    renderPicker();
    document.querySelectorAll("[data-window]").forEach((k) => k.classList.toggle("active", Number(k.dataset.window) === S.window));
    const shown = S.charted.filter((n) => S.channels[n]);
    const box = $("charts");
    if (box.dataset.keys !== shown.join(",")) {
      box.dataset.keys = shown.join(",");
      box.replaceChildren(...shown.map((n) => {
        const a = el("article", "panel chart"); a.dataset.name = n;
        const lg = el("div", "legend-box"); lg.append(el("span", "lbl", labelOf(n)), el("b", null, "—"), el("span", "sub", unitOf(n)));
        a.append(lg, el("canvas"));
        return a;
      }));
    }
    setText("chart-note", `${S.window} s window${S.paused ? " · paused" : ""}`);
    const now = performance.now(), span = S.window * 1000;
    for (const card of box.children) {
      const n = card.dataset.name, h = S.history[n] || [];
      card.querySelector("b").textContent = show(n, val(n));
      card.querySelector(".sub").textContent = unitOf(n);
      drawChart(card.querySelector("canvas"), h.filter((p) => p[0] >= now - span), now, span, n);
    }
  }
  function drawChart(cv, pts, now, span, name) {
    const dpr = window.devicePixelRatio || 1, w = cv.clientWidth, h = cv.clientHeight;
    if (!w || !h) return;
    if (cv.width !== Math.round(w * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
    const ctx = cv.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0); ctx.clearRect(0, 0, w, h);
    if (pts.length < 2) { ctx.fillStyle = C.faint; ctx.font = `12px ${C.font}`; ctx.fillText("collecting…", 10, h / 2); return; }
    let lo = Infinity, hi = -Infinity;
    for (const [, v] of pts) { const t = tempOut(name, v); if (t < lo) lo = t; if (t > hi) hi = t; }
    if (hi - lo < 1e-6) { hi += 1; lo -= 1; }
    const pad = (hi - lo) * 0.1; lo -= pad; hi += pad;
    const x = (t) => w - ((now - t) / span) * w, y = (v) => h - 4 - ((tempOut(name, v) - lo) / (hi - lo)) * (h - 8);
    ctx.strokeStyle = C.grid; ctx.lineWidth = 1; ctx.beginPath();
    for (let i = 1; i < 4; i++) { ctx.moveTo(0, (h * i) / 4); ctx.lineTo(w, (h * i) / 4); }
    ctx.stroke();
    ctx.fillStyle = C.muted; ctx.font = `10px ${C.num}`;
    ctx.fillText(fmt(hi, 2), 4, 12); ctx.fillText(fmt(lo, 2), 4, h - 4);
    ctx.strokeStyle = C.cyan; ctx.lineWidth = 1.6; ctx.beginPath();
    pts.forEach(([t, v], i) => (i ? ctx.lineTo(x(t), y(v)) : ctx.moveTo(x(t), y(v))));
    ctx.stroke();
  }

  // Diagnostics -----------------------------------------------------------
  function renderDiag() {
    const sensors = S.status?.sensors || [];
    $("diag-sensors").tBodies[0].replaceChildren(...sensors.map((st) => {
      const tr = el("tr", st.enabled && !st.connected ? "stale" : null);
      tr.append(el("td", null, st.name), el("td", null, st.driver), el("td", null, st.connection_display),
        el("td", "n", st.enabled ? `${fmt(st.rate_hz, 0)} Hz` : "off"), el("td", "n age", ageText(st.ageMs)));
      return tr;
    }));
    const h = S.status?.health;
    kv("diag-link", [
      ["Health", h?.status || "—", h?.status === "ok" ? "ok" : "warn", h ? `${h.sensors_live}/${h.sensors} sensors` : ""],
      ["Reading rate", h ? `${fmt(h.sample_rate_hz, 0)} Hz` : "—"],
      ["Last reading", h && isNum(h.sample_age_s) ? `${(h.sample_age_s * 1000).toFixed(0)} ms ago` : "—", h && h.sample_age_s > 2 ? "warn" : null],
      ["MQTT", h?.mqtt || "—", h?.mqtt === "connected" ? "ok" : h?.mqtt === "disabled" ? null : "warn"],
    ]);
    const c = S.status?.counters || {}, r = (k) => (isNum(S.rates[k]) ? `${S.rates[k].toFixed(1)}/s` : "");
    kv("diag-counters", [
      ["Readings", String(c.readings ?? "—"), null, r("readings")],
      ["Read errors", String(c.read_errors ?? "—"), c.read_errors ? "warn" : null, r("read_errors")],
      ["Reconnects", String(c.reconnects ?? "—"), c.reconnects ? "warn" : null],
      ["MQTT published", String(c.mqtt_published ?? "—"), null, r("mqtt_published")],
      ["MQTT dropped", String(c.mqtt_dropped ?? "—"), c.mqtt_dropped ? "warn" : null, r("mqtt_dropped")],
    ]);
  }

  // Configuration ---------------------------------------------------------
  let configRendered = false;
  const flat = (v) => (v == null ? "unset" : Array.isArray(v) ? v.join(", ") : typeof v === "object" ? JSON.stringify(v) : String(v));
  function renderConfig() {
    const st = S.status;
    if (st) kv("cfg-wiring", [["MQTT", st.info.mqtt || "disabled"], ...st.info.outputs.map((o, i) => [i ? "" : "Outputs", o])]);
    if (!$("cfg-endpoints").children.length) {
      const eps = [["health", "open"], ["metrics", "open, Prometheus"], ["api/v1/status", "token"], ["api/v1/live", "token"],
        ["api/v1/channels", "token"], ["api/v1/config", "token"], ["api/v1/stream", "token, SSE"]];
      $("cfg-endpoints").replaceChildren(...eps.map(([p, note]) => {
        const li = el("li"), a = el("a", null, `/${p}`); a.href = p; a.target = "_blank"; a.rel = "noopener";
        li.append(a, el("small", null, note)); return li;
      }));
    }
    if (!S.config || configRendered) return;
    configRendered = true;
    const sec = (title, obj) => {
      const s = el("section"), dl = el("dl", "kv");
      s.append(el("h3", null, title), dl);
      for (const k of Object.keys(obj).sort()) {
        const v = obj[k];
        dl.append(el("dt", null, k), el("dd", v == null ? "unset" : null, flat(v)));
      }
      return s;
    };
    const top = Object.fromEntries(Object.entries(S.config).filter(([k, v]) => typeof v !== "object" || v === null));
    const parts = [sec("General", top), sec("MQTT", S.config.mqtt || {}), sec("HTTP", S.config.http || {})];
    for (const s of S.config.sensors || []) {
      parts.push(sec(`Sensor ${s.name}`, { driver: s.driver, enabled: s.enabled, connection: s.connection, settings: s.settings, channels: s.channels }));
    }
    $("cfg-groups").replaceChildren(...parts);
  }

  // ── controls ──────────────────────────────────────────────────────────
  function setMode(m) {
    S.mode = m; store.set("mode", m);
    if (location.hash !== `#${m}`) history.replaceState(null, "", `#${m}`);
    for (const id of MODES) $(`mode-${id}`).hidden = id !== m;
    document.querySelectorAll(".key[data-mode]").forEach((k) => k.classList.toggle("active", k.dataset.mode === m));
    schedule();
  }
  document.querySelectorAll(".key[data-mode]").forEach((k) => k.addEventListener("click", () => setMode(k.dataset.mode)));
  window.addEventListener("hashchange", () => { const m = location.hash.slice(1); if (MODES.includes(m)) setMode(m); });
  document.addEventListener("keydown", (e) => {
    if (e.target.closest("input, dialog")) return;
    const i = "12345".indexOf(e.key);
    if (i >= 0) setMode(MODES[i]);
  });
  function applyTemp() { setText("k-temp-t", S.fahrenheit ? "°F" : "°C"); document.querySelectorAll("#dash-gauges,#charts").forEach((b) => { b.dataset.keys = ""; }); schedule(); }
  $("k-temp").addEventListener("click", () => { S.fahrenheit = !S.fahrenheit; store.set("temp", S.fahrenheit ? "F" : "C"); applyTemp(); });
  function applyTheme() {
    document.documentElement.dataset.theme = S.theme;
    document.querySelector('meta[name="theme-color"]').content = S.theme === "light" ? "#e9ecee" : "#08090a";
    setText("k-theme-t", S.theme === "light" ? "Light" : "Dark");
    themeColours(); schedule();
  }
  $("k-theme").addEventListener("click", () => { S.theme = S.theme === "light" ? "dark" : "light"; store.set("theme", S.theme); applyTheme(); });
  document.querySelectorAll("[data-window]").forEach((k) => k.addEventListener("click", () => {
    S.window = Number(k.dataset.window); store.set("chartWindow", String(S.window)); schedule();
  }));
  $("k-chart-pause").addEventListener("click", () => {
    S.paused = !S.paused;
    $("k-chart-pause").classList.toggle("active", S.paused);
    $("k-chart-pause").querySelector("span").textContent = S.paused ? "Resume" : "Pause";
    schedule();
  });

  new ResizeObserver(schedule).observe(document.body);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) { pollStatus(); schedule(); } });

  applyTheme(); applyTemp(); setMode(S.mode);
  loadOnce(); pollStatus(); stream();
  setInterval(pollStatus, 2000);
  setInterval(trimHistory, 5000);
  setInterval(() => { if (S.mode !== "config") schedule(); }, 250);
})();
