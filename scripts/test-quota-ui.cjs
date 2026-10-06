const {discardEditor}=require('./ui-test-helpers.cjs');
// End-to-end quota UI flows on both desktop surfaces, with isolated in-memory IPC.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.setInterval=()=>0;
  const now=Math.floor(Date.now()/1000),tokens={input:0,output:0,cacheRead:0,cacheWrite:0};
  const summary={tokens,total:0,cost:0,pricedTokens:0,events:0};
  const win=(id,name,minutes,groupName)=>({id,name,windowMinutes:minutes,usedPercent:35,resetsAt:now+10000,groupName});
  const rows=[{provider:'codex',accountId:'c',name:'个人订阅',sourceId:'c',updatedAt:now,origin:'live',windows:[win('weekly','7d',10080)]},{provider:'agy',accountId:'a',name:'工作空间 agy',sourceId:'a',updatedAt:now,origin:'live',windows:['Claude Opus','Claude Sonnet','Gemini Pro'].flatMap(g=>[win(g+':5h',g+' · 5h',300,g),win(g+':7d',g+' · 7d',10080,g)])}];
  const settings={accounts:rows.map(q=>({id:q.accountId,name:q.name,provider:q.provider,quotaEnabled:true})),sources:rows.map(q=>({id:q.accountId,name:q.name,provider:q.provider,accountId:q.accountId,enabled:true,path:'/fixture'})),hosts:[],modelMappings:{},proxy:{mode:'system',url:''},refreshSeconds:300,serverRefreshSeconds:10};
  window.quotaFixture={rows,settings,order:['codex:c','agy:a'],records:[],calls:[],fail:false,panelOpen:true};
  const info=()=>({platform:'windows',material:'opaque',mode:'floating',effectiveMode:'floating',sessions:[],summary:'暂无活跃会话',panelOpen:window.quotaFixture.panelOpen,panelPinned:true,hidden:false});
  window.__TAURI__={event:{async listen(){return ()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return ()=>{};}})},core:{async invoke(command,args){
    const f=window.quotaFixture;if(command==='desktop_info')return info();if(command==='desktop_panel'){f.panelOpen=args.open;return info();}if(command==='desktop_panel_cursor_inside')return true;if(command!=='engine_call')return {};
    const {method,params}=args;f.calls.push({method,params});
    if(method==='hello')return {version:'test'};if(method==='settings.get')return structuredClone(settings);
    if(method==='dashboard')return {generatedAt:now,summary,quotas:[...rows].sort((a,b)=>f.order.indexOf(a.provider+':'+a.accountId)-f.order.indexOf(b.provider+':'+b.accountId)),quotaOrder:f.order,quotaEstimates:f.records,modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
    if(method==='sources.scan')return [];if(method==='quotas.refresh')return rows;
    if(method==='quotas.order.set'){if(f.fail)throw new Error('保存失败，请重试');f.order=params.keys;return f.order;}
    if(method.startsWith('quotaEstimates.')){
      if(f.fail)throw new Error('测试同步失败，采样已保留');
      if(method.endsWith('start')||method.endsWith('restart')){if(!params.confirmed)throw new Error('请确认');if(method.endsWith('.start')&&!params.sourceIds.length)throw new Error('请选择用量数据源');f.records=[{id:'sample',accountKey:'codex:c',windowId:'weekly',windowName:'7d',sourceIds:['c'],sourceNames:['本机'],status:'active',reason:'',startedAt:now-60,checkpointAt:now,consumedPercent:6,cost:1.2,totalTokens:100,pricedTokens:100,weeklyValue:20,calculationNote:'手动采样估值',prices:[]}];}
      if(method.endsWith('.stop'))f.records[0].status='completed';return f.records[0];
    }
    return [];
  }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),output=path.resolve(__dirname,'../.local/quota-review/web');fs.mkdirSync(output,{recursive:true});
 const server=http.createServer((req,res)=>{const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}res.setHeader('content-type',({'.js':'application/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{
  browser=await chromium.launch({headless:true});
  for(const surface of ['index.html','floating.html']){
   const page=await browser.newPage({viewport:{width:surface==='floating.html'?450:1120,height:760}}),errors=[];
   page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
   const agy=page.locator('[data-quota="agy:a"]:visible');await agy.waitFor({state:'visible'});
   assert.equal(await agy.locator('.agy-group').count(),3);
   assert.equal(await agy.locator('.resource-bar:visible').count(),6,'single account shows all model groups');
   assert.equal(await agy.locator('.quota-reset:visible').count(),6);
   await agy.locator('.quota-title').click();assert.equal(await agy.locator('.quota-reset:visible').count(),6,'single account cannot collapse');
   await page.locator('#order-quotas').click();await page.locator('[data-order="agy:a"] [data-direction="-1"]').click();
   await page.locator('[data-order="codex:c"]').dragTo(page.locator('[data-order="agy:a"]'));
   await page.locator('[data-order="agy:a"]').dragTo(page.locator('[data-order="codex:c"]'));
   await page.evaluate(()=>quotaFixture.fail=true);await page.locator('#quota-order-save').click();await page.waitForFunction(()=>document.querySelector('#quota-tool-error').textContent.includes('保存失败'));
   assert(await page.locator('#quota-dialog').evaluate(e=>e.open));await page.evaluate(()=>quotaFixture.fail=false);await page.locator('#quota-order-save').click();await page.waitForFunction(()=>!document.querySelector('#quota-dialog').open);
   assert.equal(await page.locator('[data-quota]').first().getAttribute('data-quota'),'agy:a');
   if(!await page.locator('[data-quota="codex:c"]:visible .quota-disclosure').evaluate(el=>el.open))await page.locator('[data-quota="codex:c"]:visible .quota-disclosure summary').click();
   await page.locator('[data-estimate="codex:c"]:visible').click();assert(await page.locator('#estimate-start').isDisabled());await page.locator('#estimate-confirm').check();
   await page.locator('[name=estimate-source]').uncheck();await page.locator('#estimate-start').click();await page.waitForFunction(()=>document.querySelector('#quota-tool-error').textContent.includes('数据源'));
   await page.locator('[name=estimate-source]').check();await page.locator('#estimate-start').click();await page.locator('#estimate-stop').waitFor();assert.equal(await page.locator('#quota-dialog').getByText('$20.00 USD',{exact:true}).count(),1);
   await page.locator('#quota-dialog details summary').click();await page.screenshot({path:path.join(output,surface+'-estimate.png')});
   await page.keyboard.press('Escape');await page.waitForFunction(()=>!document.querySelector('#quota-dialog').open);if(surface==='floating.html')assert.equal(await page.evaluate(()=>quotaFixture.panelOpen),true,'Escape does not close underlying panel');
   await page.locator('[data-estimate="codex:c"]:visible').click();
   await page.evaluate(async()=>{quotaFixture.records[0].status='pending';quotaFixture.records[0].reason='检测到提前重置';await loadDashboard();});
   await page.locator('#estimate-restart').waitFor();assert(await page.locator('#estimate-restart').isDisabled());await page.locator('#estimate-confirm').check();await page.locator('#estimate-restart').click();await page.locator('#estimate-stop').waitFor();
   await page.locator('#estimate-stop').click();await page.locator('#estimate-start').waitFor();assert.equal(await page.locator('#quota-dialog').getByText('已结束',{exact:true}).count(),1);
   await page.locator('#quota-close').click();
   for(const colorScheme of ['light','dark']){await page.emulateMedia({colorScheme});await page.screenshot({path:path.join(output,surface+'-'+colorScheme+'.png')});}
   assert.deepEqual(errors,[]);await page.close();
  }
  console.log('Quota UI: agy compact/expanded, ordering retry, manual start/stop, live reset confirmation/restart, source validation, history, focus and panel Escape passed on both surfaces');
 }finally{await browser?.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
