#!/usr/bin/env node
// Claude Code Stop hook: extract the new conversation delta from the
// transcript and write it to the kern capture intake. Fail-open.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

/** Split transcript text into JSONL lines, dropping a single trailing
 *  empty element produced by the final newline. */
export function readLines(raw) {
  const lines = raw.split('\n');
  if (lines.length && lines[lines.length - 1] === '') lines.pop();
  return lines;
}

/** Extract user prompts + assistant text from transcript lines past `offset`.
 *  Returns { text, consumed } where consumed is the new line count. */
export function extractDelta(lines, offset) {
  const out = [];
  let i = offset;
  for (; i < lines.length; i++) {
    const raw = lines[i];
    if (!raw || !raw.trim()) continue;
    let o;
    try { o = JSON.parse(raw); } catch { continue; }
    if (o.type === 'user') {
      const c = o.message?.content;
      if (typeof c === 'string' && c.trim()) out.push(`user: ${c.trim()}`);
      // user content that is an array = tool_result; skip.
    } else if (o.type === 'assistant') {
      const c = o.message?.content;
      if (Array.isArray(c)) {
        for (const b of c) {
          if (b?.type === 'text' && b.text?.trim()) {
            out.push(`assistant: ${b.text.trim()}`);
          }
        }
      }
    }
  }
  return { text: out.join('\n\n'), consumed: lines.length };
}

function offsetsFile(intake) { return path.join(intake, '.offsets.json'); }

function readOffsets(intake) {
  try { return JSON.parse(fs.readFileSync(offsetsFile(intake), 'utf8')); }
  catch { return {}; }
}

function writeOffsets(intake, offsets) {
  // Atomic write: a tmp file (pid-tagged so two rapidly-firing Stop hooks don't
  // clobber each other's temp) then a rename, so a concurrent reader never sees
  // a half-written `.offsets.json`.
  const dst = offsetsFile(intake);
  const tmp = `${dst}.${process.pid}.tmp`;
  try {
    fs.writeFileSync(tmp, JSON.stringify(offsets));
    fs.renameSync(tmp, dst);
  } catch {
    try { fs.unlinkSync(tmp); } catch {}
  }
}

/** Cap on capture files kept in the intake. */
export const MAX_INTAKE_FILES = 500;

/** Return the capture file names to delete so at most `maxFiles` remain,
 *  dropping the OLDEST (lowest `mtimeMs`) first. `entries` is `[{name,mtimeMs}]`.
 *  Pure, so the eviction policy is unit-testable without touching the fs. */
export function intakeEvictions(entries, maxFiles) {
  if (entries.length <= maxFiles) return [];
  return [...entries]
    .sort((a, b) => a.mtimeMs - b.mtimeMs)
    .slice(0, entries.length - maxFiles)
    .map((e) => e.name);
}

async function main() {
  const input = await new Promise((res) => {
    let buf = '';
    process.stdin.on('data', (d) => (buf += d));
    process.stdin.on('end', () => res(buf));
  });
  let ev;
  try { ev = JSON.parse(input); } catch { return; }
  const { transcript_path, cwd, session_id } = ev;
  if (!transcript_path || !cwd || !session_id || !fs.existsSync(transcript_path)) return;

  // Opt-in: only capture in projects that already have a `.kern` dir (a kern
  // is or has been active here). Prevents this global hook from polluting
  // unrelated projects with empty intake dirs.
  if (!fs.existsSync(path.join(cwd, '.kern'))) return;

  const intake = path.join(cwd, '.kern', 'capture');
  fs.mkdirSync(intake, { recursive: true });

  const lines = readLines(fs.readFileSync(transcript_path, 'utf8'));
  const offsets = readOffsets(intake);
  const start = offsets[session_id] || 0;
  if (start >= lines.length) return;

  const { text, consumed } = extractDelta(lines, start);
  if (text.trim()) {
    const file = path.join(intake, `${session_id}-${consumed}.txt`);
    fs.writeFileSync(file, text);
  }
  offsets[session_id] = consumed;
  writeOffsets(intake, offsets);

  // Cap the intake so an un-drained capture dir can't grow without bound across
  // many sessions; evict the oldest beyond MAX_INTAKE_FILES.
  try {
    const entries = fs
      .readdirSync(intake)
      .filter((n) => n.endsWith('.txt'))
      .map((n) => ({ name: n, mtimeMs: fs.statSync(path.join(intake, n)).mtimeMs }));
    for (const name of intakeEvictions(entries, MAX_INTAKE_FILES)) {
      try { fs.unlinkSync(path.join(intake, name)); } catch {}
    }
  } catch {}
}

// Only run main when invoked directly (not when imported by tests).
if (process.argv[1] && process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch(() => {}).finally(() => process.exit(0));
}
