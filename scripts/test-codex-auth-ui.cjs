const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
 window.setInterval=()=>0;
 const settings={accounts:[{id:'a',name:'个人账号',provider:'codex',quotaEnabled:true,quotaProfileId:'home:one',identityKey:'one'},{id:'b',name:'工作账号',provider:'codex',quotaEnabled:true,quotaProfileId:'home:two',identityKey:'two'}],sources:[{id:'s',name:'本机 Codex',provider:'codex',path:'/fixture/.codex',accountId:'',codexHomeId:'home',enabled:true}],hosts:[],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,serverRefreshSeconds:10};
 window.authTest={calls:[],unknown:false,loginDone:false,loggedIn:true,active:'one',target:'two',processCount:24,preparedCount:0};
 authTest.forgetSecond=()=>{settings.accounts=settings.accounts.filter(a=>a.id!=='b');};
 window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
  if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:true};if(command!=='engine_call')return {};
  const {method,params}=args;authTest.calls.push({method,params});
  if(method==='hello')return {version:'test'};if(method==='settings.get')return structuredClone(settings);
  if(method==='dashboard')return {generatedAt:Date.now()/1000,summary:{tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0},quotas:[],quotaOrder:[],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
  if(['sources.scan','prices.list','quotas.refresh','wakeups.list'].includes(method))return [];
  if(method==='accounts.status.get')return [];
  if(method==='accounts.status.refresh'){
    const row={accountKey:params.accountKey,sourceId:params.sourceId,machineId:'local',current:authTest.loggedIn&&authTest.active===(params.accountKey==='codex:a'?'one':'two'),credential:authTest.loggedIn};
    if(authTest.pauseNextStatus){authTest.pauseNextStatus=false;return new Promise(resolve=>{authTest.releaseStatus=()=>resolve(row);});}
    return row;
  }
  if(method==='codexAuth.inspect'){
    if(authTest.failInspectNext){authTest.failInspectNext=false;throw new Error('fixture 状态读取失败');}
    return {version:'codex-cli 0.161.0',storageMode:'file',processes:[],profiles:[{id:'one',name:'个人账号',current:authTest.active==='one',identity:{key:'one',email:'one@example.test',plan:'plus',workspace:'team'}},{id:'two',name:'工作账号',current:authTest.active==='two',identity:{key:'two',email:'two@example.test',plan:'pro',workspace:'team'}}]};
  }
  if(method==='codexAuth.switch.prepare'){
    authTest.target=params.profileId;
    return {operationId:'switch-'+(++authTest.preparedCount),profileId:params.profileId,status:'prepared',preservedProcesses:[{pid:999,name:'Codex',canClose:false,blockingReason:'已确认使用独立 API 凭据，将保留'}],processes:Array.from({length:authTest.processCount},(_,i)=>({pid:123+i,parentPid:i?123:null,name:i?'codex-code-mode-host':'ChatGPT 应用',canClose:!authTest.unknown||i!==1,blockingReason:authTest.unknown&&i===1?'无法确认此辅助进程与 Codex 的父子关系，请手动关闭后重新检查':null}))};
  }
  if(method==='codexAuth.switch.commit'){
    if(authTest.failCommit)throw new Error('切换检查已过期，请重新检查');
    let accountId;
    if(!authTest.ignoreCommit){
      authTest.active=authTest.target;
      if(authTest.target==='two'&&!settings.accounts.some(a=>a.id==='b')){
        settings.accounts.push({id:'b',name:'工作账号',provider:'codex',quotaEnabled:true,quotaProfileId:'home:two',identityKey:'two'});accountId='b';
      }
    }
    return {operationId:params.operationId,status:'succeeded',accountId,message:'账号已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话'};
  }
  if(method==='codexAuth.login.start')return {operationId:'login',status:'running'};
  if(method==='codexAuth.login.status')return authTest.loginDone?{operationId:'login',status:'succeeded'}:{operationId:'login',status:'awaitingAuthorization',authUrl:'https://auth.openai.com/device',userCode:'FIXTURE-CODE'};
  if(method==='codexAuth.login.cancel'){authTest.loginDone=true;return {cancelRequested:true};}
  return {};
 }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404);return res.end();}res.setHeader('content-type',({'.js':'application/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
  await page.evaluate(()=>{state.page='settings';state.settingsTab='accounts';render();});
  await page.locator('[data-device-manage][data-intent="switch"]').first().click();
  assert.equal(await page.getByRole('button',{name:'读取账号',exact:true}).count(),0);await page.getByText('工作账号',{exact:true}).waitFor();
  await page.locator('[data-auth="switch"]').click();
  await page.locator('#codex-switch[open]').waitFor();
  await page.getByRole('button',{name:'关闭并切换',exact:true}).waitFor();
  await page.locator('#codex-switch').getByText('切换账号将关闭此设备上使用登录账号的 Codex 和 ChatGPT 进程及应用；已确认使用独立 API 凭据的实例及其辅助进程会保留。请先保存工作。',{exact:true}).waitFor();
  await page.locator('#codex-switch').getByText('ChatGPT 应用 · PID 123',{exact:true}).waitFor();
  await page.locator('#codex-switch').getByText('将保留 1 个使用独立 API 凭据的进程（含辅助进程）。',{exact:true}).waitFor();
  assert.equal(await page.locator('#codex-switch .codex-switch-processes').getByText('PID 999',{exact:false}).count(),0);
  assert.equal(await page.evaluate(()=>authTest.calls.some(c=>c.method==='codexAuth.switch.commit')),false);
  await page.setViewportSize({width:640,height:620});
  const commitBox=await page.locator('[data-auth="commit"]').boundingBox();
  assert(commitBox.y>=0&&commitBox.y+commitBox.height<=620,'confirmation action must remain visible with many processes');
  await page.screenshot({path:path.resolve(__dirname,'../.local/codex-switch-confirm.png'),animations:'disabled'});
  const dialogBox=await page.locator('#codex-switch').boundingBox();
  assert(dialogBox.y>=16&&dialogBox.y+dialogBox.height<=604,'the entire confirmation must fit inside the small window');
  await page.locator('[data-auth="cancelSwitch"]').click();
  assert.equal(await page.evaluate(()=>authTest.calls.some(c=>c.method==='codexAuth.switch.commit')),false);

  await page.evaluate(()=>authTest.unknown=true);
  await page.locator('[data-auth="switch"]').click();
  await page.locator('#codex-switch').getByText('暂时无法切换，请先处理以下进程，再重新检查。').waitFor();
  assert(await page.locator('[data-auth="commit"]').isDisabled());
  await page.evaluate(()=>authTest.unknown=false);
  await page.locator('[data-auth="retrySwitch"]').click();
  await page.waitForFunction(()=>!document.querySelector('[data-auth="commit"]').disabled);

  // Hold an observation of the old current account while a switch completes.
  await page.evaluate(()=>{authTest.pauseNextStatus=true;authTest.background=AieyesAgentSettings.refreshStatuses(true);});
  await page.waitForFunction(()=>typeof authTest.releaseStatus==='function');
  const preparations=await page.evaluate(()=>authTest.preparedCount);
  await page.locator('[data-auth="commit"]').click();
  await page.getByText('账号已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话',{exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>authTest.active),'two');
  assert.equal(await page.evaluate(()=>authTest.preparedCount),preparations,'refresh must not prepare another switch');
  assert.equal(await page.evaluate(()=>authTest.calls.find(c=>c.method==='codexAuth.switch.commit').params.closeProcesses),true);
  await page.evaluate(()=>{authTest.releaseStatus();return authTest.background;});
  await page.evaluate(()=>AieyesAgentSettings.openDetails(state.settings.accounts.find(a=>a.id==='a')));
  assert.equal(await page.getByText('正在使用',{exact:true}).count(),0,'late old response must not restore the old current account');
  await page.getByText('已登录 · 未使用',{exact:true}).waitFor();

  // Account entry auto-prepares once; expired checks require an explicit recheck.
  await page.locator('[data-account-login][data-intent="switch"]').click();
  await page.locator('#codex-switch[open]').waitFor();
  await page.evaluate(()=>authTest.failCommit=true);
  await page.locator('[data-auth="commit"]').click();
  await page.locator('#codex-switch').getByText('切换检查已过期，请重新检查',{exact:true}).waitFor();
  assert(await page.locator('[data-auth="commit"]').isDisabled());
  await page.evaluate(()=>authTest.failCommit=false);
  await page.locator('[data-auth="retrySwitch"]').click();
  await page.waitForFunction(()=>!document.querySelector('[data-auth="commit"]').disabled);
  await page.evaluate(()=>authTest.failInspectNext=true);
  await page.locator('[data-auth="commit"]').click();
  await page.getByText('切换已提交，当前账号待确认。',{exact:true}).waitFor();
  const commits=await page.evaluate(()=>authTest.calls.filter(c=>c.method==='codexAuth.switch.commit').length);
  await page.locator('[data-auth="refresh"]').click();
  await page.getByText('账号已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话',{exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>authTest.active),'one');
  assert.equal(await page.evaluate(()=>authTest.calls.filter(c=>c.method==='codexAuth.switch.commit').length),commits,'state retry must not resubmit a switch');

  // A nominal success with a different actual identity must not produce a success notice.
  await page.evaluate(()=>AieyesAgentSettings.openDetails(state.settings.accounts.find(a=>a.id==='b')));
  await page.locator('[data-account-login][data-intent="switch"]').click();
  await page.locator('#codex-switch[open]').waitFor();
  await page.evaluate(()=>authTest.ignoreCommit=true);
  await page.locator('[data-auth="commit"]').click();
  await page.getByText('当前登录与目标账号不一致，可能已被其他程序修改，请重新检查。',{exact:false}).waitFor();
  assert.equal(await page.getByText('账号已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话',{exact:true}).count(),0);
  await page.evaluate(async()=>{
    authTest.ignoreCommit=false;authTest.forgetSecond();
    AieyesAgentSettings.accept(await api('settings.get'));renderSettings(false);
  });
  await page.locator('[data-auth="switch"][data-profile="two"]').click();
  await page.locator('#codex-switch[open]').waitFor();
  await page.locator('[data-auth="commit"]').click();
  await page.getByText('账号已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话',{exact:true}).waitFor();
  assert.equal(await page.evaluate(()=>state.settings.accounts.some(a=>a.id==='b')),true,'a switched saved profile must appear in this installation account list');
  await page.setViewportSize({width:1180,height:900});
  await page.evaluate(()=>authTest.loggedIn=false);await page.locator('#accounts-back').click();if(await page.locator('#account-detail-back').count())await page.locator('#account-detail-back').click();await page.locator('[data-device-manage][data-intent="login"]').first().click();await page.locator('[data-auth="reauth"][data-profile="one"]').click();assert.equal(await page.locator('#field-name').count(),0);await page.locator('#editor-form button[type="submit"]').click();await page.getByText('FIXTURE-CODE',{exact:true}).waitFor();assert.equal(await page.evaluate(()=>authTest.calls.find(c=>c.method==='codexAuth.login.start').params.profileId),'one');
  await page.locator('[data-auth="open"]').click();assert.equal(await page.evaluate(()=>authTest.calls.find(c=>c.method==='codexAuth.openUrl').params.url),'https://auth.openai.com/device');
  await page.screenshot({path:path.resolve(__dirname,'../.local/codex-accounts.png'),fullPage:true});
  await page.locator('[data-auth="cancelLogin"]').click();await page.getByText('登录成功，已启用此账号的额度查询。',{exact:true}).waitFor();
  await page.evaluate(()=>{state.settingsDraft.sources[0].name='未保存';renderSettings();});assert(await page.locator('[data-auth="adopt"]').isDisabled());
  assert.deepEqual(errors,[]);console.log('Codex auth UI: visible confirmation, helper blocking/recheck, actual identity, stale observations, retry without recommit, reauthorization and dirty settings passed');
 }finally{if(browser)await browser.close();server.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
