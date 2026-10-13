/* Sprite rendering only; the core owns agent-state priority and animation choice. */
(() => {
  const sprite=document.querySelector('#pet-sprite'), ball=document.querySelector('#ball');
  const reduced=matchMedia('(prefers-reduced-motion: reduce)');
  let state=null, start=0, timer=null, enabled=false, frames=[], assetKey=null, loading=null, scale=1;
  const context=sprite?.getContext('2d',{alpha:true});
  function frame(s,elapsed,still=false){
    if(still)return s.row*8;
    return s.row*8+Math.floor(elapsed/s.frameMs)%s.frames;
  }
  function paint(){
    if(!state||!sprite)return;
    const index=frame(state,Math.max(0,performance.now()-start),reduced.matches);
    const density=(window.devicePixelRatio||1)*scale, width=Math.round(132*density), height=Math.round(143*density);
    if(sprite.width!==width||sprite.height!==height){sprite.width=width;sprite.height=height;}
    context.clearRect(0,0,width,height);
    if(frames[index]){
      context.imageSmoothingEnabled=true;context.imageSmoothingQuality='high';
      context.drawImage(frames[index],0,0,width,height);
    }
    sprite.dataset.frame=String(index);
  }
  function schedule(){
    clearInterval(timer);timer=null;paint();
    if(enabled&&!document.hidden&&!reduced.matches)timer=setInterval(paint,100);
  }
  async function loadAsset(asset){
    const key=asset?.spritePath;
    if(!key){loading=null;assetKey=null;frames=[];paint();return;}
    if(key===assetKey||key===loading)return;
    loading=key;
    try {
      const result=await api('pets.current',{includeImage:true});
      if(loading!==key)return;
      if(result.error)throw new Error(result.error);
      const sheet=new Image();sheet.src=result.imageDataUrl;await sheet.decode();
      if(loading!==key)return;
      const {cellWidth,cellHeight,rows}=result.pet;
      // Crop at native resolution before interpolation. Adjacent atlas frames and
      // invisible RGB pixels must not bleed into a scaled frame's transparent edge.
      frames=Array.from({length:rows*8},(_,index)=>{
        const frame=document.createElement('canvas');frame.width=cellWidth;frame.height=cellHeight;
        frame.getContext('2d').drawImage(sheet,index%8*cellWidth,Math.floor(index/8)*cellHeight,cellWidth,cellHeight,0,0,cellWidth,cellHeight);
        return frame;
      });
      assetKey=result.pet.spritePath;sprite.dataset.asset=assetKey;sprite.dataset.rows=String(rows);
      paint();
    } catch(error) { frames=[];delete sprite.dataset.asset;paint(); }
    finally { if(loading===key)loading=null; }
  }
  function update(info){
    if(!info.nativePet)void loadAsset(info.petAsset);
    enabled=!!info.petEnabled&&!info.nativePet&&!info.panelOpen;
    scale=Number.isFinite(info.petScale)?Math.min(2,Math.max(0.5,info.petScale)):1;
    ball.style.setProperty('--pet-scale',String(scale));
    document.body.dataset.pet=String(!!info.petEnabled);
    if(info.pet){if(state?.key!==info.pet.key)start=performance.now();state=info.pet;}
    if(state&&info.petEnabled){
      document.body.dataset.petBehavior=state.behavior;
      const phase={attention:'needsInput',blocked:'failed',success:'complete',rest:'interrupted',running:'working',working:'tool',thinking:'thinking',unknown:'unknown',idle:'idle'}[state.behavior]||'unknown';
      const label=info.petError?'请导入桌宠':info.recovery==='failed'?'恢复失败':info.recovery==='refreshing'?'恢复中':state.label;
      document.body.style.setProperty('--phase-color',`var(--phase-${info.recovery==='failed'?'failed':phase})`);
      document.querySelector('#phase').textContent=label;
      ball.style.height=((168+18*(label.split('\n').length-1))*scale)+'px';
      ball.title=`Aieyes · ${label}\n单击打开面板 · 拖动移动 · 右键菜单`;
      ball.setAttribute('aria-label',ball.title);
    }
    schedule();
  }
  reduced.addEventListener('change',schedule);document.addEventListener('visibilitychange',schedule);
  window.addEventListener('resize',paint);
  window.addEventListener('pagehide',()=>{clearInterval(timer);timer=null;});
  window.AieyesPet={update,frame};
})();
