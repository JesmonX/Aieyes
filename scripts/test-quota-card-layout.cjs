// Production cards with different subscriptions/content. No personal data or account calls.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {fixture}=require('./workspace-ui-fixture.cjs');
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web');
 const server=http.createServer((req,res)=>{
  if(require('./ui-test-helpers.cjs').serveFont(req,res))return;
  const file=path.join(root,path.basename(new URL(req.url,'http://test').pathname)||'index.html');
  try{res.setHeader('content-type',({'.js':'text/javascript','.html':'text/html','.css':'text/css','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));}catch{res.writeHead(404);res.end();}
 });
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
 try{
  browser=await chromium.launch({headless:true});
  for(const platform of ['windows','linux'])for(const surface of ['index.html','floating.html']){
   const page=await browser.newPage({viewport:{width:surface==='floating.html'?450:1120,height:820}}),errors=[];
   page.on('pageerror',e=>errors.push(String(e)));
   await page.addInitScript(fixture,{platform});
   await page.addInitScript(()=>{
    const rows=navigationFixture.quotas;
    rows[0].name='A very long subscription account name with wrapping';rows[0].plan='plus';
    rows[1].plan='pro';rows[1].credits={balance:'12345678.1234',hasCredits:true};
    rows[2].windows=rows[2].windows.slice(0,1);rows[2].error='Synthetic offline quota update error';
    rows[3].windows.push({...rows[3].windows[0],id:'extra',name:'A long additional quota window name'});
   });
   await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);
   await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
   await page.evaluate(()=>{state.dashboard.summary.pricedTokens=state.dashboard.summary.total;renderOverview();});
   const usageHeight=(await page.locator('.overview-usage').boundingBox()).height;
   await page.evaluate(()=>{state.dashboard.summary.pricedTokens=0;renderOverview();});
   assert.equal((await page.locator('.overview-usage').boundingBox()).height,usageHeight,'Missing-price warning does not add a row');
   const cards=page.locator('.quota-card[data-provider=codex]:visible');await cards.first().waitFor();
   await page.evaluate(()=>document.querySelectorAll('#content .quota-disclosure').forEach(node=>node.open=false));
   const equal=()=>page.waitForFunction(()=>{
    const rows=[...document.querySelectorAll('#content .quota-card[data-provider=codex]')].filter(x=>x.getBoundingClientRect().width);
    const heights=rows.map(x=>x.getBoundingClientRect().height);
    return rows.length>1&&rows.every(x=>x.style.getPropertyValue('--quota-collapsed-min'))&&Math.max(...heights)-Math.min(...heights)<=1;
   });
   await equal();
   if(surface==='floating.html')assert(await page.evaluate(()=>Math.abs(document.querySelector('#panel').getBoundingClientRect().width-innerWidth)<=1),'Panel fills the first viewport');
   await page.evaluate(()=>{document.documentElement.style.setProperty('--font-secondary','20px');document.documentElement.style.setProperty('--font-card-title','22px');});
   await equal();
   await page.setViewportSize({width:surface==='floating.html'?360:980,height:700});await equal();
   await page.evaluate(async()=>{navigationFixture.quotas.splice(1,1);await loadDashboard();});
   await equal();
   await page.waitForFunction(()=>{
    const rows=[...document.querySelectorAll('#content .quota-card[data-provider=codex],#content .quota-placeholder[data-provider=codex]')].filter(x=>x.getBoundingClientRect().width);
    const heights=rows.map(x=>x.getBoundingClientRect().height);
    return rows.some(x=>x.classList.contains('quota-placeholder'))&&Math.max(...heights)-Math.min(...heights)<=1;
   });
   await cards.first().locator('.quota-title').click();
   await page.waitForFunction(()=>document.querySelector('#content .quota-card[data-provider=codex] .quota-disclosure').open);
   assert.equal(await cards.first().locator('.quota-disclosure').evaluate(x=>x.open),true);
   await page.evaluate(()=>document.querySelector('#content .quota-card[data-provider=codex]').remove());await equal();
   await page.evaluate(()=>{
    const old=state.dashboard.quotas.find(q=>state.settings.accounts.some(a=>a.provider===q.provider&&a.id===q.accountId));
    old.updatedAt=Date.now()/1000+18000;old.error='busy';
    const fresh={...old,updatedAt:Math.floor(Date.now()/1000),error:null,observationRevision:1};
    applyQuotaSamples([fresh]);
    if(state.dashboard.quotas.find(q=>q.accountId===old.accountId).error)throw Error('Future cache blocked success');
    applyQuotaSamples([{...fresh,observationRevision:0,error:'obsolete'}]);
    if(state.dashboard.quotas.find(q=>q.accountId===old.accountId).error)throw Error('Old revision replaced success');
   });
   assert.deepEqual(errors,[]);await page.close();
  }
  console.log('Quota card layout: matching collapsed heights across subscriptions, font changes, resize, expansion and removal; first panel width passed.');
 }finally{if(browser)await browser.close();server.close();}
})().catch(error=>{console.error(error);process.exit(1);});
