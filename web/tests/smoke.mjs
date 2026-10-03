import { readFile } from 'node:fs/promises';
import { strict as assert } from 'node:assert';
import { transformWithOxc } from 'vite';
import { runInNewContext } from 'node:vm';

const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
const source = await readFile(new URL('../src/main.ts', import.meta.url), 'utf8');
const styles = await readFile(new URL('../src/style.css', import.meta.url), 'utf8');
assert.match(html, /<title>Bilicraft AI Stock<\/title>/);
assert.match(html, /md-filled-button/);
assert.match(source, /@material\/web/);
assert.match(source, /lightweight-charts/);
assert.match(styles, /prefers-reduced-motion/);
// Exercise the actual form collector and payload builder, not a hand-written API payload.
const helpers = [
  source.slice(source.indexOf('function inputValue('), source.indexOf('\nfunction toast(')),
  source.slice(source.indexOf('function collectFields('), source.indexOf('\nfunction webPasswordPanel(')),
].join('\n');
const compiled = await transformWithOxc(
  `${helpers}\n({ collectFields, configurationPatch, verifyConfigPatch, inputValue })`,
  'settings-test.ts',
);
const { collectFields, configurationPatch, verifyConfigPatch, inputValue } = runInNewContext(compiled.code);
const control = (path, value, type = 'text', extra = {}) => ({
  dataset: { path, ...extra }, value,
  getAttribute: (key) => key === 'type' ? type : key === 'value' ? 'old-value' : null,
});
const fields = [
  control('ai_budget', '20000', 'number'),
  control('ai_budget_percent', '10', 'number'),
  control('automation_enabled', '', 'text', { boolean: 'true' }),
  control('trading_windows', '[]', 'text', { json: 'true' }),
];
const values = collectFields({ querySelectorAll: () => fields });
const patch = configurationPatch('risk', values);
assert.equal(JSON.stringify(patch), JSON.stringify({ risk: {
  ai_budget: 20000, ai_budget_percent: 10, automation_enabled: false, trading_windows: [],
} }));
assert.equal(Object.keys(configurationPatch('minecraft', {}))[0], 'server');
assert.equal(Object.keys(configurationPatch('openai', {}))[0], 'ai');
for (const section of ['market', 'qq', 'web', 'paths']) {
  assert.equal(Object.keys(configurationPatch(section, {}))[0], section);
}
assert.equal(inputValue(control('system_prompt', '')), '');
assert.doesNotThrow(() => verifyConfigPatch(patch, patch));
assert.throws(() => verifyConfigPatch({ risk: { ai_budget: 0 } }, patch), /risk.ai_budget/);
console.log('web smoke and settings payload checks passed');
