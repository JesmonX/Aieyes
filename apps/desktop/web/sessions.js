/* Live session rows, shared by the details window and the floating panel so both
   surfaces stay identical. Fills the static #live-sessions markup of either page. */
(() => {
  const mark = path => `<svg viewBox="0 0 24 24"><path d="${path}"/></svg>`;
  const phases = {
    working:[mark('M8 5v14l11-7Z'),'进行中'], thinking:[mark('M12 3 9 9 3 12l6 3 3 6 3-6 6-3-6-3Z'),'思考中'],
    tool:[mark('m8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18'),'执行工具'], complete:[mark('m5 12 4 4L19 6'),'已完成'],
    interrupted:[mark('M8 5v14M16 5v14'),'已中断'], unknown:[mark('M9 8a3 3 0 1 1 5 2c-2 1-2 2-2 4m0 4h.01'),'状态待确认'],
  };
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  globalThis.renderLiveSessions = info => {
    const summary = document.querySelector('#session-summary');
    const list = document.querySelector('#session-list');
    const warning = document.querySelector('#session-warning');
    if (!summary || !list || !warning) return;
    summary.textContent = info.summary;
    list.innerHTML = info.sessions.map(s => {
      const [symbol, label] = phases[s.phase] || phases.unknown;
      return `<div class="live-row"><span class="phase-mark" data-phase="${escape(s.phase)}" aria-hidden="true">${symbol}</span><div><strong>${escape(s.source || 'Codex')}</strong><small>会话 ${escape(s.id.slice(0,8))} · ${new Date(s.updatedAt*1000).toLocaleTimeString('zh-CN')}</small></div><span>${label}</span></div>`;
    }).join('') || '<p class="muted">暂无活跃会话</p>';
    warning.hidden = !info.unavailable;
  };
})();
