// Control regressions use the same synthetic data as the Windows capture suite.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path');
const design=require('../docs/ui-design-tokens.json').dimensions;
const alpha=color=>color.startsWith('rgba')?Number(color.match(/,\s*([\d.]+)\)$/)[1]):1;

async function checkChoices(page) {
  const choices=await page.locator('label:has(> input[type=checkbox])').evaluateAll(labels=>labels.map(label=>{
    const input=label.querySelector('input'),box=input.getBoundingClientRect();
    if(!box.width||!box.height)return null;
    const walker=document.createTreeWalker(label,NodeFilter.SHOW_TEXT);
    let text;while((text=walker.nextNode())&&!text.textContent.trim()){}
    if(!text)return null;
    const start=text.textContent.search(/\S/),range=document.createRange();
    range.setStart(text,start);range.setEnd(text,start+1);
    const glyph=range.getBoundingClientRect();
    return {label:label.textContent.trim(),switch:input.getAttribute('role')==='switch',width:box.width,height:box.height,
      offset:glyph.y+glyph.height/2-box.y-box.height/2,gap:glyph.x-box.right};
  }).filter(Boolean));
  assert(choices.length,'the fixture contains visible choices');
  for(const choice of choices) {
    assert.equal(choice.width,design[choice.switch?'switch-width':'checkbox-size'],`${choice.label}: consistent width`);
    assert.equal(choice.height,design[choice.switch?'switch-height':'checkbox-size'],`${choice.label}: consistent height`);
    assert(Math.abs(choice.offset)<1.6,`${choice.label}: first text line aligns with the control (${choice.offset}px)`);
    assert(Math.abs(choice.gap-design['space-control'])<1,`${choice.label}: text has a consistent gap (${choice.gap}px)`);
  }
}

module.exports=async function verifyControls({browser,url,data,installFixture,out}) {
  const directory=path.join(out,'controls');fs.mkdirSync(directory,{recursive:true});
  for(const platform of ['windows','linux']) {
    const context=await browser.newContext({viewport:{width:1294,height:840},reducedMotion:'reduce'});
    await context.addInitScript(installFixture,{...data,platform});
    const page=await context.newPage(),errors=[];page.on('pageerror',error=>errors.push(String(error)));
    const session=await context.newCDPSession(page);
    try {
      await page.goto(url);await page.evaluate(()=>AieyesApp.ready);
      await page.evaluate(()=>AieyesTheme.loadFonts());
      const capture=async name=>{await page.evaluate(()=>document.fonts.ready);await page.screenshot({path:path.join(directory,`${platform}-${name}.png`),animations:'disabled'});};
      for(const theme of ['light','dark']) {
        for(const material of platform==='windows'?['opaque','acrylic','blur']:['opaque']) {
          await page.setViewportSize({width:1294,height:840});
          await page.evaluate(({theme,material})=>{
            state.settings.appearance={...state.settings.appearance,theme};
            state.page='settings';state.settingsTab='general';state.generalTab='appearance';render();
            __setDesktopMaterials(material,'opaque');window.scrollTo(0,0);
          },{theme,material});
          const appearance=await page.locator('[data-general-section=appearance] select').evaluateAll(selects=>selects.map(select=>{
            const control=select.closest('.win-select')??select;
            return {rect:control.getBoundingClientRect().toJSON(),row:select.closest('.form-row').getBoundingClientRect().toJSON()};
          }));
          assert.equal(appearance.length,4);
          for(const {rect,row} of appearance) {
            assert(rect.width>=Math.min(160,row.right-rect.x)&&rect.right<=row.right,'appearance choices have a readable width and fit their form');
            assert.equal(rect.x,appearance[0].rect.x,'selected text starts in the same column');
            assert.equal(rect.height,design['input-height']);
          }
          const surfaces=await page.evaluate(()=>({card:getComputedStyle(document.querySelector('#general-form')).backgroundColor,
            panel:getComputedStyle(document.documentElement).getPropertyValue('--panel').trim(),
            sidebar:getComputedStyle(document.querySelector('aside > nav')).backgroundColor}));
          assert.equal(alpha(surfaces.sidebar),0,'settings navigation shares the window backdrop');
          assert.equal(alpha(surfaces.card)<1,material!=='opaque','settings cards follow the material of their own window');
          if(material!=='opaque')assert.deepEqual(surfaces.card.match(/[\d.]+/g).map(Number),surfaces.panel.match(/[\d.]+/g).map(Number),'settings and floating cards share a surface');
          await capture(`appearance-${theme}-${material}`);
          if(material!=='opaque')continue;
          await page.setViewportSize({width:640,height:440});
          const minimum=await page.locator('[data-general-section=appearance] .form-row').evaluateAll(rows=>rows.map(row=>({
            row:row.getBoundingClientRect().toJSON(),label:row.querySelector('label').getBoundingClientRect().toJSON(),select:(row.querySelector('.win-select')??row.querySelector('select')).getBoundingClientRect().toJSON()})));
          for(const {row,label,select} of minimum) {
            assert(select.y>=label.bottom||select.x>=label.right,'minimum-window choices do not overlap labels');
            assert(select.x>=row.x&&select.right<=row.right,'minimum-window choices fit their form');
          }
          // The unified sidebar leaves more form width. Exercise the narrow
          // container breakpoint independently from the overall window width.
          await page.locator('.settings-subpage').evaluate(el=>el.style.maxWidth='400px');
          const narrow=await page.locator('[data-general-section=appearance] .form-row').evaluateAll(rows=>rows.map(row=>({
            row:row.getBoundingClientRect().toJSON(),label:row.querySelector('label').getBoundingClientRect().toJSON(),select:(row.querySelector('.win-select')??row.querySelector('select')).getBoundingClientRect().toJSON()})));
          for(const {row,label,select} of narrow) {
            assert(select.y>=label.bottom,'narrow appearance choices stack below their labels');
            assert(select.x>=row.x&&select.right<=row.right,'narrow choices fit their form');
          }
          assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'settings do not overflow the minimum window');
          await capture(`appearance-${theme}-minimum`);
          await page.locator('.settings-subpage').evaluate(el=>el.style.maxWidth='');
        }
        await page.setViewportSize({width:1294,height:840});
        await page.evaluate(()=>{__setDesktopMaterials('opaque','opaque');state.settingsTab='integrations';renderSettings(false);window.scrollTo(0,0);});
        await checkChoices(page);await capture(`agents-${theme}`);
        await page.evaluate(()=>{
          // Use real control markup without application save handlers to inspect every visual state.
          document.querySelector('#settings-panel').innerHTML=`<form id="control-states" class="card" style="max-width:540px;display:grid;gap:14px">
            <label><input id="control-choice" name="choice" type="checkbox">显示并查询额度</label>
            <label><input name="checked" type="checkbox" checked>Codex / Claude Code · 已选择</label>
            <label><input id="control-mixed" name="mixed" type="checkbox">部分选择</label>
            <label><input name="disabled" type="checkbox" disabled>暂不可用</label>
            <label><input name="disabled-checked" type="checkbox" checked disabled>已选择 · 暂不可用</label>
            <label><input id="control-wrap" type="checkbox"><span>我确认所有设备的用量均已纳入所选来源；包含中文说明、English text 和需要换行的较长选项，以检查首行与复选框的对齐。</span></label>
            <label><input name="switch" type="checkbox" role="switch">资源监控</label>
            <label><input type="checkbox" role="switch" checked>Agent 采集</label>
            <label><input type="checkbox" role="switch" checked disabled>已开启 · 暂不可用</label>
            <label>执行位置<select name="location" style="width:100%"><option value="local">本机 · Codex / Claude Code</option><option value="remote">训练服务器 · SSH</option></select></label>
            <div id="control-multi"></div><div class="actions"><button type="button" class="primary">保存设置</button><button type="button">取消</button><button type="button" disabled>不可用操作</button></div></form>`;
          document.querySelector('#control-mixed').indeterminate=true;
          const selected=new Set(['cpu']);
          window.controlPreview=AieyesSelect.mount(document.querySelector('#control-multi'),{
            title:'监控设备与显示内容',options:()=>[{id:'cpu',label:'CPU · 处理器'},{id:'network',label:'网络 · Ethernet'}],
            selected:()=>[...selected],onChange:values=>{selected.clear();values.forEach(value=>selected.add(value));}
          });
          window.scrollTo(0,0);
        });
        await checkChoices(page);
        const choice=page.locator('#control-choice');await choice.focus();await page.keyboard.press('Space');
        assert(await choice.isChecked(),'the custom checkbox retains Space activation');
        assert.equal(await choice.evaluate(el=>el.matches(':focus-visible')),true,'keyboard focus stays visible');
        await page.locator('[name=switch]').focus();await page.keyboard.press('Space');
        assert(await page.locator('[name=switch]').isChecked(),'switches retain Space activation');
        assert.deepEqual(await page.evaluate(()=>[...new FormData(document.querySelector('#control-states')).keys()]),['choice','checked','switch','location'],'native form values retain checked and disabled semantics');
        await capture(`states-${theme}`);
        for(const scale of [1.25,1.5,2]) {
          await session.send('Emulation.setDeviceMetricsOverride',{width:1294,height:840,deviceScaleFactor:scale,mobile:false});
          await checkChoices(page);await capture(`states-${theme}-scale-${scale}`);
        }
        await session.send('Emulation.clearDeviceMetricsOverride');
        await page.setViewportSize({width:1294,height:720});
        await page.locator('#control-multi .multi-trigger').click();
        await page.locator('.multi-panel:visible input[value=network]').focus();await page.keyboard.press('Space');
        const triggerBox=await page.locator('#control-multi .multi-trigger').boundingBox();
        const shortList=await page.locator('.multi-panel:visible').boundingBox();
        assert(Math.min(Math.abs(shortList.y-triggerBox.y-triggerBox.height-4),Math.abs(triggerBox.y-shortList.y-shortList.height-4))<1,'short multiselect lists stay next to their trigger');
        await checkChoices(page);await capture(`multiselect-${theme}`);
        await page.locator('.multi-panel:visible input[type=search]').fill('网络');
        const filtered=await page.locator('.multi-panel:visible').boundingBox();
        assert(filtered.height<shortList.height,'filtering shrinks the short option list');
        assert(Math.min(Math.abs(filtered.y-triggerBox.y-triggerBox.height-4),Math.abs(triggerBox.y-filtered.y-filtered.height-4))<1,'filtered lists remain attached to the trigger');
        await capture(`multiselect-${theme}-filtered`);
        await page.locator('.multi-panel:visible input[type=search]').fill('');
        await page.setViewportSize({width:640,height:440});
        await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
        const popup=await page.locator('.multi-panel:visible').boundingBox();
        assert(popup.x>=0&&popup.y>=(platform==='windows'?40:0)&&popup.x+popup.width<=640&&popup.y+popup.height<=440,'an open multiselect stays within the client area after resizing');
        await capture(`multiselect-${theme}-resized`);
        await page.keyboard.press('Escape');
        assert(await page.locator('#control-multi .multi-trigger').evaluate(el=>el===document.activeElement),'closing a multiselect restores focus');
        await page.setViewportSize({width:1294,height:840});
        await page.emulateMedia({colorScheme:theme,forcedColors:'active'});
        await choice.focus();await page.keyboard.press('Space');assert(!await choice.isChecked(),'high contrast retains keyboard activation');
        const selectedColors=await page.locator('aside nav button.active,.settings-tabs button.active,.primary').evaluateAll(elements=>elements.map(el=>getComputedStyle(el).color));
        assert.equal(new Set(selectedColors).size,1,'selected navigation and primary controls use the same readable system foreground');
        await checkChoices(page);await capture(`high-contrast-${theme}`);
        await page.emulateMedia({forcedColors:'none'});
        await page.evaluate(()=>controlPreview.destroy());
      }
      await page.evaluate(()=>{state.settingsTab='general';state.generalTab='appearance';renderSettings(false);__setDesktopMaterials('acrylic','opaque');});
      await session.send('Emulation.setEmulatedMedia',{features:[{name:'prefers-reduced-transparency',value:'reduce'}]});
      assert.equal(alpha(await page.locator('#general-form').evaluate(el=>getComputedStyle(el).backgroundColor)),1,'reduced transparency makes settings cards opaque');
      await session.send('Emulation.setEmulatedMedia',{features:[]});
      assert.deepEqual(errors,[]);
    } finally {await context.close();}
  }
  console.log('Control appearance: shared settings surfaces, narrow select columns, checkbox/label alignment, native form semantics, keyboard selection and high contrast passed');
};
