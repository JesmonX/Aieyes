/* Actual Tauri frontend, production English catalog, synthetic IPC only. */
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {execFileSync}=require('node:child_process');
const {chromium}=require('../../apps/desktop/node_modules/playwright');
const {encode}=require('./encode.cjs');
const root=path.resolve(__dirname,'../..'),web=path.join(root,'apps/desktop/web');
const data=JSON.parse(execFileSync('python3',[path.join(__dirname,'fixture.py'),'--fixture'],{encoding:'utf8',maxBuffer:16*1024*1024}));
function install(data){
  const NativeDate=Date;
  window.Date=class extends NativeDate{constructor(...args){super(...(args.length?args:[1792584060000]));}static now(){return 1792584060000;}};
  window.setInterval=()=>0;
  const listeners={};
  const info={platform:'windows',page:'agent',mode:'floating',effectiveMode:'floating',nativePet:true,panelOpen:true,panelPinned:false,material:'acrylic',materials:{main:'acrylic',floating:'acrylic'},sessions:data.sessions.sessions,activeCount:3,summary:'3 active sessions'};
  const results={'hello':{version:'preview'},'settings.get':data.settings,'dashboard':data.dashboard,'dashboard.summary':data.dashboard,'dashboard.status':data.dashboard,'prices.list':data.prices,'hosts.sample':data.hosts,'sessions.list':data.sessions,'wakeups.list':data.wakeups,'wakeups.status':{installed:true,timezone:'UTC+08:00',nextRunAt:1792629000,history:[{startedAt:1792542600,status:'success',model:'gpt-5.1-codex',effort:'medium',message:'Completed'}]},'quotas.refresh':data.dashboard.quotas,'sources.scan':[],'accounts.status.get':data.accountStatuses,'accounts.targets':[],'accounts.cleanup.list':[],'network.status':data.network,'network.test':data.network,'pets.list':data.petLibrary,'pets.asset':data.petAsset,'codexApi.status':{ready:true,installed:true},'codexAuth.inspect':{profiles:[],processes:[],storageMode:'file'},'quotaEstimates.list':[],'wakeups.history':[]};
  window.__TAURI__={event:{async listen(name,fn){(listeners[name]||=[]).push(fn);return()=>{};}},window:{getCurrentWindow:()=>({isMaximized:async()=>false,onResized:async()=>()=>{}})},core:{async invoke(command,args){
    if(command==='desktop_info'||command==='desktop_panel')return structuredClone(info);
    if(command==='updates_info')return{currentVersion:'preview',phase:'idle',automatic:true};
    if(command!=='engine_call')return{};
    return structuredClone(results[args.method]??{});
  }}};
  localStorage.setItem('aieyes.appearance',JSON.stringify(data.settings.appearance));
  localStorage.setItem('aieyes.language','en');
  localStorage.setItem('quota.expanded.v2.detail.codex:codex','true');
  localStorage.setItem('quota.expanded.v2.panel.codex:codex','true');
}
(async()=>{
  const server=http.createServer((req,res)=>{
    const url=new URL(req.url,'http://localhost'),name=decodeURIComponent(url.pathname==='/'?'/index.html':url.pathname);
    const file=path.resolve(web,'.'+name);
    if(!file.startsWith(web+path.sep)||!fs.existsSync(file)||!fs.statSync(file).isFile()){res.writeHead(404);return res.end();}
    const ext=path.extname(file);res.setHeader('Content-Type',({'.html':'text/html; charset=utf-8','.js':'text/javascript; charset=utf-8','.css':'text/css','.png':'image/png','.ttf':'font/ttf'})[ext]||'application/octet-stream');
    res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser=await chromium.launch(),origin=`http://127.0.0.1:${server.address().port}`;
  const out=path.join(root,'apps/website/public/screenshots');fs.mkdirSync(out,{recursive:true});
  const context=await browser.newContext({viewport:{width:1360,height:1000},deviceScaleFactor:1,colorScheme:'dark',locale:'en-US',reducedMotion:'reduce'});
  await context.addInitScript(install,data);
  const errors=[],unknown={};
  try{
    const page=await context.newPage();page.on('pageerror',e=>errors.push(String(e)));
    await page.goto(origin);await page.evaluate(()=>AieyesApp.ready);await page.evaluate(()=>AieyesTheme.loadFonts());
    const shots=[['overview','agent'],['analytics','analysis'],['servers','servers']];
    for(const [feature,target,tab] of shots){
      await page.evaluate(async({target,tab,feature})=>{
        if(tab)await AieyesApp.openSettings({category:tab,provider:'codex'});
        else await AieyesApp.navigatePage(target);
        if(target==='servers')await sample();
        if(target==='analysis'){state.days=7;await loadDashboard();}
        document.querySelector('#live-sessions').open=feature==='sessions';
      },{target,tab,feature});
      if(feature==='wakeups')await page.locator('[data-wake-row]').waitFor();
      await page.evaluate(async()=>{await AieyesTheme.loadFonts();await document.fonts.ready;});
      await page.waitForTimeout(220);
      unknown[feature]=await page.locator('body').innerText();
      const png=await page.screenshot({path:path.join(root,'.local/website',`windows-${feature}.png`),animations:'disabled'});
      fs.writeFileSync(path.join(out,`windows-${feature}.webp`),await encode(page,png));
      console.log(`Rendered windows-${feature}`);
    }
    const panel=await context.newPage();await panel.setViewportSize({width:450,height:720});
    await panel.goto(origin+'/floating.html');await panel.evaluate(()=>AieyesApp.ready);await panel.evaluate(()=>AieyesTheme.loadFonts());
    const png=await panel.screenshot({path:path.join(root,'.local/website/windows-panel.png'),animations:'disabled'});
    fs.writeFileSync(path.join(out,'windows-panel.webp'),await encode(panel,png));
    fs.writeFileSync(path.join(root,'.local/website/windows-text.json'),JSON.stringify(unknown,null,2));
    if(errors.length)throw Error(errors.join('\n'));
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
