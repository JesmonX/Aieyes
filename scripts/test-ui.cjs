// Browser integration test with in-memory IPC; never reads personal settings.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require(process.env.AIEYES_PLAYWRIGHT || '../apps/desktop/node_modules/playwright');
const root = path.resolve(__dirname, '../apps/desktop/web');
const output = path.resolve(__dirname, '../.local/ui-previews');

(async () => {
  const server = http.createServer((req, res) => {
    const name = path.basename(new URL(req.url, 'http://localhost').pathname) || 'index.html';
    const file = path.join(root, name);
    if (!fs.existsSync(file)) { res.writeHead(404); res.end(); return; }
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
      window.fixtureSettings={accounts:[],sources:[],hosts:[{id:'host1',name:'训练服务器',target:'gpu-lab',port:null,identityFile:'',shell:'/bin/sh',preCommand:'',enabled:true,metrics:groups,devices:[],details}],proxy:{mode:'system',url:''},refreshSeconds:300,serverRefreshSeconds:10,githubRepository:'JesmonX/Aieyes',modelMappings:{}};
      window.saved=[]; window.discoveryFails=false; window.nativeCalls=[]; let maximized=false;
      window.__TAURI__={
        event:{async listen(){return ()=>{};}},
        window:{getCurrentWindow:()=>({async isMaximized(){return maximized;},async toggleMaximize(){maximized=!maximized;window.nativeCalls.push('maximize');},async minimize(){window.nativeCalls.push('minimize');},async close(){window.nativeCalls.push('close');},async onResized(){return ()=>{};}})},
        core:{async invoke(command,args){
          if(command==='desktop_info')return {platform:'windows',material:'opaque',mode:'auto',effectiveMode:'floating',sessions:[],summary:'暂无活跃会话'};
          if(command!=='engine_call')return {};
          switch(args.method){
            case 'hello':return {version:'0.1.0'};
            case 'settings.get':return structuredClone(window.fixtureSettings);
            case 'settings.save':window.fixtureSettings=structuredClone(args.params);window.saved.push(structuredClone(args.params));return {};
            case 'sources.scan':return {};
            case 'dashboard':return {summary:usage,quotas:[],models:[{...usage,key:'Claude Sonnet'}],trendDays:trend,dayModels:trend.map(row=>({day:row.key,model:'Claude Sonnet',usage:row})),heatmap:history,sources:[],pricingGaps:[]};
            case 'hosts.sample':return [];
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
    await page.locator('#editor-form button[type=submit]').click();
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
    await page.locator('#cancel-editor').click();
    assert.equal(await page.evaluate(()=>window.saved.length),1);
    // A new dialog must discard old popovers and drafts, including failed discovery.
    assert.equal(await page.locator('.multi-panel').count(),0);
    await page.locator('[data-edit=host1]').click();
    await network.click();
    assert.equal(await panel.locator('input:checked').count(),0);
    await page.keyboard.press('Escape');
    await page.locator('#cancel-editor').click();
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
    await page.screenshot({animations:'disabled',path:path.join(output,'server-selection-dark-compact.png'),fullPage:true});
    await page.keyboard.press('Escape');
    await page.locator('#cancel-editor').click();
    // Explicit legacy IDs remain editable, while disabled categories retain their values.
    await page.evaluate(()=>{state.settings.hosts[0].devices=['network:missing0'];});
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
    await page.locator('#editor-form button[type=submit]').click();
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
    assert.deepEqual(errors,[]);
    console.log(`Browser integration checks passed; previews: ${output}`);
  } finally {
    await browser?.close();
    await new Promise(resolve=>server.close(resolve));
  }
})().catch(error=>{console.error(error);process.exitCode=1;});
