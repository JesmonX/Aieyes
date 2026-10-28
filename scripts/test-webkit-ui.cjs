// WebKit rendering channel for Linux CSS. Native GTK/Wayland acceptance is separate.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { webkit } = require('../apps/desktop/node_modules/playwright');
const { createPreviewServer } = require('./desktop-preview/server.cjs');
const pages = [
  ['overview', '[data-page=agent]'], ['analysis', '[data-page=analysis]'],
  ['servers', '[data-page=servers]'], ['integrations', '[data-settings-tab=integrations]'],
  ['hosts', '[data-settings-tab=hosts]'], ['prices', '[data-settings-tab=prices]'],
  ['wakeups', '[data-settings-tab=wakeups]'], ['settings', '[data-page=settings]'],
];
(async () => {
  const output = path.resolve(process.env.AIEYES_WEBKIT_OUTPUT || path.join(__dirname, '../.build/ui-webkit'));
  fs.mkdirSync(output, { recursive: true });
  const preview = createPreviewServer();
  await new Promise(resolve => preview.server.listen(0, '127.0.0.1', resolve));
  let browser, checked = 0;
  const manifest = [];
  try {
    browser = await webkit.launch();
    for (const theme of ['light', 'dark']) for (const language of ['zh-CN', 'en']) for (const size of ['default', 'minimum']) {
      const page = await browser.newPage({ viewport: { width: 1360, height: 1200 }, reducedMotion: 'reduce' });
      const errors = [];page.on('pageerror', error => errors.push(error.message));
      try {
        await page.goto(`http://127.0.0.1:${preview.server.address().port}/?platform=linux&scenario=multi&surface=main`);
        await page.waitForFunction(() => document.querySelector('#app')?.dataset.ready === 'true');
        await page.selectOption('#size', size);
        await page.selectOption('#zoom', '1');
        if (theme !== 'light') await page.selectOption('#theme', theme);
        if (language !== 'zh-CN') await page.selectOption('#language', language);
        const frame = page.frames().find(frame => frame.url().includes('/app/'));
        await frame.evaluate(() => AieyesApp.ready);
        await frame.waitForFunction(({ theme, language }) => document.documentElement.dataset.theme === theme && AieyesI18n.language === language && state.dashboard && !state.dashboardPending, { theme, language });
        await frame.evaluate(() => document.fonts.ready);
        for (const [name, selector] of pages) {
          await frame.locator(selector).first().click();
          await frame.waitForFunction(() => !state.busy && !state.dashboardPending);
          await frame.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
          const metrics = await frame.evaluate(() => ({
            scroll: document.documentElement.scrollWidth, viewport: innerWidth,
            heading: document.querySelector('h1')?.textContent.trim(),
            font: getComputedStyle(document.body).fontSize,
            css: [...document.styleSheets].every(sheet => sheet.cssRules.length > 0),
          }));
          assert(metrics.css, `${theme}/${language}/${size}/${name}: stylesheet did not load`);
          assert.equal(metrics.font, '15px', 'generated font tokens must apply in WebKit');
          assert(metrics.heading, `${name}: missing heading`);
          assert(metrics.scroll <= metrics.viewport + 1, `${name}: horizontal overflow ${metrics.scroll}/${metrics.viewport}`);
          const file = `${theme}-${language}-${size}-${name}.png`;
          await page.locator('#app').screenshot({ path: path.join(output, file) });
          manifest.push({ theme, language, size, page: name, file, ...metrics });checked++;
        }
        assert.deepEqual(errors, [], 'WebKit application errors');
      } finally { await page.close(); }
    }
    fs.writeFileSync(path.join(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
    console.log(`WebKit: ${checked} Linux page renders, light/dark, Chinese/English, default/minimum; screenshots: ${output}`);
  } finally { await browser?.close();await preview.close(); }
})().catch(error => { console.error(error);process.exitCode = 1; });
