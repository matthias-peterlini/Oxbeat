// ---------- A small playable piano, with the same keys as the app ----------
(() => {
  const piano = document.getElementById("piano");
  if (!piano) return;
  const NAMES = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
  const noteEl = document.getElementById("play-note");
  const octaveEl = document.getElementById("play-octave");
  const keys = new Map([...piano.querySelectorAll(".key")].map((k) => [k.dataset.key, k]));
  let octave = 4;
  let ctx = null, out = null;
  const voices = new Map();

  function audio() {
    if (ctx) return ctx;
    ctx = new (window.AudioContext || window.webkitAudioContext)();
    const master = ctx.createGain();
    master.gain.value = 0.32;
    const comp = ctx.createDynamicsCompressor();
    // A short echo gives the notes some room.
    const delay = ctx.createDelay(1);
    delay.delayTime.value = 0.27;
    const feedback = ctx.createGain();
    feedback.gain.value = 0.28;
    const wet = ctx.createGain();
    wet.gain.value = 0.22;
    out = ctx.createGain();
    out.connect(master);
    out.connect(delay);
    delay.connect(feedback).connect(delay);
    delay.connect(wet).connect(master);
    master.connect(comp).connect(ctx.destination);
    return ctx;
  }

  function on(note) {
    if (voices.has(note)) return;
    const ac = audio();
    if (ac.state === "suspended") ac.resume();
    const midi = 12 * (octave + 1) + note;
    const f = 440 * Math.pow(2, (midi - 69) / 12);
    const t = ac.currentTime;
    const env = ac.createGain();
    env.gain.setValueAtTime(0, t);
    env.gain.linearRampToValueAtTime(0.9, t + 0.006);
    env.gain.exponentialRampToValueAtTime(0.35, t + 0.5);
    const filter = ac.createBiquadFilter();
    filter.type = "lowpass";
    filter.frequency.setValueAtTime(Math.min(9000, f * 9), t);
    filter.frequency.exponentialRampToValueAtTime(Math.min(4000, f * 3), t + 0.6);
    const a = ac.createOscillator();
    a.type = "triangle";
    a.frequency.value = f;
    const b = ac.createOscillator();
    b.type = "sine";
    b.frequency.value = f * 2;
    b.detune.value = 4;
    const bGain = ac.createGain();
    bGain.gain.value = 0.25;
    a.connect(filter);
    b.connect(bGain).connect(filter);
    filter.connect(env).connect(out);
    a.start(t);
    b.start(t);
    voices.set(note, { env, oscs: [a, b] });
    piano.querySelector(`[data-note="${note}"]`)?.classList.add("on");
    noteEl.textContent = NAMES[midi % 12] + (Math.floor(midi / 12) - 1);
  }

  function off(note) {
    const v = voices.get(note);
    if (!v) return;
    voices.delete(note);
    const t = ctx.currentTime;
    v.env.gain.cancelScheduledValues(t);
    v.env.gain.setValueAtTime(Math.max(v.env.gain.value, 0.0001), t);
    v.env.gain.exponentialRampToValueAtTime(0.0001, t + 0.35);
    v.oscs.forEach((o) => o.stop(t + 0.4));
    piano.querySelector(`[data-note="${note}"]`)?.classList.remove("on");
  }

  const allOff = () => [...voices.keys()].forEach(off);
  const setOctave = (o) => {
    allOff();
    octave = Math.max(1, Math.min(7, o));
    octaveEl.textContent = `Octave ${octave}`;
  };

  // Computer keyboard, the same letters as in the app.
  const typing = (e) => e.target.closest?.("input, textarea, select, [contenteditable]");
  window.addEventListener("keydown", (e) => {
    if (e.repeat || e.ctrlKey || e.metaKey || e.altKey || typing(e)) return;
    const k = e.key.toLowerCase();
    if (k === "z") { setOctave(octave - 1); e.preventDefault(); return; }
    if (k === "x") { setOctave(octave + 1); e.preventDefault(); return; }
    const key = keys.get(k);
    if (!key) return;
    e.preventDefault();
    on(Number(key.dataset.note));
  });
  window.addEventListener("keyup", (e) => {
    const key = keys.get(e.key.toLowerCase());
    if (key) off(Number(key.dataset.note));
  });
  window.addEventListener("blur", allOff);

  // Mouse and touch, including sliding a finger across the keys.
  const pointers = new Map();
  const press = (pid, key) => {
    const prev = pointers.get(pid);
    if (prev === key) return;
    if (prev) off(Number(prev.dataset.note));
    pointers.set(pid, key);
    if (key) on(Number(key.dataset.note));
  };
  const release = (e) => {
    const prev = pointers.get(e.pointerId);
    if (prev) off(Number(prev.dataset.note));
    pointers.delete(e.pointerId);
  };
  piano.addEventListener("pointerdown", (e) => {
    const key = e.target.closest(".key");
    if (!key) return;
    e.preventDefault();
    try { piano.setPointerCapture(e.pointerId); } catch {}
    press(e.pointerId, key);
  });
  piano.addEventListener("pointermove", (e) => {
    if (!pointers.has(e.pointerId)) return;
    const key = document.elementFromPoint(e.clientX, e.clientY)?.closest(".key") || null;
    press(e.pointerId, key);
  });
  ["pointerup", "pointercancel", "lostpointercapture"].forEach((t) => piano.addEventListener(t, release));
  // Keyboard users: Enter or Space on a focused key plays it.
  piano.addEventListener("click", (e) => {
    const key = e.target.closest(".key");
    if (!key || e.detail !== 0) return;
    const n = Number(key.dataset.note);
    on(n);
    setTimeout(() => off(n), 300);
  });
})();

// ---------- Downloads: link straight to the files of the latest release ----------
(() => {
  const ua = navigator.userAgent;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  let os = null;
  if (/Win/i.test(platform) || /Windows/i.test(ua)) os = "windows";
  else if (/Mac/i.test(platform) && !/iPhone|iPad/i.test(ua)) os = "mac";
  else if (/Linux/i.test(platform) && !/Android/i.test(ua)) os = "linux";

  const names = { windows: "Windows", mac: "macOS", linux: "Linux" };
  const heroBtn = document.getElementById("hero-dl");
  const heroLabel = heroBtn.querySelector(".button-label");
  const heroSub = document.getElementById("hero-dl-sub");
  if (os) {
    heroLabel.textContent = `Download for ${names[os]}`;
    const row = document.querySelector(`.downloads li[data-os="${os === "mac" ? "mac-arm" : os}"]`);
    row && row.classList.add("mine");
    if (os === "mac") document.querySelector('.downloads li[data-os="mac-intel"]').classList.add("mine");
  }

  const match = {
    windows: (n) => /\.exe$/i.test(n),
    "mac-arm": (n) => /aarch64.*\.dmg$/i.test(n),
    "mac-intel": (n) => /x86_64.*\.dmg$/i.test(n),
    linux: (n) => /\.appimage$/i.test(n),
    deb: (n) => /\.deb$/i.test(n),
  };
  const mb = (bytes) => `${Math.max(1, Math.round(bytes / 1048576))} MB`;

  fetch("https://api.github.com/repos/matthias-peterlini/Oxbeat/releases/latest", {
    headers: { Accept: "application/vnd.github+json" },
  })
    .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
    .then((rel) => {
      const version = String(rel.tag_name || "").replace(/^v/, "");
      if (version) {
        document.getElementById("dl-version").textContent = version;
        document.getElementById("meta-version").textContent = version;
      }
      const found = {};
      document.querySelectorAll(".downloads li").forEach((li) => {
        const asset = (rel.assets || []).find((a) => match[li.dataset.os](a.name));
        if (!asset) return;
        found[li.dataset.os] = asset;
        const a = li.querySelector(".dl-get");
        a.href = asset.browser_download_url;
        a.querySelector(".dl-size").textContent = mb(asset.size);
        a.setAttribute("aria-label", `Download ${asset.name}, ${mb(asset.size)}`);
      });
      // On Windows there's only one file, so the big button can fetch it directly.
      if (os === "windows" && found.windows) {
        heroBtn.href = found.windows.browser_download_url;
        heroSub.textContent = `Version ${version} · ${mb(found.windows.size)}`;
      } else if (version) {
        heroSub.textContent = `Version ${version} · free, no account`;
      }
    })
    .catch(() => {});
})();
