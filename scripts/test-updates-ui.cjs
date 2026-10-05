const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require('../apps/desktop/node_modules/playwright');
const root = path.resolve(__dirname, '../apps/desktop/web');
function fixture() {
  const usage={tokens:{input:0,output:0,cacheRead:0,cacheWrite:0},total:0,cost:0,pricedTokens:0,events:0};
  const settings={accounts:[],sources:[],hosts:[],proxy:{mode:'system',url:''},modelMappings:{},refreshSeconds:300,serverRefreshSeconds:10};
  window.updateTest={calls:[],next:'current',listeners:{},status:{currentVersion:'1.2.3',latestVersion:null,notes:'',phase:'idle',message:'',automatic:true,prompt:false}};
  window.updateTest.emit=next=>{Object.assign(window.updateTest.status,next);for(const fn of window.updateTest.listeners['updates:status']??[])fn({payload:structuredClone(window.updateTest.status)});};
  window.setInterval=()=>0;
  window.__TAURI__={event:{async listen(name,fn){(window.updateTest.listeners[name]??=[]).push(fn);}},core:{async invoke(command,args){
    const t=window.updateTest;t.calls.push(command);
    if(command==='updates_info')return structuredClone(t.status);
    if(command==='updates_preferences'){t.status.automatic=args.automatic;return structuredClone(t.status);}
    if(command==='updates_later'){t.status.prompt=false;return structuredClone(t.status);}
    if(command==='updates_check'){
      t.emit({phase:'checking',message:'正在检查更新…',prompt:false});await new Promise(resolve=>setTimeout(resolve,60));
      t.emit(t.next==='current'?{phase:'current',message:'当前已是最新版本 v1.2.3',latestVersion:'1.2.3'}:t.next==='newer'?{phase:'current',message:'当前版本 v1.2.3 高于最新稳定版 v1.0.0',latestVersion:'1.0.0'}:t.next==='error'?{phase:'error',message:'更新连接失败'}:{phase:'available',message:'发现新版本 v1.2.4',latestVersion:'1.2.4',notes:'改进与修复\n<script>bad()</script>',prompt:true});
      return structuredClone(t.status);
    }
    if(command==='updates_install'){
      t.emit({phase:'downloading',message:'正在下载更新…',downloaded:1024,total:4096,prompt:false});
      await new Promise(resolve=>setTimeout(resolve,150));
      t.emit({phase:'error',message:'更新未完成：签名校验失败'});return structuredClone(t.status);
    }
    if(command==='desktop_info')return {platform:'linux',mode:'tray',effectiveMode:'tray',sessions:[],summary:'无活跃会话'};
    if(command!=='engine_call')return {};
    switch(args.method){
      case 'hello':return {version:'1.2.3'};
      case 'settings.get':return structuredClone(settings);
      case 'settings.save':Object.assign(settings,args.params);return {};
      case 'dashboard':return {summary:usage,quotas:[],sources:[],models:[],dayModels:[],modelOptions:[],trendDays:[],heatmap:[],pricingGaps:[]};
      case 'sources.scan':case 'prices.list':return [];
      default:return {};
    }
  }}};
}
(async()=>{
  const server=http.createServer((req,res)=>{const name=path.basename(new URL(req.url,'http://localhost').pathname)||'index.html';const file=path.join(root,name);if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}res.setHeader('content-type',({'.html':'text/html','.js':'application/javascript','.css':'text/css'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  let browser;
  try{
    browser=await chromium.launch({headless:true});const page=await browser.newPage();const errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);
    await page.goto(`http://127.0.0.1:${server.address().port}`);await page.waitForFunction(()=>window.AieyesUpdates&&state.settings&&!state.busy);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='general';renderSettings();});
    assert.match(await page.locator('#update-current').textContent(),/1\.2\.3/);
    await page.locator('#updates').click();await page.waitForFunction(()=>document.querySelector('#update-result').textContent.includes('当前已是最新'));
    assert.equal(await page.locator('#update-dialog[open]').count(),0);
    await page.evaluate(()=>updateTest.next='newer');await page.locator('#updates').click();await page.waitForFunction(()=>document.querySelector('#update-result').textContent.includes('高于'));
    await page.evaluate(()=>updateTest.next='available');await page.locator('#updates').click();await page.waitForSelector('#update-dialog[open]');
    assert.match(await page.locator('#update-versions').textContent(),/1\.2\.3.*1\.2\.4/);
    assert.equal(await page.locator('#update-notes script').count(),0);
    await page.locator('#update-later').click();assert.equal(await page.locator('#update-dialog[open]').count(),0);
    await page.locator('#field-refreshSeconds').fill('123');await page.locator('#update-view').click();await page.locator('#update-install').click();
    assert.match(await page.locator('#update-message').textContent(),/保存设置/);
    assert.equal(await page.evaluate(()=>updateTest.calls.filter(x=>x==='updates_install').length),0);
    await page.locator('#update-later').click();await page.locator('#general-form button').click();await page.waitForFunction(()=>!state.busy);
    await page.locator('#update-view').click();await page.locator('#update-install').click();
    await page.waitForFunction(()=>document.querySelector('#update-message').textContent.includes('签名校验失败'));
    assert.equal(await page.evaluate(()=>updateTest.calls.filter(x=>x==='updates_install').length),1);
    assert.equal(await page.locator('#update-install').textContent(),'重新检查');
    await page.locator('#update-later').click();await page.locator('#updates-automatic').uncheck();assert.equal(await page.evaluate(()=>updateTest.status.automatic),false);
    await page.evaluate(()=>updateTest.next='error');await page.locator('#updates').click();await page.waitForFunction(()=>document.querySelector('#update-result').textContent.includes('连接失败'));
    assert.deepEqual(errors,[]);
    const previews=path.resolve(__dirname,'../.local/ui-previews');
    fs.mkdirSync(previews,{recursive:true});await page.screenshot({path:path.join(previews,'updates-settings.png')});
    console.log('Update UI: current/newer/new versions, safe notes, deferral, unsaved settings, progress, signature failure, errors and automatic preference passed');
  }finally{await browser?.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
