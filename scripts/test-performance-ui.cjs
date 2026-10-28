// Production renderer/query queue with synthetic IPC. No real hosts or accounts.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const fixture=require('./fixtures/browser-ui.cjs');
require('node:child_process').execFileSync(process.env.PYTHON||(process.platform==='win32'?'python':'python3'),[path.join(__dirname,'test-estimate-display.py')],{cwd:path.resolve(__dirname,'..'),stdio:'inherit'});
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web');
 const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://test').pathname)||'index.html');try{res.setHeader('content-type',({'.js':'text/javascript','.html':'text/html','.css':'text/css'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));}catch{res.writeHead(404);res.end();}});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const browser=await chromium.launch({headless:true});
 try{
 const page=await browser.newPage({viewport:{width:1120,height:800}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(()=>AieyesApp.ready);
 await page.evaluate(()=>{refreshStopped=true;clearTimeout(refreshTimer);});
 // A one-shot scheduler must be armed again after the operation that blocked it.
 const resumes=await page.evaluate(async()=>{
   const settings=structuredClone(state.settings),invoke=window.__TAURI__.core.invoke;
   const originalSet=window.setTimeout,originalClear=window.clearTimeout,active=new Set();
   window.setTimeout=(fn,ms,...args)=>{const id=originalSet(fn,ms,...args);if(fn===runScheduledRefreshes)active.add(id);return id;};
   window.clearTimeout=id=>{active.delete(id);return originalClear(id);};
   try {
     state.settings={...settings,hosts:[],localMonitor:{...settings.localMonitor,enabled:false}};
     state.lastScan=Date.now();refreshStopped=false;
     window.__TAURI__.core.invoke=async(command,args)=>{
       if(command==='engine_call'){
         if(args.method==='quotas.schedule')return {nextDueAt:null};
         if(args.method==='quotas.refresh')return [];
         if(args.method==='settings.patch')return args.params.settings;
         if(args.method==='settings.get'){await new Promise(r=>originalSet(r,30));return state.settings;}
       }
       return invoke(command,args);
     };
     await quotas(undefined,true);
     const afterQuota=active.size>0;
     clearTimeout(refreshTimer);
     await saveAllSettings({section:'sources',refresh:false});
     return {afterQuota,afterSave:active.size>0};
   } finally {
     refreshStopped=true;clearTimeout(refreshTimer);state.settings=settings;
     window.__TAURI__.core.invoke=invoke;window.setTimeout=originalSet;window.clearTimeout=originalClear;
   }
 });assert.deepEqual(resumes,{afterQuota:true,afterSave:true});
 await page.evaluate(()=>AieyesApp.navigatePage('analysis'));
 // Only the latest of a burst of filter changes is issued.
 const burst=await page.evaluate(async()=>{
   const before=engineCalls.filter(c=>c.method==='dashboard').length;
   await Promise.all([1,7,30,90,365].map(days=>changeFilters({days})));
   return {count:engineCalls.filter(c=>c.method==='dashboard').length-before,days:state.appliedFilters.days};
 });assert.deepEqual(burst,{count:1,days:365});
 // An already-running request may finish, but must not publish stale filters.
 const coalesced=await page.evaluate(async()=>{
   const invoke=window.__TAURI__.core.invoke;let release,calls=0;
   window.__TAURI__.core.invoke=async(command,args)=>{if(command==='engine_call'&&args.method==='dashboard'&&++calls===1)await new Promise(r=>release=r);return invoke(command,args);};
   state.days=1;const first=loadDashboard();await Promise.resolve();
   const pending=[7,30,90].map(days=>{state.days=days;return loadDashboard();});
   release();await Promise.all([first,...pending]);window.__TAURI__.core.invoke=invoke;
   return {calls,days:state.appliedFilters.days};
 });assert.deepEqual(coalesced,{calls:2,days:90});
 await page.evaluate(()=>{
   state.page='servers';delete state.settings.localMonitor;state.settings.hosts=Array.from({length:30},(_,i)=>({id:'perf-'+i,name:'Synthetic '+i,target:'fixture',enabled:true,metrics:['cpu'],details:[],devices:[]}));
   state.hosts=[];
   applyHostSamples({rows:state.settings.hosts.map(host=>({id:host.id,sampleSession:'test',sampleVersion:1,sample:{timestamp:Date.now()/1000,cpu:Array.from({length:257},(_,i)=>({id:i?'cpu'+(i-1):'cpu',utilization:20})),load:[],errors:{}}}))});renderServers();
 });
 assert.equal(await page.locator('.server-card').count(),30);assert.equal(await page.locator('.metric-device').count(),0);
 await page.locator('.server-card').first().locator('.server-more > summary').click();await page.locator('[data-metric="perf-0:cpu"] > summary').click();await page.locator('.metric-device').first().waitFor();
 assert((await page.locator('.metric-device').count())<40,'Only a viewport of 256 cores should be mounted');
 const retention=await page.evaluate(async()=>{
   const card=document.querySelector('.server-card'),summary=card.querySelector('[data-metric] > summary');summary.focus();const rows=state.hosts;
   applyHostSamples({rows});applyHostSamples(rows);
   await new Promise(r=>requestAnimationFrame(r));
   return {card:card===document.querySelector('.server-card'),focus:summary===document.activeElement,open:summary.parentElement.open};
 });assert.deepEqual(retention,{card:true,focus:true,open:true});
 await page.evaluate(()=>{
   const group=document.querySelector('[data-metric="perf-0:cpu"]');(window.pageScroller?.()??document.scrollingElement).scrollBy(0,group.getBoundingClientRect().top+10000);
 });
 await page.waitForFunction(()=>[...document.querySelectorAll('.metric-device')].some(e=>Number(e.getAttribute('aria-posinset'))>80));
 assert((await page.locator('.metric-device').count())<40,'Scrolling must recycle device nodes');
 await page.evaluate(()=>{(window.pageScroller?.()??document.scrollingElement).scrollTo(0,0);applyHostFailure('Synthetic transport failure','perf-0');});
 await page.waitForFunction(()=>state.hosts[0].error==='Synthetic transport failure');
 assert.equal(await page.evaluate(()=>state.hosts[0].sample.cpu[0].utilization),20,'Transport failures preserve previous samples despite version deduplication');
 await page.evaluate(()=>{state.page='agent';render();state.page='servers';render();});
 assert.equal(await page.locator('.server-card .server-more').first().evaluate(e=>e.open),true);
 await page.locator('[data-metric="perf-0:cpu"] > summary').waitFor();assert.equal(await page.locator('[data-metric="perf-0:cpu"]').evaluate(e=>e.open),true);
 assert.equal(await page.evaluate(()=>uiTimers.some(t=>t.ms===500)),false,'There must be no 500 ms background scan timer');
 // A thousand-model catalog keeps five visible rows and recycles offscreen entries.
 await page.evaluate(()=>{state.page='settings';state.settingsTab='prices';state.prices=Array.from({length:1000},(_,i)=>({id:'perf/model-'+i,name:'Model '+i,input:1e-6,output:3e-6}));state.priceSearch='';renderSettings(false);});
 const catalog=await page.evaluate(()=>{
   const durations=[];for(let i=0;i<15;i++){const start=performance.now();renderSettings(false);document.querySelector('.prices-list').getBoundingClientRect();durations.push(performance.now()-start);}
   const viewport=document.querySelector('.prices-list');return {durations,p95:durations.slice().sort((a,b)=>a-b)[14],height:viewport.clientHeight,rows:viewport.querySelectorAll('[data-price]').length};
 });assert(catalog.p95<100,`1000 model hot tab P95 ${catalog.p95.toFixed(1)} ms`);assert(catalog.rows<=9);assert(catalog.height<=322);
 await page.locator('#price-search').fill('perf/model-999');assert.equal(await page.locator('[data-price="perf/model-999"]').count(),1);
 await page.locator('#price-search').fill('');await page.locator('.prices-list').evaluate(el=>el.scrollTop=el.scrollHeight);
 await page.waitForFunction(()=>document.querySelector('[data-price="perf/model-999"]'));
 // Tab walks the virtual list without losing focus: rows are reused by id and the focused row is never dropped.
 const frames=()=>page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
 await page.locator('.prices-list').evaluate(el=>el.scrollTop=0);await page.waitForFunction(()=>document.querySelector('[data-price="perf/model-0"]'));
 await page.locator('[data-price="perf/model-0"]').focus();
 for(let i=0;i<24;i++){await page.keyboard.press('Tab');await frames();}
 const walked=await page.evaluate(()=>document.activeElement.closest('[data-price-row]')?.dataset.priceRow);
 assert.equal(walked,'perf/model-12','Twenty-four Tabs land on row 12 inside the list');
 await page.locator('.prices-list').evaluate(el=>el.scrollTop=el.scrollHeight);await frames();
 assert.equal(await page.evaluate(()=>document.activeElement.closest('[data-price-row]')?.dataset.priceRow),walked,'Scrolling away keeps the focused row');
 assert.equal(await page.evaluate(()=>{const order=[...document.querySelectorAll('[data-price-row]')].map(row=>+row.dataset.priceIndex);return order.every((v,i)=>!i||order[i-1]<v);}),true,'Rows stay in list order for Tab');
 // Per-source responses update row projections while keeping the page and input nodes.
 const status=await page.evaluate(async()=>{
   while(AieyesAgentSettings.statusBusy)await new Promise(r=>setTimeout(r,5));
   const invoke=window.__TAURI__.core.invoke,original=renderSettings;let rebuilds=0,queries=0;
   window.__TAURI__.core.invoke=async(command,args)=>{
     if(command==='engine_call'&&args.method==='accounts.status.refresh'){queries++;return {accountKey:args.params.accountKey,sourceId:args.params.sourceId,machineId:'local',current:true,credential:true,checkedAt:Date.now()/1000};}
     if(command==='engine_call'&&args.method==='settings.get')return structuredClone(state.settings);
     return invoke(command,args);
   };
   state.settings={...state.settings,localDeviceEnabled:true,modelMappings:{},accounts:Array.from({length:10},(_,i)=>({provider:'custom',id:'a'+i,name:'Account '+i,quotaEnabled:true})),sources:Array.from({length:30},(_,i)=>({provider:'custom',id:'s'+i,accountId:'a'+Math.floor(i/3),name:'Source '+i,path:'/fixture/'+i,enabled:true})),agents:[{provider:'custom',enabled:true,machineIds:['local']}],hosts:[]};
   state.settingsDraft=structuredClone(state.settings);state.settingsTab='integrations';state.settingsAgent='custom';renderSettings(false);
   while(AieyesAgentSettings.statusBusy)await new Promise(r=>setTimeout(r,5));
   await AieyesAgentSettings.openDetails(state.settings.accounts[0]);
   while(AieyesAgentSettings.statusBusy)await new Promise(r=>setTimeout(r,5));
   const input=document.querySelector('#account-name');input.focus();input.value='尚未保存的名称';input.dispatchEvent(new Event('input',{bubbles:true}));input.setSelectionRange(2,4);
   queries=0;
   renderSettings=(...args)=>{rebuilds++;return original(...args);};
   const card=document.querySelector('.account-config');
   try{await AieyesAgentSettings.refreshStatuses(true,'custom');await new Promise(r=>requestAnimationFrame(r));return {rebuilds,queries,retained:card===document.querySelector('.account-config'),focus:input===document.activeElement,draft:input.value,selection:[input.selectionStart,input.selectionEnd]};}
   finally{renderSettings=original;window.__TAURI__.core.invoke=invoke;}
 });assert.equal(status.rebuilds,0);assert(status.retained);assert(status.queries>0);
 assert(status.focus);assert.equal(status.draft,'尚未保存的名称');assert.deepEqual(status.selection,[2,4]);
 // Modern protocol: operation/revision deduplication and settings-specific invalidation.
 const modern=await browser.newPage({viewport:{width:1120,height:800}});modern.on('pageerror',e=>errors.push(String(e)));await modern.addInitScript(fixture);
 await modern.addInitScript(()=>{
   const invoke=window.__TAURI__.core.invoke;
   window.modernCalls=[];window.modernRevisions={settings:1,usage:1,quotas:1,estimates:1,prices:1,sources:1,accounts:1,pets:1,notifications:1};
   window.__TAURI__.core.invoke=async(command,args)=>{
     if(command!=='engine_call')return invoke(command,args);
     window.modernCalls.push(structuredClone(args));
     if(args.method==='hello')return {runtimeInvalidations:true};
     if(args.method==='runtime.revisions')return structuredClone(window.modernRevisions);
     if(args.method==='accounts.status.refreshBatch')return [];
     if(args.method==='accounts.deployments.sync')return {failedTasks:[]};
     if(args.method==='quotas.refresh'){
       window.modernRevisions.quotas++;
       const revisions=structuredClone(window.modernRevisions);
       // A cross-process observer can publish first, before the operation's own event.
       window.__listeners['runtime:changed']?.({payload:{revisions}});
       await new Promise(r=>setTimeout(r,30));
       window.__listeners['runtime:changed']?.({payload:{revisions,operationId:args.operationId}});
       window.__listeners['desktop:invalidate']?.({payload:{method:args.method,operationId:args.operationId}});
       window.__listeners['desktop:data-changed']?.({payload:args.method});
       return [];
     }
     return invoke(command,args);
   };
 });
 await modern.goto(`http://127.0.0.1:${server.address().port}`);await modern.evaluate(()=>AieyesApp.ready);
 await modern.evaluate(async()=>{refreshStopped=true;clearTimeout(refreshTimer);__listeners['runtime:changed']({payload:{revisions:modernRevisions}});await new Promise(r=>setTimeout(r,350));});
 const invalidations=await modern.evaluate(async()=>{
   const pause=()=>new Promise(r=>setTimeout(r,350)),queries=()=>modernCalls.filter(c=>c.method==='dashboard').length;
   let before=queries(),statusBefore=modernCalls.filter(c=>c.method==='dashboard.status').length;await api('quotas.refresh');await loadDashboard();await pause();const own=queries()-before,ownStatus=modernCalls.filter(c=>c.method==='dashboard.status').length-statusBefore;
   before=queries();const revisions={...modernRevisions,usage:++modernRevisions.usage};
   __listeners['runtime:changed']({payload:{revisions,operationId:'external'}});
   __listeners['desktop:invalidate']({payload:{method:'sources.scan',operationId:'external'}});
   await pause();const external=queries()-before;
   __listeners['runtime:changed']({payload:{revisions}});await pause();const duplicate=queries()-before-external;
   before=queries();await loadDashboard();const unchanged=queries()-before;
   return {own,ownStatus,external,duplicate,unchanged};
 });assert.deepEqual(invalidations,{own:0,ownStatus:1,external:1,duplicate:0,unchanged:0});
 const quotaOnly=await modern.evaluate(async()=>{
   const before=modernCalls.length;
   for(let i=0;i<5;i++){
     modernRevisions.quotas++;
     __listeners['runtime:changed']({payload:{revisions:{...modernRevisions}}});
     await new Promise(r=>setTimeout(r,300));
   }
   const calls=modernCalls.slice(before);
   return {full:calls.filter(c=>c.method==='dashboard').length,status:calls.filter(c=>c.method==='dashboard.status').length};
 });assert.deepEqual(quotaOnly,{full:0,status:5});
 const busyRefresh=await modern.evaluate(async()=>{
   const set=window.setTimeout;let retries=0;
   window.setTimeout=(fn,ms,...args)=>{if(ms===120)retries++;return set(fn,ms,...args);};
   const before=modernCalls.length;
   try{
     state.busy=true;modernRevisions.quotas++;
     __listeners['runtime:changed']({payload:{revisions:{...modernRevisions}}});
     await new Promise(r=>set(r,700));
     const whileBusy=modernCalls.slice(before).filter(c=>c.method.startsWith('dashboard')).length;
     state.busy=false;updateActivity();await new Promise(r=>set(r,350));
     return {retries,whileBusy,status:modernCalls.slice(before).filter(c=>c.method==='dashboard.status').length};
   }finally{state.busy=false;window.setTimeout=set;}
 });assert.deepEqual(busyRefresh,{retries:1,whileBusy:0,status:1});
 const hostMonitor=await modern.evaluate(async()=>{
   const saved=structuredClone(state.settings),invoke=window.__TAURI__.core.invoke;
   let configurations=0,refreshes=0,version=10;
   const rows=[{id:'fast',sampleSession:'monitor',sampleVersion:1,sample:{timestamp:Date.now()/1000}},{id:'slow'}];
   const snapshot=busy=>({rows,inFlight:busy,pending:[],sampleSession:'monitor',version:version++});
   try{
     state.settings={...saved,hosts:['fast','slow'].map(id=>({id,name:id,enabled:true,metrics:[]})),localMonitor:{enabled:false}};
     window.__TAURI__.core.invoke=async(command,args)=>{
       if(command==='engine_call'&&args.method==='hosts.monitor.configure'){configurations++;return snapshot(['slow']);}
       if(command==='engine_call'&&args.method==='hosts.monitor.refresh'){refreshes++;return snapshot(['slow','fast']);}
       return invoke(command,args);
     };
     state.hostMonitoring=true;state.lastScan=Date.now();refreshStopped=false;hostMonitorConfiguration='';
     await configureHostMonitor();
     for(let i=0;i<20;i++)updateActivity();
     await sample('fast');const fastBusy=hostRefreshBusy('fast');
     const complete=snapshot(['slow']);complete.rows=[{...rows[0],sampleVersion:2,sample:{timestamp:Date.now()/1000,cpu:[]}},rows[1]];
     __listeners['hosts:changed']({payload:complete});
     __listeners['hosts:changed']({payload:{...complete,version:complete.version-1,rows:[{...rows[0],sampleVersion:99}]}});
     return {configurations,refreshes,fastBusy,fastAvailable:!hostRefreshBusy('fast'),slowBusy:hostRefreshBusy('slow'),version:state.hosts.find(h=>h.id==='fast').sampleVersion};
   }finally{refreshStopped=true;clearTimeout(refreshTimer);state.hostMonitoring=false;state.serverBusy=false;state.settings=saved;state.refreshState.hosts={};window.__TAURI__.core.invoke=invoke;}
 });assert.deepEqual(hostMonitor,{configurations:1,refreshes:1,fastBusy:true,fastAvailable:true,slowBusy:true,version:2});
 const settingsChecks=await modern.evaluate(async()=>{
   while(AieyesAgentSettings.statusBusy)await new Promise(r=>setTimeout(r,5));
   let saved=structuredClone(state.settings);saved.accounts=[{id:'a',name:'Fixture',provider:'codex',quotaEnabled:false}];saved.sources=[{id:'s',name:'Fixture',provider:'codex',path:'/fixture/one',enabled:true}];saved.hosts=[];
   window.fixtureSettings=structuredClone(saved);AieyesAgentSettings.accept(saved);await new Promise(r=>setTimeout(r,50));while(AieyesAgentSettings.statusBusy)await new Promise(r=>setTimeout(r,5));
   let before=modernCalls.filter(c=>c.method==='accounts.status.refreshBatch').length;
   saved=structuredClone(state.settings);saved.appearance={theme:'dark',accent:'teal'};saved.modelMappings={fixture:'fixture/model'};AieyesAgentSettings.accept(saved);
   await new Promise(r=>setTimeout(r,50));const unrelated=modernCalls.filter(c=>c.method==='accounts.status.refreshBatch').length-before;
   before=modernCalls.filter(c=>c.method==='accounts.status.refreshBatch').length;
   saved=structuredClone(state.settings);saved.sources[0].path='/fixture/two';window.fixtureSettings=structuredClone(saved);AieyesAgentSettings.accept(saved);await new Promise(r=>setTimeout(r,50));while(AieyesAgentSettings.statusBusy)await new Promise(r=>setTimeout(r,5));
   const changed=modernCalls.filter(c=>c.method==='accounts.status.refreshBatch').slice(before);
   await AieyesAgentSettings.refreshStatuses(true);const explicit=modernCalls.filter(c=>c.method==='accounts.status.refreshBatch').at(-1);
   return {unrelated,changed:changed.map(c=>c.params),explicit:explicit.params};
 });assert.equal(settingsChecks.unrelated,0);assert.equal(settingsChecks.changed.length,1);assert.equal(settingsChecks.changed[0].dueOnly,true);assert.equal(settingsChecks.explicit.dueOnly,false);
 const timezone=await modern.context().newCDPSession(modern);
 await timezone.send('Emulation.setTimezoneOverride',{timezoneId:'UTC'});await modern.evaluate(()=>loadDashboard());
 const timezoneBefore=await modern.evaluate(()=>({queries:modernCalls.filter(c=>c.method==='dashboard').length,date:new Date().toDateString()}));
 await timezone.send('Emulation.setTimezoneOverride',{timezoneId:'Africa/Abidjan'});await modern.evaluate(()=>loadDashboard());
 const timezoneAfter=await modern.evaluate(()=>({queries:modernCalls.filter(c=>c.method==='dashboard').length,date:new Date().toDateString()}));
 assert.equal(timezoneBefore.date,timezoneAfter.date);assert.equal(timezoneAfter.queries,timezoneBefore.queries+1,'Timezone identity invalidates even on the same local day');
 await modern.close();
 console.log('Modern revisions: quota-only 0 usage reads; busy refresh 1 debounce and 1 completion flush; host configuration coalesces and per-host refresh stays available; unchanged/cache hits 0 reads');
 console.log('Catalog P95:',catalog.p95.toFixed(1)+' ms','mounted rows:',catalog.rows,'status requests:',status.queries,'full settings rebuilds:',status.rebuilds);
 assert.deepEqual(errors,[]);console.log('Scheduler resume after quota/save, latest-only dashboard queries, version deduplication, retained DOM, lazy 30×256 device rendering, scrolling and expansion persistence passed');
 }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exit(1)});
