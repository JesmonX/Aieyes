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
  if(method==='accounts.targets')return f.targets??[{accountKey:'codex:a',accountName:'订阅账户',provider:'codex',sourceId:'s',sourceName:'本机订阅',machineId:'local',machineName:'本机',path:'/fixture',profileId:null,canWake:true}];
  if(method==='codexAuth.reconcile')return {settings,issues:[]};
  if(method==='wakeups.list')return structuredClone(f.tasks);
  if(method==='wakeups.suggest')return f.suggestion??{typicalStart:'09:10',suggested:'06:15',resetAt:'11:15',days:12,timezone:'Asia/Shanghai',excludedSessions:1};
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
 const root=path.resolve(__dirname,'../apps/desktop/web'),server=http.createServer(require('./ui-test-helpers.cjs').serveWebRoot(root));
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{browser=await chromium.launch({headless:true});
 for(const surface of ['index.html','floating.html']){
  const page=await browser.newPage({viewport:{width:surface==='index.html'?1120:450,height:850}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
  assert.equal(await page.getByText('1234.13 credits',{exact:true}).count(),1);assert.equal(await page.locator('.bank .bank-count').filter({hasText:'2 次可用'}).count(),1);
  const card=()=>page.locator('#content .quota-card');
  await card().locator('.quota-title').click();
  assert.equal(await card().locator('[data-credit-estimate]').textContent(),'credits 估值');
  assert.equal(await card().locator('.bank summary .ui-icon').count(),1);
  assert(await card().evaluate(card=>!!(card.querySelector('.credit-row').compareDocumentPosition(card.querySelector('.bank'))&Node.DOCUMENT_POSITION_FOLLOWING)));
  const expandedBalanceBox=await card().locator('.credit-row').boundingBox(),entryBox=await card().locator('[data-credit-estimate]').boundingBox();assert(entryBox.y>=expandedBalanceBox.y+expandedBalanceBox.height,'Credits estimate is on its own line below the balance');
  const decimals=[['1234.125','1234.13'],['1.005','1.01'],['10','10.00'],['0','0.00'],['-0.004','0.00'],['-1.005','-1.01'],['1e-3','0.00'],['9.999','10.00'],[null,'—'],['','—'],['NaN','—'],['12bad','—'],['Infinity','—']];
  for(const [input,expected] of decimals)assert.equal(await page.evaluate(v=>creditAmount(v),input),expected);
  const thresholds=[[100,'category'],[30.01,'category'],[30,'resource-warn'],[10,'resource-warn'],[9.99,'resource-high'],[0,'resource-high'],[null,'muted']];
  for(const [remaining,tint] of thresholds){
   const result=await page.evaluate(value=>{
    const holder=document.createElement('div');holder.innerHTML=resourceBar(value,'7d 剩余额度',true);
    return {color:holder.querySelector('span').style.background,amount:holder.firstChild.getAttribute('aria-valuenow'),text:quotaColor(value)};
   },remaining);
   assert.equal(result.color,'var(--'+tint+')');assert.equal(result.amount,remaining===null?null:String(remaining));
  }
  // Healthy quota bars resolve to the provider palette; low quotas switch to named warning colors.
  const barColors=await page.evaluate(()=>{
   const probe=document.createElement('span'),fixture=document.createElement('div');document.body.append(probe,fixture);
   const token=name=>{probe.style.background=`var(${name})`;return getComputedStyle(probe).backgroundColor;};
   const bar=(provider,value)=>{fixture.dataset.provider=provider;fixture.innerHTML=resourceBar(value,'7d 剩余额度',true);return getComputedStyle(fixture.querySelector('span')).backgroundColor;};
   const result={codex:bar('codex',100),claude:bar('claude',100),warning:bar('codex',30),high:bar('codex',9.99),
    codexToken:token('--category-codex'),claudeToken:token('--category-claude'),warningToken:token('--resource-warn'),highToken:token('--resource-high')};
   fixture.remove();probe.remove();return result;
  });
  assert.equal(barColors.codex,barColors.codexToken,'Healthy quota bars use the provider palette');
  assert.notEqual(barColors.codex,barColors.claude,'Provider palettes stay distinct');
  assert.equal(barColors.claude,barColors.claudeToken,'Each provider resolves its own palette');
  assert.equal(barColors.warning,barColors.warningToken,'Low quota bars use the warning color');
  assert.equal(barColors.high,barColors.highToken,'Critical quota bars use the high warning color');
  // Usage bars keep their category color; high load is a red value plus a named warning mark.
  const high=await page.evaluate(()=>{
    const holder=document.createElement('div'),danger=document.createElement('i');holder.innerHTML=resourceBlock('CPU',95,'','cpu');danger.style.color='var(--danger)';document.body.append(holder,danger);
    const mark=holder.querySelector('.high-load-mark'),result={bar:holder.querySelector('.resource-bar > span').style.background,value:getComputedStyle(holder.querySelector('.resource-block > strong')).color,
      mark:mark&&getComputedStyle(mark).color,label:mark?.getAttribute('aria-label'),role:mark?.getAttribute('role'),danger:getComputedStyle(danger).color,
      calm:(()=>{const calm=document.createElement('div');calm.innerHTML=resourceBlock('CPU',89,'','cpu');return !calm.querySelector('.high-load-mark,.high-load');})()};
    holder.remove();danger.remove();return result;
  });
  assert.deepEqual(high,{bar:'',value:high.danger,mark:high.danger,label:'高负载',role:'img',danger:high.danger,calm:true},'High CPU load must still be red');
  assert.equal(await page.evaluate(()=>creditValue({status:'pending',valuePer1000:200})),'待确认');
  if(await card().locator('.quota-disclosure').evaluate(el=>el.open))await card().locator('.quota-disclosure summary').click();
  await page.evaluate(()=>loadDashboard());
  assert.equal(await card().locator('.quota-disclosure').evaluate(el=>el.open),false,'Single account preserves its collapsed summary after refresh');assert(!await card().locator('.credit-row').isVisible());
  assert(await card().locator('.quota-collapsed-summary').isVisible());
  const titleBox=await card().locator('.quota-title h3').boundingBox(),balanceBox=await card().locator('.quota-collapsed-summary').boundingBox();
  assert(balanceBox.y>=titleBox.y+titleBox.height-1,'Collapsed credits and Bank Reset use their own row below the account');
  assert.equal(await card().locator('.quota-collapsed-summary .credit-label').evaluate(el=>getComputedStyle(el).color),await page.evaluate(()=>{const probe=document.createElement('span');probe.style.color='var(--muted)';document.body.append(probe);const color=getComputedStyle(probe).color;probe.remove();return color;}));
  assert.equal(await card().locator('.quota-title .provider-mark img').evaluate(img=>img.complete&&img.naturalWidth>0),true,'Provider icon is bundled and loads');
  assert.equal(await card().locator('.subscription-badge').textContent(),'Pro');
  const edgeCases=await page.evaluate(()=>{
    const holder=document.createElement('div');holder.style.width='414px';document.body.append(holder);
    const cases=[{hasCredits:true,unlimited:false,balance:'0'},{hasCredits:true,unlimited:true},{hasCredits:true,unlimited:false},{hasCredits:false,unlimited:false},{hasCredits:true,unlimited:false,balance:'123456789.125'}];
    try { return cases.map(credits=>{
      holder.innerHTML=quotaCard({...state.dashboard.quotas[0],name:'非常长的订阅账户名称用于检查折叠标题和余额各自完整显示',credits});
      const title=holder.querySelector('.quota-title'),label=holder.querySelector('.quota-collapsed-summary'),value=label.querySelector('strong'),a=title.getBoundingClientRect(),b=label.getBoundingClientRect();
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
   await page.emulateMedia({colorScheme});await page.evaluate(()=>{document.querySelector('.panel-body')?.scrollTo(0,0);(window.pageScroller?.()??document.scrollingElement).scrollTo(0,0);});
   await page.screenshot({animations:'disabled',path:path.join(previews,'credit-'+surface+'-'+colorScheme+'.png')});
  }
  await page.emulateMedia({colorScheme:'light'});
  if(surface==='floating.html'){
   await page.evaluate(()=>{state.days=30;state.cost=true;render();});
   await page.locator('#content .quota-disclosure summary').click();
   await page.reload();await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
   assert.equal(await page.evaluate(()=>state.days),1,'Panel reconstruction always uses today, including old range preferences');assert.equal(await page.evaluate(()=>state.cost),true);
   assert.equal(await page.locator('.quota-disclosure').evaluate(el=>el.open),false,'Panel reconstruction restores the collapsed summary');
   assert.equal(await page.evaluate(()=>testFeature.calls.some(c=>c.method==='creditEstimates.start')),false,'Recreating the panel does not start another sample');
  }
  if(surface==='index.html'){
   await page.evaluate(async()=>{
     testFeature.targets=await api('accounts.targets',{includeUnavailable:true});
     state.settings.sources.push({id:'claude-source',provider:'claude',accountId:'c',name:'Claude source',path:'/claude',enabled:true});
     state.settings.accounts.push({id:'c',provider:'claude',name:'Claude account',quotaEnabled:true});
     testFeature.targets.push({accountKey:'claude:c',accountName:'Claude account',provider:'claude',sourceId:'claude-source',machineId:'local',machineName:'本机',path:'/claude',profileId:null,canWake:true});
     state.page='settings';state.settingsTab='wakeups';renderSettings();
   });await page.locator('#wake-add:not([disabled])').waitFor();await page.locator('#wake-add').click();
   assert(await page.evaluate(()=>Boolean(document.querySelector('#wake-agent').compareDocumentPosition(document.querySelector('#wake-account'))&Node.DOCUMENT_POSITION_FOLLOWING)),'Agent is chosen before account');
   assert.equal(await page.locator('#wake-agent').inputValue(),'codex');
   assert.deepEqual(await page.locator('#wake-account option').evaluateAll(es=>es.map(e=>e.value)),['codex:a']);
   await page.locator('#field-name').fill('Agent-first draft');
   await page.locator('#editor summary').filter({hasText:'执行选项'}).click();
   await page.locator('#field-binary').fill('/custom/codex');await page.locator('#wake-model').fill('previous-model');
   await page.locator('#wake-agent').selectOption('claude');
   assert.deepEqual(await page.locator('#wake-account option').evaluateAll(es=>es.map(e=>e.value)),['claude:c']);
   assert.equal(await page.locator('#wake-source').inputValue(),'claude-source');
   assert.equal(await page.locator('#wake-model').inputValue(),'');assert.equal(await page.locator('#field-binary').inputValue(),'');
   assert.equal(await page.locator('#field-name').inputValue(),'Agent-first draft','Agent changes preserve other draft fields');
   await page.locator('#wake-agent').selectOption('antigravity');
   assert(await page.locator('#wake-account').isDisabled());assert(await page.locator('#wake-probe').isDisabled());
   assert.equal(await page.locator('#wake-source').inputValue(),'');
   await page.locator('#wake-agent').selectOption('codex');
   assert.equal(await page.locator('#wake-account').inputValue(),'codex:a');assert.equal(await page.locator('#wake-source').inputValue(),'s');
   await page.locator('#editor summary').filter({hasText:'执行选项'}).click();

   // Suggestion: the untouched default of a new task is replaced; the same time is never added twice; nothing is saved.
   await page.locator('#wake-apply-suggestion:not([hidden])').waitFor();
   assert.match(await page.locator('#wake-suggestion').textContent(),/通常 09:10 开始使用，建议 06:15 唤醒，5h 窗口约 11:15 重置.*12 个活跃日，按所选来源整体统计/);
   await page.locator('#wake-apply-suggestion').click();assert.equal(await page.locator('#field-times').inputValue(),'06:15');
   await page.locator('#wake-apply-suggestion').click();assert.equal(await page.locator('#field-times').inputValue(),'06:15','The same time is not repeated');
   assert.equal(await page.evaluate(()=>testFeature.calls.filter(c=>c.method==='wakeups.save').length),0,'Applying only edits the draft');
   await page.locator('[data-wake-time="0"][data-time-part="hour"]').selectOption('08');await page.locator('[data-wake-time="0"][data-time-part="minute"]').selectOption('00');
   await page.locator('#wake-apply-suggestion').click();assert.equal(await page.locator('#field-times').inputValue(),'08:00,06:15','A chosen 08:00 is kept and the suggestion is added');
   await page.locator('[data-remove-time="1"]').click();
   await page.evaluate(()=>{testFeature.suggestion={insufficient:true,reason:'days',days:3};});await page.locator('#wake-source').dispatchEvent('change');
   await page.waitForFunction(()=>/只有 3 个活跃日/.test(document.querySelector('#wake-suggestion').textContent));assert(await page.locator('#wake-apply-suggestion').isHidden());
   await page.evaluate(()=>{delete testFeature.suggestion;});
   await page.locator('#wake-probe').click();await page.waitForFunction(()=>document.querySelector('#wake-model')?.value==='cheap-model');assert.equal(await page.locator('#wake-effort').inputValue(),'low');await page.locator('#wake-add-time').click();
   // Adding a time focuses its field; removing one moves focus to the neighbouring remove button, then to Add.
   assert(await page.evaluate(()=>document.activeElement===AieyesSingleSelect.visible(document.querySelector('[data-wake-time="1"][data-time-part="hour"]'))),'A new time focuses its hour picker');
   await page.locator('#wake-add-time').click();await page.locator('[data-remove-time="2"]').click();
   assert.equal(await page.evaluate(()=>document.activeElement.dataset.removeTime),'1','Removing the last time focuses the previous remove button');
   await page.locator('[data-remove-time="1"]').click();await page.locator('#wake-add-time').click();
   assert.deepEqual(await page.evaluate(()=>[...document.querySelectorAll('[data-wake-time]')].map(input=>input.getAttribute('aria-label'))),['目标时间 1：小时','目标时间 1：分钟','目标时间 2：小时','目标时间 2：分钟'],'Labels are whole sentences');
   assert.equal(await page.locator('#wake-time-rows input').count(),0,'Time selection never requires typing');
   assert.equal(await page.locator('[data-wake-time="0"][data-time-part="hour"] option').count(),24);
   assert.equal(await page.locator('[data-wake-time="0"][data-time-part="minute"] option').count(),60);
   await page.locator('[data-wake-time="0"][data-time-part="hour"]').selectOption('00');
   await page.locator('[data-wake-time="0"][data-time-part="minute"]').selectOption('00');
   await page.locator('[data-wake-time="1"][data-time-part="hour"]').selectOption('23');
   await page.locator('[data-wake-time="1"][data-time-part="minute"]').selectOption('59');
   assert.equal(await page.locator('#field-times').inputValue(),'00:00,23:59');
   assert.equal(await page.locator('[data-remove-time="1"]').getAttribute('aria-label'),'移除时刻 23:59');
   await page.getByRole('combobox',{name:'目标时间 2：小时',exact:true}).click();
   await page.getByRole('option',{name:'13',exact:true}).click();
   await page.getByRole('combobox',{name:'目标时间 2：分钟',exact:true}).click();
   await page.getByRole('option',{name:'30',exact:true}).click();
   fs.mkdirSync(path.resolve(__dirname,'../.local/wakeup-review'),{recursive:true});
   await page.screenshot({path:path.resolve(__dirname,'../.local/wakeup-review/time-picker.png')});
   await page.setViewportSize({width:640,height:720});
   assert(await page.locator('#wake-time-rows').evaluate(el=>el.scrollWidth<=el.clientWidth+1),'Time controls fit a narrow editor');
   await page.setViewportSize({width:1120,height:850});
await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
   assert.deepEqual(await page.evaluate(()=>testFeature.tasks[0].task.times),['00:00','13:30'],'Selected components save as target-machine wall clock times');
   await page.locator('[data-wake-action=edit]').click();
   assert.equal(await page.locator('[data-wake-time="0"][data-time-part="hour"]').inputValue(),'00');
   assert.equal(await page.locator('[data-wake-time="1"][data-time-part="hour"]').inputValue(),'13');
   assert.equal(await page.locator('[data-wake-time="1"][data-time-part="minute"]').inputValue(),'30');
   await discardEditor(page);
   assert.equal(await page.evaluate(()=>testFeature.calls.filter(c=>c.method==='wakeups.deploy').length),0,'saving does not deploy');await page.locator('[data-wake-action=deploy]').click();await page.locator('.wake-more summary').click();await page.locator('[data-wake-action=disable]').waitFor();await page.locator('[data-wake-action=disable]').click();await page.locator('.wake-more summary').click();await page.locator('[data-wake-action=enable]').waitFor();await page.locator('[data-wake-action=enable]').click();await page.locator('[data-wake-action=status]').click();await page.locator('#quota-dialog').getByText('已完成',{exact:true}).waitFor();await page.locator('#quota-close').click();
   await page.evaluate(()=>testFeature.failRemoval=true);await page.locator('.wake-more summary').click();await page.locator('.wake-more [data-wake-action=remove]').click();await page.locator('#editor-form button[type=submit]').click();await page.getByText('待移除',{exact:true}).waitFor();assert.match(await page.locator('#content').textContent(),/尚未确认移除/);await discardEditor(page);await page.evaluate(()=>testFeature.failRemoval=false);await page.locator('.wake-more summary').click();await page.locator('.wake-more [data-wake-action=remove]').click();await page.locator('#editor-form button[type=submit]').click();await page.locator('[data-wake-action=delete]').waitFor({state:'attached'});assert.doesNotMatch(await page.locator('#content').textContent(),/尚未确认移除/,'successful removal clears its own retained error');
   fs.mkdirSync(path.resolve(__dirname,'../.local/wakeup-review'),{recursive:true});await page.screenshot({path:path.resolve(__dirname,'../.local/wakeup-review/desktop.png')});
  }
  assert.deepEqual(errors,[]);await page.close();
 }
 console.log('Credits and wakeup UI: both surfaces, sampling, recommended model/effort, draft/deploy separation, enable/disable, history and failed removal passed');
 }finally{await browser?.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
