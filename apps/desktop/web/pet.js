/* Sprite rendering only; the core owns agent-state priority and animation choice. */
(() => {
  const sprite=document.querySelector('#pet-sprite'), ball=document.querySelector('#ball');
  const reduced=matchMedia('(prefers-reduced-motion: reduce)');
  let state=null, start=0, timer=null, enabled=false;
  function frame(s,elapsed,still=false){
    if(still)return s.row*8;
    const cycle=s.frames*s.frameMs;
    if(s.row!==0&&elapsed>=cycle*3)return Math.floor((elapsed-cycle*3)/900)%6;
    return s.row*8+Math.floor(elapsed/s.frameMs)%s.frames;
  }
  function paint(){
    if(!state||!sprite)return;
    const index=frame(state,Math.max(0,performance.now()-start),reduced.matches);
    sprite.style.backgroundPosition=`${index%8/7*100}% ${Math.floor(index/8)/8*100}%`;
    sprite.dataset.frame=String(index);
  }
  function schedule(){
    clearInterval(timer);timer=null;paint();
    if(enabled&&!document.hidden&&!reduced.matches)timer=setInterval(paint,100);
  }
  function update(info){
    enabled=!!info.petEnabled&&!info.nativeCapsule&&!info.panelOpen;
    document.body.dataset.pet=String(!!info.petEnabled);
    if(info.pet){if(state?.key!==info.pet.key)start=performance.now();state=info.pet;}
    if(state&&info.petEnabled){
      document.body.dataset.petBehavior=state.behavior;
      const phase={attention:'needsInput',blocked:'failed',success:'complete',rest:'interrupted',running:'working',working:'tool',thinking:'thinking',unknown:'unknown',idle:'idle'}[state.behavior]||'unknown';
      const label=info.recovery==='failed'?'恢复失败':info.recovery==='refreshing'?'恢复中':state.label;
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
