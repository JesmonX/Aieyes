// Shared desktop/panel server rendering with synthetic IPC and no personal data.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.setInterval=()=>0;
  const GiB=1024**3;
  const host={id:'summary-host',name:'多卡训练服务器',target:'fixture.invalid',enabled:true,metrics:['cpu','memory','gpu','network','filesystems'],devices:[],details:['uptime','cpuTimes','memoryCache','swap','gpuMemory','gpuThermals','networkTotals','networkErrors','fsAvailable','fsType','inodes']};
  const sample={timestamp:Date.now()/1000,uptime:90061,load:[1,2,3],errors:{},cpu:[{id:'cpu',utilization:0},{id:'cpu0',utilization:75}],memory:{total:32*GiB,available:24*GiB,cached:GiB,buffers:0,swapTotal:0,swapFree:0},gpu:[
    {id:'0',name:'NVIDIA RTX 4090',utilization:0,memoryUsedMiB:0,memoryTotalMiB:24576,temperature:35,powerWatts:20},
    {id:'1',name:'NVIDIA A100 80GB PCIe',utilization:87,memoryUsedMiB:61440,memoryTotalMiB:81920,temperature:68,powerWatts:270},
    {id:'2',name:'缺失读数的显卡',utilization:null,memoryUsedMiB:null,memoryTotalMiB:0},
  ],filesystems:[{id:'/',used:25*GiB,total:100*GiB,available:75*GiB,type:'ext4'},{id:'/data',used:0,total:GiB,available:GiB},{id:'/missing',used:null,total:0}],network:[{id:'eth0',rxBytes:3*GiB,txBytes:GiB,rxBytesPerSecond:1024,txBytesPerSecond:2048},{id:'ib0',rxBytes:9*GiB,txBytes:0}]};
  window.serverFixture={host,sample,settings:{accounts:[],sources:[],hosts:[host],proxy:{mode:'system',url:''},modelMappings:{},refreshSeconds:300,serverRefreshSeconds:10}};
  const summary={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
  window.__TAURI__={event:{async listen(){return ()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return ()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {platform:'windows',mode:'floating',effectiveMode:'floating',panelOpen:true,page:'agent',sessions:[]};
    if(command!=='engine_call')return {};
    switch(args.method){
      case 'hello':return {version:'test'};
      case 'settings.get':return structuredClone(serverFixture.settings);
      case 'settings.patch':serverFixture.settings=structuredClone(args.params.settings);return structuredClone(serverFixture.settings);
      case 'settings.save':serverFixture.settings=structuredClone(args.params);return {};
      case 'hosts.sample':return [{id:host.id,sample:structuredClone(sample)}];
      case 'dashboard':return {summary,quotas:[],sources:[],models:[],trendDays:[],heatmap:[],dayModels:[],pricingGaps:[],modelOptions:[]};
      default:return [];
    }
  }}};
}
(async()=>{
 const root=path.resolve(__dirname,'../apps/desktop/web'),out=path.resolve(__dirname,'../.local/server-reset/web');fs.mkdirSync(out,{recursive:true});
 const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://test').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}res.setHeader('content-type',({'.js':'application/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
 try{
  browser=await chromium.launch({headless:true});
  for(const surface of ['index.html','floating.html']){
   const panel=surface==='floating.html',page=await browser.newPage({viewport:{width:panel?420:1120,height:800}}),errors=[];
   page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.evaluate(()=>AieyesApp.ready);
   await page.locator('[data-page=servers]').click();await page.waitForFunction(()=>!state.serverBusy);
   const card=page.locator('.server-card'),summary=card.locator(':scope > summary');
   assert.equal(await card.evaluate(el=>el.open),false);
   assert.deepEqual(await summary.locator('.resource-ring strong').allTextContents(),['0.0%','25.0%']);
   assert.equal(await summary.locator('.resource-ring:visible').count(),2);
   assert.match(await summary.locator('[data-gpu="0"]').textContent(),/NVIDIA RTX 4090.*0\/24 GiB/);
   assert.match(await summary.locator('[data-gpu="1"]').textContent(),/NVIDIA A100 80GB PCIe.*60\/80 GiB/);
   assert.equal(await summary.locator('[data-gpu] .server-resource-strip').count(),6);
   assert.equal(await summary.locator('[data-filesystem] .resource-bar').count(),3);
   assert.match(await summary.locator('[data-filesystem="/"]').textContent(),/25\/100 GiB.*25%/);
   assert.match(await summary.locator('[data-host-uptime]').textContent(),/1 天 1 小时 1 分/);
   assert.match(await summary.locator('[data-network-total="eth0"]').textContent(),/3.0 GiB.*1.0 GiB/);
   for(const theme of ['light','dark']){
    await page.emulateMedia({colorScheme:theme});
    for(const width of panel?[450,420,360]:[1120,640]){
     await page.setViewportSize({width,height:800});
     assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
     const overflow=await summary.evaluate(el=>[...el.querySelectorAll('.server-gpu,.resource-gauge,.server-network-total,.server-resource-strip')].filter(child=>child.scrollWidth>child.clientWidth+1).map(child=>child.className));
     assert.deepEqual(overflow,[],`${surface} ${width} ${theme} has overflowing resources`);
     await page.screenshot({path:path.join(out,`${surface}-${width}-${theme}.png`),fullPage:true});
    }
   }
   await summary.click();assert.equal(await card.evaluate(el=>el.open),true);
   assert.equal(await card.locator('.resource-ring').count(),2,'Opening keeps exactly one resource overview');
   assert.equal(await card.locator('[data-metric="summary-host:gpu"] .resource-bar').count(),0);
   assert.equal(await card.locator('[data-metric="summary-host:cpu"] .resource-bar[aria-label="cpu"]').count(),0);
   assert.equal(await card.locator('[data-metric="summary-host:cpu"] .resource-bar[aria-label="cpu0"]').count(),0,'Collapsed groups allocate no device rows');
   assert.equal(await card.locator('[data-metric="summary-host:network"]').textContent().then(t=>t.includes('累计')),false);
   await card.locator('[data-metric="summary-host:cpu"] > summary').click();
   await card.locator('[data-metric="summary-host:cpu"] .resource-bar[aria-label="cpu0"]').waitFor();
   await page.evaluate(()=>sample());assert(await card.evaluate(el=>el.open));assert(await card.locator('[data-metric="summary-host:cpu"]').evaluate(el=>el.open));
   await summary.click();await page.evaluate(()=>sample());assert.equal(await card.evaluate(el=>el.open),false);
   await page.evaluate(()=>{state.settings.hosts[0].devices=['gpu:1','network:ib0','cpu:__none__','filesystems:/data'];renderServers();});
   assert.deepEqual(await card.locator('[data-gpu]').evaluateAll(es=>es.map(e=>e.dataset.gpu)),['1']);
   assert.deepEqual(await card.locator('[data-filesystem]').evaluateAll(es=>es.map(e=>e.dataset.filesystem)),['/data']);
   assert.deepEqual(await card.locator('[data-network-total]').evaluateAll(es=>es.map(e=>e.dataset.networkTotal)),['ib0']);
   assert.equal(await summary.locator('.server-system-metrics .resource-ring').count(),2,'CPU total survives core filtering');
   await page.evaluate(()=>{state.settings.hosts[0].details=[];renderServers();});
   assert.equal(await summary.locator('[data-host-uptime],[data-network-total]').count(),0);
   assert.equal(await summary.locator('[data-gpu] .server-resource-strip').count(),1);
   await page.evaluate(()=>{state.settings.hosts[0].details=['uptime'];state.settings.hosts[0].metrics=[];renderServers();});
   assert.equal(await summary.locator('.resource-ring').count(),0);assert.equal(await summary.locator('[data-host-uptime]').count(),1);
   await page.evaluate(()=>{state.settings.hosts[0]={...structuredClone(serverFixture.host),details:null,devices:[]};state.hosts[0].sample={...structuredClone(serverFixture.sample),gpu:[],memory:{total:0,available:0}};renderServers();});
   assert.equal(await summary.locator('[data-gpu]').count(),0);assert.match(await summary.textContent(),/GPU 无已选设备/);
   assert.deepEqual(await summary.locator('.server-system-metrics .resource-ring strong').allTextContents(),['0.0%','—']);
   await page.evaluate(()=>{state.hosts[0].sample={timestamp:Date.now()/1000,load:[],errors:{gpu:'unavailable'},memory:{total:1024}};renderServers();});
   assert.match(await summary.textContent(),/GPU —/);assert.match(await summary.locator('[data-host-uptime]').textContent(),/—/);
   assert.match(await summary.locator('.server-system-metrics').textContent(),/— \/ 1.0 KiB/);
   assert.doesNotMatch(await card.textContent(),/NaN|Infinity/);
   // The new independent setting remains editable with no collection groups.
   if(!panel){
    await page.evaluate(()=>{state.page='settings';state.settingsTab='hosts';state.settings.hosts[0].metrics=[];state.settings.hosts[0].details=[];render();});
    await page.locator('[data-edit="summary-host"]').click();await page.locator('#detail-select .multi-trigger').click();
    assert.equal(await page.locator('.multi-panel:visible .multi-option').count(),1);
    await page.locator('.multi-panel:visible input[value=uptime]').check();assert.equal(await page.locator('.multi-panel:visible input[value=uptime]').isChecked(),true);if(await page.locator('.multi-panel:visible').count())await page.keyboard.press('Escape');
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
    assert.deepEqual(await page.evaluate(()=>draftSettings().hosts[0].details),['uptime']);
   }
   assert.deepEqual(errors,[]);await page.close();
  }
  console.log('Server summary: per-GPU memory/utilization, device filters, uptime, counters, missing data, disclosure persistence and narrow light/dark layouts passed on both surfaces');
 }finally{await browser?.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
