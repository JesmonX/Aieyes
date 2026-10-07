// The Python fixture also drives the native macOS renderer; no account/network access.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../apps/desktop/web');
const out=path.resolve(__dirname,'../.local/windows-ui-sync/browser');
function installFixture(data) {
  window.setInterval=()=>0;
  window.__fixture=data;
  const info={platform:'windows',page:'agent',mode:'auto',effectiveMode:'floating',nativeCapsule:true,panelOpen:true,panelPinned:false,material:'opaque',sessions:[],activeCount:0,summary:'暂无活跃会话'};
  window.__TAURI__={event:{listen:async()=>()=>{}},window:{getCurrentWindow:()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})},core:{invoke:async(name,args)=>{
    if(name==='desktop_info')return info;
    if(name==='desktop_panel')return {...info,panelOpen:args.open};
    if(name!=='engine_call')return {};
    return structuredClone({'hello':{version:'fixture'},'settings.get':data.settings,'dashboard':data.dashboard,'prices.list':data.prices,'hosts.sample':data.hosts,'sessions.list':[],'wakeups.list':data.wakeups,'quotas.refresh':data.dashboard.quotas,'sources.scan':[]}[args.method]??{});
  }}};
}
(async()=>{
  fs.mkdirSync(out,{recursive:true});
  const server=http.createServer((req,res)=>{
    const file=path.join(root,path.basename(new URL(req.url,'http://localhost').pathname)||'index.html');
    if(!fs.existsSync(file)){res.writeHead(404);return res.end();}
    res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png'})[path.extname(file)]||'text/plain');
    res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost");
    res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser=await chromium.launch();
  try {
    for(const scenario of ['empty','single','multi','long','failure','capacity']) {
      const data=JSON.parse(execFileSync(process.platform==='win32'?'python':'python3',[path.join(__dirname,'preview-quota-core.py'),'--fixture'],{env:{...process.env,AIEYES_UI_SCENARIO:scenario,AIEYES_UI_STAMP:'1791342000'},encoding:'utf8'}));
      for(const dark of [false,true]) {
        const context=await browser.newContext({viewport:{width:1100,height:800},colorScheme:dark?'dark':'light'});
        await context.addInitScript(installFixture,data);
        const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
        await page.goto(`http://127.0.0.1:${server.address().port}`);
        await page.waitForFunction(()=>window.AieyesApp&&state.dashboard&&!state.busy);
        const theme=dark?'dark':'light';
        await page.evaluate(dark=>{document.documentElement.dataset.theme=dark?'dark':'light';drawTrend();},dark);
        const shot=async name=>{assert(!/TypeError|ReferenceError|SyntaxError/.test(await page.locator('body').innerText()),'no caught runtime errors in rendered UI');return page.screenshot({path:path.join(out,`${scenario}-${theme}-${name}.png`),animations:'disabled'});};
        await shot('overview');
        if(scenario==='single'&&!dark) {
          const chart=page.locator('#trend');await chart.scrollIntoViewIfNeeded();const box=await chart.boundingBox();
          await page.mouse.move(box.x+90,box.y+30);await page.waitForTimeout(650);
          await page.mouse.move(box.x+150,box.y+30);await page.waitForTimeout(650);
          assert.equal(await chart.getAttribute('title'),null);
          assert.equal(await page.locator('#trend-reading').count(),1);
          assert.match(await page.locator('#trend-reading').innerText(),/2026-10-/);
          assert.equal(await page.locator('.hint:visible').count(),0);
          await chart.focus();await page.keyboard.press('ArrowRight');
          assert.equal(await chart.getAttribute('title'),null);
          await page.locator('[data-cost-mode=true]').click();assert.equal(await page.locator('[data-cost-mode=true]').getAttribute('aria-pressed'),'true');
          await page.locator('[data-cost-mode=false]').click();
          await page.locator('#trend').focus();
          await page.evaluate(()=>drawTrend());assert.equal(await page.locator('#trend-reading').count(),1);
          // A dynamically changing tooltip has one owner, including before delay.
          await page.evaluate(()=>{const b=document.createElement('button');b.id='hint-fixture';b.title='第一条';b.textContent='提示回归';b.style='position:fixed;top:80px;left:20px;z-index:9999';document.body.append(b);});
          const hint=page.locator('#hint-fixture');await hint.hover();await page.mouse.move(700,90);await page.waitForTimeout(650);
          assert.equal(await page.locator('.hint:visible').count(),0,'leaving before delay cancels the pending hint');
          await hint.hover();await page.waitForTimeout(650);
          assert.equal(await page.locator('.hint:visible').count(),1);
          await hint.evaluate(el=>el.title='更新后的提示');await page.waitForTimeout(50);
          assert.equal(await hint.getAttribute('title'),null);assert.equal(await page.locator('.hint:visible').innerText(),'更新后的提示');
          await hint.evaluate(el=>el.remove());await page.waitForTimeout(200);assert.equal(await page.locator('.hint:visible').count(),0);
        }
        for(const tab of ['sources','hosts','prices','wakeups','general']) {
          await page.evaluate(tab=>{state.page='settings';state.settingsTab=tab;state.prices=__fixture.prices;render();},tab);
          await shot(`settings-${tab}`);
          assert.equal(await page.locator('#settings-save-all').innerText(),'保存应用配置');
        }
        await page.evaluate(()=>{state.page='servers';render();});await shot('servers');
        await page.setViewportSize({width:640,height:440});await shot('minimum');
        assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'minimum window must not overflow');
        await page.goto(`http://127.0.0.1:${server.address().port}/floating.html`);
        await page.setViewportSize({width:450,height:720});await page.waitForFunction(()=>state.dashboard&&!state.busy);
        await page.evaluate(dark=>document.documentElement.dataset.theme=dark?'dark':'light',dark);await shot('panel');
        assert.deepEqual(errors,[]);await context.close();
      }
    }
    // Real Windows runs resolve the installed Windows faces; DPR checks work on all hosts.
    for(const deviceScaleFactor of [1,1.5,2]) {
      const context=await browser.newContext({viewport:{width:450,height:720},deviceScaleFactor});
      const data=JSON.parse(execFileSync(process.platform==='win32'?'python':'python3',[path.join(__dirname,'preview-quota-core.py'),'--fixture'],{env:{...process.env,AIEYES_UI_SCENARIO:'single'},encoding:'utf8'}));
      await context.addInitScript(installFixture,data);const page=await context.newPage();
      await page.goto(`http://127.0.0.1:${server.address().port}/floating.html`);await page.waitForFunction(()=>state.dashboard&&!state.busy);
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
      const fonts=await page.evaluate(()=>({body:getComputedStyle(document.body).fontFamily,title:getComputedStyle(document.querySelector('h2')).fontFamily,dpr:devicePixelRatio}));
      assert.equal(fonts.body,fonts.title);
      const session=await context.newCDPSession(page);await session.send('DOM.enable');await session.send('CSS.enable');
      const {root:documentNode}=await session.send('DOM.getDocument');
      const {nodeId}=await session.send('DOM.querySelector',{nodeId:documentNode.nodeId,selector:'.overview-heading h2'});
      const resolved=await session.send('CSS.getPlatformFontsForNode',{nodeId});
      assert(resolved.fonts.length>0,'record the fonts actually used to render Chinese headings');
      fs.writeFileSync(path.join(out,`fonts-${deviceScaleFactor}.json`),JSON.stringify({platform:process.platform,...fonts,resolved:resolved.fonts},null,2));
      await page.screenshot({path:path.join(out,`panel-dpi-${deviceScaleFactor}.png`)});await context.close();
    }
    console.log('Windows UI sync: shared native fixtures, six scenarios, two themes, all settings, hints and DPI passed');
  } finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
