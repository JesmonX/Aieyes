// WCAG AA text contrast on every main page: Windows and Linux, light and dark, synthetic data (no accounts or network).
// Each visible text run is composited over its actual stack of backgrounds and opacities.
const assert = require('node:assert/strict');
const { chromium } = require('../apps/desktop/node_modules/playwright');
const { createPreviewServer } = require('./desktop-preview/server.cjs');
const PAGES = ['[data-page=agent]', '[data-page=analysis]', '[data-page=servers]', '[data-settings-tab=integrations]', '[data-settings-tab=hosts]', '[data-settings-tab=prices]', '[data-settings-tab=wakeups]', '[data-page=settings]'];
const SIZES = { windows: [1294, 840], linux: [1120, 800] };

function measure() {
  const probe = document.createElement('i');document.body.append(probe);
  const parse = (value) => {probe.style.color = '';probe.style.color = value;const c = getComputedStyle(probe).color;
    const m = c.match(/rgba?\(([^)]+)\)/);if (m) {const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number);return [p[0], p[1], p[2], p[3] ?? 1];}
    const s = c.match(/color\(srgb ([^)]+)\)/);if (s) {const p = s[1].split(/[ /]+/).filter(Boolean).map(Number);return [p[0] * 255, p[1] * 255, p[2] * 255, p[3] ?? 1];}
    return [0, 0, 0, 0];};
  const over = (top, bottom) => {const a = top[3];return [0, 1, 2].map((i) => top[i] * a + bottom[i] * (1 - a)).concat(1);};
  const lum = (c) => {const [r, g, b] = c.slice(0, 3).map((v) => {v /= 255;return v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4;});return .2126 * r + .7152 * g + .0722 * b;};
  const ratio = (a, b) => {const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);return (x + .05) / (y + .05);};
  const base = parse(getComputedStyle(document.documentElement).getPropertyValue('--window-base').trim() || '#fff');
  const backgroundOf = (el) => {const chain = [];for (let n = el; n && n !== document.documentElement; n = n.parentElement) chain.unshift(n);
    let bg = base;for (const n of chain) bg = over(parse(getComputedStyle(n).backgroundColor), bg);return bg;};
  const rows = [];
  for (const el of document.body.querySelectorAll('*')) {
    if (![...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()) || !el.getClientRects().length) continue;
    if (el.closest('[disabled],[aria-disabled=true],[hidden],svg,.sr-only,option,script,style')) continue;
    const s = getComputedStyle(el);if (s.visibility === 'hidden') continue;
    let alpha = +s.opacity;for (let n = el.parentElement; n; n = n.parentElement) alpha *= +getComputedStyle(n).opacity;
    if (alpha === 0) continue;
    const size = parseFloat(s.fontSize),large = size >= 24 || size >= 18.66 && +s.fontWeight >= 700;
    const bg = backgroundOf(el),c = parse(s.color),fg = over([c[0], c[1], c[2], c[3] * alpha], bg);
    rows.push({ where: el.closest('[id]')?.id || '', cls: String(el.className?.baseVal ?? el.className ?? '') || el.tagName.toLowerCase(),
      text: el.textContent.trim().slice(0, 24), ratio: +ratio(fg, bg).toFixed(2), need: large ? 3 : 4.5 });
  }
  probe.remove();return rows;
}

(async () => {
  const host = createPreviewServer();
  await new Promise((resolve) => host.server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${host.server.address().port}`;
  const browser = await chromium.launch(),rows = [];
  try {
    for (const platform of ['windows', 'linux']) for (const theme of ['light', 'dark']) {
      const [width, height] = SIZES[platform];
      const page = await browser.newPage({ viewport: { width: width + 160, height: height + 1000 } });
      await page.goto(`${origin}/?platform=${platform}&scenario=multi&surface=main`);
      await page.waitForFunction(() => document.querySelector('#app')?.dataset.ready === 'true');
      await page.selectOption('#zoom', '1');
      await page.selectOption('#size', 'custom');
      await page.fill('#width', String(width));await page.locator('#width').dispatchEvent('change');
      await page.fill('#height', String(height));await page.locator('#height').dispatchEvent('change');
      if (theme !== 'light') await page.selectOption('#theme', theme);
      await page.waitForTimeout(900);
      const frame = page.frames().find((f) => f.url().includes('/app/'));
      await frame.evaluate(() => AieyesApp.ready);
      await frame.waitForFunction(() => state.settings && state.dashboard && !state.dashboardPending);
      for (const selector of PAGES) {
        await frame.locator(selector).first().click();await page.waitForTimeout(900);
        await frame.evaluate(() => document.querySelectorAll('details.server-more').forEach((d) => d.open = true));await page.waitForTimeout(300);
        for (const row of await frame.evaluate(measure)) rows.push({ platform, theme, page: selector.replace(/\[data-[\w-]+=(\w+)\]/, '$1'), ...row });
      }
      await page.close();
    }
  } finally {await browser.close();host.server.close();}
  assert(rows.length > 1000, `Too little text measured (${rows.length}); the pages did not render`);
  const failures = rows.filter((r) => r.ratio < r.need).map((r) => `${r.platform}/${r.theme} ${r.page} #${r.where} .${r.cls} ${JSON.stringify(r.text)} ${r.ratio}:1 < ${r.need}:1`);
  assert.deepEqual([...new Set(failures)].slice(0, 20), [], 'Text below WCAG AA contrast');
  console.log(`Text contrast: ${rows.length} text runs on ${PAGES.length} pages, Windows/Linux, light/dark meet WCAG AA`);
})().catch((error) => {console.error(error);process.exitCode = 1;});
