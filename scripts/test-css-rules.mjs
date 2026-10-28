import assert from 'node:assert/strict';
import stylelint from '../apps/desktop/node_modules/stylelint/lib/index.mjs';
import { fileURLToPath } from 'node:url';
const configFile = fileURLToPath(new URL('../apps/desktop/stylelint.config.cjs', import.meta.url));
const filename = fileURLToPath(new URL('../apps/desktop/web/regression.css', import.meta.url));
for (const [code, expected] of [
  ['@layer components { .x { font-size:var(--font-body); border-radius:var(--radius-control); color:var(--per-instance, red); } }', []],
  ['@layer components { .x { font-size:calc(var(--font-body) + 1px); border-radius:9px; color:var(--missing-regression-token); } }', ['aieyes/design-tokens']],
  ['.x { color:red; }', ['aieyes/design-tokens']],
  ['@layer components { .x {color:red} .x {color:blue} }', ['no-duplicate-selectors']],
  ['@layer components { .x {color:red;color:blue} }', ['declaration-block-no-duplicate-properties']],
]) {
  const result = await stylelint.lint({ code, codeFilename: filename, configFile });
  const rules = new Set(result.results.flatMap(result => result.warnings.map(w => w.rule)));
  for (const rule of expected) assert(rules.has(rule), `Missing ${rule} for ${code}`);
  if (!expected.length) assert.equal(result.errored, false, JSON.stringify(result.results.map(r => r.warnings)));
}
console.log('CSS governance: layering, token arithmetic, undefined variables and duplicate regressions passed');
