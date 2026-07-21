import { writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { collectPages, site } from '../lib/llm-txt.mjs';

const publicDir = join(process.cwd(), 'public');
const pages = collectPages();
for (const p of pages) {
  const dest = join(publicDir, `${p.slug}.txt`);
  mkdirSync(dirname(dest), { recursive: true });
  writeFileSync(dest, p.txt);
}
console.log(`llm txt: ${pages.length} page .txt files -> ${publicDir} (site: ${site})`);
