/* UI contract test with real settings/core and mocked auth responses. Network and
   keyring behavior are tested separately by buddy_auth's local HTTP fixtures. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright'),{serveFont,revealControl}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-buddy-ui-')));
const core=spawn(process.env.AIEYES_CORE_BINARY||path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env:{...process.env,AIEYES_AUTO_SCAN:'off',HOME:root,USERPROFILE:root,CODEX_HOME:path.join(root,'.codex'),HTTPS_PROXY:'http://127.0.0.1:9',HTTP_PROXY:'http://127.0.0.1:9'}});
const pending=new Map(),calls=[];let sequence=0,stderr='',auth={id:'fixture',phase:'idle',connected:false},rejectStart=false;
core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error(stderr||'Core exited'));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params={}){
  if(method.startsWith('buddyAuth.')){calls.push({method,params});if(method==='buddyAuth.start'){if(rejectStart)throw Error('Buddy 登录失败，请重试或在官方客户端查看余额');auth={id:'fixture',phase:'waiting',connected:false,message:'请在浏览器中完成 Buddy 登录',authUrl:'https://www.workbuddy.cn/login?state=fixture'};}if(method==='buddyAuth.cancel')auth={id:'fixture',phase:'cancelled',connected:false};if(method==='buddyAuth.logout')auth={id:'fixture',phase:'idle',connected:false};return auth;}
  if(['quotas.refresh','hosts.sample','wakeups.list','accounts.status.refresh','accounts.status.refreshBatch','accounts.status.get','accounts.devices','sources.scan','prices.list'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],sources:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  return rpc(method,params);
}
function bridge(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='engine_call')return window.testRPC(args.method,args.params??{});if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true};return {};}}};}
(async()=>{let browser,server;try{
  const settings=await rpc('settings.get');for(const a of settings.agents){a.enabled=['workbuddy','codebuddy'].includes(a.provider);a.machineIds=['local'];}await rpc('settings.save',settings);
  const web=path.join(project,'apps/desktop/web');server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});await new Promise(r=>server.listen(0,'127.0.0.1',r));
  browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1120,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(bridge);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
  for(const provider of ['workbuddy','codebuddy']){
    await page.evaluate(provider=>{state.page='settings';state.settingsTab='sources';state.settingsAgent=provider;render();},provider);
    assert.equal(calls.filter(c=>c.method==='buddyAuth.start').length,provider==='workbuddy'?0:2);
    await revealControl(page,`[data-buddy-login="${provider}"]`);await page.locator(`[data-buddy-login="${provider}"]`).click();const dialog=page.locator('#buddy-login');await dialog.waitFor();assert.match(await dialog.innerText(),/非公开接口|unofficial API/);
    const before=calls.length;await page.waitForTimeout(100);assert(!calls.slice(before).some(c=>c.method==='buddyAuth.start'),'Opening the dialog never starts login');
    rejectStart=true;await dialog.locator('#buddy-start').click();await dialog.locator('#buddy-error').filter({hasText:/Buddy/}).waitFor();assert.equal(await dialog.locator('#buddy-start').isDisabled(),false);
    rejectStart=false;await dialog.locator('#buddy-region').selectOption('global');await dialog.locator('#buddy-start').click();await dialog.locator('#buddy-url').waitFor({state:'visible'});assert.equal(calls.at(-1).params.provider,provider);assert.equal(calls.at(-1).params.region,'global');assert.equal(await dialog.locator('#buddy-start').isDisabled(),true);
    await dialog.locator('#buddy-close').click();await page.waitForFunction(()=>!document.querySelector('#buddy-login'));assert.equal(calls.at(-1).method,'buddyAuth.cancel');
  }
  auth={id:'fixture',phase:'connected',connected:true,message:'Buddy 登录完成，余额与 CodeBuddy / WorkBuddy 共用'};
  await revealControl(page,'[data-buddy-login="codebuddy"]');await page.locator('[data-buddy-login="codebuddy"]').click();const dialog=page.locator('#buddy-login');await dialog.locator('#buddy-logout').waitFor({state:'visible'});await dialog.locator('#buddy-logout').click();await dialog.locator('#buddy-logout').waitFor({state:'hidden'});await dialog.locator('#buddy-close').click();assert.deepEqual(errors,[]);
  console.log('Buddy UI: optional sign-in, no auto-login, retry, region/provider, cancellation and shared logout passed');
}finally{if(browser)await browser.close();if(server)await new Promise(r=>server.close(r));core.kill();await new Promise(r=>core.exitCode!==null?r():core.once('exit',r));fs.rmSync(root,{recursive:true,force:true});}})().catch(e=>{console.error(e);process.exitCode=1;});
