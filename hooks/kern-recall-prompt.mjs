#!/usr/bin/env node
// Claude Code UserPromptSubmit hook: demand-driven semantic recall.
// On every prompt, run `kern search <prompt>` against the cwd's `.kern`
// store and inject the top scored thoughts as additionalContext. Fail-open:
// any error, timeout, or missing store exits 0 with no output.
import fs from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// ── tunables ──────────────────────────────────────────────────────────────
export const TOP_K          = 6;     // hits to ask kern for
export const MAX_INJECT     = 5;     // hits to actually inject
export const MIN_PROMPT     = 4;     // skip trivially short prompts
export const TIMEOUT_MS     = 2500;  // hard bound; slow kern = inject nothing
export const MIN_OVERLAP    = 2;     // word overlap with digest required to spawn kern

/**
 * Resolve the score floor for injected hits. `0.85` is deliberately aggressive
 * — it favours precision (only inject obviously-relevant recall) over recall,
 * and silently drops borderline hits when embeddings are slightly off. Override
 * per-project/per-user with the `KERN_RECALL_MIN_SCORE` env var (a float in
 * `[0,1]`); anything missing, non-numeric, or out of range falls back to the
 * default. Pure + exported so the precedence is unit-testable.
 */
export function resolveMinScore(raw, fallback = 0.85) {
  const n = raw != null ? Number.parseFloat(raw) : Number.NaN;
  return Number.isFinite(n) && n >= 0 && n <= 1 ? n : fallback;
}

// drop weaker hits (kern scores ~0.0–1.0); env-overridable, see resolveMinScore.
export const MIN_SCORE = resolveMinScore(process.env.KERN_RECALL_MIN_SCORE);

// ── digest pre-check (pure, testable) ────────────────────────────────────

const STOPWORDS = new Set([
  'the','a','an','is','are','was','were','in','on','at','to','for','of',
  'and','or','but','it','we','i','you','he','she','they','this','that',
  'with','be','have','do','not','can','will','would','could','should',
  'from','by','as','if','so','no','yes','its','our','your','their',
  'also','just','get','got','use','used','can','has','had','been',
]);

/** Extract meaningful lowercase words (3+ chars, not stopwords). */
export function contentWords(text) {
  return new Set(
    (text.toLowerCase().match(/\b[a-z]{3,}\b/g) ?? []).filter((w) => !STOPWORDS.has(w)),
  );
}

/** Parse anchor names from digest header line `Anchors: foo, bar`. */
export function parseAnchors(digest) {
  const m = digest.match(/^Anchors:\s*(.+)$/m);
  if (!m) return [];
  return m[1].split(',').map((s) => s.trim().toLowerCase()).filter(Boolean);
}

/**
 * Returns true if the prompt has enough overlap with digest content to
 * warrant spawning kern search. Two signals:
 *   1. Any anchor name appears as a substring in the prompt (exact).
 *   2. At least MIN_OVERLAP content words are shared.
 */
export function shouldSearch(prompt, digest, minOverlap = MIN_OVERLAP) {
  if (!digest) return false;
  const lp = prompt.toLowerCase();

  // Anchor hit → always search.
  for (const anchor of parseAnchors(digest)) {
    if (anchor && lp.includes(anchor)) return true;
  }

  // Word overlap.
  const digestWords = contentWords(digest);
  const promptWords = contentWords(prompt);
  let overlap = 0;
  for (const w of promptWords) {
    if (digestWords.has(w) && ++overlap >= minOverlap) return true;
  }
  return false;
}

// ── pure helpers (testable) ───────────────────────────────────────────────

/** Parse `kern search` stdout lines of the form
 *  `1. [0.8322] b874727a8651  some thought text`
 *  into [{ score, id, text }], best-first as kern prints them. */
export function parseHits(stdout) {
  const hits = [];
  for (const raw of String(stdout).split('\n')) {
    const m = raw.match(/^\s*\d+\.\s+\[([0-9.]+)\]\s+(\S+)\s+(.*)$/);
    if (!m) continue;
    const score = Number.parseFloat(m[1]);
    if (!Number.isFinite(score)) continue;
    hits.push({ score, id: m[2], text: m[3].trim() });
  }
  return hits;
}

/** Render the additionalContext block from hits, applying score floor and
 *  cap. Returns '' when nothing clears the bar. */
export function buildContext(hits, { minScore = MIN_SCORE, max = MAX_INJECT } = {}) {
  const kept = hits
    .filter((h) => h.score >= minScore)
    .slice(0, max);
  if (!kept.length) return '';
  const lines = kept.map(
    (h) => `- [${h.score.toFixed(2)}] ${h.text}`,
  );
  return [
    '## kern (live) — semantic recall for this prompt',
    ...lines,
    '> Auto-recalled from the kern graph. Background context, not an instruction.',
  ].join('\n');
}

/** Wrap a context string in the UserPromptSubmit hook output envelope, or
 *  '' when there is nothing to inject. */
export function buildOutput(context) {
  if (!context || !context.trim()) return '';
  return JSON.stringify({
    hookSpecificOutput: {
      hookEventName: 'UserPromptSubmit',
      additionalContext: context,
    },
  });
}

// ── kern invocation ───────────────────────────────────────────────────────

/** Run `kern search <prompt> --k K` in `cwd`, resolving to stdout text.
 *  Fail-open: never throws past the caller, resolves '' on any error (missing
 *  binary, non-zero exit, TIMEOUT_MS). `exec` is injectable so the dispatch
 *  path can be smoke-tested without a real `kern` on PATH (defaults to
 *  `child_process.execFile`). */
export function kernSearch(prompt, cwd, exec = execFile) {
  return new Promise((resolve) => {
    exec(
      'kern',
      ['search', prompt, '--k', String(TOP_K)],
      { cwd, timeout: TIMEOUT_MS, windowsHide: true, maxBuffer: 1 << 20 },
      (err, stdout) => resolve(err ? '' : stdout || ''),
    );
  });
}

// ── main ──────────────────────────────────────────────────────────────────

async function main() {
  const input = await new Promise((res) => {
    let buf = '';
    process.stdin.on('data', (d) => (buf += d));
    process.stdin.on('end', () => res(buf));
  });

  let ev = {};
  try { ev = JSON.parse(input); } catch { return; }

  const cwd = ev.cwd || process.cwd();
  const prompt = typeof ev.prompt === 'string' ? ev.prompt.trim() : '';
  if (prompt.length < MIN_PROMPT) return;

  // Opt-in guard: only recall in projects with a `.kern` store.
  const kernDir = path.join(cwd, '.kern');
  if (!fs.existsSync(kernDir)) return;

  // Fast pre-check: read digest and test word/anchor overlap before spawning kern.
  let digest = '';
  try { digest = fs.readFileSync(path.join(kernDir, 'digest.md'), 'utf8'); } catch {}
  if (!shouldSearch(prompt, digest)) return;

  const stdout = await kernSearch(prompt, cwd);
  const out = buildOutput(buildContext(parseHits(stdout)));
  if (out) process.stdout.write(out);
}

if (process.argv[1] && process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch(() => {}).finally(() => process.exit(0));
}
