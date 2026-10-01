import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';

const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
const source = await readFile(new URL('../src/main.ts', import.meta.url), 'utf8');
const styles = await readFile(new URL('../src/style.css', import.meta.url), 'utf8');
assert.match(html, /<title>Bilicraft AI Stock<\/title>/);
assert.match(html, /md-filled-button/);
assert.match(source, /@material\/web/);
assert.match(source, /lightweight-charts/);
assert.match(styles, /prefers-reduced-motion/);
console.log('web smoke checks passed');
