const {discardEditor,commitSettings}=require('./ui-test-helpers.cjs');
// Regression coverage for the cross-platform interaction review. Uses only in-memory IPC.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require(process.env.AIEYES_PLAYWRIGHT || '../apps/desktop/node_modules/playwright');
const root = path.resolve(__dirname, '../apps/desktop/web');
function fixture() {
  const tokens={input:12000,output:3000,cacheRead:5000,cacheWrite:0};
  const usage={tokens,total:20000,events:3,pricedTokens:20000,cost:0.04};
  const empty={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,events:0,pricedTokens:0,cost:0};
  const host={id:'host',name:'训练服务器',target:'gpu-lab',enabled:true,port:null,identityFile:'',shell:'/bin/bash',preCommand:'',metrics:['cpu','memory','network'],devices:[],details:['cpuTimes','memoryCache','networkTotals']};
  const sample={timestamp:Date.now()/1000,load:[0.1,0.2,0.3],errors:{},cpu:[{id:'cpu',utilization:42}],memory:{total:16000000000,available:4000000000}};
  window.review={empty:true,failScan:false,failQuota:false,failHosts:false,quotaRows:[],scanRows:[],saved:[],actions:[],calls:[],settings:{accounts:[],sources:[],hosts:[host],modelMappings:{},proxy:{mode:'system',url:''},refreshSeconds:300,serverRefreshSeconds:10},hosts:[{id:'host',sample}]};
  window.reviewListeners={};window.reviewEmit=(name,payload)=>Promise.all((window.reviewListeners[name]??[]).map(fn=>fn({payload})));
  window.reviewTimers=[];window.setInterval=(fn,ms)=>{window.reviewTimers.push({fn,ms});return window.reviewTimers.length;};
  let panelOpen=false;
  const info=()=>({platform:'windows',material:'opaque',mode:'auto',effectiveMode:'floating',sessions:[],summary:'暂无活跃会话',activeCount:0,panelOpen,panelPinned:false,hidden:false});
  window.__TAURI__={event:{async listen(name,fn){(window.reviewListeners[name]??=[]).push(fn);return ()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return ()=>{};},async minimize(){},async toggleMaximize(){},async close(){}})},core:{async invoke(command,args){
    if(command==='desktop_info')return info();
    if(command==='desktop_panel'){panelOpen=args.open;return info();}
    if(command==='desktop_action'){window.review.actions.push(args.action);return info();}
    if(command!=='engine_call')return {};
    const r=window.review;r.calls.push(args.method);
    switch(args.method){
      case 'hello':return {version:'test'};
      case 'settings.get':return structuredClone(r.settings);
      case 'settings.save':r.settings=structuredClone(args.params);r.saved.push(structuredClone(args.params));return {};
      case 'sources.scan':if(r.failScan)throw new Error('测试扫描失败');return r.scanRows;
      case 'quotas.refresh':if(r.failQuota)throw new Error('测试限额失败');return r.quotaRows;
      case 'hosts.sample':if(r.failHosts)throw new Error('测试采样 RPC 失败');return structuredClone(r.hosts);
      case 'prices.list':return [{id:'model-a',name:'Model A',input:0.000003,output:0.000006}];
      case 'prices.save':return {};
      case 'dashboard':{
        const models=args.params.model?[args.params.model]:['Model A','Model B'];
        const summary=r.empty?empty:usage;
        const days=Array.from({length:Math.max(7,args.params.days||1)},(_,i)=>({...summary,key:new Date(Date.UTC(2026,8,1+i)).toISOString().slice(0,10)}));
        return {summary,quotas:r.quotaRows,modelOptions:['Model A','Model B'],models:r.empty?[]:models.map(key=>({...usage,key})),dayModels:r.empty?[]:days.flatMap(day=>models.map(model=>({day:day.key,model,usage}))),trendDays:days,heatmap:days,sources:[],pricingGaps:[]};
      }
      default:return {};
    }
  }}};
}
(async()=>{
  const server=http.createServer((req,res)=>{
    const name=path.basename(new URL(req.url,'http://localhost').pathname)||'index.html';
    const file=path.join(root,name);
    if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}
    res.setHeader('content-type',({'.html':'text/html','.css':'text/css','.js':'application/javascript'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  let browser;
  try {
    browser=await chromium.launch({headless:true});
    const page=await browser.newPage({viewport:{width:1120,height:800}});const errors=[];
    page.on('pageerror',error=>errors.push(String(error)));await page.addInitScript(fixture);
    const base=`http://127.0.0.1:${server.address().port}`;await page.goto(base);
    await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    assert.equal(await page.locator('#content > .onboarding:first-child').count(),1);
    assert.equal(await page.locator('#trend').count(),0);
    const previews=path.resolve(__dirname,'../.local/ui-previews');fs.mkdirSync(previews,{recursive:true});
    await page.screenshot({animations:'disabled',path:path.join(previews,'onboarding-fixed.png')});
    // Critical small-text tokens and the primary action remain legible in both themes.
    for(const colorScheme of ['light','dark']){
      await page.emulateMedia({colorScheme});
      const contrasts=await page.evaluate(()=>{
        const style=getComputedStyle(document.documentElement),rgb=name=>style.getPropertyValue(name).trim().slice(1).match(/../g).map(v=>parseInt(v,16));
        const lum=values=>values.map(v=>{v/=255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4;}).reduce((n,v,i)=>n+v*[.2126,.7152,.0722][i],0);
        const contrast=(a,b)=>(Math.max(lum(a),lum(b))+.05)/(Math.min(lum(a),lum(b))+.05);
        const canvas=document.createElement('canvas'),ctx=canvas.getContext('2d');
        // Model colors are data graphics: check many hashes against the surface.
        for(let i=0;i<10000;i++){
          ctx.fillStyle=color('contrast-model-'+i);ctx.fillRect(0,0,1,1);
          const ratio=contrast([...ctx.getImageData(0,0,1,1).data].slice(0,3),rgb('--solid'));
          if(ratio<3)throw new Error('Model graphic contrast '+ratio);
        }
        return [...['--muted','--faint','--ok','--warn','--danger'].map(name=>[name,contrast(rgb(name),rgb('--solid'))]),['primary',contrast(rgb('--accent-ink'),rgb('--accent').map(v=>v*.97+255*.03))]];
      });
      for(const [name,value] of contrasts)assert.ok(value>=4.5,`${colorScheme} ${name} contrast ${value}`);
    }
    await page.emulateMedia({colorScheme:'light'});
    await page.locator('#add-first-source').click();await page.locator('#field-provider').selectOption('deepseek');
    assert.equal(await page.locator('#field-isAccount').isChecked(),true);
    assert.match(await page.locator('#account-help').textContent(),/余额/);
    await discardEditor(page);assert.equal(await page.evaluate(()=>state.settings.sources.length),0);
    await page.locator('[data-settings-tab=hosts]').click();await page.locator('[data-edit=host]').click();
    await page.locator('#field-name').focus();let advanced=false,save=false;
    for(let i=0;i<24;i++){
      const target=await page.evaluate(()=>({tag:document.activeElement.tagName,submit:document.activeElement.type==='submit'}));
      advanced ||= target.tag==='SUMMARY';save ||= target.submit;await page.keyboard.press('Tab');
    }
    assert.equal(advanced,true,'Tab must reach Advanced settings');assert.equal(save,true,'Tab must reach Save past closed details');
    await page.locator('.advanced > summary').focus();await page.keyboard.press('Space');assert.equal(await page.locator('.advanced').evaluate(el=>el.open),true);
    await page.keyboard.press('Tab');assert.equal(await page.locator('#field-shell').evaluate(el=>el===document.activeElement),true);
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
    assert.equal(await page.locator('[data-edit=host]').evaluate(el=>el===document.activeElement),true,'Save must return focus to the rebuilt row');
    await page.evaluate(()=>{state.settings.sources=[{id:'remote',name:'远端记录',provider:'codex',path:'~/.codex',accountId:'',hostId:'host',enabled:true}];review.settings=structuredClone(state.settings);state.settingsDraft=null;state.settingsBaseline=null;renderSettings();});
    await page.locator('[data-remove=host]').click();assert.match(await page.locator('#editor-fields').textContent(),/远端记录/);
    assert.equal(await page.evaluate(()=>state.settings.hosts.length),1);await discardEditor(page);
    await page.locator('[data-remove=host]').click();await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);await commitSettings(page);
    assert.deepEqual(await page.evaluate(()=>[state.settings.hosts.length,state.settings.sources[0].hostId,state.settings.sources[0].enabled]),[0,null,false]);
    assert.equal(await page.locator('.settings-draft-label').evaluate(el=>el===document.activeElement),true);
    await page.evaluate(()=>{review.empty=false;});await page.locator('[data-page=agent]').click();await page.evaluate(()=>loadDashboard());
    await page.locator('#model').selectOption('Model A');await page.waitForFunction(()=>state.dashboard.dayModels.every(row=>row.model==='Model A'));
    assert.deepEqual(await page.locator('#model option').allTextContents(),['全部模型','Model A','Model B']);
    await page.locator('#model').selectOption('Model B');
    const lastSuccess=await page.evaluate(()=>state.lastSuccessfulUpdate);
    await page.evaluate(async()=>{review.failScan=true;await scan();});
    assert.equal(await page.evaluate(()=>state.lastSuccessfulUpdate),lastSuccess);assert.match(await page.locator('#activity').textContent(),/失败/);
    await page.evaluate(()=>{state.settings.accounts=[{id:'account',provider:'deepseek',name:'余额账户',quotaEnabled:true}];state.settings.sources.push({id:'balance',provider:'deepseek',accountId:'account',enabled:true,path:''});state.provider='';state.accountKey='';render();});
    assert.match(await page.locator('.quota-placeholder').textContent(),/尚未读取/);
    await page.evaluate(async()=>{review.failQuota=true;await quotas();});assert.match(await page.locator('.quota-placeholder').textContent(),/读取失败/);
    await page.evaluate(async()=>{
      review.failQuota=false;review.quotaRows=[{provider:'deepseek',accountId:'account',name:'余额账户',updatedAt:Date.now()/1000,windows:[],error:'账户凭证失效'},{provider:'codex',accountId:'healthy',name:'可用账户',updatedAt:Date.now()/1000,windows:[],error:null}];
      await quotas();
    });
    assert.equal(await page.evaluate(()=>state.lastSuccessfulUpdate),lastSuccess);
    assert.equal(await page.locator('#activity').getAttribute('data-status'),'error');
    assert.match(await page.locator('.quotas').textContent(),/账户凭证失效/);
    await page.evaluate(async()=>{review.failScan=false;review.scanRows=[{id:'remote',error:'远程记录读取失败'}];await scan();});
    assert.equal(await page.evaluate(()=>state.lastSuccessfulUpdate),lastSuccess);assert.match(await page.locator('#message').textContent(),/远程记录读取失败/);
    await page.locator('[data-page=settings]').click();await page.locator('[data-settings-tab=prices]').click();await page.locator('#add-price').click();
    assert.equal(await page.locator('#field-id').count(),1);assert.deepEqual(await page.locator('#field-id').evaluate(el=>[...el.labels].map(label=>label.textContent)),['模型 ID']);
    await discardEditor(page);await page.locator('[data-settings-tab=prices]').focus();await page.keyboard.press('ArrowLeft');
    assert.equal(await page.locator('[data-settings-tab=hosts]').getAttribute('aria-selected'),'true');
    await page.evaluate(()=>{state.settings.hosts=structuredClone(review.settings.hosts);state.settings.hosts=[{id:'host',name:'训练服务器',target:'gpu-lab',enabled:true,details:[]}];});
    await page.locator('[data-page=servers]').click();await page.waitForFunction(()=>!state.serverBusy);
    await page.locator('.server-card > summary').click();
    await page.locator('[data-metric="host:cpu"] > summary').click();await page.locator('[data-metric="host:cpu"] > summary').focus();await page.evaluate(()=>sample());
    assert.equal(await page.locator('[data-metric="host:cpu"] > summary').evaluate(el=>el===document.activeElement),true);
    assert.equal(await page.locator('[data-metric="host:cpu"]').evaluate(el=>el.open),true);
    await page.evaluate(()=>{
      const sample=structuredClone(review.hosts[0].sample);sample.cpu[0].utilization=77;
      applyHostSamples([{id:'host',error:'部分采样暂不可用',sample}]);
    });
    assert.equal(await page.locator('.resource-ring strong').first().textContent(),'77.0%','An error row retains its own valid sample');
    const beforeFailure=Date.now();await page.evaluate(async()=>{review.failHosts=true;await sample();});
    assert.match(await page.locator('.server-card').textContent(),/测试采样 RPC 失败/);
    assert.equal(await page.locator('.resource-ring strong').first().textContent(),'77.0%');
    assert.ok(await page.evaluate(stamp=>state.hosts[0].lastAttemptAt>=stamp,beforeFailure));
    await page.evaluate(async()=>{review.failHosts=false;await sample();});
    assert.doesNotMatch(await page.locator('#message').textContent(),/测试采样 RPC 失败/,'A successful host retry clears its obsolete failure');
    const panel=await browser.newPage({viewport:{width:420,height:640}});panel.on('pageerror',error=>errors.push(String(error)));await panel.addInitScript(fixture);
    await panel.goto(base+'/floating.html');await panel.locator('#ball').click();
    assert.equal(await panel.locator('#panel').evaluate(el=>el.clientWidth),420,'Panel must fill its window');
    await panel.evaluate(()=>reviewEmit('desktop:refresh-status',{key:'scan',busy:true}));
    assert.match(await panel.locator('#activity').textContent(),/同步记录中/);
    await panel.locator('#panel-refresh').click();assert.equal(await panel.locator('[data-refresh=scan]').isDisabled(),true);await panel.keyboard.press('Escape');
    await panel.evaluate(()=>reviewEmit('desktop:refresh-status',{key:'scan',busy:false,success:null,failures:[{id:'remote',error:'另一窗口同步失败'}]}));
    assert.match(await panel.locator('.error-list').textContent(),/另一窗口同步失败/);
    await panel.evaluate(()=>reviewEmit('desktop:refresh-status',{key:'scan',busy:false,success:Date.now(),failures:[]}));
    assert.equal(await panel.locator('.error-list').count(),0);
    await panel.locator('#panel-more > summary').click();await panel.locator('#panel-quit').waitFor();
    await panel.keyboard.press('Escape');assert.equal(await panel.locator('#panel-more').evaluate(el=>el.open),false);
    assert.equal(await panel.locator('body').getAttribute('data-view'),'panel','Esc dismisses More before the panel');
    await panel.locator('#add-first-quota').click();assert.ok((await panel.evaluate(()=>review.actions)).includes('add-quota'));
    await panel.locator('#add-first-source').click();
    assert.ok((await panel.evaluate(()=>review.actions)).includes('add-source'));
    await panel.evaluate(async()=>{review.empty=false;review.settings.sources=[{id:'preview',name:'示例日志',provider:'codex',path:'~/.codex',enabled:false,accountId:''}];await reviewEmit('desktop:settings',null);});
    await panel.screenshot({animations:'disabled',path:path.join(previews,'floating-panel-fixed-light.png')});
    await panel.emulateMedia({colorScheme:'dark'});await panel.screenshot({animations:'disabled',path:path.join(previews,'floating-panel-fixed-dark.png')});await panel.emulateMedia({colorScheme:'light'});
    assert.equal(await panel.locator('body').getAttribute('data-view'),'panel','Changing theme preserves the current panel state');
    await panel.evaluate(async()=>{review.empty=false;await loadDashboard();});await panel.locator('[data-agent-detail=panel-trend] > summary').click();await panel.locator('[data-day-toggle]').first().click();await panel.locator('[data-day-toggle]').first().focus();
    await panel.evaluate(()=>loadDashboard());assert.equal(await panel.locator('[data-agent-detail][open]').count(),2);
    assert.equal(await panel.locator('[data-day-toggle]').first().evaluate(el=>el===document.activeElement),true);
    assert.equal(await panel.evaluate(()=>state.days),1,'Glance always shows today; historical range belongs to details');
    await panel.evaluate(async()=>{
      review.settings.sources=[{id:'new-source',name:'另一窗口添加的来源',provider:'codex',path:'~/.codex',enabled:true,accountId:''}];
      await reviewEmit('desktop:settings',null);
    });
    assert.equal(await panel.evaluate(()=>state.settings.sources[0].id),'new-source');
    await panel.locator('[data-page=servers]').click();await panel.waitForFunction(()=>!state.serverBusy);
    await panel.evaluate(async()=>{const rows=structuredClone(review.hosts);rows[0].sample.cpu[0].utilization=91;await reviewEmit('desktop:hosts',rows);});
    assert.equal(await panel.locator('.resource-ring strong').first().textContent(),'91.0%');
    await panel.locator('.server-card > summary').click();await panel.locator('[data-metric="host:cpu"] > summary').focus();
    await panel.evaluate(()=>{window.previousCard=document.querySelector('.server-card');state.serverBusy=true;state.hosts[0].sample.timestamp=Date.now()/1000-11;reviewTimers.find(timer=>timer.ms===1000).fn();});
    assert.equal(await panel.locator('.host-status').textContent(),'数据已延迟');
    assert.equal(await panel.locator('.server-card').evaluate(el=>el===window.previousCard),true,'Aging must not recreate the server card');
    assert.equal(await panel.locator('[data-metric="host:cpu"] > summary').evaluate(el=>el===document.activeElement),true);
    await panel.evaluate(async()=>{state.serverBusy=false;await reviewEmit('desktop:hosts-error','后台采样 RPC 失败');});
    assert.equal(await panel.locator('.host-status').textContent(),'连接失败');
    assert.equal(await panel.locator('.resource-ring strong').first().textContent(),'91.0%');
    const panelSuccess=await panel.evaluate(()=>state.lastSuccessfulUpdate);
    await panel.evaluate(async()=>{review.quotaRows=[{provider:'deepseek',accountId:'a',name:'余额',windows:[],error:'账户凭证失效'}];await reviewEmit('desktop:data-changed','quotas.refresh');});
    assert.equal(await panel.locator('#activity').getAttribute('data-status'),'error');
    assert.equal(await panel.evaluate(()=>state.lastSuccessfulUpdate),panelSuccess);
    await panel.evaluate(async()=>{review.settings.hosts=[];await reviewEmit('desktop:settings',null);});
    await panel.locator('#add-first-host').click();assert.ok((await panel.evaluate(()=>review.actions)).includes('add-host'));
    await page.evaluate(async()=>{state.page='agent';state.lastMetrics=Date.now()-3000;await reviewEmit('desktop:status',{panelOpen:true,panelPage:'servers',platform:'windows',sessions:[],summary:'暂无活跃会话',mode:'auto',effectiveMode:'floating'});});
    const hostCalls=await page.evaluate(()=>review.calls.filter(method=>method==='hosts.sample').length);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='hosts';render();editItem();reviewTimers.find(timer=>timer.ms===500).fn();});await page.waitForFunction(()=>!state.serverBusy);
    assert.equal(await page.locator('#editor').evaluate(el=>el.open),true);
    assert.equal(await page.evaluate(()=>review.calls.filter(method=>method==='hosts.sample').length),hostCalls+1,'An open server panel keeps the main sampler at the foreground cadence');
    await page.setViewportSize({width:640,height:440});
    const editor=await page.locator('#editor').boundingBox(),saveButton=await page.locator('#editor-form button[type=submit]').boundingBox();
    assert.ok(editor.x>=0&&editor.y>=40&&editor.x+editor.width<=640&&editor.y+editor.height<=440,'Editor fits the smallest Windows work area');
    assert.ok(saveButton.y+saveButton.height<=440,'Save stays reachable at minimum height');
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
    await page.screenshot({animations:'disabled',path:path.join(previews,'editor-minimum-fixed.png')});
    await discardEditor(page);await page.locator('[data-page=servers]').click();await page.waitForFunction(()=>!state.serverBusy);
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
    await page.screenshot({animations:'disabled',path:path.join(previews,'servers-minimum-fixed.png')});
    await panel.close();assert.deepEqual(errors,[]);console.log('UI review interaction regressions passed');
  } finally {await browser?.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
