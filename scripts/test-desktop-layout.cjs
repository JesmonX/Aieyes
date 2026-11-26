// Production geometry and interaction checks for the desktop UI repair.
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const { chromium } = require('../apps/desktop/node_modules/playwright');
const { fixture } = require('./workspace-ui-fixture.cjs');
const { dimensions } = require('../docs/ui-design-tokens.json');
const fontSizes = Object.entries(dimensions).filter(([name]) => name.startsWith('font-'));

function setTextScale(page, scale) {
  return page.evaluate(({ fontSizes, scale }) => {
    const root = document.documentElement;
    for (const [name, size] of fontSizes) {
      if (scale === 1) root.style.removeProperty('--' + name);
      else root.style.setProperty('--' + name, (size * scale) + 'px');
    }
  }, { fontSizes, scale });
}

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
      // Panel accounts are a settings page; a long account name wraps inside its group.
      await page.evaluate(async()=>{state.settings.accounts[0].name='Long-account-name-'.repeat(12);await openPanelAccounts();});
      for(const width of [450,760]) {
        await page.setViewportSize({width,height:720});
        const overflow=await page.locator('.panel-account-group').first().evaluate(el=>el.scrollWidth-el.clientWidth);assert(overflow<=1,`Account picker overflow: ${overflow} `+JSON.stringify(await page.locator('.panel-account-group').first().evaluate(el=>{const b=el.getBoundingClientRect();return {width:b.width,client:el.clientWidth,scroll:el.scrollWidth,items:[...el.querySelectorAll('*')].map(c=>({tag:c.tagName,cls:c.className,w:c.getBoundingClientRect().width,right:c.getBoundingClientRect().right-b.right})).filter(c=>c.right>0)}})));
        await page.locator('[data-panel-account]').last().scrollIntoViewIfNeeded();assert(await page.locator('[data-panel-account]').last().isVisible());
        await page.locator('[data-settings-section=accounts]').screenshot({path:path.join(out,`${platform}-panel-accounts-${width}.png`)});
      }
      assert.equal(await page.locator('#quota-dialog[open]').count(),0);
      // Narrow window + large text scaling: #read-quotas stays single-line, data-show-labels never
      // forces wrapping, machine rows keep single-line actions without overlap, and sidebar exposes
      // vertical scroll fade when clipped.
      await page.setViewportSize({width:640,height:440});
      await setTextScale(page, 1.35);
      for (const language of ['zh-CN', 'en']) {
        await page.evaluate(async lang => { await setLanguage(lang); await navigatePage('agent'); }, language);
        await page.waitForFunction(() => Boolean(document.querySelector('#read-quotas')));
        await page.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
        const quotaHeader = await page.evaluate(() => {
          const btn = document.querySelector('#read-quotas');
          const heading = document.querySelector('.quota-heading');
          const title = heading.querySelector('h2'), actions = heading.querySelector('.section-actions');
          const range = document.createRange();range.selectNodeContents(btn);
          const lines = new Set([...range.getClientRects()].filter(r => r.width > 0.5 && r.height > 0.5).map(r => Math.round(r.top / 4))).size;
          const b = btn.getBoundingClientRect(), t = title.getBoundingClientRect(), a = actions.getBoundingClientRect();
          const nav = document.querySelector('aside > nav');
          return {
            lines,
            height: b.height,
            fontSize: parseFloat(getComputedStyle(btn).fontSize),
            showLabels: heading.hasAttribute('data-show-labels'),
            sameRow: Math.abs(t.top - a.top) < Math.max(t.height, a.height) * 0.6,
            navScrolls: nav.scrollHeight > nav.clientHeight + 2,
            navFade: nav.dataset.voverflow || '',
            overflow: document.documentElement.scrollWidth > innerWidth + 1,
          };
        });
        assert.equal(quotaHeader.lines, 1, `${platform} ${language} #read-quotas must stay single-line: ${JSON.stringify(quotaHeader)}`);
        assert(quotaHeader.height <= quotaHeader.fontSize * 2.1, `${platform} ${language} #read-quotas height stays compact: ${JSON.stringify(quotaHeader)}`);
        if (quotaHeader.showLabels) assert(quotaHeader.sameRow, `${platform} ${language} data-show-labels must not stay on when actions wrap`);
        assert(!quotaHeader.overflow, `${platform} ${language} narrow large-font overview fits horizontally`);
        if (quotaHeader.navScrolls) assert(quotaHeader.navFade.includes('end'), `${platform} ${language} overflowing sidebar nav shows vertical fade`);
      }
      await page.setViewportSize({width:820,height:600});
      await page.evaluate(async () => { await setLanguage('zh-CN'); await openSettings({category:'hosts'}); });
      await page.waitForFunction(() => Boolean(document.querySelector('.machine-row')));
      const hostRows = await page.evaluate(() => [...document.querySelectorAll('.machine-row')].map(row => {
        const kids = [...row.children].filter(k => k.getBoundingClientRect().width > 0).map(k => k.getBoundingClientRect());
        let overlap = false;
        for (let i = 0; i < kids.length; i++) for (let j = i + 1; j < kids.length; j++) {
          if (Math.min(kids[i].right, kids[j].right) - Math.max(kids[i].left, kids[j].left) > 2 &&
              Math.min(kids[i].bottom, kids[j].bottom) - Math.max(kids[i].top, kids[j].top) > 2) overlap = true;
        }
        const edit = row.querySelector('.machine-actions button');
        let editLines = 1;
        if (edit && edit.innerText.trim()) {
          const range = document.createRange();range.selectNodeContents(edit);
          editLines = new Set([...range.getClientRects()].filter(r => r.width > 0.5 && r.height > 0.5).map(r => Math.round(r.top / 4))).size;
        }
        return {overlap, editLines, overflow: row.scrollWidth > row.clientWidth + 2};
      }));
      assert(hostRows.every(r => !r.overlap && r.editLines === 1 && !r.overflow), `${platform} large-font host rows stay aligned without overlap: ${JSON.stringify(hostRows)}`);
      await page.evaluate(() => openSettings({category:'general', section:'notifications'}));
      await page.waitForFunction(() => Boolean(document.querySelector('.sound-volume')));
      assert(await page.evaluate(() => [...document.querySelectorAll('.setting-row')].every(r => r.scrollWidth <= r.clientWidth + 2)), `${platform} large-font notification rows stay within container`);

      await page.setViewportSize({width:640,height:440});
      await setTextScale(page, 2);
      await page.evaluate(async () => {
        await setLanguage('en');
        await openSettings({category:'hosts'});
        await AieyesTheme.loadFonts();
        const invoke = __TAURI__.core.invoke;
        __TAURI__.core.invoke = async (command, args) => args?.method === 'hosts.sshConfig.read'
          ? { path:'/fixture/.ssh/config', hosts:[{ alias:'gpu', hostName:'gpu.example.test' }] }
          : invoke(command, args);
      });
      const sshToolbar = page.locator('.ssh-import-toolbar');
      const sshLayout = () => sshToolbar.evaluate(toolbar => {
        const bounds = toolbar.getBoundingClientRect();
        const [add, importConfig] = [...toolbar.querySelectorAll('button')].map(button => button.getBoundingClientRect());
        const fits = [add, importConfig].every(button => button.left >= bounds.left - 1 && button.right <= bounds.right + 1);
        return { fits, stacked:importConfig.top >= add.bottom };
      });
      assert.deepEqual(await sshLayout(), { fits:true, stacked:true }, `${platform} SSH actions stack within the minimum window`);
      await sshToolbar.locator('[data-ssh-import]').click();
      const importDialog = page.locator('#ssh-config-import');
      await importDialog.locator('.ssh-import-row').filter({hasText:'gpu.example.test'}).waitFor();
      await importDialog.locator('.dialog-head [data-import-close]').click();
      await page.setViewportSize({width:1120,height:800});
      await setTextScale(page, 1);
      assert.deepEqual(await sshLayout(), { fits:true, stacked:false }, `${platform} SSH actions share one row at normal size`);
      assert.deepEqual(errors,[]);await page.close();
      // Floating panel: no top-right close '×' button on Windows or Linux; single-line #read-quotas
      // and zero horizontal overflow at 360px compact panel with large font in both languages.
      const panelPage = await browser.newPage({viewport:{width:360,height:540}, reducedMotion:'reduce'}), panelErrors = [];
      panelPage.on('pageerror', e => panelErrors.push(String(e)));
      await panelPage.addInitScript(fixture, {platform});
      await panelPage.goto(`http://127.0.0.1:${server.address().port}/floating.html`);
      await panelPage.evaluate(() => AieyesApp.ready);
      assert.equal(await panelPage.locator('#panel-close').count(), 0, `${platform} floating panel must not render a top-right close button`);
      await setTextScale(panelPage, 1.35);
      for (const language of ['zh-CN', 'en']) {
        await panelPage.evaluate(lang => setLanguage(lang), language);
        await panelPage.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
        const panelState = await panelPage.evaluate(() => {
          const btn = document.querySelector('#read-quotas');
          const range = document.createRange();range.selectNodeContents(btn);
          const lines = new Set([...range.getClientRects()].filter(r => r.width > 0.5 && r.height > 0.5).map(r => Math.round(r.top / 4))).size;
          const usage = document.querySelector('.overview-usage');
          const tabs = document.querySelector('.panel-tabs'), tb = tabs.getBoundingClientRect();
          return {
            lines,
            usageOverflow: usage.scrollWidth > usage.clientWidth + 2,
            tabsCentered: Math.abs((tb.left + tb.right) / 2 - innerWidth / 2) < 1.5,
            tabsClipped: [...tabs.children].some(b => b.scrollWidth > b.clientWidth + 2),
            pageOverflow: document.documentElement.scrollWidth > innerWidth + 1,
          };
        });
        assert.equal(panelState.lines, 1, `${platform} ${language} panel #read-quotas stays single-line`);
        assert(!panelState.usageOverflow, `${platform} ${language} panel .overview-usage fits without overflow`);
        assert(panelState.tabsCentered && !panelState.tabsClipped, `${platform} ${language} panel tabs stay centered and unclipped: ${JSON.stringify(panelState)}`);
        assert(!panelState.pageOverflow, `${platform} ${language} compact panel fits viewport`);
      }
      const quotaCard = panelPage.locator('[data-quota="codex:a0"]');
      await quotaCard.locator('.quota-disclosure:not([open]) > summary').click();
      for (const language of ['zh-CN', 'en']) {
        await panelPage.evaluate(lang => setLanguage(lang), language);
        for (const scale of [1.5, 2]) {
          await setTextScale(panelPage, scale);
          const layout = await quotaCard.evaluate(card => {
            const content = card.querySelector('.quota-content');
            const bounds = content.getBoundingClientRect();
            const fits = rect => rect.left >= bounds.left - 1 && rect.right <= bounds.right + 1;
            const textFits = element => {
              const range = document.createRange();
              range.selectNodeContents(element);
              const rects = [...range.getClientRects()];
              return rects.length > 0 && rects.every(fits);
            };
            const estimates = [...content.querySelectorAll('.estimate-entry')];
            return {
              overflow:card.scrollWidth - card.clientWidth,
              estimatesFit:estimates.length === 2 && estimates.every(button =>
                textFits(button.querySelector('span')) && fits(button.lastElementChild.getBoundingClientRect())),
              countsFit:textFits(content.querySelector('.device-count-line')),
            };
          });
          assert(layout.overflow <= 1 && layout.estimatesFit && layout.countsFit,
            `${platform} ${language} expanded quota card fits at ${scale * 100}%: ${JSON.stringify(layout)}`);
        }
      }
      assert.deepEqual(panelErrors, []);await panelPage.close();
    }
    console.log('Desktop tables, heatmap alignment, menus, account picker, GPU warning semantics, panel chrome and narrow/large-font layout passed');
  } finally {await browser?.close();server.close();}
})().catch(e=>{console.error(e);process.exit(1);});
