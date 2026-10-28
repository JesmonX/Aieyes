(() => {
  const escape = (value) => window.AieyesUI.escapeHTML(value);
  const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);
  let status = { currentVersion: '', phase: 'idle', message: '', automatic: true, prompt: false };
  let dialog = null,submitting = false,preparing = false,cancelled = false,answer = null,waited = '';
  const busy = () => preparing || submitting || ['checking', 'downloading', 'verifying', 'installing'].includes(status.phase);
  const installing = () => ['downloading', 'verifying', 'installing'].includes(status.phase);
  function settingsHTML() {
    return `<div class="card update-settings"><h2>${AieyesI18n.t("应用更新")}</h2><p id="update-current">${AieyesI18n.t("当前版本 {0}", escape(status.currentVersion ? 'v' + status.currentVersion : AieyesI18n.t("读取中…")))}</p><label class="quota-confirm"><input id="updates-automatic" type="checkbox" ${status.automatic ? 'checked' : ''}>${AieyesI18n.t("启动时及每天检查更新")}</label><div class="actions"><button id="updates" ${busy() ? 'disabled' : ''}>${status.phase === 'checking' ? AieyesI18n.t("检查中…") : AieyesI18n.t("检查更新")}</button><button id="update-view" ${status.phase === 'available' ? '' : 'hidden'}>${AieyesI18n.t("查看更新")}</button></div><div id="update-result" role="status">${status.phase === 'error' ? AieyesUI.errorHTML(status.message, AieyesI18n.t("更新未完成")) : escape(AieyesI18n.message(status.message))}</div></div>`;
  }
  function hasUpdate() {
    const parts = (v) => String(v ?? '').replace(/^v/, '').split('.').map(Number),latest = parts(status.latestVersion),current = parts(status.currentVersion);
    return latest.length === 3 && current.length === 3 && latest.every(Number.isFinite) && current.every(Number.isFinite) && latest.some((n, i) => n > current[i] && latest.slice(0, i).every((v, j) => v === current[j]));
  }
  function updateView() {
    if (typeof scheduleRefreshes === 'function') scheduleRefreshes();
    const panel = document.querySelector('#panel-update');
    if (panel) {panel.disabled = status.phase === 'checking';panel.title = hasUpdate() ? AieyesI18n.t("发现新版本 · 查看更新") : status.message || AieyesI18n.t("检查更新");panel.setAttribute('aria-label', hasUpdate() ? AieyesI18n.t("发现新版本，查看更新") : AieyesI18n.t("检查更新"));panel.querySelector('.update-badge').hidden = !hasUpdate();}
    const current = document.querySelector('#update-current');
    if (current) current.textContent = AieyesI18n.t("当前版本 {0}", status.currentVersion ? 'v' + status.currentVersion : AieyesI18n.t("读取中…"));
    const result = document.querySelector('#update-result');
    if (result) {if (status.phase === 'error') AieyesUI.showError(result, status.message, AieyesI18n.t("更新未完成"));else result.textContent = AieyesI18n.message(status.message);result.classList.toggle('error', status.phase === 'error');}
    const check = document.querySelector('#updates');
    if (check) {check.disabled = busy();check.textContent = status.phase === 'checking' ? AieyesI18n.t("检查中…") : AieyesI18n.t("检查更新");}
    const view = document.querySelector('#update-view');
    if (view) view.hidden = !['available', 'downloading', 'verifying', 'installing'].includes(status.phase);
    const automatic = document.querySelector('#updates-automatic');
    if (automatic) automatic.checked = status.automatic;
    if (dialog?.open) renderDialog();
    maybePresent();
  }
  function accept(next) {status = next;updateView();}
  async function run(command, args) {
    try {accept(await invoke(command, args));}
    catch (error) {status = { ...status, phase: 'error', message: String(error) };updateView();}
  }
  function bindSettings() {
    document.querySelectorAll('#general-form').forEach((form) => {form.dataset.updateSavedValues = JSON.stringify([...new FormData(form)]);});
    const check = document.querySelector('#updates');
    if (check) check.onclick = () => run('updates_check');
    const view = document.querySelector('#update-view');
    if (view) view.onclick = present;
    const automatic = document.querySelector('#updates-automatic');
    if (automatic) automatic.onchange = () => run('updates_preferences', { automatic: automatic.checked });
    updateView();
  }
  function maybePresent() {
    if (!PANEL && status.prompt && status.phase === 'available' && document.hasFocus() && !document.querySelector('dialog[open]')) present();
  }
  function renderDialog() {
    dialog.querySelector('#update-versions').textContent = AieyesI18n.t("当前版本 v{0} → 新版本 v{1}", status.currentVersion, status.latestVersion ?? '—');
    dialog.querySelector('#update-notes').textContent = status.notes || AieyesI18n.t("此版本包含改进与修复。");
    const message = dialog.querySelector('#update-message');if (status.phase === 'error') AieyesUI.showError(message, status.message, AieyesI18n.t("更新未完成"));else if (message.textContent !== AieyesI18n.message(status.message)) message.textContent = AieyesI18n.message(status.message);message.classList.toggle('error', status.phase === 'error');
    const progress = dialog.querySelector('progress');progress.hidden = !installing();
    if (status.total > 0) {progress.max = Math.max(status.total, status.downloaded);progress.value = status.downloaded;} else {progress.removeAttribute('value');}
    dialog.querySelector('#update-bytes').textContent = status.phase === 'downloading' ? `${(status.downloaded / 1048576).toFixed(1)} MB${status.total ? ' / ' + (status.total / 1048576).toFixed(1) + ' MB' : ''}` : preparing ? waited : '';
    const install = dialog.querySelector('#update-install');install.disabled = busy();install.textContent = AieyesI18n.message(status.phase === 'error' ? AieyesI18n.t("重新检查") : AieyesI18n.t("更新并重启"));
    install.hidden = !['available', 'error', 'downloading', 'verifying', 'installing'].includes(status.phase);
    dialog.querySelector('#update-later').disabled = busy() && !preparing;
    dialog.querySelector('#update-later').textContent = preparing ? AieyesI18n.t("取消等待") : AieyesI18n.t("稍后");
  }
  function present() {
    if (!dialog) {
      dialog = document.createElement('dialog');dialog.id = 'update-dialog';dialog.setAttribute('aria-labelledby', 'update-title');
      dialog.innerHTML = `<div class="dialog-head"><h2 id="update-title">${AieyesI18n.t("Aieyes 更新")}</h2></div><p id="update-versions"></p><div id="update-notes"></div><p id="update-message" role="status"></p><progress hidden></progress><p id="update-bytes"></p><div class="dialog-foot"><button id="update-later">${AieyesI18n.t("稍后")}</button><button id="update-install" class="primary">${AieyesI18n.t("更新并重启")}</button></div>`;
      document.body.append(dialog);
      dialog.oncancel = (event) => {event.preventDefault();if (!busy()) later();};
      dialog.querySelector('#update-later').onclick = later;
      dialog.querySelector('#update-install').onclick = install;
    }
    renderDialog();
    if (!dialog.open) dialog.showModal();
  }
  async function later() {if (preparing) {cancelled = true;answer?.('cancel');return;}await run('updates_later');dialog?.close();}
  function unsavedForms() {
    if (typeof settingsFormsDirty === 'function') return settingsFormsDirty();
    return [...document.querySelectorAll('#general-form')].some((form) => form.dataset.updateSavedValues && form.dataset.updateSavedValues !== JSON.stringify([...new FormData(form)]));
  }
  function decide(message, choices) {
    status.message = message;renderDialog();
    return new Promise((resolve) => {
      const actions = document.createElement('div');actions.className = 'actions';actions.id = 'update-preparation-actions';
      answer = (value) => {actions.remove();answer = null;resolve(value);};
      for (const [value, label] of choices) {const button = document.createElement('button');button.textContent = label;button.onclick = () => answer?.(value);actions.append(button);}
      dialog.querySelector('#update-message').after(actions);actions.querySelector('button')?.focus();
    });
  }
  function blockers() {
    return [[state.settingsSaving || state.settingsCommitting || editorSaving, AieyesI18n.t("保存配置")], [state.repricing, AieyesI18n.t("重算价格")], [window.AieyesAgyAuth?.busy, AieyesI18n.t("Antigravity 登录授权")], [window.AieyesCodexAuth?.busy, AieyesI18n.t("Codex 登录授权")]].filter(([busy]) => busy).map(([, label]) => label);
  }
  async function prepare() {
    const started = new Map();let deadline = Date.now() + 30000;
    while (!cancelled) {
      const tasks = blockers();if (!tasks.length) break;
      for (const task of tasks) if (!started.has(task)) started.set(task, Date.now());
      // The live message names the tasks; elapsed seconds stay in the non-live detail line.
      status.message = AieyesI18n.t("等待当前任务完成：") + tasks.join('、');
      waited = AieyesI18n.t("等待当前任务完成：{0}", tasks.map(task => AieyesI18n.t("{0}（已等待 {1} 秒）", task, Math.floor((Date.now() - started.get(task)) / 1000))).join("、"));renderDialog();
      if (Date.now() >= deadline) {if ((await decide(AieyesI18n.t("{0}。可取消等待，回到对应页面处理。", waited), [['continue', AieyesI18n.t("继续等待")], ['cancel', AieyesI18n.t("取消并返回")]])) === 'cancel') return false;deadline = Date.now() + 30000;}
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    if (cancelled) return false;
    if (window.AieyesAgentSettings?.flush && !(await window.AieyesAgentSettings.flush())) {status.message = AieyesI18n.t("账户设置保存失败，请检查账户页标出的字段。");dialog.close();return false;}
    const editor = document.querySelector('#editor');
    if (editor?.open) {
      if (editorSnapshot() !== editorBaseline) {
        const choice = await decide(AieyesI18n.t("更新前保存“{0}”中的更改？", document.querySelector('#editor-title').textContent), [['save', AieyesI18n.t("保存并更新")], ['discard', AieyesI18n.t("放弃更改并更新")], ['cancel', AieyesI18n.t("取消")]]);
        if (choice === 'cancel') return false;
        if (choice === 'save') {const form = document.querySelector('#editor-form');if (!form.reportValidity()) {dialog.close();return false;}await form.onsubmit({ preventDefault() {}, target: form });if (editor.open) {status.message = AieyesI18n.t("编辑器保存失败，请检查显示的错误。");dialog.close();document.querySelector('#editor-error')?.focus();return false;}} else
        closeEditor(true);
      } else closeEditor(true);
    }
    if (unsavedForms()) {
      const choice = await decide(AieyesI18n.t("更新前保存应用设置中的更改？"), [['save', AieyesI18n.t("保存并更新")], ['discard', AieyesI18n.t("放弃更改并更新")], ['cancel', AieyesI18n.t("取消")]]);
      if (choice === 'cancel') return false;
      if (choice === 'discard') discardAllSettings();else
      try {await saveAllSettings();} catch (error) {status.message = AieyesI18n.t("设置保存失败：") + String(error);dialog.close();notify(status.message, 'error');document.getElementById(error.field)?.focus();return false;}
    }
    return !cancelled;
  }
  async function install() {
    if (busy()) return;
    if (status.phase === 'error') {await run('updates_check');return;}
    submitting = true;cancelled = false;renderDialog();
    try {
      await run('updates_download');
      if (status.phase === 'error') return;
      submitting = false;preparing = true;renderDialog();
      if (!(await prepare())) {if (dialog.open) {status.message = cancelled ? AieyesI18n.t("已取消更新等待；后台刷新已恢复") : status.message;renderDialog();}return;}
      let operations = await api('operations.prepareUpdate');
      while (operations.protected.length && !cancelled) {status.message = AieyesI18n.t("正在完成账户或配置操作：") + operations.protected.join('、');renderDialog();await new Promise((resolve) => setTimeout(resolve, 250));operations = await api('operations.status');}
      if (cancelled) return;
      preparing = false;submitting = true;renderDialog();await run('updates_install');
    } catch (error) {status = { ...status, phase: 'error', message: String(error.message ?? error) };renderDialog();} finally
    {await api('operations.resume').catch(() => {});preparing = false;submitting = false;answer?.('cancel');updateView();}
  }
  window.AieyesUpdates = { settingsHTML, bindSettings, get busy() {return preparing || status.phase === 'installing';} };
  window.addEventListener('focus', maybePresent);
  document.addEventListener('visibilitychange', maybePresent);
  document.addEventListener('close', () => queueMicrotask(maybePresent), true);
  async function boot() {
    if (!window.__TAURI__) return;
    await window.__TAURI__.event.listen('updates:open', () => {if (!PANEL) present();});
    const panel = document.querySelector('#panel-update');if (panel) panel.onclick = async () => {await invoke('desktop_action', { action: 'open' });await run('updates_panel_check');};
    await window.__TAURI__.event.listen('updates:status', (event) => accept(event.payload));
    accept(await invoke('updates_info'));
    if (state.page === 'settings' && state.settingsTab === 'general' && state.generalTab === 'updates') renderSettings();
  }
  Promise.resolve(appReady).then(boot).catch((error) => {status.message = String(error);status.phase = 'error';updateView();});
})();
