/* Production catalogs and populated IPC fixtures. No provider calls or personal data. */
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http'), vm = require('node:vm');
const {chromium} = require('../apps/desktop/node_modules/playwright');
const {fixture} = require('./workspace-ui-fixture.cjs');
const {serveFont} = require('./ui-test-helpers.cjs');
const root = path.resolve(__dirname, '..'), web = path.join(root, 'apps/desktop/web');
const catalog = JSON.parse(fs.readFileSync(path.join(root, 'locales/en.json'), 'utf8'));

const sandbox = vm.createContext({localStorage: {getItem: () => null, setItem() {}}});
for (const name of ['i18n-catalog.js', 'i18n.js']) vm.runInContext(fs.readFileSync(path.join(web, name), 'utf8'), sandbox);
const locale = sandbox.AieyesI18n;
locale.apply('en');
assert.equal(locale.t('剩余 {0}', '用户 {1}'), '用户 {1} remaining');
assert.equal(locale.message('3 个活跃会话'), '3 active sessions');
assert.equal(locale.message('/Users/用户/设置.json'), '/Users/用户/设置.json');
assert.equal(locale.t('token'), 'token');
assert.equal(locale.t('{0} 个账户', 1), '1 account');
assert.equal(locale.t('{0} 个账户', 2), '2 accounts');
assert.equal(locale.t('{0}', '设置'), '设置');
locale.apply('zh-CN');
assert.equal(locale.message('用户 {1} remaining'), '剩余 用户 {1}');
assert.equal(locale.message('3 active sessions'), '3 个活跃会话');
assert.equal(locale.t('剩余 {0}', '42%'), '剩余 42%');
assert.equal(locale.message('1 account'), '1 个账户');

function populate() {
  const nav = window.navigationFixture;
  for (const account of nav.settings.accounts) account.name = account.id === 'a0' ? '中文账户 token' : account.provider + ' ' + account.id;
  for (const source of nav.settings.sources) source.name = 'Source ' + source.id;
  nav.settings.sources[0].path = '/资料/代码';
  nav.settings.hosts[0].name = 'Lab';
  nav.settings.codexApis[0].name = 'API fixture';
  for (const quota of nav.quotas) {
    quota.name = nav.settings.accounts.find(a => a.id === quota.accountId).name;
    quota.windows[0] && (quota.windows[0].name = '默认');
  }
  nav.quotas[0].metadataError = '读取失败';
  for (const record of nav.records) {record.sourceNames = ['Source s-a0'];record.calculationNote = '读取失败';}
  nav.settings.appearance.language = localStorage.getItem('aieyes.language') || 'en';
  localStorage.setItem('aieyes.language', nav.settings.appearance.language);
  localStorage.setItem('aieyes.appearance', JSON.stringify(nav.settings.appearance));
  localStorage.setItem('quota.expanded.v2.detail.codex:a0', 'true');
  localStorage.setItem('quota.expanded.v2.panel.codex:a0', 'true');
  // Each fixture window reads the same saved preference, as real windows do through IPC.
  window.addEventListener('storage', event => {
    if (event.key === 'aieyes.appearance') nav.settings.appearance = JSON.parse(event.newValue);
  });
  const invoke = window.__TAURI__.core.invoke;
  window.__TAURI__.core.invoke = async (command, args) => {
    if (command === 'updates_info' || command === 'updates_check') return {phase: 'current', message: '已是最新版本 v0.1.1', currentVersion: '0.1.1', latestVersion: '0.1.1', automatic: true};
    if (command === 'engine_call' && args.method === 'pets.list') return {pets: [], selectedId: null};
    return invoke(command, args);
  };
}

(async () => {
  const server = http.createServer((req, res) => {
    if (serveFont(req, res)) return;
    const file = path.join(web, path.basename(new URL(req.url, 'http://local').pathname) || 'index.html');
    if (!fs.existsSync(file)) {res.writeHead(404).end();return;}
    res.setHeader('content-type', ({'.js': 'text/javascript', '.css': 'text/css', '.html': 'text/html'})[path.extname(file)] || 'application/octet-stream');
    res.end(fs.readFileSync(file));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch(), origin = `http://127.0.0.1:${server.address().port}`;
  const evidence = path.join(root, '.local/i18n');fs.mkdirSync(evidence, {recursive: true});
  const reports = [];
  try {
    for (const platform of ['windows', 'linux']) {
      const context = await browser.newContext({viewport: {width: 1120, height: 800}, reducedMotion: 'reduce'});
      await context.addInitScript(fixture, {platform});await context.addInitScript(populate);
      const page = await context.newPage(), errors = [];page.setDefaultTimeout(12000);
      page.on('pageerror', e => errors.push(String(e)));
      await page.goto(origin);await page.evaluate(() => AieyesApp.ready);
      async function audit(name, scope = 'body') {
        const result = await page.evaluate(scope => {
          const root = document.querySelector(scope), walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT), texts = [];
          let node;
          while ((node = walker.nextNode())) {
            const element = node.parentElement;
            if (!element || !element.checkVisibility({checkVisibilityCSS: true}) || element.closest('script,style,input,textarea,[translate=no]')) continue;
            const text = node.nodeValue.replaceAll('中文账户 token', '').replaceAll('/资料/代码', '').trim();
            if (/[\u3400-\u9fff]/.test(text)) texts.push(text);
          }
          return {chinese: [...new Set(texts)], missing: AieyesI18n.missing, overflow: document.documentElement.scrollWidth > innerWidth + 1};
        }, scope);
        reports.push({platform, name, ...result});
        assert.deepEqual(result.chinese, [], `${platform} ${name}: untranslated visible text`);
        assert.deepEqual(result.missing, [], `${platform} ${name}: missing catalog keys`);
        assert.equal(result.overflow, false, `${platform} ${name}: horizontal page overflow`);
        assert.deepEqual(errors, []);
      }
      await audit('overview');
      assert.match(await page.locator('.quota-name').first().innerText(), /中文账户 token/);
      assert.match(await page.locator('.quota-identity-note').first().innerText(), /Read failed/);
      const width = await page.locator('.quota-card').first().evaluate(el => el.getBoundingClientRect().width);
      await page.locator('[data-language-toggle]').click();await page.waitForFunction(() => AieyesI18n.language === 'zh-CN');
      assert.match(await page.locator('#title').innerText(), /总览/);
      assert.match(await page.locator('.quota-identity-note').first().innerText(), /读取失败/);
      await page.locator('[data-language-toggle]').click();await page.waitForFunction(() => AieyesI18n.language === 'en');
      assert(Math.abs(width - await page.locator('.quota-card').first().evaluate(el => el.getBoundingClientRect().width)) < 2);
      await audit('restored overview');
      for (const name of ['analysis', 'servers']) {await page.evaluate(name => AieyesApp.navigatePage(name), name);await audit(name);}
      for (const category of ['integrations', 'hosts', 'prices', 'wakeups']) {
        await page.evaluate(category => AieyesApp.openSettings({category}), category);await audit(category);
      }
      for (const section of ['appearance', 'desktop', 'network', 'refresh', 'notifications', 'updates']) {
        await page.evaluate(section => AieyesApp.openSettings({category: 'general', section}), section);await audit(section);
      }
      const providers = await page.evaluate(() => Object.keys(AieyesContract.providers).filter(p => p !== 'all'));
      for (const provider of providers) {
        await page.evaluate(provider => AieyesApp.openSettings({category: 'integrations', provider}), provider);
        await page.locator('#content details').evaluateAll(rows => rows.forEach(row => row.open = true));
        await audit('provider ' + provider);
      }
      await page.evaluate(() => AieyesApp.openSettings({category: 'hosts'}));
      await page.evaluate(() => editItem(state.settings.hosts[0]));
      await page.locator('#editor[open]').waitFor();
      await page.locator('#field-name').fill('编辑中的名称');
      await page.locator('#field-identityFile').fill('/资料/密钥 {0}');
      const selected = await page.locator('#field-authMode').inputValue();
      await page.evaluate(() => setLanguage('zh-CN'));
      assert.equal(await page.locator('label[for=field-name]').innerText(), '名称');
      await page.evaluate(() => setLanguage('en'));
      assert.equal(await page.locator('#field-name').inputValue(), '编辑中的名称');
      assert.equal(await page.locator('#field-identityFile').inputValue(), '/资料/密钥 {0}');
      assert.equal(await page.locator('#field-authMode').inputValue(), selected);
      await audit('open host editor', '#editor');
      await page.evaluate(() => document.querySelector('#editor').close());
      await page.evaluate(() => AieyesAgentSettings.advanced(state.settings.sources[0], 'codex'));
      await page.evaluate(() => setLanguage('zh-CN'));await page.evaluate(() => setLanguage('en'));
      assert.equal(await page.locator('#field-path').inputValue(), '/资料/代码');
      await audit('open directory editor', '#editor');
      await page.evaluate(() => document.querySelector('#editor').close());
      // A user label identical to a catalog key must remain user text in a modal.
      await page.evaluate(() => {state.settings.accounts[0].name = '设置';openPanelAccounts();});
      await page.evaluate(() => setLanguage('zh-CN'));await page.evaluate(() => setLanguage('en'));
      assert.equal(await page.locator('[data-panel-account="codex:a0"]').locator('..').innerText(), '设置');
      await page.locator('#quota-close').click();
      const panel = await context.newPage();await panel.setViewportSize({width: 360, height: 720});
      await panel.goto(origin + '/floating.html');await panel.evaluate(() => AieyesApp.ready);
      await page.evaluate(() => AieyesApp.openSettings({category: 'general', section: 'network'}));
      await page.locator('#field-proxyTestUrls').fill('https://draft.invalid');
      await panel.evaluate(() => setLanguage('zh-CN'));await page.waitForFunction(() => AieyesI18n.language === 'zh-CN');
      assert.equal(await page.locator('#field-proxyTestUrls').inputValue(), 'https://draft.invalid');
      await panel.evaluate(() => setLanguage('en'));await page.waitForFunction(() => AieyesI18n.language === 'en');
      assert.equal(await page.locator('#field-proxyTestUrls').inputValue(), 'https://draft.invalid');
      assert.deepEqual(await page.evaluate(() => navigationFixture.settings.proxyTestUrls), ['https://fixture.invalid']);
      assert.equal(await panel.evaluate(() => document.documentElement.scrollWidth > innerWidth + 1), false);
      await panel.screenshot({path: path.join(evidence, `${platform}-panel-en.png`)});
      // Keep the desktop layout usable at its narrow supported width.
      await page.setViewportSize({width: 760, height: 800});await audit('narrow network settings');
      await page.screenshot({path: path.join(evidence, `${platform}-settings-en.png`)});
      assert.deepEqual(errors, []);await context.close();
    }
    console.log(`Bilingual UI: ${Object.keys(catalog).length} messages, populated accounts/all providers, dialogs, shared windows, retained drafts, user names/paths and narrow layouts passed`);
  } finally {
    fs.writeFileSync(path.join(evidence, 'browser-audit.json'), JSON.stringify(reports, null, 2));
    await browser.close();await new Promise(resolve => server.close(resolve));
  }
})().catch(error => {console.error(error);process.exitCode = 1;});
