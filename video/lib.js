// Shared toolkit for the QuipKeys pitch-video scenes.
// Every scene is a pure function of time: window.scene = { duration, render(t) }.
// The renderer (render.js) calls render(t) for each frame and screenshots it,
// so nothing may use CSS transitions/animations or real timers.

const W = 1920, H = 1080;

// ---------- easing / interpolation ----------
function clamp(x, a = 0, b = 1) { return Math.max(a, Math.min(b, x)); }
function bezier(x1, y1, x2, y2) {
  // cubic-bezier easing like CSS; solve x(t) = p by Newton iterations
  const cx = 3 * x1, bx = 3 * (x2 - x1) - cx, ax = 1 - cx - bx;
  const cy = 3 * y1, by = 3 * (y2 - y1) - cy, ay = 1 - cy - by;
  const sx = (t) => ((ax * t + bx) * t + cx) * t, sy = (t) => ((ay * t + by) * t + cy) * t;
  const dx = (t) => (3 * ax * t + 2 * bx) * t + cx;
  return (p) => {
    let t = p;
    for (let i = 0; i < 8; i++) { const e = sx(t) - p, d = dx(t); if (Math.abs(e) < 1e-5 || Math.abs(d) < 1e-6) break; t -= e / d; }
    return sy(clamp(t));
  };
}
const ease = {
  out: bezier(0.16, 1, 0.3, 1),
  inOut: bezier(0.65, 0, 0.35, 1),
  in: bezier(0.55, 0, 1, 0.45),
  linear: (x) => x,
  // damped spring overshoot (settles to 1)
  spring: (x) => 1 - Math.exp(-6.5 * x) * Math.cos(9.5 * x),
};
// interpolate(t, [t0, t1], [v0, v1], easing) with clamping
function lerp(t, [t0, t1], [v0, v1], e = ease.out) {
  const p = t1 === t0 ? (t >= t1 ? 1 : 0) : clamp((t - t0) / (t1 - t0));
  return v0 + (v1 - v0) * e(p);
}
// seeded random
function rng(seed) { let s = seed >>> 0; return () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296); }

// ---------- DOM helpers ----------
function el(tag, cls, parent, html) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html !== undefined) e.innerHTML = html;
  (parent || document.getElementById("stage")).appendChild(e);
  return e;
}
function place(e, x, y, extra = "") { e.style.transform = `translate(${x}px, ${y}px) ${extra}`; }
function show(e, o) { e.style.opacity = o; }

// ---------- keyboard ----------
const ROWS = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
// Build a keyboard; returns {root, keys: {char: el}, pos: {char: [x,y]}}
function keyboard(parent, x, y, k = 120, gap = 14, opts = {}) {
  const root = el("div", "kb", parent);
  place(root, x, y);
  const keys = {}, pos = {};
  const offs = [0, 0.35, 0.85];
  ROWS.forEach((row, r) => {
    [...row].forEach((c, i) => {
      const key = el("div", "cap", root, `<span>${c}</span>`);
      const kx = (offs[r] * k) + i * (k + gap), ky = r * (k + gap);
      key.style.left = kx + "px"; key.style.top = ky + "px";
      key.style.width = key.style.height = k + "px";
      key.style.fontSize = Math.round(k * 0.34) + "px";
      keys[c] = key; pos[c] = [kx + k / 2, ky + k / 2];
    });
  });
  if (opts.space !== false) {
    const sp = el("div", "cap space", root);
    const sx = 2.2 * k, sy = 3 * (k + gap);
    sp.style.left = sx + "px"; sp.style.top = sy + "px"; sp.style.width = (5.6 * k) + "px"; sp.style.height = k + "px";
    keys[" "] = sp; pos[" "] = [sx + 2.8 * k, sy + k / 2];
  }
  return { root, keys, pos, k };
}
// press amount 0..1 (0 = up)
function press(key, amt, tint) {
  if (!key) return;
  const d = Math.round(amt * 7);
  key.style.transform = `translateY(${d}px)`;
  key.style.boxShadow = `inset 0 -${8 - d}px 0 var(--cap-edge), 0 ${7 - d}px 0 var(--cap-shadow), 0 ${10 - d}px ${16 - d}px rgba(23,20,10,.22)`;
  key.style.background = tint || "";
}
// a short press envelope: down fast, up slower
function tap(t, at, dur = 0.16) { if (t < at || t > at + dur) return 0; const p = (t - at) / dur; return p < 0.3 ? p / 0.3 : 1 - (p - 0.3) / 0.7; }

// ---------- text ----------
// typed text helper: how many characters of `s` are visible at time t,
// typing from `start` with `cps` characters per second.
function typed(s, t, start, cps = 14) { return s.slice(0, Math.max(0, Math.floor((t - start) * cps))); }
function esc(s) { return s.replace(/&/g, "&amp;").replace(/</g, "&lt;"); }
