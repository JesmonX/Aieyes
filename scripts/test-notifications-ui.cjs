// Production UI with a synthetic native notification transport; no OS notifications are sent.
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const fixture=require('./fixtures/browser-ui.cjs');
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web');
  const server=http.createServer((req,res)=>{
    if(require('./ui-test-helpers.cjs').serveFont(req,res))return;
    const file=path.join(root,path.basename(new URL(req.url,'http://test').pathname)||'index.html');
    try{res.setHeader('content-type',({'.js':'text/javascript','.html':'text/html','.css':'text/css'})[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));}catch{res.writeHead(404);res.end();}
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));let browser;
  try{
    browser=await chromium.launch({headless:true});
    const page=await browser.newPage({viewport:{width:1024,height:800}}),errors=[];
    page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture);
    await page.addInitScript(()=>{
      const original=__TAURI__.core.invoke;
      window.notificationFixture={permission:'denied',tests:0,fail:false,sounds:[]};
      __TAURI__.core.invoke=async(command,args)=>{
        if(command==='session_sounds_preview'){notificationFixture.sounds.push(args.kind);return;}
        if(command==='notifications_info')return {permission:notificationFixture.permission};
        if(command==='notifications_test'){notificationFixture.tests++;if(notificationFixture.fail)throw Error('模拟系统通知失败');return;}
        return original(command,args);
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);await page.evaluate(()=>AieyesApp.ready);
    await page.waitForFunction(()=>!state.busy&&!state.quotaBusy);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='general';state.generalTab='notifications';render();});
    await page.waitForFunction(()=>document.querySelector('#notification-permission')?.textContent.includes('未授权'));
    assert.equal(await page.locator('[name=needsInputSound]').isChecked(),true);
    assert.equal(await page.locator('[name=completeSound]').isChecked(),true);
    await page.locator('[data-sound-preview=needsInput]').click();
    await page.locator('[data-sound-preview=complete]').click();
    assert.deepEqual(await page.evaluate(()=>notificationFixture.sounds),['needsInput','complete']);
    await page.locator('[name=needsInputSound]').uncheck();
    await page.locator('[name=completeSound]').uncheck();
    await page.waitForFunction(()=>state.settingsDraft.sessionSounds?.needsInput===false && state.settingsDraft.sessionSounds?.complete===false);
    await page.evaluate(()=>notificationFixture.fail=true);await page.locator('#notification-test').click();
    await page.waitForFunction(()=>document.querySelector('#notification-error')?.textContent.includes('模拟系统通知失败'));
    assert.equal(await page.locator('#notification-test').isEnabled(),true);
    await page.evaluate(()=>{notificationFixture.fail=false;notificationFixture.permission='granted';});
    await page.locator('#notification-test').click();
    await page.waitForFunction(()=>document.querySelector('#notification-permission')?.textContent.includes('接口可用')&&!document.querySelector('#notification-error').textContent);
    assert.equal(await page.evaluate(()=>notificationFixture.tests),2);
    // Expired quota snapshots in the webview must not create a second automatic poller.
    const count=await page.evaluate(()=>engineCalls.filter(c=>c.method==='quotas.refresh').length);
    await page.evaluate(()=>{
      state.dashboard.quotas=[{provider:'codex',accountId:'fixture',windows:[{id:'primary',name:'5h',usedPercent:0,resetsAt:Date.now()/1000-60}]}];
      state.lastScan=Date.now();runScheduledRefreshes();runScheduledRefreshes();
    });
    assert.equal(await page.evaluate(()=>engineCalls.filter(c=>c.method==='quotas.refresh').length),count);
    await page.setViewportSize({width:640,height:480});
    assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth));
    assert.deepEqual(errors,[]);
    console.log('Notification UI: permission status, failed-test recovery, minimum width and native-owned quota polling passed');
  }finally{if(browser)await browser.close();server.close();}
})().catch(error=>{console.error(error);process.exitCode=1;});
