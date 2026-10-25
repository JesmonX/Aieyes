const {openGeneralPage} = require('./ui-test-helpers.cjs');
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont}=require('./ui-test-helpers.cjs');

function fixture(){
  window.setInterval=()=>0;
  const now=Date.now()/1000,summary={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
  const settings={version:7,agents:[{provider:'codex',enabled:true,machineIds:['local','remote','paused']}],accounts:[{id:'a',provider:'codex',name:'订阅账户',quotaEnabled:true}],sources:[{id:'local',provider:'codex',accountId:'a',name:'本机目录',path:'/fixture/local',enabled:true},{id:'remote',provider:'codex',accountId:'a',name:'远程目录',hostId:'remote',path:'/fixture/remote',enabled:true},{id:'paused',provider:'codex',accountId:'a',name:'暂停目录',hostId:'paused',path:'/fixture/paused',enabled:true}],hosts:[{id:'remote',name:'慢服务器',enabled:true},{id:'paused',name:'暂停服务器',enabled:false}],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,historyRefreshSeconds:300,serverForegroundRefreshSeconds:3,serverRefreshSeconds:10};
  const task=id=>({id,name:id,sourceId:id==='slow'?'remote':'local',codexProfileId:null,times:['08:00'],model:'test-model',effort:'low',prompt:'test',binary:''});
  const tasks=['slow','fast','bad','pending'].map(id=>({task:task(id),deployment:{state:id==='pending'?'settings-pending':'deployed',enabled:id!=='pending',target:id==='slow'?'慢服务器':'本机',timezone:'UTC',changed:id==='slow'}}));
  const f=window.wakeupFixture={settings,tasks,calls:[],holdSlow:true,holdSync:true,failSync:true,failRun:{fast:true,bad:true},failStatus:{}};
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {platform:'linux',mode:'tray',effectiveMode:'tray',sessions:[],panelOpen:true};
    if(command!=='engine_call')return {};
    const {method,params}=args;f.calls.push({method,params:structuredClone(params)});
    if(method==='hello')return {version:'fixture'};
    if(method==='settings.get')return structuredClone(settings);
    if(method==='settings.patch'){for(const key of Object.keys(params.settings))if(JSON.stringify(params.settings[key])!==JSON.stringify(params.base[key]))settings[key]=structuredClone(params.settings[key]);return structuredClone(settings);}
    if(method==='dashboard')return {generatedAt:now,summary,quotas:[],quotaEstimates:[],creditEstimates:[],sources:[],models:[],dayModels:[],modelOptions:[],trendDays:[],heatmap:[],pricingGaps:[]};
    if(method==='sessions.list')return {sessions:[],warnings:[]};
    if(method==='accounts.targets')return settings.sources.map(s=>({accountKey:'codex:a',accountName:'订阅账户',provider:'codex',sourceId:s.id,sourceName:s.name,path:s.path,machineId:s.hostId||'local',machineName:s.hostId==='paused'?'暂停服务器':s.hostId?'慢服务器':'本机',profileId:null,canDraft:true,canWake:s.id!=='paused',available:s.id!=='paused',reason:s.id==='paused'?'采集已暂停':null}));
    if(method==='accounts.deployments.sync'){
      if(!params?.taskId)return {failedTasks:[]};
      if(f.holdSync){f.holdSync=false;await new Promise(resolve=>f.releaseSync=resolve);}
      if(f.failSync)return {failedTasks:[{id:params.taskId,error:'待同步：服务器离线'}]};
      tasks.find(r=>r.task.id===params.taskId).deployment.state='deployed';return {failedTasks:[]};
    }
    if(method==='wakeups.list')return structuredClone(tasks);
    if(method==='wakeups.status'){
      if(params.id==='slow'&&f.holdSlow){f.holdSlow=false;await new Promise(resolve=>f.releaseSlow=resolve);return {installed:false,timezone:'UTC',history:[]};}
      if(f.failStatus[params.id])throw Error(params.id+' 状态失败');
      return {installed:true,timezone:'UTC',nextRunAt:now+3600,history:[]};
    }
    if(method==='wakeups.save'){
      const record=tasks.find(r=>r.task.id===params.task.id);
      if(record)record.task=structuredClone(params.task);else tasks.push({task:{...params.task,id:'draft'},deployment:null});
      return record??tasks.at(-1);
    }
    if(method==='wakeups.deploy'){const record=tasks.find(r=>r.task.id===params.id);record.deployment.changed=false;return record;}
    if(method==='wakeups.run'){if(f.failRun[params.id])throw Error(params.id+' 运行失败');return {started:true};}
    return [];
  }}};
}

(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web'),server=http.createServer((req,res)=>{
    if(serveFont(req,res))return;
    const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');
    if(!fs.existsSync(file)){res.writeHead(404).end();return;}
    res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));const browser=await chromium.launch({headless:true});
  try{
    const page=await browser.newPage({viewport:{width:1120,height:850},reducedMotion:'reduce'}),errors=[];
    page.on('pageerror',error=>errors.push(String(error)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(()=>AieyesApp.ready);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='wakeups';renderSettings();});
    await page.waitForFunction(()=>wakeupFixture.releaseSlow&&wakeupFixture.releaseSync);
    const row=id=>page.locator('[data-wake-row="'+id+'"]'),action=(id,kind)=>row(id).locator('.actions [data-wake-action="'+kind+'"]');
    assert(!await page.locator('#wake-refresh').isDisabled(),'the task list finishes loading before slow status and sync calls');
    assert(await action('pending','edit').isDisabled(),'only the task being synchronized is locked');
    assert(!await action('fast','edit').isDisabled());
    await action('fast','edit').click();await page.locator('#field-name').fill('编辑不等待远程状态');await page.locator('#editor-form button[type=submit]').click();
    await page.waitForFunction(()=>!document.querySelector('#editor').open);
    assert.match(await row('fast').textContent(),/编辑不等待远程状态/);
    assert.equal(await page.evaluate(()=>wakeupFixture.calls.filter(c=>c.method==='accounts.deployments.sync').length),1,'list refreshes do not repeatedly start the same pending synchronization');

    await page.locator('#wake-add').click();await page.locator('#wake-source').selectOption('paused');await page.locator('#wake-model').fill('saved-offline');await page.locator('#editor-form button[type=submit]').click();
    await page.waitForFunction(()=>!document.querySelector('#editor').open);
    assert.equal(await page.evaluate(()=>wakeupFixture.tasks.find(r=>r.task.id==='draft').task.sourceId),'paused');
    assert(await page.evaluate(()=>wakeupFixture.calls.some(c=>c.method==='accounts.targets'&&c.params.includeUnavailable===true)));
    assert.equal(await page.evaluate(()=>wakeupFixture.calls.filter(c=>['wakeups.probe','wakeups.deploy'].includes(c.method)).length),0,'saving an associated paused target does not probe or deploy');

    await page.locator('[data-settings-tab=general]').click();await openGeneralPage(page, 'refresh');
    await page.locator('#field-refreshSeconds').fill('360');await page.locator('#field-refreshSeconds').blur();
    await page.waitForFunction(()=>state.settings.refreshSeconds===360&&!state.settingsCommitting);
    assert.equal(await page.evaluate(()=>wakeupFixture.tasks.find(r=>r.task.id==='pending').deployment.state),'settings-pending','general saves finish while a remote sync remains pending');
    await page.locator('[data-settings-tab=wakeups]').click();

    await action('slow','deploy').click();await action('slow','status').waitFor();
    await page.evaluate(()=>wakeupFixture.releaseSlow());
    await page.waitForTimeout(60);
    assert.equal(await action('slow','deploy').count(),0,'a late pre-deployment status cannot restore the stale missing-task state');

    for(const id of ['fast','bad']){await row(id).locator('.wake-more summary').click();await action(id,'run').click();await row(id).getByText(id+' 运行失败',{exact:false}).waitFor();}
    await page.evaluate(()=>wakeupFixture.failRun.fast=false);await row('fast').locator('.error [data-wake-action=run]').click();
    await page.waitForFunction(()=>!document.querySelector('[data-wake-row=fast]').textContent.includes('fast 运行失败'));
    assert.match(await row('bad').textContent(),/bad 运行失败/,'successful retry clears only that task and operation error');

    await page.evaluate(()=>wakeupFixture.releaseSync());
    await row('pending').getByText(/待同步：服务器离线/).waitFor();
    assert.equal(await row('pending').locator('.error [data-wake-action=sync]').count(),1);
    await page.evaluate(()=>wakeupFixture.failSync=false);await row('pending').locator('.error [data-wake-action=sync]').click();
    await page.waitForFunction(()=>document.querySelector('[data-wake-row=pending]').textContent.includes('已停用')&&!document.querySelector('[data-wake-row=pending]').textContent.includes('服务器离线'));
    assert.equal(await page.evaluate(()=>wakeupFixture.tasks.find(r=>r.task.id==='pending').deployment.enabled),false,'sync preserves the disabled deployment state');
    assert.match(await row('bad').textContent(),/bad 运行失败/);

    await page.evaluate(()=>wakeupFixture.failStatus.fast=true);await action('fast','status').click();await row('fast').getByText(/fast 状态失败/).waitFor();
    await page.evaluate(()=>wakeupFixture.failStatus.fast=false);await page.locator('#wake-refresh').click();
    await page.waitForFunction(()=>!document.querySelector('[data-wake-row=fast]').textContent.includes('fast 状态失败'));
    assert.deepEqual(errors,[]);
    console.log('Wakeup isolation: list-first loading, independent edits and errors, paused drafts, nonblocking sync, preserved enable state, stale status rejection and retry recovery passed');
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
