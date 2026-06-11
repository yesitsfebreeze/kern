import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildOutput, resolveDigest } from '../kern-recall.mjs';

// ── buildOutput ───────────────────────────────────────────────────────────

test('empty string returns empty string', () => {
  assert.equal(buildOutput(''), '');
});

test('whitespace-only returns empty string', () => {
  assert.equal(buildOutput('   '), '');
  assert.equal(buildOutput('\n\t\n'), '');
});

test('null/undefined returns empty string', () => {
  assert.equal(buildOutput(null), '');
  assert.equal(buildOutput(undefined), '');
});

test('valid digest returns SessionStart JSON envelope', () => {
  const out = JSON.parse(buildOutput('# kern memory\n- some fact'));
  assert.equal(out.hookSpecificOutput.hookEventName, 'SessionStart');
  assert.equal(out.hookSpecificOutput.additionalContext, '# kern memory\n- some fact');
});

test('digest passed through verbatim', () => {
  const digest = '# kern memory\n\nAnchors: foo, bar\n\n## What I know\n\n- detail one\n';
  const out = JSON.parse(buildOutput(digest));
  assert.equal(out.hookSpecificOutput.additionalContext, digest);
});

// ── resolveDigest (the main() core: event → .kern path → output) ─────────────

test('resolveDigest reads <cwd>/.kern/digest.md and wraps it', () => {
  let seen;
  const fakeRead = (p) => { seen = p; return '# kern memory\n- fact'; };
  const out = JSON.parse(resolveDigest({ cwd: '/proj' }, fakeRead));
  assert.equal(out.hookSpecificOutput.hookEventName, 'SessionStart');
  assert.equal(out.hookSpecificOutput.additionalContext, '# kern memory\n- fact');
  assert.ok(seen.includes('.kern') && seen.includes('digest.md'), 'derives the .kern/digest.md path');
  assert.ok(seen.includes('proj'), 'path is anchored at ev.cwd');
});

test('resolveDigest is fail-open: empty string when the digest is unreadable', () => {
  const throwing = () => { throw new Error('ENOENT'); };
  assert.equal(resolveDigest({ cwd: '/proj' }, throwing), '');
});

test('resolveDigest yields empty output for a blank digest', () => {
  const blank = () => '   \n\t';
  assert.equal(resolveDigest({ cwd: '/proj' }, blank), '');
});
