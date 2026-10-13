const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function fixture(data){
  const interval=window.setInterval;window.setInterval=(fn,ms)=>ms===100?interval(fn,ms):0;
  const listeners=new Map(),info={platform:'linux',page:'agent',mode:'pet',effectiveMode:'floating',nativeCapsule:false,petEnabled:true,panelOpen:false,panelPinned:false,sessions:[],summary:'空闲',activeCount:0,pet:{key:'idle',behavior:'idle',label:'空闲',row:0,frames:6,frameMs:900}};
  window.petTest={info,calls:[],emit(pet){info.pet=pet;for(const f of listeners.get('desktop:status')||[])f({payload:{...info}});}};
  window.__TAURI__={event:{async listen(name,fn){listeners.set(name,[...(listeners.get(name)||[]),fn]);return()=>{};}},window:{getCurrentWindow:()=>({async startDragging(){window.petTest.dragged=true;},async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(name,args={}){
    petTest.calls.push({name,args});
    if(name==='desktop_info')return {...info};
    if(name==='desktop_panel'){info.panelOpen=args.open;return {...info};}
    if(name==='desktop_mode'){info.mode=args.mode;info.petEnabled=args.mode==='pet';return {...info};}
    if(name!=='engine_call')return {};
    return structuredClone({'hello':{version:'fixture'},'settings.get':data.settings,'dashboard':data.dashboard,'prices.list':data.prices,'hosts.sample':data.hosts,'sessions.list':{sessions:[],unavailable:false},'wakeups.list':[],'quotas.refresh':[],'sources.scan':[]}[args.method]??[]);
  }}};
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web'),out=path.resolve(__dirname,'../.local/pets');fs.mkdirSync(out,{recursive:true});
  const data=JSON.parse(execFileSync(process.platform==='win32'?'python':'python3',[path.join(__dirname,'preview-quota-core.py'),'--fixture'],{encoding:'utf8'}));
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'floating.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));const browser=await chromium.launch();
  try{
    const page=await browser.newPage({viewport:{width:480,height:780}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture,data);
    await page.goto(`http://127.0.0.1:${server.address().port}/floating.html`);await page.waitForFunction(()=>window.AieyesPet&&document.body.dataset.pet==='true');
    const box=await page.locator('#ball').boundingBox();assert.equal(box.width,144);assert.equal(box.height,168);
    for(const [name,row,count,ms] of [['idle',0,6,900],['running',1,8,150],['attention',3,4,180],['success',4,5,180],['blocked',5,8,180],['unknown',6,6,260],['working',7,6,150],['thinking',8,6,220]]){
      await page.emulateMedia({reducedMotion:'reduce'});
      const state={behavior:name,label:name,row,frames:count,frameMs:ms,key:name};await page.evaluate(s=>petTest.emit(s),state);
      assert.equal(await page.locator('#pet-sprite').getAttribute('data-frame'),String(row*8));
      assert.equal(await page.locator('#phase').textContent(),name);
      assert.equal(await page.locator('body').getAttribute('data-pet-behavior'),name);
      assert.match(await page.locator('#ball').getAttribute('aria-label'),new RegExp(name));
      const color=await page.evaluate(name=>{
        const phase={attention:'needsInput',blocked:'failed',success:'complete',running:'working',working:'tool',thinking:'thinking',unknown:'unknown',idle:'idle'}[name];
        const probe=document.createElement('span');probe.style.color=`var(--phase-${phase})`;document.body.append(probe);
        const result=[getComputedStyle(document.querySelector('#phase'),'::before').backgroundColor,getComputedStyle(probe).color];probe.remove();return result;
      },name);assert.equal(color[0],color[1],'indicator uses selected pet state, including attention over another running session');
      assert.equal(await page.evaluate(s=>AieyesPet.frame(s,100000,false)<6,state),true);
      await page.locator('#ball').screenshot({path:path.join(out,name+'.png')});
    }
    await page.evaluate(()=>{petTest.info.recovery='failed';petTest.emit(petTest.info.pet);});
    assert.equal(await page.locator('#phase').textContent(),'恢复失败');assert.match(await page.locator('#ball').getAttribute('aria-label'),/恢复失败/);
    await page.evaluate(()=>{delete petTest.info.recovery;petTest.emit(petTest.info.pet);});
    await page.emulateMedia({reducedMotion:'no-preference'});await page.evaluate(()=>petTest.emit({behavior:'running',label:'进行中',row:1,frames:8,frameMs:150,key:'fresh'}));
    await page.waitForFunction(()=>Number(document.querySelector('#pet-sprite').dataset.frame)>8);
    const before=await page.locator('#pet-sprite').getAttribute('data-frame');await page.evaluate(()=>petTest.emit(petTest.info.pet));assert.notEqual(before,'8');assert.notEqual(await page.locator('#pet-sprite').getAttribute('data-frame'),'8','poll does not restart animation');
    await page.locator('#ball').click();assert.equal(await page.locator('body').getAttribute('data-view'),'panel');await page.keyboard.press('Escape');assert.equal(await page.locator('body').getAttribute('data-view'),'ball');
    await page.locator('#ball').focus();await page.keyboard.press('Enter');assert.equal(await page.locator('body').getAttribute('data-view'),'panel');await page.keyboard.press('Escape');
    await page.locator('#ball').dispatchEvent('pointerdown',{button:0,buttons:1,clientX:50,clientY:60});await page.locator('#ball').dispatchEvent('pointermove',{buttons:1,clientX:65,clientY:70});assert.equal(await page.evaluate(()=>petTest.dragged),true);
    await page.emulateMedia({forcedColors:'active'});await page.locator('#ball').screenshot({path:path.join(out,'high-contrast.png')});
    await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.settings);await page.evaluate(()=>{state.page='settings';state.settingsTab='general';render();});
    await page.locator('#desktop-mode').selectOption('floating');await page.waitForFunction(()=>petTest.info.mode==='floating');await page.locator('#desktop-mode').selectOption('pet');await page.waitForFunction(()=>petTest.info.petEnabled);
    assert.deepEqual(errors,[]);console.log('Pet UI: eight behaviors, reduced motion, stable polling, panel/keyboard/drag, high contrast and optional mode passed');
  }finally{await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
