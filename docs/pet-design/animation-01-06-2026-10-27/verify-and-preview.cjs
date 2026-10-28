
const fs=require('fs'),path=require('path'),assert=require('assert'),vm=require('vm'),crypto=require('crypto');
const sharp=require('../../../apps/desktop/node_modules/sharp');
const root=__dirname,report=JSON.parse(fs.readFileSync(path.join(root,'assembly-report.json')));
const contract=JSON.parse(fs.readFileSync(path.resolve(root,'../../core-contracts.json')));
const clock=fs.readFileSync(path.resolve(root,'../../../apps/desktop/web/pet-animation.js'),'utf8');
const sandbox={window:{AieyesContract:contract}};vm.runInNewContext(clock,sandbox);
const result={checkedAt:new Date().toISOString(),checks:[],characters:[]};
(async()=>{
for(const pet of report.characters){
 const p=path.join(root,pet.id,'spritesheet.png'),meta=await sharp(p).metadata();
 assert.equal(meta.width,1536);assert.equal(meta.height,1872);assert(meta.hasAlpha);
 let active=0,blank=0;const hashes=new Set();
 const {data,info}=await sharp(p).raw().toBuffer({resolveWithObject:true});
 for(let row=0;row<9;row++){
  const expected=contract.petFrameDurations[row];assert.equal(pet.rows[row].frames,expected.length);
  let boundary=0;
  for(let c=0;c<expected.length;c++){assert.equal(sandbox.window.AieyesPetAnimation.frame({row,frames:expected.length,frameDurationsMs:expected},boundary),row*8+c);boundary+=expected[c];}
  assert.equal(sandbox.window.AieyesPetAnimation.frame({row,frames:expected.length,frameDurationsMs:expected},boundary),row*8);
  assert.equal(sandbox.window.AieyesPetAnimation.frame({row,frames:expected.length,frameDurationsMs:expected},123,true),row*8);
  for(let col=0;col<8;col++){
   let count=0,edge=0;for(let y=0;y<208;y++)for(let x=0;x<192;x++){
    const a=data[((row*208+y)*1536+col*192+x)*4+3];
    if(a){count++;if(x<4||x>=188||y<4||y>=204)edge++;}
   }
   if(col<expected.length){assert(count>500);assert.equal(edge,0,pet.id+' edge '+row+':'+col);active++;
    const f=pet.rows[row].items[col];const hash=crypto.createHash('sha256').update(fs.readFileSync(path.join(root,pet.id,f.file))).digest('hex');assert(!hashes.has(hash),'duplicate frame');hashes.add(hash);
   }else{assert.equal(count,0);blank++;}
  }
 }
 assert.equal(active,57);assert.equal(blank,15);
 result.characters.push({id:pet.id,width:meta.width,height:meta.height,channels:meta.channels,validFrames:active,transparentUnusedCells:blank,uniqueFrameHashes:hashes.size,sizeBytes:fs.statSync(p).size});
}
result.checks=['exact 1536x1872 RGBA','8 columns x 9 rows','192x208 cells','57 nonempty unique frames per character','15 transparent unused cells per character','no alpha in 4px cell boundary','row counts match live core-contracts.json','actual production JS frame clock at every boundary and loop wrap','reduced-motion uses first frame of each action'];
fs.writeFileSync(path.join(root,'validation.json'),JSON.stringify(result,null,2)+'\n');
const images=Object.fromEntries(report.characters.map(p=>[p.id,'data:image/png;base64,'+fs.readFileSync(path.join(root,p.id,'spritesheet.png')).toString('base64')]));
const html=`<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>Aieyes · 01 / 06 动作验收</title><style>
*{box-sizing:border-box}body{margin:0;background:#f5f3ef;color:#242c38;font:14px -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}main{max-width:1180px;margin:auto;padding:28px}header{display:flex;align-items:start;justify-content:space-between;gap:24px}h1{margin:5px 0 8px;font-size:30px;letter-spacing:-1px}p{color:#667080;margin:7px 0;line-height:1.6}.eyebrow{font-size:11px;letter-spacing:3px;color:#647991}.tools{position:sticky;top:0;z-index:2;padding:12px 0;background:#f5f3efee;display:flex;gap:12px;align-items:center;flex-wrap:wrap}button,select,input{font:inherit}button,select{background:white;border:1px solid #d6dae1;border-radius:8px;padding:7px 12px;color:inherit}button{cursor:pointer}button[aria-pressed=true]{background:#293f62;color:white}.head,.row{display:grid;grid-template-columns:220px 1fr 1fr;gap:12px}.head{padding:12px 0;font-weight:650}.row{margin:10px 0;align-items:stretch}.label{padding:15px 8px}.label b{display:block;margin-bottom:5px}.label small{color:#75808d;line-height:1.8}.pair{border:1px solid #e0e3e6;border-radius:12px;overflow:hidden;display:grid;grid-template-columns:1fr 1fr}.stage{display:flex;min-height:136px;align-items:center;justify-content:center;position:relative;background:#fff}.stage.dark{background:#202632}.stage span{position:absolute;bottom:8px;left:10px;font-size:10px;color:#7b8491}.stage.dark span{color:#a6afbf}canvas{width:96px;height:104px;display:block}.cellInfo{font-size:10px;color:#8994a1}.foot{margin-top:25px;padding:14px;border-top:1px solid #d9dde2}a{color:#365b91}.zoom canvas{width:144px;height:156px}.zoom .stage{min-height:178px}@media(max-width:800px){main{padding:14px}.head,.row{grid-template-columns:130px 1fr 1fr;gap:6px}.stage{min-height:118px}.pair{grid-template-columns:1fr}.stage.dark{display:none}h1{font-size:24px}}
</style><main><header><div><div class="eyebrow">AIEYES · MOTION STUDY 01 / 06</div><h1>两位伙伴，九组动作</h1><p>同一状态、同一时钟 · 每只 57 帧 · 浅色与深色桌面对照</p></div><div><p>标准图集 1536 × 1872<br>每格 192 × 208 · 透明 PNG</p></div></header>
<div class="tools"><button id="play" aria-pressed="true">暂停</button><label>速度 <select id="speed"><option value="1">1× 实际时序</option><option value=".5">0.5× 慢放</option><option value=".25">0.25× 逐帧观察</option></select></label><button id="zoom">放大至 144 px</button><button id="still">减少动态效果</button><label>时间 <input id="scrub" type="range" min="0" max="2000" value="0" style="width:160px"></label><span id="time" class="cellInfo"></span></div>
<div class="head"><div>会话状态 / 图集行</div><div>01 · 折耳巡游兽</div><div>06 · 墨滴工匠</div></div><section id="rows"></section><div class="foot"><p>工作中使用右跑；左跑为 Codex 图集完整性保留。已中断复用 idle。所有状态依 Aieyes 当前逐帧时长循环。</p><p>暂停后可拖动时间滑块检查帧；“减少动态效果”显示该动作第一帧。此预览验证图像、分格和播放器时序，不代表原生桌面输入验收。</p></div></main>
<script>window.AieyesContract=${JSON.stringify({petFrameDurations:contract.petFrameDurations})};</script><script>${clock}</script><script>
const pets=${JSON.stringify(report.characters.map(p=>({id:p.id,name:p.displayName,rows:p.rows.map(r=>({row:r.row,action:r.action,label:r.label,frames:r.frames,frameDurationsMs:r.frameDurationsMs,cycleMs:r.cycleMs}))})))};
const sources=${JSON.stringify(images)};const imgs={},views=[];let elapsed=0,last=0,playing=true,still=false,speed=1;window.previewReady=false;
for(let r=0;r<9;r++){let row=document.createElement('div');row.className='row';const d=pets[0].rows[r];row.innerHTML='<div class="label"><b>'+d.label+'</b><small>R'+r+' · '+d.action+'<br>'+d.frames+' 帧 · '+d.cycleMs+' ms / 轮</small></div>';for(const p of pets){const pair=document.createElement('div');pair.className='pair';for(const dark of [false,true]){const stage=document.createElement('div');stage.className='stage'+(dark?' dark':'');const c=document.createElement('canvas');c.width=192;c.height=208;const tag=document.createElement('span');stage.append(c,tag);pair.append(stage);views.push({id:p.id,state:p.rows[r],canvas:c,ctx:c.getContext('2d'),tag});}row.append(pair);}document.querySelector('#rows').append(row);}
function render(){for(const v of views){const f=AieyesPetAnimation.frame(v.state,elapsed,still);v.ctx.clearRect(0,0,192,208);v.ctx.drawImage(imgs[v.id],f%8*192,Math.floor(f/8)*208,192,208,0,0,192,208);v.tag.textContent='F'+(f%8+1)+' / '+v.state.frames;v.canvas.dataset.frame=f;}document.querySelector('#time').textContent=Math.round(elapsed)+' ms';}
window.setPreviewTime=(t)=>{playing=false;elapsed=t;render();};
Promise.all(pets.map(p=>new Promise((resolve,reject)=>{const im=new Image;im.onload=()=>{imgs[p.id]=im;resolve()};im.onerror=reject;im.src=sources[p.id]}))).then(()=>{window.previewReady=true;render();requestAnimationFrame(function loop(t){if(playing)elapsed+=last?(t-last)*speed:0;last=t;render();requestAnimationFrame(loop)});});
document.querySelector('#play').onclick=e=>{playing=!playing;e.target.textContent=playing?'暂停':'播放';e.target.setAttribute('aria-pressed',String(playing));};
document.querySelector('#speed').onchange=e=>speed=+e.target.value;
document.querySelector('#zoom').onclick=e=>{document.body.classList.toggle('zoom');e.target.textContent=document.body.classList.contains('zoom')?'恢复 96 px':'放大至 144 px';};
document.querySelector('#still').onclick=e=>{still=!still;e.target.setAttribute('aria-pressed',String(still));render();};
document.querySelector('#scrub').oninput=e=>{playing=false;elapsed=+e.target.value;document.querySelector('#play').textContent='播放';render();};
</script></html>`;
fs.writeFileSync(path.join(root,'preview.html'),html);
console.log(JSON.stringify(result,null,2));
})().catch(e=>{console.error(e);process.exitCode=1});
