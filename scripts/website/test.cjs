const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const browsers=require('../../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../..'),site=path.join(root,'dist/website'),qa=path.join(root,'.local/website/qa');
// The two policies of scripts/website/Caddyfile: the site's, then the live demo's.
const policies=[...fs.readFileSync(path.join(__dirname,'Caddyfile'),'utf8').matchAll(/Content-Security-Policy "([^"]+)"/g)].map(match=>match[1]);
assert(policies.length===2&&!policies[0].includes('unsafe-inline')&&policies[1].includes("frame-ancestors 'self'"),'Caddyfile policies');
const mime={'.jpg':'image/jpeg','.html':'text/html; charset=utf-8','.js':'text/javascript','.css':'text/css','.webp':'image/webp','.png':'image/png','.xml':'application/xml','.txt':'text/plain','.woff2':'font/woff2','.json':'application/json'};
async function checkMobile(browser,origin,errors,failed){
  for(const [language,url] of [['en','/'],['zh-CN','/zh-CN/index.html']]){
    for(const [width,height] of [[320,700],[360,800],[390,844],[430,932],[768,1024],[844,390]]){
      const context=await browser.newContext({viewport:{width,height},isMobile:true,hasTouch:true,deviceScaleFactor:2,reducedMotion:'reduce'});
      try{
        const page=await context.newPage(),label=`${language} ${width}×${height}`;
        page.on('pageerror',error=>errors.push(String(error)));
        page.on('response',response=>{if(response.status()>=400)failed.push(response.url());});
        await page.goto(origin+url);await page.waitForSelector('.showcase.is-live');
        await page.waitForFunction(()=>document.querySelector('.stage-image').complete&&document.querySelector('.stage-image').naturalWidth>0);
        assert.equal(await page.locator('.hero-stage').getAttribute('data-surface'),width<=700?'panel':'main',`${label}: phones start on the panel`);
        const checkLayout=async()=>{
          const layout=await page.evaluate(()=>{
            const visible=el=>el.getClientRects().length&&getComputedStyle(el).visibility!=='hidden';
            const box=el=>el.getBoundingClientRect();
            const within=(inner,outer)=>inner.left>=outer.left-1&&inner.right<=outer.right+1&&inner.top>=outer.top-1&&inner.bottom<=outer.bottom+1;
            const overlap=(a,b)=>a.left<b.right&&b.left<a.right&&a.top<b.bottom&&b.top<a.bottom;
            const controls=[...document.querySelectorAll('.brand,.menu-toggle,.header nav a,.platform-switch button,.surface-switch button,.tour-step,.button,.package-button,[data-feature-tab],.dialog-close,.release-link,.faq summary,.footer-links a,.footer-bottom a')].filter(visible);
            const small=controls.filter(el=>box(el).width<43.9||box(el).height<43.9).map(el=>el.outerHTML.slice(0,160));
            const overflowing=[...document.querySelectorAll('.wrap,.hero-pet,.hero-pet .animated-pet,.stage-tag,.stage-window,.tour,.preview-toolbar,.feature-copy,.type-art,.pet-line,.pet-line em,.pet-line .animated-pet,.download-card,.footer-links')].filter(visible).filter(el=>box(el).left< -1||box(el).right>innerWidth+1).map(el=>el.className);
            const clippedPets=[...document.querySelectorAll('.animated-pet')].filter(visible).filter(el=>{const container=el.closest('.type-art');return container&&!within(box(el),box(container));});
            const coveredPets=[...document.querySelectorAll('.pet-line')].filter(visible).filter(el=>overlap(box(el.querySelector('em')),box(el.querySelector('.animated-pet'))));
            return {width:document.documentElement.scrollWidth,viewport:innerWidth,small,overflowing,clippedPets:clippedPets.length,coveredPets:coveredPets.length,heroCovered:overlap(box(document.querySelector('.hero-pet')),box(document.querySelector('.stage-tag')))};
          });
          assert(layout.width<=layout.viewport+1,`${label}: page fits the viewport`);
          assert.deepEqual(layout.overflowing,[],`${label}: content stays inside the viewport`);
          assert.deepEqual(layout.small,[],`${label}: controls have at least a 44px touch target`);
          assert.equal(layout.clippedPets+layout.coveredPets,0,`${label}: pet sprites fit without covering text`);
          assert.equal(layout.heroCovered,false,`${label}: the screenshot caption does not cover the pet`);
        };
        await checkLayout();
        if(width===320||width===390)await page.screenshot({path:path.join(qa,`${language}-mobile-${width}-hero.png`)});
        await page.locator('.hero-pet').scrollIntoViewIfNeeded();
        if(width===320)await page.screenshot({path:path.join(qa,`${language}-mobile-pet.png`)});
        await page.locator('.menu-toggle').tap();
        assert.equal(await page.locator('.menu-toggle').getAttribute('aria-expanded'),'true');
        const menu=await page.locator('#navigation').boundingBox();
        assert(menu.y>=0&&menu.y+menu.height<=height+1,`${label}: navigation fits a short screen`);
        await checkLayout();
        await page.locator('#navigation a[href="#overview"]').tap();
        assert.equal(await page.locator('.menu-toggle').getAttribute('aria-expanded'),'false');
        assert.equal(await page.locator('.workspace-tabs').getAttribute('aria-orientation'),'horizontal');
        const checkSelectedTab=async()=>{
          const visible=await page.locator('[data-feature-tab][aria-selected=true]').evaluate(el=>{const a=el.getBoundingClientRect(),b=el.closest('[role=tablist]').getBoundingClientRect();return a.left>=b.left-1&&a.right<=b.right+1;});
          assert(visible,`${label}: the selected feature tab stays in view`);
        };
        for(const id of ['overview','analytics','accounts','sessions','servers','wakeups']){
          await page.locator(`[data-feature-tab="${id}"]`).tap();
          assert.equal(await page.locator('[data-feature-panel]:visible').count(),1);
          assert(await page.locator(`#${id}`).isVisible());
          await checkSelectedTab();await checkLayout();
          if(width===320&&id==='sessions')await page.locator('#sessions').screenshot({path:path.join(qa,`${language}-mobile-sessions.png`)});
        }
        await page.locator('[data-feature-tab="wakeups"]').focus();
        await page.keyboard.press('Home');await checkSelectedTab();
        await page.keyboard.press('End');await checkSelectedTab();
        assert.equal(await page.locator('button[data-platform=windows]').getAttribute('aria-pressed'),'true');
        await page.waitForFunction(()=>document.querySelector('.stage-frame:not([hidden])')&&document.querySelector('.stage-status').hidden,null,{timeout:30000});
        assert.equal(await page.locator('.hero-stage').getAttribute('data-mode'),'live');
        await checkLayout();
        await page.locator('[data-tour-step="1"]').tap();await checkLayout();
        await page.locator('button[data-platform=macos]').tap();
        assert.equal(await page.locator('.hero-stage').getAttribute('data-mode'),'native');await checkLayout();
        await page.locator('[data-feature-tab="overview"]').tap();
        await page.waitForFunction(()=>{const image=document.querySelector('#overview .shot img');return image.complete&&image.naturalWidth>0;});
        await page.locator('#overview .shot').tap();
        const dialog=await page.locator('#image-dialog').evaluate(el=>({open:el.open,scroll:el.scrollHeight,height:el.clientHeight,box:el.getBoundingClientRect().toJSON(),close:el.querySelector('button').getBoundingClientRect().toJSON()}));
        assert(dialog.open&&dialog.box.y>=0&&dialog.box.bottom<=height+1,`${label}: the image dialog fits`);
        assert(dialog.scroll<=dialog.height+1,`${label}: the dialog image and caption fit without clipping`);
        assert(dialog.close.width>=44&&dialog.close.height>=44&&dialog.close.bottom<=height,`${label}: close remains easy to reach`);
        await page.locator('.dialog-close').tap();
        assert.equal(await page.locator('#image-dialog').evaluate(el=>el.open),false);
        await page.locator('.faq summary').first().tap();
        assert(await page.locator('.faq details').first().getAttribute('open')!==null);
        await page.goto(origin+url+'#wakeups');await checkSelectedTab();
        assert(await page.locator('#wakeups').isVisible());
        console.log(`Mobile passed: ${label}, touch controls, stage, live frame, six features, visible tabs, pets, zoom, FAQ.`);
      }finally{await context.close();}
    }
  }
}

async function checkFontsAndAnimation(browser,origin){
  const contracts=JSON.parse(fs.readFileSync(path.join(root,'docs/core-contracts.json'),'utf8'));
  const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'no-preference'});
  try{
    const fontRequests=[];page.on('request',request=>{if(request.url().includes('/fonts/'))fontRequests.push(new URL(request.url()).pathname);});
    await page.goto(origin+'/#sessions');await page.evaluate(()=>document.fonts.ready);
    assert(!fontRequests.includes('/fonts/lxgw-wenkai-regular.woff2'),'English pages use the small LXGW face only: '+fontRequests.join(', '));
    const fonts=await page.evaluate(async()=>{await Promise.all([document.fonts.load('18px "Comic Neue"','Aieyes'),document.fonts.load('700 18px "Comic Neue"','Aieyes'),document.fonts.load('18px "LXGW WenKai"','工作账户')]);return {comic:document.fonts.check('18px "Comic Neue"','Aieyes'),bold:document.fonts.check('700 18px "Comic Neue"','Aieyes'),chinese:document.fonts.check('18px "LXGW WenKai"','工作账户'),family:getComputedStyle(document.body).fontFamily,body:getComputedStyle(document.body).fontSize,weights:[...new Set([...document.querySelectorAll('body *')].map(el=>getComputedStyle(el).fontWeight))].sort()};});
    // The shipped webfont leads so every platform renders the same Regular/Bold pair.
    assert(fonts.comic&&fonts.bold&&fonts.chinese);assert.equal(fonts.family.split(',')[0].trim().replace(/^["']|["']$/g,''),'Comic Neue');assert.equal(fonts.body,'18px');
    assert.deepEqual(fonts.weights.filter(weight=>!['400','700'].includes(weight)),[],'only shipped font weights are used');
    assert.equal(await page.locator('.provider',{hasText:'ChatGPT Work'}).count(),0);
    assert.equal(await page.locator('.provider',{hasText:'DeepSeek'}).count(),1);
    for(const name of ['Qoder CN','Trae CN'])assert.equal(await page.locator('.provider',{hasText:name}).count(),1);
    for(const provider of contracts.providers.filter(p=>p.icon&&p.id!=='custom'&&!['chatgpt-work','deepseek-harness'].includes(p.id))){
      const asset=provider.icon.darkAsset||provider.icon.asset;
      assert(await page.locator(`.provider img[src="/providers/provider-${asset}.webp"]`).count()>0);
      if(provider.icon.monochrome)assert.equal(await page.locator(`.provider img[src="/providers/provider-${asset}.webp"]`).first().evaluate(el=>getComputedStyle(el).filter),'brightness(0) invert(1)');
    }
    const frames=[];
    // The website sheet keeps the idle, working and complete rows of the shared contract.
    for(const [sheetRow,[state,row]] of [['idle',0],['working',1],['complete',4]].entries()){
      const actual=await page.locator(`#sessions .pet-${state}`).evaluate(async(el,{sheetRow,durations})=>{
        const animation=el.getAnimations().find(a=>a.effect.target===el);animation.pause();await animation.ready;
        const timing=animation.effect.getTiming(),size=parseFloat(getComputedStyle(el).getPropertyValue('--pet-size'));
        const sprite=new Image();sprite.src='/pet-spritesheet.webp';await sprite.decode();
        const canvas=document.createElement('canvas');canvas.width=sprite.width/8;canvas.height=sprite.height/3;const context=canvas.getContext('2d');
        let elapsed=0;const frames=[];
        for(const [frame,duration] of durations.entries()){
          // Sample a complete later iteration so staggered negative delays do
          // not put the animation before its active interval.
          animation.currentTime=timing.delay+timing.duration+elapsed+duration/2;
          context.clearRect(0,0,canvas.width,canvas.height);context.drawImage(sprite,frame*canvas.width,sheetRow*canvas.height,canvas.width,canvas.height,0,0,canvas.width,canvas.height);
          const rgba=context.getImageData(0,0,canvas.width,canvas.height).data;let visible=0;for(let i=3;i<rgba.length;i+=4)if(rgba[i]>16)visible++;
          frames.push({frame,x:parseFloat(getComputedStyle(el).backgroundPositionX),expected:-frame*size,visible});elapsed+=duration;
        }
        return {duration:timing.duration,frames};
      },{sheetRow,durations:contracts.petFrameDurations[row]});
      assert.equal(actual.duration,contracts.petFrameDurations[row].reduce((a,b)=>a+b,0));
      for(const frame of actual.frames){assert(Math.abs(frame.x-frame.expected)<.1,JSON.stringify(frame));assert(frame.visible>100,`${state} frame ${frame.frame} is not blank`);}
      frames.push({state,...actual});
    }
    fs.writeFileSync(path.join(qa,`animation-${process.env.AIEYES_WEBSITE_BROWSER||'chromium'}.json`),JSON.stringify({fonts,frames},null,2));
  }finally{await page.close();}
}
// The stage: the live frontend for Windows, shown first, and native macOS captures of the same places.
async function checkStage(browser,origin,errors,failed){
  for(const [language,url] of [['en','/'],['zh-CN','/zh-CN/index.html']]){
    const data=JSON.parse(fs.readFileSync(path.join(site,`demo/stage.${language}.json`),'utf8'));
    const main=data.surfaces.main,panel=data.surfaces.panel;
    // Every state has its picture, and none carries click targets: macOS is a capture.
    for(const group of Object.values(data.surfaces))for(const [name,state] of Object.entries(group.states)){
      assert(fs.statSync(path.join(site,'demo/macos',language,name+'.webp')).size>1000,`${language}: ${name}.webp`);
      assert.equal(state.spots,undefined,`${language} ${name}: no click targets`);
    }
    for(const [src] of Object.values(data.poster))assert(fs.statSync(path.join(site,new URL(src,origin).pathname)).size>1000,src);
    if(language==='en')assert(!/[㐀-鿿]/.test(JSON.stringify(data)),'the English stage has no Chinese text');
    const context=await browser.newContext({viewport:{width:1440,height:1100},reducedMotion:'reduce'});
    try{
      const page=await context.newPage(),label=`${language} stage`;
      page.on('pageerror',error=>errors.push(String(error)));page.on('console',message=>{if(message.type()==='error')errors.push(message.text());});
      page.on('response',response=>{if(response.status()>=400)failed.push(response.url());});
      await page.goto(origin+url);await page.waitForSelector('.showcase.is-live');
      const view=()=>page.evaluate(()=>({mode:document.querySelector('.hero-stage').dataset.mode,surface:document.querySelector('.hero-stage').dataset.surface,image:new URL(document.querySelector('.stage-image').src).pathname,count:document.querySelector('.tour-count').textContent,stop:document.querySelector('.tour-title strong').textContent,note:document.querySelector('.stage-note').textContent,frames:document.querySelectorAll('.stage-frame:not([hidden])').length}));
      const title=id=>main.stops.find(stop=>stop.id===id).title;
      const live=()=>page.waitForFunction(()=>document.querySelector('.stage-frame.is-ready:not([hidden])')&&document.querySelector('.stage-status').hidden&&document.querySelector('.stage-image').hidden,null,{timeout:30000});
      const reaches=check=>page.waitForFunction(check,null,{timeout:30000});
      const shows=async(surface,name,step)=>{
        const group=data.surfaces[surface],state=group.states[name],index=group.stops.findIndex(stop=>stop.id===state.stop);
        await page.waitForFunction(pathname=>new URL(document.querySelector('.stage-image').src).pathname===pathname&&!document.querySelector('.stage-image').hidden,`/demo/macos/${language}/${name}.webp`);
        assert.deepEqual(await view(),{mode:'native',surface,image:`/demo/macos/${language}/${name}.webp`,count:`${String(index+1).padStart(2,'0')} / ${String(group.stops.length).padStart(2,'0')}`,stop:group.stops[index].title,note:data.text.native,frames:0},`${label}: ${step}`);
      };
      // Windows comes first: the frontend itself, in a frame.
      assert.deepEqual(await page.locator('.platform-switch button').evaluateAll(buttons=>buttons.map(button=>[button.dataset.platform,button.getAttribute('aria-pressed')])),[['windows','true'],['macos','false']],`${label}: Windows is the default`);
      assert.equal(await page.locator('html').getAttribute('data-platform'),'windows');
      await live();
      const frame=page.frameLocator('.stage-frame:not([hidden])'),inside=()=>page.locator('.stage-frame:not([hidden])').evaluate(element=>element.contentWindow.AieyesDemo.view());
      assert.deepEqual(await view(),{mode:'live',surface:'main',image:new URL(data.poster.main[0],origin).pathname,count:`01 / ${String(main.stops.length).padStart(2,'0')}`,stop:main.stops[0].title,note:data.text.live,frames:1},`${label}: first view`);
      assert.equal(await page.locator('.stage-spot').count(),0);
      assert.equal(await frame.locator('html').getAttribute('lang'),language);
      assert.equal((await inside()).page,'agent');
      await page.locator('#showcase').screenshot({path:path.join(qa,`${language}-stage-windows.png`)});
      // The tour: buttons and arrow keys, wrapping at both ends.
      await page.locator('[data-tour-step="1"]').click();await page.locator('[data-tour-step="1"]').click();
      await reaches(()=>document.querySelector('.stage-frame:not([hidden])').contentWindow.AieyesDemo.view().page==='analysis');
      assert.equal((await view()).stop,title('analytics'));
      await page.keyboard.press('ArrowLeft');await page.keyboard.press('ArrowLeft');
      await reaches(()=>document.querySelector('.stage-frame:not([hidden])').contentWindow.AieyesDemo.view().page==='agent');
      assert.equal((await view()).stop,main.stops[0].title);
      await page.locator('[data-tour-step="-1"]').click();
      await reaches(()=>document.querySelector('.stage-frame:not([hidden])').contentWindow.AieyesDemo.view().page==='setup');
      assert.equal((await view()).stop,main.stops.at(-1).title);
      // Moving inside the frame moves the tour.
      await frame.locator('aside nav button[data-page="analysis"]').click();
      await page.waitForFunction(title=>document.querySelector('.tour-title strong').textContent===title,title('analytics'));
      assert.equal((await inside()).page,'analysis');
      // The frame has the window's real size and is scaled to fit the stage.
      const fitted=await page.evaluate(()=>{const frame=document.querySelector('.stage-frame:not([hidden])').getBoundingClientRect(),screen=document.querySelector('.stage-screen').getBoundingClientRect();return Math.abs(frame.width-screen.width)<2&&Math.abs(frame.height-screen.height)<2;});
      assert(fitted,`${label}: the live window fills the stage`);
      await page.locator('[data-surface-tab=panel]').click();
      await reaches(()=>document.querySelector('.stage-frame.is-ready:not([hidden])')?.contentWindow.AieyesDemo?.panel&&document.querySelector('.stage-status').hidden);
      assert.equal(await page.locator('[data-surface-tab=panel]').getAttribute('aria-pressed'),'true');
      await page.locator('#showcase').screenshot({path:path.join(qa,`${language}-stage-windows-panel.png`)});
      // macOS: a capture of the place the visitor reached on Windows, with nothing to click in it.
      await page.locator('[data-surface-tab=main]').click();await page.locator('button[data-platform=macos]').click();await shows('main','analytics','platforms share one place');
      assert.equal(await page.locator('.stage-screen button').count(),0,`${label}: the capture has no click targets`);
      await page.locator('#showcase').screenshot({path:path.join(qa,`${language}-stage-macos.png`)});
      await page.locator('[data-tour-step="1"]').click();await shows('main',main.stops[3].state,'the tour steps through captures');
      await page.locator('[data-surface-tab=panel]').click();await shows('panel',panel.initial,'panel capture');
      await page.locator('#showcase').screenshot({path:path.join(qa,`${language}-stage-panel.png`)});
      await page.locator('[data-surface-tab=main]').click();await shows('main',main.stops[3].state,'each window keeps its place');
      // The choice lasts for the visit, and Windows picks up where macOS left.
      await page.reload();await page.waitForSelector('.showcase.is-live');
      assert.equal(await page.locator('button[data-platform=macos]').getAttribute('aria-pressed'),'true');await shows('main',main.initial,'macOS is remembered');
      await page.locator('[data-tour-step="1"]').click();await page.locator('[data-tour-step="1"]').click();await shows('main','analytics','tour on macOS');
      await page.locator('button[data-platform=windows]').click();await live();
      await reaches(()=>document.querySelector('.stage-frame:not([hidden])').contentWindow.AieyesDemo.view().page==='analysis');
      assert.equal((await view()).mode,'live');
      assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),true);
      console.log(`Stage passed: ${language}, live Windows frame first, tour, panel, macOS captures.`);
    }finally{await context.close();}
  }
}
async function checkNoScript(browser,origin){
  const context=await browser.newContext({javaScriptEnabled:false});
  try{const page=await context.newPage();for(const [language,url] of [['en','/'],['zh-CN','/zh-CN/index.html']]){
    await page.goto(origin+url);assert.equal(await page.locator('.package-button').count(),6);
    assert(await page.locator('.hero-actions a[href="#download"]').isVisible());
    const preview=new URL(await page.locator('.stage-image').getAttribute('src'),origin);
    assert.equal(preview.pathname,`/screenshots/${language}/windows-overview.webp`);
    assert(fs.statSync(path.join(site,preview.pathname)).size>1000);
    assert.equal(await page.locator('.stage-spot').count(),0);
    assert.match(preview.searchParams.get('v'),/^[a-f0-9]{12}$/);
    for(const platform of ['macos','windows'])for(const feature of ['overview','analytics','servers','panel']){
      const file=path.join(site,'screenshots',language,`${platform}-${feature}.webp`);assert(fs.statSync(file).size>1000,file);
      if(language==='en')assert(fs.readFileSync(file).equals(fs.readFileSync(path.join(site,'screenshots',`${platform}-${feature}.webp`))));
    }
  }}finally{await context.close();}
}

for(const name of ['README.md','README.zh-CN.md','docs/development.md']){
  const markdown=fs.readFileSync(path.join(root,name),'utf8');
  const targets=[...markdown.matchAll(/(?:\]\(|src=")([^\s)"#]+)(?:#[^\s)"]*)?[)"]/g)].map(m=>m[1]).filter(t=>!/^https?:|^mailto:/.test(t));
  for(const target of targets)assert(fs.existsSync(path.resolve(root,path.dirname(name),target)),`${name}: missing ${target}`);
}
(async()=>{
  fs.mkdirSync(qa,{recursive:true});
  const server=http.createServer((req,res)=>{
    let file=path.resolve(site,'.'+decodeURIComponent(new URL(req.url,'http://localhost').pathname));
    if(fs.existsSync(file)&&fs.statSync(file).isDirectory())file=path.join(file,'index.html');
    if(!file.startsWith(site+path.sep)||!fs.existsSync(file)){res.writeHead(404,{'Content-Type':'text/html'});return res.end(fs.readFileSync(path.join(site,'404.html')));}
    const app=path.relative(site,file).split(path.sep).slice(0,2).join('/')==='demo/app';
    res.setHeader('Content-Security-Policy',policies[app?1:0]);res.setHeader('X-Frame-Options',app?'SAMEORIGIN':'DENY');
    res.setHeader('Content-Type',mime[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browserType=browsers[process.env.AIEYES_WEBSITE_BROWSER||'chromium'];
  assert(browserType?.launch,'AIEYES_WEBSITE_BROWSER must name a Playwright browser');
  const browser=await browserType.launch(),origin=(process.env.AIEYES_WEBSITE_ORIGIN||`http://127.0.0.1:${server.address().port}`).replace(/\/$/,'');
  const errors=[],failed=[];
  try{
    const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce'});
    page.on('pageerror',error=>errors.push(String(error)));page.on('response',response=>{if(response.status()>=400)failed.push(response.url());});
    for(const [language,url] of [['en','/'],['zh-CN','/zh-CN/index.html']]){
      await page.goto(origin+url);await page.evaluate(()=>document.fonts.ready);assert.equal(await page.locator('html').getAttribute('lang'),language);
      assert.equal(await page.locator('h1').count(),1);assert.equal(await page.locator('.feature').count(),6);
      await page.evaluate(()=>document.querySelectorAll('img').forEach(image=>image.loading='eager'));
      await page.waitForFunction(()=>[...document.images].filter(image=>image.getAttribute('src')).every(image=>image.complete&&image.naturalWidth>0));
      assert.equal(await page.locator('link[rel=alternate][hreflang]').count(),3);
      const share=new URL(await page.locator('meta[property="og:image"]').getAttribute('content'));
      assert(fs.existsSync(path.join(site,share.pathname))&&share.pathname===(language==='en'?'/og.jpg':'/og-zh-CN.jpg'),`${language}: sharing image`);
      assert.equal(await page.locator('meta[property="og:image:width"]').getAttribute('content'),'1200');
      assert(await page.locator('meta[property="og:image:alt"]').getAttribute('content'));
      assert.equal(await page.locator('.downloads .package-button').count(),6);
      assert(await page.locator('a[data-language]').getAttribute('href'));
      assert.equal(await page.locator('a[data-track="github"]').first().getAttribute('href'),'https://aieyes.2h3ngy4n.space/go/github');
      for(const route of ['macos/arm64/dmg','macos/x64/dmg','windows/x64/exe','linux/x64/appimage','linux/x64/deb','linux/x64/rpm'])assert.equal(await page.locator(`a[data-track="download-${route.replaceAll('/','-')}"]`).getAttribute('href'),`https://aieyes.2h3ngy4n.space/go/download/${route}`);
      assert.equal(await page.locator('a[data-track="download-latest"]').getAttribute('href'),'https://aieyes.2h3ngy4n.space/go/download/latest');
      await page.screenshot({path:path.join(qa,`${language}-desktop.png`)});
      for(const [name,selector] of [['providers','.ecosystem'],['downloads','.download']]){
        await page.locator(selector).evaluate(el=>el.scrollIntoView({block:'center'}));
        await page.locator(selector).screenshot({path:path.join(qa,`${language}-${name}.png`)});
      }
      for(const id of ['overview','analytics','accounts','sessions','servers','wakeups']){
        await page.locator(`[data-feature-tab="${id}"]`).click();
        assert.equal(await page.locator('[data-feature-panel]:visible').count(),1);
        assert.equal(await page.locator(`#${id}`).isVisible(),true);
        assert.equal(await page.locator(`[data-feature-tab="${id}"]`).getAttribute('tabindex'),'0');
        if(['accounts','sessions','wakeups'].includes(id))assert.equal(await page.locator(`#${id} img[data-preview]`).count(),0);
        await page.locator(`#${id}`).screenshot({path:path.join(qa,`${language}-${id}.png`)});
      }
      await page.locator('[data-feature-tab="wakeups"]').focus();await page.keyboard.press('Home');
      assert.equal(await page.locator('[data-feature-tab="overview"]').evaluate(el=>el===document.activeElement),true);
      assert.equal(await page.locator('.animated-pet').first().evaluate(el=>getComputedStyle(el).animationName),'none');

      const previewVersion=new URL(await page.locator('.feature img[data-preview]').first().getAttribute('src'),origin).searchParams.get('v');
      assert(await page.locator('.feature img[data-preview]').evaluateAll(images=>images.every(image=>image.src.includes('/windows-')&&image.alt.endsWith(' — Windows'))));
      await page.locator('button[data-platform=macos]').click();
      assert(await page.locator('.feature img[data-preview]').evaluateAll(images=>images.every(image=>image.src.includes('/macos-')&&image.alt.endsWith(' — macOS'))));
      await page.locator('button[data-platform=windows]').click();
      assert.match(previewVersion,/^[a-f0-9]{12}$/);
      assert(await page.locator('img[data-preview]').evaluateAll((images,version)=>images.every(image=>new URL(image.src).searchParams.get('v')===version),previewVersion));
      assert.equal(await page.locator('button[data-platform=windows]').getAttribute('aria-pressed'),'true');
      assert(await page.locator('.feature img[data-preview]').evaluateAll(images=>images.every(image=>image.src.includes('/windows-'))));
      await page.waitForFunction(()=>[...document.querySelectorAll('img[data-preview]')].every(image=>image.complete&&image.naturalWidth>0));
      await page.locator('[data-feature-tab="overview"]').click();await page.locator('#overview .shot').click();assert(await page.locator('#image-dialog').evaluate(dialog=>dialog.open));
      await page.keyboard.press('Escape');assert.equal(await page.locator('#image-dialog').evaluate(dialog=>dialog.open),false);
      assert(await page.locator('#overview .shot').evaluate(button=>button===document.activeElement));
      await page.locator('.faq details').first().locator('summary').click();assert(await page.locator('.faq details').first().getAttribute('open')!==null);
      await page.setViewportSize({width:390,height:844});await page.goto(origin+url);
      if(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth+1)){
        console.log('Overflow:',await page.evaluate(()=>({width:innerWidth,scroll:document.documentElement.scrollWidth,elements:[...document.querySelectorAll('body *')].map(element=>({tag:element.tagName,class:element.className,x:element.getBoundingClientRect().x,right:element.getBoundingClientRect().right,width:element.getBoundingClientRect().width})).filter(r=>r.width&&r.right>innerWidth+1).slice(0,12)})));
        await page.screenshot({path:path.join(qa,'overflow.png')});
      }
      assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'mobile must not overflow horizontally');
      await page.screenshot({path:path.join(qa,`${language}-mobile.png`),fullPage:true});
      await page.screenshot({path:path.join(qa,`${language}-mobile-hero.png`)});
      await page.locator('.menu-toggle').click();assert.equal(await page.locator('.menu-toggle').getAttribute('aria-expanded'),'true');
      await page.locator('#navigation a[href="#overview"]').click();assert.equal(await page.locator('.menu-toggle').getAttribute('aria-expanded'),'false');
      await page.setViewportSize({width:768,height:1024});assert(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),'tablet must not overflow horizontally');
      await page.setViewportSize({width:1440,height:1100});
    }
    await page.goto(origin+'/#accounts');await page.locator('[data-language]').click();assert.equal(new URL(page.url()).pathname,'/zh-CN/index.html');assert.equal(new URL(page.url()).hash,'#accounts');
    await page.locator('[data-feature-tab="servers"]').click();
    await page.locator('#servers').screenshot({path:path.join(qa,'server-section.png')});
    await page.goto(origin+'/');await page.waitForFunction(()=>document.querySelector('.stage-frame.is-ready:not([hidden])'),null,{timeout:30000});await page.evaluate(()=>scrollTo(0,0));await page.screenshot({path:path.join(qa,'hero-final.png')});
    await checkStage(browser,origin,errors,failed);
    await checkFontsAndAnimation(browser,origin);
    await checkNoScript(browser,origin);
    await checkMobile(browser,origin,errors,failed);
    assert.deepEqual(errors,[]);assert.deepEqual(failed,[]);
    console.log('Passed: bilingual content, assets, links, the live stage with its macOS captures, six feature galleries, platform switching, zoom/focus/Escape, navigation, FAQ, mobile/tablet layout, language anchors.');
    console.log('Visual review:',qa);
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
