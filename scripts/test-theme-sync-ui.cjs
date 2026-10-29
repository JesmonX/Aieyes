const {openGeneralPage} = require('./ui-test-helpers.cjs');
/* Browser windows share storage and a disposable core configuration. */
const assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path'),http=require('node:http'),readline=require('node:readline');
const {spawn,execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-theme-'));
execFileSync(process.platform==='win32'?'python':'python3',['-c','import sqlite3,json,sys; c=sqlite3.connect(sys.argv[1]); c.execute("CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT NOT NULL)"); c.execute("INSERT INTO kv VALUES(?,?)",("settings",json.dumps(dict(version=4,sources=[],accounts=[],hosts=[])))); c.commit()',path.join(root,'aieyes.sqlite')]);
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',root],{stdio:['pipe','pipe','pipe']});
let id=0,failSave=false;const pending=new Map();
readline.createInterface({input:core.stdout}).on('line',line=>{const value=JSON.parse(line),promise=pending.get(value.id);if(!promise)return;pending.delete(value.id);value.error?promise.reject(new Error(value.error.message)):promise.resolve(value.result);});
const rpc=(method,params={})=>new Promise((resolve,reject)=>{pending.set(++id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});
const empty={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
async function invoke(method,params){
  if(method==='settings.patch'&&failSave)throw new Error('模拟主题保存失败');
  if(['settings.get','settings.patch'].includes(method))return rpc(method,params);
  if(method==='hello')return {version:'test'};
  if(method==='dashboard')return {generatedAt:Date.now()/1000,summary:empty,quotas:[],quotaOrder:[],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
  if(method==='sessions.list')return {sessions:[],warnings:[]};
  if(method==='accounts.deployments.sync')return {failedTasks:[]};
  return [];
}
function fixture(){
  window.setInterval=()=>0;window.themeNative=[];
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:true};
    if(command==='desktop_appearance'){themeNative.push(args);return {};}
    if(command==='engine_call')return testRPC(args.method,args.params??{});return {};
  }}};
}
(async()=>{
  const initial={version:4,sources:[],accounts:[],hosts:[],appearance:{theme:'light',accent:'teal'}};
  await rpc('settings.save',initial);
  const web=path.join(project,'apps/desktop/web'),out=path.join(project,'.local/theme-sync');fs.mkdirSync(out,{recursive:true});
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'image/png');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try {
    browser=await chromium.launch({headless:true});const context=await browser.newContext({colorScheme:'light'}),errors=[];
    await context.addInitScript(fixture);await context.exposeFunction('testRPC',invoke);
    const main=await context.newPage(),panel=await context.newPage();await main.setViewportSize({width:1120,height:820});await panel.setViewportSize({width:450,height:760});
    for(const [page,file] of [[main,'index.html'],[panel,'floating.html']]){page.on('pageerror',e=>errors.push(String(e)));await page.goto(`http://127.0.0.1:${server.address().port}/${file}`);await page.evaluate(()=>AieyesApp.ready);}
    const pickTheme=async(page,theme)=>{await page.locator('[data-theme-menu] > summary').click();await page.locator(`[data-theme-choice=${theme}]`).click();};
    const expectTheme=async(theme)=>{for(const page of [main,panel]){await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);assert.equal(await page.evaluate(()=>state.settings.appearance.theme),theme);}};
    await expectTheme('light');
    await main.evaluate(()=>{state.settingsDraft=structuredClone(state.settings);state.settingsDraft.modelMappings.unsaved='keep-this-draft';});
    await pickTheme(main,'dark');await expectTheme('dark');assert.deepEqual((await rpc('settings.get')).appearance,{theme:'dark',accent:'teal',chartPalette:'classic',chartContrast:'high',language:'zh-CN',settingsAgentIds:null});
    assert.equal(await main.evaluate(()=>state.settingsDraft.modelMappings.unsaved),'keep-this-draft');assert.equal((await rpc('settings.get')).modelMappings.unsaved,undefined);
    await panel.evaluate(()=>render());await expectTheme('dark');await panel.evaluate(()=>loadDashboard());await expectTheme('dark');
    await pickTheme(panel,'light');await expectTheme('light');
    await main.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
    await pickTheme(panel,'dark');await expectTheme('dark');assert.equal(await main.locator('[name=appearanceTheme]').inputValue(),'dark');
    await main.locator('[name=appearanceTheme]').selectOption('light');await main.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting);await expectTheme('light');
    await main.locator('[name=appearanceTheme]').selectOption('system');await main.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting);
    for(const page of [main,panel]){await page.emulateMedia({colorScheme:'dark'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');assert.equal(await page.evaluate(()=>state.settings.appearance.theme),'system');}
    // Following the system stays one choice away instead of being lost to a two-state switch.
    assert.equal(await panel.locator('[data-theme-choice=system]').getAttribute('aria-checked'),'true');
    await pickTheme(panel,'light');await expectTheme('light');
    // A failed write does not broadcast or change either surface.
    failSave=true;await pickTheme(panel,'dark');await panel.getByText('模拟主题保存失败',{exact:false}).waitFor();await panel.waitForFunction(()=>!state.themeSaving);await expectTheme('light');failSave=false;
    await pickTheme(panel,'dark');await expectTheme('dark');await panel.waitForFunction(()=>!state.themeSaving);assert(await panel.locator('#message').isHidden());
    await main.reload();await main.evaluate(()=>AieyesApp.ready);await expectTheme('dark');
    // The native shell reads the saved settings at startup, so a page never pushes its cached appearance: a stale
    // copy (localStorage commits late, so a crash loses recent changes) must not revert the tray and pet.
    await main.evaluate(()=>localStorage.setItem('aieyes.appearance',JSON.stringify({theme:'light',accent:'teal',language:'en'})));
    await main.reload();await main.evaluate(()=>AieyesApp.ready);await expectTheme('dark');
    const pushed=await main.evaluate(()=>themeNative.map((a)=>`${a.theme}/${a.language}`));
    assert(pushed.length&&pushed.every((p)=>p==='dark/zh-CN'),`Native appearance after a stale cache: ${pushed}`);
    // A sibling theme change also updates an inactive appearance page, preserving a failed local edit.
    await main.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
    failSave=true;await main.locator('[name=appearanceAccent]').selectOption('blue');
    await main.waitForFunction(()=>!state.generalSavePending&&state.saveError==='模拟主题保存失败');
    await openGeneralPage(main, 'network');failSave=false;
    await pickTheme(panel,'light');await expectTheme('light');
    await openGeneralPage(main, 'appearance');
    assert.equal(await main.locator('[name=appearanceTheme]').inputValue(),'light','Inactive appearance page adopts the saved sibling theme');
    assert.equal(await main.locator('[name=appearanceAccent]').inputValue(),'blue','Failed local appearance edit is retained');
    await main.locator('#settings-retry-save').click();
    await main.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting&&!state.saveError);
    assert.deepEqual((await rpc('settings.get')).appearance,{theme:'light',accent:'blue',chartPalette:'classic',chartContrast:'high',language:'zh-CN',settingsAgentIds:null});
    for(const page of [main,panel])await page.waitForFunction(()=>document.documentElement.dataset.accent==='blue');
    await pickTheme(panel,'dark');await expectTheme('dark');
    // Exercise the saved controls against the real core, including every theme/contrast combination.
    await main.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
    const analysis=await context.newPage();analysis.on('pageerror',e=>errors.push(String(e)));
    await analysis.goto(`http://127.0.0.1:${server.address().port}/index.html`);await analysis.evaluate(()=>AieyesApp.ready);await analysis.evaluate(()=>AieyesApp.navigatePage('analysis'));
    for(const palette of ['classic','tableau','tol','set2','nord']){
      await main.locator('[name=appearanceChartPalette]').selectOption(palette);
      await main.waitForFunction(p=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting&&state.settings.appearance.chartPalette===p,palette);
      for(const contrast of ['high','low']){
        await main.locator('[name=appearanceChartContrast]').selectOption(contrast);
        await main.waitForFunction(c=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting&&state.settings.appearance.chartContrast===c,contrast);
        for(const page of [main,panel,analysis])await page.waitForFunction(({palette,contrast})=>document.documentElement.dataset.chartPalette===palette&&document.documentElement.dataset.chartContrast===contrast,{palette,contrast});
        const saved=(await rpc('settings.get')).appearance;assert.equal(saved.chartPalette,palette);assert.equal(saved.chartContrast,contrast);
        assert.equal(await main.locator('.chart-palette-sample').count(),2);
        await main.locator('#chart-palette-preview').screenshot({path:path.join(out,`palette-${palette}-${contrast}.png`)});
        for(const theme of ['light','dark']){
          const result=await analysis.evaluate(({theme})=>{
            state.settings.appearance.theme=theme;state.page='analysis';
            const names=['chart-a','chart-b','chart-c'];
            const usage={tokens:{input:100,output:20,cacheRead:0,cacheWrite:0},total:120,cost:1,pricedTokens:120,events:1};
            state.dashboard.trendDays=[{...usage,key:'2026-10-11',total:360}];state.dashboard.dayModels=names.map(model=>({day:'2026-10-11',model,usage}));
            // Seven models: the ring shows the top five plus "其他", the trend shares the first three.
            const ranked=[...names,'chart-d','chart-e','chart-f','chart-g'];state.dashboard.models=ranked.map((key,i)=>({...usage,key,total:700-i*100}));render();
            const ctx=document.querySelector('#trend').getContext('2d'),paint=[],original=ctx.fillRect;
            ctx.fillRect=function(...args){paint.push(this.fillStyle);return original.apply(this,args);};
            try{drawTrend();}finally{ctx.fillRect=original;}
            const expected=names.map(name=>color(name)),slots=names.map(name=>modelSlots.get(document.documentElement.dataset.chartPalette).get(name));
            const probe=document.body.appendChild(document.createElement('i')),rgb=value=>{probe.style.color=value;return getComputedStyle(probe).color;};
            const ring=[...document.querySelectorAll('.donut path')].map(path=>rgb(getComputedStyle(path).fill)),swatches=[...document.querySelectorAll('.pie-wrap .legend .color')].map(swatch=>rgb(getComputedStyle(swatch).backgroundColor));
            const sync={ring,swatches,paint:paint.map(rgb),models:ranked.slice(0,5).map(name=>rgb(color(name))),palette:chartColors().map(rgb),otherSlot:modelSlots.get(document.documentElement.dataset.chartPalette).has(AieyesI18n.t('其他'))};probe.remove();
            return {paint,expected,slots,colors:chartColors(),sync};
          },{theme});
          assert.deepEqual(result.paint,result.expected);assert.equal(new Set(result.slots).size,3);
          assert(result.expected.every(color=>result.colors.includes(color)));
          // Bars, ring and list read one model mapping; "其他" is a neutral outside the model slots.
          const {sync}=result,other=sync.ring[5];
          assert.deepEqual(sync.ring.slice(0,3),sync.paint);assert.deepEqual(sync.ring.slice(0,5),sync.models);
          assert(!sync.palette.includes(other)&&!sync.otherSlot,`"其他" must not take a model color: ${palette} ${contrast} ${theme}`);
          assert.deepEqual(sync.swatches,[...sync.models,other,other]);
        }
      }
    }
    await analysis.close();
    // Reload proves persistence rather than relying on the live storage broadcast.
    await panel.reload();await panel.evaluate(()=>AieyesApp.ready);
    assert.equal(await panel.locator('html').getAttribute('data-chart-palette'),'nord');assert.equal(await panel.locator('html').getAttribute('data-chart-contrast'),'low');
    await main.locator('[name=appearanceChartPalette]').selectOption('classic');await main.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting&&state.settings.appearance.chartPalette==='classic');
    await main.locator('[name=appearanceChartContrast]').selectOption('high');await main.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting&&state.settings.appearance.chartContrast==='high');
    await expectTheme('dark');
    for(const [page,name] of [[main,'main'],[panel,'panel']]){
      assert.equal(await page.locator('[data-theme-menu] > summary').getAttribute('aria-label'),'主题：深色');
      assert.equal(await page.locator('[data-theme-choice=dark]').getAttribute('aria-checked'),'true');
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
      await page.screenshot({path:path.join(out,name+'-dark.png'),fullPage:true});
      assert.equal(await page.evaluate(()=>themeNative.at(-1).theme),'dark');
    }
    assert.deepEqual(errors,[]);console.log('Theme: bidirectional main/panel sync, inactive appearance page, retained failed drafts, stale rerender, settings picker, system mode, failed saves, reload and 20 chart palette variants passed');
  }finally{if(browser)await browser.close();server.close();core.stdin.end();if(core.exitCode===null)await new Promise(resolve=>core.once('exit',resolve));fs.rmSync(root,{recursive:true,force:true});}
})().catch(error=>{console.error(error);core.kill();process.exitCode=1;});
