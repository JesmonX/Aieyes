const {discardEditor,commitSettings}=require('./ui-test-helpers.cjs');
// Browser integration test with in-memory IPC; never reads personal settings.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require(process.env.AIEYES_PLAYWRIGHT || '../apps/desktop/node_modules/playwright');
const root = path.resolve(__dirname, '../apps/desktop/web');
const output = path.resolve(__dirname, '../.local/ui-previews');

(async () => {
  const server = http.createServer((req, res) => {if(require('./ui-test-helpers.cjs').serveFont(req,res))return;
    const name = path.basename(new URL(req.url, 'http://localhost').pathname) || 'index.html';
    const file = path.join(root, name);
    if (!fs.existsSync(file)) { res.writeHead(404); res.end(); return; }
    res.setHeader('Content-Security-Policy',JSON.parse(fs.readFileSync(path.join(root,'../src-tauri/tauri.conf.json'))).app.security.csp);
    res.setHeader('content-type', ({'.html':'text/html','.css':'text/css','.js':'application/javascript'})[path.extname(file)] || 'application/octet-stream');
    res.end(fs.readFileSync(file));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({headless:true, args:['--no-sandbox']});
    const page = await browser.newPage({viewport:{width:1120,height:800}});
    const errors=[]; page.on('pageerror', error=>errors.push(String(error)));
    const fixture = () => {
      const groups=['cpu','memory','gpu','filesystems','disk','network'];
      const details=['cpuTimes','memoryCache','swap','fsAvailable','fsType','inodes','diskIops','diskBusy','networkTotals','networkErrors','gpuMemory','gpuThermals'];
      const tokens={input:240000,output:180000,cacheRead:160000,cacheWrite:20000};
      const usage={tokens,total:600000,events:128,pricedTokens:600000,cost:2.46};
      const day='2026-10-03';
      const history=Array.from({length:365},(_,i)=>({...usage,key:new Date(Date.UTC(2025,9,4+i)).toISOString().slice(0,10),total:i%5?100000+(i*71239)%850000:0}));
      const trend=history.slice(-7);
      window.fixtureSettings={accounts:[],sources:[],hosts:[{id:'host1',name:'训练服务器',target:'gpu-lab',port:null,identityFile:'',shell:'/bin/bash',preCommand:'',enabled:true,metrics:groups,devices:[],details}],proxy:{mode:'system',url:''},refreshSeconds:300,serverRefreshSeconds:10,githubRepository:'JesmonX/Aieyes',modelMappings:{}};
      window.saved=[]; window.discoveryFails=false; window.nativeCalls=[]; window.engineCalls=[]; window.settingsSaveFails=false; window.quotaFails=false; let maximized=false;
      window.uiTimers=[];
      window.setInterval=(fn,ms)=>{window.uiTimers.push({fn,ms});return window.uiTimers.length;};
      window.desktopInfo={platform:'windows',material:'opaque',mode:'auto',effectiveMode:'floating',page:'agent',sessions:[],summary:'暂无活跃会话',panelOpen:false,panelPinned:false,hidden:false};
      window.desktopCalls=[]; window.__listeners={};
      window.__TAURI__={
        event:{async listen(name,fn){window.__listeners[name]=fn;return ()=>{};}},
        window:{getCurrentWindow:()=>({async isMaximized(){return maximized;},async toggleMaximize(){maximized=!maximized;window.nativeCalls.push('maximize');},async minimize(){window.nativeCalls.push('minimize');},async close(){window.nativeCalls.push('close');},async onResized(){return ()=>{};}})},
        core:{async invoke(command,args){
          if(command==='updates_info'||command==='updates_check')return {currentVersion:'0.1.1',latestVersion:'0.1.1',phase:command==='updates_check'?'current':'idle',message:command==='updates_check'?'当前已是最新版本 v0.1.1':'',automatic:true,prompt:false};
          if(command==='desktop_info')return structuredClone(window.desktopInfo);
          if(command==='desktop_detail'||command==='desktop_panel'||command==='desktop_panel_pin'||command==='desktop_action'){
            window.desktopCalls.push({command,args:structuredClone(args)});
            if(command==='desktop_panel')window.desktopInfo.panelOpen=args.open;
            if(command==='desktop_panel_pin')window.desktopInfo.panelPinned=args.pinned;
            return structuredClone(window.desktopInfo);
          }
          if(command!=='engine_call')return {};
          window.engineCalls.push({method:args.method,params:structuredClone(args.params)});
          switch(args.method){
            case 'hello':return {version:'0.1.1'};
            case 'settings.get':return structuredClone(window.fixtureSettings);
            case 'sources.configure': {
              const next=structuredClone(window.fixtureSettings),item=args.params.source,i=next.sources.findIndex(s=>s.id===item.id);if(i<0)next.sources.push(item);else next.sources[i]=item;
              window.fixtureSettings=next;window.saved.push(structuredClone(next));return structuredClone(next);
            }
            case 'settings.patch':
              args.params=args.params.settings;
              if(window.settingsSaveFails)throw new Error('磁盘不可写');
              if(args.params.sources.some(s=>!['antigravity','deepseek'].includes(s.provider)&&!s.path.trim()))throw new Error('请输入数据目录');
              if(args.params.refreshSeconds<10||args.params.refreshSeconds>86400)throw new Error('Agent 刷新间隔范围为 10–86400 秒');
              window.fixtureSettings=structuredClone(args.params);window.saved.push(structuredClone(args.params));return structuredClone(args.params);
            case 'credentials.save':return {path:`test-credentials/${args.params.sourceId}.key`};
            case 'quotas.schedule':return {nextDueAt:window.fixtureSettings.accounts.some(a=>a.quotaEnabled&&!a.archived)?(window.fixtureQuotaAttempt??Date.now()/1000)+300:null};
            case 'quotas.refresh':window.fixtureQuotaAttempt=Date.now()/1000;if(window.quotaFails)throw new Error('限额连接失败');return [];
            case 'sources.scan':return {};
            case 'dashboard':return {summary:window.fixtureUsage??usage,quotas:[],models:[{...usage,key:'Claude Sonnet'}],trendDays:trend,dayModels:trend.map(row=>({day:row.key,model:'Claude Sonnet',usage:row})),heatmap:history,sources:window.fixtureSources??[],pricingGaps:[]};
            case 'hosts.sample':return window.fixtureHosts??[];
            case 'prices.list':return window.fixturePrices??[{id:'anthropic/claude-sonnet',name:'Claude Sonnet',input:0.000003,output:0.000015},{id:'openai/gpt-test',name:'GPT Test',input:0.000001,output:0.000002}];
            case 'prices.save':window.fixturePrices=args.params.prices;return {};
            case 'prices.sync':return {};
            case 'updates.open':return {opened:true};
            case 'updates.check':return {version:'v0.1.1',url:'https://github.com/JesmonX/Aieyes/releases/tag/v0.1.1'};
            case 'hosts.discover':
              if(window.discoveryFails)throw new Error('连接失败');
              return {cpu:[{id:'cpu'},...Array.from({length:256},(_,i)=>({id:`cpu${i}`}))],network:[{id:'eth0'},{id:'eth1'},{id:'ib0',name:'高速网卡'}],filesystems:Array.from({length:100},(_,i)=>({id:`/data/挂载点-${i}-long-name`,type:'ext4'})),gpu:[{id:'0',name:'GPU'}],disk:[{id:'nvme0n1'}],errors:{}};
            default:throw new Error(`Unexpected call: ${args.method}`);
          }
        }},
      };
    };
    await page.addInitScript(fixture);
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.locator('.stat-value').first().waitFor();
    for(const [selector,size] of [['.titlebar-mark',16],['.brand .eye',34]]) {
      const box=await page.locator(selector).boundingBox(),art=await page.locator(selector+' img').boundingBox();
      assert.equal(box.width,size);assert.equal(box.height,size);
      assert(art.x>=box.x&&art.y>=box.y&&art.x+art.width<=box.x+box.width&&art.y+art.height<=box.y+box.height,'Brand image stays inside chrome');
    }
    fs.mkdirSync(output,{recursive:true});
    await page.screenshot({animations:'disabled',path:path.join(output,'overview-light.png'),fullPage:true});
    await page.emulateMedia({colorScheme:'dark'});
    await page.screenshot({animations:'disabled',path:path.join(output,'overview-dark.png'),fullPage:true});
    await page.emulateMedia({colorScheme:'light'});
    await page.locator('nav [data-page=settings]').click();
    await page.locator('[data-settings-tab=hosts]').click();
    await page.locator('[data-edit=host1]').click();
    await page.locator('#discover-devices').click();
    const network = page.locator('#device-selects .multi-trigger').filter({hasText:'网络'});
    const panel = page.locator('.multi-panel:visible');
    await network.click();
    await panel.getByRole('button',{name:'清空',exact:true}).click();
    assert.equal(await panel.locator('input[type=checkbox]:checked').count(),0);
    assert.equal(await panel.isVisible(),true);
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('#editor').evaluate(e=>e.open),true);
    assert.equal(await network.evaluate(e=>e===document.activeElement),true);
    await page.locator('#editor-form button[type=submit]').click();await commitSettings(page);
    assert.ok((await page.evaluate(()=>window.saved.at(-1).hosts[0].devices)).includes('network:__none__'));
    await page.locator('[data-edit=host1]').click();
    await page.locator('#discover-devices').click();
    await network.click();
    assert.equal(await panel.locator('input[type=checkbox]:checked').count(),0);
    await panel.getByRole('button',{name:'全选',exact:true}).click();
    assert.equal(await panel.locator('input[type=checkbox]:checked').count(),3);
    await panel.getByRole('searchbox').fill('eth');
    await panel.getByRole('button',{name:'反选结果'}).click();
    await panel.getByRole('searchbox').fill('');
    assert.equal(await panel.locator('input[value=ib0]').isChecked(),true);
    assert.equal(await panel.locator('input[value=eth0]').isChecked(),false);
    await page.keyboard.press('Escape');
    await page.locator('#device-selects .multi-trigger').filter({hasText:'CPU'}).click();
    assert.equal(await panel.locator('input[type=checkbox]').count(),256);
    assert.equal(await panel.locator('input[value=cpu]').count(),0);
    await panel.getByRole('searchbox').fill('cpu25');
    await panel.getByRole('button',{name:'清空结果'}).click();
    await page.screenshot({animations:'disabled',path:path.join(output,'server-selection-light.png'),fullPage:true});
    await page.keyboard.press('Escape');
    await page.evaluate(()=>window.discoveryFails=true);
    await page.locator('#discover-devices').click();
    assert.match(await page.locator('#discovery-status').textContent(),/连接失败/);
    await network.click();
    assert.equal(await panel.locator('input[value=ib0]').isChecked(),true);
    await page.keyboard.press('Escape');
    await discardEditor(page);
    assert.equal(await page.evaluate(()=>window.saved.length),1);
    // A new dialog must discard old popovers and drafts, including failed discovery.
    assert.equal(await page.locator('.multi-panel').count(),0);
    await page.locator('[data-edit=host1]').click();
    await network.click();
    assert.equal(await panel.locator('input:checked').count(),0);
    await page.keyboard.press('Escape');
    await discardEditor(page);
    await page.locator('#window-maximize').click();
    assert.equal(await page.locator('#window-maximize').getAttribute('aria-label'),'还原');
    await page.locator('#window-close').click();
    assert.deepEqual(await page.evaluate(()=>window.nativeCalls),['maximize','close']);
    await page.setViewportSize({width:840,height:600});
    await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});
    await page.locator('[data-edit=host1]').click();
    await page.evaluate(()=>window.discoveryFails=false);
    await page.locator('#discover-devices').click();
    await page.locator('#device-selects .multi-trigger').filter({hasText:'文件系统'}).click();
    const bounds=await panel.boundingBox();
    assert.ok(bounds.x>=0&&bounds.y>=0&&bounds.x+bounds.width<=841&&bounds.y+bounds.height<=601);
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
    const editorBounds=await page.locator('#editor').boundingBox();
    assert.ok(editorBounds.y>=40&&editorBounds.y+editorBounds.height<=600);
    await page.screenshot({animations:'disabled',path:path.join(output,'server-selection-dark-compact.png'),fullPage:true});
    await page.keyboard.press('Escape');
    await discardEditor(page);
    // Explicit legacy IDs remain editable, while disabled categories retain their values.
    await page.evaluate(()=>{state.settings.hosts[0].devices=['network:missing0'];window.fixtureSettings=structuredClone(state.settings);state.settingsDraft=null;state.settingsBaseline=null;renderSettings();});
    await page.locator('[data-edit=host1]').click();
    await page.locator('#discover-devices').click();
    await network.click();
    assert.equal(await panel.locator('input[value=missing0]').isChecked(),true);
    assert.equal(await panel.getByText('暂不可用').count(),1);
    await page.keyboard.press('Escape');
    await page.locator('#metric-select .multi-trigger').click();
    await panel.locator('input[value=network]').uncheck();
    await page.keyboard.press('Escape');
    assert.equal(await network.isDisabled(),true);
    await page.locator('#detail-select .multi-trigger').click();
    await panel.getByRole('button',{name:'清空',exact:true}).click();
    await page.keyboard.press('Escape');
    await page.locator('#editor-form button[type=submit]').click();await commitSettings(page);
    assert.ok((await page.evaluate(()=>window.saved.at(-1).hosts[0].details)).includes('networkTotals'));
    assert.deepEqual(await page.evaluate(()=>window.saved.at(-1).hosts[0].devices),['network:missing0']);
    await page.emulateMedia({forcedColors:'active'});
    assert.equal(await page.locator('body').evaluate(e=>getComputedStyle(e).backgroundColor),'rgb(0, 0, 0)');
    for(const scale of [1.5,2]) {
      const scaled=await browser.newPage({viewport:{width:840,height:600},deviceScaleFactor:scale});
      await scaled.addInitScript(fixture);
      await scaled.goto(`http://127.0.0.1:${server.address().port}`);
      await scaled.locator('.stat-value').first().waitFor();
      assert.equal(await scaled.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
      await scaled.screenshot({animations:'disabled',path:path.join(output,`overview-scale-${scale}.png`)});
      await scaled.close();
    }
    await page.emulateMedia({forcedColors:'none',colorScheme:'light'});
    await page.setViewportSize({width:1120,height:800});
    await page.locator('[data-page=agent]').click();
    await page.locator('.daily>summary').click();
    await page.locator('[data-day-toggle]').first().click();
    await page.locator('[data-day-toggle]').first().focus();
    await page.evaluate(()=>loadDashboard());
    assert.equal(await page.locator('.daily[open]').count(),1);
    assert.equal(await page.locator('tbody[open]').count(),1);
    assert.equal(await page.locator('[data-day-toggle]').first().evaluate(e=>e===document.activeElement),true);

    await page.locator('[data-page=settings]').click();
    assert.deepEqual(await page.locator('[data-settings-tab]').allTextContents(),['Agents','服务器','价格','账户','定时唤醒','通用']);
    assert.equal(await page.locator('[data-add-query]').count(),0);
    await page.locator('[data-settings-tab=sources]').click();
    await page.evaluate(()=>AieyesAgentSettings.advanced(null,'codex'));
    await page.locator('#field-path').fill('');
    await page.locator('#editor-form button[type=submit]').click();
    await page.waitForFunction(()=>!editorSaving);
    assert.match(await page.locator('#editor-error').textContent(),/请输入数据目录/);
    assert.equal(await page.locator('#editor-error').isVisible(),true);
    assert.equal(await page.locator('.shell').evaluate(e=>e.inert),true);
    const callsBefore=await page.evaluate(()=>window.nativeCalls.length);
    await page.locator('#window-minimize').click();
    await page.locator('#window-maximize').click();
    await page.locator('#window-close').click();
    assert.deepEqual(await page.evaluate(n=>window.nativeCalls.slice(n),callsBefore),['minimize','maximize','close']);
    assert.equal(await page.locator('#editor').evaluate(e=>e.open),true);
    await page.locator('#editor-form button[type=submit]').focus();
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('#window-minimize').evaluate(e=>e===document.activeElement),true);
    await page.screenshot({animations:'disabled',path:path.join(output,'editor-validation-fixed.png')});
    await page.keyboard.press('Escape');
    if(await page.locator('#editor-confirm-discard').isVisible())await page.locator('#editor-confirm-discard').click();
    assert.equal(await page.locator('.shell').evaluate(e=>e.inert),false);
    assert.equal(await page.locator('[data-settings-tab=sources]').evaluate(e=>e===document.activeElement),true);

    // Account/machine flows are covered against the real core in test-agent-settings-ui.
    // Source-specific advanced settings still round trip without changing identity.
    await page.evaluate(()=>AieyesAgentSettings.advanced(null,'codex'));
    await page.locator('#field-sourceMode').selectOption('custom');
    await page.locator('#field-sourceProtocol').selectOption('socks5h');
    await page.locator('#field-sourceHost').fill('::1');
    await page.locator('#field-sourcePort').fill('1080');
    await page.locator('#editor-form button[type=submit]').click();await commitSettings(page);
    assert.equal(await page.evaluate(()=>state.settings.sources[0].accountId),'');
    await page.locator('[data-machine-directories=codex][data-machine=local]').click();await page.locator('[data-source-advanced]').first().click();
    assert.equal(await page.locator('#field-sourceHost').inputValue(),'::1');
    assert.equal(await page.locator('#field-sourceProtocol').inputValue(),'socks5h');
    await discardEditor(page);

    await page.locator('[data-settings-tab=general]').click();
    await page.locator('#field-refreshSeconds').fill('1');
    await page.evaluate(()=>document.activeElement?.blur());await page.waitForFunction(()=>!state.generalSavePending&&!state.settingsCommitting&&!state.settingsSaving);
    await page.waitForFunction(()=>!state.busy);
    assert.equal(await page.evaluate(()=>state.settings.refreshSeconds),300);
    assert.equal(await page.evaluate(()=>window.fixtureSettings.refreshSeconds),300);
    await page.locator('[data-settings-tab=hosts]').click();
    await page.evaluate(()=>window.settingsSaveFails=true);
    await page.locator('[data-enable=host1]').click();
    await page.waitForFunction(()=>!state.busy);
    await page.waitForFunction(()=>!state.settingsSaving);assert.equal(await page.locator('[data-enable=host1]').isChecked(),true);
    assert.equal(await page.evaluate(()=>state.settings.hosts[0].enabled),true);
    await page.evaluate(()=>window.settingsSaveFails=false);
    await page.locator('[data-settings-tab=general]').click();await page.locator('#field-refreshSeconds').fill('300');await commitSettings(page);
    assert.equal(await page.evaluate(()=>window.fixtureSettings.hosts[0].enabled),true);

    await page.locator('[data-settings-tab=general]').click();
    assert.equal(await page.locator('#field-githubRepository').count(),0);
    assert.equal(await page.locator('#field-refreshSeconds').inputValue(),'300');await page.locator('#field-refreshSeconds').fill('300');
    await page.locator('#field-appMode').selectOption('custom');
    await page.locator('#field-appHost').fill('proxy.example');
    await page.locator('#field-appPort').fill('8443');
    await page.locator('#field-appProtocol').selectOption('https');
    await page.evaluate(()=>document.activeElement?.blur());await page.waitForFunction(()=>!state.generalSavePending&&!state.settingsCommitting&&!state.settingsSaving);
    await page.waitForFunction(()=>!state.busy);
    assert.equal(await page.evaluate(()=>state.settings.proxy.url),'https://proxy.example:8443');
    await page.locator('#field-appPort').fill('0');
    await page.evaluate(()=>document.activeElement?.blur());await page.waitForFunction(()=>!state.generalSavePending&&!state.settingsCommitting&&!state.settingsSaving);
    await page.waitForFunction(()=>!state.busy);
    assert.equal(await page.evaluate(()=>state.settings.proxy.url),'https://proxy.example:8443');
    await page.locator('[data-settings-tab=general]').click();
    assert.equal(await page.locator('#field-appPort').inputValue(),'0','Invalid proxy draft is retained');await page.locator('#field-appPort').fill('8443');
    await page.locator('#field-appProtocol').selectOption('url');
    await page.locator('#field-appURL').fill('socks5://localhost:1080');
    await page.evaluate(()=>document.activeElement?.blur());await page.waitForFunction(()=>!state.generalSavePending&&!state.settingsCommitting&&!state.settingsSaving);
    await page.waitForFunction(()=>!state.busy);
    assert.equal(await page.evaluate(()=>state.settings.proxy.url),'socks5://localhost:1080');
    await page.locator('#updates').click();
    await page.waitForFunction(()=>!state.busy);
    await page.waitForFunction(()=>document.querySelector('#update-result').textContent.includes('当前已是最新版本'));
    assert.match(await page.locator('#update-current').textContent(),/0\.1\.1/);
    assert.equal(await page.locator('#update-result a').count(),0);
    assert.equal(await page.evaluate(()=>window.engineCalls.filter(c=>c.method==='updates.open').length),0);
    await page.screenshot({animations:'disabled',path:path.join(output,'general-light.png'),fullPage:true});

    await page.locator('[data-settings-tab=prices]').click();
    await page.locator('#price-search').fill(' SONNET ');
    assert.equal(await page.locator('.prices-list [data-price]').count(),1);
    await page.locator('[data-price]').click();
    await page.locator('#editor-form button[type=submit]').click();
    await page.waitForFunction(()=>!document.querySelector('#editor').open);await commitSettings(page);
    assert.equal(await page.locator('#price-search').inputValue(),' SONNET ');
    await page.locator('#price-search').fill('no-such-model');
    assert.match(await page.locator('.prices-list').textContent(),/无匹配模型/);
    await page.locator('#price-search').fill('anthropic');
    await page.locator('#sync-prices').click();
    await page.waitForFunction(()=>!state.busy);
    assert.equal(await page.locator('#price-search').inputValue(),'anthropic');
    assert.equal(await page.locator('.prices-list [data-price]').count(),1);
    await page.screenshot({animations:'disabled',path:path.join(output,'prices-light.png'),fullPage:true});

    await page.locator('[data-page=agent]').click();
    assert.equal(await page.locator('.stat').count(),3);
    assert.equal(await page.locator('.stat small').count(),0);
    assert.equal(await page.locator('#repair-pricing').count(),0);
    assert.equal(await page.getByRole('heading',{name:'数据来源',exact:true}).count(),0);
    assert.deepEqual(await page.locator('.token-breakdown strong').allTextContents(),['240.0K','180.0K','160.0K','20.0K','38.1%']);
    assert.equal(await page.locator('.token-breakdown').evaluate(el=>[...el.children].every(row=>row.querySelector('strong').getBoundingClientRect().top>=row.querySelector('span').getBoundingClientRect().bottom)),true);
    await page.evaluate(()=>{state.settings.accounts=[{id:'fixture-account',provider:'codex',name:'测试账户',quotaEnabled:true}];state.settings.sources[0].accountId='fixture-account';state.settings.sources.push({...state.settings.sources[0],id:'second-source'});window.fixtureSettings=structuredClone(state.settings);});
    await page.evaluate(()=>{const src=state.settings.sources[0],account=state.settings.accounts[0];state.dashboard.sources=[{...src,accountIds:['',account.id]}];state.accountKey=account.provider+':'+account.id;renderAgent();});
    assert.equal(await page.locator('#source option').count(),3);
    await page.evaluate(()=>{state.accountKey='';renderAgent();});
    await page.evaluate(()=>{state.dashboard.summary.pricedTokens=599999;renderAgent();});
    assert.equal(await page.locator('#repair-pricing').count(),1);
    await page.locator('#repair-pricing').click();
    assert.equal(await page.locator('[data-settings-tab=prices].active').count(),1);
    await page.evaluate(()=>{state.page='agent';state.dashboard.summary.total=0;state.dashboard.summary.pricedTokens=0;render();});
    assert.equal(await page.locator('#repair-pricing').count(),0);

    const sampleData={timestamp:Date.now()/1000,load:[0.3,0.4,0.5],errors:{},cpu:[{id:'cpu',utilization:42},{id:'cpu0',utilization:96}],memory:{total:16000000000,available:4000000000,cached:1000000000,buffers:50000000,swapTotal:0,swapFree:0},gpu:[{id:'0',name:'Training GPU',utilization:91,memoryUsedMiB:7500,memoryTotalMiB:10000,temperature:70,powerWatts:210},{id:'1',name:'Offline GPU',utilization:null,memoryUsedMiB:null,memoryTotalMiB:null}],filesystems:[{id:'/',used:50,total:100,available:50},{id:'/empty',used:0,total:0}],disk:[{id:'nvme0n1',readBytesPerSecond:1024,writeBytesPerSecond:2048,busyMsPerSecond:1200}],network:[{id:'eth0',rxBytesPerSecond:2048,txBytesPerSecond:4096}]};
    await page.evaluate(sample=>{window.fixtureHosts=[{id:'host1',sample}];state.settings.hosts[0].enabled=true;state.settings.hosts[0].details=Object.keys(detailOptions);state.settings.hosts[0].metrics=Object.keys(groups);state.settings.hosts[0].devices=[];},sampleData);
    await page.locator('[data-page=servers]').click();
    await page.waitForFunction(()=>!state.serverBusy);
    await page.waitForFunction(()=>document.querySelector('.resource-ring strong')?.textContent==='42.0%');
    assert.equal(await page.locator('.resource-ring').count(),2);
    assert.deepEqual(await page.locator('.resource-ring strong').allTextContents(),['42.0%','75.0%']);
    assert.equal(await page.locator('.host-status').textContent(),'正常');
    await page.locator('.server-card > summary').click();
    for(const group of ['cpu','memory','gpu','filesystems','disk','network'])await page.locator(`[data-metric="host1:${group}"]>summary`).click();
    assert.equal(await page.locator('.resource-bar[aria-label="忙碌率"]').getAttribute('aria-valuenow'),'100');
    assert.equal(await page.locator('[data-filesystem="/empty"] .resource-bar').getAttribute('role'),'img');
    assert.equal(await page.locator('[data-metric="host1:network"] .resource-bar').count(),0);
    await page.evaluate(()=>{notify('');window.scrollTo(0,0);});
    await page.screenshot({animations:'disabled',path:path.join(output,'servers-light.png'),fullPage:true});
    await page.evaluate(()=>sample());
    assert.equal(await page.locator('[data-metric][open]').count(),6);
    await page.emulateMedia({colorScheme:'dark'});
    await page.evaluate(()=>window.scrollTo(0,0));
    await page.screenshot({animations:'disabled',path:path.join(output,'servers-dark.png'),fullPage:true});
    await page.setViewportSize({width:840,height:600});
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
    await page.evaluate(()=>window.scrollTo(0,0));
    await page.screenshot({animations:'disabled',path:path.join(output,'servers-dark-compact.png'),fullPage:true});
    for(const [kind,label] of [['stale','数据已延迟'],['partial','部分采集失败'],['failed','连接失败'],['paused','已暂停'],['waiting','等待采样']]){
      await page.evaluate(kind=>{state.settings.hosts[0].enabled=kind!=='paused';state.hosts=kind==='waiting'?[]:[{id:'host1',error:kind==='failed'?'连接失败':null,sample:{...window.fixtureHosts[0].sample,timestamp:Date.now()/1000-(kind==='stale'?20:0),errors:kind==='partial'?{gpu:'Unavailable'}:{}}}];renderServers();},kind);
      assert.equal(await page.locator('.host-status').textContent(),label);
    }

    // Startup queries configured accounts without any manual action.
    const quotaPage=await browser.newPage({viewport:{width:1120,height:800}});
    quotaPage.on('pageerror',error=>errors.push(String(error)));
    await quotaPage.addInitScript(fixture);
    await quotaPage.addInitScript(()=>{
      window.fixtureSettings.accounts=[{id:'balance',provider:'deepseek',name:'Balance',quotaEnabled:true,archived:false}];
      window.fixtureSettings.sources=[{id:'balance-source',provider:'deepseek',name:'Balance',accountId:'balance',enabled:true,path:''}];
    });
    await quotaPage.goto(`http://127.0.0.1:${server.address().port}`);
    await quotaPage.waitForFunction(()=>typeof state!=='undefined'&&state.lastQuota>0&&!state.busy);
    assert.equal(await quotaPage.evaluate(()=>window.engineCalls.filter(c=>c.method==='quotas.refresh').length),1);
    await quotaPage.evaluate(async()=>{
      window.quotaFails=true;quotaScheduleKnown=true;quotaDueAt=Date.now()-1;state.lastScan=Date.now();
      runScheduledRefreshes();
    });
    await quotaPage.waitForFunction(()=>!state.busy);
    assert.equal(await quotaPage.evaluate(()=>window.engineCalls.filter(c=>c.method==='quotas.refresh').length),2);
    await quotaPage.evaluate(()=>runScheduledRefreshes());
    assert.equal(await quotaPage.evaluate(()=>window.engineCalls.filter(c=>c.method==='quotas.refresh').length),2);
    await quotaPage.evaluate(()=>{state.settings.accounts[0].archived=true;window.fixtureSettings.accounts[0].archived=true;invalidateQuotaSchedule();runScheduledRefreshes();});
    assert.equal(await quotaPage.evaluate(()=>window.engineCalls.filter(c=>c.method==='quotas.refresh').length),2);
    await quotaPage.close();

    // The floating window expands into the compact panel and mirrors the menu bar layout.
    const floating=await browser.newPage({viewport:{width:420,height:640}});
    floating.on('pageerror',error=>errors.push(String(error)));
    await floating.addInitScript(fixture);
    await floating.addInitScript(()=>{
      window.fixtureHosts=[{id:'host1',sample:{timestamp:Date.now()/1000,load:[0.2,0.3,0.4],errors:{},cpu:[{id:'cpu',utilization:42}],memory:{total:16000000000,available:4000000000,cached:1000000000,buffers:50000000,swapTotal:0,swapFree:0}}}];
    });
    await floating.goto(`http://127.0.0.1:${server.address().port}/floating.html`);
    await floating.locator('#phase').waitFor();
    assert.equal(await floating.locator('body').getAttribute('data-view'),'ball');
    assert.equal(await floating.locator('#panel').isVisible(),false);
    await floating.locator('#ball').click();
    await floating.waitForFunction(()=>window.desktopInfo.panelOpen===true);
    assert.equal(await floating.locator('body').getAttribute('data-view'),'panel');
    assert.equal(await floating.locator('.panel-tabs [data-page=agent].active').count(),1);
    await floating.locator('.panel-stats .stat-value').first().waitFor();
    assert.equal(await floating.locator('.panel-stats .stat').count(),2);
    assert.equal(await floating.locator('#live-sessions').isVisible(),true);
    assert.equal(await floating.locator('#trend').isVisible(),false);await floating.locator('[data-agent-detail=panel-trend] > summary').click();assert.equal(await floating.locator('#trend').isVisible(),true);
    assert.equal(await floating.evaluate(()=>window.engineCalls.filter(c=>c.method==='dashboard').length>0),true);
    await floating.screenshot({animations:'disabled',path:path.join(output,'floating-panel-light.png')});
    await floating.locator('.panel-tabs [data-page=servers]').click();
    await floating.locator('.server-card').first().waitFor();
    await floating.locator('.server-card > summary').first().click();
    await floating.locator('.server-card .resource-ring').first().waitFor();
    assert.equal(await floating.locator('.server-card .resource-ring').count(),2);
    await floating.locator('.panel-tabs [data-page=agent]').click();
    await floating.locator('.panel-stats').waitFor();
    await floating.locator('#panel-more > summary').click();
    await floating.locator('#panel-pin').click();
    await floating.locator('#panel-more > summary').click();
    await floating.waitForFunction(()=>window.desktopInfo.panelPinned===true);
    await floating.locator('#panel-refresh').click();
    await floating.locator('[data-refresh=quotas]').click();
    await floating.waitForFunction(()=>window.engineCalls.some(c=>c.method==='quotas.refresh'));
    await floating.locator('#panel-detail').click();
    await floating.waitForFunction(()=>window.desktopCalls.some(c=>c.command==='desktop_detail'));
    assert.equal(await floating.evaluate(()=>window.desktopInfo.panelOpen),false);
    await floating.mouse.move(1,1);
    await floating.waitForTimeout(800);
    await floating.locator('#ball').hover();
    await floating.waitForTimeout(400);
    assert.equal(await floating.evaluate(()=>window.desktopInfo.panelOpen),false);
    await floating.locator('#ball').click();
    await floating.waitForFunction(()=>window.desktopInfo.panelOpen===true);
    await floating.keyboard.press('Escape');
    await floating.waitForFunction(()=>window.desktopInfo.panelOpen===false);
    await floating.emulateMedia({colorScheme:'dark'});
    await floating.locator('#ball').click();
    await floating.waitForFunction(()=>window.desktopInfo.panelOpen===true);
    await floating.screenshot({animations:'disabled',path:path.join(output,'floating-panel-dark.png')});
    await floating.close();
    assert.deepEqual(errors,[]);
    console.log(`Browser integration checks passed; previews: ${output}`);
  } finally {
    await browser?.close();
    await new Promise(resolve=>server.close(resolve));
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
