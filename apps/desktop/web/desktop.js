(() => {
  let current = null;
  const mark = path => `<svg viewBox="0 0 24 24"><path d="${path}"/></svg>`;
  const phases = {
    working:[mark('M8 5v14l11-7Z'),'进行中'], thinking:[mark('M12 3 9 9 3 12l6 3 3 6 3-6 6-3-6-3Z'),'思考中'],
    tool:[mark('m8 6-6 6 6 6m8-12 6 6-6 6m-3-15-2 18'),'执行工具'], complete:[mark('m5 12 4 4L19 6'),'已完成'],
    interrupted:[mark('M8 5v14M16 5v14'),'已中断'], unknown:[mark('M9 8a3 3 0 1 1 5 2c-2 1-2 2-2 4m0 4h.01'),'状态待确认'],
  };
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  function update(info) {
    current = info;
    document.body.dataset.platform = info.platform;
    document.documentElement.dataset.material = info.material || 'opaque';
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
  async function bindTitlebar(info) {
    if (info.platform !== 'windows') return;
    const titlebar = document.querySelector('#titlebar');
    titlebar.hidden = false;
    const native = window.__TAURI__.window.getCurrentWindow();
    const maximize = document.querySelector('#window-maximize');
    async function updateMaximized() {
      const maximized = await native.isMaximized();
      const label = maximized ? '还原' : '最大化';
      maximize.setAttribute('aria-label', label); maximize.title = label;
      maximize.innerHTML = maximized ? '<svg viewBox="0 0 16 16"><path d="M6 3h7v7M3 6h7v7H3Z"/></svg>' : '<svg viewBox="0 0 16 16"><rect x="3.5" y="3.5" width="9" height="9"/></svg>';
    }
    document.querySelector('#window-minimize').onclick = () => native.minimize().catch(e=>notify(String(e)));
    maximize.onclick = () => native.toggleMaximize().then(updateMaximized).catch(e=>notify(String(e)));
    document.querySelector('#window-close').onclick = () => native.close().catch(e=>notify(String(e)));
    await native.onResized(() => updateMaximized().catch(e=>notify(String(e))));
    await updateMaximized();
  }
  window.AieyesDesktop = {
    settingsHTML() {
      return `<div class="card"><h2>桌面显示</h2><div class="form-row"><label for="desktop-mode">显示方式</label><select id="desktop-mode"><option value="auto">跟随系统</option><option value="floating">悬浮球</option><option value="tray">系统状态栏 / 托盘</option></select></div><p class="muted tiny" id="desktop-effective">${escape(current ? effectiveText(current) : '正在检测桌面…')}</p><div class="between"><button type="button" id="reset-ball">重置悬浮球位置</button><button type="button" id="quit-app">退出 Aieyes</button></div></div>`;
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
      await bindTitlebar(info);
      if (info.page === 'settings') { state.page = 'settings'; state.settingsTab = 'general'; render(); }
    } catch (error) { notify(String(error)); }
  }
  boot();
})();
