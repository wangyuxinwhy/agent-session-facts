import { readFileSync, statSync } from 'node:fs';

const expectedFiles = [
  'doc_build/llms.txt',
  'doc_build/llms-full.txt',
  'doc_build/guide/installation.md',
  'doc_build/guide/installation.html',
];

for (const path of expectedFiles) {
  if (!statSync(path).isFile() || statSync(path).size === 0) {
    throw new Error(`missing or empty Agent-friendly output: ${path}`);
  }
}

const index = readFileSync('doc_build/llms.txt', 'utf8');
const markdown = readFileSync('doc_build/guide/installation.md', 'utf8');
const html = readFileSync('doc_build/guide/installation.html', 'utf8');

for (const page of [
  'guide/overview.md',
  'guide/installation.md',
  'guide/usage.md',
  'reference/schema.md',
]) {
  if (!index.includes(page)) {
    throw new Error(`llms.txt does not index ${page}`);
  }
}

if (!markdown.startsWith('> For AI agents:')) {
  throw new Error('page Markdown is missing the Agent directive');
}

for (const marker of [
  'For AI agents:',
  'rp-llms-copy-button',
  'rp-llms-view-options__trigger',
]) {
  if (!html.includes(marker)) {
    throw new Error(`page HTML is missing Agent UI marker: ${marker}`);
  }
}

console.log('Agent-friendly documentation outputs are complete.');
