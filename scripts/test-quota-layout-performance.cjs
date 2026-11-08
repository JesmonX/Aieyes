// Layout work must stay bounded as accounts grow, including refreshes of collapsed groups.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {fixture}=require('./workspace-ui-fixture.cjs');
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web');
 const server=http.createServer((req,res)=>{
  if(require('./ui-test-helpers.cjs').serveFont(req,res))return;
  const name=path.basename(new URL(req.url,'http://fixture').pathname)||'index.html';
  const file=name==='quota-card-layout.js'&&process.env.AIEYES_QUOTA_LAYOUT_SCRIPT?process.env.AIEYES_QUOTA_LAYOUT_SCRIPT:path.join(root,name);
  try{res.setHeader('content-type',({'.js':'text/javascript','.html':'text/html','.css':'text/css'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));}catch{res.writeHead(404);res.end();}
 });
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const browser=await chromium.launch({headless:true});
 try{
  const page=await browser.newPage({viewport:{width:1600,height:900},reducedMotion:'reduce'}),errors=[];
  page.on('pageerror',error=>errors.push(String(error)));
  await page.addInitScript(fixture,{platform:'windows'});
  await page.addInitScript(()=>{
   const Native=window.ResizeObserver;window.layoutObservers=[];
   window.ResizeObserver=class extends Native{
    constructor(callback){super(callback);this.targets=new Set();layoutObservers.push(this);}
    observe(target,options){this.targets.add(target);super.observe(target,options);}
    unobserve(target){this.targets.delete(target);super.unobserve(target);}
    disconnect(){this.targets.clear();super.disconnect();}
   };
  });
  await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(()=>AieyesApp.ready);
  await page.evaluate(async()=>{refreshStopped=true;clearTimeout(refreshTimer);await document.fonts.ready;});
  const settle=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))));
  await settle();
  const cdp=await page.context().newCDPSession(page);await cdp.send('Performance.enable');
  const layouts=async()=>(await cdp.send('Performance.getMetrics')).metrics.find(m=>m.name==='LayoutCount').value;
  const failures=[],passes=[];
  for(const count of [8,32]){
   const before=await layouts();
   await page.evaluate(count=>{
    const account=state.settings.accounts[0],source=state.settings.sources[0],quota=state.dashboard.quotas[0];
    state.settings.accounts=Array.from({length:count},(_,i)=>({...account,id:'stress-'+i,name:'Synthetic '+i}));
    state.settings.sources=state.settings.accounts.map(a=>({...source,id:'s-'+a.id,accountId:a.id}));
    state.dashboard.quotas=state.settings.accounts.map(a=>({...structuredClone(quota),accountId:a.id,name:a.name,sourceId:'s-'+a.id}));
    renderOverview();
   },count);
   await settle();assert.equal(await page.locator('#content .quota-card').count(),count);
   const value=await layouts()-before;passes.push({accounts:count,layouts:value});
   if(value>12)failures.push(`${count} accounts triggered ${value} layouts; measuring each card separately blocks interaction`);
  }
  await page.evaluate(()=>document.querySelectorAll('.quota-group').forEach(group=>group.open=false));await settle();
  assert.equal(await page.locator('.quota-group[open]').count(),0);
  for(let i=0;i<20;i++){await page.evaluate(()=>renderOverview());await settle();}
  const retained=await page.evaluate(()=>{
   const targets=layoutObservers.flatMap(observer=>[...observer.targets]).filter(target=>target.matches('.quota-card,.quota-placeholder'));
   return {observed:targets.length,detached:targets.filter(target=>!target.isConnected).length,probes:document.querySelector('.quota-measure-layer').childElementCount};
  });
  if(retained.detached)failures.push(`${retained.detached} removed collapsed cards still registered for resize observation`);
  assert.equal(retained.probes,0);assert.equal(retained.observed,32);assert.deepEqual(errors,[]);
  console.log('Quota layout performance:',JSON.stringify({passes,retained}));assert.deepEqual(failures,[]);
 }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
