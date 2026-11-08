// Render scenes to 1080p30 MP4 (H.264), no audio.
//   node video/render.js                 -> every scene in video/scenes
//   node video/render.js 01 04           -> scenes whose file name starts with these
//   node video/render.js --stills 01     -> only contact-sheet stills (for checking)
const { chromium } = require("playwright");
const { spawn } = require("child_process");
const fs = require("fs"), path = require("path");

const FPS = 30;
const dir = path.join(__dirname, "scenes"), out = path.join(__dirname, "out");
const args = process.argv.slice(2);
const stillsOnly = args.includes("--stills");
const picks = args.filter((a) => !a.startsWith("--"));
const files = fs.readdirSync(dir).filter((f) => f.endsWith(".html") && (!picks.length || picks.some((p) => f.startsWith(p)))).sort();

(async () => {
  const browser = await chromium.launch();
  for (const f of files) {
    const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
    const errors = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.goto("file://" + path.join(dir, f), { waitUntil: "networkidle" });
    await page.evaluate(() => document.fonts.ready);
    const duration = await page.evaluate(() => window.scene.duration);
    const name = f.replace(/\.html$/, "");
    if (stillsOnly) {
      for (const frac of [0.15, 0.4, 0.65, 0.95]) {
        await page.evaluate((t) => window.scene.render(t), duration * frac);
        await page.screenshot({ path: path.join(out, `${name}-still-${Math.round(frac * 100)}.png`) });
      }
      console.log(name, "stills", errors.length ? errors : "");
      await page.close();
      continue;
    }
    const mp4 = path.join(out, `${name}.mp4`);
    const ff = spawn("ffmpeg", ["-y", "-loglevel", "error", "-f", "image2pipe", "-framerate", String(FPS), "-i", "-",
      "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "16", "-preset", "medium", "-movflags", "+faststart", mp4]);
    const frames = Math.round(duration * FPS);
    for (let i = 0; i < frames; i++) {
      await page.evaluate((t) => window.scene.render(t), i / FPS);
      const buf = await page.screenshot({ type: "jpeg", quality: 95 });
      if (!ff.stdin.write(buf)) await new Promise((r) => ff.stdin.once("drain", r));
    }
    ff.stdin.end();
    await new Promise((r) => ff.on("close", r));
    console.log(`${name}: ${frames} frames, ${duration}s -> ${path.relative(process.cwd(), mp4)}`, errors.length ? errors : "");
    await page.close();
  }
  await browser.close();
})();
