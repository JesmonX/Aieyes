/* Real core settings + browser forms; paid service requests/keyring are simulated. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont,discardEditor}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-subscriptions-'));
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',root],{stdio:['pipe','pipe','pipe']});
const pending=new Map();let seq=0,stderr='',fail=true;const calls=[];
core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error(stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params){
  calls.push({method,params});
  if(method==='subscriptions.openLogin')return {opened:true};
  if(method==='subscriptions.connect'){
    if(fail)throw new Error('订阅查询失败（HTTP 401）');
    const settings=await rpc('settings.get'),id=params.accountId||'fixture-'+params.provider;
    settings.accounts.push({id,provider:params.provider,name:params.name,quotaEnabled:true});
    settings.sources.push({id:'source-'+id,provider:params.provider,name:params.name,accountId:id,enabled:true,path:'',subscription:{...params.subscription,credentialRef:'opaque-fixture'}});
    await rpc('settings.save',settings);return {settings:await rpc('settings.get'),accountId:id};
  }
  if(['sources.scan','quotas.refresh','accounts.status.refresh'].includes(method))return {};
  return rpc(method,params);
}
function fixture(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true};if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};}}};}
(async()=>{
  const settings=await rpc('settings.get');settings.sources=[];settings.accounts=[];
  const providers=['zcode','qoder','grok','cursor','opencode'];
  for(const p of providers){const a=settings.agents.find(a=>a.provider===p);a.enabled=true;a.machineIds=['local'];}
  await rpc('settings.save',settings);
  const web=path.join(project,'apps/desktop/web');
  const server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try{
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];
    page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(fixture);
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    for(const provider of providers){
      await page.evaluate(p=>{state.page='settings';state.settingsTab='integrations';AieyesAgentSettings.navigate({provider:p});render();},provider);
      await page.locator(`[data-account-add="${provider}"]`).click();
      assert.equal(await page.locator('#field-secret').getAttribute('type'),'password');
      const previews=path.join(project,'.local/subscriptions');fs.mkdirSync(previews,{recursive:true});await page.screenshot({path:path.join(previews,provider+'.png')});
      await page.locator('#subscription-login').click();
      assert(calls.some(c=>c.method==='subscriptions.openLogin'&&c.params.provider===provider));
      await page.locator('#field-name').fill(provider+' fixture');await page.locator('#field-secret').fill('private-fixture-secret');
      fail=true;await page.locator('#editor-form button[type=submit]').click();await page.locator('#editor-error:visible').waitFor();
      assert.equal(await page.locator('#field-secret').inputValue(),'private-fixture-secret');
      assert(!(await rpc('settings.get')).accounts.some(a=>a.provider===provider));
      fail=false;await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open).catch(async e=>{throw new Error(provider+': '+await page.locator('#editor-error').textContent()+' / '+e.message);});
      const saved=await rpc('settings.get');assert(saved.accounts.some(a=>a.provider===provider));assert(!JSON.stringify(saved).includes('private-fixture-secret'));
    }
    await page.locator('[data-account-add="opencode"]').click();await page.locator('[name=useLocalLogin]').check();
    await discardEditor(page);assert.equal((await rpc('settings.get')).accounts.length,5);
    assert.deepEqual(errors,[]);console.log('Subscription forms: five providers, login routing, retry, cancellation, opaque settings passed');
  }finally{if(browser)await browser.close();server.close();core.kill();await new Promise(r=>core.once('close',r));fs.rmSync(root,{recursive:true,force:true});}
})().catch(e=>{console.error(e);core.kill();process.exitCode=1;});
