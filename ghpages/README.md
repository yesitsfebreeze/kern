# kern — landing page

The marketing/landing page for kern, served at
**https://yesitsfebreeze.github.io/kern/**.

Static, dependency-free: plain HTML/CSS/JS, no build step.

- `index.html` — the page (amber-phosphor terminal readout)
- `styles.css` — styling (monospace, CSS phosphor glow + chromatic aberration)
- `crt.js` — WebGL CRT post-process overlay: curved scanlines, vignette, roll,
  grain, flicker. Draws over the live DOM (kept crisp/interactive); 2D fallback
  where WebGL is absent. Toggle with the `[CRT]` button in the status bar.
- `.nojekyll` — tells Pages to serve files as-is

## Preview locally

```bash
cd ghpages
python3 -m http.server 8080
# open http://localhost:8080
```

## Deploy

Pushed automatically by `.github/workflows/pages.yml` on any push to `master`
that touches `ghpages/`. One-time setup: in the repo, **Settings → Pages →
Build and deployment → Source: GitHub Actions**.
