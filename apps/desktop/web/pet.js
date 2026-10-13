/* Sprite rendering only; the core owns agent-state priority and animation choice. */
(() => {
  const sprite=document.querySelector('#pet-sprite'), ball=document.querySelector('#ball');
  const reduced=matchMedia('(prefers-reduced-motion: reduce)');
  let state=null, start=0, timer=null, enabled=false, rows=9, assetKey=null, loading=null;
  function frame(s,elapsed,still=false){
    if(still)return s.row*8;
    const cycle=s.frames*s.frameMs;
    if(s.row!==0&&elapsed>=cycle*3)return Math.floor((elapsed-cycle*3)/900)%6;
    return s.row*8+Math.floor(elapsed/s.frameMs)%s.frames;
  }
  function paint(){
    if(!state||!sprite)return;
    const index=frame(state,Math.max(0,performance.now()-start),reduced.matches);
    sprite.style.backgroundPosition=`${index%8/7*100}% ${Math.floor(index/8)/(rows-1)*100}%`;
    sprite.dataset.frame=String(index);
  }
  function schedule(){
    clearInterval(timer);timer=null;paint();
    if(enabled&&!document.hidden&&!reduced.matches)timer=setInterval(paint,100);
  }
  async function loadAsset(asset){
    const key=asset?.spritePath;
    if(!key){loading=null;assetKey=null;if(sprite)sprite.style.backgroundImage='none';return;}
    if(key===assetKey||key===loading)return;
    loading=key;
    try {
      const result=await api('pets.current',{includeImage:true});
      if(loading!==key)return;
      if(result.error)throw new Error(result.error);
      rows=result.pet.rows;assetKey=result.pet.spritePath;
      sprite.style.backgroundImage=`url("${result.imageDataUrl}")`;
      sprite.style.backgroundSize=`800% ${rows*100}%`;
      paint();
    } catch(error) { if(sprite)sprite.style.backgroundImage='none'; }
    finally { if(loading===key)loading=null; }
  }
  function update(info){
    if(!info.nativePet)void loadAsset(info.petAsset);
    enabled=!!info.petEnabled&&!info.nativePet&&!info.panelOpen;
    document.body.dataset.pet=String(!!info.petEnabled);
    if(info.pet){if(state?.key!==info.pet.key)start=performance.now();state=info.pet;}
    if(state&&info.petEnabled){
      document.body.dataset.petBehavior=state.behavior;
      const phase={attention:'needsInput',blocked:'failed',success:'complete',rest:'interrupted',running:'working',working:'tool',thinking:'thinking',unknown:'unknown',idle:'idle'}[state.behavior]||'unknown';
      const label=info.petError?'请导入桌宠':info.recovery==='failed'?'恢复失败':info.recovery==='refreshing'?'恢复中':state.label;
      document.body.style.setProperty('--phase-color',`var(--phase-${info.recovery==='failed'?'failed':phase})`);
      document.querySelector('#phase').textContent=label;
      ball.title=`Aieyes · ${label}\n单击打开面板 · 拖动移动 · 右键菜单`;
      ball.setAttribute('aria-label',ball.title);
    }
    schedule();
  }
  reduced.addEventListener('change',schedule);document.addEventListener('visibilitychange',schedule);
  window.addEventListener('pagehide',()=>{clearInterval(timer);timer=null;});
  window.AieyesPet={update,frame};
})();
