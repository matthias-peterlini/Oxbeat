// Equalizer bars behind the hero.
(() => {
  const eq = document.querySelector(".eq");
  if (!eq) return;
  const n = Math.min(64, Math.floor(window.innerWidth / 14));
  for (let i = 0; i < n; i++) {
    const s = document.createElement("span");
    const wave = Math.sin((i / n) * Math.PI);
    s.style.setProperty("--h", `${18 + wave * 90 + Math.random() * 14}px`);
    s.style.setProperty("--d", `${0.5 + Math.random() * 0.9}s`);
    s.style.setProperty("--delay", `${-Math.random() * 2}s`);
    eq.appendChild(s);
  }
})();

// The tour tabs.
(() => {
  // Fetch the other screens early so switching is instant.
  ["editor", "effects", "mixer"].forEach((n) => { new Image().src = `/img/${n}.png`; });
  const captions = {
    editor: "Draw notes, drag them around and fix their timing in one click.",
    beats: "Lay out your song: every instrument and voice gets its own lane of clips.",
    effects: "Sound flows left to right through big, friendly knobs.",
    mixer: "A fader, pan and meter for every track, and a limiter on the master.",
  };
  const tabs = [...document.querySelectorAll(".tabs button")];
  const img = document.getElementById("shot-img");
  const caption = document.getElementById("shot-caption");
  const select = (tab) => {
    tabs.forEach((t) => t.setAttribute("aria-selected", String(t === tab)));
    img.style.opacity = "0.4";
    img.onload = img.onerror = () => { img.style.opacity = "1"; };
    img.src = `/img/${tab.dataset.img}.png`;
    img.alt = tab.dataset.alt;
    caption.textContent = captions[tab.dataset.img];
  };
  tabs.forEach((t, i) => {
    t.addEventListener("click", () => select(t));
    t.addEventListener("keydown", (e) => {
      const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      if (!d) return;
      const next = tabs[(i + d + tabs.length) % tabs.length];
      next.focus();
      select(next);
    });
  });
})();

// Point the main button at the visitor's own system.
(() => {
  const ua = navigator.userAgent;
  const platform = navigator.userAgentData?.platform || navigator.platform || "";
  let os = null, label = null;
  if (/Win/i.test(platform) || /Windows/i.test(ua)) { os = "windows"; label = "Download for Windows"; }
  else if (/Mac/i.test(platform) && !/iPhone|iPad/i.test(ua)) { os = "mac"; label = "Download for macOS"; }
  else if (/Linux/i.test(platform) && !/Android/i.test(ua)) { os = "linux"; label = "Download for Linux"; }
  if (!os) return;
  document.querySelector(`.platform[data-os="${os}"]`)?.classList.add("mine");
  const btn = document.querySelector("#hero-download span");
  if (btn) btn.textContent = label;
})();
