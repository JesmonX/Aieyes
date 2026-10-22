/* Source-keyed UI messages. No substring replacement or observation of user data. */
(() => {
  const messages = globalThis.AieyesMessages || {};
  const normalize = value => value === 'en' ? 'en' : 'zh-CN';
  let language = 'zh-CN';
  try { language = normalize(localStorage.getItem('aieyes.language')); } catch (_) {}
  const missing = new Set(), cache = new Map(), staticText = [], staticAttributes = [], builtinText = [];
  const han = /[\u3400-\u9fff]/;
  const substitute = (text, values) => text.replace(/\{(\d+)\}/g, (match, key) => key in values ? String(values[key] ?? '') : match);
  const escapePattern = value => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const reverse = Object.fromEntries(Object.entries(messages).map(([key, value]) => [value, key]));
  const makePatterns = entries => entries.filter(([key]) => /\{\d+\}/.test(key)).sort((a,b) => b[0].replace(/\{\d+\}/g,'').length-a[0].replace(/\{\d+\}/g,'').length).map(([key, value]) => {
    const slots = []; let end = 0, expression = '^';
    for (const match of key.matchAll(/\{(\d+)\}/g)) {
      expression += escapePattern(key.slice(end, match.index)) + '([\\s\\S]*?)';
      slots.push(match[1]); end = match.index + match[0].length;
    }
    return {expression: new RegExp(expression + escapePattern(key.slice(end)) + '$'), value, slots};
  });
  const patterns = makePatterns(Object.entries(messages)), reversePatterns = makePatterns(Object.entries(reverse));
  function message(source) {
    source = String(source ?? '');
    const dictionary = language === 'en' ? messages : reverse;
    if (Object.hasOwn(dictionary, source)) return dictionary[source];
    if (language !== 'en' && Object.hasOwn(messages, source)) return source;
    if (language === 'en' && !han.test(source)) return source;
    if (cache.has(source)) return cache.get(source);
    for (const pattern of language === 'en' ? patterns : reversePatterns) {
      const match = pattern.expression.exec(source);
      if (!match) continue;
      const values = Object.fromEntries(pattern.slots.map((slot, i) => [slot, match[i + 1]]));
      const result = substitute(pattern.value, values);
      if (cache.size >= 2048) cache.clear();
      cache.set(source, result); return result;
    }
    if (source.includes('\n')) return source.split('\n').map(message).join('\n');
    if (language === 'en') missing.add(source); return source;
  }
  function t(source, ...values) {
    // Exact count forms (for example, one account) take precedence over templates.
    if (language === 'en' && values.length && Object.hasOwn(messages, source)) {
      const rendered = substitute(source, values);
      if (Object.hasOwn(messages, rendered)) return messages[rendered];
    }
    return substitute(language === 'en' ? message(source) : source, values);
  }
  function captureStatic() {
    const walker = document.createTreeWalker(document.documentElement, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      if (han.test(node.nodeValue) && !node.parentElement.closest('script,style,[translate=no],input,textarea,#content,#session-list,#editor-fields')) staticText.push({node, source: node.nodeValue, previous: node.nodeValue});
    }
    for (const element of document.querySelectorAll('*')) {
      for (const name of ['title', 'aria-label', 'aria-description', 'placeholder', 'alt']) {
        const source = element.getAttribute(name);
        if (source && han.test(source)) staticAttributes.push({element, name, source, previous: source});
      }
    }
    function captureBuiltins(value) {
      if (!value || typeof value !== 'object') return;
      for (const [key, source] of Object.entries(value)) {
        if (typeof source === 'string' && han.test(source)) builtinText.push({value, key, source});
        else if (source && typeof source === 'object') captureBuiltins(source);
      }
    }
    captureBuiltins(window.AieyesContract); captureBuiltins(window.AieyesChartPalettes);
  }
  function applyStatic() {
    document.documentElement.lang = language;
    for (const record of staticText) {
      if (record.node.isConnected && record.node.nodeValue === record.previous) record.node.nodeValue = record.previous = t(record.source);
    }
    for (const record of staticAttributes) {
      if (record.element.isConnected && record.element.getAttribute(record.name) === record.previous) record.element.setAttribute(record.name, record.previous = t(record.source));
    }
    for (const record of builtinText) record.value[record.key] = t(record.source);
    document.querySelectorAll('[data-language-toggle]').forEach(button => {
      button.textContent = language === 'en' ? '中文' : 'English';
      button.title = language === 'en' ? 'Switch to Chinese' : '切换到英文';
      button.setAttribute('aria-label', language === 'en' ? 'Switch to Chinese' : '切换到英文');
    });
  }
  function apply(value) {
    const next = normalize(value), changed = next !== language;
    if (changed) cache.clear();
    language = next;
    try { localStorage.setItem('aieyes.language', next); } catch (_) {}
    if (typeof NodeFilter !== 'undefined') applyStatic();
    if (changed && typeof window !== 'undefined' && window.dispatchEvent) window.dispatchEvent(new CustomEvent('aieyes:language', {detail: {language}}));
  }
  let controlMessages, reverseControls;
  function prepareControls() {
    if (controlMessages) return;
    controlMessages = {...messages};
    // A translated HTML block also owns its individual static labels. Pair the
    // text nodes, without treating interpolated values as translatable copy.
    for (const [source, target] of Object.entries(messages)) {
      if (!/<[a-z][^>]*>/i.test(source)) continue;
      const a = document.createElement('template'), b = document.createElement('template');
      a.innerHTML = source; b.innerHTML = target;
      const texts = root => {
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT), result = [];
        let node; while ((node = walker.nextNode())) result.push(node.nodeValue);
        return result;
      };
      const left = texts(a.content), right = texts(b.content);
      if (left.length !== right.length) continue;
      left.forEach((value, index) => {
        if (han.test(value) && !/\{\d+\}/.test(value) && right[index].trim()) controlMessages[value] ??= right[index];
      });
    }
    reverseControls = Object.fromEntries(Object.entries(controlMessages).map(([key, value]) => [value, key]));
  }
  // Existing dialogs keep their DOM and input state when another window changes locale.
  // Only catalogue-backed control copy is updated; user values are never inspected.
  function refreshControls(root) {
    prepareControls();
    const dictionary = language === 'en' ? controlMessages : reverseControls;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let node;
    while ((node = walker.nextNode())) {
      const element = node.parentElement;
      if (!element.closest('button,label,legend,summary,h2,h3,p,option,[role=status],.error,.muted') || element.closest('input,textarea,[translate=no],[data-user-text],.source-path,.win-select-label,.win-select-option')) continue;
      const source = node.nodeValue;
      const translated = dictionary[source];
      if (translated) node.nodeValue = translated;
    }
    for (const element of root.querySelectorAll('[title],[aria-label],[placeholder]')) {
      if (element.closest('[translate=no],[data-user-text]')) continue;
      for (const name of ['title','aria-label','placeholder']) {
        const source = element.getAttribute(name), translated = dictionary[source];
        if (translated) element.setAttribute(name, translated);
      }
    }
    window.AieyesSingleSelect?.refresh(root);
  }
  globalThis.AieyesI18n = {t, message, apply, normalize, refreshControls, get language() {return language;}, get locale() {return language === 'en' ? 'en-US' : 'zh-CN';}, get missing() {return [...missing];}};
  if (typeof window !== 'undefined') window.AieyesI18n = globalThis.AieyesI18n;
  if (typeof NodeFilter === 'undefined') return;
  document.documentElement.lang = language;
  document.addEventListener('DOMContentLoaded', () => {captureStatic();applyStatic();});
})();
