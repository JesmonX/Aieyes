// Layout acceptance shared by the Windows capture suite and Linux browser fixtures.
const assert=require('node:assert/strict');
const path=require('node:path');
const design=require('../docs/ui-design-tokens.json').dimensions;
const near=(actual,expected,label)=>assert(Math.abs(actual-expected)<1,`${label}: ${actual} vs ${expected}`);

async function verifyTrendLabels(page,platform,width) {
  // Observe actual glyph bounds so dense histories cannot clip or overlap date labels.
  const cases=await page.evaluate(()=>{
    const previous={dashboard:state.dashboard,cost:state.cost,trendDayIndex:state.trendDayIndex};
    const canvas=document.querySelector('#trend'),ctx=canvas.getContext('2d'),fillText=ctx.fillText,results=[];
    let labels=[];
    ctx.fillText=function(text,x,y,...args) {
      if(/^\d{2}-\d{2}$/.test(text)) {
        const glyphs=this.measureText(text);
        labels.push({text,left:x-glyphs.actualBoundingBoxLeft,right:x+glyphs.actualBoundingBoxRight});
      }
      return fillText.call(this,text,x,y,...args);
    };
    try {
      for(const cost of [false,true])for(const count of [0,1,7,100,365]) {
        const days=Array.from({length:count},(_,i)=>({...previous.dashboard.trendDays[0],key:new Date(Date.UTC(2026,0,i+1)).toISOString().slice(0,10)}));
        state.dashboard={...previous.dashboard,trendDays:days,dayModels:days.map(day=>({day:day.key,model:'test-model',usage:day}))};
        state.cost=cost;state.trendDayIndex=null;labels=[];drawTrend();
        results.push({count,cost,width:canvas.clientWidth,labels});
      }
    } finally {ctx.fillText=fillText;Object.assign(state,previous);drawTrend();}
    return results;
  });
  for(const test of cases) {
    const label=`${platform} ${width}px ${test.count} days ${test.cost?'USD':'Token'}`;
    assert(test.labels.length>=Math.min(test.count,2),`${label}: dates remain visible`);
    for(const [index,glyphs] of test.labels.entries()) {
      assert(glyphs.left>=0&&glyphs.right<=test.width,`${label}: ${glyphs.text} must fit the canvas (${glyphs.left}–${glyphs.right} of ${test.width})`);
      if(index)assert(glyphs.left>test.labels[index-1].right,`${label}: date labels must not overlap`);
    }
  }
}

module.exports=async function verifyAlignment({browser,url,data,installFixture,out}) {
  for(const platform of ['windows','linux']) {
    const context=await browser.newContext({viewport:{width:1294,height:840},reducedMotion:'reduce'});
    await context.addInitScript(installFixture,{...data,platform});
    const page=await context.newPage(),errors=[];
    page.on('pageerror',error=>errors.push(String(error)));
    const ready=async()=>{
      await page.waitForFunction(()=>window.AieyesApp&&state.dashboard&&!state.busy);
      await page.evaluate(async()=>{await AieyesTheme.loadFonts();await document.fonts.ready;});
    };
    const capture=async name=>{await page.mouse.move(0,0);return page.screenshot({path:path.join(out,`alignment-${platform}-${name}.png`),animations:'disabled'});};
    const checkDialog=async(selector,token,name)=>{
      const dialog=page.locator(selector);await dialog.waitFor({state:'visible'});
      await dialog.evaluate(el=>el.getAnimations().forEach(animation=>animation.finish()));
      const box=await dialog.boundingBox(),viewport=page.viewportSize();
      near(box.width,Math.min(design[token],viewport.width-32),`${platform} ${name} width`);
      assert(box.y>=(platform==='windows'?40:0)&&box.y+box.height<=viewport.height,`${name} stays in the client area`);
      assert(await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth+1),`${name} has no horizontal overflow`);
      const button=dialog.locator('.dialog-foot button:visible').last();
      if(await button.count()) {
        await button.scrollIntoViewIfNeeded();const target=await button.boundingBox();
        assert(target.y>=box.y&&target.y+target.height<=box.y+box.height,`${name} footer stays reachable`);
      }
      await capture(name);
      await page.keyboard.press('Escape');await dialog.waitFor({state:'hidden'});
    };
    try {
      await page.goto(url);await ready();
      for(const width of [640,900,1040,1294,1800]) {
        await page.setViewportSize({width,height:840});
        const geometry=await page.evaluate(()=>{
          const rect=el=>{const r=el.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,right:r.right};};
          const stats=document.querySelector('.stats'),cache=document.querySelector('.cache-card .token-breakdown');
          return {padding:getComputedStyle(document.querySelector('main')).paddingLeft,stats:rect(stats),cards:[...stats.children].map(rect),cache:rect(cache),cacheItems:[...cache.children].map(rect),overflow:document.documentElement.scrollWidth>innerWidth};
        });
        assert(!geometry.overflow,`${platform} ${width}: content must fit`);
        assert.equal(geometry.padding,`${design['content-padding']}px`);
        near(geometry.cards[0].x,geometry.stats.x,'first stat begins at the content edge');
        near(geometry.cards.at(-1).right,geometry.stats.right,'two stats fill the entire row');
        near(geometry.cards[0].width,geometry.cards[1].width,'stat widths match');
        if(geometry.cache.width>=geometry.cacheItems.length*120+(geometry.cacheItems.length-1)*12) {
          assert(geometry.cacheItems.every(item=>item.y===geometry.cacheItems[0].y),'cache metrics use one row when space permits');
          near(geometry.cacheItems.at(-1).right,geometry.cache.right,'cache metrics fill the available row');
        }
        await verifyTrendLabels(page,platform,width);
      }
      await page.setViewportSize({width:1294,height:840});
      await capture('overview');
      await page.locator('.chart-row').scrollIntoViewIfNeeded();
      await capture('charts');
      await page.locator('.heatmap').scrollIntoViewIfNeeded();
      await capture('heatmap');
      const rings=await page.evaluate(()=>{
        const previous=state.cost,probe=document.createElement('div');document.body.append(probe);
        const results=[];
        for(const cost of [false,true])for(const count of [0,1,8]) {
          state.cost=cost;
          const models=Array.from({length:count},(_,i)=>({key:`model-${i}`,total:(i+1)*100,cost:(i+1)/10}));
          probe.innerHTML=modelDistribution(models);
          const svg=probe.querySelector('svg'),paths=[...svg.querySelectorAll('path')],size=svg.viewBox.baseVal.width;
          results.push({cost,count,paths:paths.length,empty:svg.querySelectorAll('circle').length,
            finite:paths.every(p=>!/(NaN|Infinity|undefined)/.test(p.getAttribute('d'))&&p.getTotalLength()>0),
            openCenter:paths.every(p=>!p.isPointInFill(new DOMPoint(size/2,size/2))),
            ringFilled:count===1?paths[0].isPointInFill(new DOMPoint(size/2,4)):true});
        }
        state.cost=previous;probe.remove();return results;
      });
      for(const ring of rings) {
        assert.equal(ring.paths,Math.min(ring.count,6));assert.equal(ring.empty,ring.count?0:1);
        assert(ring.finite&&ring.openCenter&&ring.ringFilled,'model rings retain a hollow center and valid filled sectors');
      }
      for(const narrow of [false,true]) {
        await page.setViewportSize(narrow?{width:640,height:440}:{width:1294,height:840});
        const suffix=narrow?'-minimum':'';
        await page.evaluate(()=>{state.page='settings';state.settingsTab='hosts';render();editItem();});
        await checkDialog('#editor','dialog-host-width','host-editor'+suffix);
        await page.evaluate(()=>editPrice(__fixture.prices[0]));
        await checkDialog('#editor','dialog-price-width','price-editor'+suffix);
        await page.evaluate(()=>AieyesAgentSettings.advanced(state.settings.sources[0],'codex'));
        await checkDialog('#editor','dialog-source-width','source-editor'+suffix);
        await page.evaluate(()=>{state.page='settings';state.settingsTab='integrations';state.settingsAgent='codex';render();});
        await page.locator('[data-codex-api=add]').click();
        await checkDialog('#editor','dialog-api-width','api-editor'+suffix);
        await page.evaluate(()=>openPanelAccounts());
        await checkDialog('#quota-dialog','dialog-selection-width','account-picker'+suffix);
        await page.evaluate(()=>openQuotaOrder());
        await checkDialog('#quota-dialog','dialog-selection-width','account-order'+suffix);
        await page.evaluate(()=>openProviderEstimates('codex'));
        await checkDialog('#quota-dialog','dialog-valuation-width','valuation'+suffix);
        await page.evaluate(()=>AieyesAccountTargets.open('codex:codex'));
        await checkDialog('#account-targets','dialog-targets-width','account-targets'+suffix);
      }
      await page.goto(url+'/floating.html');await ready();
      for(const viewport of [{width:450,height:720},{width:360,height:480}]) {
        await page.setViewportSize(viewport);
        const panel=await page.evaluate(()=>{
          const rect=selector=>{const r=document.querySelector(selector).getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom};};
          return {tabs:rect('.panel-tabs'),head:rect('.panel-head'),body:rect('.panel-body'),foot:rect('.panel-foot'),stats:rect('.stats'),cards:[...document.querySelectorAll('.stats .stat')].map(el=>({height:el.getBoundingClientRect().height})),buttons:[...document.querySelectorAll('.panel-actions > button,.panel-more > summary')].map(el=>el.getBoundingClientRect().toJSON()),overflow:document.documentElement.scrollWidth>innerWidth};
        });
        assert(!panel.overflow,'compact panel fits its client area');
        near(panel.tabs.width,design['panel-tabs-width'],'visible segmented picker width');
        near(panel.tabs.height,design['control-compact-height'],'visible segmented picker height');
        near(panel.head.height,design['panel-head-icon-size']+2*design['panel-head-padding-y']+design['border-width'],'header height');
        near(panel.tabs.x+panel.tabs.width/2,viewport.width/2,'segmented picker remains centered');
        near(panel.head.bottom,panel.body.y,'panel body begins below fixed header');
        near(panel.body.bottom,panel.foot.y,'panel body ends above fixed footer');
        near(panel.foot.bottom,viewport.height,'footer remains at the window edge');
        near(panel.stats.x,design['panel-padding'],'compact content inset');
        near(panel.stats.width,viewport.width-2*design['panel-padding'],'compact stat row width');
        for(const card of panel.cards)near(card.height,design['panel-stat-height'],'compact stat card height');
        for(const [i,button] of panel.buttons.entries()) {
          near(button.width,design['icon-button-size'],'footer button size');
          if(i)near(button.x-panel.buttons[i-1].right,design['space-control'],'footer button gap');
        }
        await capture('panel-'+viewport.width);
        await page.locator('#panel-refresh').click();
        const menu=await page.locator('#panel-menu').boundingBox();
        assert(menu.x>=0&&menu.x+menu.width<=viewport.width,'refresh menu remains inside the panel');
        await page.keyboard.press('Escape');
        await page.evaluate(()=>openPanelAccounts());
        const picker=await page.locator('#quota-dialog').boundingBox();
        assert(picker.x>=0&&picker.y>=0&&picker.x+picker.width<=viewport.width&&picker.y+picker.height<=viewport.height,'panel picker fits the viewport');
        await capture('panel-picker-'+viewport.width);await page.keyboard.press('Escape');
      }
      assert.deepEqual(errors,[]);
    } finally {await context.close();}
  }
  console.log('UI alignment: Windows/Linux shared geometry, wide and narrow cards, unclipped trend labels, model ring sectors, eight native-sized dialogs, compact panel chrome and menus passed');
};
