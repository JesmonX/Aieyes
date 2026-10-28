/* Fresh-install race: another window's operation invalidates shared data while this window is still booting.
   The shared refresh then fetches settings before boot assigns them, and must not report an error. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-boot-')),home=path.join(root,'home');fs.mkdirSync(home);
// Empty data directory and home: the real core starts as a fresh install, without the developer's agent data.
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env:{...process.env,HOME:home,USERPROFILE:home,CLAUDE_CONFIG_DIR:path.join(home,'.claude'),CODEX_HOME:path.join(home,'.codex')}});
// `kill()` requests termination; Windows can retain SQLite/credential handles
// until the process and its stdio have actually closed.
const coreClosed=new Promise(resolve=>core.once('close',resolve));
let counter=0,stderr='',settingsReads=0;const pending=new Map();core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);if(r.error)p.reject(new Error(r.error.message));else p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error('Core exited: '+stderr));pending.clear();});
const rpc=(method,params={})=>new Promise((resolve,reject)=>{const id=++counter;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});
async function invoke(method,params){
  // Hold boot's settings read so that the invalidation arrives first.
  if(method==='settings.get'&&++settingsReads===1)await new Promise(resolve=>setTimeout(resolve,1500));
  if(['quotas.refresh','sources.scan','hosts.sample'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  return rpc(method,params);
}
function browserFixture(panel){
  window.setInterval=()=>0;window.testListeners={};
  window.__TAURI__={event:{async listen(name,handler){(window.testListeners[name]??=[]).push(handler);return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:panel,nativePet:true};
    if(command==='desktop_panel')return {panelOpen:args.open,nativePet:true};
    if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};
  }}};
}
(async()=>{
  const web=path.join(project,'apps/desktop/web');
  const server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');fs.createReadStream(file).pipe(res);});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try {
    browser=await chromium.launch({headless:true});
    for(const [surface,panel] of [['index.html',false],['floating.html',true]]){
      settingsReads=0;
      const page=await browser.newPage({viewport:panel?{width:450,height:720}:{width:1180,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));
      await page.exposeFunction('testRPC',invoke);await page.addInitScript(browserFixture,panel);
      await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);
      await page.waitForFunction(()=>typeof state!=='undefined'&&state.revisionEvents);
      assert.equal(await page.evaluate(()=>state.settings),null,`${surface}: the invalidation arrives before boot assigns settings`);
      await page.evaluate(()=>{for(const handler of window.testListeners['desktop:invalidate']??[])handler({payload:{method:'settings.patch',operationId:'other-window'}});});
      await page.waitForFunction(()=>state.settings&&state.dashboard);await page.waitForTimeout(500);
      assert.ok(settingsReads>=2,`${surface}: the shared refresh read settings during boot`);
      assert.deepEqual(errors,[],`${surface}: no page errors`);
      const message=await page.locator('#message').textContent();
      assert.doesNotMatch(message,/TypeError|操作未完成/,`${surface}: the early shared refresh reports no error`);
      await page.close();
    }
    console.log('Boot refresh race UI checks passed');
  } finally {
    await browser?.close();
    await new Promise(resolve=>server.close(resolve));
    core.kill();
    await coreClosed;
    fs.rmSync(root,{recursive:true,force:true,maxRetries:10,retryDelay:100});
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
