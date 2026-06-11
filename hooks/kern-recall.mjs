#!/usr/bin/env node
// Claude Code SessionStart hook: inject the kern recall digest. Fail-open.
//
// Contract: Claude Code runs this at SessionStart and feeds the hook event as
// JSON on stdin (`{ cwd, ... }`). The hook prints ONE JSON line on stdout of the
// form `{ hookSpecificOutput: { hookEventName: "SessionStart", additionalContext } }`;
// `additionalContext` is prepended to the model's context for the session — here,
// the kern recall digest at `<cwd>/.kern/digest.md`. Fail-open: any stdin-parse or
// read error, or a missing/blank digest, exits 0 with NO output so a broken hook
// never blocks a session from starting.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** Build the SessionStart additionalContext payload from a digest string,
 *  or an empty string when there is nothing to inject. Pure (testable). */
export function buildOutput(digest) {
  if (!digest || !digest.trim()) return '';
  return JSON.stringify({
    hookSpecificOutput: {
      hookEventName: 'SessionStart',
      additionalContext: digest,
    },
  });
}

/**
 * Resolve the SessionStart output for a parsed hook event: read
 * `<cwd>/.kern/digest.md` and wrap it via {@link buildOutput}. Returns `''` when
 * the digest is absent, unreadable, or blank (fail-open). `readFile` is injectable
 * so the event→path→output flow is unit-testable without the real filesystem.
 *
 * @param {{cwd?: string}} ev parsed hook event (may lack `cwd`)
 * @param {(p: string, enc: string) => string} [readFile] defaults to fs.readFileSync
 * @returns {string} the stdout JSON envelope, or '' if nothing to inject
 */
export function resolveDigest(ev, readFile = fs.readFileSync) {
  const cwd = (ev && ev.cwd) || process.cwd();
  if (!(ev && ev.cwd) && process.env.KERN_HOOK_DEBUG) {
    // `cwd` absent from the event → falling back to process.cwd(). Usually fine,
    // but a mismatch reads the wrong project's digest; surface it ONLY in debug
    // mode so normal sessions stay silent (this is a fail-open hook).
    process.stderr.write('kern-recall: no cwd in hook event; using process.cwd()\n');
  }
  const digestPath = path.join(cwd, '.kern', 'digest.md');
  let digest = '';
  try {
    digest = readFile(digestPath, 'utf8');
  } catch {
    return '';
  }
  return buildOutput(digest);
}

async function main() {
  const input = await new Promise((res) => {
    let buf = '';
    process.stdin.on('data', (d) => (buf += d));
    process.stdin.on('end', () => res(buf));
  });
  let ev = {};
  try { ev = JSON.parse(input); } catch {}
  const out = resolveDigest(ev);
  if (out) process.stdout.write(out);
}

if (process.argv[1] && process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch(() => {}).finally(() => process.exit(0));
}
