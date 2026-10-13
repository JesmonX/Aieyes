// Date controls, persisted filters and lazy multi-year rendering using synthetic history.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const fixture=require('./fixtures/browser-ui.cjs');
function historyFixture(){
  const invoke=window.__TAURI__.core.invoke,days=[];
  for(let t=Date.UTC(2023,0,1);t<=Date.UTC(2026,0,2);t+=86400000)days.push({key:new Date(t).toISOString().slice(0,10),tokens:{input:1,output:0,cacheRead:0,cacheWrite:0},total:1,cost:0.01,pricedTokens:1,events:1});
  window.rangeQueries=[];
  window.__TAURI__.core.invoke=async(command,args)=>{
    if(command!=='engine_call'||!['dashboard','dashboard.summary'].includes(args.method))return invoke(command,args);
    const p=args.params??{},r=p.dateRange;rangeQueries.push(structuredClone(p));
    const selected=r?.kind==='all'?days:r?.kind==='custom'?days.filter(d=>d.key>=r.start&&d.key<=r.end):days.slice(-(p.days??1));
    const summary={tokens:{input:selected.length,output:0,cacheRead:0,cacheWrite:0},total:selected.length,cost:selected.length*0.01,pricedTokens:selected.length,events:selected.length};
    return {generatedAt:Date.now()/1000,summary,quotas:[],sources:[],pricingGaps:[],models:[{...summary,key:'fixture/model'}],modelOptions:['fixture/model'],trendDays:selected,dayModels:selected.map(d=>({day:d.key,model:'fixture/model',usage:d})),heatmap:selected};
  };
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web');
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');try{res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));}catch{res.writeHead(404).end();}});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const browser=await chromium.launch({headless:true});
  try{
    const page=await browser.newPage({viewport:{width:1120,height:800}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));
    await page.addInitScript(fixture);await page.addInitScript(historyFixture);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.evaluate(()=>AieyesApp.ready);
    await page.evaluate(()=>{refreshStopped=true;clearTimeout(refreshTimer);});
    assert.deepEqual(await page.locator('#days option').allTextContents(),['今日','最近 7 天','最近 30 天','最近 90 天','最近一年','所有','自定义日期']);
    await page.locator('#days').selectOption('all');await page.waitForFunction(()=>state.appliedFilters.dateRange?.kind==='all'&&!state.dashboardPending);
    assert.equal(await page.locator('[data-heat-year]').count(),4);
    assert.equal(await page.locator('[data-heat-day]').count(),2,'Only the latest year is mounted initially');
    assert.equal(await page.locator('[data-day-toggle]').count(),31);
    await page.locator('.daily > summary').click();
    await page.locator('[data-more-days]').click();assert.equal(await page.locator('[data-day-toggle]').count(),62);
    await page.locator('[data-heat-year="2024"] > summary').click();await page.waitForFunction(()=>document.querySelectorAll('[data-heat-year="2024"] [data-heat-day]').length===366);
    await page.locator('[data-heat-year="2024"] > summary').click();await page.waitForFunction(()=>!document.querySelector('[data-heat-year="2024"] [data-heat-day]'));
    await page.locator('#days').selectOption('custom');await page.locator('#range-apply').waitFor();
    await page.waitForFunction(()=>!state.dashboardPending);
    await page.locator('#range-start').fill('2024-03-01');await page.locator('#range-end').fill('2024-02-28');
    const before=await page.evaluate(()=>rangeQueries.length);await page.locator('#range-apply').click();
    assert.equal(await page.evaluate(()=>rangeQueries.length),before,'Reversed dates never query the core');
    await page.locator('#range-start').fill('2024-02-28');await page.locator('#range-end').fill('2024-03-01');await page.locator('#range-apply').click();
    await page.waitForFunction(()=>state.appliedFilters.dateRange?.start==='2024-02-28'&&!state.dashboardPending);
    assert.deepEqual(await page.evaluate(()=>rangeQueries.at(-1).dateRange),{kind:'custom',start:'2024-02-28',end:'2024-03-01'});
    assert.deepEqual(await page.locator('[data-day-toggle]').allTextContents(),['2024-03-01','2024-02-29','2024-02-28']);
    if(!await page.locator('[data-heat-year="2024"]').evaluate(el=>el.open))await page.locator('[data-heat-year="2024"] > summary').click();
    await page.waitForFunction(()=>document.querySelectorAll('[data-heat-day]').length===3);
    assert.equal(await page.locator('[data-heat-day]').count(),3);
    await page.reload();await page.evaluate(()=>AieyesApp.ready);
    assert.equal(await page.locator('#days').inputValue(),'custom');assert.equal(await page.locator('#range-start').inputValue(),'2024-02-28');
    await page.locator('#days').selectOption('7');await page.waitForFunction(()=>state.appliedFilters.days===7&&!state.appliedFilters.dateRange&&!state.dashboardPending);
    assert.equal(await page.locator('#range-start').count(),0);assert.equal(await page.evaluate(()=>rangeQueries.at(-1).dateRange),null);
    assert.deepEqual(errors,[]);console.log('Date ranges: all/custom/shortcuts, inclusive leap-day controls, invalid input, persistence, lazy years and daily pagination passed');
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exit(1);});
