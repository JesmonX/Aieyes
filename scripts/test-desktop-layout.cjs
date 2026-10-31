// Production geometry and interaction checks for the desktop UI repair.
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const { chromium } = require('../apps/desktop/node_modules/playwright');
const { fixture } = require('./workspace-ui-fixture.cjs');
(async () => {
  const root = path.resolve(__dirname, '../apps/desktop/web');
  const out = path.resolve(__dirname, '../.local/ui-desktop-layout');fs.mkdirSync(out, {recursive:true});
  const server = http.createServer((req, res) => {
    if (require('./ui-test-helpers.cjs').serveFont(req, res)) return;
    const file = path.join(root, path.basename(new URL(req.url, 'http://test').pathname) || 'index.html');
    try {res.setHeader('content-type', ({'.html':'text/html','.css':'text/css','.js':'text/javascript','.png':'image/png'})[path.extname(file)] || 'text/plain');res.end(fs.readFileSync(file));} catch {res.writeHead(404).end();}
  });
  await new Promise(r => server.listen(0, '127.0.0.1', r));let browser;
  try {
    browser = await chromium.launch();
    for (const platform of ['windows', 'linux']) {
      const page = await browser.newPage({viewport:{width:1000,height:850}, reducedMotion:'reduce'}), errors=[];
      page.on('pageerror', e => errors.push(String(e)));
      await page.addInitScript(fixture, {platform});
      await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(() => AieyesApp.ready);
      await page.evaluate(() => {
        const usage=state.dashboard.summary;
        const days=Array.from({length:365},(_,i)=>({...usage,key:new Date(Date.UTC(2025,10,1+i)).toISOString().slice(0,10)}));
        state.dashboard.heatmap=days;state.dashboard.trendDays=days.slice(-7);state.dashboard.dayModels=days.slice(-7).map(d=>({day:d.key,model:'fixture',usage:d}));state.page='analysis';render();document.querySelector('.daily').open=true;
      });
      for (const width of [760,1000,1440]) for (const theme of ['light','dark']) {
        await page.setViewportSize({width,height:850});await page.evaluate(t=>document.documentElement.dataset.theme=t,theme);
        const table=page.locator('#daily-usage');
        await table.locator('[data-day-toggle]').first().click();
        const layout=await table.evaluate(el=>{
          const cells=sel=>[...el.querySelector(sel).children].map(c=>{const r=c.getBoundingClientRect();return [r.left,r.right]});
          return {header:cells('thead tr'),day:cells('tr.day'),model:cells('tr.day-model:not([hidden])'),buttonBorder:getComputedStyle(el.querySelector('[data-day-toggle]')).borderTopWidth,weight:getComputedStyle(el.querySelector('thead th')).fontWeight};
        });
        for(const row of [layout.day,layout.model])row.forEach((bounds,i)=>bounds.forEach((v,j)=>assert(Math.abs(v-layout.header[i][j])<1,'Daily header and cells share columns')));
        assert.equal(layout.buttonBorder,'0px');assert(Number(layout.weight)<=500);
        await table.locator('[data-day-toggle]').first().click();
        await page.mouse.move(0,0);await table.screenshot({path:path.join(out,`${platform}-daily-${width}-${theme}.png`)});
        await page.locator('.heat-table').evaluate(el=>el.open=true);
        const heat=page.locator('.heat-scroll');
        const geometry=await heat.evaluate(el=>{
          const months=[...el.querySelectorAll('.heat-months span')],cell=el.querySelector('[data-heat-day]'),r=cell.getBoundingClientRect();
          const gap=parseFloat(getComputedStyle(el.querySelector('.heatmap')).columnGap);
          return {square:Math.abs(r.width-r.height),diffs:months.map(m=>{const col=Number(m.style.gridColumnStart)-1;return Math.abs(m.getBoundingClientRect().left-(el.querySelector('.heatmap').getBoundingClientRect().left+col*(r.width+gap)));}),overflow:el.scrollWidth-el.clientWidth};
        });
        assert(geometry.square<1);assert(geometry.diffs.every(d=>d<1),JSON.stringify(geometry));assert(geometry.overflow<=1);
        await heat.locator('..').screenshot({path:path.join(out,`${platform}-heat-${width}-${theme}.png`)});
      }
      // A short range must not spread a handful of squares across the entire card.
      await page.evaluate(()=>{document.querySelector('.heat-scroll').outerHTML=heatmapHTML(state.dashboard.heatmap.slice(0,14));});
      assert(await page.locator('.heatmap').first().evaluate(el=>el.getBoundingClientRect().width<100));
      await page.evaluate(()=>{
        const days=Array.from({length:365},(_,i)=>({...state.dashboard.summary,key:new Date(Date.UTC(2025,9,30+i)).toISOString().slice(0,10)}));
        document.querySelector('.heat-scroll').outerHTML=heatmapHTML(days);
      });
      const monthBounds=await page.locator('.heat-months span').evaluateAll(items=>items.map(el=>{const range=document.createRange();range.selectNodeContents(el);const r=range.getBoundingClientRect();return [r.left,r.right]}));
      for(let i=1;i<monthBounds.length;i++)assert(monthBounds[i][0]>=monthBounds[i-1][1], 'Month labels must not overlap at a month boundary');
      // GPU compute saturation is normal; capacity warnings retain their meaning.
      assert.equal(await page.evaluate(()=>{const el=document.createElement('div');el.innerHTML=serverSummary({metrics:['gpu'],details:['gpuMemory'],devices:[]},{gpu:[{id:'0',utilization:100,memoryUsedMiB:100,memoryTotalMiB:100}]});return [...el.querySelectorAll('.server-resource-strip')].map(s=>s.querySelectorAll('.high-load-mark').length);}).then(v=>v.join(',')),'0,1');
      await page.locator('[data-settings-tab=integrations]').click();
      await page.locator('#agent-section-machines').evaluate(el=>el.open=true);
      const menu=page.locator('.machine-menu').first();await menu.locator('summary').click();
      for(const intent of ['login','switch'])assert.deepEqual(await menu.locator(`[data-intent=${intent}]`).allTextContents(),[intent==='login'?'登录':'切换账户']);
      assert.equal(await menu.locator('button').first().evaluate(el=>getComputedStyle(el).borderTopWidth),'0px');
      await page.screenshot({path:path.join(out,`${platform}-machine-menu.png`)});
      await menu.locator('summary').focus();await page.keyboard.press('ArrowDown');assert.equal(await menu.locator('button').first().evaluate(el=>el===document.activeElement),true);
      await page.keyboard.press('Escape');assert.equal(await menu.getAttribute('open'),null);
      await page.evaluate(()=>{state.settings.accounts[0].name='Long-account-name-'.repeat(12);openPanelAccounts();});
      for(const width of [450,760]) {
        await page.setViewportSize({width,height:720});
        const overflow=await page.locator('.panel-account-group').first().evaluate(el=>el.scrollWidth-el.clientWidth);assert(overflow<=1,`Account picker overflow: ${overflow} `+JSON.stringify(await page.locator('.panel-account-group').first().evaluate(el=>{const b=el.getBoundingClientRect();return {width:b.width,client:el.clientWidth,scroll:el.scrollWidth,items:[...el.querySelectorAll('*')].map(c=>({tag:c.tagName,cls:c.className,w:c.getBoundingClientRect().width,right:c.getBoundingClientRect().right-b.right})).filter(c=>c.right>0)}})));
        await page.locator('[data-panel-account]').last().scrollIntoViewIfNeeded();assert(await page.locator('[data-panel-account]').last().isVisible());await page.locator('.quota-dialog-body').evaluate(el=>el.scrollTop=0);
        await page.locator('#quota-dialog').screenshot({path:path.join(out,`${platform}-panel-accounts-${width}.png`)});
      }
      await page.locator('#panel-accounts-cancel').click();assert.deepEqual(errors,[]);await page.close();
    }
    console.log('Desktop tables, heatmap alignment, menus, account picker and GPU warning semantics passed');
  } finally {await browser?.close();server.close();}
})().catch(e=>{console.error(e);process.exit(1);});
