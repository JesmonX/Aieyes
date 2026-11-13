// Sessions page, settings governance, deep links, motion fallbacks and the outbound settings pages.
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const {chromium} = require('../apps/desktop/node_modules/playwright');
const {serveFont} = require('./ui-test-helpers.cjs');
const {fixture} = require('./workspace-ui-fixture.cjs');

(async () => {
  const root = path.resolve(__dirname, '../apps/desktop/web');
  const server = http.createServer((req, res) => {
    if (serveFont(req, res)) return;
    const file = path.join(root, path.basename(new URL(req.url, 'http://local').pathname) || 'index.html');
    if (!fs.existsSync(file)) {res.writeHead(404).end(); return;}
    res.setHeader('content-type', ({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)] || 'text/plain'); res.end(fs.readFileSync(file));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({headless: true}), url = `http://127.0.0.1:${server.address().port}`;
  const open = async (platform, options = {}) => {
    const context = await browser.newContext({viewport: {width: 1120, height: 800}, reducedMotion: 'reduce', ...options}), errors = [];
    const page = await context.newPage(); page.on('pageerror', error => errors.push(String(error)));
    await page.addInitScript(fixture, {platform});
    await page.goto(url); await page.evaluate(() => AieyesApp.ready);
    return {context, page, errors};
  };
  const status = (page, sessions, extra = {}) => page.evaluate(async ({sessions, extra}) => {
    const info = await window.__TAURI__.core.invoke('desktop_info');
    await navigationFixture.emit('desktop:status', {...info, sessions, sources: [], ...extra});
  }, {sessions, extra});
  // The general pages this platform carries, in the order of the shared index.
  const GENERAL_PAGES = ['外观', '面板', '桌宠与会话', '启动与系统', '网络代理', '刷新与历史', '通知', 'Webhook', '聊天渠道', '高级', '更新与关于'];
  try {
    // aieyes:// settings links accept the same general page keys as the settings index.
    {
      const rust = fs.readFileSync(path.resolve(__dirname, '../apps/desktop/src-tauri/src/system.rs'), 'utf8');
      const web = fs.readFileSync(path.join(root, 'settings-index.js'), 'utf8');
      const list = (text, pattern) => [...text.match(pattern)[1].matchAll(/["']([a-z]+)["']/g)].map(m => m[1]);
      assert.deepEqual(list(rust, /GENERAL_PAGES: \[&str; \d+\] = \[([^\]]+)\]/), list(web, /generalPageKeys = \[([^\]]+)\]/));
    }
    // Server trend lines: fixed 60-minute window, gaps break the line, null minutes are skipped.
    {
      const vm = require('node:vm');
      const sandbox = {AieyesI18n: {t: (text, ...args) => text.replace(/\{(\d+)\}/g, (_, i) => String(args[i]))}, AieyesUI: {escapeHTML: (v) => String(v)}, state: {settings: {serverRefreshSeconds: 10, serverForegroundRefreshSeconds: 2}}};
      sandbox.globalThis = sandbox;vm.createContext(sandbox);vm.runInContext(fs.readFileSync(path.join(root, 'host-trend.js'), 'utf8'), sandbox);
      const trend = sandbox.AieyesHostTrend,end = 1_700_000_000;
      assert.equal(trend.gap({serverRefreshSeconds: 10, serverForegroundRefreshSeconds: 2}), 150);
      assert.equal(trend.gap({serverRefreshSeconds: 120, serverForegroundRefreshSeconds: 2}), 300);
      const minutes = Array.from({length: 90}, (_, i) => ({t: end - (89 - i) * 60, cpu: i % 2 ? 40 : 20, cpuMax: 95, memory: i >= 80 && i < 85 ? null : 50, gpu: null}));
      const cpu = trend.series(minutes, 'cpu', 150);
      assert.equal(cpu.points.length, 60, 'the window holds the last 60 minutes');
      assert.equal(cpu.start, end - 59 * 60);
      assert.equal(cpu.peak, 95, 'CPU peak uses the minute maximum');
      assert.equal(trend.summary(cpu), '60 分钟走势 · 均值 30% · 峰值 95%');
      const memory = trend.series(minutes, 'memory', 150);
      assert.equal(memory.points.length, 55, 'minutes without the metric are skipped, not drawn as 0');
      assert.equal((trend.svg(memory).match(/class="host-trend-line" d="([^"]+)"/)[1].match(/M/g) ?? []).length, 2, 'the missing minutes break the line');
      assert.equal(trend.hasLine(trend.series(minutes, 'gpu', 150)), false);
      const lone = trend.series([{t: end, cpu: 10}], 'cpu', 150);
      assert.equal(trend.hasLine(lone), false);assert(trend.svg(lone).includes('host-trend-dot'), 'an isolated reading is a dot');
      const stale = trend.series([{t: end - 600, cpu: 10}, {t: end - 540, cpu: 12}, {t: end, cpu: null, memory: 3}], 'cpu', 150);
      assert.equal(trend.latest(stale), null, 'no marker on a reading older than the gap');
      assert(!trend.svg(stale).includes('host-trend-dot'));
    }
    for (const platform of ['windows', 'linux']) {
      const {context, page, errors} = await open(platform);

      // Sessions page: empty state, tiles, waiting first, agent chips, lifecycle metadata only.
      await page.locator('[data-page=sessions]').click();
      assert.equal(await page.locator('#title').innerText(), '会话');
      assert.equal(await page.locator('[data-page=sessions]').getAttribute('aria-current'), 'page');
      await page.locator('.session-page .empty-state').waitFor();
      assert.match(await page.locator('.session-page .empty-state-text').innerText(), /暂无会话动态/);
      assert.equal(await page.locator('[data-session-connect]').count(), 0, 'accounts are already connected');
      assert(await page.locator('#live-sessions').isHidden(), 'the overview strip stays on the overview');
      assert(await page.locator('header .refresh-wrap').isHidden(), 'the live page has nothing to refresh');
      const now = Date.now();
      const sessions = [
        {id: 's1', provider: 'codex', source: 'Codex', machine: '本机', machineId: 'local', workspace: '/work/alpha', nativeSession: '01234567-89ab-4cde-8f01-23456789abcd', subagentCount: 2, phase: 'working', updatedAt: now / 1000, turnStartedAt: now - 65000, pid: 4242, processStart: 1, usage: {sessionTokens: 1200, turnTokens: 300, turnOutputTokens: 100, measuredOutputTokensPerSecond: 20, timedOutputTokens: 40, untimedRequests: 1, session: {input: 600, output: 100, cacheRead: 400, cacheWrite: 100}, turn: {input: 150, output: 100, cacheRead: 50, cacheWrite: 0}}},
        {id: 's2', provider: 'claude', machine: '测试服务器', machineId: 'host', workspace: '/work/beta', phase: 'needsInput', updatedAt: now / 1000, turnStartedAt: now - 5000},
        {id: 's3', provider: 'codex', client: 'codex_desktop', nativeSession: '01234567-89ab-4cde-8f01-23456789abce', machine: '本机', machineId: 'local', workspace: 'C:\\work\\gamma', phase: 'complete', updatedAt: now / 1000 - 60, turnStartedAt: now - 120000, turnEndedAt: now - 60000},
        {id: 's4', provider: 'codex', machine: '本机', machineId: 'local', phase: 'idle', updatedAt: now / 1000},
      ];
      await status(page, sessions);
      await page.locator('[data-session-list=running] .session-row').first().waitFor();
      assert.deepEqual(await page.locator('.session-stats .stat-value').allInnerTexts(), ['1', '1', '1']);
      assert.deepEqual(await page.locator('[data-session-list=running] .session-row').evaluateAll(rows => rows.map(row => row.dataset.phase)), ['needsInput', 'working'], 'waiting sessions lead');
      assert.deepEqual(await page.locator('[data-session-list=running] .session-info strong').allInnerTexts(), ['beta', 'alpha']);
      assert.deepEqual(await page.locator('[data-session-list=ended] .session-info strong').allInnerTexts(), ['gamma']);
      assert.equal(await page.locator('.session-page [data-focus-session]').count(), 2, 'local processes and native desktop thread routes can be focused');
      assert.equal(await page.locator('.session-page [data-focus-session=s3]').getAttribute('title'), '打开此会话');
      assert.match(await page.locator('.session-page .session-usage').innerText(), /部分 20 tok\/s/);
      assert.match(await page.locator('.session-page .session-usage').getAttribute('title'), /40%/);
      assert.match(await page.locator('.session-page .session-info').allInnerTexts().then(rows => rows.join('\n')), /#6789abcd/);
      const preview = path.resolve(__dirname, '../.local/ui-previews/session-improvements');
      fs.mkdirSync(preview, {recursive: true});
      await page.screenshot({path: path.join(preview, `sessions-${platform}.png`), animations: 'disabled'});
      assert.match(await page.locator('[data-session-timing=s1]').innerText(), /^本轮 1m0\ds$/);
      assert.match(await page.locator('[data-session-timing=s3]').innerText(), /^本轮 1m00s · /);
      assert(!/turnTokens|null|undefined/.test(await page.locator('.session-page').innerText()), 'unknown usage parts are omitted');
      assert.equal(await page.locator('[data-session-provider]').count(), 3);
      await page.locator('[data-session-provider=codex]').click();
      assert.deepEqual(await page.locator('.session-stats .stat-value').allInnerTexts(), ['1', '0', '1']);
      assert.equal(await page.locator('[data-session-provider=codex]').getAttribute('aria-pressed'), 'true');
      // The same rows stay in place while only the timing text ticks.
      const row = await page.locator('[data-session-list=running] .session-row').first().elementHandle();
      await status(page, sessions.map(s => ({...s, updatedAt: s.updatedAt})));
      assert(await row.evaluate(el => el.isConnected), 'an unchanged snapshot keeps the rendered rows');
      // A filter for an agent without sessions falls back to everything.
      await status(page, sessions.filter(s => s.provider !== 'codex'));
      assert.equal(await page.locator('[data-session-provider]').count(), 0);
      assert.deepEqual(await page.locator('[data-session-list=running] .session-info strong').allInnerTexts(), ['beta']);
      await status(page, [], {unavailable: true});
      assert.match(await page.locator('.session-unavailable').innerText(), /会话读取失败/);
      await status(page, []);
      await page.keyboard.press('Control+1');
      assert.equal(await page.locator('#title').innerText(), '总览');
      await page.keyboard.press('Control+4');
      assert.equal(await page.locator('#title').innerText(), '会话');
      await page.evaluate(() => navigationFixture.emit('desktop:detail-view', {page: 'sessions'}));
      assert.equal(await page.evaluate(() => state.page), 'sessions');

      // Settings governance: one page list, one search index, and pages where dialogs used to be.
      await page.locator('[data-settings-tab=general]').click();
      assert.deepEqual(await page.locator('[data-general-page]').allTextContents(), GENERAL_PAGES);
      const search = page.locator('#settings-search');
      await search.fill('排序');
      assert.deepEqual(await page.locator('[data-settings-result] strong').allInnerTexts(), ['账户顺序']);
      assert.equal(await page.locator('[data-settings-result] small').innerText(), '设置 › 面板');
      assert(await page.locator('#settings-panel').isHidden(), 'results stand in for the page while searching');
      assert(await search.evaluate(el => el === document.activeElement), 'typing keeps the search box focused');
      await search.press('Escape');
      assert.equal(await search.inputValue(), ''); assert(await page.locator('#settings-panel').isVisible());
      await search.fill('mica');
      assert.equal(await page.locator('[data-settings-result]').count(), platform === 'windows' ? 2 : 0, 'window materials are offered on Windows only');
      if (platform !== 'windows') assert.match(await page.locator('#settings-search-results').innerText(), /没有匹配项/);
      await search.fill('SSH');
      assert.deepEqual(await page.locator('[data-settings-result] strong').allInnerTexts(), ['机器与采集']);
      await search.press('Enter');
      assert.equal(await page.evaluate(() => [state.page, state.settingsTab].join()), 'settings,hosts');
      assert.equal(await page.locator('#settings-search').inputValue(), '', 'opening a result clears the query');
      await page.locator('#settings-search').fill('sampling');
      await page.locator('[data-settings-result]').first().click();
      assert.equal(await page.evaluate(() => [state.settingsTab, state.generalTab].join()), 'general,advanced');
      assert.equal(await page.locator('[data-settings-section=sampling] .sampling-row').count(), 1);
      await page.locator('[data-sampling-stop]').click();
      await page.waitForFunction(() => !state.estimateBusy && !document.querySelector('[data-sampling-stop]'));
      assert.match(await page.locator('[data-settings-section=sampling]').innerText(), /当前没有进行中的采样/);
      await page.locator('[data-page=agent]').click();
      await page.evaluate(() => openQuotaOrder());
      assert.equal(await page.evaluate(() => [state.page, state.settingsTab, state.generalTab].join()), 'settings,general,panel');
      assert(await page.locator('#quota-order-save').isDisabled(), 'an unchanged order has nothing to save');
      assert.equal(await page.locator('#quota-dialog[open]').count(), 0);
      // Panel width is a native preference on the panel page.
      assert.deepEqual(await page.locator('[data-panel-width]').evaluateAll(buttons => buttons.map(b => [b.dataset.panelWidth, b.getAttribute('aria-pressed')])), [['standard', 'true'], ['compact', 'false']]);
      await page.locator('[data-panel-width=compact]').click();
      await page.waitForFunction(() => document.querySelector('[data-panel-width=compact]')?.getAttribute('aria-pressed') === 'true');
      assert.deepEqual(await page.evaluate(() => navigationFixture.calls.filter(call => call.command === 'desktop_panel_width').map(call => call.args)), [{width: 'compact'}]);
      await page.evaluate(() => {navigationFixture.failPanelWidth = true;});
      await page.locator('[data-panel-width=standard]').click();
      await page.waitForFunction(() => !document.querySelector('[data-panel-width]:disabled'));
      assert.equal(await page.locator('[data-panel-width=compact]').getAttribute('aria-pressed'), 'true', 'a failed save keeps the stored width');
      await page.evaluate(() => {navigationFixture.failPanelWidth = false;});
      await page.locator('[data-panel-width=standard]').click();
      await page.waitForFunction(() => document.querySelector('[data-panel-width=standard]')?.getAttribute('aria-pressed') === 'true');

      // Settings › 启动与系统: launch at login applies at once; links are navigation-only and listed.
      await page.evaluate(() => openSettings({category: 'general', section: 'system'}));
      await page.locator('#system-autostart:enabled').waitFor();
      assert.equal(await page.locator('#system-autostart').isChecked(), false);
      await page.locator('#system-autostart').click();
      await page.waitForFunction(() => document.querySelector('#system-autostart')?.checked && !document.querySelector('#system-autostart').disabled);
      assert.equal(await page.evaluate(() => navigationFixture.autostart), true);
      await page.evaluate(() => {navigationFixture.failAutostart = true;});
      await page.locator('#system-autostart').click();
      await page.locator('[data-settings-section=launch] .error').waitFor();
      assert.equal(await page.locator('#system-autostart').isChecked(), true, 'a failed change shows the stored state');
      assert.match(await page.locator('[data-settings-section=launch] .error').innerText(), /登录时启动设置未保存/);
      await page.evaluate(() => {navigationFixture.failAutostart = false;});
      assert.deepEqual(await page.locator('.system-link-row code').allInnerTexts(), ['aieyes://panel', 'aieyes://overview', 'aieyes://analysis', 'aieyes://servers', 'aieyes://sessions', 'aieyes://settings']);
      await search.fill('链接');
      assert.deepEqual(await page.locator('[data-settings-result] strong').allInnerTexts(), ['应用链接']);
      await search.press('Escape');

      // Settings › Webhook: receivers show the host only; every change saves at once.
      await page.evaluate(() => openSettings({category: 'general', section: 'webhooks'}));
      assert.match(await page.locator('[data-settings-section=targets]').innerText(), /尚未添加接收地址/);
      await page.locator('#webhooks-enabled').click();
      await page.waitForFunction(() => navigationFixture.settings.webhooks?.enabled === true);
      await page.locator('#webhook-add').click();
      await page.locator('#field-webhookUrl').fill('ftp://example.invalid');
      await page.locator('#editor-form button[type=submit]').click();
      await page.getByText('请输入以 http:// 或 https:// 开头的完整地址').waitFor();
      await page.locator('#field-webhookFormat').selectOption('bark');
      assert.match(await page.locator('#webhook-format-note').innerText(), /Bark/);
      await page.locator('#field-webhookUrl').fill('https://hooks.example.invalid/push/SECRET-TOKEN');
      await page.locator('#editor-form button[type=submit]').click();
      await page.waitForFunction(() => !document.getElementById('editor').open);
      const hookRow = page.locator('.webhook-row');
      await hookRow.waitFor();
      assert.equal(await hookRow.locator('strong').innerText(), 'hooks.example.invalid');
      assert.equal(await hookRow.locator('small').innerText(), 'Bark · hooks.example.invalid');
      assert(!/SECRET-TOKEN/.test(await page.locator('#content').innerText()), 'the list never shows the address path');
      assert.deepEqual(await page.evaluate(() => navigationFixture.settings.webhooks.targets.map(t => [t.format, t.url, t.enabled])), [['bark', 'https://hooks.example.invalid/push/SECRET-TOKEN', true]]);
      await hookRow.locator('[data-webhook-test]').click();
      await page.getByText('测试消息已送达').waitFor();
      await page.evaluate(() => {navigationFixture.failWebhook = true;});
      await page.locator('.webhook-row [data-webhook-test]').click();
      await page.locator('.webhook-row .error').waitFor();
      assert.match(await page.locator('.webhook-row .error').innerText(), /测试消息未送达/);
      await page.evaluate(() => {navigationFixture.failWebhook = false;});
      await page.locator('[data-webhook-event=sessionComplete]').click();
      await page.waitForFunction(() => navigationFixture.settings.webhooks.sessionComplete === false);
      await page.locator('.webhook-row [data-webhook-enable]').click();
      await page.waitForFunction(() => navigationFixture.settings.webhooks.targets[0].enabled === false);
      await page.locator('.webhook-row [data-webhook-remove]').click();
      await page.locator('#editor-form button[type=submit]').click();
      await page.waitForFunction(() => navigationFixture.settings.webhooks.targets.length === 0);
      assert.match(await page.locator('[data-settings-section=targets]').innerText(), /尚未添加接收地址/);

      // Servers: the CPU tile carries its last hour between title and value without growing the header.
      await page.evaluate(() => {
        const end = Math.floor(Date.now() / 60000) * 60;
        navigationFixture.hostHistory = [{id: 'host', minutes: Array.from({length: 60}, (_, i) => ({t: end - (59 - i) * 60, cpu: 10 + i % 7, cpuMax: 30, memory: null, gpu: null}))}];
      });
      await page.locator('[data-page=servers]').click();
      const tile = page.locator('.resource-block[data-resource=cpu]').first();
      await tile.locator('.host-trend').waitFor();
      assert.match(await tile.locator('.host-trend').getAttribute('aria-label'), /^60 分钟走势 · 均值 1\d% · 峰值 30%$/);
      assert(await tile.evaluate(el => {const head = el.querySelector('.between'),value = el.querySelector(':scope > strong');return head.getBoundingClientRect().height <= value.getBoundingClientRect().height + 0.5;}), 'the trend keeps the header height');
      assert(await tile.evaluate(el => {const a = el.querySelector('.between > span').getBoundingClientRect(),t = el.querySelector('.host-trend').getBoundingClientRect(),v = el.querySelector(':scope > strong').getBoundingClientRect();return a.right <= t.left && t.right <= v.left + 0.5;}), 'title, trend, value from left to right');
      assert.equal(await page.locator('.resource-block[data-resource=memory] .host-trend').count(), 0, 'a metric without readings draws nothing');

      // Settings › 通知: server alert thresholds and duration, saved with the page.
      await page.evaluate(() => openSettings({category: 'general', section: 'notifications'}));
      assert.deepEqual(await page.locator('#field-serverAlertDuration option').allInnerTexts(), ['1 分钟', '3 分钟', '5 分钟', '10 分钟', '15 分钟', '30 分钟']);
      assert.equal(await page.locator('#field-serverAlertCpu').inputValue(), '90');
      await page.locator('input[name=serverAlertsEnabled]').click();
      await page.locator('#field-serverAlertCpu').selectOption('80');
      await page.locator('#field-serverAlertDuration').selectOption('600');
      await page.waitForFunction(() => navigationFixture.settings.serverAlerts?.durationSeconds === 600, null, {timeout: 5000});
      assert.deepEqual(await page.evaluate(() => navigationFixture.settings.serverAlerts), {enabled: true, cpuPercent: 80, memoryPercent: 90, durationSeconds: 600});

      // Panel → main window account link: the overview scrolls to that card and outlines it once.
      await page.locator('[data-page=sessions]').click();
      await page.evaluate(() => changeFilters({provider: 'claude'}));
      await page.evaluate(() => navigationFixture.emit('desktop:detail-view', {page: 'agent', focusAccount: 'codex:a5'}));
      await page.waitForFunction(() => document.querySelector('#content [data-highlight]'));
      assert.equal(await page.evaluate(() => [state.page, state.provider].join()), 'agent,', 'a filter that hides the card is cleared');
      const focused = page.locator('#content [data-highlight]');
      assert.equal(await focused.count(), 1, 'one element carries the outline');
      assert.equal(await focused.evaluate(el => el.dataset.quotaAccount ?? el.dataset.quota), 'codex:a5');
      assert(await focused.evaluate(el => {const r = el.getBoundingClientRect(); return r.top >= 0 && r.bottom <= innerHeight;}), 'the card is scrolled into view');
      assert.equal(await focused.evaluate(el => getComputedStyle(el).animationName), 'none', 'reduced motion holds the outline still');
      await page.evaluate(() => render());
      assert.equal(await page.locator('#content [data-highlight]').count(), 1, 'a live re-render keeps the same outline');
      await page.waitForFunction(() => !document.querySelector('[data-highlight]'), null, {timeout: 4000});
      await page.evaluate(() => render());
      assert.equal(await page.locator('[data-highlight]').count(), 0, 'the outline plays once');
      await page.evaluate(() => navigationFixture.emit('desktop:detail-view', {page: 'agent', focusAccount: 'codex:missing'}));
      assert.equal(await page.locator('[data-highlight]').count(), 0, 'an unknown account opens the overview without an outline');

      // A slow query reports real progress and can be cancelled without applying its late result.
      await page.evaluate(async () => {
        await navigatePage('analysis');
        clearUsageSnapshots();
        const original = window.__TAURI__.core.invoke;
        window.queryCancelFixture = {original, cancelled: null};
        window.__TAURI__.core.invoke = async (command, args) => {
          if (command === 'engine_call' && args.method === 'dashboard') {
            window.queryCancelFixture.id = args.params.operationId;
            return new Promise((resolve, reject) => {window.queryCancelFixture.reject = reject;});
          }
          if (command === 'engine_call' && args.method === 'dashboard.cancel') {
            window.queryCancelFixture.cancelled = args.params.operationId;
            window.queryCancelFixture.reject(new Error('Query cancelled'));
            return {cancelled: 1};
          }
          return original(command, args);
        };
        void loadDashboard().catch(() => {});
      });
      await page.waitForFunction(() => window.queryCancelFixture?.id);
      await page.evaluate(() => navigationFixture.emit('operations:progress', {kind:'dashboard.progress', operationId:queryCancelFixture.id, done:25, total:100}));
      assert.match(await page.locator('#query-progress').innerText(), /25%/);
      await page.locator('#query-progress button').click();
      await page.waitForFunction(() => !dashboardWork);
      assert.equal(await page.locator('#query-progress').count(), 0);
      assert(await page.evaluate(() => queryCancelFixture.cancelled === queryCancelFixture.id && !state.dashboardPending));
      await page.evaluate(() => {window.__TAURI__.core.invoke = queryCancelFixture.original;});

      assert.deepEqual(errors, []);
      await context.close();
    }
    // The panel sends the account to the main window instead of navigating itself.
    {
      const context = await browser.newContext({viewport: {width: 450, height: 720}, reducedMotion: 'reduce'}), errors = [];
      const panel = await context.newPage(); panel.on('pageerror', error => errors.push(String(error)));
      await panel.addInitScript(fixture, {platform: 'windows'});
      await panel.goto(url + '/floating.html'); await panel.evaluate(() => AieyesApp.ready);
      // Like "查看分析", the entry sits in the expanded part of the card.
      await panel.locator('[data-show-account]').first().waitFor({state: 'attached'});
      if (!await panel.locator('[data-show-account]:visible').count()) await panel.locator('.quota-card:has([data-show-account]) .quota-disclosure > summary').first().click();
      const link = panel.locator('[data-show-account]:visible').first();
      await link.waitFor();
      const key = await link.getAttribute('data-show-account');
      assert.equal(await link.getAttribute('aria-label') ?? await link.getAttribute('title'), '在主窗口中查看');
      await link.click();
      const sent = await panel.evaluate(() => navigationFixture.calls.filter(call => call.command === 'desktop_detail').map(call => call.args.view));
      assert.deepEqual(sent, [{page: 'agent', focusAccount: key}]);
      // The compact panel (360) keeps every row inside the window.
      await panel.setViewportSize({width: 360, height: 720});
      await panel.waitForTimeout(100);
      // Content inside a horizontal scroller (agent tabs) is clipped by it and is not an overflow.
      const overflow = await panel.evaluate(() => {
        const clipped = el => {for (let parent = el.parentElement; parent; parent = parent.parentElement) if (getComputedStyle(parent).overflowX !== 'visible') return parent.getBoundingClientRect().right <= innerWidth + 0.5; return false;};
        return [...document.querySelectorAll('body *')].filter(el => el.getClientRects().length && el.getBoundingClientRect().right > innerWidth + 0.5 && getComputedStyle(el).position !== 'fixed' && !el.closest('[hidden],dialog:not([open])') && !clipped(el)).map(el => el.tagName + '.' + el.className).slice(0, 5);
      });
      assert.deepEqual(overflow, [], 'no element extends past the compact panel');
      assert(await panel.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'the compact panel does not scroll sideways');
      assert.deepEqual(errors, []);
      await context.close();
    }
    // Motion: ring arcs sweep, refreshed tiles brighten once, numbers roll, the refresh mark settles;
    // with reduced motion the same changes land at once with no animation at all.
    for (const reducedMotion of ['no-preference', 'reduce']) {
      const {context, page, errors} = await open('windows', {reducedMotion});
      await page.evaluate(() => {localStorage.setItem(AieyesQuotaRings.keys.detail, 'tile');render();});
      await page.locator('#content .quota-tile path.ring-fill').first().waitFor();
      const animating = () => page.evaluate(() => [...new Set(document.getAnimations().map(a => a.effect?.target?.dataset?.winAnimating).filter(Boolean))].sort());
      if (reducedMotion === 'no-preference') {
        await page.evaluate(() => navigatePage('analysis'));
        await page.evaluate(() => navigatePage('agent'));
        assert((await animating()).includes('ring'), 'opening the overview draws the rings');
        await page.evaluate(() => document.getAnimations().forEach(a => a.finish()));
      }
      await page.evaluate(async () => {navigationFixture.quotas[0].windows[0].usedPercent = 60;await loadDashboard();});
      const kinds = await animating();
      const number = await page.evaluate(async () => {
        const el = document.createElement('div');el.className = 'stat';el.innerHTML = '<div class="stat-label">x</div><div class="stat-value">1,200</div>';document.querySelector('#content').append(el);
        window.AieyesMotion.captureNavigation();el.querySelector('.stat-value').textContent = '1,800';
        const clone = el.cloneNode(true);el.replaceWith(clone);window.AieyesMotion.didRender('agent');
        await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
        const midway = clone.querySelector('.stat-value').textContent;
        await new Promise(r => setTimeout(r, 900));
        return [midway, clone.querySelector('.stat-value').textContent];
      });
      assert.equal(number[1], '1,800', 'a rolling number ends on the rendered text');
      const refresh = page.locator('#content [data-refresh-status]').first();
      await page.evaluate(() => {const key = document.querySelector('#content [data-refresh-status]').dataset.refreshStatus;state.refreshState[key] = {busy: true};updateHeadingIndicators();state.refreshState[key] = {success: Date.now()};updateHeadingIndicators();});
      const settled = await refresh.evaluate(el => el.querySelector('svg')?.dataset.winAnimating ?? '');
      if (reducedMotion === 'no-preference') {
        assert.deepEqual(kinds.filter(kind => ['ring', 'refresh'].includes(kind)), ['refresh', 'ring'], 'the changed ring sweeps and its tile brightens');
        assert.match(number[0], /^1,[2-8]\d\d$/, 'the number counts through intermediate values');
        assert.equal(settled, 'settle', 'a finished refresh settles its mark once');
      } else {
        assert.deepEqual(kinds, [], 'reduced motion runs no animation');
        assert.equal(number[0], '1,800', 'reduced motion shows the new number at once');
        assert.equal(settled, '');
        assert.equal(await page.evaluate(() => document.getAnimations().length), 0);
      }
      assert.deepEqual(errors, []);
      await context.close();
    }
    // 本次更新: the bundled notes parse like macOS ReleaseNotes; the banner shows once after an update.
    {
      const vm = require('node:vm');
      const sandbox = {AieyesUI: {escapeHTML: (v) => String(v)}, AieyesI18n: {t: (v) => v}, addEventListener() {}, window: {}};
      sandbox.globalThis = sandbox;sandbox.window = sandbox;vm.createContext(sandbox);
      vm.runInContext(fs.readFileSync(path.join(root, 'whats-new.js'), 'utf8').replace(/\n  \/\/ After every script[^\n]*\n[^\n]*\n/, '\n'), sandbox);
      const version = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../apps/desktop/src-tauri/tauri.conf.json'), 'utf8')).version;
      const markdown = fs.readFileSync(path.resolve(__dirname, `../docs/release-notes/v${version}.md`), 'utf8');
      const notes = sandbox.AieyesWhatsNew.parse(markdown);
      assert(notes.english.items.length > 0 && notes.chinese.items.length > 0);
      assert.match(notes.changelog, /^https:\/\/github\.com\//);
      assert.equal(sandbox.AieyesWhatsNew.part(notes, true), notes.chinese);
      assert.equal(sandbox.AieyesWhatsNew.parse('# Only a title\nSome text'), null, 'notes without items show nothing');
      const english = sandbox.AieyesWhatsNew.parse('# T\n- a\n## 中文\n说明');
      assert.equal(sandbox.AieyesWhatsNew.part(english, true), english.english, 'a part without items falls back to the other');
      const context = await browser.newContext({viewport: {width: 1120, height: 800}, reducedMotion: 'reduce'}),page = await context.newPage(),errors = [];
      page.on('pageerror', error => errors.push(String(error)));
      await page.addInitScript(fixture, {platform: 'windows'});
      await page.addInitScript(({version, markdown, changelog}) => {navigationFixture.whatsNew = {version, previousVersion: '0.2.4', updated: true, notes: markdown, changelog};}, {version, markdown, changelog: notes.changelog});
      await page.goto(url);await page.evaluate(() => AieyesApp.ready);
      const banner = page.locator('#whats-new .whats-new-banner');
      await banner.waitFor();
      assert.match(await banner.innerText(), new RegExp(`已更新到 v${version.replace(/\./g, '\\.')}`));
      await page.evaluate(() => render());
      assert.equal(await page.locator('#whats-new').count(), 1, 'a re-render keeps one banner');
      await page.locator('[data-page=sessions]').click();
      assert(await page.locator('#whats-new').isHidden(), 'the banner belongs to the overview');
      await page.locator('[data-page=agent]').click();
      await page.locator('[data-whats-new-open]').click();
      await page.locator('[data-settings-section=release-notes]').waitFor();
      assert.equal(await page.evaluate(() => [state.page, state.generalTab].join()), 'settings,updates');
      assert.equal(await page.locator('.release-notes-items li').count(), notes.chinese.items.length);
      const headline = sandbox.AieyesWhatsNew.part(notes, true).headline;
      assert.equal(await page.locator('[data-settings-section=release-notes] .muted.tiny').first().innerText(), `Aieyes v${version}${headline ? ' · ' + headline : ''}`);
      await page.locator('#release-notes-changelog').click();
      assert(await page.evaluate(() => navigationFixture.calls.some(call => call.command === 'whats_new_open_changelog' && !call.args)), 'the page names no address');
      await page.locator('[data-page=agent]').click();
      assert.equal(await page.locator('#whats-new:not([hidden])').count(), 0, 'opening the notes dismisses the banner');
      await page.reload();await page.evaluate(() => AieyesApp.ready);await page.waitForTimeout(200);
      assert.equal(await page.locator('#whats-new:not([hidden])').count(), 0, 'once per version');
      assert.deepEqual(errors, []);
      await context.close();
    }
    // Perceived performance: the page outline before the first result, then the last result of this
    // window type while a relaunch's first query runs.
    {
      const context = await browser.newContext({viewport: {width: 1120, height: 800}, reducedMotion: 'reduce'}),page = await context.newPage(),errors = [];
      page.on('pageerror', error => errors.push(String(error)));
      await page.addInitScript(fixture, {platform: 'windows'});
      await page.addInitScript(() => {navigationFixture.holdDashboard = true;});
      await page.goto(url);
      await page.locator('.overview-skeleton').waitFor();
      assert.equal(await page.locator('.overview-skeleton .quota-placeholder').count(), 3);
      await page.evaluate(() => navigationFixture.releaseDashboard());
      await page.evaluate(() => AieyesApp.ready);
      await page.locator('#content .quota-card').first().waitFor();
      await page.reload();
      await page.locator('#content .quota-card').first().waitFor();
      assert.equal(await page.evaluate(() => state.dashboardPending), true, 'the remembered result shows while the query runs');
      assert.equal(await page.locator('.overview-skeleton').count(), 0);
      await page.evaluate(() => navigationFixture.releaseDashboard());
      await page.evaluate(() => AieyesApp.ready);
      await page.waitForFunction(() => !state.dashboardPending);
      assert.deepEqual(errors, []);
      await context.close();
    }
    // Sample the live Web Animation between two readings, rather than just its final frame.
    {
      const {context, page, errors} = await open('windows', {reducedMotion:'no-preference'});
      const result = await page.evaluate(async () => {
        AieyesMotion.cancelAll();
        const content = document.querySelector('#content');
        const reading = (value) => `<div data-quota="probe"><svg width="120" height="40"><path class="ring-fill" d="M 0 20 L ${value} 20" stroke="red"/></svg></div>`;
        content.innerHTML = reading(30);
        AieyesMotion.didRender('agent', content);AieyesMotion.cancelAll();
        AieyesMotion.captureNavigation();content.innerHTML = reading(90);AieyesMotion.didRender('agent', content);
        const path = content.querySelector('path');
        const animation = path.getAnimations()[0];
        if (!animation) return null;
        animation.pause();animation.currentTime = Number(animation.effect.getTiming().duration) / 2;
        await new Promise(requestAnimationFrame);
        const dash = parseFloat(getComputedStyle(path).strokeDasharray);
        return {dash, frames:animation.effect.getKeyframes().length};
      });
      assert(result && result.frames >= 2 && result.dash > 30 && result.dash < 90, 'the arc passes through an intermediate length');
      assert.deepEqual(errors, []);await context.close();
    }
    console.log('Experience UI: sessions page and settings governance passed');
  } finally {
    await browser.close(); server.close();
  }
})().catch(error => {console.error(error); process.exit(1);});
