/* Two real browser windows share storage and a disposable core configuration. */
const assert=require('node:assert/strict'),fs=require('node:fs'),os=require('node:os'),path=require('node:path'),http=require('node:http'),readline=require('node:readline');
const {spawn,execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-theme-'));
execFileSync('python3',['-c','import sqlite3,json,sys; c=sqlite3.connect(sys.argv[1]); c.execute("CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT NOT NULL)"); c.execute("INSERT INTO kv VALUES(?,?)",("settings",json.dumps(dict(version=4,sources=[],accounts=[],hosts=[])))); c.commit()',path.join(root,'aieyes.sqlite')]);
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
    const expectTheme=async(theme)=>{for(const page of [main,panel]){await page.waitForFunction(t=>document.documentElement.dataset.theme===t,theme);assert.equal(await page.evaluate(()=>state.settings.appearance.theme),theme);}};
    await expectTheme('light');
    await main.evaluate(()=>{state.settingsDraft=structuredClone(state.settings);state.settingsDraft.modelMappings.unsaved='keep-this-draft';});
    await main.locator('[data-theme-toggle]').click();await expectTheme('dark');assert.deepEqual((await rpc('settings.get')).appearance,{theme:'dark',accent:'teal'});
    assert.equal(await main.evaluate(()=>state.settingsDraft.modelMappings.unsaved),'keep-this-draft');assert.equal((await rpc('settings.get')).modelMappings.unsaved,undefined);
    await panel.evaluate(()=>render());await expectTheme('dark');await panel.evaluate(()=>loadDashboard());await expectTheme('dark');
    await panel.locator('[data-theme-toggle]').click();await expectTheme('light');
    await main.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
    await panel.locator('[data-theme-toggle]').click();await expectTheme('dark');assert.equal(await main.locator('[name=appearanceTheme]').inputValue(),'dark');
    await main.locator('[name=appearanceTheme]').selectOption('light');await main.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting);await expectTheme('light');
    await main.locator('[name=appearanceTheme]').selectOption('system');await main.waitForFunction(()=>!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting);
    for(const page of [main,panel]){await page.emulateMedia({colorScheme:'dark'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');assert.equal(await page.evaluate(()=>state.settings.appearance.theme),'system');}
    await panel.locator('[data-theme-toggle]').click();await expectTheme('light');
    // A failed write does not broadcast or change either surface.
    failSave=true;await panel.locator('[data-theme-toggle]').click();await panel.getByText('模拟主题保存失败',{exact:false}).waitFor();await panel.waitForFunction(()=>!state.themeSaving);await expectTheme('light');failSave=false;
    await panel.locator('[data-theme-toggle]').click();await expectTheme('dark');await panel.waitForFunction(()=>!state.themeSaving);assert(await panel.locator('#message').isHidden());
    await main.reload();await main.evaluate(()=>AieyesApp.ready);await expectTheme('dark');
    for(const [page,name] of [[main,'main'],[panel,'panel']]){
      assert.equal(await page.locator('[data-theme-toggle]').getAttribute('aria-label'),'切换到浅色模式');
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
      await page.screenshot({path:path.join(out,name+'-dark.png'),fullPage:true});
      assert.equal(await page.evaluate(()=>themeNative.at(-1).theme),'dark');
    }
    assert.deepEqual(errors,[]);console.log('Theme: bidirectional main/panel sync, stale rerender, settings picker, system mode, drafts, failed saves and reload passed');
  }finally{if(browser)await browser.close();server.close();core.stdin.end();fs.rmSync(root,{recursive:true,force:true});}
})().catch(error=>{console.error(error);core.kill();process.exitCode=1;});
