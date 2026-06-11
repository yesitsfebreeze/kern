import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  parseHits, buildContext, buildOutput, MIN_SCORE,
  contentWords, parseAnchors, shouldSearch,
  resolveMinScore, kernSearch, TOP_K,
} from '../kern-recall-prompt.mjs';

const SAMPLE = [
  '1. [0.9322] b874727a8651  Trello board routing for kern work (corrected 2026-06-05)',
  '2. [0.8754] 2fadb1541c40  Kern tasks and findings must use the dedicated Kern Trello board',
  '3. [0.5011] fa5826a3afe0  A weak, below-threshold hit that should be dropped',
  'garbage line that is not a hit',
].join('\n');

test('parseHits extracts score/id/text and ignores non-hit lines', () => {
  const hits = parseHits(SAMPLE);
  assert.equal(hits.length, 3);
  assert.deepEqual(hits[0], { score: 0.9322, id: 'b874727a8651', text: 'Trello board routing for kern work (corrected 2026-06-05)' });
  assert.equal(hits[2].score, 0.5011);
});

test('parseHits returns [] on empty / junk', () => {
  assert.deepEqual(parseHits(''), []);
  assert.deepEqual(parseHits('no hits here\nfoo bar'), []);
});

test('buildContext drops sub-threshold hits and caps count', () => {
  const ctx = buildContext(parseHits(SAMPLE));
  assert.match(ctx, /semantic recall/);
  assert.match(ctx, /\[0\.93\] Trello board routing/);
  assert.match(ctx, /\[0\.88\] Kern tasks/);
  assert.doesNotMatch(ctx, /below-threshold/); // 0.50 < MIN_SCORE
});

test('buildContext respects max cap', () => {
  const many = Array.from({ length: 10 }, (_, i) => `${i + 1}. [0.90] id${i}  thought ${i}`).join('\n');
  const ctx = buildContext(parseHits(many), { max: 3 });
  const bullets = ctx.split('\n').filter((l) => l.startsWith('- '));
  assert.equal(bullets.length, 3);
});

test('buildContext returns empty when nothing clears the floor', () => {
  assert.equal(buildContext(parseHits('1. [0.10] x  weak')), '');
  assert.ok(MIN_SCORE > 0.10);
});

// ── score-floor override ────────────────────────────────────────────────

test('resolveMinScore: valid env override wins, junk/out-of-range falls back', () => {
  assert.equal(resolveMinScore('0.5'), 0.5);
  assert.equal(resolveMinScore('0'), 0);
  assert.equal(resolveMinScore('1'), 1);
  assert.equal(resolveMinScore(undefined), 0.85, 'missing -> default');
  assert.equal(resolveMinScore('notanumber'), 0.85, 'non-numeric -> default');
  assert.equal(resolveMinScore('1.5'), 0.85, 'above 1 -> default');
  assert.equal(resolveMinScore('-0.2'), 0.85, 'below 0 -> default');
  assert.equal(resolveMinScore('0.3', 0.9), 0.3, 'explicit fallback only used when invalid');
});

// ── kern invocation (dependency-injected exec) ──────────────────────────────

test('kernSearch resolves stdout on success and "" on error', async () => {
  const ok = await kernSearch('q', '/tmp', (_f, _a, _o, cb) => cb(null, 'OUT'));
  assert.equal(ok, 'OUT');
  const failed = await kernSearch('q', '/tmp', (_f, _a, _o, cb) => cb(new Error('boom'), 'ignored'));
  assert.equal(failed, '', 'fail-open: any error -> empty string');
  const nullStdout = await kernSearch('q', '/tmp', (_f, _a, _o, cb) => cb(null, undefined));
  assert.equal(nullStdout, '', 'undefined stdout coerces to empty');
});

test('kernSearch dispatches the expected argv and cwd', async () => {
  let seen;
  await kernSearch('hello world', '/proj', (file, args, opts, cb) => {
    seen = { file, args, cwd: opts.cwd };
    cb(null, '');
  });
  assert.equal(seen.file, 'kern');
  assert.deepEqual(seen.args, ['search', 'hello world', '--k', String(TOP_K)]);
  assert.equal(seen.cwd, '/proj');
});

test('buildOutput wraps in the UserPromptSubmit envelope, empty stays empty', () => {
  assert.equal(buildOutput(''), '');
  const out = JSON.parse(buildOutput('hello'));
  assert.equal(out.hookSpecificOutput.hookEventName, 'UserPromptSubmit');
  assert.equal(out.hookSpecificOutput.additionalContext, 'hello');
});

// ── pre-check tests ───────────────────────────────────────────────────────

const DIGEST = `# kern memory\n\nAnchors: digest, anchors\n\n## What I know\n\n- bincode positional rename preserved data\n- shards live under .kern/data directory\n`;

test('contentWords strips stopwords and short words', () => {
  const ws = contentWords('the quick brown fox is fast');
  assert.ok(ws.has('quick'));
  assert.ok(ws.has('brown'));
  assert.ok(ws.has('fox')); // 3 chars, kept
  assert.ok(ws.has('fast'));
  assert.ok(!ws.has('the')); // stopword
  assert.ok(!ws.has('is'));  // stopword
});

test('parseAnchors extracts names from digest header', () => {
  const anchors = parseAnchors(DIGEST);
  assert.deepEqual(anchors, ['digest', 'anchors']);
});

test('parseAnchors returns [] when no Anchors line', () => {
  assert.deepEqual(parseAnchors('# kern memory\n\n## What I know\n'), []);
});

test('shouldSearch: anchor name in prompt triggers search', () => {
  assert.ok(shouldSearch('how does the digest work', DIGEST));
});

test('shouldSearch: sufficient word overlap triggers search', () => {
  // "bincode" and "shards" appear in digest bullets
  assert.ok(shouldSearch('explain the bincode shards layout', DIGEST));
});

test('shouldSearch: unrelated prompt skips search', () => {
  assert.ok(!shouldSearch('what time is lunch today', DIGEST));
});

test('shouldSearch: empty digest skips search', () => {
  assert.ok(!shouldSearch('bincode shards data', ''));
});

test('shouldSearch: single overlap word below threshold skips', () => {
  // only "bincode" matches — need MIN_OVERLAP (2) to trigger
  assert.ok(!shouldSearch('explain bincode please', DIGEST, 2));
});
