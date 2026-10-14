const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
function transparentEdgeAtlas(){
  return 'data:image/png;base64,'+execFileSync(process.platform==='win32'?'python':'python3',['-c',String.raw`
import base64, struct, zlib
w,h=96,117
raw=bytearray()
for y in range(h):
    raw.append(0)
    for x in range(w):
        if x<12 and y<13:
            alpha=0 if x==0 or y in (0,12) else 128 if x==1 or y in (1,11) else 255
            raw.extend((255,0,0,alpha) if alpha else (255,255,255,0))
        else:
            raw.extend((0,0,255,255))
def chunk(kind,data):
    return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
png=b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b'')
print(base64.b64encode(png).decode())
`],{encoding:'utf8'}).trim();
}
function fixture(data){
  const interval=window.setInterval;window.setInterval=(fn,ms)=>ms===100?interval(fn,ms):0;
  const listeners=new Map(),info={platform:'linux',page:'agent',mode:'pet',effectiveMode:'pet',nativePet:false,petEnabled:true,petScale:Number(localStorage.getItem('fixture.petScale')||1),panelOpen:false,panelPinned:false,sessions:[],summary:'空闲',activeCount:0,petAsset:data.pet.pet,pet:{key:'idle',behavior:'idle',label:'空闲',row:0,frames:6,frameMs:100}};
  window.petTest={info,selection:data.pet,calls:[],emit(pet){info.pet=pet;for(const f of listeners.get('desktop:status')||[])f({payload:{...info}});}};
  window.__TAURI__={event:{async listen(name,fn){listeners.set(name,[...(listeners.get(name)||[]),fn]);return()=>{};}},window:{getCurrentWindow:()=>({async startDragging(){window.petTest.dragged=true;},async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(name,args={}){
    petTest.calls.push({name,args});
    if(name==='desktop_info')return {...info};
    if(name==='desktop_panel'){info.panelOpen=args.open;return {...info};}
    if(name==='desktop_pet_scale'){if(petTest.failScale)throw new Error('模拟缩放保存失败');info.petScale=args.scale;localStorage.setItem('fixture.petScale',String(args.scale));return {...info};}
    if(name==='desktop_mode'){info.mode=args.mode;info.petEnabled=args.mode==='pet';return {...info};}
    if(name==='select_local_path')return '/fixture/import.zip';
    if(name!=='engine_call')return {};
    if(args.method==='pets.current')return petTest.selection;
    if(args.method==='pets.import'){info.petAsset={...data.pet.pet,displayName:'导入成功'};return {pet:info.petAsset};}
    return structuredClone({'hello':{version:'fixture'},'settings.get':data.settings,'dashboard':data.dashboard,'prices.list':data.prices,'hosts.sample':data.hosts,'sessions.list':{sessions:[],unavailable:false},'wakeups.list':[],'quotas.refresh':[],'sources.scan':[]}[args.method]??[]);
  }}};
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web'),out=path.resolve(__dirname,'../.local/pets');fs.mkdirSync(out,{recursive:true});
  const data=JSON.parse(execFileSync(process.platform==='win32'?'python':'python3',[path.join(__dirname,'preview-quota-core.py'),'--fixture'],{encoding:'utf8'}));
  const os=require('node:os'),temp=fs.mkdtempSync(path.join(os.tmpdir(),'aieyes-pet-ui-'));
  const core=path.resolve(__dirname,'../target/debug/aieyes-core'+(process.platform==='win32'?'.exe':''));
  data.pet=JSON.parse(execFileSync(core,['--data-dir',temp,'--call','pets.current','{"includeImage":true}'],{encoding:'utf8',maxBuffer:16*1024*1024}));
  fs.rmSync(temp,{recursive:true,force:true});
  const server=http.createServer((req,res)=>{if(require('./ui-test-helpers.cjs').serveFont(req,res))return;const file=path.join(root,path.basename(new URL(req.url,'http://local').pathname)||'floating.html');if(!fs.existsSync(file)){res.writeHead(404).end();return;}res.setHeader('content-type',({'.html':'text/html','.js':'text/javascript','.css':'text/css','.png':'image/png'})[path.extname(file)]||'text/plain');res.end(fs.readFileSync(file));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));const browser=await chromium.launch();
  try{
    const page=await browser.newPage({viewport:{width:480,height:780}}),errors=[];page.on('pageerror',e=>errors.push(String(e)));await page.addInitScript(fixture,data);
    await page.goto(`http://127.0.0.1:${server.address().port}/floating.html`);await page.waitForFunction(()=>window.AieyesPet&&document.body.dataset.pet==='true');
    await page.waitForFunction(()=>Boolean(document.querySelector('#pet-sprite').dataset.asset));
    const box=await page.locator('#ball').boundingBox();assert.equal(box.width,144);assert.equal(box.height,168);
    for(const [name,row,count,ms] of [['idle',0,6,100],['running',1,8,100],['attention',3,4,100],['success',4,5,100],['blocked',5,8,100],['rest',2,8,100],['unknown',6,6,100],['working',7,6,100],['thinking',8,6,100]]){
      await page.emulateMedia({reducedMotion:'reduce'});
      const state={behavior:name,label:name,row,frames:count,frameMs:ms,key:name};await page.evaluate(s=>petTest.emit(s),state);
      assert.equal(await page.locator('#pet-sprite').getAttribute('data-frame'),String(row*8));
      assert.equal(await page.locator('#phase').textContent(),name);
      assert.equal(await page.locator('body').getAttribute('data-pet-behavior'),name);
      assert.match(await page.locator('#ball').getAttribute('aria-label'),new RegExp(name));
      const color=await page.evaluate(name=>{
        const phase={attention:'needsInput',blocked:'failed',success:'complete',rest:'interrupted',running:'working',working:'tool',thinking:'thinking',unknown:'unknown',idle:'idle'}[name];
        const probe=document.createElement('span');probe.style.color=`var(--phase-${phase})`;document.body.append(probe);
        const result=[getComputedStyle(document.querySelector('#phase'),'::before').backgroundColor,getComputedStyle(probe).color];probe.remove();return result;
      },name);assert.equal(color[0],color[1],'indicator uses selected pet state, including attention over another running session');
      assert.equal(await page.evaluate(s=>Math.floor(AieyesPet.frame(s,100000,false)/8),state),row,'long-running animation keeps the row described by its label');
      assert.deepEqual(await page.evaluate(s=>Array.from({length:s.frames*8},(_,step)=>AieyesPet.frame(s,step*100+1,false)),state),Array.from({length:count*8},(_,step)=>row*8+step%count),'all behaviors use the same cadence for more than three cycles');
      await page.locator('#ball').screenshot({path:path.join(out,name+'.png')});
    }
    await page.emulateMedia({reducedMotion:'no-preference'});
    await page.evaluate(()=>petTest.emit({behavior:'idle',label:'空闲',row:0,frames:6,frameMs:100,key:'idle-cadence'}));
    await page.waitForFunction(()=>Number(document.querySelector('#pet-sprite').dataset.frame)>=4,null,{timeout:1800});
    await page.evaluate(()=>{petTest.info.recovery='failed';petTest.emit(petTest.info.pet);});
    assert.equal(await page.locator('#phase').textContent(),'恢复失败');assert.match(await page.locator('#ball').getAttribute('aria-label'),/恢复失败/);
    await page.evaluate(()=>{delete petTest.info.recovery;petTest.emit(petTest.info.pet);});
    await page.emulateMedia({reducedMotion:'no-preference'});await page.evaluate(()=>petTest.emit({behavior:'running',label:'进行中',row:1,frames:8,frameMs:100,key:'fresh'}));
    await page.waitForFunction(()=>Number(document.querySelector('#pet-sprite').dataset.frame)>8);
    // Frequent status updates must not starve the animation or introduce held/skipped frames.
    const cadence=await page.evaluate(()=>new Promise(resolve=>{
      petTest.emit({behavior:'idle',label:'空闲',row:0,frames:6,frameMs:100,key:'continuous'});
      const started=performance.now(),gaps=[],frames=[];let changed=started,previous=0,updated=started;
      function observe(now){
        if(now-updated>=37){petTest.emit({...petTest.info.pet,activeCount:Math.floor(now)%4});updated=now;}
        const frame=Number(document.querySelector('#pet-sprite').dataset.frame);
        if(frame!==previous){gaps.push(now-changed);frames.push(frame);changed=now;previous=frame;}
        if(now-started<30000)requestAnimationFrame(observe);else resolve({gaps,frames});
      }
      requestAnimationFrame(observe);
    }));
    assert(cadence.frames.length>=240,'30 seconds of frequent polls must keep the 10 fps animation progressing');
    const sorted=cadence.gaps.slice().sort((a,b)=>a-b);
    assert(sorted[Math.floor(sorted.length*0.95)]<180,'normal frame cadence remains close to 100 ms');
    assert(Math.max(...cadence.gaps)<500,'no extra pauses under frequent status updates');
    await page.evaluate(()=>petTest.emit({behavior:'running',label:'进行中',row:1,frames:8,frameMs:100,key:'fresh-after-cadence'}));
    await page.waitForFunction(()=>Number(document.querySelector('#pet-sprite').dataset.frame)>8);
    const before=await page.locator('#pet-sprite').getAttribute('data-frame');await page.evaluate(()=>petTest.emit(petTest.info.pet));assert.notEqual(before,'8');assert.notEqual(await page.locator('#pet-sprite').getAttribute('data-frame'),'8','poll does not restart animation');
    assert.equal(await page.evaluate(()=>petTest.calls.filter(c=>c.args.method==='pets.current').length),1,'status polls reuse the loaded asset');
    await page.emulateMedia({reducedMotion:'reduce'});
    await page.evaluate(async()=>{
      const image=await createImageBitmap(await (await fetch(petTest.selection.imageDataUrl)).blob());
      const canvas=document.createElement('canvas');canvas.width=image.width;canvas.height=petTest.selection.pet.cellHeight*11;
      canvas.getContext('2d').drawImage(image,0,0);image.close();
      petTest.selection={pet:{...petTest.selection.pet,rows:11,spriteVersionNumber:2,spritePath:'/fixture/v2.png'},imageDataUrl:canvas.toDataURL()};
      petTest.info.petAsset=petTest.selection.pet;
      petTest.emit({behavior:'thinking',label:'思考中',row:8,frames:6,frameMs:100,key:'v2'});
    });
    await page.waitForFunction(()=>document.querySelector('#pet-sprite').dataset.rows==='11');
    assert.equal(await page.locator('#pet-sprite').getAttribute('data-frame'),'64','v2 crops row 8 of 11');
    await page.locator('#ball').screenshot({path:path.join(out,'v2.png')});
    for(const scale of [0.5,1,1.25,1.5,2]){
      await page.evaluate(scale=>{petTest.info.petScale=scale;petTest.emit({...petTest.info.pet,label:'等待1 · 工作2\n思考1'});},scale);
      const box=await page.locator('#ball').boundingBox(),sprite=await page.locator('#pet-sprite').boundingBox();
      assert.equal(box.width,144*scale);assert.equal(box.height,186*scale);
      assert.equal(sprite.width,132*scale);assert.equal(sprite.height,143*scale);
      const pixels=await page.locator('#pet-sprite').evaluate(el=>{const p=el.getContext('2d').getImageData(0,0,el.width,el.height).data;return {corner:p[3],visible:p.some((v,i)=>i%4===3&&v>0),width:el.width};});
      assert.equal(pixels.corner,0);assert(pixels.visible);assert.equal(pixels.width,Math.round(132*scale));
      for(const color of ['#fafafa','#151b28']){
        await page.evaluate(color=>document.body.style.background=color,color);
        await page.locator('#ball').screenshot({path:path.join(out,`scale-${scale}-${color==='#fafafa'?'light':'dark'}.png`)});
      }
    }
    // Hidden white RGB and opaque blue neighboring cells must not tint red edges.
    await page.evaluate(imageDataUrl=>{
      petTest.selection={pet:{...petTest.selection.pet,cellWidth:12,cellHeight:13,rows:9,spritePath:'/fixture/alpha.png'},imageDataUrl};
      petTest.info.petAsset=petTest.selection.pet;petTest.emit({behavior:'idle',label:'透明边缘',row:0,frames:1,frameMs:100,key:'alpha'});
    },transparentEdgeAtlas());
    await page.waitForFunction(()=>document.querySelector('#pet-sprite').dataset.asset==='/fixture/alpha.png');
    for(const scale of [0.5,1,1.25,1.5,2]){
      await page.evaluate(scale=>{petTest.info.petScale=scale;petTest.emit(petTest.info.pet);},scale);
      const colors=await page.locator('#pet-sprite').evaluate(el=>{
        const pixels=el.getContext('2d').getImageData(0,0,el.width,el.height).data;let visible=0,fringe=0;
        for(let i=0;i<pixels.length;i+=4)if(pixels[i+3]>8){visible++;if(pixels[i]<250||pixels[i+1]>3||pixels[i+2]>3)fringe++;}
        return {visible,fringe};
      });assert(colors.visible>0);assert.equal(colors.fringe,0,'Isolated premultiplied frames have no black/white or neighboring-frame fringe at '+scale);
    }
    await page.evaluate(()=>{petTest.info.petScale=1;petTest.emit({...petTest.info.pet,label:'思考中'});});
    await page.locator('#ball').click();assert.equal(await page.locator('body').getAttribute('data-view'),'panel');await page.keyboard.press('Escape');assert.equal(await page.locator('body').getAttribute('data-view'),'ball');
    await page.locator('#ball').focus();await page.keyboard.press('Enter');assert.equal(await page.locator('body').getAttribute('data-view'),'panel');await page.keyboard.press('Escape');
    await page.locator('#ball').dispatchEvent('pointerdown',{button:0,buttons:1,clientX:50,clientY:60});await page.locator('#ball').dispatchEvent('pointermove',{buttons:1,clientX:65,clientY:70});assert.equal(await page.evaluate(()=>petTest.dragged),true);
    await page.emulateMedia({forcedColors:'active'});await page.locator('#ball').screenshot({path:path.join(out,'high-contrast.png')});
    await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);await page.waitForFunction(()=>typeof state!=='undefined'&&state.settings);await page.evaluate(()=>{state.page='settings';state.settingsTab='general';state.generalTab='desktop';render();});
    await page.locator('#desktop-mode').selectOption('tray');await page.waitForFunction(()=>petTest.info.mode==='tray');await page.locator('#desktop-mode').selectOption('pet');await page.waitForFunction(()=>petTest.info.petEnabled);
    assert.deepEqual(await page.locator('#desktop-mode option').evaluateAll(options=>options.map(o=>o.value)),['pet','tray']);
    await page.locator('#pet-scale').fill('1.25');await page.locator('#pet-scale').dispatchEvent('change');
    await page.waitForFunction(()=>petTest.info.petScale===1.25);
    assert.equal(await page.locator('#pet-scale-value').textContent(),'125%');
    await page.evaluate(()=>{petTest.failScale=true;});
    await page.locator('#pet-scale').fill('1.5');await page.locator('#pet-scale').dispatchEvent('change');
    await page.waitForFunction(()=>document.querySelector('#pet-scale').value==='1.25');
    await page.evaluate(()=>{petTest.failScale=false;});
    await page.locator('#reset-pet-scale').click();await page.waitForFunction(()=>petTest.info.petScale===1);
    await page.locator('#import-pet').click();await page.waitForFunction(()=>document.querySelector('#current-pet').textContent.includes('导入成功'));
    assert.ok(await page.evaluate(()=>petTest.calls.some(c=>c.args.method==='pets.import'&&c.args.params.path==='/fixture/import.zip')));
    const retina=await browser.newPage({viewport:{width:480,height:780},deviceScaleFactor:2,reducedMotion:'reduce'});
    await retina.addInitScript(fixture,data);await retina.goto(`http://127.0.0.1:${server.address().port}/floating.html`);
    await retina.waitForFunction(()=>Boolean(document.querySelector('#pet-sprite').dataset.asset));
    await retina.evaluate(()=>{petTest.info.petScale=1.25;petTest.emit(petTest.info.pet);});
    assert.equal(await retina.locator('#pet-sprite').evaluate(el=>el.width),330);
    assert.equal((await retina.locator('#ball').boundingBox()).width,180);
    await retina.close();
    assert.deepEqual(errors,[]);console.log('Pet UI: nine behaviors, 10 fps, v1/v2 grids, asset caching, reduced motion, scale/DPI geometry, transparent/adjacent-frame edges, panel/keyboard/drag, high contrast and file import passed');
  }finally{await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
