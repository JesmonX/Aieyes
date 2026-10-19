/* Native JSONL -> real core -> dashboard/UI. Disposable data, no client login. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process'),{chromium}=require('../apps/desktop/node_modules/playwright'),{serveFont}=require('./ui-test-helpers.cjs');
const provider=process.env.AIEYES_CREDITS_UI_PROVIDER||process.argv[2]||'qoder';assert(['qoder','codebuddy'].includes(provider));
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-native-credits-'));
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env:{...process.env,HOME:root,USERPROFILE:root,CODEX_HOME:path.join(root,'.codex')}});
const pending=new Map();let sequence=0,stderr='';core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const response=JSON.parse(line),p=pending.get(response.id);if(!p)return;pending.delete(response.id);response.error?p.reject(new Error(response.error.message)):p.resolve(response.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error('Core exited: '+stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params={}){
  if(['quotas.refresh','hosts.sample','wakeups.list'].includes(method))return [];
  if(method==='sessions.list')return {sessions:[],sources:[],warnings:[]};
  if(method.startsWith('network.'))return {status:'ok',sites:[],testedAt:Date.now()/1000};
  return rpc(method,params);
}
function bridge(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='engine_call')return window.testRPC(args.method,args.params??{});if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'window',effectiveMode:'window',sessions:[]};return {};}}};}
(async()=>{
  let browser,server;
  try {
    const folder=path.join(root,provider,'projects/project');fs.mkdirSync(folder,{recursive:true});
    const template=JSON.parse(fs.readFileSync(path.join(project,'crates/core/tests/fixtures/native-agents/qoder.jsonl'),'utf8').trim().split('\n').at(-1));
    const rows=[['one',.25,true],['two',.125,false],['three',.0625,null]].map(([id,credits,billable])=>{const row=structuredClone(template);row.timestamp=new Date().toISOString();row.message.id=id;row.message.model=id==='three'?'qfmodel-other':'qfmodel';row.message.content='PRIVATE_RESPONSE';Object.assign(row.message.usage,{credits,original_credits:credits*2,billable,extra:'PRIVATE_USAGE'});return row;});
    fs.writeFileSync(path.join(folder,'session.jsonl'),[...rows,rows[0],{type:'result',total_credits:999,usage:{credits:999}}].map(row=>JSON.stringify(row)).join('\n')+'\n');
    if(provider==='codebuddy'){
      const stamp=Date.now(),model=(id,messageId,session,credit,name,type='model-usage')=>({type,id,sessionId:session,timestamp:stamp,providerData:{messageId,model:name,conversationRequestId:session+'-turn',rawUsage:{credit}},content:'PRIVATE_RESPONSE'});
      const main=model('main-row','main-response','parent',.25,'qfmodel');
      const duplicate={...structuredClone(main),type:'function_call',id:'tool-split'};
      const generated={type:'credit-usage',id:'image-credit',sessionId:'parent',timestamp:stamp,providerData:{source:'generation_tool',stageId:'image:one',conversationRequestId:'parent-turn',credit:.125,extra:'PRIVATE_USAGE'}};
      const mirror={type:'credit-usage',id:'team-credit',sessionId:'parent',timestamp:stamp,providerData:{source:'team_member',stageId:'team:child-row',conversationRequestId:'parent-turn',credit:.0625}};
      fs.writeFileSync(path.join(folder,'session.jsonl'),[main,duplicate,generated,mirror,{type:'result',total_credits:999}].map(row=>JSON.stringify(row)).join('\n')+'\n');
      fs.writeFileSync(path.join(folder,'child.jsonl'),JSON.stringify(model('child-row','child-response','child',.0625,'qfmodel-other'))+'\n');
    }
    const settings=await rpc('settings.get');settings.sources=[{id:provider,provider,name:provider+' fixture',path:path.join(root,provider),enabled:true}];settings.accounts=[];settings.hosts=[];for(const agent of settings.agents){agent.enabled=agent.provider===provider;agent.machineIds=['local'];}await rpc('settings.save',settings);
    assert.equal((await rpc('sources.scan'))[0].result.newEvents,provider==='codebuddy'?4:3);
    assert.equal((await rpc('sources.scan'))[0].result.newEvents,0);
    const d=await rpc('dashboard');assert.equal(d.summary.total,0);assert.equal(d.summary.nativeCredits[provider].credits,.4375);assert.equal(d.summary.nativeCredits[provider].requests,3);assert(!JSON.stringify(d).includes('PRIVATE'));
    const web=path.join(project,'apps/desktop/web'),out=path.join(project,'.local/native-credits',provider);fs.mkdirSync(out,{recursive:true});
    server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(bridge);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    await page.locator('[data-native-credits]').getByText(provider==='codebuddy'?/0.4375 Credits · 3 条计量/:/0.4375 Credits · 3 次请求/).waitFor();
    assert.match(await page.locator('.usage-inline').textContent(),/API 等价成本\s*—/);
    await page.screenshot({path:path.join(out,'overview.png')});
    await page.setViewportSize({width:450,height:900});
    assert(await page.locator('[data-native-credits]').isVisible());
    await page.screenshot({path:path.join(out,'overview-narrow.png')});
    await page.setViewportSize({width:1180,height:900});
    await page.locator('#open-detail').click();await page.waitForFunction(()=>state.page==='analysis'&&!state.dashboardPending);
    const credits=page.locator('[data-native-credits]');if(provider==='codebuddy'){await credits.getByText(/计费状态未提供 0.4375（3 次）/).waitFor();assert(!/标记可计费|标记不计费|折扣前已记录/.test(await credits.textContent()));}else{await credits.getByText(/标记可计费 0.25（1 次）/).waitFor();assert.match(await credits.textContent(),/标记不计费 0.125（1 次）/);assert.match(await credits.textContent(),/计费状态未提供 0.0625（1 次）/);assert.match(await credits.textContent(),/折扣前已记录 0.875（3\/3 次请求）/);}
    assert.equal(await page.locator('.empty-usage').count(),0);
    await credits.locator('summary').click();assert.match(await credits.textContent(),/qfmodel-other/);
    await page.locator('details.daily > summary').click();await page.locator('[data-day-toggle]').first().click();assert.match(await page.locator('.daily-table').textContent(),/原生记录/);
    await page.evaluate(()=>window.scrollTo(0,0));await page.screenshot({path:path.join(out,'analysis.png'),fullPage:true});
    await page.locator('#model').selectOption('qfmodel-other');await page.waitForFunction(()=>state.appliedFilters.model==='qfmodel-other'&&!state.dashboardPending);assert.equal(await page.evaluate(provider=>state.dashboard.summary.nativeCredits[provider].credits,provider),.0625);
    await page.locator('#provider').selectOption('claude');await page.waitForFunction(()=>state.appliedFilters.provider==='claude'&&!state.dashboardPending);assert.equal(await page.locator('[data-native-credits]').count(),0);
    assert.deepEqual(errors,[]);console.log(provider+' native Credits: deduplication, cumulative-result exclusion, zero Token display, daily/model details and filters passed');
  } finally {
    if(browser)await browser.close();if(server)await new Promise(resolve=>server.close(resolve));core.kill();await new Promise(resolve=>core.exitCode!==null?resolve():core.once('exit',resolve));fs.rmSync(root,{recursive:true,force:true});
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
