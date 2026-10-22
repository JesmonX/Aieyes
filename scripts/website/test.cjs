const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {chromium}=require('../../apps/desktop/node_modules/playwright');
const root=path.resolve(__dirname,'../..'),site=path.join(root,'dist/website'),qa=path.join(root,'.local/website/qa');
const mime={'.html':'text/html; charset=utf-8','.js':'text/javascript','.css':'text/css','.webp':'image/webp','.png':'image/png','.xml':'application/xml','.txt':'text/plain'};
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
    res.setHeader('Content-Type',mime[path.extname(file)]||'application/octet-stream');res.end(fs.readFileSync(file));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const browser=await chromium.launch(),origin=(process.env.AIEYES_WEBSITE_ORIGIN||`http://127.0.0.1:${server.address().port}`).replace(/\/$/,'');
  const errors=[],failed=[];
  try{
    const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce'});
    page.on('pageerror',error=>errors.push(String(error)));page.on('response',response=>{if(response.status()>=400)failed.push(response.url());});
    for(const [language,url] of [['en','/'],['zh-CN','/zh-CN/']]){
      await page.goto(origin+url);assert.equal(await page.locator('html').getAttribute('lang'),language);
      assert.equal(await page.locator('h1').count(),1);assert.equal(await page.locator('.feature').count(),6);
      await page.evaluate(()=>document.querySelectorAll('img').forEach(image=>image.loading='eager'));
      await page.waitForFunction(()=>[...document.images].filter(image=>image.getAttribute('src')).every(image=>image.complete&&image.naturalWidth>0));
      assert.equal(await page.locator('link[rel=alternate][hreflang]').count(),3);
      assert.equal(await page.locator('.downloads a').count(),3);
      assert(await page.locator('a[data-language]').getAttribute('href'));
      await page.screenshot({path:path.join(qa,`${language}-desktop.png`)});
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

      await page.locator('button[data-platform=windows]').click();
      assert.equal(await page.locator('button[data-platform=windows]').getAttribute('aria-pressed'),'true');
      assert(await page.locator('.feature img[data-preview]').evaluateAll(images=>images.every(image=>image.src.includes('/windows-'))));
      await page.waitForFunction(()=>[...document.querySelectorAll('img[data-preview]')].every(image=>image.complete&&image.naturalWidth>0));
      await page.locator('.hero-shot').click();assert(await page.locator('#image-dialog').evaluate(dialog=>dialog.open));
      await page.keyboard.press('Escape');assert.equal(await page.locator('#image-dialog').evaluate(dialog=>dialog.open),false);
      assert(await page.locator('.hero-shot').evaluate(button=>button===document.activeElement));
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
    await page.goto(origin+'/#accounts');await page.locator('[data-language]').click();assert.equal(new URL(page.url()).pathname,'/zh-CN/');assert.equal(new URL(page.url()).hash,'#accounts');
    await page.locator('[data-feature-tab="servers"]').click();
    await page.locator('#servers').screenshot({path:path.join(qa,'server-section.png')});
    await page.goto(origin+'/');await page.locator('button[data-platform=macos]').click();await page.evaluate(()=>scrollTo(0,0));await page.screenshot({path:path.join(qa,'hero-final.png')});
    assert.deepEqual(errors,[]);assert.deepEqual(failed,[]);
    console.log('Passed: bilingual content, assets, links, six feature galleries, platform switching, zoom/focus/Escape, navigation, FAQ, mobile/tablet layout, language anchors.');
    console.log('Visual review:',qa);
  }finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
