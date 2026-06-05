#!/usr/bin/env node
// Claude Code Stop hook: extract the new conversation delta from the
// transcript and write it to the kern capture spool. Fail-open.
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

function offsetsFile(spool) { return path.join(spool, '.offsets.json'); }

function readOffsets(spool) {
  try { return JSON.parse(fs.readFileSync(offsetsFile(spool), 'utf8')); }
  catch { return {}; }
}

function writeOffsets(spool, offsets) {
  try { fs.writeFileSync(offsetsFile(spool), JSON.stringify(offsets)); } catch {}
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
  // unrelated projects with empty spool dirs.
  if (!fs.existsSync(path.join(cwd, '.kern'))) return;

  const spool = path.join(cwd, '.kern', 'capture');
  fs.mkdirSync(spool, { recursive: true });

  const lines = readLines(fs.readFileSync(transcript_path, 'utf8'));
  const offsets = readOffsets(spool);
  const start = offsets[session_id] || 0;
  if (start >= lines.length) return;

  const { text, consumed } = extractDelta(lines, start);
  if (text.trim()) {
    const file = path.join(spool, `${session_id}-${consumed}.txt`);
    fs.writeFileSync(file, text);
  }
  offsets[session_id] = consumed;
  writeOffsets(spool, offsets);
}

// Only run main when invoked directly (not when imported by tests).
if (process.argv[1] && process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch(() => {}).finally(() => process.exit(0));
}
