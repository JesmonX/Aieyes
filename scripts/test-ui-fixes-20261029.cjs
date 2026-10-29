// Behavioral layout regression using production CSS and synthetic data only.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {fixture}=require('./workspace-ui-fixture.cjs');
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),out=path.resolve(__dirname,'../.local/ui-fixes-2026-10-29/web');fs.mkdirSync(out,{recursive:true});
 const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://test').pathname)||'index.html');try{res.setHeader('content-type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));}catch{res.writeHead(404).end();}});
 await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
 try{browser=await chromium.launch();const page=await browser.newPage({viewport:{width:1120,height:850},reducedMotion:'reduce'});await page.addInitScript(fixture,{platform:'windows'});await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(()=>AieyesApp.ready);
 await page.locator('[data-settings-tab=integrations]').click();
 for(const width of [760,900,1120,1440])for(const theme of ['light','dark']){
  await page.setViewportSize({width,height:900});await page.evaluate(theme=>{document.documentElement.dataset.theme=theme;document.querySelectorAll('.settings-disclosure').forEach(d=>d.open=true);},theme);
  const overflow=await page.evaluate(()=>{const group=document.querySelector('.agent-settings-group'),box=group.getBoundingClientRect();return [...group.querySelectorAll('.list-row,input,select,fieldset')].filter(el=>{const r=el.getBoundingClientRect();return r.width>0&&(r.left<box.left-1||r.right>box.right+1)}).map(el=>({class:el.className,tag:el.tagName,left:el.getBoundingClientRect().left,right:el.getBoundingClientRect().right,cardLeft:box.left,cardRight:box.right}));});assert.deepEqual(overflow,[],`settings remain inside card at ${width} ${theme}`);
  assert(await page.locator('[data-settings-agent]').first().isVisible(),'Added entries remain visible at narrow widths');
  const clipped=await page.locator('[data-settings-agent] > span').first().evaluate(el=>getComputedStyle(el).clipPath);assert.equal(clipped,'none');
  await page.screenshot({path:path.join(out,`settings-${width}-${theme}.png`),fullPage:true});
 }
 await page.locator('[data-account-edit]').first().click();
 for(const width of [760,900,1120]){
  await page.setViewportSize({width,height:850});
  const overflow=await page.locator('.account-settings-group').evaluate(group=>{const box=group.getBoundingClientRect();return [...group.querySelectorAll('input,select,textarea,.list-row')].filter(el=>{const r=el.getBoundingClientRect();return r.width>0&&(r.left<box.left-1||r.right>box.right+1);}).map(el=>el.tagName);});
  assert.deepEqual(overflow,[],`Account settings stay inside shared card at ${width}`);
 }
 await page.evaluate(()=>{state.dashboard.quotas[0].bankReset={availableCount:2,credits:[]};state.dashboard.quotas[0].plan='plus';state.page='agent';render();document.querySelector('.quota-disclosure').open=true;});await page.screenshot({path:path.join(out,'accounts-windows.png'),fullPage:true});
 assert.equal(await page.locator('.quota-card').first().locator('.bank summary .ui-icon').count(),1);
 await page.locator('.quota-card').first().screenshot({path:path.join(out,'account-expanded-windows.png')});
 console.log('Windows settings bounds, entry labels and light/dark layout passed at 760/900/1120/1440px');
 }finally{await browser?.close();server.close();}
})().catch(e=>{console.error(e);process.exit(1)});
