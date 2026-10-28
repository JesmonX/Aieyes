const assert = require('node:assert/strict');
const path = require('node:path');
const { chromium } = require('../apps/desktop/node_modules/playwright');
(async () => {
  const browser = await chromium.launch();
  try {
    for (const locale of ['en-US', 'en-GB']) {
      const context = await browser.newContext({ locale });
      const page = await context.newPage();
      await page.addScriptTag({ path: path.join(__dirname, '../apps/desktop/web/i18n.js') });
      for (const language of ['zh-CN', 'en']) {
        const result = await page.evaluate(language => {
          AieyesI18n.apply(language);
          const formatter = new Intl.DateTimeFormat(AieyesI18n.locale, { hour: '2-digit', minute: '2-digit', ...AieyesI18n.clock });
          return { cycle: formatter.resolvedOptions().hourCycle, parts: formatter.formatToParts(new Date(2026, 0, 1, 13, 5)) };
        }, language);
        assert.equal(result.parts.some(p => p.type === 'dayPeriod'), locale === 'en-US', `${locale}/${language} clock must follow the runtime locale`);
        assert.equal(Number(result.parts.find(p => p.type === 'hour').value), locale === 'en-US' ? 1 : 13);
      }
      await context.close();
    }
    console.log('Clock: 12/24-hour runtime preference is preserved across Chinese/English interface changes');
  } finally { await browser.close(); }
})().catch(error => {console.error(error);process.exitCode = 1;});
