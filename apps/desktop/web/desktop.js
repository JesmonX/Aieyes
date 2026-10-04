(() => {
  let current = null;
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  function update(info) {
    current = info;
    document.body.dataset.platform = info.platform;
    document.documentElement.dataset.material = info.material || 'opaque';
    renderLiveSessions(info);
    const mode = document.querySelector('#desktop-mode');
    if (mode && document.activeElement !== mode) mode.value = info.mode;
    const effective = document.querySelector('#desktop-effective');
    if (effective) effective.textContent = effectiveText(info);
    const hide = document.querySelector('#hide-ball');
    if (hide && document.activeElement !== hide) hide.textContent = info.hidden ? '恢复显示悬浮球' : '暂时隐藏悬浮球';
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
      return `<div class="card"><h2>桌面显示</h2><div class="form-row"><label for="desktop-mode">显示方式</label><select id="desktop-mode"><option value="auto">跟随系统</option><option value="floating">悬浮球</option><option value="tray">系统状态栏 / 托盘</option></select></div><p class="muted tiny" id="desktop-effective">${escape(current ? effectiveText(current) : '正在检测桌面…')}</p><p class="muted tiny">悬浮球单击或悬停打开面板，拖动松手后自动贴边；隐藏后可从托盘菜单恢复，重启应用也会恢复。</p><div class="between"><button type="button" id="reset-ball">重置悬浮球位置</button><button type="button" id="hide-ball">暂时隐藏悬浮球</button><button type="button" id="quit-app">退出 Aieyes</button></div></div>`;
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
      const hide = document.querySelector('#hide-ball');
      if (hide) {
        hide.textContent = current?.hidden ? '恢复显示悬浮球' : '暂时隐藏悬浮球';
        hide.onclick = () => invoke('desktop_action',{action: current?.hidden ? 'show-ball' : 'hide-ball'}).catch(e=>notify(String(e)));
      }
      document.querySelector('#quit-app').onclick = () => invoke('desktop_action',{action:'quit'}).catch(e=>notify(String(e)));
    },
  };
  async function boot() {
    if (!window.__TAURI__) return;
    try {
      await window.__TAURI__.event.listen('desktop:status', event => update(event.payload));
      await window.__TAURI__.event.listen('desktop:error', event => notify(String(event.payload)));
      await window.__TAURI__.event.listen('desktop:navigate', event => {
        const page = event.payload;
        state.page = page === 'settings' || page === 'prices' ? 'settings' : 'agent';
        if (state.page === 'settings') state.settingsTab = page === 'prices' ? 'prices' : 'general';
        render();
        if (page === 'prices') api('prices.list').then(prices => { state.prices = prices; render(); }).catch(e => notify(String(e)));
      });
      const info = await invoke('desktop_info');
      update(info);
      await bindTitlebar(info);
      if (info.page === 'settings' || info.page === 'prices') {
        state.page = 'settings';
        state.settingsTab = info.page === 'prices' ? 'prices' : 'general';
        if (info.page === 'prices') state.prices = await api('prices.list');
        render();
      }
    } catch (error) { notify(String(error)); }
  }
  boot();
})();
