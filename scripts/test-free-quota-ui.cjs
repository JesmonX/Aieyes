/* Settings → Advanced hides free accounts' quota cards. Uses the real core, so
   the `freePlan` flag comes from the same code the app ships. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {openGeneralPage}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-free-quota-'));
const logs=path.join(root,'logs');fs.mkdirSync(logs);
const core=spawn(path.join(project,'target/debug/aieyes-core'+(process.platform==='win32'?'.exe':'')),['--data-dir',root],{stdio:['pipe','pipe','pipe']});
let sequence=0,stderr='';
const pending=new Map();
core.stderr.on('data',buffer=>stderr+=buffer);
readline.createInterface({input:core.stdout}).on('line',line=>{const response=JSON.parse(line),request=pending.get(response.id);if(!request)return;pending.delete(response.id);if(response.error)request.reject(new Error(response.error.message));else request.resolve(response.result);});
core.on('exit',()=>{for(const request of pending.values())request.reject(new Error('Core exited: '+stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params){
  if(['hosts.sample','accounts.status.get','accounts.status.refresh','sources.scan'].includes(method))return [];
  return rpc(method,params);
}
// A Codex-shaped quota reply; the core reads the plan from `planType`.
function command(plan){
  const body=JSON.stringify({rateLimits:{planType:plan,primary:{usedPercent:20,windowDurationMins:300,resetsAt:Math.floor(Date.now()/1000)+18000}}});
  return process.platform==='win32'?`Write-Output '${body}'`:`echo '${body}'`;
}
function browserFixture(){
  window.setInterval=()=>0;
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(['desktop_info','desktop_panel'].includes(command))return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true,panelPinned:true};
    if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};
  }}};
}
const cards=page=>page.locator('[data-quota]:visible').evaluateAll(nodes=>nodes.map(node=>node.dataset.quota).sort());
(async()=>{
  let server,browser;
  try {
    assert.equal((await rpc('settings.get')).hideFreeQuotaCards,false,'The setting defaults to off');
    await rpc('settings.save',{version:12,sources:['free','plus'].map(id=>({id,name:id,provider:'codex',accountId:id,path:logs,enabled:true,quotaCommand:command(id)})),accounts:['free','plus'].map(id=>({id,name:id==='free'?'免费账户':'付费账户',provider:'codex',quotaEnabled:true})),hosts:[],localMonitor:{enabled:false},modelMappings:{}});
    const rows=await rpc('quotas.refresh',{});
    assert.deepEqual(rows.map(q=>[q.accountId,q.freePlan===true,q.error??null]).sort(),[['free',true,null],['plus',false,null]],'The core flags only the free plan');
    const web=path.join(project,'apps/desktop/web');
    server=http.createServer(require('./ui-test-helpers.cjs').serveWebRoot(web));
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    browser=await chromium.launch({headless:true});
    const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];
    page.on('pageerror',error=>errors.push(String(error)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(browserFixture);
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.deepEqual(await cards(page),['codex:free','codex:plus'],'Off: every account keeps its card');

    await openGeneralPage(page,'advanced');
    assert.deepEqual(await page.locator('[data-general-page]').allTextContents(),['外观','面板','桌宠与会话','启动与系统','网络代理','刷新与历史','通知','Webhook','聊天渠道','高级','更新与关于']);
    assert.equal(await page.locator('#field-hideFreeQuotaCards').isChecked(),false);
    await page.locator('#field-hideFreeQuotaCards').check();
    await page.waitForFunction(()=>state.settings.hideFreeQuotaCards===true&&!state.settingsCommitting);
    assert.equal((await rpc('settings.get')).hideFreeQuotaCards,true,'The toggle autosaves to the core');
    const shots=path.join(project,'.local/free-quota-ui');fs.mkdirSync(shots,{recursive:true});
    await page.screenshot({path:path.join(shots,'advanced.png')});

    await page.evaluate(()=>{state.page='agent';render();});
    assert.deepEqual(await cards(page),['codex:plus'],'On: the free account card is hidden');
    assert.equal(await page.locator('.quota-heading .count').textContent(),'1','Counts follow the visible cards');
    assert.equal(await page.locator('.quota-placeholder').count(),0,'A hidden account leaves no placeholder');

    await page.evaluate(()=>{state.accountKey='codex:free';render();});
    await page.locator('[data-free-quota-hidden]').waitFor();
    await page.screenshot({path:path.join(shots,'hidden-empty.png')});
    await page.locator('[data-open-advanced]').click();
    await page.waitForFunction(()=>state.page==='settings'&&state.settingsTab==='general'&&state.generalTab==='advanced');
    await page.evaluate(()=>{state.accountKey='';state.page='agent';render();});

    await page.evaluate(()=>{state.page='settings';state.settingsTab='integrations';render();window.AieyesAgentSettings.navigate({accountKey:'codex:free'});render();});
    await page.locator('[data-free-quota-hint]').waitFor();
    await page.evaluate(()=>{state.page='agent';render();});

    // An upgrade reported by a pushed sample brings the card back without a reload.
    await page.evaluate(()=>applyQuotaSamples([{...state.dashboard.quotas.find(q=>q.accountId==='free'),plan:'plus',freePlan:undefined,observationRevision:999}]));
    assert.deepEqual(await cards(page),['codex:free','codex:plus'],'A paid plan shows the card again');
    await page.evaluate(()=>applyQuotaSamples([{...state.dashboard.quotas.find(q=>q.accountId==='free'),plan:'free',freePlan:true,observationRevision:1000}]));
    assert.deepEqual(await cards(page),['codex:plus'],'Returning to free hides it again');

    const panel=await browser.newPage({viewport:{width:450,height:760}});
    panel.on('pageerror',error=>errors.push(String(error)));await panel.exposeFunction('testRPC',invoke);await panel.addInitScript(browserFixture);
    await panel.goto(`http://127.0.0.1:${server.address().port}/floating.html`);await panel.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.deepEqual(await cards(panel),['codex:plus'],'The panel hides the free account too');
    assert.deepEqual(await panel.evaluate(()=>AieyesUI.panelAccounts(shownQuotaSettings(),state.dashboard.quotaOrder??[]).codex),['codex:plus'],'Automatic panel slots skip hidden accounts');
    await page.evaluate(()=>openPanelAccounts());
    assert.deepEqual(await page.locator('[data-panel-account]').evaluateAll(nodes=>nodes.map(node=>node.dataset.panelAccount)),['codex:plus'],'The panel account settings offer only shown accounts');

    await openGeneralPage(page,'advanced');
    await page.locator('#field-hideFreeQuotaCards').uncheck();
    await page.waitForFunction(()=>state.settings.hideFreeQuotaCards===false&&!state.settingsCommitting);
    await page.evaluate(()=>{state.page='agent';render();});
    assert.deepEqual(await cards(page),['codex:free','codex:plus'],'Turning it off restores the card');
    assert.equal((await rpc('settings.get')).hideFreeQuotaCards,false);
    assert.deepEqual(errors,[]);
    console.log('Free quota cards: core flag, autosaved Advanced toggle, hidden cards and counts, hidden empty state, account hint, tier flips and panel selection passed');
  } finally {
    if(browser)await browser.close();if(server)await new Promise(resolve=>server.close(resolve));
    core.stdin.end();if(core.exitCode===null)await new Promise(resolve=>core.once('exit',resolve));fs.rmSync(root,{recursive:true,force:true});
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
