// Real browser interactions with synthetic IPC. Static layout checks live in
// test-windows-ui; these checks deliberately leave animations enabled.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const fixture=require('./fixtures/browser-ui.cjs');
const root=path.resolve(__dirname,'../apps/desktop/web');
const out=path.resolve(process.env.AIEYES_UI_OUT||path.join(__dirname,'../.local/windows-ui-sync/interactions'));
const settle=page=>page.evaluate(async()=>{
  await Promise.all(document.getAnimations().filter(a=>a.effect?.getComputedTiming().iterations!==Infinity).map(a=>a.finished.catch(()=>{})));
});

(async()=>{
  fs.mkdirSync(out,{recursive:true});
  const server=http.createServer((req,res)=>{
    if(require('./ui-test-helpers.cjs').serveFont(req,res))return;
    const file=path.join(root,path.basename(new URL(req.url,'http://localhost').pathname)||'index.html');
    if(!fs.existsSync(file)){res.writeHead(404);res.end();return;}
    res.setHeader('Content-Type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png'})[path.extname(file)]||'application/octet-stream');
    res.setHeader('Content-Security-Policy',JSON.parse(fs.readFileSync(path.join(root,'../src-tauri/tauri.conf.json'))).app.security.csp);
    res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const url=`http://127.0.0.1:${server.address().port}`;
  const browser=await chromium.launch();
  try {
    const context=await browser.newContext({viewport:{width:1294,height:840},reducedMotion:'no-preference'});
    await context.addInitScript(fixture);
    const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(String(error)));
    await page.goto(url);await page.evaluate(()=>AieyesApp.ready);await page.evaluate(()=>AieyesTheme.loadFonts());await settle(page);
    await page.evaluate(()=>{
      const host=document.createElement('div');host.id='geometry-fixture';
      host.innerHTML='<select id="geometry-native" aria-label="运行位置"><option>Local · 本机</option><option value="remote" selected>Remote · 服务器</option></select>';
      document.body.append(host);AieyesSingleSelect.mountAll(host);
    });
    await page.waitForSelector('#geometry-native-choice');
    const geometry=await page.locator('#geometry-fixture .win-select').evaluateAll(wrappers=>wrappers.map(wrapper=>{
      const a=wrapper.querySelector('select').getBoundingClientRect(),b=wrapper.querySelector('button').getBoundingClientRect();
      return Math.max(...['x','y','width','height'].map(key=>Math.abs(a[key]-b[key])));
    }));
    assert(geometry.every(delta=>delta<.1),'the visible choices retain their native control geometry');

    // App navigation animates once; background refresh never replays it.
    const navigation=await page.evaluate(()=>{
      state.page='settings';state.settingsTab='general';state.generalTab='appearance';render();
      const active=document.querySelector('#settings-panel').getAnimations().map(a=>a.id);
      return {active,key:AieyesSingleSelect.visible(document.querySelector('#field-appearanceTheme')).id};
    });
    assert(navigation.active.includes('page'));
    await settle(page);
    assert.equal(await page.evaluate(()=>{renderSettings(false);return document.querySelector('#settings-panel').getAnimations().length;}),0,'refresh keeps the current page still');
    for(const theme of ['light','dark']){
      await page.locator('#field-appearanceTheme').selectOption(theme);
      await page.waitForFunction(theme=>document.documentElement.dataset.theme===theme&&!state.generalSavePending&&!state.settingsSaving,theme);
      await page.evaluate(()=>document.documentElement.dataset.material='acrylic');
      await settle(page);await page.screenshot({path:path.join(out,`appearance-${theme}.png`)});
      await page.locator('#field-appearanceChartPalette-choice').click();await settle(page);
      await page.screenshot({path:path.join(out,`dropdown-${theme}.png`)});
      await page.keyboard.press('Escape');await settle(page);
    }
    await page.evaluate(()=>{
      AieyesMotion.cancelAll();
      document.querySelector('#content').innerHTML=`<form id="choice-form" class="card" style="display:grid;gap:14px;max-width:540px">
        <label for="choice">运行位置</label><select id="choice" name="location" required>
          <option value="">请选择</option><option value="local" selected>Local · 本机</option>
          <option value="disabled" disabled>不可用设备</option><option value="remote">Remote · 服务器</option>
          <optgroup label="开发机器"><option value="dev">Development · 开发环境</option></optgroup>
        </select><fieldset id="choice-fieldset"><label>另一台机器<select id="other-choice" name="other"><option value="a">Alpha</option><option value="b">Beta</option></select></label></fieldset>
        <button id="after-choice" type="button">下一个控件</button><button type="submit">保存</button></form>`;
      window.choiceEvents={input:0,change:0,submit:0};window.choiceSource=document.querySelector('#choice');
      choiceSource.addEventListener('input',()=>choiceEvents.input++);choiceSource.addEventListener('change',()=>choiceEvents.change++);
      document.querySelector('#choice-form').onsubmit=event=>{event.preventDefault();choiceEvents.submit++;};
      AieyesSingleSelect.mountAll(document.querySelector('#choice-form'));
    });
    const trigger=page.locator('#choice-choice');
    await page.locator('label[for=choice]').click();assert(await trigger.evaluate(el=>el===document.activeElement),'labels focus the accessible proxy');
    await page.keyboard.press('ArrowDown');await page.keyboard.press('ArrowDown');
    assert.equal(await page.locator('#choice').inputValue(),'local','arrow navigation only previews');
    assert.equal(await page.locator('.win-select-option[data-active=true]').innerText(),'Remote · 服务器','disabled options are skipped');
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('#choice').inputValue(),'local');
    assert.equal(await page.locator('.win-select-menu:popover-open').count(),0);
    await page.keyboard.press('r');await page.keyboard.press('Enter');
    assert.equal(await page.locator('#choice').inputValue(),'remote');
    assert.deepEqual(await page.evaluate(()=>choiceEvents),{input:1,change:1,submit:0},'commits emit exactly one input/change and never submit the form');
    await trigger.click();await page.keyboard.press('Home');await page.locator('#title').click();
    assert.equal(await page.locator('#choice').inputValue(),'remote','outside click cancels preview');
    await trigger.click();await page.keyboard.press('End');await page.keyboard.press('Tab');
    assert.equal(await page.locator('#choice').inputValue(),'dev');
    assert(await page.locator('#other-choice-choice').evaluate(el=>el===document.activeElement),'Tab commits then moves to the next control');
    await page.evaluate(()=>{choiceSource.value='local';});
    assert.match(await trigger.innerText(),/Local/,'programmatic rollback refreshes the visible label');
    assert.equal(await page.evaluate(()=>choiceEvents.change),2,'property updates do not emit another change');
    await trigger.click();
    await page.evaluate(()=>{const option=document.createElement('option');option.value='extra';option.textContent='Zebra · 新机器';choiceSource.append(option);});
    await page.keyboard.press('End');await page.keyboard.press('Space');assert.equal(await page.locator('#choice').inputValue(),'extra');
    assert.deepEqual(await page.evaluate(()=>[...new FormData(document.querySelector('#choice-form'))]),[['location','extra'],['other','a']]);
    await page.evaluate(()=>document.querySelector('#choice-form').reset());
    await page.waitForFunction(()=>document.querySelector('#choice-choice').textContent.includes('Local'));
    await page.evaluate(()=>document.querySelector('#choice-fieldset').disabled=true);
    await page.waitForFunction(()=>document.querySelector('#other-choice-choice').disabled);
    await page.evaluate(()=>{
      const fieldset=document.createElement('fieldset');fieldset.id='extra-fieldset';
      fieldset.innerHTML='<select id="extra-choice"><option value="one">One</option><option value="two" selected>Two</option></select>';
      document.querySelector('#choice-form').append(fieldset);
    });
    await page.waitForSelector('#extra-choice-choice');
    await page.evaluate(()=>{document.querySelector('#choice-fieldset').disabled=false;document.querySelector('#extra-fieldset').disabled=true;});
    await page.waitForFunction(()=>!document.querySelector('#other-choice-choice').disabled&&document.querySelector('#extra-choice-choice').disabled);
    for(const [property,value] of [['multiple',true],['size',4]]){
      await page.evaluate(([property,value])=>{document.querySelector('#extra-choice')[property]=value;},[property,value]);
      await page.waitForFunction(()=>!document.querySelector('#extra-choice-choice'));
      await page.evaluate(property=>{document.querySelector('#extra-choice')[property]=property==='multiple'?false:1;},property);
      await page.waitForSelector('#extra-choice-choice');
      assert.match(await page.locator('#extra-choice-choice').innerText(),/Two/,'restoring a single select remounts its proxy without changing its value');
    }
    await page.evaluate(()=>{choiceSource.value='';document.querySelector('#choice-form').reportValidity();});
    assert.equal(await trigger.getAttribute('aria-invalid'),'true');assert(await trigger.evaluate(el=>el===document.activeElement));
    await page.evaluate(()=>choiceSource.value='local');assert.equal(await trigger.getAttribute('aria-invalid'),'false');

    // Fast reversals preserve one listbox and its final value.
    for(let i=0;i<4;i++){await trigger.click();await page.keyboard.press('Escape');}
    await trigger.click();assert.equal(await page.locator('.win-select-menu:popover-open').count(),1);
    await page.setViewportSize({width:640,height:440});await settle(page);
    const bounds=await page.locator('.win-select-menu:popover-open').boundingBox();
    assert(bounds.x>=0&&bounds.y>=40&&bounds.x+bounds.width<=640&&bounds.y+bounds.height<=440);
    await page.keyboard.press('Escape');
    await page.emulateMedia({forcedColors:'active'});
    await page.waitForFunction(()=>!document.querySelector('.win-select'));
    assert.equal(await page.locator('#choice').inputValue(),'local');assert.equal(await page.locator('#choice').getAttribute('aria-hidden'),null);
    await page.locator('#choice').selectOption('remote');
    await page.emulateMedia({forcedColors:'none'});await page.waitForSelector('#choice-choice');assert.match(await trigger.innerText(),/Remote/);

    // A combobox inside the editor closes before its enclosing dialog. Pending
    // dashboard data must not replace an open standalone choice either.
    await page.setViewportSize({width:1294,height:840});
    await page.evaluate(()=>{state.page='settings';state.settingsTab='hosts';render();});await settle(page);
    assert.equal(await page.evaluate(()=>Object.hasOwn(choiceSource,'value')),false,'removed controls release property hooks and listeners');
    await page.locator('[data-edit=host1]').click();
    await page.locator('#field-authMode-choice').click();await page.keyboard.press('ArrowDown');await page.keyboard.press('Escape');
    assert(await page.locator('#editor').evaluate(el=>el.open));assert(await page.locator('#field-authMode-choice').evaluate(el=>el===document.activeElement));
    await page.locator('#cancel-editor').click();await settle(page);
    await page.evaluate(()=>AieyesApp.navigatePage('analysis'));await settle(page);
    await page.locator('#provider-choice').click();
    const retained=await page.evaluate(async()=>{const control=document.querySelector('#provider-choice');await loadDashboard();return {same:control===document.querySelector('#provider-choice'),pending:pendingRender};});
    assert.deepEqual(retained,{same:true,pending:true});
    await page.keyboard.press('Escape');await page.waitForFunction(()=>!pendingRender);

    // Reduction applies to in-flight JS and CSS animations, not just future ones.
    await page.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
    await page.emulateMedia({reducedMotion:'reduce'});
    await page.waitForFunction(()=>!document.querySelector('[data-win-animating]'));
    await page.evaluate(()=>{state.generalTab='desktop';renderSettings();});
    assert.equal(await page.evaluate(()=>document.getAnimations().filter(a=>a.id==='page').length),0);
    await page.emulateMedia({reducedMotion:'no-preference'});
    await page.evaluate(()=>{AieyesMotion.enter(document.querySelector('#settings-panel'));AieyesMotion.panel(false);});
    assert.equal(await page.locator('[data-win-animating]').count(),0);
    assert.deepEqual(errors,[]);
    await context.close();

    for(const platform of ['linux','windows-no-popover']){
      const fallback=await browser.newContext({viewport:{width:1040,height:800}});
      await fallback.addInitScript(fixture);
      await fallback.addInitScript(platform=>{if(platform==='linux')desktopInfo.platform='linux';else HTMLElement.prototype.showPopover=undefined;},platform);
      const view=await fallback.newPage();await view.goto(url);await view.evaluate(()=>AieyesApp.ready);await view.evaluate(()=>AieyesApp.navigatePage('analysis'));
      assert.equal(await view.locator('.win-select').count(),0,`${platform} retains native selects`);
      await view.locator('#provider').selectOption('codex');assert.equal(await view.locator('#provider').inputValue(),'codex');
      await fallback.close();
    }
    console.log('Windows glass interactions: native values, keyboard, rollback/reset, dynamic options, nested Escape, refresh retention, interrupted motion and native fallbacks passed');
  } finally {await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exit(1);});
