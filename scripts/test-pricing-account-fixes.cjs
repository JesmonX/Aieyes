const assert = require('node:assert/strict'), fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const {chromium} = require('../apps/desktop/node_modules/playwright');
const {fixture} = require('./workspace-ui-fixture.cjs');

(async () => {
  const root = path.resolve(__dirname, '../apps/desktop/web');
  const server = http.createServer((req, res) => {
    if (require('./ui-test-helpers.cjs').serveFont(req, res)) return;
    const file = path.join(root, path.basename(new URL(req.url, 'http://fixture').pathname) || 'index.html');
    try {res.setHeader('content-type', ({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)] || 'application/octet-stream');res.end(fs.readFileSync(file));} catch {res.writeHead(404).end();}
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({headless:true});
  try {
    for (const surface of ['index.html', 'floating.html']) {
      const page = await browser.newPage({viewport:{width:surface === 'floating.html' ? 450 : 1120,height:850}}), errors = [];
      page.on('pageerror', error => errors.push(String(error)));
      await page.addInitScript(fixture, {platform:'windows'});
      await page.addInitScript(() => {
        const f = navigationFixture;
        for (const provider of ['qoder','qoder-cn']) {
          f.settings.accounts.push({provider,id:provider,name:provider,quotaEnabled:true});
          f.quotas.push({provider,accountId:provider,sourceId:'fixture',name:provider,origin:'live',updatedAt:Date.now()/1000,plan:'personal_professional',identity:{key:provider+':hash',providerAccountId:'hidden-account-id',email:''},windows:[],meters:[{id:'plan',label:'套餐 Credits',unit:'credits',total:2000,used:0,remaining:2000},{id:'addon',label:'加购 Credits',unit:'credits',total:100,used:3,remaining:97}]});
        }
        const prices = [{id:'qwen/qwen3.8-flash',name:'Qwen: Qwen3.8 Flash',input:1e-6,output:2e-6,cacheRead:1e-7},{id:'other/partial',name:'Partial',input:1e-6},{id:'other/model',name:'Other'}];
        f.settings.modelMappings = {resolved:'qwen/qwen3.8-flash',partial:'other/partial'};
        const gaps = ['resolved','partial','unknown'].map(model => ({model,tokens:{input:100,output:0,cacheRead:50,cacheWrite:0},unpricedTokens:150}));
        const invoke = __TAURI__.core.invoke;
        __TAURI__.core.invoke = async (command, args) => {
          if (command === 'engine_call' && args.method === 'prices.list') return structuredClone(prices);
          const result = await invoke(command,args);
          if (command === 'engine_call' && args.method === 'dashboard') result.pricingGaps = structuredClone(gaps); // Deliberately stale backend snapshot.
          return result;
        };
      });
      await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);
      await page.waitForFunction(() => state.dashboard && !state.busy);
      for (const provider of ['qoder','qoder-cn']) {
        const card = page.locator(`[data-quota="${provider}:${provider}"]`);
        assert.equal(await card.locator('.quota-email').count(),0);
        assert.equal(await card.locator('.subscription-badge').innerText(),'Pro');
        assert.equal(await card.locator('[data-meter="plan"] strong').innerText(),'2000 / 2000 Credits');
        assert.equal(await card.locator('[data-meter="addon"] strong').innerText(),'97 / 100 Credits');
        assert(!((await card.innerText()).includes('hidden-account-id')));
      }
      if (surface === 'index.html') {
        await page.evaluate(async () => {state.page='settings';state.settingsTab='prices';state.prices=await api('prices.list');renderSettings();});
        for (const query of ['qwen flash','flash QWEN','qwen3.8 flash','QWEN_3.8-FLASH','qwen/flash']) {
          await page.locator('#price-search').fill(query);
          assert.equal(await page.locator('[data-price-row]').count(),1,query);
          assert.equal(await page.locator('[data-price-row]').getAttribute('data-price-row'),'qwen/qwen3.8-flash');
        }
        assert.equal(await page.locator('.pricing-gaps .list-row').count(),2,'Complete existing mappings disappear; incomplete cache prices remain');
        await page.locator('[data-gap-map="1"]').click();
        assert.equal(await page.locator('#field-mappingModel').inputValue(),'partial','Filtered gap buttons retain the correct model');
        assert.equal(await page.locator('#field-mappingId').inputValue(),'other/partial');
        await page.locator('#field-mappingId').fill('qwen/qwen3.8-flash');
        // Editing an existing mapping uses its dedicated editor; add a fresh alias instead.
        await page.locator('#field-mappingModel').fill('unknown');
        await page.locator('#mapping-form > button').click();
        await page.waitForFunction(() => !state.busy && state.settings.modelMappings.unknown);
        assert.equal(await page.locator('.pricing-gaps .list-row').count(),1,'New mappings resolve immediately even with stale usage data');
        await page.evaluate(async () => {await loadDashboard();});
        assert.equal(await page.locator('.pricing-gaps .list-row').count(),1,'Background refresh retains the resolved gap state');
      }
      assert.deepEqual(errors,[]);
      await page.close();
    }
    console.log('Qoder remaining credits/identity/plans, keyword price search and immediate mapping-gap updates passed on main and panel surfaces.');
  } finally {await browser.close();await new Promise(resolve => server.close(resolve));}
})().catch(error => {console.error(error);process.exitCode=1;});
