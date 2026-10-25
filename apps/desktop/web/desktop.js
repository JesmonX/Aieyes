(() => {
  let current = null;
  let scalePending = null,scaleSaving = false,scaleTimer = null;
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const escape = (value) => window.AieyesUI.escapeHTML(value);
  function update(info) {
    current = info;
    window.AieyesAgentSettings?.updateSession(info);
    document.body.dataset.platform = info.platform;
    document.documentElement.dataset.platform = info.platform;
    document.documentElement.dataset.material = info.materials?.main || info.material || 'opaque';
    window.AieyesTheme?.loadFonts?.();
    renderLiveSessions(info);
    const mode = document.querySelector('#desktop-mode');
    if (mode && document.activeElement !== mode) mode.value = info.mode;
    const scale = document.querySelector('#pet-scale');
    if (scale && document.activeElement !== scale && !scaleSaving && scalePending === null) showScale(info.petScale ?? 1);
    const effective = document.querySelector('#desktop-effective');
    if (effective) effective.textContent = effectiveText(info);
    const count = document.querySelector('#desktop-count');if (count) count.checked = !!info.showCount;
    const hide = document.querySelector('#hide-ball');
    if (hide && document.activeElement !== hide) hide.textContent = info.hidden ? AieyesI18n.t("恢复显示桌宠") : AieyesI18n.t("暂时隐藏桌宠");
  }
  function effectiveText(info) {
    return AieyesI18n.message(info.reason) || AieyesI18n.t("当前使用{0}。", info.effectiveMode === 'pet' ? AieyesI18n.t("桌面宠物") : AieyesI18n.t("系统状态栏"));
  }
  function showScale(value) {
    const slider = document.querySelector('#pet-scale'),output = document.querySelector('#pet-scale-value'),reset = document.querySelector('#reset-pet-scale');
    if (slider) slider.value = String(value);
    if (output) output.textContent = Math.round(value * 100) + '%';
    if (reset) reset.disabled = value === 1;
  }
  async function saveScale() {
    if (scaleSaving || scalePending === null) return;
    scaleSaving = true;
    try {
      while (scalePending !== null) {
        const value = scalePending;scalePending = null;
        const info = await invoke('desktop_pet_scale', { scale: value });
        update(info);
      }
    } catch (error) {scalePending = null;showScale(current?.petScale ?? 1);notify(String(error), 'error');} finally
    {scaleSaving = false;}
  }
  function changeScale(value) {
    showScale(value);scalePending = value;clearTimeout(scaleTimer);scaleTimer = setTimeout(saveScale, 75);
  }
  async function navigate(page) {
    if (page === 'sampling') {await window.AieyesApp.ready;await loadDashboard();openSampling();return;}
    if (page.startsWith('settings-target:')) {try {await window.AieyesApp.openSettings(JSON.parse(page.slice(16)));} catch (error) {notify(String(error), 'error');}return;}
    if (page.startsWith("edit-account:")) {await window.AieyesApp.ready;await window.AieyesApp.openSettings({ accountKey: page.slice(13) });return;}
    if (page === 'add-source' || page === 'add-host' || page === 'add-quota') {
      await window.AieyesApp.openSetupInMain(page === 'add-host' ? 'hosts' : page === 'add-quota' ? 'quota' : 'sources');
      return;
    }
    if (page === 'settings' || page === 'prices') {await window.AieyesApp.openSettings(page === 'prices' ? { category: 'prices' } : { category: 'general' });return;}
    await window.AieyesApp.navigatePage(['analysis', 'servers'].includes(page) ? page : 'agent');
  }
  async function bindTitlebar(info) {
    if (info.platform !== 'windows') return;
    const titlebar = document.querySelector('#titlebar');
    titlebar.hidden = false;
    const native = window.__TAURI__.window.getCurrentWindow();
    const maximize = document.querySelector('#window-maximize');
    async function updateMaximized() {
      const maximized = await native.isMaximized();
      document.documentElement.dataset.maximized = String(maximized);
      const label = maximized ? AieyesI18n.t("还原") : AieyesI18n.t("最大化");
      maximize.setAttribute('aria-label', label);maximize.title = label;
      maximize.innerHTML = maximized ? '<svg viewBox="0 0 16 16"><path d="M6 3h7v7M3 6h7v7H3Z"/></svg>' : '<svg viewBox="0 0 16 16"><rect x="3.5" y="3.5" width="9" height="9"/></svg>';
    }
    document.querySelector('#window-minimize').onclick = () => native.minimize().catch((e) => notify(String(e)));
    // Tauri's drag-region handler owns double-click maximize; onResized updates the icon.
    maximize.onclick = () => native.toggleMaximize().then(updateMaximized).catch((e) => notify(String(e)));
    document.querySelector('#window-close').onclick = async () => {if (window.AieyesAgentSettings && !(await window.AieyesAgentSettings.flush())) return;await native.close().catch((e) => notify(String(e)));};
    await native.onResized(() => updateMaximized().catch((e) => notify(String(e))));
    await updateMaximized();
  }
  window.AieyesDesktop = {
    settingsHTML() {
      return `<div class="card"><h2>${AieyesI18n.t("桌面显示")}</h2><div class="form-row"><label for="desktop-mode">${AieyesI18n.t("显示方式")}</label><select id="desktop-mode"><option value="pet">${AieyesI18n.t("桌面宠物")}</option><option value="tray">${AieyesI18n.t("系统状态栏 / 托盘")}</option></select></div><div class="form-row pet-scale-control"><label for="pet-scale">${AieyesI18n.t("桌宠缩放")}</label><input id="pet-scale" type="range" min="0.5" max="2" step="0.05" value="1"><output id="pet-scale-value" for="pet-scale">100%</output><button type="button" id="reset-pet-scale">${AieyesI18n.t("恢复 100%")}</button></div><label class="quota-confirm"><input type="checkbox" role="switch" id="desktop-count">${AieyesI18n.t("显示活跃会话数")}</label><p class="muted tiny" id="desktop-effective">${escape(current ? effectiveText(current) : AieyesI18n.t("正在检测桌面…"))}</p>${helpHTML(AieyesI18n.t("桌面显示"), AieyesI18n.t("单击打开面板，悬停查看状态，拖动松手后贴边。隐藏后可从托盘菜单恢复。"))}${window.AieyesPetLibrary?.html() ?? ''}<div class="between"><button type="button" id="reset-ball">${AieyesI18n.t("重置桌宠位置")}</button><button type="button" id="hide-ball">${AieyesI18n.t("暂时隐藏桌宠")}</button><button type="button" id="quit-app">${AieyesI18n.t("退出 Aieyes")}</button></div></div>`;
    },
    bindSettings() {
      window.AieyesPetLibrary?.bind();
      const select = document.querySelector('#desktop-mode');
      if (!select) return;
      select.value = current?.mode || 'pet';
      const scale = document.querySelector('#pet-scale');showScale(current?.petScale ?? 1);
      scale.oninput = () => changeScale(Number(scale.value));
      scale.onchange = () => {clearTimeout(scaleTimer);void saveScale();};
      document.querySelector('#reset-pet-scale').onclick = () => {changeScale(1);clearTimeout(scaleTimer);void saveScale();};
      const count = document.querySelector('#desktop-count');if (count) {count.checked = !!current?.showCount;count.onchange = async () => {count.disabled = true;try {await invoke('desktop_action', { action: 'toggle-count' });update(await invoke('desktop_info'));} catch (error) {count.checked = !!current?.showCount;notify(String(error));} finally {count.disabled = false;}};}
      select.onchange = async () => {
        select.disabled = true;
        try {update(await invoke('desktop_mode', { mode: select.value }));}
        catch (error) {select.value = current?.mode || 'pet';notify(String(error));} finally
        {select.disabled = false;}
      };
      document.querySelector('#reset-ball').onclick = () => invoke('desktop_action', { action: 'reset-position' }).catch((e) => notify(String(e)));
      const hide = document.querySelector('#hide-ball');
      if (hide) {
        hide.textContent = current?.hidden ? AieyesI18n.t("恢复显示桌宠") : AieyesI18n.t("暂时隐藏桌宠");
        hide.onclick = () => invoke('desktop_action', { action: current?.hidden ? 'show-ball' : 'hide-ball' }).catch((e) => notify(String(e)));
      }
      document.querySelector('#quit-app').onclick = () => invoke('desktop_action', { action: 'quit' }).catch((e) => notify(String(e)));
    }
  };
  async function boot() {
    if (!window.__TAURI__) return;
    try {
      await window.__TAURI__.event.listen('desktop:quit-requested', async () => {
        if (document.querySelector('#editor')?.open) {await invoke('desktop_action', { action: 'settings' });notify(AieyesI18n.t("请先完成或放弃当前编辑，再退出应用"), 'error');return;}
        if (window.AieyesAgentSettings && !(await window.AieyesAgentSettings.flush())) return;
        if (settingsFormsDirty()) {await invoke('desktop_action', { action: 'settings' });showEditor(AieyesI18n.t("退出前有未保存更改"), AieyesI18n.t("<p>通用配置或模型映射仍有草稿。可取消返回保存，或明确放弃后退出。</p>"), async () => {await invoke('desktop_action', { action: 'quit-confirmed' });});document.querySelector('#editor-form button[type=submit]').textContent = AieyesI18n.t("放弃更改并退出");document.querySelector('#cancel-editor').focus();} else await invoke('desktop_action', { action: 'quit-confirmed' });
      });
      await window.__TAURI__.event.listen('desktop:status', (event) => update(event.payload));
      await window.__TAURI__.event.listen('desktop:error', (event) => notify(String(event.payload)));
      await window.__TAURI__.event.listen('desktop:navigate', (event) => {
        navigate(event.payload).catch((e) => notify(String(e)));
      });
      await window.__TAURI__.event.listen('desktop:detail-view', async (event) => {
        await window.AieyesApp?.ready;
        const view = event.payload;
        const hasFilters = ['provider', 'sourceId', 'accountKey', 'model', 'days', 'dateRange', 'cost'].some((key) => Object.hasOwn(view, key));
        await window.AieyesApp.navigatePage(['analysis', 'servers'].includes(view.page) ? view.page : 'agent', hasFilters ? view : undefined);
      });
      const info = await invoke('desktop_info');
      update(info);
      await bindTitlebar(info);
      if (info.page !== 'agent') await navigate(info.page);
    } catch (error) {notify(String(error));}
  }
  boot();
})();
