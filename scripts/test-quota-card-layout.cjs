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
   // Measure layout, not a page entrance transform still in flight: its sub-pixel offset shows up in the box height.
   const settled=()=>page.evaluate(()=>Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{}))));
   await page.evaluate(()=>{state.dashboard.summary.pricedTokens=state.dashboard.summary.total;renderOverview();});
   await settled();
   const usageHeight=(await page.locator('.overview-usage').boundingBox()).height;
   await page.evaluate(()=>{state.dashboard.summary.pricedTokens=0;renderOverview();});
   await settled();
   // A row would add whole pixels; sub-pixel float noise (77.99999 vs 78) is not a row.
   assert(Math.abs((await page.locator('.overview-usage').boundingBox()).height-usageHeight)<.5,'Missing-price warning does not add a row');
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
   const key = await page.locator('.quota-card:visible').first().getAttribute('data-quota');
   await page.locator('.quota-card:visible').first().click({button:'right'});
   await page.locator('#account-context [role=menuitem]').first().waitFor();
   assert.equal(await page.locator('#account-context [role=menuitem]').count(),3);
   await page.keyboard.press('Escape');
   assert.equal(await page.locator('#account-context').count(),0);
   await page.locator('.quota-card:visible').first().click({button:'right'});
   await page.locator('#account-context [role=menuitem]').first().click();
   await page.waitForFunction(key=>state.settings.accounts.find(a=>a.provider+':'+a.id===key)?.quotaEnabled===false,key);
   assert.equal(await page.locator(`[data-quota="${key}"]`).count(),0);
   const meter = await page.evaluate(() => {
     const probe = document.createElement('div');probe.style.width='320px';
     probe.innerHTML = quotaCard({provider:'qoder-cn',accountId:'probe',name:'Qoder CN',windows:[],identity:{email:'',providerAccountId:'public-user'},meters:[{id:'userQuota',label:'套餐 Credits',unit:'credits',used:0,total:2000,remaining:2000}],updatedAt:1});
     document.getElementById('content').append(probe);
     const value=probe.querySelector('[data-meter] strong');
     return {text:value.textContent,scroll:[value.scrollWidth,value.scrollHeight],client:[value.clientWidth,value.clientHeight],whiteSpace:getComputedStyle(value).whiteSpace,clipped:value.scrollWidth>value.clientWidth+1 || value.scrollHeight>value.clientHeight+1};
   });
   assert.equal(meter.text, '2000 / 2000 Credits');
   assert.equal(meter.clipped,false,'Qoder CN remaining credits must wrap without clipping: '+JSON.stringify(meter));
   assert.deepEqual(errors,[]);await page.close();
  }
  // A partial multi-account group must leave the unused workspace columns free, including its heading hit area.
  for(const platform of ['windows','linux']){
   const page=await browser.newPage({viewport:{width:1600,height:1000},reducedMotion:'reduce'}),errors=[];
   page.on('pageerror',e=>errors.push(String(e)));
   await page.addInitScript(fixture,{platform});
   await page.addInitScript(()=>{
    const f=navigationFixture;
    f.settings.accounts.splice(2,4);f.quotas.splice(2,4);
    const account=f.settings.accounts.find(a=>a.provider==='claude');account.provider='qoder';
    f.settings.sources.find(s=>s.accountId===account.id).provider='qoder';
    f.settings.agents.push({provider:'qoder',enabled:true,machineIds:['local']});
    const q=f.quotas.find(q=>q.provider==='claude');q.provider='qoder';q.windows=[];
    q.meters=[{id:'credits',label:'套餐 Credits',unit:'credits',used:0,total:0,remaining:0}];
   });
   await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);await page.evaluate(()=>AieyesApp.ready);
   const geometry=()=>page.evaluate(()=>{
    const box=el=>{const r=el.getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:r.width};};
    return [...document.querySelectorAll('.quota-group')].map(group=>({provider:group.dataset.provider,heading:box(group.querySelector('summary')),cards:[...group.querySelectorAll('.quota-card')].filter(c=>c.getClientRects().length).map(box)}));
   });
   const settle=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))));
   const check=async width=>{
    await page.setViewportSize({width,height:1000});await settle();
    const groups=await geometry(),[codex,qoder,deepseek]=groups;
    const visible=groups.flatMap(g=>[g.heading,...g.cards]);
    for(let i=0;i<visible.length;i++)for(let j=i+1;j<visible.length;j++){
     const a=visible[i],b=visible[j];
     assert(!(Math.min(a.right,b.right)-Math.max(a.x,b.x)>1&&Math.min(a.bottom,b.bottom)-Math.max(a.y,b.y)>1),`${platform} ${width}px: headings and cards must not overlap: ${JSON.stringify(groups)}`);
    }
    if(width>=1390){
     assert(Math.abs(codex.heading.y-qoder.heading.y)<1,'Qoder heading aligns with the partial Codex group');
     assert(Math.abs(codex.cards[0].y-qoder.cards[0].y)<1,'Qoder card aligns with Codex cards');
     assert(codex.heading.right<qoder.heading.x,'Codex heading hit area ends before Qoder');
     assert(Math.abs(codex.cards[0].width-qoder.cards[0].width)<1,'All accounts retain one workspace column');
     if(width>=1800)assert(Math.abs(deepseek.heading.y-codex.heading.y)<1,'Both single-account groups fill the spare columns');
    }else if(width<974)assert(qoder.heading.y>codex.cards.at(-1).bottom,'Narrow layouts stack provider groups');
   };
   for(const width of [1600,1900,1120,760,1600])await check(width);
   const qoder=page.locator('.quota-group[data-provider=qoder]'),codex=page.locator('.quota-group[data-provider=codex]');
   await qoder.locator('> summary').click();await settle();
   assert.equal(await qoder.evaluate(el=>el.open),false);assert.equal(await codex.evaluate(el=>el.open),true);
   await qoder.locator('> summary').click();await check(1600);
   await codex.locator('.quota-title').first().click();await check(1600);
   // The shorter Codex column still accepts following groups when a sibling card is expanded.
   await page.evaluate(()=>{
    const card=document.querySelector('.quota-card[data-provider=codex]');
    card.style.minHeight='800px';
    const qoder=document.querySelector('.quota-card[data-provider=qoder]');qoder.style.minHeight='800px';
   });await settle();
   const expanded=await geometry();
   assert(Math.abs(expanded[2].heading.x-expanded[0].cards[1].x)<1,'DeepSeek fills the shorter Codex column');
   assert(expanded[2].heading.y>=expanded[0].cards[1].bottom,'Following group clears the shorter card');
   assert(expanded[2].heading.y<expanded[0].cards[0].bottom,'Expanded card does not force a blank row');
   await page.evaluate(()=>{document.documentElement.style.setProperty('--font-secondary','20px');document.documentElement.style.setProperty('--font-card-title','22px');});
   await check(1600);
   await page.evaluate(()=>{
    state.settings.accounts.push({...state.settings.accounts[0],id:'extra'});
    state.dashboard.quotas.push({...state.dashboard.quotas[0],accountId:'extra'});
    renderOverview();
   });
   await page.setViewportSize({width:1900,height:1000});await settle();
   const three=await geometry();
   assert.equal(three[0].cards.length,3);
   assert(Math.abs(three[0].heading.y-three[1].heading.y)<1,'A three-account group leaves the fourth column free');
   assert(three[0].heading.right<three[1].heading.x,'Three-account heading stays in its own columns');
   assert(three[0].cards.every(c=>Math.abs(c.width-three[1].cards[0].width)<1),'Nested three-column grids retain workspace card widths');
   assert.deepEqual(errors,[]);await page.close();
  }
  console.log('Quota card layout: collapsed heights, panel width, partial-group alignment, heading hit areas, resize and masonry expansion passed.');
 }finally{if(browser)await browser.close();server.close();}
})().catch(error=>{console.error(error);process.exit(1);});
