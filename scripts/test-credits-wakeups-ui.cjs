const {discardEditor}=require('./ui-test-helpers.cjs');
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
 window.setInterval=()=>0;
 const now=Date.now()/1000,summary={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
 const q={provider:'codex',accountId:'a',sourceId:'s',name:'订阅账户',plan:'pro',updatedAt:now,origin:'live',windows:[],credits:{hasCredits:true,unlimited:false,balance:'1234.125'},creditsUpdatedAt:now,bankReset:{availableCount:2,credits:[]}};
 const settings={accounts:[{id:'a',provider:'codex',name:'订阅账户',quotaEnabled:true}],sources:[{id:'s',provider:'codex',accountId:'a',name:'本机订阅',path:'/fixture',enabled:true}],hosts:[],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,serverRefreshSeconds:10};
 window.testFeature={records:[],tasks:[],calls:[],failRemoval:false};
 window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
  if(command==='desktop_info')return {platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:true};if(command!=='engine_call')return {};
  const f=window.testFeature,{method,params}=args;f.calls.push({method,params});
  if(method==='hello')return {version:'test'};if(method==='settings.get')return settings;
  if(method==='dashboard')return {generatedAt:now,summary,quotas:[q],quotaOrder:['codex:a'],quotaEstimates:[],creditEstimates:f.records,modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
  if(method==='sources.scan'||method==='prices.list')return [];if(method==='quotas.refresh')return [q];
  if(method==='creditEstimates.start'){f.records=[{id:'credit-test',kind:'credits',accountKey:'codex:a',windowId:'credits',windowName:'Credits',sourceIds:['s'],sourceNames:['本机'],status:'active',reason:'',startedAt:now-60,checkpointAt:now,consumedPercent:0,consumedCredits:10,cost:2,totalTokens:100,pricedTokens:100,valuePer500:100,valuePer1000:200,calculationNote:'手动采样估值',prices:[]}];return f.records[0];}
  if(method==='creditEstimates.stop'){f.records[0].status='completed';return f.records[0];}
  if(method==='wakeups.list')return structuredClone(f.tasks);
  if(method==='wakeups.probe')return {models:[{id:'cheap-model',efforts:['low','medium'],referenceCost:0.00015}],recommendedModel:'cheap-model',efforts:['low','medium'],timezone:'CST +0800',message:'按 1000 输入 + 100 输出 Token 推荐'};
  if(method==='wakeups.save'){let r={task:{...params.task,id:params.task.id||'wake-test'},deployment:null};const i=f.tasks.findIndex(r=>r.task.id===params.task.id);if(i>=0){r.deployment=f.tasks[i].deployment;f.tasks[i]=r;}else f.tasks.push(r);return r;}
  if(method==='wakeups.deploy'){f.tasks[0].deployment={task:f.tasks[0].task,deployedAt:now,enabled:true,state:'deployed',timezone:'CST +0800',target:'本机',changed:false};return f.tasks[0];}
  if(method==='wakeups.remove'){if(f.failRemoval){f.tasks[0].deployment.state='pending-removal';throw Error('服务器不可达，自动任务尚未确认移除');}f.tasks[0].deployment=null;return f.tasks[0];}
  if(method==='wakeups.disable'||method==='wakeups.enable'){f.tasks[0].deployment.enabled=method.endsWith('.enable');return f.tasks[0];}
  if(method==='wakeups.status')return {installed:true,enabled:true,timezone:'CST +0800',nextRunAt:now+1000,history:[{startedAt:now,endedAt:now+1,status:'success',model:'cheap-model',effort:'low'}]};
  if(method==='wakeups.run')return {started:true};
  return {};
 }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404);return res.end();}res.setHeader('content-type',({'.js':'application/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{browser=await chromium.launch({headless:true});
 for(const surface of ['index.html','floating.html']){
  const page=await browser.newPage({viewport:{width:surface==='index.html'?1120:450,height:850}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
  assert.equal(await page.getByText('1234.13 credits',{exact:true}).count(),1);assert.equal(await page.getByText('Bank Reset · 2 次可用',{exact:true}).count(),1);
  const card=()=>page.locator('#content .quota-card');
  assert.match(await card().locator('[data-credit-estimate]').textContent(),/^credits估值（当前1000credits≈—）$/);
  const expandedBalanceBox=await card().locator('.credit-row').boundingBox(),entryBox=await card().locator('[data-credit-estimate]').boundingBox();assert(entryBox.y>=expandedBalanceBox.y+expandedBalanceBox.height,'Credits estimate is on its own line below the balance');
  const decimals=[['1234.125','1234.13'],['1.005','1.01'],['10','10.00'],['0','0.00'],['-0.004','0.00'],['-1.005','-1.01'],['1e-3','0.00'],['9.999','10.00'],[null,'—'],['','—'],['NaN','—'],['12bad','—'],['Infinity','—']];
  for(const [input,expected] of decimals)assert.equal(await page.evaluate(v=>creditAmount(v),input),expected);
  const thresholds=[[100,'accent'],[30.01,'accent'],[30,'resource-warn'],[10,'resource-warn'],[9.99,'resource-high'],[0,'resource-high'],[null,'muted']];
  for(const [remaining,tint] of thresholds){
   const result=await page.evaluate(value=>{
    const holder=document.createElement('div');holder.innerHTML=resourceBar(value,'7d 剩余额度',true);
    return {color:holder.querySelector('span').style.background,amount:holder.firstChild.getAttribute('aria-valuenow'),text:quotaColor(value)};
   },remaining);
   assert.equal(result.color,'var(--'+tint+')');assert.equal(result.amount,remaining===null?null:String(remaining));
  }
  assert.equal(await page.evaluate(()=>resourceColor(95)),'var(--resource-high)','High CPU load must still be red');
  assert.equal(await page.evaluate(()=>creditValue({status:'pending',valuePer1000:200})),'待确认');
  if(await card().locator('.quota-disclosure').evaluate(el=>el.open))await card().locator('.quota-disclosure summary').click();
  await page.evaluate(()=>loadDashboard());
  assert.equal(await card().locator('.quota-disclosure').evaluate(el=>el.open),false,'Single account preserves its collapsed summary after refresh');assert(!await card().locator('.credit-row').isVisible());
  assert(await card().locator('.quota-credit-summary').isVisible());
  const titleBox=await card().locator('.quota-title h3').boundingBox(),balanceBox=await card().locator('.quota-credit-summary').boundingBox();
  assert(balanceBox.x>=titleBox.x+titleBox.width-1 && Math.abs((balanceBox.y+balanceBox.height/2)-(titleBox.y+titleBox.height/2))<3,'Collapsed credits stay to the right of the title on the same line');
  assert.equal(await card().locator('.quota-credit-summary .credit-label').evaluate(el=>getComputedStyle(el).color),await page.evaluate(()=>{const probe=document.createElement('span');probe.style.color='var(--accent)';document.body.append(probe);const color=getComputedStyle(probe).color;probe.remove();return color;}));
  assert.equal(await card().locator('.quota-title .provider-mark img').evaluate(img=>img.complete&&img.naturalWidth>0),true,'Provider icon is bundled and loads');
  assert.equal(await card().locator('.subscription-badge').textContent(),'Pro');
  const edgeCases=await page.evaluate(()=>{
    const holder=document.createElement('div');holder.style.width='414px';document.body.append(holder);
    const cases=[{hasCredits:true,unlimited:false,balance:'0'},{hasCredits:true,unlimited:true},{hasCredits:true,unlimited:false},{hasCredits:false,unlimited:false},{hasCredits:true,unlimited:false,balance:'123456789.125'}];
    try { return cases.map(credits=>{
      holder.innerHTML=quotaCard({...state.dashboard.quotas[0],name:'非常长的订阅账户名称用于检查折叠标题和余额在同一行',credits});
      const title=holder.querySelector('.quota-title'),label=holder.querySelector('.quota-credit-summary'),value=label.querySelector('strong'),a=title.getBoundingClientRect(),b=label.getBoundingClientRect();
      return {text:value.textContent,sameLine:b.top>=a.top&&b.bottom<=a.bottom,withinCard:b.right<=a.right};
    }); } finally {holder.remove();}
  });
  assert.deepEqual(edgeCases.map(c=>c.text),['0.00','无限','数量未知','—','123456789.13']);assert(edgeCases.every(c=>c.sameLine&&c.withinCard));
  const foldedPreviews=path.resolve(__dirname,'../.local/ui-previews');fs.mkdirSync(foldedPreviews,{recursive:true});
  for(const colorScheme of ['light','dark']){await page.emulateMedia({colorScheme});await page.screenshot({animations:'disabled',path:path.join(foldedPreviews,'credit-folded-'+surface+'-'+colorScheme+'.png')});}
  await page.emulateMedia({colorScheme:'light'});
  await card().locator('.quota-disclosure summary').click();
  await page.locator('[data-credit-estimate]:visible').click();await page.locator('#estimate-confirm').check();await page.locator('#estimate-start').click();await page.locator('#estimate-stop').waitFor();assert.match(await page.locator('#quota-dialog').textContent(),/1000 credit ≈ \$200.00 USD/);assert.doesNotMatch(await page.locator('#quota-dialog').textContent(),/500 credits/);await page.locator('#estimate-stop').click();await page.locator('#estimate-start').waitFor();await page.locator('#quota-close').click();
  await page.waitForFunction(()=>document.querySelector('#content .credit-row')?.textContent.includes('≈'));assert.match(await card().locator('.credit-row').textContent(),/余额 1234.13 credits ≈ \$246.83 USD/);
  const previews=path.resolve(__dirname,'../.local/ui-previews');fs.mkdirSync(previews,{recursive:true});
  for(const colorScheme of ['light','dark']){
   await page.emulateMedia({colorScheme});await page.evaluate(()=>{document.querySelector('.panel-body')?.scrollTo(0,0);window.scrollTo(0,0);});
   await page.screenshot({animations:'disabled',path:path.join(previews,'credit-'+surface+'-'+colorScheme+'.png')});
  }
  await page.emulateMedia({colorScheme:'light'});
  if(surface==='floating.html'){
   await page.evaluate(()=>{state.days=30;state.cost=true;render();});
   await page.locator('#content .quota-disclosure summary').click();
   await page.reload();await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
   assert.equal(await page.evaluate(()=>state.days),30);assert.equal(await page.evaluate(()=>state.cost),true);
   assert.equal(await page.locator('.quota-disclosure').evaluate(el=>el.open),false,'Panel reconstruction restores the collapsed summary');
   assert.equal(await page.evaluate(()=>testFeature.calls.some(c=>c.method==='creditEstimates.start')),false,'Recreating the panel does not start another sample');
  }
  if(surface==='index.html'){
   await page.evaluate(()=>{state.page='settings';state.settingsTab='wakeups';renderSettings();});await page.locator('#wake-add:not([disabled])').waitFor();await page.locator('#wake-add').click();await page.locator('#wake-probe').click();await page.waitForFunction(()=>document.querySelector('#wake-model')?.value==='cheap-model');assert.equal(await page.locator('#wake-effort').inputValue(),'low');await page.locator('#wake-add-time').click();await page.locator('[data-wake-time="1"]').fill('13:30');await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
   assert.equal(await page.evaluate(()=>testFeature.calls.filter(c=>c.method==='wakeups.deploy').length),0,'saving does not deploy');await page.locator('[data-wake-action=deploy]').click();await page.locator('.wake-more summary').click();await page.locator('[data-wake-action=disable]').waitFor();await page.locator('[data-wake-action=disable]').click();await page.locator('.wake-more summary').click();await page.locator('[data-wake-action=enable]').waitFor();await page.locator('[data-wake-action=enable]').click();await page.locator('[data-wake-action=status]').click();await page.locator('#quota-dialog').getByText('已完成',{exact:true}).waitFor();await page.locator('#quota-close').click();
   await page.evaluate(()=>testFeature.failRemoval=true);await page.locator('.wake-more summary').click();await page.locator('[data-wake-action=remove]').click();await page.locator('#editor-form button[type=submit]').click();await page.getByText('待移除',{exact:true}).waitFor();assert.match(await page.locator('#content').textContent(),/尚未确认移除/);await discardEditor(page);await page.evaluate(()=>testFeature.failRemoval=false);await page.locator('.wake-more summary').click();await page.locator('[data-wake-action=remove]').click();await page.locator('#editor-form button[type=submit]').click();await page.locator('[data-wake-action=delete]').waitFor({state:'attached'});
   fs.mkdirSync(path.resolve(__dirname,'../.local/wakeup-review'),{recursive:true});await page.screenshot({path:path.resolve(__dirname,'../.local/wakeup-review/desktop.png')});
  }
  assert.deepEqual(errors,[]);await page.close();
 }
 console.log('Credits and wakeup UI: both surfaces, sampling, recommended model/effort, draft/deploy separation, enable/disable, history and failed removal passed');
 }finally{await browser?.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
