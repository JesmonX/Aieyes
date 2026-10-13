const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont}=require('./ui-test-helpers.cjs');
function fixture(){
  window.setInterval=()=>0;
  Object.defineProperty(navigator,'clipboard',{value:{async writeText(text){window.copied=text;}}});
  const settings={version:7,sources:[{id:'s',name:'Codex 本机',provider:'codex',path:'/fixture',enabled:true,accountId:'',autoValuation:{enabled:true,consentVersion:1}}],accounts:[],hosts:[],codexApis:[],modelMappings:{},proxy:{mode:'direct'},refreshSeconds:300,serverRefreshSeconds:10};
  const summary={tokens:{input:100,output:20,cacheRead:10,cacheWrite:0},total:130,cost:1,pricedTokens:130,events:1};
  window.apiFixture={settings,calls:[],defaultProvider:null,failSave:false};
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {platform:'windows',mode:'floating',effectiveMode:'floating',summary:'Idle',sessions:[],panelOpen:true};
    if(command!=='engine_call')return {};
    const {method,params}=args,f=window.apiFixture;f.calls.push({method,params});
    if(method==='hello')return {version:'test'};
    if(method==='settings.get')return structuredClone(settings);
    if(method==='dashboard')return {generatedAt:Date.now()/1000,summary,billingSummary:{apiCost:2,apiTokens:120,apiPricedTokens:120,subscriptionEquivalentCost:3,subscriptionTokens:200,subscriptionPricedTokens:200,unknownTokens:10,incompleteRequests:1,apiUnmeteredTokens:5},quotas:[],quotaOrder:[],quotaEstimates:[],creditEstimates:[],modelOptions:[],models:[],dayModels:[],trendDays:[],heatmap:[],sources:[],pricingGaps:[]};
    if(method==='codexApi.status')return {configRevision:'config-1',defaultProvider:f.defaultProvider,installed:!!settings.codexApis.length,ready:!!settings.codexApis.length};
    if(method==='codexApi.save'){
      if(f.failSave){f.failSave=false;throw Error('模拟部署失败，输入已保留');}
      const c={...params.connection,id:params.connection.id||'api-one',revision:'revision-one'};
      settings.codexApis=[c];return {settings:structuredClone(settings),connection:c};
    }
    if(method==='codexApi.command')return {launchCommand:"& { $env:CODEX_HOME = '/fixture'; & codex --profile api }",resumeCommand:"& { $env:CODEX_HOME = '/fixture'; & codex --profile api resume }"};
    if(method==='codexApi.probe')return {reachable:true,httpStatus:200,message:'API 认证与模型列表接口可用；尚未发起计费调用'};
    if(method==='codexApi.default'){f.defaultProvider=params.id==='subscription'?'openai':'aieyes_api_'+params.id+'_revision-one';return {configRevision:'config-1'};}
    if(method==='codexApi.remove'){settings.codexApis=[];return {settings:structuredClone(settings)};}
    if(method==='autoEstimates.configure'){settings.sources[0].autoValuation=params.enabled?{enabled:true,consentVersion:1,mode:params.mode}:null;return {settings:structuredClone(settings)};}
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
      page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);
      await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
      const billing=page.locator('.billing-breakdown');await billing.waitFor();
      if(surface==='floating.html'){assert.equal(await billing.locator('details').evaluate(e=>e.open),false);await billing.locator('summary').click();}
      assert.match(await billing.textContent(),/订阅 API 等价估值/);assert.match(await billing.textContent(),/不完整请求/);
      if(surface==='floating.html'){assert.equal(errors.length,0,errors.join('\n'));await page.close();continue;}
      await page.evaluate(()=>{state.page='settings';state.settingsTab='accounts';render();});
      await page.locator('[data-codex-api="add"]').click();
      await page.locator('#field-apiName').fill('测试 API');await page.locator('#field-apiModel').fill('custom-model');await page.locator('#field-apiKey').fill('fixture-key');
      await page.waitForFunction(()=>document.querySelector('#api-config-status').textContent.includes('设备配置已读取'));
      await page.evaluate(()=>apiFixture.failSave=true);await page.locator('#editor-form button[type=submit]').click();
      await page.locator('#editor-error').waitFor({state:'visible'});assert.equal(await page.locator('#field-apiKey').inputValue(),'fixture-key');
      await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
      assert.equal(await page.evaluate(()=>state.settings.codexApis[0].prices.length),0,'Blank optional prices are accepted');
      assert(!await page.evaluate(()=>JSON.stringify(state.settings).includes('fixture-key')));
      assert(!await page.evaluate(()=>JSON.stringify(localStorage).includes('fixture-key')));
      await page.locator('[data-codex-api="command"]').click();await page.locator('#api-copy-resume').click();
      assert.match(await page.evaluate(()=>window.copied),/--profile api resume \}$/);
      await page.locator('#quota-close').click();
      await page.locator('[data-codex-api="default"]').click();await page.waitForFunction(()=>apiFixture.defaultProvider?.startsWith('aieyes_api_'));
      const defaults=await page.evaluate(()=>apiFixture.calls.find(c=>c.method==='codexApi.default'));
      assert.equal(defaults.params.revision,'revision-one');assert.equal(defaults.params.configRevision,'config-1');
      await page.locator('[data-codex-api="subscription"]').click();await page.waitForFunction(()=>apiFixture.defaultProvider==='openai');
      await page.evaluate(()=>AieyesAutomatic.open(['s']));
      assert.equal(await page.locator('[data-auto-mode="s"]').inputValue(),'subscription','Legacy consent stays subscription only');
      await page.locator('[data-auto-mode="s"]').selectOption('mixed');assert(await page.locator('[data-auto-apply="s"]').isDisabled());
      await page.locator('[data-auto-consent="s"]').check();await page.locator('[data-auto-apply="s"]').click();
      await page.waitForFunction(()=>state.settings.sources[0].autoValuation?.mode==='mixed');
      await page.locator('#quota-close').click();
      await page.locator('[data-codex-api="remove"]').click();await page.locator('#editor-form button[type=submit]').click();
      await page.waitForFunction(()=>state.settings.codexApis.length===0&&!document.querySelector('#editor').open);
      assert.equal(errors.length,0,errors.join('\n'));await page.close();
    }
    console.log('Codex API Web management, billing panel, draft recovery, and mixed-mode consent passed');
  }finally{await browser?.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
