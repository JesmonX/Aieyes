const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
 window.setInterval=()=>0;
 const settings={accounts:[{id:'a',name:'个人账号',provider:'codex',quotaEnabled:true,quotaProfileId:'home:one'}],sources:[{id:'s',name:'本机 Codex',provider:'codex',path:'/fixture/.codex',accountId:'',codexHomeId:'home',enabled:true}],hosts:[],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,serverRefreshSeconds:10};
 window.authTest={calls:[],unknown:false,loginDone:false,loggedIn:true};
 window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
  if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:true};if(command!=='engine_call')return {};
  const {method,params}=args;authTest.calls.push({method,params});
  if(method==='hello')return {version:'test'};if(method==='settings.get')return structuredClone(settings);
  if(method==='dashboard')return {generatedAt:Date.now()/1000,summary:{tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0},quotas:[],quotaOrder:[],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
  if(['sources.scan','prices.list','quotas.refresh','wakeups.list'].includes(method))return [];
  if(method==='accounts.status.refresh')return {accountKey:params.accountKey,sourceId:params.sourceId,machineId:'local',current:authTest.loggedIn,credential:authTest.loggedIn};
  if(method==='codexAuth.inspect')return {version:'codex-cli 0.161.0',storageMode:'file',processes:[],profiles:[{id:'one',name:'个人账号',current:true,identity:{email:'one@example.test',plan:'plus',workspace:'team'}},{id:'two',name:'工作账号',current:false,identity:{email:'two@example.test',plan:'pro',workspace:'team'}}]};
  if(method==='codexAuth.switch.prepare')return {operationId:'switch',status:'prepared',processes:[{pid:123,name:'Codex',canClose:!authTest.unknown}]};
  if(method==='codexAuth.switch.commit')return {status:'succeeded',message:'账号已切换，请重新打开 Codex 并恢复会话'};
  if(method==='codexAuth.login.start')return {operationId:'login',status:'running'};
  if(method==='codexAuth.login.status')return authTest.loginDone?{operationId:'login',status:'succeeded'}:{operationId:'login',status:'awaitingAuthorization',authUrl:'https://auth.openai.com/device',userCode:'FIXTURE-CODE'};
  if(method==='codexAuth.login.cancel'){authTest.loginDone=true;return {cancelRequested:true};}
  return {};
 }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),server=http.createServer((req,res)=>{const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404);return res.end();}res.setHeader('content-type',({'.js':'application/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
  await page.evaluate(()=>{state.page='settings';state.settingsTab='accounts';render();});
  await page.locator('[data-device-manage][data-intent="switch"]').first().click();
  assert.equal(await page.getByRole('button',{name:'读取账号',exact:true}).count(),0);await page.getByText('工作账号',{exact:true}).waitFor();
  await page.locator('[data-auth="switch"]').click();await page.getByRole('button',{name:'关闭并切换',exact:true}).waitFor();assert.equal(await page.evaluate(()=>authTest.calls.some(c=>c.method==='codexAuth.switch.commit')),false);
  await page.locator('[data-auth="cancelSwitch"]').click();assert.equal(await page.evaluate(()=>authTest.calls.some(c=>c.method==='codexAuth.switch.commit')),false);
  await page.evaluate(()=>authTest.unknown=true);await page.locator('[data-auth="switch"]').click();assert(await page.locator('[data-auth="commit"]').isDisabled());await page.locator('[data-auth="cancelSwitch"]').click();
  await page.evaluate(()=>authTest.unknown=false);await page.locator('[data-auth="switch"]').click();await page.locator('[data-auth="commit"]').click();await page.getByText('账号已切换，请重新打开 Codex 并恢复会话',{exact:true}).waitFor();assert.equal(await page.evaluate(()=>authTest.calls.find(c=>c.method==='codexAuth.switch.commit').params.closeProcesses),true);
  await page.evaluate(()=>authTest.loggedIn=false);await page.locator('#accounts-back').click();await page.locator('[data-device-manage][data-intent="login"]').first().click();await page.locator('[data-auth="reauth"][data-profile="one"]').click();assert.equal(await page.locator('#field-name').count(),0);await page.locator('#editor-form button[type="submit"]').click();await page.getByText('FIXTURE-CODE',{exact:true}).waitFor();assert.equal(await page.evaluate(()=>authTest.calls.find(c=>c.method==='codexAuth.login.start').params.profileId),'one');
  await page.locator('[data-auth="open"]').click();assert.equal(await page.evaluate(()=>authTest.calls.find(c=>c.method==='codexAuth.openUrl').params.url),'https://auth.openai.com/device');
  await page.screenshot({path:path.resolve(__dirname,'../.local/codex-accounts.png'),fullPage:true});
  await page.locator('[data-auth="cancelLogin"]').click();await page.getByText('登录成功，已启用此账号的额度查询。',{exact:true}).waitFor();
  await page.evaluate(()=>{state.settingsDraft.sources[0].name='未保存';renderSettings();});assert(await page.locator('[data-auth="adopt"]').isDisabled());
  assert.deepEqual(errors,[]);console.log('Codex auth UI: reviewed switch, unknown-process block, reauthorization, URL, polling, dirty settings passed');
 }finally{if(browser)await browser.close();server.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
