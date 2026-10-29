const {revealControl} = require('./ui-test-helpers.cjs');
const assert=require('node:assert/strict'),fs=require('node:fs'),path=require('node:path'),http=require('node:http');
const {execFileSync}=require('node:child_process');
const {chromium}=require('../apps/desktop/node_modules/playwright');
const frameDurations=[
  [280,110,110,140,140,320],[120,120,120,120,120,120,120,220],
  [120,120,120,120,120,120,120,220],[140,140,140,280],[140,140,140,140,280],
  [140,140,140,140,140,140,140,240],[150,150,150,150,150,260],
  [120,120,120,120,120,220],[150,150,150,150,150,280]
];
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
  const listeners=new Map(),info={platform:'linux',page:'agent',mode:'pet',effectiveMode:'pet',nativePet:false,petEnabled:true,petScale:Number(localStorage.getItem('fixture.petScale')||1),petUniformTiming:localStorage.getItem('fixture.petUniformTiming')==='true',panelOpen:false,panelPinned:false,sessions:[],summary:'空闲',activeCount:0,petAsset:data.pet.pet,pet:{key:'idle',behavior:'idle',label:'空闲',row:0,frames:6,frameMs:100,frameDurationsMs:data.frameDurations[0]}};
  const pets=Array.from({length:18},(_,i)=>({...data.pet.pet,libraryId:i===0?data.pet.pet.libraryId:'fixture-'+i,spritePath:'/fixture/pet-'+i+'.png',displayName:i<2?'同名桌宠':'桌宠 '+i,creator:'测试作者',source:'https://example.invalid/pets/'+i,importedAt:'2026-10-'+String(i+1).padStart(2,'0')+'T08:00:00Z'}));
  window.petTest={info,pets,selection:data.pet,calls:[],emit(pet){info.pet=pet;for(const f of listeners.get('desktop:status')||[])f({payload:{...info}});}};
  window.__TAURI__={event:{async listen(name,fn){listeners.set(name,[...(listeners.get(name)||[]),fn]);return()=>{};}},window:{getCurrentWindow:()=>({async startDragging(){window.petTest.dragged=true;},async isMaximized(){return false;},async onResized(){return()=>{};}})},core:{async invoke(name,args={}){
    petTest.calls.push({name,args});
    if(name==='desktop_info')return {...info};
    if(name==='desktop_panel'){info.panelOpen=args.open;return {...info};}
    if(name==='desktop_pet_uniform_timing'){if(petTest.failTiming)throw new Error('模拟调试保存失败');info.petUniformTiming=args.enabled;localStorage.setItem('fixture.petUniformTiming',String(args.enabled));return {...info};}
    if(name==='desktop_pet_scale'){if(petTest.failScale)throw new Error('模拟缩放保存失败');info.petScale=args.scale;localStorage.setItem('fixture.petScale',String(args.scale));return {...info};}
    if(name==='desktop_mode'){info.mode=args.mode;info.petEnabled=args.mode==='pet';return {...info};}
    if(name==='select_local_path')return '/fixture/import.zip';
    if(name!=='engine_call')return {};
    if(args.method==='pets.current')return petTest.selection;
    if(args.method==='pets.list'){
      if(petTest.failList)throw Error('模拟库读取失败');
      const result={pets:structuredClone(pets),selectedLibraryId:petTest.selection.pet.libraryId,warnings:[{path:'legacy/broken',error:'缓存损坏，其他桌宠仍可使用'}]};
      if(petTest.holdList){petTest.holdList=false;await new Promise(resolve=>petTest.releaseList=resolve);}return result;
    }
    if(args.method==='pets.asset')return {pet:pets.find(p=>p.libraryId===args.params.libraryId),imageDataUrl:data.pet.imageDataUrl};
    if(args.method==='pets.import'){
      if(petTest.failImport)throw Error('模拟导入失败');
      info.petAsset={...data.pet.pet,libraryId:'imported',spritePath:'/fixture/imported.png',displayName:'导入成功'};
      if(!pets.some(p=>p.libraryId==='imported'))pets.push(info.petAsset);
      petTest.selection={pet:info.petAsset,imageDataUrl:data.pet.imageDataUrl};return {pet:info.petAsset};
    }
    if(args.method==='pets.select'){
      if(petTest.failSelect)throw Error('模拟选择失败');
      info.petAsset=pets.find(p=>p.libraryId===args.params.libraryId);petTest.selection={pet:info.petAsset,imageDataUrl:data.pet.imageDataUrl};return {pet:info.petAsset};
    }
    return structuredClone({'hello':{version:'fixture'},'settings.get':data.settings,'dashboard':data.dashboard,'prices.list':data.prices,'hosts.sample':data.hosts,'sessions.list':{sessions:[],unavailable:false},'wakeups.list':[],'quotas.refresh':[],'sources.scan':[]}[args.method]??[]);
  }}};
}
(async()=>{
  const root=path.resolve(__dirname,'../apps/desktop/web'),out=path.resolve(__dirname,'../.local/ui-review-20261014-implementation/web/pets');fs.mkdirSync(out,{recursive:true});
  const data=JSON.parse(execFileSync(process.platform==='win32'?'python':'python3',[path.join(__dirname,'preview-quota-core.py'),'--fixture'],{encoding:'utf8'}));
  data.frameDurations=frameDurations;
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
    for(const [name,row] of [['idle',0],['running',1],['attention',3],['success',4],['blocked',5],['rest',0],['unknown',6],['working',7],['thinking',8]]){
      await page.emulateMedia({reducedMotion:'reduce'});
      const timing=frameDurations[row],count=timing.length,state={behavior:name,label:name==='rest'?'已中断':name,row,frames:count,frameMs:100,frameDurationsMs:timing,key:name};await page.evaluate(s=>petTest.emit(s),state);
      assert.equal(await page.locator('#pet-sprite').getAttribute('data-frame'),String(row*8));
      assert.equal(await page.locator('#phase').textContent(),state.label);
      assert.equal(await page.locator('body').getAttribute('data-pet-behavior'),name);
      assert.match(await page.locator('#ball').getAttribute('aria-label'),new RegExp(state.label));
      const color=await page.evaluate(name=>{
        const phase={attention:'needsInput',blocked:'failed',success:'complete',rest:'interrupted',running:'working',working:'tool',thinking:'thinking',unknown:'unknown',idle:'idle'}[name];
        const probe=document.createElement('span');probe.style.color=`var(--phase-${phase})`;document.body.append(probe);
        const result=[getComputedStyle(document.querySelector('#phase'),'::before').backgroundColor,getComputedStyle(probe).color];probe.remove();return result;
      },name);assert.equal(color[0],color[1],'indicator uses selected pet state, including attention over another running session');
      assert.equal(await page.evaluate(s=>Math.floor(AieyesPet.frame(s,100000,false)/8),state),row,'long-running animation keeps the row described by its label');
      const cycle=timing.reduce((sum,ms)=>sum+ms,0),boundaries=timing.map((_,i)=>timing.slice(0,i+1).reduce((sum,ms)=>sum+ms,0));
      const samples=boundaries.flatMap((boundary,i)=>[{elapsed:boundary-1,expected:row*8+i},{elapsed:boundary,expected:row*8+(i+1)%count},{elapsed:cycle*8+boundary-1,expected:row*8+i}]);
      assert.deepEqual(await page.evaluate(({state,samples})=>samples.map(s=>AieyesPet.frame(state,s.elapsed,false)),{state,samples}),samples.map(s=>s.expected),'per-frame boundaries and continuing loops match the agreed timing');
      await page.locator('#ball').screenshot({path:path.join(out,name+'.png')});
    }
    await page.emulateMedia({reducedMotion:'no-preference'});
    const continuity=new Set();
    for(let turn=0;turn<16;turn++) {
      await page.evaluate(turn=>petTest.emit({behavior:'idle',label:'Idle',row:0,frames:6,frameMs:40,frameDurationsMs:Array(6).fill(40),key:'session-'+turn}),turn);
      await page.waitForTimeout(30);continuity.add(await page.locator('#pet-sprite').getAttribute('data-frame'));
    }
    assert(continuity.size>=4,'Session changes do not hold the animation on its first frame');
    assert.deepEqual(await page.evaluate(()=>{AieyesPetAnimation.setUniform(true);const result=AieyesPetAnimation.durations(AieyesPetAnimation.idle());AieyesPetAnimation.setUniform(false);return result;}),Array(6).fill(100),'Uniform debug clock uses 100ms frames');
    assert.deepEqual(await page.evaluate(()=>AieyesPetAnimation.idle().frameDurationsMs),frameDurations[0],'library idle and the live player share the generated timing');
    assert.equal(await page.evaluate(()=>AieyesPet.frame({row:2,frames:8,frameDurationsMs:[120,120,120,120,120,120,120,220]},1059)),23,'running-left uses its eight-frame row');
    assert.equal(await page.evaluate(()=>AieyesPet.frame({row:2,frames:8,frameDurationsMs:[120,120,120,120,120,120,120,220]},1060)),16);
    assert.equal(await page.evaluate(()=>AieyesPet.frame({row:0,frames:2,frameMs:75},75)),1,'legacy frameMs remains a fallback for custom frame counts');
    assert.deepEqual(await page.evaluate(()=>AieyesPetAnimation.durations({row:0,frames:6,frameDurationsMs:[],frameMs:100})),frameDurations[0],'empty duration arrays use the same shared timing as core');
    assert.deepEqual(await page.evaluate(()=>AieyesPetAnimation.durations({row:0,frames:2,frameDurationsMs:[Infinity,100],frameMs:75})),[75,75],'invalid nonempty arrays retain the scalar fallback');
    assert.deepEqual(await page.evaluate(()=>AieyesPetAnimation.durations({row:0,frames:2,frameDurationsMs:[1e308,1e308],frameMs:1e308})),[100,100],'overflowing cycle durations remain finite');
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
        if(now-started<4500)requestAnimationFrame(observe);else resolve({gaps,frames});
      }
      requestAnimationFrame(observe);
    }));
    assert(cadence.frames.length>=23,'frequent polls keep the variable-duration idle animation progressing beyond three loops');
    const sorted=cadence.gaps.slice().sort((a,b)=>a-b);
    assert(sorted[Math.floor(sorted.length*0.95)]<390,'held frames respect the 280/320 ms timing without restarting');
    assert(Math.max(...cadence.gaps)<550,'no extra pauses under frequent status updates');
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
    await page.locator('#ball').click();assert.equal(await page.locator('body').getAttribute('data-view'),'panel');
    await page.setViewportSize({width:450,height:720});
    await page.evaluate(()=>{document.body.classList.add('sidebar-collapsed');document.body.style.setProperty('--font-entry','14px');});
    const navigation=await page.locator('.panel-head .panel-tabs').evaluate(nav=>{
      const box=nav.getBoundingClientRect();
      return {center:(box.left+box.right)/2,lines:[...nav.children].map(button=>{const range=document.createRange();range.selectNodeContents(button);return range.getClientRects().length;})};
    });
    assert.deepEqual(navigation.lines,[1,1],'panel labels stay on one line with collapsed sidebar styles');
    assert(Math.abs(navigation.center-225)<1,'Linux panel navigation stays centered beside the close/refresh actions');
    await page.keyboard.press('Escape');assert.equal(await page.locator('body').getAttribute('data-view'),'ball');
    await page.setViewportSize({width:480,height:780});
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
    await page.locator('details:has(#pet-uniform) > summary').click();
    await page.locator('#pet-uniform').check();
    await page.waitForFunction(()=>petTest.info.petUniformTiming===true);
    assert.deepEqual(await page.evaluate(()=>AieyesPetAnimation.durations(AieyesPetAnimation.idle())),Array(6).fill(100),'Settings toggle updates the preview clock');
    await page.reload();await page.evaluate(()=>AieyesApp.ready);
    await page.evaluate(()=>{state.page='settings';state.settingsTab='general';state.generalTab='desktop';render();});
    await page.locator('details:has(#pet-uniform) > summary').click();
    assert(await page.locator('#pet-uniform').isChecked(),'Debug preference survives reopening');
    await page.evaluate(()=>petTest.failTiming=true);await page.locator('#pet-uniform').click();
    await page.waitForFunction(()=>!document.querySelector('#pet-uniform').disabled);
    assert(await page.locator('#pet-uniform').isChecked(),'Failed preference save restores the toggle');
    await page.evaluate(()=>petTest.failTiming=false);await page.locator('#pet-uniform').uncheck();
    await page.waitForFunction(()=>petTest.info.petUniformTiming===false);
    assert.deepEqual(await page.evaluate(()=>AieyesPetAnimation.durations(AieyesPetAnimation.idle())),frameDurations[0]);
    await page.locator('#pet-library-list').scrollIntoViewIfNeeded();
    await page.waitForFunction(()=>document.querySelectorAll('.pet-library-card canvas[data-frame]').length>0);
    assert.equal(await page.locator('[data-pet-select]').count(),18,'library metadata loads independently of sprite sheets');
    const requested=await page.evaluate(()=>petTest.calls.filter(c=>c.args.method==='pets.asset').length);
    assert(requested>0&&requested<18,'only visible previews request their image assets');
    assert.match(await page.locator('#pet-library-warnings').textContent(),/缓存损坏/);
    assert.match(await page.locator('[data-pet-id="fixture-1"]').textContent(),/测试作者/);
    assert.match(await page.locator('[data-pet-id="fixture-1"]').textContent(),/2026/,'duplicate names are distinguished by import dates');
    await page.emulateMedia({forcedColors:'none',reducedMotion:'no-preference'});
    const first=page.locator('.pet-library-card').first();await first.scrollIntoViewIfNeeded();
    await page.waitForFunction(()=>Number(document.querySelector('.pet-library-card canvas').dataset.frame)>0);
    await page.locator('.pet-library-card').last().scrollIntoViewIfNeeded();
    await page.waitForFunction(()=>document.querySelector('.pet-library-card:last-child canvas').dataset.frame!==undefined);
    const offscreenFrame=await first.locator('canvas').getAttribute('data-frame');
    await page.waitForTimeout(380);
    assert.equal(await first.locator('canvas').getAttribute('data-frame'),offscreenFrame,'offscreen previews stop painting');
    await page.emulateMedia({reducedMotion:'reduce'});
    await page.waitForFunction(()=>document.querySelector('.pet-library-card:last-child canvas').dataset.frame==='0');
    await page.evaluate(()=>{petTest.previewCanvas=document.querySelector('.pet-library-card:last-child canvas');petTest.previewPaints=0;new MutationObserver(()=>petTest.previewPaints++).observe(petTest.previewCanvas,{attributes:true,attributeFilter:['data-frame']});state.generalTab='appearance';renderSettings();});
    await page.emulateMedia({reducedMotion:'no-preference'});await page.waitForTimeout(380);
    assert.equal(await page.evaluate(()=>petTest.previewPaints),0,'leaving desktop settings disposes preview playback');
    await page.evaluate(()=>{state.generalTab='desktop';renderSettings();});
    await page.evaluate(()=>{petTest.failImport=true;});
    await revealControl(page, '#pet-import-input');await page.locator('#pet-import-input').fill('npx @fixture/pet@latest');await page.locator('#import-pet').click();
    await page.getByText('模拟导入失败',{exact:true}).waitFor();
    assert.equal(await page.locator('#pet-import-input').inputValue(),'npx @fixture/pet@latest');
    assert.notEqual(await page.evaluate(()=>petTest.selection.pet.libraryId),'imported','failed imports preserve the selected pet');
    await page.evaluate(()=>{petTest.failImport=false;});await page.locator('#pet-import-retry').click();
    await page.waitForFunction(()=>document.querySelector('#current-pet').textContent.includes('导入成功'));
    assert.ok(await page.evaluate(()=>petTest.calls.some(c=>c.args.method==='pets.import'&&c.args.params.input==='npx @fixture/pet@latest')));
    assert.equal(await page.locator('[data-pet-select]').count(),19,'successful imports preserve the existing library');
    await revealControl(page, '#select-pet-file');await page.locator('#select-pet-file').click();
    await page.waitForFunction(()=>!document.querySelector('#select-pet-file').disabled);
    assert.ok(await page.evaluate(()=>petTest.calls.some(c=>c.args.method==='pets.import'&&c.args.params.input==='/fixture/import.zip')));
    assert.equal(await page.locator('[data-pet-select]').count(),19,'identical imports reuse a library entry');
    await page.evaluate(()=>{petTest.failSelect=true;});await page.locator('[data-pet-select="fixture-1"]').click();
    await page.getByText('模拟选择失败',{exact:true}).waitFor();
    assert.equal(await page.evaluate(()=>petTest.selection.pet.libraryId),'imported');
    await page.evaluate(()=>{petTest.failSelect=false;});await page.locator('#pet-import-retry').click();
    await page.waitForFunction(()=>document.querySelector('[data-pet-select="fixture-1"]').getAttribute('aria-pressed')==='true');
    assert(!await page.locator('#import-pet').isDisabled());assert(!await page.locator('#pet-import-input').isDisabled());assert(!await page.locator('#select-pet-file').isDisabled());
    assert(await page.locator('[data-pet-select][aria-pressed=false]').evaluateAll(buttons=>buttons.every(button=>!button.disabled)),'all noncurrent pets remain selectable after switching');
    for(const width of [1120,640]){await page.setViewportSize({width,height:850});for(const theme of ['light','dark']){await page.evaluate(theme=>{notify('');AieyesTheme.apply({...state.settings.appearance,theme});document.querySelector('#pet-library').scrollIntoView({block:'start'});document.querySelector('#pet-library-list').scrollTop=0;},theme);await page.screenshot({path:path.join(out,'library-'+width+'-'+theme+'.png')});}}
    const retina=await browser.newPage({viewport:{width:480,height:780},deviceScaleFactor:2,reducedMotion:'reduce'});
    await retina.addInitScript(fixture,data);await retina.goto(`http://127.0.0.1:${server.address().port}/floating.html`);
    await retina.waitForFunction(()=>Boolean(document.querySelector('#pet-sprite').dataset.asset));
    await retina.evaluate(()=>{petTest.info.petScale=1.25;petTest.emit(petTest.info.pet);});
    assert.equal(await retina.locator('#pet-sprite').evaluate(el=>el.width),330);
    assert.equal((await retina.locator('#ball').boundingBox()).width,180);
    await retina.close();
    const race=await browser.newPage({viewport:{width:1120,height:850},reducedMotion:'reduce'});race.on('pageerror',e=>errors.push(String(e)));await race.addInitScript(fixture,data);
    await race.goto(`http://127.0.0.1:${server.address().port}/index.html`);await race.evaluate(()=>AieyesApp.ready);
    await race.evaluate(()=>{petTest.holdList=true;state.page='settings';state.settingsTab='general';state.generalTab='desktop';render();});
    await race.waitForFunction(()=>petTest.releaseList);
    await revealControl(race, '#pet-import-input');await race.locator('#pet-import-input').fill('@fixture/fast-import');await race.locator('#import-pet').click();
    await race.waitForFunction(()=>petTest.selection.pet.libraryId==='imported');await race.evaluate(()=>petTest.releaseList());
    await race.waitForFunction(()=>document.querySelector('#current-pet').textContent.includes('导入成功')&&!document.querySelector('#import-pet').disabled);
    assert.equal(await race.locator('[data-pet-select]').count(),19,'a late initial list cannot hide a pet imported while metadata was loading');
    await race.evaluate(()=>{state.generalTab='appearance';renderSettings();delete petTest.releaseList;petTest.holdList=true;state.generalTab='desktop';renderSettings();});
    await race.waitForFunction(()=>petTest.releaseList);await race.locator('[data-pet-select="fixture-1"]').click();
    await race.waitForFunction(()=>petTest.selection.pet.libraryId==='fixture-1');await race.evaluate(()=>petTest.releaseList());
    await race.waitForFunction(()=>document.querySelector('[data-pet-select="fixture-1"]').getAttribute('aria-pressed')==='true'&&!document.querySelector('#import-pet').disabled);
    assert.equal(await race.locator('[data-pet-select="imported"]').getAttribute('aria-pressed'),'false','a list captured before selecting cannot restore the previous pet');
    await race.evaluate(()=>{state.generalTab='appearance';renderSettings();petTest.selection.pet=petTest.pets.find(p=>p.libraryId==='fixture-2');state.generalTab='desktop';renderSettings();});
    await race.waitForFunction(()=>document.querySelector('[data-pet-select="fixture-2"]').getAttribute('aria-pressed')==='true');
    assert.match(await race.locator('#current-pet').textContent(),/桌宠 2/,'reopening desktop settings reloads an externally changed selection');
    assert.equal(await race.locator('#pet-import-status').textContent(),'','an external selection clears the previous selection success text');await race.close();
    assert.deepEqual(errors,[]);console.log('Pet UI: variable frame timing, interrupted idle, continuing loops, v1/v2 grids, visible-only library assets/playback, reduced motion, scale/DPI geometry, alpha edges, panel/keyboard/drag, import/select recovery and library preservation passed');
  }finally{await browser.close();await new Promise(r=>server.close(r));}
})().catch(e=>{console.error(e);process.exitCode=1;});
