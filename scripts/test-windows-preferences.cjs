/* Called by test-windows-preferences.ps1 in an isolated interactive WebView2 process. */
const assert = require('node:assert/strict');
const {chromium} = require(process.env.AIEYES_PLAYWRIGHT_PATH || '../apps/desktop/node_modules/playwright');

(async () => {
  const [cycle, mode, output] = process.argv.slice(2);
  let browser;
  try {
    let connectionError;
    for (let attempt = 0; attempt < 100; attempt++) {
      try { browser = await chromium.connectOverCDP('http://127.0.0.1:9334'); break; }
      catch (error) { connectionError = error; await new Promise(resolve => setTimeout(resolve, 200)); }
    }
    assert(browser, `WebView2 debugging endpoint did not start: ${connectionError}`);
    let page;
    for (let attempt = 0; attempt < 100; attempt++) {
      for (const candidate of browser.contexts().flatMap(context => context.pages())) {
        if (await candidate.evaluate(() => !!window.AieyesApp && !document.querySelector('#panel')).catch(() => false)) {page = candidate; break;}
      }
      if (page) break;
      await new Promise(resolve => setTimeout(resolve, 200));
    }
    assert(page, 'Main WebView2 page missing');
    await page.evaluate(() => AieyesApp.ready);
    const invoke = (command, args = {}) => page.evaluate(({command, args}) => __TAURI__.core.invoke(command, args), {command, args});
    let info = await invoke('desktop_info');
    assert.equal(info.hourCycle, cycle, 'Native hour-cycle override');
    for (const language of ['zh-CN', 'en']) {
      const value = await page.evaluate(language => {
        AieyesI18n.apply(language);
        return new Intl.DateTimeFormat(AieyesI18n.locale, {hour:'2-digit', minute:'2-digit', ...AieyesI18n.clock}).formatToParts(new Date(2026, 0, 1, 13, 5));
      }, language);
      assert.equal(Number(value.find(part => part.type === 'hour').value), cycle === 'h12' ? 1 : 13);
      assert.equal(value.some(part => part.type === 'dayPeriod'), cycle === 'h12');
    }
    if (mode === 'materials') {
      assert.deepEqual(info.materialPreferences, {main:'default', floating:'default'});
      assert.equal(info.materials.main, 'mica');
      assert.equal(info.materials.floating, 'acrylic');
      await page.evaluate(() => AieyesApp.openSettings({category:'general', section:'desktop'}));
      for (const [main, floating] of [['acrylic','mica'], ['opaque','mica'], ['mica','opaque'], ['default','default'], ['acrylic','mica']]) {
        await page.locator('#material-main').selectOption(main);
        await page.waitForFunction(main => document.querySelector('#material-main')?.value === main && !document.querySelector('#material-main').disabled, main);
        await page.locator('#material-floating').selectOption(floating);
        await page.waitForFunction(floating => document.querySelector('#material-floating')?.value === floating && !document.querySelector('#material-floating').disabled, floating);
        info = await invoke('desktop_info');
        assert.deepEqual(info.materialPreferences, {main, floating});
        assert.equal(info.materials.main, main === 'default' ? 'mica' : main);
        assert.equal(info.materials.floating, floating === 'default' ? 'acrylic' : floating);
      }
      await assert.rejects(invoke('desktop_materials', {main:'invalid', floating:'mica'}));
      assert.deepEqual((await invoke('desktop_info')).materialPreferences, {main:'acrylic', floating:'mica'});
      await page.screenshot({path:output});
    } else {
      assert.deepEqual(info.materialPreferences, {main:'acrylic', floating:'mica'}, 'Independent choices survive restart');
      assert.equal(info.materials.main, mode === 'opaque' ? 'opaque' : 'acrylic');
      assert.equal(info.materials.floating, mode === 'opaque' ? 'opaque' : 'mica');
    }
    console.log(`PASS native WebView2 ${cycle}, ${mode}: ${JSON.stringify(info.materialPreferences)} / ${JSON.stringify(info.materials)}`);
  } finally { if (browser) await browser.close(); }
})().catch(error => {console.error(error); process.exitCode = 1;});
