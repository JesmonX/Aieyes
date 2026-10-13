const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const {chromium} = require('../apps/desktop/node_modules/playwright');
const {serveFont} = require('./ui-test-helpers.cjs');

function fixture({platform, failStatus = false}) {
  window.setInterval = () => 0;
  const stamp = Math.floor(Date.now() / 1000);
  const accounts = Array.from({length: 6}, (_, i) => ({id: 'a' + i, name: 'Codex 账号 ' + i, provider: 'codex', quotaEnabled: true}));
  accounts.push({id: 'claude', name: 'Claude 账号', provider: 'claude', quotaEnabled: true}, {id: 'deepseek', name: 'DeepSeek 账号', provider: 'deepseek', quotaEnabled: true});
  const sources = accounts.map(a => ({id: 's-' + a.id, provider: a.provider, accountId: a.id, name: a.name + '来源', path: '/fixture/' + a.id, enabled: true, hostId: null}));
  const settings = {version: 6, accounts, sources, agents: ['codex','claude','antigravity','deepseek','custom'].map(provider => ({provider, enabled: sources.some(s => s.provider === provider), machineIds: ['local']})), hosts: [{id: 'host', name: '测试服务器', target: 'fixture.invalid', enabled: true, metrics: ['cpu'], details: [], devices: [], shell: '/bin/bash', identityFile: ''}], modelMappings: {}, refreshSeconds: 300, historyRefreshSeconds: 400, serverForegroundRefreshSeconds: 3, serverRefreshSeconds: 12, historyShowDeletedAccounts: true, proxy: {mode: 'custom', url: 'http://proxy.invalid:8080'}, proxyTestUrls: ['https://fixture.invalid'], appearance: {theme: 'system', accent: 'indigo', chartPalette: 'classic', chartContrast: 'high'}, codexApis: [{id: 'api', sourceId: 's-a0', name: '已有 API', model: 'fixture', baseUrl: 'https://fixture.invalid/v1'}]};
  const usage = {total: 70000, cost: 1, pricedTokens: 70000, events: 1, tokens: {input: 30000, output: 10000, cacheRead: 28000, cacheWrite: 2000}};
  const record = {id: 'running', accountKey: 'codex:a0', kind: 'quota', status: 'active', samplingMode: 'manual', valuationMode: 'fiveHour', fiveHourValue: 20, weeklyValue: 100, weeklyDirectValue: 100, weeklyRatioValue: 110, startedAt: stamp - 100, checkpointAt: stamp, sourceIds: ['s-a0'], sourceNames: ['Codex 账号 0来源'], windowName: '5h', windowId: 'five', calculationNote: '测试样本', reason: '', consumedPercent: 10, totalTokens: 70000, pricedTokens: 70000, cost: 1, prices: []};
  const quotas = accounts.map(a => ({provider: a.provider, accountId: a.id, name: a.name, sourceId: 's-' + a.id, updatedAt: stamp, origin: 'live', windows: a.provider === 'deepseek' ? [] : [{id: 'five', name: '5h', windowMinutes: 300, usedPercent: 20, resetsAt: stamp + 1000}, {id: 'week', name: '7d', windowMinutes: 10080, usedPercent: 10, resetsAt: stamp + 100000}], credits: a.provider === 'codex' ? {balance: '100', unlimited: false} : null}));
  const nav = window.navigationFixture = {settings, quotas, records: [record, {...record, id: 'history', status: 'completed', checkpointAt: stamp - 20}], calls: [], listeners: {}, failSave: false, holdSave: false, failStatus};
  const copy = v => structuredClone(v);
  nav.emit = (name, payload) => Promise.all((nav.listeners[name] || []).map(fn => fn({payload})));
  function merge(base, next, current) {
    const out = copy(current);
    for (const key of Object.keys(next)) if (JSON.stringify(base[key]) !== JSON.stringify(next[key])) out[key] = copy(next[key]);
    return out;
  }
  function dashboard(params = {}) {
    const day = new Date().toISOString().slice(0, 10);
    return {generatedAt: Date.now() / 1000, summary: usage, quotas: quotas.filter(q => (!params.provider || q.provider === params.provider) && (!params.accountId || q.accountId === params.accountId)), quotaOrder: accounts.map(a => a.provider + ':' + a.id), quotaEstimates: copy(nav.records), creditEstimates: [], models: [{...usage, key: 'fixture'}], modelOptions: ['fixture'], trendDays: [{...usage, key: day}], heatmap: [{...usage, key: day}], dayModels: [{day, model: 'fixture', usage}], pricingGaps: [], sources: sources.map(s => ({...s, status: {updatedAt: stamp}}))};
  }
  window.__TAURI__ = {
    event: {async listen(name, fn) {(nav.listeners[name] ||= []).push(fn); return () => {}; }},
    window: {getCurrentWindow: () => ({async isMaximized() {return false;}, async onResized() {return () => {};}})},
    core: {async invoke(command, args) {
      nav.calls.push({command, args: copy(args)});
      if (command === 'desktop_info' || command === 'desktop_panel') return {platform, page: 'agent', mode: 'floating', effectiveMode: 'floating', sessions: [], panelOpen: true, materials: {main: 'opaque', floating: 'opaque'}};
      if (command !== 'engine_call') return {};
      const {method, params} = args;
      if (method === 'hello') return {version: 'fixture'};
      if (method === 'settings.get') return copy(nav.settings);
      if (method === 'dashboard' || method === 'dashboard.summary') return dashboard(method === 'dashboard.summary' ? {} : params);
      if (method === 'settings.patch') {
        if (nav.holdSave) {nav.holdSave = false; await new Promise(resolve => nav.releaseSave = resolve);}
        if (nav.failSave) throw Error('模拟保存失败');
        nav.settings = merge(params.base, params.settings, nav.settings); return copy(nav.settings);
      }
      if (method === 'credentials.save') return {path: '/fixture/updated-key'};
      if (method === 'quotas.refresh') return copy(quotas);
      if (method === 'quotaEstimates.stop') {nav.records.find(r => r.id === params.id).status = 'completed'; return {};}
      if (method === 'hosts.sample') return [{id: 'host', sample: {timestamp: Date.now()/1000, cpu: [{id: 'cpu', utilization: 12}], errors: {}}}];
      if (method === 'accounts.status.refresh') return {accountKey: params.accountKey, sourceId: params.sourceId, machineId: 'local', current: params.accountKey.endsWith(':a0'), credential: true, checkedAt: stamp};
      if (method === 'accounts.status.get' && nav.failStatus) throw Error('模拟设备状态读取失败');
      if (['accounts.status.get','accounts.cleanup.list','sources.scan','prices.list','wakeups.list'].includes(method)) return [];
      if (method === 'accounts.deployments.sync') return {failedTasks: []};
      if (method === 'sessions.list') return {sessions: [], warnings: []};
      if (method === 'quotas.schedule') return {nextDueAt: stamp + 86400};
      return {};
    }}
  };
}

(async () => {
  const root = path.resolve(__dirname, '../apps/desktop/web'), out = path.resolve(__dirname, '../.local/ui-adjustments-20261013/web');
  fs.mkdirSync(out, {recursive: true});
  const server = http.createServer((req, res) => {
    if (serveFont(req, res)) return;
    const file = path.join(root, path.basename(new URL(req.url, 'http://local').pathname) || 'index.html');
    if (!fs.existsSync(file)) {res.writeHead(404).end(); return;}
    res.setHeader('content-type', ({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)] || 'text/plain'); res.end(fs.readFileSync(file));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({headless: true}), url = `http://127.0.0.1:${server.address().port}`;
  const settled = page => page.waitForFunction(() => !state.generalSavePending && !state.settingsCommitting && !state.settingsSaving && !state.busy);
  try {
    for (const platform of ['windows', 'linux']) {
      const context = await browser.newContext({viewport: {width: 1120, height: 800}, reducedMotion: 'reduce'}), errors = [];
      const page = await context.newPage(); page.on('pageerror', error => errors.push(String(error)));
      await page.addInitScript(fixture, {platform, failStatus: true});
      await page.goto(url); await page.evaluate(() => AieyesApp.ready);
      await page.locator('[data-page=settings]').click();
      assert.deepEqual(await page.locator('[data-settings-tab]').allTextContents(), ['Agent 接入','服务器','价格','定时唤醒','常规']);
      assert.equal(await page.locator('[data-settings-agent]').count(), 5);
      assert.equal(await page.locator('[data-agent-enable]').count(), 1);
      await page.locator('#account-status-error').waitFor();
      const failedStatusCalls = await page.evaluate(() => navigationFixture.calls.filter(c => c.args?.method === 'accounts.status.get').length);
      await page.locator('[data-settings-agent=claude]').click();
      await page.locator('[data-settings-tab=prices]').click();
      await page.evaluate(() => navigationFixture.emit('desktop:navigate', 'settings'));
      await page.locator('[data-settings-tab=integrations][aria-selected=true]').waitFor();
      assert.equal(await page.locator('[data-settings-agent=claude]').getAttribute('aria-current'), 'page');
      assert.equal(await page.evaluate(() => navigationFixture.calls.filter(c => c.args?.method === 'accounts.status.get').length), failedStatusCalls, 'failed automatic status reads do not retry on every render');
      await page.locator('[data-account-retry-status]').click();
      await page.waitForFunction(() => !AieyesAgentSettings.statusBusy);
      assert.equal(await page.evaluate(() => navigationFixture.calls.filter(c => c.args?.method === 'accounts.status.get').length), failedStatusCalls + 1, 'manual retry performs one read after failure');
      await page.evaluate(() => navigationFixture.failStatus = false);
      await page.locator('[data-account-retry-status]').click();
      await page.waitForFunction(() => !AieyesAgentSettings.statusBusy);
      await page.locator('#account-status-error').waitFor({state:'detached'});
      assert.equal(await page.locator('#account-status-error').count(), 0, 'successful retry clears the retained error');
      assert(!await page.locator('#message').innerText().then(text => text.includes('模拟设备状态读取失败')), 'successful retry clears its own notification');
      await page.evaluate(() => openSettings({category: 'accounts', accountKey: 'codex:a5'}));
      assert(await page.locator('[data-account-panel]').isDisabled());
      assert.match(await page.locator('.account-config').innerText(), /5\/5.*最多显示 5/);
      await page.locator('[data-account-panel-manage]').click();
      await page.locator('#quota-dialog[open]').waitFor();
      assert(await page.locator('[data-panel-account="codex:a5"]').isDisabled());
      await page.locator('[data-panel-account="codex:a0"]').uncheck();
      await page.locator('[data-panel-account="codex:a5"]').check();
      const order = await page.evaluate(() => state.dashboard.quotaOrder);
      await page.locator('#panel-accounts-save').click();
      assert(await page.locator('[data-account-panel]').isChecked());
      assert.deepEqual(await page.evaluate(() => state.dashboard.quotaOrder), order);

      await page.locator('[data-settings-agent=deepseek]').click();
      assert.equal(await page.locator('[data-agent-estimates]').count(), 0);
      await page.locator('[data-account-edit="deepseek:deepseek"]').click();
      await page.getByRole('button', {name: '更新 API Key', exact: true}).click();
      assert.match(await page.locator('#editor-title').innerText(), /更新 API Key/);
      await page.locator('#field-apiKey').fill('fixture-key');
      await page.locator('#editor-form button[type=submit]').click();
      await page.waitForFunction(() => !document.querySelector('#editor').open);
      assert.equal(await page.evaluate(() => state.settings.sources.find(s => s.id === 's-deepseek').path), '/fixture/updated-key');
      assert.equal(await page.evaluate(() => state.settings.accounts.filter(a => a.provider === 'deepseek').length), 1);
      await page.locator('[data-settings-agent=claude]').click();
      await page.locator('[data-account-edit="claude:claude"]').click();
      await page.getByRole('button', {name: '连接机器', exact: true}).click();
      assert.equal(await page.locator('[name=sourceIds]').count(), 1);
      await page.locator('#cancel-editor').click();

      await page.locator('[data-settings-tab=general]').click();
      assert.deepEqual(await page.locator('[data-general-page]').allTextContents(), ['外观','桌面显示','网络代理','刷新与历史','更新']);
      for (const shortcut of ['Control+,', 'Meta+,']) {
        await page.keyboard.press(shortcut);
        assert.equal(await page.evaluate(() => state.settingsTab), 'integrations', shortcut + ' opens Agent integrations from another settings category');
        await page.locator('[data-settings-tab=general]').click();
      }
      const before = await page.evaluate(() => structuredClone(state.settings));
      await page.locator('#field-appearanceAccent').selectOption('teal'); await settled(page);
      const after = await page.evaluate(() => state.settings);
      for (const key of ['refreshSeconds','historyRefreshSeconds','serverForegroundRefreshSeconds','serverRefreshSeconds','proxy','proxyTestUrls','historyShowDeletedAccounts','codexApis']) assert.deepEqual(after[key], before[key], 'appearance leaves ' + key + ' intact');
      await page.locator('[data-general-page=refresh]').click();
      await page.evaluate(() => navigationFixture.failSave = true);
      await page.locator('#field-refreshSeconds').fill('600'); await page.locator('#field-refreshSeconds').blur(); await settled(page);
      await page.locator('#settings-retry-save:visible').waitFor();
      await page.locator('[data-general-page=network]').click();
      assert.equal(await page.locator('#field-refreshSeconds').count(), 0);
      await page.locator('[data-general-page=refresh]').click();
      assert.equal(await page.locator('#field-refreshSeconds').inputValue(), '600');
      await page.evaluate(() => navigationFixture.failSave = false);
      await page.locator('#settings-retry-save').click(); await settled(page);
      assert.equal(await page.evaluate(() => state.settings.refreshSeconds), 600);

      await page.locator('[data-general-page=appearance]').click();
      await page.evaluate(() => {navigationFixture.holdSave = true; navigationFixture.settings.codexApis[0].name = '并发更新的 API';});
      await page.locator('#field-appearanceTheme').selectOption('dark');
      await page.waitForFunction(() => navigationFixture.releaseSave);
      await page.locator('[data-general-page=network]').click();
      await page.locator('#field-proxyTestUrls').fill('https://new.invalid'); await page.locator('#field-proxyTestUrls').blur();
      await page.evaluate(() => navigationFixture.releaseSave()); await settled(page);
      assert.deepEqual(await page.evaluate(() => state.settings.proxyTestUrls), ['https://new.invalid']);
      assert.equal(await page.evaluate(() => state.settings.codexApis[0].name), '并发更新的 API');
      assert.equal(await page.evaluate(() => state.settings.appearance.theme), 'dark');

      await page.locator('[data-page=agent]').click();
      const scans = await page.evaluate(() => navigationFixture.calls.filter(c => c.args?.method === 'sources.scan').length);
      await page.locator('[data-refresh-status=scan]').click();
      await page.locator('#refresh-status-popover').waitFor({state: 'visible'}); await page.keyboard.press('Escape');
      assert.equal(await page.evaluate(() => navigationFixture.calls.filter(c => c.args?.method === 'sources.scan').length), scans);
      await page.evaluate(async () => {state.provider = 'claude'; await loadDashboard();});
      await page.evaluate(() => navigationFixture.missingQuota = navigationFixture.quotas.shift());
      await page.locator('[data-manage-sampling]').click();
      await page.locator('[data-sampling-open]').first().click();
      await page.locator('#estimate-stop').waitFor();
      assert.match(await page.locator('#quota-dialog-title').innerText(), /Codex 账号 0/);
      assert.match(await page.locator('#quota-dialog').innerText(), /当前没有可用的额度快照；已有采样和历史仍可管理/);
      assert.equal(await page.evaluate(() => state.provider), 'claude');
      await page.locator('#estimate-stop').click();
      await page.waitForFunction(() => navigationFixture.records.every(r => r.status === 'completed'));
      await page.waitForFunction(() => !state.estimateBusy); await page.locator('#quota-close').click();
      await page.evaluate(() => navigationFixture.quotas.unshift(navigationFixture.missingQuota));
      await page.evaluate(() => openSettings({provider: 'codex'}));
      await page.locator('[data-agent-estimates=codex]').click();
      await page.locator('#manual-sampling').waitFor();
      assert.equal(await page.locator('#manual-sampling').evaluate(el=>el.open), false);
      assert.equal(await page.locator('[data-valuation-record]').count(), 0, 'manual records are not mounted until expanded');
      assert.equal(await page.locator('[data-auto-save][role=switch]').count(), 6);
      assert.match(await page.locator('#quota-dialog').innerText(), /自动估值5h\/7d限额和credits的价值/);
      await page.locator('#manual-sampling > summary').click();
      await page.getByText('手动采样历史', {exact: true}).waitFor();
      assert.equal(await page.locator('[data-valuation-record]').count(), 2);
      await page.screenshot({path: path.join(out, platform + '-valuation.png')});
      await page.locator('#quota-close').click();

      await page.locator('[data-page=servers]').click(); await page.waitForFunction(() => !state.serverBusy);
      await page.locator('#server-filter').selectOption('errors');
      assert.equal(await page.locator('.server-card').count(), 0);
      assert.equal(await page.locator('#add-first-host').count(), 0);
      await page.locator('#clear-server-filter').click();
      await page.locator('[data-host-settings=host]').click();
      assert.equal(await page.locator('#field-target').inputValue(), 'fixture.invalid');
      await page.locator('#cancel-editor').click();
      await page.locator('[data-page=servers]').click();
      await page.locator('#add-server').click();
      assert.equal(await page.locator('#field-target').inputValue(), '');
      await page.locator('#cancel-editor').click();
      await page.locator('[data-page=settings]').click();
      await page.screenshot({path: path.join(out, platform + '-integration.png')});
      await page.setViewportSize({width: 640, height: 440});
      for (const provider of ['codex','claude','antigravity','deepseek','custom']) {
        await page.locator('[data-settings-agent=' + provider + ']').click();
        assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), provider + ' settings fit the minimum window');
      }
      await page.locator('[data-settings-agent=codex]').click();
      await page.screenshot({path: path.join(out, platform + '-integration-minimum.png')});
      await page.locator('[data-settings-agent=codex]').focus(); await page.keyboard.press('ArrowDown');
      assert.equal(await page.locator('[data-settings-agent=claude]').getAttribute('aria-current'), 'page');
      assert(await page.locator('[data-settings-agent=claude]').evaluate(el => el === document.activeElement));
      await page.locator('[data-settings-tab=general]').click(); await page.locator('[data-general-page=network]').click();
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'network settings fit the minimum window');
      await page.screenshot({path: path.join(out, platform + '-network-minimum.png')});
      await page.setViewportSize({width: 1120, height: 800});

      const panel = await context.newPage(); panel.on('pageerror', error => errors.push(String(error)));
      await panel.addInitScript(fixture, {platform});
      await panel.setViewportSize({width: 450, height: 720}); await panel.goto(url + '/floating.html'); await panel.evaluate(() => AieyesApp.ready);
      assert.equal(await panel.locator('.quota-disclosure[open]').count(), 0, 'multiple displayed accounts initially collapse');
      await page.evaluate(() => localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify({version: 2, providers: {codex: {mode: 'custom', keys: ['codex:a0']}, claude: {mode: 'custom', keys: []}, deepseek: {mode: 'custom', keys: []}}})));
      await panel.waitForFunction(() => document.querySelectorAll('.quota-card').length === 1);
      assert.equal(await panel.locator('.quota-disclosure[open]').count(), 1, 'a single account initially expands after selection changes');
      const counts = panel.locator('.device-count-line').first(); await counts.waitFor();
      assert(await counts.evaluate(el => {const r = el.getBoundingClientRect(); return r.right <= innerWidth && r.height <= parseFloat(getComputedStyle(el).lineHeight) + 1;}));
      assert.match(await panel.locator('#token-summary').getAttribute('title'), /缓存读取 ÷（普通输入＋缓存读取＋缓存写入），不包含输出 Token/);
      assert.match(await panel.locator('[data-estimate="codex:a0"]').getAttribute('title'), /7d 左值：5h 等价值 × 近期容量倍率；右值：同一采样时段的 API 等价成本 ÷ 周额度消耗比例/);
      await panel.screenshot({path: path.join(out, platform + '-single-panel.png')});
      await panel.locator('.quota-title').click();
      await panel.reload(); await panel.evaluate(() => AieyesApp.ready);
      assert.equal(await panel.locator('.quota-disclosure[open]').count(), 0, 'manual expansion preference survives reload');
      assert(await panel.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
      await panel.evaluate(() => openSettings({category: 'hosts', hostId: 'host'}));
      const target = await panel.evaluate(() => navigationFixture.calls.findLast(c => c.command === 'desktop_action').args.action);
      assert.equal(target, 'settings-target:' + JSON.stringify({category: 'hosts', hostId: 'host'}));
      await page.evaluate(target => navigationFixture.emit('desktop:navigate', target), target);
      assert.equal(await page.locator('#field-target').inputValue(), 'fixture.invalid', 'panel target opens the matching host editor in the main window');
      await page.locator('#cancel-editor').click();
      assert.deepEqual(errors, []); await context.close();
    }
    console.log('Settings navigation: Windows/Linux routes, minimum windows, keyboard navigation, status retry, limits, credentials, isolated saves, retained drafts, concurrent edits, missing-quota sampling, host entries, numeric help and compact defaults passed');
  } finally {await browser.close(); await new Promise(resolve => server.close(resolve));}
})().catch(error => {console.error(error); process.exitCode = 1;});
