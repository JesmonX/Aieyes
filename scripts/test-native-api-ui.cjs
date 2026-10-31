const {revealControl} = require('./ui-test-helpers.cjs');
/* Browser -> real core -> local HTTP fixture -> native config. No paid key or default client files. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const {serveFont}=require('./ui-test-helpers.cjs');
const provider=process.env.AIEYES_API_UI_PROVIDER||'opencode';
const qoderFamily=['qoder','qoder-cn'].includes(provider);
const qoderBinary=provider==='qoder-cn'?'qoderclicn':'qodercli';
const piFamily=['pi','omp'].includes(provider);
const authMode=process.env.AIEYES_API_UI_AUTH_MODE||'api-key';
const project=path.resolve(__dirname,'..'),root=fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-native-api-')));
const testEnv={...process.env,HOME:root,USERPROFILE:root,PATH:path.join(project,'apps/desktop/node_modules/.bin')+path.delimiter+process.env.PATH,PI_CODING_AGENT_DIR:path.join(root,provider),XDG_CONFIG_HOME:path.join(root,'config'),CODEBUDDY_CONFIG_DIR:path.join(root,'codebuddy'),KIMI_CODE_HOME:path.join(root,'kimi'),GROK_HOME:path.join(root,'grok'),DSH_HOME:path.join(root,'dsh'),CLAUDE_CONFIG_DIR:path.join(root,'claude'),ZCODE_PERSONAL_PROVIDER_CONFIG_FILE:path.join(root,'zcode/provider_config.json'),QODER_CONFIG_DIR:path.join(root,'qoder'),QODERCN_CONFIG_DIR:path.join(root,'qoder-cn')};
if(qoderFamily){
  // Mock only the installed CLI's documented BYOK control API. The model HTTP
  // validation and Aieyes persistence remain real; no user login is read.
  const bin=path.join(root,'bin');fs.mkdirSync(bin);
  const script=path.join(bin,'qoder-fixture.cjs');fs.writeFileSync(script,`const fs=require('node:fs'),path=require('node:path'),readline=require('node:readline');const dir=process.argv[process.argv.indexOf('--config-dir')+1];readline.createInterface({input:process.stdin}).on('line',line=>{const frame=JSON.parse(line),r={...frame.request,type:frame.request.type.replace(/^daemon[.]/,'')};let result={};if(r.type==='initialize'){if(frame.request.auth.type!=='qodercli')throw Error('native auth missing');result={daemon_protocol_version:2,capabilities:['byok_config_management_v1']}}else if(r.type==='check_byok_config')result={success:!fs.existsSync(path.join(dir,'fixture-check-fails'))};else if(['create_byok_config','update_byok_config'].includes(r.type)){const file=path.join(dir,'settings.json'),settings=JSON.parse(fs.readFileSync(file)),entry={...r.config};delete entry.providerId;settings.providers??={};settings.providers[r.config.providerId]=entry;fs.writeFileSync(file,JSON.stringify(settings));result={config:{providerId:r.config.providerId}};}else throw Error('unexpected control request');process.stdout.write(JSON.stringify({type:'control_response',response:{subtype:'success',request_id:frame.request_id,response:result}})+'\\n');});`);
  if(process.platform==='win32')fs.writeFileSync(path.join(bin,qoderBinary+'.cmd'),'@"'+process.execPath+'" "'+script+'" %*\r\n');
  else {fs.writeFileSync(path.join(bin,qoderBinary),'#!/bin/sh\nexec '+"'"+process.execPath.replaceAll("'","'\\''")+"' '"+script.replaceAll("'","'\\''")+"' \"$@\"\n");fs.chmodSync(path.join(bin,qoderBinary),0o700);}
  testEnv.HOME=root;testEnv.USERPROFILE=root;
  testEnv.PATH=bin+path.delimiter+testEnv.PATH;
}
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{stdio:['pipe','pipe','pipe'],env:testEnv});
const pending=new Map();let seq=0,stderr='',fail=true,requests=0,changeNative=false,expectedEffort='low';
core.stderr.on('data',b=>stderr+=b);
readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
core.on('exit',()=>{for(const p of pending.values())p.reject(new Error(stderr));pending.clear();});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
async function invoke(method,params){
  if(['sources.scan','quotas.refresh','accounts.status.refreshBatch'].includes(method))return [];
  return rpc(method,params);
}
function fixture(){window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',sessions:[],panelOpen:true};if(command==='engine_call')return window.testRPC(args.method,args.params??{});return {};}}};}
(async()=>{
  const settings=await rpc('settings.get');settings.sources=[{id:'open',provider,name:provider+' fixture',path:root,enabled:true}];settings.accounts=[];settings.hosts=[];settings.proxy={mode:'direct',url:''};
  for(const agent of settings.agents){agent.enabled=agent.provider===provider;agent.machineIds=['local'];}await rpc('settings.save',settings);
  const harness=provider==='deepseek-harness';
  const config=path.join(root,provider==='pi'?'pi/models.json':provider==='omp'?'omp/models.yml':provider==='opencode'?'config/opencode/opencode.json':provider==='kimi'?'kimi/config.toml':provider==='grok'?'grok/config.toml':harness?'dsh/profiles/fixture/cordis.patch.yml':provider==='claude'?'claude/settings.json':qoderFamily?provider+'/settings.json':provider==='zcode'?'zcode/provider_config.json':'codebuddy/settings.json');fs.mkdirSync(path.dirname(config),{recursive:true});
  if(harness){fs.writeFileSync(path.join(path.dirname(config),'package.json'),JSON.stringify({name:'dsh-profile-headless',private:true,dependencies:{},dsh:{profile:{bundles:['@deepseek-ai/dsh-base','@deepseek-ai/dsh-headless']}}}));fs.writeFileSync(path.join(path.dirname(config),'cordis.yml'),'[]\n');}
  const original=provider==='omp'?'{"providers":{"other":{"apiKey":"private-other-key"}},"keep":true}':provider==='pi'?{providers:{other:{apiKey:'private-other-key'}},keep:true}:['kimi','grok'].includes(provider)?"# original preferences\ntelemetry = false\n[loop_control]\nmax_attempts_per_step = 3\n":harness?"- id: session-persistence-jsonl\n  config:\n    root: !!js dshHomePath('sessions')\n":provider==='zcode'?{schemaVersion:1,config:{providerConfigRules:{providerRules:[{providerId:'other',providerName:'Other',config:{group:'standard-personal',access:{type:'api-key',apiKey:'private-other-key'},api:{type:'openai-chat-completions',baseUrl:'https://other.test/v1'},personalModelIds:['other'],modelOrder:['other']}}]},modelConfigRules:{providerModelRules:[],manualProviderModelRules:[]}}}:{permission:{bash:'ask'},provider:{other:{options:{apiKey:'private-other-key'}}}};fs.writeFileSync(config,typeof original==='string'?original:JSON.stringify(original));
  const readConfig=()=>['kimi','grok','omp'].includes(provider)||harness?fs.readFileSync(config,'utf8'):JSON.parse(fs.readFileSync(config));
  const modelPath=path.join(path.dirname(config),'models.json'),originalModels={models:[{id:'other-model',apiKey:'private-other-model-key',extra:true}],availableModels:['other-model'],extra:{keep:true}};
  if(provider==='codebuddy')fs.writeFileSync(modelPath,JSON.stringify(originalModels));
  const web=path.join(project,'apps/desktop/web');
  const server=http.createServer((req,res)=>{
    if(req.url==='/v1/chat/completions'||req.url==='/v1/messages'){
      let data='';req.on('data',b=>data+=b);req.on('end',()=>{
        requests++;const body=JSON.parse(data);assert.equal(body.model,'fixture-model');
        if(provider==='claude') {assert.equal(req.url,'/v1/messages');assert.equal(req.headers['anthropic-version'],'2023-06-01');assert.equal(body.output_config.effort,'low');if(authMode==='api-key'){assert.equal(req.headers['x-api-key'],'private-test-key');assert.equal(req.headers.authorization,undefined);}else{assert.equal(req.headers.authorization,'Bearer private-test-key');assert.equal(req.headers['x-api-key'],undefined);}}
        else {assert.equal(req.headers.authorization,'Bearer private-test-key');if(provider==='codebuddy'){assert.equal(req.headers['x-api-key'],'private-test-key');assert.equal(body.reasoning_effort,expectedEffort);}if(provider==='zcode'||piFamily)assert.equal(body.reasoning_effort,'low');}
        if(changeNative){fs.writeFileSync(path.join(root,'dsh/cordis.patch.yml'),'[{"id":"llm-retry","disabled":true}]\n');changeNative=false;}res.writeHead(fail?401:200,{'content-type':'application/json'});res.end(fail?'private-error-body':JSON.stringify(provider==='claude'?{type:'message',role:'assistant',content:[{type:'text',text:'AIEYES_API_OK'}]}:{choices:[{message:{content:'AIEYES_API_OK'}}]}));
      });return;
    }
    if(serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));
  });await new Promise(r=>server.listen(0,'127.0.0.1',r));let browser;
  try {
    browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1180,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.exposeFunction('testRPC',invoke);await page.addInitScript(fixture);await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.dashboard&&!state.busy);
    await page.evaluate(provider=>{state.page='settings';state.settingsTab='integrations';AieyesAgentSettings.navigate({provider});render();},provider);
    await page.waitForFunction(()=>[...document.querySelectorAll('.provider-mark img')].every(img=>img.complete&&img.naturalWidth>0));
    await page.locator('#agent-section-api > summary').click();await page.locator('[data-native-api="open"]').click();
    if(harness){await page.locator('#field-configPath').fill(config);await page.locator('#native-api-read').click();await page.waitForFunction(()=>!document.querySelector('#native-api-read').disabled);assert.equal(await page.locator('#native-api-read-status').textContent(),'已读取');}
    await page.locator('#field-baseUrl').fill(`http://127.0.0.1:${server.address().port}`+(provider==='claude'?'':'/v1'));await page.locator('#field-model').fill('fixture-model');await page.locator('#field-apiKey').fill('private-test-key');
    await page.locator('#editor .settings-details > summary').click();
    if(['kimi','grok','qoder','qoder-cn'].includes(provider)||harness||provider==='zcode'||piFamily) { await revealControl(page, '#field-contextWindow');await page.locator('#field-contextWindow').fill('261000'); }
    if(harness||['zcode','grok','qoder','qoder-cn','pi','omp'].includes(provider)) { await revealControl(page, '#field-maxOutputTokens');await page.locator('#field-maxOutputTokens').fill('2048'); }
    if(['claude','zcode','codebuddy','pi','omp'].includes(provider)) { await revealControl(page, '#field-effort');await page.locator('#field-effort').selectOption('low'); }
    if(provider==='claude')await page.locator('#field-authMode').selectOption(authMode);
    await page.locator('#editor-form button[type=submit]').click();await page.locator('#editor-error:visible').waitFor();
    assert.equal(await page.locator('#field-apiKey').inputValue(),'private-test-key');assert.deepEqual(readConfig(),original);assert(!(await page.locator('#editor-error').textContent()).includes('private-error-body'));
    if(provider==='codebuddy')assert.deepEqual(JSON.parse(fs.readFileSync(modelPath)),originalModels);
    fail=false;
    if(qoderFamily){
      const flag=path.join(path.dirname(config),'fixture-check-fails');fs.writeFileSync(flag,'');
      await page.locator('#editor-form button[type=submit]').click();await page.getByText('Qoder 原生模型验证失败',{exact:false}).waitFor();assert.deepEqual(readConfig(),original);fs.unlinkSync(flag);
    }
    if(harness){
      changeNative=true;await page.locator('#editor-form button[type=submit]').click();await page.getByText('验证期间 Harness 合成配置已改变，请重新读取后保存',{exact:false}).waitFor();assert.deepEqual(readConfig(),original);
      await page.locator('#native-api-read').click();await page.waitForFunction(()=>!document.querySelector('#native-api-read').disabled);
      await page.locator('#field-baseUrl').fill(`http://127.0.0.1:${server.address().port}/v1`);await page.locator('#field-model').fill('fixture-model');await page.locator('#field-apiKey').fill('private-test-key');await revealControl(page, '#field-contextWindow');await page.locator('#field-contextWindow').fill('261000');await revealControl(page, '#field-maxOutputTokens');await page.locator('#field-maxOutputTokens').fill('2048');
    }
    await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open).catch(async e=>{throw new Error(provider+': '+await page.locator('#editor-error').textContent()+' / '+e.message);});
    const saved=readConfig();
    if(provider==='grok')assert.match(saved,/session_summary = "aieyes-managed"/);
    if(piFamily){
      if(provider==='pi'){assert.deepEqual(saved.providers.other,original.providers.other);assert.equal(saved.keep,true);}else{assert.match(saved,/private-other-key/);assert.match(saved,/keep: true/);}
      const current=await rpc('agentApi.inspect',{sourceId:'open'});assert.equal(current.model,'fixture-model');assert.equal(current.contextWindow,261000);assert.equal(current.maxOutputTokens,2048);assert.equal(current.effort,'low');assert.equal(current.hasApiKey,true);
      const defaults=fs.readFileSync(path.join(path.dirname(config),provider==='pi'?'settings.json':'config.yml'),'utf8');assert.match(defaults,/aieyes-managed/);assert.match(defaults,/fixture-model/);
    }else if(['kimi','grok','qoder','qoder-cn'].includes(provider)||harness||provider==='zcode') {
      if(harness){assert.match(saved,/session-persistence-jsonl/);assert.match(saved,/!!js dshHomePath/);}else if(['kimi','grok'].includes(provider)){assert.match(saved,/max_attempts_per_step = 3/);assert.match(saved,/telemetry = false/);}else if(qoderFamily){assert.deepEqual(saved.permission,original.permission);assert.deepEqual(saved.provider,original.provider);assert.equal(saved.model.name,'aieyes-managed/fixture-model');assert.equal(saved.providers['aieyes-managed'].apiKey,'private-test-key');}else{assert.deepEqual(saved.config.providerConfigRules.providerRules[0],original.config.providerConfigRules.providerRules[0]);assert.equal(saved.config.defaultModelSelection.options.maxOutputTokens,undefined);}
      const current=await rpc('agentApi.inspect',{sourceId:'open'});assert.equal(current.model,'fixture-model');assert.equal(current.contextWindow,261000);assert.equal(current.hasApiKey,true);
    }else {assert.deepEqual(saved.permission,original.permission);assert.deepEqual(saved.provider.other,original.provider.other);assert.equal(saved.model,provider==='opencode'?'aieyes-managed/fixture-model':'fixture-model');assert.equal(provider==='opencode'?saved.provider['aieyes-managed'].options.apiKey:provider==='claude'?saved.env[authMode==='api-key'?'ANTHROPIC_API_KEY':'ANTHROPIC_AUTH_TOKEN']:saved.env.CODEBUDDY_API_KEY,'private-test-key');}
    assert.equal(requests,harness||qoderFamily?3:2);assert(!JSON.stringify(await rpc('settings.get')).includes('private-test-key'));
    if(qoderFamily){
      const edited=readConfig();edited.providers.other={baseUrl:'https://other.test/v1',apiKey:'private-unrelated-key'};edited.model.preferences={effort:'low'};fs.writeFileSync(config,JSON.stringify(edited));
    }
    await page.locator('[data-native-api="open"]').click();await page.waitForFunction(()=>document.querySelector('#editor').open);assert.equal(await page.locator('#field-apiKey').inputValue(),'');assert(!(await page.locator('#editor-fields').innerHTML()).includes('private-test-key'));assert.deepEqual(errors,[]);
    if(provider==='codebuddy'){
      assert.equal(saved.env.CODEBUDDY_MODEL,'fixture-model');assert.equal(saved.reasoningEffort,'low');assert.equal(saved.alwaysThinkingEnabled,true);
      const models=JSON.parse(fs.readFileSync(modelPath));assert.deepEqual(models.models[0],originalModels.models[0]);assert.deepEqual(models.extra,originalModels.extra);assert.deepEqual(models.availableModels,['other-model','fixture-model']);assert.equal(models.models[1].apiKey,'private-test-key');assert.equal(models.models[1].reasoning.defaultEffort,'low');
      assert(!(await rpc('agentApi.inspect',{sourceId:'open'})).apiKey);
      for(const choice of ['disabled','minimal']){
        expectedEffort=choice==='disabled'?undefined:choice;
        await revealControl(page, '#field-effort');await page.locator('#field-effort').selectOption(choice);await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
        const changed=readConfig();assert.equal(changed.alwaysThinkingEnabled,choice!=='disabled');if(choice!=='disabled')assert.equal(changed.reasoningEffort,choice);
        const catalog=JSON.parse(fs.readFileSync(modelPath));assert.equal(catalog.models.length,2);assert.deepEqual(catalog.models[0],originalModels.models[0]);assert.equal(catalog.models[1].supportsReasoning,choice!=='disabled');if(choice!=='disabled')assert.equal(catalog.models[1].reasoning.defaultEffort,choice);
        await page.locator('[data-native-api="open"]').click();await page.waitForFunction(()=>document.querySelector('#editor').open);assert.equal(await page.locator('#field-effort').inputValue(),choice);assert.equal(await page.locator('#field-apiKey').inputValue(),'');
      }
      assert.equal(requests,4);
    }
    if(qoderFamily){
      await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
      const updated=readConfig();assert.equal(updated.providers['aieyes-managed'].apiKey,'private-test-key');assert.equal(updated.providers.other.apiKey,'private-unrelated-key');assert.deepEqual(updated.model.preferences,{effort:'low'});assert.equal(requests,4);
      await page.locator('[data-native-api="open"]').click();await page.waitForFunction(()=>document.querySelector('#editor').open);
    }
    await page.screenshot({animations:'disabled',path:path.join(project,'.local/agent-integration/native-api-ui-'+provider+'.png')});
    console.log(provider+' native API form: real HTTP validation, failed retry, native merge, no secret readback passed');
  }finally{if(browser)await browser.close();server.close();core.kill();await new Promise(r=>core.once('close',r));fs.rmSync(root,{recursive:true,force:true});}
})().catch(e=>{console.error(e);core.kill();process.exitCode=1;});
