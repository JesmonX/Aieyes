import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import stylelint from 'stylelint';
import postcss from 'postcss';
const ruleName = 'aieyes/design-tokens';
const web = fileURLToPath(new URL('./web', import.meta.url));
const messages = stylelint.utils.ruleMessages(ruleName, {
  token: property => `Use a design token for ${property}`,
  undefined: name => `Undefined custom property ${name}; declare it or supply a fallback`,
  layer: () => 'Place style rules inside a named cascade layer',
});
const plugin = stylelint.createPlugin(ruleName, enabled => (root, result) => {
  if (!enabled) return;
  const definitions = new Set();
  for (const name of fs.readdirSync(web).filter(name => name.endsWith('.css'))) {
    postcss.parse(fs.readFileSync(path.join(web, name), 'utf8')).walkDecls(decl => {
      if (decl.prop.startsWith('--')) definitions.add(decl.prop);
    });
  }
  // These variables are set per element by renderers rather than by theme rules.
  for (const name of ['--provider-mask', '--weeks']) definitions.add(name);
  const report = (node, message) => stylelint.utils.report({ ruleName, result, node, message });
  const tokenFile = path.basename(root.source.input.file || '') === 'tokens.css';
  root.walkRules(node => {
    let parent = node.parent;
    while (parent && !(parent.type === 'atrule' && parent.name === 'layer')) parent = parent.parent;
    if (!parent && node.parent?.name !== 'keyframes') report(node, messages.layer());
  });
  root.walkDecls(decl => {
    if (!tokenFile && !decl.prop.startsWith('--')) {
      if (decl.prop === 'font-size' && (!/var\(--font-/.test(decl.value) || /[1-9]\d*(?:\.\d+)?(?:px|pt|rem|em)\b/.test(decl.value.replace(/var\([^)]*\)/g, ''))) && !/^(0|inherit|100%|1em)$/.test(decl.value)) report(decl, messages.token(decl.prop));
      if ((/radius$/.test(decl.prop) || decl.prop === 'font') && /\b(?:\d*\.)?[1-9]\d*(?:\.\d+)?(?:px|pt|rem|em)\b/.test(decl.value.replace(/var\([^)]*\)/g, ''))) report(decl, messages.token(decl.prop));
    }
    for (const match of decl.value.matchAll(/var\(\s*(--[\w-]+)\s*(,)?/g)) {
      if (!definitions.has(match[1]) && !match[2]) report(decl, messages.undefined(match[1]));
    }
  });
});
export default plugin;
