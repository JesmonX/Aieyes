// Real production DOM/layout with disposable synthetic IPC, never live accounts.
// node scripts/benchmark-interactions-ui.cjs [output.json]
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const {chromium} = require('../apps/desktop/node_modules/playwright');
const {fixture} = require('./workspace-ui-fixture.cjs');
const root = path.resolve(__dirname, '../apps/desktop/web');
const output = path.resolve(process.argv[2] || '.local/performance-ui/interactions.json');
const counts = (process.env.AIEYES_PERF_ACCOUNTS || '8,32,128').split(',').map(Number);
const iterations = Number(process.env.AIEYES_PERF_ITERATIONS || 8);
const reducedMotion = process.env.AIEYES_PERF_REDUCED_MOTION || 'reduce';
const layoutScript = process.env.AIEYES_QUOTA_LAYOUT_SCRIPT || path.join(root,'quota-card-layout.js');
const quantile = (values, p) => [...values].sort((a,b) => a-b)[Math.ceil(values.length*p)-1];
(async () => {
  const server = http.createServer((req,res) => {
    if (require('./ui-test-helpers.cjs').serveFont(req,res)) return;
    const name = path.basename(new URL(req.url,'http://fixture').pathname) || 'index.html';
    const file = name === 'quota-card-layout.js' ? layoutScript : path.join(root,name);
    try { res.setHeader('content-type', ({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)] || 'application/octet-stream'); res.end(fs.readFileSync(file)); }
    catch { res.writeHead(404); res.end(); }
  });
  await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
  const browser = await chromium.launch({headless:true});
  const report = {startedAt:new Date().toISOString(), browser:browser.version(), reducedMotion, layoutScriptSha256:require('node:crypto').createHash('sha256').update(fs.readFileSync(layoutScript)).digest('hex'), samples:[], errors:[], note:'Headless Chromium production renderer with fixture RPCs. Render latency includes three animation frames; this does not measure native WebView or network latency.'};
  try {
    for (const count of counts) {
      const page = await browser.newPage({viewport:{width:1600,height:900}, reducedMotion});
      page.on('pageerror', error => report.errors.push(String(error)));
      await page.addInitScript(fixture,{platform:'windows'});
      await page.addInitScript(count => {
        const nav = navigationFixture, stamp = Math.floor(Date.now()/1000);
        const account = nav.settings.accounts[0], quota = nav.quotas[0], source = nav.settings.sources[0];
        nav.settings.accounts.splice(0, nav.settings.accounts.length, ...Array.from({length:count},(_,i) => ({...account,id:'perf-'+i,name:'Synthetic account '+i})));
        nav.settings.sources.splice(0,nav.settings.sources.length,...nav.settings.accounts.map(a => ({...source,id:'s-'+a.id,accountId:a.id,path:'/fixture/'+a.id})));
        nav.quotas.splice(0,nav.quotas.length,...nav.settings.accounts.map((a,i) => ({...structuredClone(quota),accountId:a.id,name:a.name,sourceId:'s-'+a.id,updatedAt:stamp,windows:quota.windows.map(w=>({...w,usedPercent:10+i%80}))})));
      },count);
      const cdp = await page.context().newCDPSession(page);
      await cdp.send('Performance.enable');
      const metrics = async () => Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m=>[m.name,m.value]));
      await page.goto(`http://127.0.0.1:${server.address().port}`); await page.evaluate(()=>AieyesApp.ready);
      await page.evaluate(async()=>{refreshStopped=true;clearTimeout(refreshTimer);await document.fonts.ready;await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));});
      const row = {accounts:count, renders:[], expansions:[], scroll:[], navigation:[]};
      for(let iteration=0;iteration<iterations;iteration++) {
        const before=await metrics();
        const ms=await page.evaluate(async iteration=>{
          state.dashboard.quotas.forEach(q=>{q.windows[0].usedPercent=20+iteration;});
          const started=performance.now();renderOverview();
          await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>requestAnimationFrame(r))));
          return performance.now()-started;
        },iteration);
        const after=await metrics();
        row.renders.push({ms,layouts:after.LayoutCount-before.LayoutCount,layoutMs:1000*(after.LayoutDuration-before.LayoutDuration),scriptMs:1000*(after.ScriptDuration-before.ScriptDuration)});
        row.expansions.push(await page.evaluate(async()=>{
          const started=performance.now();document.querySelector('.quota-title').click();
          await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>requestAnimationFrame(r))));
          return performance.now()-started;
        }));
      }
      row.scroll=await page.evaluate(async()=>{
        const scroller=pageScroller(),deltas=[];let last=performance.now();
        for(let frame=0;frame<120;frame++){
          scroller.scrollTop=(frame<60?frame:120-frame)*35;
          await new Promise(r=>requestAnimationFrame(r));const now=performance.now();deltas.push(now-last);last=now;
        }return deltas;
      });
      row.animationFrames=await page.evaluate(async()=>{
        const deltas=[];let last=performance.now();document.querySelector('.quota-title').click();
        for(let frame=0;frame<30;frame++) {await new Promise(r=>requestAnimationFrame(r));const now=performance.now();deltas.push(now-last);last=now;}
        return deltas;
      });
      for(const destination of ['analysis','agent','settings','agent']) row.navigation.push(await page.evaluate(async destination=>{
        const start=performance.now();await AieyesApp.navigatePage(destination);
        await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
        return {destination,ms:performance.now()-start};
      },destination));
      row.summary={renderMedianMs:quantile(row.renders.map(r=>r.ms),.5),renderP95Ms:quantile(row.renders.map(r=>r.ms),.95),medianLayoutPasses:quantile(row.renders.map(r=>r.layouts),.5),expansionP95Ms:quantile(row.expansions,.95),scrollFrameP95Ms:quantile(row.scroll,.95),scrollFramesOver33ms:row.scroll.filter(t=>t>33.34).length,animationFrameP95Ms:quantile(row.animationFrames,.95),animationFramesOver33ms:row.animationFrames.filter(t=>t>33.34).length};
      report.samples.push(row);fs.mkdirSync(path.dirname(output),{recursive:true});fs.writeFileSync(output,JSON.stringify(report,null,2)+'\n');
      console.log(JSON.stringify({accounts:count,...row.summary})); await page.close();
    }
    if(report.errors.length)throw new Error(report.errors.join('\n'));
    report.complete=true;fs.writeFileSync(output,JSON.stringify(report,null,2)+'\n');
  } catch(error) {
    report.complete=false;report.failure=String(error);fs.mkdirSync(path.dirname(output),{recursive:true});fs.writeFileSync(output,JSON.stringify(report,null,2)+'\n');throw error;
  } finally { await browser.close(); await new Promise(resolve=>server.close(resolve)); }
})().catch(error=>{console.error(error);process.exitCode=1;});
