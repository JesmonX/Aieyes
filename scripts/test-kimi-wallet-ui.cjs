/* Protocol snapshots -> real core cache/dashboard -> both browser surfaces.
   Rust subscriptions tests separately validate the official raw API conversion. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {DatabaseSync}=require('node:sqlite'),{spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright'),{serveFont}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-kimi-wallet-')));
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env:{...process.env,HOME:root,USERPROFILE:root,CODEX_HOME:path.join(root,'.codex')}});
const pending=new Map();let sequence=0,stderr='',db;
core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error(stderr||'Core exited'));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
function invoke(method,params={}){
  if(['quotas.refresh','hosts.sample','wakeups.list','accounts.status.refresh','accounts.devices'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],sources:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  return rpc(method,params);
}
function bridge(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='engine_call')return window.testRPC(args.method,args.params??{});if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true};return {};}}};}
const initialWallet={currency:'USD',total:'12.34',balance:'10.00',monthlyLimitEnabled:true,monthlyLimit:'15.00',monthlyUsed:'1.23'};
let version=0;
function seed(wallet=initialWallet,extra={}){
  const q={provider:'kimi',accountId:'wallet',sourceId:'source',name:'Kimi wallet',updatedAt:Math.floor(Date.now()/1000)+version++,origin:'live',windows:[{id:'limit_5h',name:'5h',usedPercent:25,windowMinutes:300}],balances:[],extraUsage:wallet,...extra};
  db.prepare('INSERT INTO quotas(account_key,payload) VALUES(?,?) ON CONFLICT(account_key) DO UPDATE SET payload=excluded.payload').run('kimi:wallet',JSON.stringify(q));return q;
}
(async()=>{
  let browser,server;
  try{
    const s=await rpc('settings.get');s.accounts=[{id:'wallet',provider:'kimi',name:'Kimi wallet',quotaEnabled:true}];s.sources=[{id:'source',provider:'kimi',name:'Kimi wallet',accountId:'wallet',enabled:true,path:'',subscription:{mode:'cn',credentialRef:'fixture-no-network'}}];s.hosts=[];
    for(const agent of s.agents){agent.enabled=agent.provider==='kimi';agent.machineIds=['local'];}await rpc('settings.save',s);
    db=new DatabaseSync(path.join(root,'data/aieyes.sqlite'));seed();
    assert.deepEqual((await rpc('dashboard')).quotas[0].extraUsage,initialWallet);
    const web=path.join(project,'apps/desktop/web'),output=path.join(project,'.local/kimi-wallet');fs.mkdirSync(output,{recursive:true});
    server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});await new Promise(r=>server.listen(0,'127.0.0.1',r));browser=await chromium.launch({headless:true});
    for(const surface of ['index.html','floating.html']){
      seed();const page=await browser.newPage({viewport:{width:surface==='floating.html'?450:1120,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(bridge);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
      const card=page.locator('[data-quota="kimi:wallet"]:visible');await card.waitFor();const wallet=card.locator('[data-extra-usage]');await wallet.getByText('USD 10.00',{exact:true}).waitFor();
      if(!await card.locator('.quota-disclosure').evaluate(el=>el.open))await card.locator('.quota-title').click();
      assert.match(await wallet.textContent(),/钱包总额USD 12.34/);assert.match(await wallet.textContent(),/本月扣费USD 1.23/);assert.match(await wallet.textContent(),/月度上限 USD 15.00/);
      assert.equal(await wallet.locator('.resource-bar').count(),0);assert.match(await card.textContent(),/剩余 75(?:\.0)?%/);
      await page.screenshot({animations:'disabled',path:path.join(output,surface+'-complete.png')});
      seed({...initialWallet,balance:'0.00',monthlyLimit:'0.00',monthlyUsed:'0.00'});await page.evaluate(()=>loadDashboard());await wallet.getByText('USD 0.00',{exact:true}).first().waitFor();assert.match(await wallet.textContent(),/月度上限 USD 0.00/);assert.doesNotMatch(await card.textContent(),/余额不足以调用 API/);assert.match(await card.textContent(),/剩余 75(?:\.0)?%/);
      seed({...initialWallet,monthlyLimitEnabled:false});await page.evaluate(()=>loadDashboard());await wallet.getByText('月度上限未启用',{exact:true}).waitFor();
      seed({currency:'USD',total:'12.34',balance:null,monthlyLimitEnabled:null,monthlyLimit:null,monthlyUsed:null},{metadataError:'部分钱包信息未提供'});await page.evaluate(()=>loadDashboard());await wallet.locator('strong').getByText('未提供',{exact:true}).waitFor();assert.match(await wallet.textContent(),/月度上限状态未提供/);assert.doesNotMatch(await wallet.textContent(),/0\.00/);assert.match(await card.textContent(),/部分钱包信息未提供/);
      seed({...initialWallet,total:'90071992.54740993',balance:'0.00000001'});await page.evaluate(()=>loadDashboard());await wallet.getByText('USD 0.00000001',{exact:true}).waitFor();assert.match(await wallet.textContent(),/90071992\.54740993/);
      await page.setViewportSize({width:360,height:900});await page.screenshot({animations:'disabled',path:path.join(output,surface+'-narrow.png')});assert(await wallet.evaluate(el=>el.scrollWidth<=el.clientWidth+1));
      seed(initialWallet,{windows:[]});await page.evaluate(()=>loadDashboard());await wallet.getByText('USD 10.00',{exact:true}).waitFor();assert.equal(await card.locator('.resource-bar').count(),0);
      seed(initialWallet,{error:'测试额度刷新失败'});await page.evaluate(()=>loadDashboard());await wallet.getByText('USD 10.00',{exact:true}).waitFor();assert.match(await card.textContent(),/额度更新失败|测试额度刷新失败/);
      seed(null);await page.evaluate(()=>loadDashboard());await page.waitForFunction(()=>!document.querySelector('[data-extra-usage]'));assert.match(await card.textContent(),/剩余 75(?:\.0)?%/);
      assert.deepEqual(errors,[]);await page.close();
    }
    console.log('Kimi wallet: real cache/dashboard, both surfaces, exact money, zero/unknown/disabled limits, wallet-only, failed refresh and narrow layout passed');
  } finally {
    if(browser)await browser.close();if(server)await new Promise(r=>server.close(r));if(db)db.close();core.kill();await new Promise(r=>core.exitCode!==null?r():core.once('exit',r));fs.rmSync(root,{recursive:true,force:true});
  }
})().catch(e=>{console.error(e);process.exitCode=1;});
