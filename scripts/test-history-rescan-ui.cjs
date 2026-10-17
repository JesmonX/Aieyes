/* Manual history replay uses the real core and an isolated historical database. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn,execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont}=require('./ui-test-helpers.cjs');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-rescan-'));
const log=path.join(root,'history.jsonl');
fs.writeFileSync(log,JSON.stringify({id:'call',sessionId:'session',timestamp:Math.floor(Date.now()/1000),model:'fixture',tokens:{input:100}})+'\n');
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',root],{stdio:['pipe','pipe','pipe']});
let sequence=0,stderr='',releaseRescan,holdRescan=false;
const pending=new Map(),calls=[];
core.stderr.on('data',buffer=>stderr+=buffer);
readline.createInterface({input:core.stdout}).on('line',line=>{const response=JSON.parse(line),request=pending.get(response.id);if(!request)return;pending.delete(response.id);if(response.error)request.reject(new Error(response.error.message));else request.resolve(response.result);});
core.on('exit',()=>{for(const request of pending.values())request.reject(new Error('Core exited: '+stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params){
  calls.push({method,params});
  if(method==='sources.rescan'&&holdRescan)await new Promise(resolve=>releaseRescan=resolve);
  if(['quotas.refresh','hosts.sample','accounts.status.get','accounts.status.refresh'].includes(method))return [];
  return rpc(method,params);
}
function browserFixture(){
  window.setInterval=()=>0;
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[]};
    if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};
  }}};
}
(async()=>{
  let server,browser;
  try {
    await rpc('settings.save',{version:6,sources:[{id:'fixture',name:'测试历史',provider:'antigravity',accountId:'account',path:log,enabled:true}],accounts:[{id:'account',provider:'antigravity',name:'测试账户',quotaEnabled:false}],hosts:[],localMonitor:{enabled:false},modelMappings:{}});
    await rpc('prices.save',{prices:[{id:'fixture',input:0.01}]});
    await rpc('sources.scan');
    execFileSync(process.platform==='win32'?'python':'python3',['-c',`
import sqlite3,json,hashlib,sys
db=sqlite3.connect(sys.argv[1]); row=db.execute('SELECT * FROM events').fetchone()
event=json.loads(row[5]); identity=event.pop('importIdentity')
db.execute('UPDATE events SET payload=? WHERE id=?',(json.dumps(event),row[0]))
event['id']=hashlib.sha256(('agy:account:'+identity).encode()).hexdigest()
db.execute('INSERT INTO events VALUES(?,?,?,?,?,?,?,?,?)',(event['id'],*row[1:5],json.dumps(event),*row[6:]))
db.execute('INSERT INTO event_sources VALUES(?,?)',(event['id'],'fixture'));db.commit();db.close()
`,path.join(root,'aieyes.sqlite')]);
    const web=path.join(project,'apps/desktop/web');
    server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];
    page.on('pageerror',error=>errors.push(String(error)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(browserFixture);
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.equal(await page.evaluate(()=>state.dashboard.summary.total),200);
    assert.equal(calls.filter(call=>call.method==='sources.rescan').length,0);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='general';state.generalTab='refresh';render();});
    assert.equal(await page.locator('#field-codexPreciseCounting').isChecked(),false);
    await page.locator('#field-codexPreciseCounting').check();
    await page.waitForFunction(()=>state.settings.codexPreciseCounting===true&&!state.settingsCommitting);
    assert.equal((await rpc('settings.get')).codexPreciseCounting,true);
    await page.locator('#field-codexPreciseCounting').uncheck();
    await page.waitForFunction(()=>state.settings.codexPreciseCounting===false&&!state.settingsCommitting);
    await page.locator('#history-rescan').click();await page.locator('#cancel-editor').click();
    assert.equal(calls.filter(call=>call.method==='sources.rescan').length,0,'Cancelling must not replay history');
    holdRescan=true;await page.locator('#history-rescan').click();await page.getByRole('button',{name:'开始重扫',exact:true}).click();
    await page.waitForFunction(()=>state.rescanning&&state.busy);
    assert(await page.locator('#history-rescan').isDisabled());
    assert.equal(await page.locator('#history-rescan').textContent(),'全量重扫中…');
    assert.equal(calls.filter(call=>call.method==='sources.rescan').length,1);
    holdRescan=false;releaseRescan();
    await page.waitForFunction(()=>!state.rescanning&&!state.busy&&state.dashboard.summary.total===100);
    assert.match(await page.locator('#history-rescan-status').textContent(),/全量重扫完成/);
    assert.equal((await rpc('dashboard')).summary.cost,1,'Frozen prices survive replay');
    fs.mkdirSync(path.join(project,'.local'),{recursive:true});await page.screenshot({path:path.join(project,'.local/manual-rescan-ui.png'),fullPage:true});
    const completedAt=await page.evaluate(()=>state.refreshState.rescan.success);
    await page.locator('#history-rescan').click();await page.getByRole('button',{name:'开始重扫',exact:true}).click();
    await page.waitForFunction(previous=>!state.rescanning&&!state.busy&&state.dashboard.summary.total===100&&state.refreshState.rescan.success>previous,completedAt);
    fs.unlinkSync(log);
    await page.locator('#history-rescan').click();await page.getByRole('button',{name:'开始重扫',exact:true}).click();
    await page.waitForFunction(()=>state.rescanFailed&&!state.rescanning&&!state.busy);
    assert.match(await page.locator('#history-rescan-status').textContent(),/测试历史/);
    assert.equal((await rpc('dashboard')).summary.total,100,'Missing logs must preserve stored history');
    fs.mkdirSync(path.join(root,'codex'));const forkDir=fs.realpathSync(path.join(root,'codex'));
    const stamp=Math.floor(Date.now()/1000)-10;
    fs.writeFileSync(path.join(forkDir,'child.jsonl'),[
      {type:'session_meta',timestamp:stamp-1,payload:{id:'child',history_base:{thread_id:'missing-parent',end_byte_offset:999999}}},
      {type:'event_msg',timestamp:stamp,payload:{type:'token_count',info:{total_token_usage:{input_tokens:1000},last_token_usage:{input_tokens:100}}}}
    ].map(JSON.stringify).join('\n')+'\n');
    const beforeFork=await rpc('settings.get'),forkSettings=structuredClone(beforeFork);
    forkSettings.sources=[{id:'fork',name:'Fork 测试',provider:'codex',accountId:'',path:forkDir,enabled:true}];
    const codexAgent=forkSettings.agents.find(agent=>agent.provider==='codex');codexAgent.enabled=true;codexAgent.machineIds=['local'];
    await rpc('settings.patch',{base:beforeFork,settings:forkSettings});
    await page.reload();await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    await page.evaluate(async()=>{await scan();state.page='settings';state.settingsTab='general';state.generalTab='refresh';render();});
    assert.equal((await rpc('dashboard')).summary.total,1100,'Default mode includes inherited cumulative usage alongside retained historical data');
    assert.equal(await page.evaluate(()=>state.refreshState.scan.error),null,JSON.stringify(await page.evaluate(()=>state.refreshState.scan.failures)));
    await page.locator('#field-codexPreciseCounting').check();await page.waitForFunction(()=>state.settings.codexPreciseCounting===true&&!state.settingsCommitting);
    await page.evaluate(async()=>{await scan();render();});
    assert.equal((await rpc('dashboard')).summary.total,100,'Precise mode skips the unresolved fork prefix and retains unrelated history');
    await page.getByRole('heading',{name:'精确计数详情',exact:true}).waitFor();
    assert.equal(await page.evaluate(()=>state.refreshState.scan.error),null,'Fork diagnostics do not create sync failures');
    await page.evaluate(()=>{state.page='agent';render();});
    assert.equal(await page.getByText('父历史待确认',{exact:false}).count(),0,'Fork diagnostics stay inside settings');
    assert.deepEqual(errors,[]);
    console.log('Manual full rescan and precise counting: autosave, real fork mode replay, settings-only diagnostics, frozen price, repeated replay and missing-log feedback passed');
  } finally {
    releaseRescan?.();if(browser)await browser.close();if(server)await new Promise(resolve=>server.close(resolve));
    core.stdin.end();if(core.exitCode===null)await new Promise(resolve=>core.once('exit',resolve));fs.rmSync(root,{recursive:true,force:true});
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
