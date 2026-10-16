const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont}=require('./ui-test-helpers.cjs');
function fixture(){
  window.setInterval=()=>0;
  const settings={version:7,sources:[{id:'s',name:'Codex 本机',provider:'codex',path:'/fixture',enabled:true,accountId:'',autoValuation:{enabled:true,consentVersion:1}}],accounts:[],hosts:[],codexApis:[],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,serverRefreshSeconds:10};
  const summary={tokens:{input:100,output:20,cacheRead:10,cacheWrite:0},total:130,cost:1,pricedTokens:130,events:1};
  const identity={key:'fixture-account',email:'fixture@example.test',user:'fixture',workspace:'workspace',plan:'plus'};
  window.apiFixture={settings,calls:[],selected:{kind:'account',provider:'openai'},revision:0,operation:0,failSave:false,failCommit:false,pausePrepare:false,failRefresh:false};
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:true};
    if(command!=='engine_call')return {};
    const {method,params}=args,f=window.apiFixture;f.calls.push({method,params});
    if(method==='hello')return {version:'test'};
    if(method==='settings.get'){if(f.failRefresh){f.failRefresh=false;throw Error('fixture refresh failed');}return structuredClone(settings);}
    if(method==='dashboard'||method==='dashboard.summary')return {generatedAt:Date.now()/1000,summary,quotas:[],quotaOrder:[],quotaEstimates:[],creditEstimates:[],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
    if(method==='codexApi.status')return {configRevision:'config-1',selected:structuredClone(f.selected)};
    if(method==='codexApi.save'){
      if(f.failSave){f.failSave=false;throw Error('模拟保存失败，输入已保留');}
      const c={...params.connection,id:params.connection.id||'api-one',revision:'revision-'+(++f.revision)};
      settings.codexApis=[c];return {settings:structuredClone(settings),connection:c};
    }
    if(method==='codexApi.probe')return {reachable:true,httpStatus:200,message:'API 认证可用'};
    if(method==='codexApi.remove'){settings.codexApis=[];return {settings:structuredClone(settings)};}
    if(method==='codexAuth.enable'){settings.sources[0].codexHomeId='home';return {homeId:'home'};}
    if(method==='codexAuth.inspect')return {version:'codex-cli fixture',storageMode:'file',selected:structuredClone(f.selected),storedIdentity:identity.key,currentIdentity:f.selected.kind==='account'?identity.key:null,profiles:[{id:'account',name:'已保存账户',identity,current:f.selected.kind==='account'}],processes:[]};
    if(method==='codexAuth.switch.prepare'){
      if(f.pausePrepare)await new Promise(resolve=>f.releasePrepare=()=>{f.pausePrepare=false;resolve();});
      f.prepared=structuredClone(params.target);return {operationId:'op-'+(++f.operation),status:'prepared',target:f.prepared,processes:[{pid:42,name:'Codex',canClose:true}],preservedProcesses:[{pid:43,name:'独立 API',canClose:false}]};
    }
    if(method==='codexAuth.switch.commit'){
      if(f.failCommit){f.failCommit=false;throw Error('配置已变化，请重新检查');}
      const c=settings.codexApis[0];f.selected=f.prepared.kind==='api'?{kind:'api',connectionId:c.id,revision:c.revision,name:c.name}:{kind:'account',provider:'openai'};
      if(f.failAfterCommit){f.failAfterCommit=false;f.failRefresh=true;}
      return {operationId:params.operationId,status:'succeeded'};
    }
    if(method==='autoEstimates.configure'){settings.sources[0].autoValuation=params.enabled?{enabled:true,consentVersion:1}:null;return {settings:structuredClone(settings)};}
    if(method==='codexAuth.reconcile')return {settings:structuredClone(settings),issues:[]};
    if(['prices.list','sources.scan','wakeups.list','accounts.status.get','accounts.cleanup.list','quotas.refresh'].includes(method))return [];
    return {};
  }}};
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web');
  const server=http.createServer((req,res)=>{if(serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('Content-Type',({'.js':'application/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try{
    browser=await chromium.launch({headless:true});
    for(const surface of ['index.html','floating.html']){
      const page=await browser.newPage({viewport:{width:surface==='index.html'?1140:460,height:850}}),errors=[];
      page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto('http://127.0.0.1:'+server.address().port+'/'+surface);
      await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
      assert.equal(await page.locator('.billing-breakdown').count(),0);
      if(surface==='floating.html'){assert.equal(errors.length,0,errors.join('\n'));await page.close();continue;}
      await page.evaluate(()=>{state.page='settings';state.settingsTab='integrations';state.settingsAgent='codex';render();});
      await page.locator('#agent-section-api > summary').click();
      await page.locator('[data-codex-api="add"]').click();
      assert.equal(await page.locator('#field-apiModel,#field-apiEffort,#field-apiProxy,#field-apiPrices').count(),0);
      await page.locator('#field-apiName').fill('测试 API');await page.locator('#field-apiKey').fill('fixture-key');
      await page.evaluate(()=>apiFixture.failSave=true);await page.locator('#editor-form button[type=submit]').click();
      await page.locator('#editor-error').waitFor({state:'visible'});assert.equal(await page.locator('#field-apiKey').inputValue(),'fixture-key');
      await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
      assert.deepEqual(await page.evaluate(()=>Object.keys(state.settings.codexApis[0]).sort()),['baseUrl','id','name','revision','sourceId']);
      assert(!await page.evaluate(()=>JSON.stringify(state.settings).includes('fixture-key')));
      assert(!await page.evaluate(()=>JSON.stringify(localStorage).includes('fixture-key')));
      assert.equal(await page.evaluate(()=>apiFixture.selected.kind),'account');
      assert.equal(await page.evaluate(()=>apiFixture.calls.filter(c=>c.method==='codexAuth.switch.commit').length),0);
      await page.evaluate(()=>apiFixture.pausePrepare=true);
      await page.locator('[data-codex-api="switch"]').click();
      await page.waitForFunction(()=>typeof apiFixture.releasePrepare==='function');
      assert(await page.locator('#quota-close').isDisabled());await page.keyboard.press('Escape');assert(await page.locator('#quota-dialog').evaluate(el=>el.open));
      await page.evaluate(()=>apiFixture.releasePrepare());await page.locator('#api-switch-confirm').waitFor();
      assert.match(await page.locator('#quota-dialog').textContent(),/PID 42/);assert.match(await page.locator('#quota-dialog').textContent(),/保留 1/);
      await page.locator('#quota-close').click();await page.waitForFunction(()=>!document.querySelector('#quota-dialog').open);
      await page.locator('[data-codex-api="switch"]').click();await page.locator('#api-switch-confirm').waitFor();
      await page.evaluate(()=>apiFixture.failCommit=true);await page.locator('#api-switch-confirm').click();
      await page.waitForFunction(()=>document.querySelector('#quota-dialog').textContent.includes('配置已变化'));
      assert(await page.locator('#api-switch-confirm').isDisabled());assert.equal(await page.evaluate(()=>apiFixture.selected.kind),'account');
      await page.locator('#api-switch-retry').click();await page.waitForFunction(()=>!!document.querySelector('#api-switch-confirm')&&!document.querySelector('#api-switch-confirm').disabled);
      await page.locator('#api-switch-confirm').click();await page.waitForFunction(()=>!document.querySelector('#quota-dialog').open);
      assert.equal(await page.evaluate(()=>apiFixture.selected.kind),'api');assert(await page.locator('[data-codex-api="switch"]').isDisabled());
      const committed=await page.evaluate(()=>apiFixture.calls.find(c=>c.method==='codexAuth.switch.commit'));
      assert.equal(committed.params.closeProcesses,true);
      assert(!await page.evaluate(()=>apiFixture.calls.some(c=>['codexApi.default','codexApi.command','codexApi.deploy'].includes(c.method))));
      await page.locator('[data-codex-api="edit"]').click();assert(await page.locator('#field-apiSource').isDisabled());
      await page.locator('#field-apiName').fill('更新 API');await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
      await page.waitForFunction(()=>document.querySelector('.codex-api-settings').textContent.includes('有已保存的更改'));
      assert.equal(await page.evaluate(()=>apiFixture.selected.revision),'revision-1');
      await page.locator('[data-codex-api="account"]').click();await page.locator('#api-switch-confirm').waitFor();
      assert.deepEqual(await page.evaluate(()=>apiFixture.prepared),{kind:'account',profileId:'account'});
      await page.evaluate(()=>apiFixture.failAfterCommit=true);await page.locator('#api-switch-confirm').click();await page.waitForFunction(()=>!document.querySelector('#quota-dialog').open);
      await page.waitForFunction(()=>document.querySelector('.codex-api-settings').textContent.includes('登录方式已切换，状态刷新未完成'));
      assert.equal(await page.evaluate(()=>apiFixture.selected.kind),'account');
      await page.evaluate(()=>AieyesAutomatic.open(['s']));
      assert.equal(await page.locator('[data-auto-mode]').count(),0);
      assert(await page.locator('[data-auto-save="s"]').isChecked());
      await page.locator('[data-auto-save="s"]').click();
      await page.waitForFunction(()=>!state.settings.sources[0].autoValuation?.enabled&&!document.querySelector('#quota-dialog').dataset.busy);
      assert.equal(await page.locator('#auto-consent').count(),0);
      await page.locator('[data-auto-save="s"]').click();await page.locator('#auto-consent[open]').waitFor();
      assert.equal(await page.locator('[data-auto-save="s"]').isChecked(),false);await page.locator('#auto-consent [data-confirm]').click();
      await page.waitForFunction(()=>state.settings.sources[0].autoValuation?.enabled===true);
      const configured=await page.evaluate(()=>apiFixture.calls.filter(c=>c.method==='autoEstimates.configure').at(-1).params);
      assert.equal(configured.confirmed,true);assert(!('mode' in configured));
      await page.locator('#quota-close').click();
      await page.locator('[data-codex-api="remove"]').click();await page.locator('#editor-form button[type=submit]').click();
      await page.waitForFunction(()=>state.settings.codexApis.length===0&&!document.querySelector('#editor').open);
      assert.equal(errors.length,0,errors.join('\n'));await page.close();
    }
    console.log('Codex API minimal editor, confirmed switching, stale checks, refresh recovery and automatic consent passed');
  }finally{await browser?.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
