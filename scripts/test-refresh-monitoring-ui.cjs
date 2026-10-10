// Deterministic refresh/layout, local monitoring, and agy authorization UI checks.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(){
  window.setInterval=()=>0;
  const source={id:'agy',provider:'antigravity',name:'Antigravity 本机',accountId:'a',enabled:true,path:'fixture',hostId:null,agyBinary:'agy'};
  const summary={tokens:{input:100,output:20,cacheRead:0,cacheWrite:0},total:120,cost:1,pricedTokens:120,events:1};
  const sample={timestamp:Date.now()/1000,uptime:900,load:[],errors:{},cpu:[{id:'cpu',utilization:15}],memory:{total:1024,available:512},filesystems:[{id:'/',total:100,available:50,used:50,type:'ext4'},{id:'/run',type:'tmpfs',total:100,available:100,used:0},{id:'/boot/efi',type:'vfat',total:100,available:100,used:0},{id:'/snap/a/1',type:'squashfs',total:100,available:0,used:100}]};
  window.refreshTest={calls:[],partial:false,hold:false,holdQuery:false,phase:'authorizing',settings:{version:5,accounts:[{id:'a',provider:'antigravity',name:'测试账户',quotaEnabled:true}],sources:[source],hosts:[],agents:[{provider:'antigravity',enabled:true,machineIds:['local']}],proxy:{mode:'system',url:''},modelMappings:{},refreshSeconds:301,historyRefreshSeconds:401,serverForegroundRefreshSeconds:3,serverRefreshSeconds:11,localMonitor:{enabled:true,metrics:['cpu','memory','filesystems'],devices:[],details:['uptime']}}};
  const identity=()=>({key:'google:fixture',email:'account@example.test',subscription:'Google AI Pro',stale:false});
  const accountStatus=()=>({sourceId:'agy',accountKey:'antigravity:a',machineId:'local',authenticated:true,identityConfirmed:!!refreshTest.known,current:refreshTest.known?true:null,credential:refreshTest.known?true:null,identity:refreshTest.known?identity():null});
  const auth=()=>({id:'session',sourceId:'agy',phase:refreshTest.phase,message:refreshTest.workspace?'CLI 正在询问是否信任临时空目录':refreshTest.phase==='authenticated'?(refreshTest.known?'登录有效，已确认账户身份':'登录有效 · 账户身份待确认'):'请打开授权网址',workspaceConfirmationRequired:!!refreshTest.workspace,authUrl:refreshTest.phase==='authorizing'&&!refreshTest.workspace?'https://accounts.google.com/o/oauth2/auth?state=test':null,authenticated:refreshTest.phase==='authenticated',identityConfirmed:!!refreshTest.known,identity:refreshTest.known?identity():null,current:refreshTest.known?true:null,accountStatus:refreshTest.phase==='authenticated'?accountStatus():null});
  window.__TAURI__={event:{async listen(){return()=>{};}},window:{getCurrentWindow:()=>({async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(command,args){
    if(command==='desktop_info')return {page:'agent',platform:'windows',mode:'floating',effectiveMode:'floating',panelOpen:true,sessions:[]};
    if(command!=='engine_call')return {};
    const {method,params}=args,t=refreshTest;t.calls.push({method,params});
    if(method==='settings.get')return structuredClone(t.settings);
    if(method==='settings.patch'){t.settings=structuredClone(params.settings);return structuredClone(t.settings);}
    if(method==='hello')return {version:'fixture'};
    if(method==='sources.scan'){if(t.hold)await new Promise(resolve=>t.release=resolve);return [{id:'agy',partial:t.partial,issues:t.partial?[{path:'conversation.db',step:4,message:'模型身份未知；已保留 Token，暂不计价'}]:[]}];}
    if(method==='dashboard'){if(t.holdQuery)await new Promise(resolve=>t.releaseQuery=resolve);return {summary,quotas:[],sources:[{...source,status:{updatedAt:Date.now()/1000}}],models:[],trendDays:[],heatmap:[],dayModels:[],pricingGaps:[],modelOptions:[],quotaOrder:[]};}
    if(method==='hosts.sample')return [{id:'local',name:'本机',sample:structuredClone(sample)}];
    if(method==='hosts.discover')return structuredClone(sample);
    if(method==='quotas.refresh'){if(t.holdQuota)await new Promise(resolve=>t.releaseQuota=resolve);if(t.failQuota)throw new Error('模拟额度查询失败');return [];}
    if(method==='accounts.status.refresh'&&t.known){if(t.failStatusRefresh)throw new Error('不应重复检查登录');t.settings.accounts[0].identityKey='google:fixture';return accountStatus();}
    if(method==='accounts.status.refresh')return {sourceId:'agy',accountKey:'antigravity:a',machineId:'local',current:null,credential:null,authenticated:true,identityConfirmed:false};
    if(method.startsWith('agyAuth.')){if(method==='agyAuth.verify'){t.phase='authenticated';if(t.known)t.settings.accounts[0].identityKey='google:fixture';}if(method==='agyAuth.confirmWorkspace')t.workspace=false;if(method==='agyAuth.cancel')t.phase='cancelled';return auth();}
    return [];
  }}};
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web');
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://test').pathname)||'index.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.js':'application/javascript','.css':'text/css','.html':'text/html','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
  try{
    browser=await chromium.launch({headless:true});
    for(const surface of ['index.html','floating.html']){
      const page=await browser.newPage({viewport:{width:surface==='index.html'?1120:450,height:900}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);
      await page.goto(`http://127.0.0.1:${server.address().port}/${surface}`);await page.evaluate(()=>AieyesApp.ready);await page.waitForFunction(()=>!state.busy&&!state.quotaBusy);
      const heading=page.locator('.overview-heading'),position=await heading.boundingBox();
      await page.evaluate(()=>{refreshTest.hold=true;void scan();});await page.waitForFunction(()=>refreshTest.release);
      assert.equal((await heading.boundingBox()).y,position.y);assert.equal(await page.locator('[data-refresh-status=scan]').getAttribute('data-busy'),'true');
      await page.evaluate(()=>{refreshTest.partial=true;refreshTest.hold=false;refreshTest.release();});await page.waitForFunction(()=>!state.busy);
      assert.equal((await heading.boundingBox()).y,position.y);assert.equal(await page.locator('#message:visible').count(),0,await page.locator('#message').textContent());
      await page.locator('[data-refresh-status=scan]').click();assert.match(await page.locator('#refresh-status-popover').textContent(),/已保留 Token/);await page.keyboard.press('Escape');
      await page.evaluate(()=>{state.refreshState.quotas={busy:false,error:'old failure',failures:[{accountId:'a',error:'old failure'}]};void quotas(undefined,true);});await page.waitForFunction(()=>!state.quotaBusy);assert.equal(await page.evaluate(()=>state.refreshState.quotas.error),'old failure');
      await page.evaluate(()=>{refreshTest.holdQuery=true;void loadDashboard();});await page.waitForFunction(()=>refreshTest.releaseQuery);assert.equal((await heading.boundingBox()).y,position.y);assert.equal(await page.locator('#query-status:visible').count(),0);await page.evaluate(()=>{refreshTest.holdQuery=false;refreshTest.releaseQuery();});await page.waitForFunction(()=>!state.dashboardPending);
      await page.locator('[data-page=servers]').click();await page.waitForFunction(()=>!state.serverBusy);assert.equal(await page.locator('.server-card').count(),1);assert.match(await page.locator('.server-card').textContent(),/本机/);assert.doesNotMatch(await page.locator('.server-filesystem-list').textContent(),/snap|efi|run/);
      if(surface==='index.html'){
        await page.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
        for(const [key,value] of [['refreshSeconds','301'],['historyRefreshSeconds','401'],['serverForegroundRefreshSeconds','3'],['serverRefreshSeconds','11']])assert.equal(await page.locator('#field-'+key).inputValue(),value);
        await page.locator('[data-settings-tab=hosts]').click();assert.equal(await page.locator('[data-remove=local]:visible').count(),0);await page.locator('[data-edit=local]').click();assert.equal(await page.locator('#field-target:visible').count(),0);await page.locator('#discover-devices').click();await page.waitForFunction(()=>document.querySelector('#discover-devices').textContent==='读取设备');await page.locator('.multi-trigger').filter({hasText:'文件系统'}).click();assert.equal(await page.locator('.multi-panel:visible input:checked').count(),1);await page.locator('.multi-panel:visible button').filter({hasText:'全选',exact:true}).click();await page.keyboard.press('Escape');await page.locator('#editor-form button[type=submit]').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);assert.deepEqual(await page.evaluate(()=>state.settings.localMonitor.devices),['filesystems:__all__']);assert.equal(await page.evaluate(()=>state.settings.hosts.length),0);
        await page.evaluate(()=>{state.settingsTab='accounts';render();});await page.locator('[data-agy-login]').first().click();await page.waitForSelector('#agy-auth-url:visible');await page.locator('#agy-code').fill('test-code');await page.locator('#agy-submit-code').click();await page.locator('#agy-verify').click();await page.waitForFunction(()=>document.querySelector('#agy-login-status').textContent.includes('身份待确认'));await page.locator('#agy-close').click();assert.equal(await page.locator('#agy-login').count(),0);
        await page.evaluate(()=>{refreshTest.known=true;refreshTest.phase='authorizing';refreshTest.holdQuota=true;refreshTest.failStatusRefresh=true;});
        await page.locator('[data-agy-login]').first().click();await page.waitForSelector('#agy-auth-url:visible');
        await page.locator('#agy-verify').click();
        await page.waitForFunction(()=>document.querySelector('#agy-login-identity').textContent.includes('account@example.test'));
        await page.waitForFunction(()=>state.settings.accounts[0].identityKey==='google:fixture');
        assert.match(await page.locator('#agy-login-identity').textContent(),/Google AI Pro/);
        await page.waitForFunction(()=>refreshTest.releaseQuota);
        assert.equal(await page.locator('#agy-close').isEnabled(),true,'Completion must not wait for quota reads');
        await page.locator('#agy-close').click();
        await page.waitForFunction(()=>!document.querySelector('#agy-login'));
        assert.match(await page.locator('.account-identity').first().textContent(),/account@example.test.*Google AI Pro/);
        assert.equal(await page.evaluate(()=>state.settings.accounts[0].name),'测试账户');
        assert(await page.evaluate(()=>refreshTest.calls.some(c=>c.method==='quotas.refresh'&&c.params.accountId==='a')));
        await page.evaluate(()=>{refreshTest.failQuota=true;refreshTest.holdQuota=false;refreshTest.releaseQuota();});
        await page.waitForFunction(()=>!window.AieyesAgentSettings.antigravityRefreshing);
        assert.match(await page.evaluate(()=>state.quotaError),/模拟额度查询失败/);
        assert.match(await page.locator('.account-identity').first().textContent(),/account@example.test.*Google AI Pro/);
        await page.evaluate(()=>{refreshTest.known=false;refreshTest.phase='authorizing';refreshTest.workspace=true;});
        await page.locator('[data-agy-login]').first().click();
        await page.locator('#agy-trust').waitFor({state:'visible'});
        await page.locator('#agy-trust').click();
        await page.waitForSelector('#agy-auth-url:visible');
        assert.equal(await page.locator('#agy-trust').isVisible(),false);
        await page.locator('#agy-close').click();
        assert.equal(await page.locator('#agy-login').count(),0);

      }
      assert.deepEqual(errors,[]);await page.close();
    }
    console.log('Stable refresh headings, partial feedback, no-op quota retention, four intervals, local monitoring/filesystem selection and agy login UI passed');
  }finally{await browser?.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
