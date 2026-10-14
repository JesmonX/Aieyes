/* Live session rows, shared by the details window and the floating panel so both
   surfaces stay identical. Fills the static #live-sessions markup of either page. */
(() => {
  const phases = Object.fromEntries(Object.entries({idle:'空闲',needsInput:'等待输入',failed:'执行失败',working:'进行中',thinking:'思考中',tool:'执行工具',complete:'已完成',interrupted:'已中断',unknown:'状态待确认'}).map(([key,label])=>[key,[uiIcon(key),label]]));
  const escape = value => window.AieyesUI.escapeHTML(value);
  globalThis.renderLiveSessions = info => {
    const summary = document.querySelector('#session-summary');
    const list = document.querySelector('#session-list');
    const warning = document.querySelector('#session-warning');
    if (!summary || !list || !warning) return;
    summary.textContent = info.summary;
    const indicator=document.querySelector('#session-indicator');
    if(indicator){const phase=info.unavailable?'unknown':Object.hasOwn(phases,info.phase)?info.phase:'idle';indicator.dataset.phase=phase;indicator.innerHTML=phases[phase][0];}

    list.innerHTML = info.sessions.map(s => {
      const [symbol, label] = phases[s.phase] || phases.unknown;
      return `<div class="live-row"><span class="phase-mark" data-phase="${escape(s.phase)}" aria-hidden="true">${symbol}</span><div><strong>${escape(({codex:'Codex',claude:'Claude Code',antigravity:'AGY'}[s.provider]||s.source||'Agent')+' · '+(s.machine||'本机'))}</strong><small>会话 ${escape(s.id.slice(0,8))} · ${new Intl.RelativeTimeFormat(undefined,{numeric:'auto'}).format(-Math.max(0,Math.round((Date.now()/1000-s.updatedAt)/60)),'minute')}</small></div><span>${label}</span></div>`;
    }).join('') || '<p class="muted">暂无活跃会话</p>';
    const failures=(info.sources||[]).filter(s=>s.state!=='healthy');
    warning.hidden = !failures.length;
    warning.textContent=failures.map(s=>`${s.provider} · ${s.machineId==='local'?'本机':s.machineId}：${s.error||'正在连接'}`).join('；');
    let notice=document.querySelector('#session-attention');
    const uncertain=info.unavailable || info.sessions.some(s=>s.phase==='unknown');
    if(uncertain){
      if(!notice){notice=document.createElement('div');notice.id='session-attention';document.querySelector('#live-sessions').after(notice);}
      notice.className='attention';notice.innerHTML=uiIcon('attention')+'<span>待确认：会话状态可能不完整</span><button type="button">重试读取</button>';
      notice.querySelector('button').onclick=async()=>{const button=notice.querySelector('button');button.disabled=true;try{renderLiveSessions(await window.__TAURI__.core.invoke('desktop_info'));}catch(error){button.disabled=false;notice.querySelector('span').textContent=String(error);}};
    }else notice?.remove();
  };
})();
