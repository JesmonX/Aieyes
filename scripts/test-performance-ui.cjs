// Production renderer/query queue with synthetic IPC. No real hosts or accounts.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const source=fs.readFileSync(path.join(__dirname,'test-ui.cjs'),'utf8');
const fixture=eval('('+source.slice(source.indexOf('const fixture = ')+16,source.indexOf('\n    await page.addInitScript(fixture);')).trim().replace(/;$/,'')+')');
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
     state.lastScan=Date.now();quotaScheduleKnown=true;quotaDueAt=null;refreshStopped=false;
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
 await page.locator('.server-card').first().locator(':scope > summary').click();await page.locator('[data-metric="perf-0:cpu"] > summary').click();await page.locator('.metric-device').first().waitFor();
 assert((await page.locator('.metric-device').count())<40,'Only a viewport of 256 cores should be mounted');
 const retention=await page.evaluate(async()=>{
   const card=document.querySelector('.server-card'),summary=card.querySelector('[data-metric] > summary');summary.focus();const rows=state.hosts;
   applyHostSamples({rows});applyHostSamples(rows);
   await new Promise(r=>requestAnimationFrame(r));
   return {card:card===document.querySelector('.server-card'),focus:summary===document.activeElement,open:summary.parentElement.open};
 });assert.deepEqual(retention,{card:true,focus:true,open:true});
 await page.evaluate(()=>{
   const group=document.querySelector('[data-metric="perf-0:cpu"]');window.scrollBy(0,group.getBoundingClientRect().top+10000);
 });
 await page.waitForFunction(()=>[...document.querySelectorAll('.metric-device')].some(e=>Number(e.getAttribute('aria-posinset'))>80));
 assert((await page.locator('.metric-device').count())<40,'Scrolling must recycle device nodes');
 await page.evaluate(()=>{window.scrollTo(0,0);applyHostFailure('Synthetic transport failure','perf-0');});
 await page.waitForFunction(()=>state.hosts[0].error==='Synthetic transport failure');
 assert.equal(await page.evaluate(()=>state.hosts[0].sample.cpu[0].utilization),20,'Transport failures preserve previous samples despite version deduplication');
 await page.evaluate(()=>{state.page='agent';render();state.page='servers';render();});
 assert.equal(await page.locator('.server-card').first().evaluate(e=>e.open),true);
 await page.locator('[data-metric="perf-0:cpu"] > summary').waitFor();assert.equal(await page.locator('[data-metric="perf-0:cpu"]').evaluate(e=>e.open),true);
 assert.equal(await page.evaluate(()=>uiTimers.some(t=>t.ms===500)),false,'There must be no 500 ms background scan timer');
 assert.deepEqual(errors,[]);console.log('Scheduler resume after quota/save, latest-only dashboard queries, version deduplication, retained DOM, lazy 30×256 device rendering, scrolling and expansion persistence passed');
 }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exit(1)});
