/* Geometry checks use the actual Windows/Linux frontend and retained fixtures. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {fixture}=require('./workspace-ui-fixture.cjs');
const {serveFont}=require('./ui-test-helpers.cjs');
const root=path.resolve(__dirname,'..'),web=path.join(root,'apps/desktop/web'),out=path.join(root,'.local/bilingual-layout');
const selectors=['nav button','.settings-sidebar button','.stat-label > span','.stat-value','.account-switch-entry','.metric-picker button','.panel-tabs button','.resource-strip-heading > strong','.quota-window .between > strong'];
function measure(selectors){
  const rows=[];
  for(const selector of selectors)document.querySelectorAll(selector).forEach((el,index)=>{
    if(!el.checkVisibility({checkVisibilityCSS:true}))return;
    const range=document.createRange(),walker=document.createTreeWalker(el,NodeFilter.SHOW_TEXT),rects=[];let node;
    while((node=walker.nextNode())){if(!node.textContent.trim()||node.parentElement.closest('svg'))continue;range.selectNodeContents(node);rects.push(...[...range.getClientRects()].filter(r=>r.width>.5&&r.height>.5));}
    const box=el.getBoundingClientRect(),lines=new Set(rects.map(r=>Math.round((r.top-box.top)/4)));
    const text=el.innerText.trim();
    const clipped=el.scrollWidth>el.clientWidth+2||rects.some(r=>r.left<box.left-2||r.right>box.right+2);
    rows.push({key:selector+':'+index,text,lines:lines.size,height:Math.round(box.height),clipped});
  });
  return {overflow:document.documentElement.scrollWidth>innerWidth+1,rows};
}
(async()=>{
  fs.mkdirSync(out,{recursive:true});
  const server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`,browser=await chromium.launch(),reports=[],failures=[];
  try{
    for(const platform of ['windows','linux'])for(const theme of ['light','dark']){
      const context=await browser.newContext({reducedMotion:'reduce',colorScheme:theme});
      await context.addInitScript(fixture,{platform});
      await context.addInitScript(theme=>{
        const s=navigationFixture.settings;s.appearance={...s.appearance,theme,language:'en'};
        s.accounts[0].name='A long account name 用户账户';s.sources[0].path='/资料/long workspace/path';
        navigationFixture.quotas[0].name=s.accounts[0].name;navigationFixture.quotas[0].metadataError='读取失败';
        localStorage.setItem('aieyes.language','en');localStorage.setItem('aieyes.appearance',JSON.stringify(s.appearance));
        localStorage.setItem('quota.expanded.v2.detail.codex:a0','true');localStorage.setItem('quota.expanded.v2.panel.codex:a0','true');
      },theme);
      const page=await context.newPage();
      for(const [kind,width] of [['main',760],['main',1120],['main',1440],['panel',360],['panel',450]]){
        await page.setViewportSize({width,height:900});await page.goto(origin+(kind==='panel'?'/floating.html':'/'));await page.evaluate(()=>AieyesApp.ready);await page.evaluate(()=>AieyesTheme.loadFonts());
        const targets=kind==='panel'?['agent']:['agent','analysis','servers','integrations','refresh','network'];
        for(const target of targets){
          await page.evaluate(async target=>{if(['refresh','network'].includes(target))await AieyesApp.openSettings({category:'general',section:target});else if(target==='integrations')await AieyesApp.openSettings({category:target,provider:'codex'});else await AieyesApp.navigatePage(target);},target);
          const pair={platform,theme,kind,width,target};
          for(const language of ['en','zh-CN']){
            await page.evaluate(language=>setLanguage(language),language);await page.evaluate(()=>document.fonts.ready);
            pair[language]=await page.evaluate(measure,selectors);
            if(pair[language].overflow)failures.push({...pair,issue:'page overflow',language});
            for(const row of pair[language].rows)if(row.clipped)failures.push({platform,theme,kind,width,target,language,issue:'clipped control',...row});
          }
          const zh=new Map(pair['zh-CN'].rows.map(r=>[r.key,r]));
          for(const en of pair.en.rows){const other=zh.get(en.key);if(other&&en.lines!==other.lines)failures.push({platform,theme,kind,width,target,issue:'different line counts',en,zh:other});}
          reports.push(pair);
          if(platform==='windows'&&theme==='dark'&&(target==='agent'||target==='integrations'))await page.screenshot({path:path.join(out,`${kind}-${width}-${target}.png`)});
        }
      }
      await context.close();
    }
  }finally{fs.writeFileSync(path.join(out,'report.json'),JSON.stringify({reports,failures},null,2));await browser.close();await new Promise(r=>server.close(r));}
  assert.equal(failures.length,0,JSON.stringify(failures.slice(0,12),null,2));
  console.log(`Bilingual layout passed: ${reports.length} language pairs, light/dark, Windows/Linux, main 760/1120/1440 and panel 360/450.`);
})().catch(error=>{console.error(error);process.exitCode=1;});
