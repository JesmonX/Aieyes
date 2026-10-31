const {revealControl} = require('./ui-test-helpers.cjs');
/* Real-core discovery; disposable credentials and no provider network calls. */
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),os=require('node:os'),http=require('node:http'),readline=require('node:readline');
const {spawn}=require('node:child_process'),{chromium}=require('../apps/desktop/node_modules/playwright');
const project=path.resolve(__dirname,'..'),root=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-account-scan-')),home=path.join(root,'home');
fs.mkdirSync(path.join(home,'.codex'),{recursive:true});fs.mkdirSync(path.join(home,'.local/share/opencode'),{recursive:true});
const jwt=v=>'e30.'+Buffer.from(JSON.stringify(v)).toString('base64url')+'.fixture';
const auth={tokens:{account_id:'workspace',access_token:jwt({'https://api.openai.com/auth':{chatgpt_user_id:'user',chatgpt_account_id:'workspace'},exp:4102444800}),id_token:jwt({sub:'user',email:'scan@example.test'})}};
fs.writeFileSync(path.join(home,'.codex/auth.json'),JSON.stringify(auth));fs.writeFileSync(path.join(home,'.local/share/opencode/auth.json'),JSON.stringify({external:{type:'api',key:'PRIVATE-KEY-NEVER-RENDER'}}));
const env={...process.env,HOME:home,USERPROFILE:home,CODEX_HOME:path.join(home,'.codex'),XDG_DATA_HOME:path.join(home,'.local/share')};
for(const key of ['CLAUDE_CONFIG_DIR','KIMI_CODE_HOME','GROK_HOME','QODER_CONFIG_DIR','CODEBUDDY_CONFIG_DIR','PI_CODING_AGENT_DIR','DSH_HOME','XDG_CONFIG_HOME'])env[key]=path.join(home,'unused',key);
const core=spawn(path.join(project,'target/debug/aieyes-core'),['--data-dir',path.join(root,'data')],{env,stdio:['pipe','pipe','pipe']});
const coreClosed=new Promise(resolve=>core.once('close',resolve));
let next=0,stderr='';const pending=new Map(),calls=[];core.stderr.on('data',b=>stderr+=b);readline.createInterface({input:core.stdout}).on('line',line=>{const r=JSON.parse(line),p=pending.get(r.id);if(!p)return;pending.delete(r.id);r.error?p.reject(new Error(r.error.message)):p.resolve(r.result);});
function rpc(method,params={}){return new Promise((resolve,reject)=>{const id=++next;pending.set(id,{resolve,reject});core.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');});}
(async()=>{
 let browser,server;
 try{
  const settings=await rpc('settings.get');for(const a of settings.agents){a.enabled=['codex','opencode'].includes(a.provider);a.machineIds=[];}settings.hosts=[{id:'paused',name:'暂停服务器',target:'fixture.invalid',enabled:false}];await rpc('settings.save',settings);
  // The engine detects native logins at launch; run that pass once for the saved configuration.
  const detected=await rpc('accounts.autoScan');assert.equal(detected.addedAccounts,1);assert.equal(detected.addedConnections,2);assert(!JSON.stringify(detected).includes('PRIVATE-KEY'));
  assert.equal((await rpc('accounts.autoScan')).addedAccounts,0,'detecting again does not duplicate accounts');
  const web=path.join(project,'apps/desktop/web');server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(web,path.basename(new URL(req.url,'http://local').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'text/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  browser=await chromium.launch({headless:true});const page=await browser.newPage({viewport:{width:1280,height:1000}});const errors=[];page.on('pageerror',e=>errors.push(String(e)));
  await page.exposeFunction('scanRPC',async(method,params)=>{calls.push({method,params});if(['quotas.refresh','sources.scan','hosts.sample','wakeups.list','accounts.status.refreshBatch'].includes(method))return [];if(method==='sessions.list')return {sessions:[],warnings:[]};if(method.startsWith('network.'))return {};return rpc(method,params);});
  await page.addInitScript(()=>{window.setInterval=()=>0;window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',sessions:[]};if(command==='engine_call')return window.scanRPC(args.method,args.params??{});return {};}}};});
  await page.goto('http://127.0.0.1:'+server.address().port);await page.waitForFunction(()=>typeof state!=='undefined'&&state.settings&&!state.busy);
  // No scan UI anywhere: the machine list and the Agent account section carry no scan buttons.
  await page.evaluate(()=>{state.page='settings';state.settingsTab='hosts';render();});
  assert.equal(await page.locator('[data-native-scan-machine],[data-native-scan-agent],.native-scan-dialog').count(),0);
  assert(!(await page.locator('main').textContent()).includes('扫描'),'machine settings do not mention scanning');
  await page.evaluate(()=>{state.page='settings';state.settingsTab='integrations';state.settingsAgent='codex';render();});
  await page.locator('#agent-accounts').getByText('scan@example.test').first().waitFor();
  assert.equal(await page.locator('[data-native-scan-agent],[data-native-scan-machine]').count(),0,'no scan buttons on the Agent page');
  const login=page.locator('[data-agent-detail="native-connections-codex-login"]');await login.locator('summary').click();
  assert.match(await login.textContent(),/已识别账户/);assert(!/待验证|需重新扫描|重新扫描/.test(await page.locator('main').textContent()),'no pending or rescan wording');
  await login.locator('summary').click();
  await page.evaluate(()=>{state.settingsAgent='opencode';renderSettings(false);});await page.locator('#agent-section-native-api > summary').click();const preview=path.join(project,'.local/account-scan');fs.mkdirSync(preview,{recursive:true});await page.evaluate(()=>document.fonts.ready);await page.screenshot({path:path.join(preview,'connections.png'),fullPage:true});
  assert.equal(await page.locator('[data-native-remove]').count(),1);assert(!(await page.locator('main').textContent()).includes('PRIVATE-KEY'));
  await page.waitForFunction(()=>!AieyesAgentSettings.statusBusy);
  const refreshes=calls.filter(c=>c.method==='accounts.status.refreshBatch').length;
  await page.evaluate(()=>{const saved=structuredClone(state.settings);for(const r of saved.nativeConnections)r.checkedAt+=1;AieyesAgentSettings.accept(saved);});
  assert.equal(calls.filter(c=>c.method==='accounts.status.refreshBatch').length,refreshes,'scan timestamps do not invalidate device configuration');
  const credentials=page.locator('[data-agent-detail="native-connections-opencode-api"]');
  assert.equal(await credentials.getAttribute('open'),null,'scanned credentials are collapsed by default');
  await credentials.locator('summary').click();assert(await page.locator('[data-native-remove]').isVisible());
  await credentials.locator('summary').click();
  // Unlinking everything is confirmed first: danger confirm, focus on Cancel, files untouched.
  await page.locator('[data-native-remove-all="opencode"]').click();
  const confirm=page.locator('#editor-form button[type=submit]');await confirm.waitFor();
  assert.equal(await page.locator('#editor-title').textContent(),'取消全部引用？');
  assert(await confirm.evaluate(el=>el.classList.contains('danger')&&el.textContent==='取消全部引用'),'unlink-all confirms with a danger button');
  assert.equal(await page.evaluate(()=>document.activeElement?.id),'cancel-editor','destructive confirmations start on Cancel');
  assert(await page.evaluate(()=>state.settings.nativeConnections.some(r=>r.provider==='opencode')),'nothing is unlinked before confirming');
  await confirm.click();await page.waitForFunction(()=>!state.settings.nativeConnections.some(r=>r.provider==='opencode'));
  assert.equal(JSON.parse(fs.readFileSync(path.join(home,'.local/share/opencode/auth.json'))).external.key,'PRIVATE-KEY-NEVER-RENDER');
  await rpc('prices.save',{prices:[{id:'anthropic/claude-sonnet-4.5',name:'Claude Sonnet 4.5',input:0.000003,output:0.000015},{id:'relay/Claude-4-5-Sonnet',name:'Claude Sonnet 4.5'}]});
  await page.evaluate(async()=>{state.settingsTab='prices';state.prices=await api('prices.list');renderSettings(false);});
  await page.locator('#reference-prices').click();
  await page.waitForFunction(()=>!state.busy&&state.settings.modelMappings['relay/Claude-4-5-Sonnet']==='anthropic/claude-sonnet-4.5');
  assert.equal((await rpc('settings.get')).modelMappings['relay/Claude-4-5-Sonnet'],'anthropic/claude-sonnet-4.5');
  await revealControl(page, '#existing-mappings > summary');await page.waitForFunction(()=>document.querySelector('[data-mapping-rows]')?.textContent.includes('relay/Claude-4-5-Sonnet'));assert.match(await page.locator('[data-mapping-rows]').textContent(),/relay\/Claude-4-5-Sonnet → anthropic\/claude-sonnet-4.5/);
  const emptyLabels=await page.evaluate(()=>({quota:estimateEntry({provider:'codex',accountId:'fixture',windows:[{windowMinutes:300},{windowMinutes:10080}]},[]),credits:creditRow({provider:'codex',accountId:'fixture',credits:{balance:'0'}},null,[])}));
  assert(!emptyLabels.quota.includes('≈ —'));assert(!emptyLabels.credits.includes('≈ —'));
  assert(emptyLabels.quota.includes('data-estimate='));assert(emptyLabels.credits.includes('data-credit-estimate='));
  await page.screenshot({path:path.join(preview,'pricing-references.png'),fullPage:true});
  assert.deepEqual(errors,[]);assert(!calls.some(c=>c.method.startsWith('accounts.scan')||c.method==='accounts.autoScan'),'the UI never triggers detection itself');
  console.log('Account discovery UI: automatic detection without scan buttons, collapsed credentials, bulk removal, smart price references and empty estimate labels passed.');
 }finally{if(browser)await browser.close();if(server)await new Promise(resolve=>server.close(resolve));core.kill();await coreClosed;fs.rmSync(root,{recursive:true,force:true});}
})().catch(error=>{console.error(error);process.exitCode=1;});
