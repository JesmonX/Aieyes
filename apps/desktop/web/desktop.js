(() => {
  let current = null;
  const phases = {working:['◉','进行中'],thinking:['✦','思考中'],tool:['⚙','执行工具'],complete:['✓','已完成'],interrupted:['Ⅱ','已中断'],unknown:['?','状态待确认']};
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  function update(info) {
    current = info;
    document.querySelector('#session-summary').textContent = info.summary;
    document.querySelector('#session-list').innerHTML = info.sessions.map(s => {
      const [symbol,label] = phases[s.phase] || phases.unknown;
      return `<div class="live-row"><span class="phase-mark" data-phase="${escape(s.phase)}" aria-hidden="true">${symbol}</span><div><strong>${escape(s.source || 'Codex')}</strong><small>会话 ${escape(s.id.slice(0,8))} · ${new Date(s.updatedAt*1000).toLocaleTimeString('zh-CN')}</small></div><span>${label}</span></div>`;
    }).join('') || '<p class="muted">暂无活跃会话</p>';
    document.querySelector('#session-warning').hidden = !info.unavailable;
    const mode = document.querySelector('#desktop-mode');
    if (mode && document.activeElement !== mode) mode.value = info.mode;
    const effective = document.querySelector('#desktop-effective');
    if (effective) effective.textContent = effectiveText(info);
  }
  function effectiveText(info) {
    return info.reason || `当前使用${info.effectiveMode === 'floating' ? '悬浮球' : '系统状态栏'}。`;
  }
  window.AieyesDesktop = {
    settingsHTML() {
      return `<div class="card"><h2>桌面显示</h2><div class="form-row"><label for="desktop-mode">显示方式</label><select id="desktop-mode"><option value="auto">跟随系统（Windows 悬浮球 / Linux 状态栏）</option><option value="floating">悬浮球</option><option value="tray">系统状态栏 / 托盘</option></select></div><p class="muted tiny" id="desktop-effective">${escape(current ? effectiveText(current) : '正在检测桌面…')}</p><p class="muted tiny">悬浮球可拖动，单击打开概览，右键打开菜单。关闭详情窗口后继续监测会话；不支持状态栏时自动使用悬浮球。</p><div class="between"><button type="button" id="reset-ball">重置悬浮球位置</button><button type="button" id="quit-app">退出 Aieyes</button></div></div>`;
    },
    bindSettings() {
      const select = document.querySelector('#desktop-mode');
      if (!select) return;
      select.value = current?.mode || 'auto';
      select.onchange = async () => {
        select.disabled = true;
        try { update(await invoke('desktop_mode',{mode:select.value})); }
        catch (error) { select.value = current?.mode || 'auto'; notify(String(error)); }
        finally { select.disabled = false; }
      };
      document.querySelector('#reset-ball').onclick = () => invoke('desktop_action',{action:'reset-position'}).catch(e=>notify(String(e)));
      document.querySelector('#quit-app').onclick = () => invoke('desktop_action',{action:'quit'}).catch(e=>notify(String(e)));
    },
  };
  async function boot() {
    if (!window.__TAURI__) return;
    try {
      await window.__TAURI__.event.listen('desktop:status', event => update(event.payload));
      await window.__TAURI__.event.listen('desktop:error', event => notify(String(event.payload)));
      await window.__TAURI__.event.listen('desktop:navigate', event => {
        state.page = event.payload === 'settings' ? 'settings' : 'agent';
        if (state.page === 'settings') state.settingsTab = 'general';
        render();
      });
      const info = await invoke('desktop_info');
      update(info);
      if (info.page === 'settings') { state.page = 'settings'; state.settingsTab = 'general'; render(); }
    } catch (error) { notify(String(error)); }
  }
  boot();
})();
