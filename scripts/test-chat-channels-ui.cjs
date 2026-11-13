/* Offline Feishu settings QA against the real Web UI and an isolated IPC fixture. */
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), http = require('node:http');
const {chromium} = require('../apps/desktop/node_modules/playwright');
const {fixture} = require('./workspace-ui-fixture.cjs');
const {serveWebRoot} = require('./ui-test-helpers.cjs');

function channels(language) {
  const f = window.channelFixture = {
    calls: [], failSave: false,
    status: {enabled:false,appId:'',hasSecret:false,revision:0,paired:false,allowTerminal:false,connection:{state:'disabled'},pending:0,undelivered:0},
    commands: [
      {id:'c-blocked',sessionId:'one',body:'<img src=x onerror=alert(1)>\nKeep original input 用户',status:'blocked',reason:'原终端状态已改变'},
      {id:'c-submitted',sessionId:'one',body:'Already sent',status:'submitted',reason:'已输入原 tmux 分屏，等待会话确认'},
      {id:'c-uncertain',sessionId:'two',body:'Check before resending',status:'uncertain',reason:'请核对原会话'},
    ],
    sessions: [{id:'one',key:'s-one',title:'s-one · Codex · 用户项目',phase:'needsInput',control:'原生会话接口'}],
  };
  navigationFixture.settings.appearance.language = language;
  localStorage.setItem('aieyes.language', language);
  localStorage.setItem('aieyes.appearance', JSON.stringify(navigationFixture.settings.appearance));
  const original = window.__TAURI__.core.invoke;
  window.__TAURI__.core.invoke = async (command, args) => {
    if (command !== 'engine_call' || !args.method.startsWith('channels.')) return original(command, args);
    const {method, params = {}} = args;f.calls.push({method,params:structuredClone(params)});
    if (method === 'channels.status') return structuredClone(f.status);
    if (method === 'channels.commands') return {commands:structuredClone(f.commands)};
    if (method === 'channels.sessions') return {sessions:structuredClone(f.sessions)};
    if (method === 'channels.configure') {
      if (f.failSave || params.revision !== f.status.revision) throw Error('聊天渠道配置已变更，请刷新后重试');
      f.status = {...f.status,revision:f.status.revision+1,enabled:params.enabled,appId:params.appId,hasSecret:true,allowTerminal:params.allowTerminal,connection:{state:'connected'}};
      return structuredClone(f.status);
    }
    if (method === 'channels.pair') return {code:'aieyes-fixture',message:'/pair aieyes-fixture',expiresAt:Date.now()/1000+600};
    if (method === 'channels.unpair') {f.status.paired=false;f.status.revision++;return structuredClone(f.status);}
    if (method === 'channels.retry' || method === 'channels.cancel') {
      const row=f.commands.find(c=>c.id===params.id);
      row.status=method.endsWith('retry')?'queued':'cancelled';return {saved:true};
    }
    throw Error('Unexpected fixture method '+method);
  };
}

(async () => {
  const web = path.resolve(__dirname,'../apps/desktop/web'), output = path.resolve(__dirname,'../.local/channel-implementation/ui');
  fs.mkdirSync(output,{recursive:true});
  const server = http.createServer(serveWebRoot(web,'index.html','application/octet-stream'));
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser = await chromium.launch();
  try {
    for (const [platform,language] of [['windows','zh-CN'],['linux','en']]) {
      const page = await browser.newPage({viewport:{width:1000,height:850},reducedMotion:'reduce'}),errors=[];
      page.on('pageerror',error=>errors.push(String(error)));
      await page.addInitScript(fixture,{platform});await page.addInitScript(channels,language);
      await page.goto(`http://127.0.0.1:${server.address().port}`);await page.evaluate(()=>AieyesApp.ready);
      await page.evaluate(()=>AieyesApp.openSettings({category:'general',section:'channels'}));
      await page.locator('[data-channel-configure]').waitFor();
      await page.waitForFunction(()=>channelFixture.calls.some(c=>c.method==='channels.status'));
      assert(await page.locator('[data-channel-pair]').isDisabled());
      assert.equal(await page.locator('[data-channel-command=c-submitted] button').count(),0);
      assert.equal(await page.locator('[data-channel-command=c-uncertain] button').count(),0);
      assert.equal(await page.locator('[data-channel-command=c-blocked] img').count(),0);
      assert.match(await page.locator('[data-channel-command=c-blocked] .channel-message').innerText(),/<img src=x/);
      assert.equal(await page.locator('[data-channel-command=c-blocked] [data-channel-retry]').count(),1);
      await page.locator('[data-channel-retry]').click();
      await page.waitForFunction(()=>channelFixture.commands[0].status==='queued');
      assert.equal(await page.locator('[data-channel-command=c-blocked] [data-channel-retry]').count(),0);
      await page.locator('[data-channel-command=c-blocked] [data-channel-cancel]').click();
      await page.waitForFunction(()=>channelFixture.commands[0].status==='cancelled');
      await page.locator('[data-channel-configure]').click();
      await page.locator('#editor[open]').waitFor();
      assert.equal(await page.locator('[name=channelAppSecret]').getAttribute('type'),'password');
      await page.locator('[name=channelEnabled]').check();
      await page.locator('[name=channelTerminal]').check();
      await page.locator('[name=channelAppId]').fill('cli_fixture');
      await page.locator('[name=channelAppSecret]').fill('fixture-secret-only');
      await page.evaluate(()=>channelFixture.failSave=true);
      await page.locator('#editor button[type=submit]').click();
      await page.locator('#editor-error:not([hidden])').waitFor();
      assert.equal(await page.locator('[name=channelAppSecret]').inputValue(),'fixture-secret-only');
      assert(await page.locator('#editor').evaluate(el=>el.open));
      await page.evaluate(()=>channelFixture.failSave=false);
      await page.locator('#editor button[type=submit]').click();
      await page.locator('#editor').waitFor({state:'hidden'});
      await page.locator('[data-channel-pair]').click();
      await page.locator('[data-channel-copy]').waitFor();
      assert.match(await page.locator('[data-channel-pairing]').innerText(),/\/pair aieyes-fixture/);
      assert(!(await page.locator('#content').innerText()).includes('fixture-secret-only'));
      await page.locator('[data-channel-configure]').click();
      await page.locator('#editor[open]').waitFor();
      assert.equal(await page.locator('[name=channelAppSecret]').inputValue(),'');
      await page.locator('[name=channelAppId]').fill('cli_unsaved');
      await page.waitForTimeout(5200);
      assert.equal(await page.locator('[name=channelAppId]').inputValue(),'cli_unsaved','Background refresh preserves the editor draft');
      await page.screenshot({path:path.join(output,`editor-${platform}-${language}.png`)});
      // The shared editor owns dirty-draft confirmation; close using its explicit cancel action.
      await page.locator('[name=channelAppId]').fill('cli_fixture');
      await page.locator('#cancel-editor').click();
      await page.locator('#editor').waitFor({state:'hidden'});
      await page.screenshot({path:path.join(output,`settings-${platform}-${language}.png`),fullPage:true});
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1),false);
      if(language==='en') {
        const copy=await page.locator('#channel-settings').innerText();assert.match(copy,/Feishu session control/);assert(!/[\u3400-\u9fff]/.test(copy));
      }
      await page.evaluate(()=>AieyesApp.openSettings({category:'general',section:'appearance'}));
      await page.waitForTimeout(5300);
      const calls=await page.evaluate(()=>channelFixture.calls.length);
      await page.waitForTimeout(300);
      assert.equal(await page.evaluate(()=>channelFixture.calls.length),calls,'Leaving the page stops channel polling');
      assert.deepEqual(errors,[]);
      await page.close();
    }
    console.log('Chat channel UI: Windows/Chinese and Linux/English configuration, pairing, drafts, status, literal text, command controls, and polling passed.');
  } finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
