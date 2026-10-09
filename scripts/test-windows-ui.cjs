// The Python fixture also drives the native macOS renderer; no account/network access.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../apps/desktop/web');
const out=path.resolve(process.env.AIEYES_UI_OUT || path.join(__dirname,'../.local/windows-ui-sync/browser'));
function installFixture(data) {
  window.setInterval=()=>0;
  window.__fixture=data;
  const info={platform:'windows',page:'agent',mode:'auto',effectiveMode:'floating',nativeCapsule:true,panelOpen:true,panelPinned:false,material:'opaque',materials:{main:'opaque',floating:'opaque'},sessions:[],activeCount:0,summary:'暂无活跃会话'};
  const listeners=new Map();
  window.__setDesktopMaterials=(main,floating)=>{info.materials={main,floating};for(const fn of listeners.get('desktop:status')||[])fn({payload:info});};
  window.__TAURI__={event:{listen:async(name,fn)=>{listeners.set(name,[...(listeners.get(name)||[]),fn]);return ()=>{};}},window:{getCurrentWindow:()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})},core:{invoke:async(name,args)=>{
    if(name==='desktop_info')return info;
    if(name==='desktop_panel')return {...info,panelOpen:args.open};
    if(name!=='engine_call')return {};
    return structuredClone({'hello':{version:'fixture'},'settings.get':data.settings,'dashboard':data.dashboard,'prices.list':data.prices,'hosts.sample':data.hosts,'sessions.list':[],'wakeups.list':data.wakeups,'quotas.refresh':data.dashboard.quotas,'sources.scan':[]}[args.method]??{});
  }}};
}
(async()=>{
  fs.mkdirSync(out,{recursive:true});
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;
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
        const context=await browser.newContext({viewport:{width:1294,height:840},colorScheme:dark?'dark':'light'});
        await context.addInitScript(installFixture,data);
        const page=await context.newPage(),errors=[];page.on('pageerror',e=>errors.push(String(e)));
        await page.goto(`http://127.0.0.1:${server.address().port}`);
        await page.waitForFunction(()=>window.AieyesApp&&state.dashboard&&!state.busy);
        await page.evaluate(()=>window.AieyesTheme.loadFonts());
        const theme=dark?'dark':'light';
        await page.evaluate(dark=>{document.documentElement.dataset.theme=dark?'dark':'light';drawTrend();},dark);
        const shot=async name=>{assert(!/TypeError|ReferenceError|SyntaxError/.test(await page.locator('body').innerText()),'no caught runtime errors in rendered UI');return page.screenshot({path:path.join(out,`${scenario}-${theme}-${name}.png`),animations:'disabled'});};
        await shot('overview');
        assert.equal(await page.locator('[data-copy-value]').count(),0,'model rows have no copy buttons');
        for(const bar of await page.locator('.resource-bar,.track').all())assert.equal(await bar.evaluate(el=>getComputedStyle(el).height),'4px');
        for(const value of await page.locator('.stat-value').all())assert.equal(await value.evaluate(el=>getComputedStyle(el).fontSize),'24px');
        if(scenario==='single'&&!dark){
          const width=await page.evaluate(()=>innerWidth);
          const initial=await page.locator('aside').evaluate(el=>el.getBoundingClientRect().width);
          assert.equal(initial,214);
          await page.locator('#sidebar-toggle').click();
          assert.equal(await page.locator('aside').evaluate(el=>el.getBoundingClientRect().width),72);
          assert.equal(await page.evaluate(()=>innerWidth),width);
          await page.locator('#sidebar-toggle').click();
        }

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
          const hint=page.locator('#hint-fixture');await hint.hover();await page.locator('#titlebar').hover({position:{x:300,y:20}});await page.waitForTimeout(650);
          assert.equal(await page.locator('.hint:visible').count(),0,'leaving before delay cancels the pending hint');
          await hint.hover();await page.waitForTimeout(650);
          assert.equal(await page.locator('.hint:visible').count(),1);
          await hint.evaluate(el=>el.title='更新后的提示');await page.waitForTimeout(50);
          assert.equal(await hint.getAttribute('title'),null);assert.equal(await page.locator('.hint:visible').innerText(),'更新后的提示');
          await hint.evaluate(el=>el.remove());await page.waitForTimeout(200);assert.equal(await page.locator('.hint:visible').count(),0);
        }
        for(const tab of ['sources','accounts','hosts','prices','wakeups','general']) {
          await page.evaluate(tab=>{state.page='settings';state.settingsTab=tab;state.prices=__fixture.prices;render();},tab);
          await shot(`settings-${tab}`);
          assert.equal(await page.locator('#settings-save-all').count(),0);assert.match(await page.locator('.settings-draft-label').innerText(),/即时保存|未保存输入|自动保存/);
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
    // Material status must never leak from the main window into the floating panel.
    const materialData=JSON.parse(execFileSync(process.platform==='win32'?'python':'python3',[path.join(__dirname,'preview-quota-core.py'),'--fixture'],{env:{...process.env,AIEYES_UI_SCENARIO:'single'},encoding:'utf8'}));
    const materialReport=[];
    for(const surface of ['index.html','floating.html']) {
      const context=await browser.newContext({viewport:{width:surface==='index.html'?1294:450,height:surface==='index.html'?840:720}});
      await context.addInitScript(installFixture,materialData);
      const page=await context.newPage();await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);
      await page.waitForFunction(()=>state.dashboard&&!state.busy);await page.evaluate(()=>AieyesTheme.loadFonts());
      const session=await context.newCDPSession(page);
      for(const theme of ['light','dark'])for(const material of ['opaque','acrylic','blur']) {
        await page.evaluate(({theme,material,surface})=>{AieyesTheme.apply({theme,accent:'blue'});__setDesktopMaterials(surface==='index.html'?material:'opaque',surface==='floating.html'?material:'opaque');},{theme,material,surface});
        const metrics=await page.evaluate(()=>({material:document.documentElement.dataset.material,backing:getComputedStyle(document.body,'::before').backgroundColor,card:getComputedStyle(document.querySelector('.stat')).backgroundColor,bar:getComputedStyle(document.querySelector('.resource-bar,.track')).height,filter:document.querySelector('.filters select').getBoundingClientRect().height}));
        assert.equal(metrics.material,material);assert.equal(metrics.bar,'4px');assert.equal(metrics.filter,26);
        const alpha=color=>color.startsWith('rgba')?Number(color.match(/, ([\d.]+)\)$/)[1]):1;
        assert.equal(alpha(metrics.backing)<1,material!=='opaque',`${surface} ${theme} ${material} backing`);
        if(material!=='opaque')assert(alpha(metrics.card)<1);
        materialReport.push({surface,theme,...metrics});
        for(const feature of ['prefers-reduced-transparency','forced-colors']) {
          await session.send('Emulation.setEmulatedMedia',{features:[{name:feature,value:feature==='forced-colors'?'active':'reduce'}]});
          const backing=await page.evaluate(()=>getComputedStyle(document.body,'::before').backgroundColor);
          assert.equal(alpha(backing),1,`${feature} must restore an opaque backing`);
          if(feature==='forced-colors')assert.equal(await page.evaluate(()=>getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()),'LinkText');
          await session.send('Emulation.setEmulatedMedia',{features:[]});
        }
      }
      if(surface==='index.html') {
        await page.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
        assert.equal(await page.locator('input[role=switch]').first().evaluate(el=>el.getBoundingClientRect().height),21);
        await page.locator('#font-license').click();await page.locator('#quota-dialog').waitFor({state:'visible'});
        assert.match(await page.locator('#quota-dialog').innerText(),/HarmonyOS Sans Fonts License Agreement/);
        await page.locator('#quota-dialog').evaluate(el=>el.getAnimations().forEach(a=>a.finish()));
        assert(await page.locator('#quota-dialog').evaluate(el=>{const r=el.getBoundingClientRect();return r.top>=40&&r.bottom<=innerHeight-8;}),'license stays inside the client area');
        assert(await page.locator('.quota-dialog-body').evaluate(el=>{el.scrollTop=el.scrollHeight;return el.scrollTop>0;}),'the entire license can be scrolled');
        await page.locator('.quota-dialog-body').evaluate(el=>el.scrollTop=0);
        await page.screenshot({path:path.join(out,'font-license.png')});
      }
      await context.close();
    }
    fs.writeFileSync(path.join(out,'materials.json'),JSON.stringify(materialReport,null,2));
    // Bundled faces must render on all hosts; DPR checks include fractional scaling.
    for(const deviceScaleFactor of [1,1.25,1.5,2]) {
      const context=await browser.newContext({viewport:{width:450,height:720},deviceScaleFactor});
      const data=JSON.parse(execFileSync(process.platform==='win32'?'python':'python3',[path.join(__dirname,'preview-quota-core.py'),'--fixture'],{env:{...process.env,AIEYES_UI_SCENARIO:'single'},encoding:'utf8'}));
      await context.addInitScript(installFixture,data);const page=await context.newPage();
      await page.goto(`http://127.0.0.1:${server.address().port}/floating.html`);await page.waitForFunction(()=>state.dashboard&&!state.busy);
      await page.evaluate(()=>window.AieyesTheme.loadFonts());
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
      const fonts=await page.evaluate(()=>({body:getComputedStyle(document.body).fontFamily,title:getComputedStyle(document.querySelector('h2')).fontFamily,dpr:devicePixelRatio}));
      assert.equal(fonts.body,fonts.title);
      const session=await context.newCDPSession(page);await session.send('DOM.enable');await session.send('CSS.enable');
      const {root:documentNode}=await session.send('DOM.getDocument');
      const {nodeId}=await session.send('DOM.querySelector',{nodeId:documentNode.nodeId,selector:'.overview-heading h2'});
      const resolved=await session.send('CSS.getPlatformFontsForNode',{nodeId});
      assert(resolved.fonts.length>0,'record the fonts actually used to render Chinese headings');
      const weights=[];
      for(const weight of [400,500,700]) {
        await page.evaluate(weight=>{let el=document.querySelector('#font-probe');if(!el){el=document.createElement('span');el.id='font-probe';el.style.cssText='position:fixed;top:100px;left:20px;z-index:9999';document.body.append(el);}el.textContent='今日概览 Token 0123456789';el.style.fontWeight=weight;},weight);
        const {nodeId:probe}=await session.send('DOM.querySelector',{nodeId:documentNode.nodeId,selector:'#font-probe'});
        await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
        const used=await session.send('CSS.getPlatformFontsForNode',{nodeId:probe});
        assert(used.fonts.length&&used.fonts.every(font=>font.isCustomFont&&/HarmonyOS/.test(font.familyName)),`bundled weight ${weight}`);
        weights.push({weight,resolved:used.fonts});
      }
      await page.locator('#font-probe').evaluate(el=>el.remove());
      assert(resolved.fonts.every(font=>font.isCustomFont&&/HarmonyOS/.test(font.familyName)),'Chinese headings must use the bundled HarmonyOS font, including on Windows');
      fs.writeFileSync(path.join(out,`fonts-${deviceScaleFactor}.json`),JSON.stringify({platform:process.platform,...fonts,resolved:resolved.fonts,weights},null,2));
      await page.screenshot({path:path.join(out,`panel-dpi-${deviceScaleFactor}.png`)});await context.close();
    }
    console.log('Windows UI sync: six scenarios, two themes, settings, geometry, hints, materials/accessibility, license, bundled font weights and fractional DPI passed');
  } finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
