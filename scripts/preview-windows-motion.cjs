// Reproducible browser recordings with synthetic IPC, never personal accounts.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const fixture=require('./fixtures/browser-ui.cjs');
const root=path.resolve(__dirname,'../apps/desktop/web');
const out=path.resolve(process.env.AIEYES_UI_OUT||path.join(__dirname,'../.local/windows-ui-sync/motion'));

(async()=>{
  fs.mkdirSync(out,{recursive:true});
  const server=http.createServer((req,res)=>{
    if(require('./ui-test-helpers.cjs').serveFont(req,res))return;
    const file=path.join(root,path.basename(new URL(req.url,'http://localhost').pathname)||'index.html');
    if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}
    res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png'})[path.extname(file)]||'application/octet-stream');
    res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser=await chromium.launch(),url=`http://127.0.0.1:${server.address().port}`;
  try {
    for(const theme of ['light','dark'])for(const surface of ['main','panel']){
      const viewport=surface==='main'?{width:1294,height:840}:{width:450,height:720};
      const context=await browser.newContext({viewport,reducedMotion:'no-preference',recordVideo:{dir:out,size:viewport}});
      await context.addInitScript(fixture);
      await context.addInitScript(theme=>{
        fixtureSettings.appearance={theme,accent:'indigo',chartPalette:'classic',chartContrast:'high'};
        desktopInfo.materials={main:'acrylic',floating:'acrylic'};
        localStorage.setItem('aieyes.appearance',JSON.stringify(fixtureSettings.appearance));
        const invoke=__TAURI__.core.invoke;
        __TAURI__.core.invoke=(command,args)=>command==='engine_call'&&['network.status','network.test'].includes(args.method)
          ?Promise.resolve({mode:'system',status:'ok',averageMs:28,testedAt:Date.now()/1000,sites:[{url:'https://example.test',latencyMs:28}]})
          :invoke(command,args);
      },theme);
      const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(String(error)));
      const pause=()=>page.waitForTimeout(550); // Deliberate viewing pauses, not test synchronization.
      await page.goto(url+(surface==='main'?'/index.html':'/floating.html'));
      await page.evaluate(()=>AieyesApp.ready);await page.evaluate(()=>AieyesTheme.loadFonts());await pause();
      if(surface==='main'){
        await page.locator('[data-page=settings]').click();await pause();
        await page.locator('[data-settings-tab=general]').click();await pause();
        await page.locator('#field-appearanceChartPalette-choice').click();await pause();
        await page.keyboard.press('ArrowDown');await pause();await page.keyboard.press('Escape');
        await page.locator('[data-general-page=desktop]').click();await pause();
        await page.locator('[data-general-page=network]').click();await pause();
        await page.locator('[data-settings-tab=hosts]').click();await pause();
        await page.locator('[data-edit=host1]').click();await pause();
        await page.locator('#field-authMode-choice').click();await pause();await page.keyboard.press('Escape');
        await page.locator('#cancel-editor').click();await pause();
        await page.locator('#sidebar-toggle').click();await pause();
        await page.locator('#sidebar-toggle').click();await pause();
        await page.locator('[data-page=agent]').click();await pause();
      }else{
        await page.waitForFunction(()=>window.__listeners['desktop:panel']);
        const show=open=>page.evaluate(open=>{desktopInfo.panelOpen=open;__listeners['desktop:panel']({payload:structuredClone(desktopInfo)});},open);
        await show(true);await pause();
        await page.locator('#provider-choice').click();await pause();
        await page.keyboard.press('ArrowDown');await pause();await page.keyboard.press('Escape');
        await page.locator('#panel-more > summary').click();await pause();await page.keyboard.press('Escape');
        await page.locator('[data-page=servers]').click();await pause();
        await page.locator('[data-page=agent]').click();await pause();
        await show(false);await pause();await show(true);await pause();
      }
      assert.deepEqual(errors,[],`${surface} ${theme} preview must have no page errors`);
      assert(!/Unexpected call:|TypeError|ReferenceError/.test(await page.locator('body').innerText()),'preview fixtures cover all visible application calls');
      await page.screenshot({path:path.join(out,`${surface}-${theme}.png`)});
      const video=page.video();await context.close();
      await video.saveAs(path.join(out,`${surface}-${theme}.webm`));await video.delete();
      console.log(`Recorded ${surface}-${theme}.webm`);
    }
    fs.writeFileSync(path.join(out,'index.html'),`<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>Windows 动效预览</title>
<style>body{font:16px system-ui;max-width:1320px;margin:32px auto;padding:0 20px;background:#f3f4f6;color:#20212a}video{display:block;max-width:100%;border-radius:12px;margin:16px 0 32px;box-shadow:0 8px 30px #0002}p{line-height:1.7}</style>
<h1>Windows 动效预览</h1><p>使用项目实际页面、合成数据和 Chromium 录制。保留共享字号与尺寸，展示页面切换、单选菜单、弹窗、侧栏及悬浮面板动效。原生 DWM 背景由模拟状态启用，本页不能代表真实 Windows 桌面材质。</p>
${['light','dark'].flatMap(theme=>['main','panel'].map(surface=>`<h2>${theme==='light'?'浅色':'深色'} · ${surface==='main'?'主窗口':'悬浮面板'}</h2><video controls loop muted playsinline preload="metadata" poster="${surface}-${theme}.png" src="${surface}-${theme}.webm"></video>`)).join('')}
</html>`);
  } finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exit(1);});
