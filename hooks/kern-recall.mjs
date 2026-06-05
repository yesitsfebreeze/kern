#!/usr/bin/env node
// Claude Code SessionStart hook: inject the kern recall digest. Fail-open.
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

async function main() {
  const input = await new Promise((res) => {
    let buf = '';
    process.stdin.on('data', (d) => (buf += d));
    process.stdin.on('end', () => res(buf));
  });
  let ev = {};
  try { ev = JSON.parse(input); } catch {}
  const cwd = ev.cwd || process.cwd();
  const digestPath = path.join(cwd, '.relay', 'kern', 'digest.md');

  let digest = '';
  try { digest = fs.readFileSync(digestPath, 'utf8'); } catch { return; }

  const out = buildOutput(digest);
  if (out) process.stdout.write(out);
}

if (process.argv[1] && process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch(() => {}).finally(() => process.exit(0));
}
