const {openGuide} = require('./ui-test-helpers.cjs');
// Acceptance of the real development host, shared mock and unmodified app pages.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('../apps/desktop/node_modules/playwright');
const { createPreviewServer, seed, scenarios } = require('./desktop-preview/server.cjs');
const { Model } = require('./desktop-preview/model.js');
const contract = require('../docs/core-contracts.json');
const output = path.resolve(__dirname, '../.local/desktop-preview');

async function modelChecks() {
  const initial = seed('workspace', 'windows');let failStorage = false;
  assert.equal(initial.dashboard.trendDays.length, 7);
  assert.equal(initial.dashboard.heatmap.length, 365);
  assert(initial.dashboard.trendDays.some(day => day.events === 0), 'Synthetic history includes zero-filled gaps');
  assert.notStrictEqual(initial.dashboard.heatmap, initial.dashboard.trendDays);
  const model = new Model(initial, contract.methods, contract.providers, { save() {if (failStorage) throw new Error('Storage full');} });
  const call = (method, params = {}) => model.invoke('engine_call', { method, params });
  let a = 0,b = 0;const offA = model.listen('test', () => a++),offB = model.listen('test', () => b++);
  model.emit('test', {});offA();model.emit('test', {});offB();assert.deepEqual([a,b], [1,2]);assert.equal(model.listeners.has('test'), false);
  const one = await call('dashboard', { days: 1 }),week = await call('dashboard', { days: 7 });
  assert.equal(one.days.length, 1);assert.equal(one.trendDays.length, 7);assert.equal(one.heatmap.length, 365);
  assert.equal(week.trendDays.length, 7);assert(week.summary.total > one.summary.total);
  const filtered = await call('dashboard', { provider: 'codex', days: 1 });assert(filtered.summary.total < one.summary.total);
  assert.equal((await call('dashboard', { provider: 'does-not-exist' })).summary.total, 0);
  assert.equal((await call('dashboard', { model: 'does-not-exist' })).summary.total, 0);
  const day = week.trendDays[0].key;assert.equal((await call('dashboard', { dateRange: { kind: 'custom', start: day, end: day } })).trendDays.length, 1);
  let base = structuredClone(model.data.settings),next = structuredClone(base);next.accounts[0].name = 'Saved account';
  model.fail = 'save';await assert.rejects(call('settings.patch', { base, settings: next }), /模拟保存失败/);assert.equal(model.data.settings.accounts[0].name, initial.settings.accounts[0].name);
  model.fail = 'none';await call('settings.patch', { base, settings: next });assert.equal(model.data.settings.accounts[0].name, 'Saved account');
  await assert.rejects(call('settings.patch', { base, settings: { ...next, accounts: base.accounts.map(v => ({ ...v, name: 'Stale edit' })) } }), /配置已变化/);
  base = structuredClone(model.data.settings);next = structuredClone(base);next.accounts[0].name = 'Failed write';failStorage = true;
  await assert.rejects(call('settings.patch', { base, settings: next }), /Storage full/);assert.equal(model.data.settings.accounts[0].name, 'Saved account');failStorage = false;
  await assert.rejects(call('codexAuth.login.start'), /预览模式/);await assert.rejects(call('wakeups.deploy'), /预览模式/);
  await assert.rejects(call('future.unknown'), /Unhandled preview call/);
  model.fail = 'query';await assert.rejects(call('dashboard'), /模拟查询失败/);model.fail = 'none';
  let active = true;model.hooks.active = () => active;model.delay = 20;const pending = call('settings.patch', { base, settings: next });active = false;await assert.rejects(pending, /预览已切换/);
  console.log('Preview backend: filtering, conflicts, rollback, failure injection, event cleanup and external boundaries passed.');
}

(async () => {
  await modelChecks();fs.mkdirSync(output, { recursive: true });
  const host = createPreviewServer();let browser;
  await new Promise(resolve => host.server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${host.server.address().port}`;
  try {
    assert.equal((await fetch(origin + '/preview/seed?platform=unknown')).status, 400);
    assert.equal((await fetch(origin + '/app/%2e%2e%2fpackage.json')).status, 403);
    assert.equal((await fetch(origin + '/app/.git/config')).status, 403);
    assert.equal((await fetch(origin + '/app/fonts/HarmonyOS_Sans_SC_Regular.ttf')).headers.get('content-type'), 'font/ttf');
    const html = await (await fetch(origin + '/app/index.html')).text();assert(html.indexOf('/preview/bridge.js') < html.indexOf('contracts.js'));
    assert(!(await (await fetch(origin + '/')).text()).includes('unsafe-eval'));
    browser = await chromium.launch();
    // Reproduce previewing Linux on a Windows host: the browser's initial
    // platform guess must yield to desktop_info before the titlebar is used.
    const page = await browser.newPage({ viewport: { width: 1600, height: 1150 }, userAgent:'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36' }),errors = [],outside = [];
    page.on('pageerror', error => errors.push(String(error)));
    page.on('request', req => {if (/^https?:/.test(req.url()) && !req.url().startsWith(origin + '/')) outside.push(req.url());});
    async function ready() {
      await page.waitForFunction(() => document.querySelector('#app').dataset.ready === 'true');
      const frame = page.frames().find(f => f.url().includes('/app/'));assert(frame);
      await frame.evaluate(() => AieyesApp.ready);
      await frame.waitForFunction(() => state.settings && state.dashboard && !state.busy && !state.dashboardPending);
      assert(!/TypeError|ReferenceError|Unhandled preview call|Unexpected call:/.test(await frame.locator('body').innerText()));
      return frame;
    }
    async function route(platform, scenario = 'workspace', surface = 'main') {await page.goto(`${origin}/?platform=${platform}&scenario=${scenario}&surface=${surface}`);return ready();}
    let frame = await route('windows');
    assert.equal(await frame.locator('html').getAttribute('data-platform'), 'windows');assert(await frame.locator('#titlebar').isVisible());
    await frame.evaluate(() => AieyesApp.openSettings({ category: 'hosts' }));
    await frame.locator('[data-edit="preview-host"]').click();await frame.locator('#field-name').fill('Browser saved server');
    await page.locator('#failure').selectOption('save');await frame.locator('#editor-form button[type=submit]').click();
    await frame.waitForFunction(() => document.querySelector('#editor-error').textContent.includes('模拟保存失败'));
    assert.equal(await frame.locator('#field-name').inputValue(), 'Browser saved server');
    assert.equal(await page.evaluate(() => PreviewHost.model.data.settings.hosts[0].name), '训练服务器');
    await page.locator('#failure').selectOption('none');await frame.locator('#editor-form button[type=submit]').click();await frame.locator('#editor').waitFor({ state: 'hidden' });
    await page.reload();frame = await ready();assert.equal(await page.evaluate(() => PreviewHost.model.data.settings.hosts[0].name), 'Browser saved server');
    await frame.evaluate(() => AieyesApp.openSettings({ accountKey: 'codex:codex' }));await frame.locator('#account-name').fill('Browser account');await frame.locator('#account-name').blur();
    await page.waitForFunction(() => PreviewHost.model.data.settings.accounts.find(a => a.id === 'codex').name === 'Browser account');
    await frame.evaluate(() => AieyesApp.openSettings({ category: 'prices' }));await frame.locator('[data-price="test-model"]').click();await frame.locator('#field-name').fill('Browser model');
    await frame.locator('#editor-form button[type=submit]').click();await frame.locator('#editor').waitFor({ state: 'hidden' });assert.equal(await page.evaluate(() => PreviewHost.model.data.prices[0].name), 'Browser model');assert.equal(await page.evaluate(() => PreviewHost.model.data.prices.length), 2);
    await frame.locator('#reference-prices').click();await frame.waitForFunction(() => document.querySelector('#message').textContent.includes('已智能引用'));assert(!/TypeError|ReferenceError/.test(await frame.locator('body').innerText()));
    await frame.evaluate(() => AieyesApp.openSettings({ category: 'wakeups' }));await frame.locator('[data-wake-action=edit]').first().click();await frame.locator('#field-name').fill('Browser wakeup');
    await frame.locator('#editor-form button[type=submit]').click();await frame.locator('#editor').waitFor({ state: 'hidden' });assert.equal(await page.evaluate(() => PreviewHost.model.data.wakeups[0].task.name), 'Browser wakeup');
    await frame.locator('#wake-add').click();await frame.locator('#field-name').fill('New draft');await frame.locator('#wake-model').fill('test-model');await frame.locator('#editor-form button[type=submit]').click();await frame.locator('#editor').waitFor({ state: 'hidden' });
    assert(await page.evaluate(() => PreviewHost.model.data.wakeups.some(r => r.task.name === 'New draft' && !r.deployment)));
    await page.locator('#theme').selectOption('dark');await frame.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
    await page.locator('#language').selectOption('en');await frame.waitForFunction(() => AieyesI18n.language === 'en');
    await page.reload();frame = await ready();assert.equal(await frame.locator('html').getAttribute('data-theme'), 'dark');
    assert.equal(await page.evaluate(() => PreviewHost.model.data.wakeups[0].task.name), 'Browser wakeup');
    await page.locator('[data-platform=linux]').click();frame = await ready();assert.equal(await frame.locator('html').getAttribute('data-platform'), 'linux');assert(await frame.locator('#titlebar').isHidden());assert(await page.locator('#linux-title').isVisible());
    assert.equal(await page.evaluate(() => PreviewHost.model.data.settings.hosts[0].name), '训练服务器');
    await frame.evaluate(() => AieyesApp.openSettings({ category: 'hosts' }));await frame.locator('[data-edit="preview-host"]').click();await frame.locator('#field-name').fill('Linux server');await frame.locator('#editor-form button[type=submit]').click();await frame.locator('#editor').waitFor({ state: 'hidden' });
    await page.locator('[data-platform=windows]').click();frame = await ready();assert.equal(await page.evaluate(() => PreviewHost.model.data.settings.hosts[0].name), 'Browser saved server');
    await page.locator('[data-surface=panel]').click();frame = await ready();assert.equal(await frame.locator('body').getAttribute('data-view'), 'panel');
    await frame.locator('#panel-settings').click();frame = await ready();assert.equal(await page.locator('[data-surface=main]').getAttribute('aria-pressed'), 'true');assert.equal(await frame.evaluate(() => state.page), 'settings');
    await page.locator('[data-surface=panel]').click();frame = await ready();await frame.locator('#panel-refresh').focus();await page.keyboard.press('Escape');
    await frame.waitForFunction(() => document.body.dataset.view === 'ball');await page.locator('#restore').click();await frame.waitForFunction(() => document.body.dataset.view === 'panel');
    await page.locator('[data-surface=main]').click();frame = await ready();await frame.locator('#window-minimize').click();assert(await page.locator('#closed').isVisible());await page.locator('#restore').click();assert(await page.locator('#scaled').isVisible());
    await page.locator('#scenario').selectOption('single');frame = await ready();assert.equal(await page.evaluate(() => PreviewHost.model.data.settings.accounts.length), 1);
    await page.locator('#scenario').selectOption('workspace');frame = await ready();assert.equal(await page.evaluate(() => PreviewHost.model.data.settings.hosts[0].name), 'Browser saved server');
    await page.locator('#reset').click();await page.locator('#confirm-reset').click();frame = await ready();assert.equal(await page.evaluate(() => PreviewHost.model.data.settings.hosts[0].name), '训练服务器');assert.equal(await frame.locator('html').getAttribute('data-theme'), 'light');
    await page.locator('#logs').evaluate(el => {el.open = true;});await page.waitForFunction(() => document.querySelector('#log-list').children.length > 0);
    await page.evaluate(async () => {try {await PreviewHost.model.invoke('engine_call', { method: 'future.unknown', params: { apiKey: 'never-display-this' } });} catch {}});
    await page.waitForFunction(() => document.querySelector('#log-list').textContent.includes('future.unknown'));assert(!(await page.locator('#log-list').innerText()).includes('never-display-this'));
    await page.locator('#logs').evaluate(el => {el.open = false;});
    for (const platform of ['windows', 'linux']) {
      frame = await route(platform);
      console.log(`Checking ${platform} pages and scenario matrix…`);
      for (const target of [{ category: 'integrations' }, { category: 'hosts' }, { category: 'prices' }, { category: 'wakeups' }, ...['appearance','desktop','network','refresh','notifications','updates'].map(section => ({ category: 'general', section }))]) {
        await frame.evaluate(target => AieyesApp.openSettings(target), target);
        await frame.waitForTimeout(80);assert(!/TypeError|ReferenceError|Unhandled preview call|Unexpected call:/.test(await frame.locator('body').innerText()), JSON.stringify(target));
      }
      await openGuide(frame);await frame.waitForFunction(() => Boolean(document.querySelector('.setup-guide')));assert(!/TypeError|ReferenceError/.test(await frame.locator('body').innerText()));
      await frame.evaluate(() => AieyesApp.navigatePage('analysis'));await frame.locator('#days').selectOption('7');await frame.waitForFunction(() => state.appliedFilters.days === 7 && !state.dashboardPending);
      assert.equal(await frame.evaluate(() => state.dashboard.trendDays.length), 7);
      await page.locator('#failure').selectOption('query');await frame.locator('#days').selectOption('30');await frame.waitForFunction(() => state.dashboardError.includes('模拟查询失败'));
      await page.locator('#failure').selectOption('none');await frame.locator('#days').selectOption('7');await frame.waitForFunction(() => !state.dashboardError && !state.dashboardPending);
      await frame.evaluate(() => AieyesApp.navigatePage('servers'));await page.screenshot({ path: path.join(output, `${platform}-servers.png`) });
      await page.locator('#size').selectOption('minimum');await frame.waitForFunction(() => innerWidth === 640);assert(await frame.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      for (const scenario of scenarios) {
        for (const surface of ['main','panel']) {
          frame = await route(platform, scenario, surface);
          await page.locator('#theme').selectOption('dark');await frame.waitForFunction(() => document.documentElement.dataset.theme === 'dark');
          if (platform === 'linux' && scenario === 'workspace' && surface === 'panel') {await frame.locator('#panel-close').click();await page.locator('#restore').click();await frame.waitForFunction(() => document.body.dataset.view === 'panel');}
          if (scenario === 'workspace' && surface === 'panel') await page.screenshot({ path: path.join(output, `${platform}-panel.png`) });
        }
      }
    }
    // A malformed saved state must recover to a usable scene.
    await page.evaluate(() => localStorage.setItem('aieyes.preview.v1.state.linux.workspace', '{broken'));
    frame = await route('linux');assert(await frame.locator('#content').isVisible());
    await page.evaluate(() => {const key = 'aieyes.preview.v1.state.linux.workspace',saved = JSON.parse(localStorage.getItem(key));delete saved.settings.appearance;localStorage.setItem(key, JSON.stringify(saved));});
    frame = await route('linux');assert.equal(await frame.locator('html').getAttribute('data-theme'), 'light');
    assert.deepEqual(errors, []);assert.deepEqual(outside, []);
    console.log('Desktop preview: Windows/Linux × 7 scenes × main/panel, all settings pages, edits/retry/persistence/reset, scoped storage, navigation, logs, fonts and minimum layout passed.');
  } finally {await browser?.close();await host.close();}
})().catch(error => {console.error(error);process.exitCode = 1;});
