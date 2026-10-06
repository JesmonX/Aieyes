(() => {
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);
  let status = {currentVersion: '', phase:'idle', message:'', automatic:true, prompt:false};
  let dialog = null, submitting = false;
  const busy = () => submitting || ['checking','downloading','verifying','installing'].includes(status.phase);
  const installing = () => ['downloading','verifying','installing'].includes(status.phase);
  function settingsHTML() {
    return `<div class="card update-settings"><h2>应用更新</h2><p id="update-current">当前版本 ${escape(status.currentVersion ? 'v'+status.currentVersion : '读取中…')}</p><label class="quota-confirm"><input id="updates-automatic" type="checkbox" ${status.automatic?'checked':''}>启动时及每天检查更新</label><div class="actions"><button id="updates" ${busy()?'disabled':''}>${status.phase==='checking'?'检查中…':'检查更新'}</button><button id="update-view" ${status.phase==='available'?'':'hidden'}>查看更新</button></div><p id="update-result" role="status">${escape(status.message)}</p></div>`;
  }
  function hasUpdate() {
    const parts=v=>String(v??'').replace(/^v/,'').split('.').map(Number),latest=parts(status.latestVersion),current=parts(status.currentVersion);
    return latest.length===3 && current.length===3 && latest.every(Number.isFinite) && current.every(Number.isFinite) && latest.some((n,i)=>n>current[i]&&latest.slice(0,i).every((v,j)=>v===current[j]));
  }
  function updateView() {
    const panel=document.querySelector('#panel-update');
    if(panel){panel.disabled=status.phase==='checking';panel.title=hasUpdate()?'发现新版本 · 查看更新':status.message||'检查更新';panel.setAttribute('aria-label',hasUpdate()?'发现新版本，查看更新':'检查更新');panel.querySelector('.update-badge').hidden=!hasUpdate();}
    const current = document.querySelector('#update-current');
    if (current) current.textContent = `当前版本 ${status.currentVersion?'v'+status.currentVersion:'读取中…'}`;
    const result = document.querySelector('#update-result');
    if (result) { result.textContent = status.message; result.classList.toggle('error', status.phase === 'error'); }
    const check = document.querySelector('#updates');
    if (check) { check.disabled = busy(); check.textContent = status.phase === 'checking' ? '检查中…' : '检查更新'; }
    const view = document.querySelector('#update-view');
    if (view) view.hidden = !['available','downloading','verifying','installing'].includes(status.phase);
    const automatic = document.querySelector('#updates-automatic');
    if (automatic) automatic.checked = status.automatic;
    if (dialog?.open) renderDialog();
    maybePresent();
  }
  function accept(next) { status = next; updateView(); }
  async function run(command, args) {
    try { accept(await invoke(command,args)); }
    catch (error) { status = {...status, phase:'error', message:String(error)}; updateView(); }
  }
  function bindSettings() {
    document.querySelectorAll('#general-form').forEach(form => { form.dataset.updateSavedValues = JSON.stringify([...new FormData(form)]); });
    const check = document.querySelector('#updates');
    if (check) check.onclick = () => run('updates_check');
    const view = document.querySelector('#update-view');
    if (view) view.onclick = present;
    const automatic = document.querySelector('#updates-automatic');
    if (automatic) automatic.onchange = () => run('updates_preferences', {automatic:automatic.checked});
    updateView();
  }
  function maybePresent() {
    if (!PANEL && status.prompt && status.phase === 'available' && document.hasFocus() && !document.querySelector('dialog[open]')) present();
  }
  function renderDialog() {
    dialog.querySelector('#update-versions').textContent = `当前版本 v${status.currentVersion} → 新版本 v${status.latestVersion ?? '—'}`;
    dialog.querySelector('#update-notes').textContent = status.notes || '此版本包含改进与修复。';
    const message = dialog.querySelector('#update-message'); message.textContent = status.message; message.classList.toggle('error',status.phase === 'error');
    const progress = dialog.querySelector('progress'); progress.hidden = !installing();
    if (status.total > 0) { progress.max = Math.max(status.total,status.downloaded); progress.value = status.downloaded; } else { progress.removeAttribute('value'); }
    dialog.querySelector('#update-bytes').textContent = status.phase === 'downloading' ? `${(status.downloaded/1048576).toFixed(1)} MB${status.total?' / '+(status.total/1048576).toFixed(1)+' MB':''}` : '';
    const install = dialog.querySelector('#update-install'); install.disabled = busy(); install.textContent = status.phase === 'error' ? '重新检查' : '更新并重启';
    install.hidden = !['available','error','downloading','verifying','installing'].includes(status.phase);
    dialog.querySelector('#update-later').disabled = busy();
  }
  function present() {
    if (!dialog) {
      dialog = document.createElement('dialog'); dialog.id = 'update-dialog'; dialog.setAttribute('aria-labelledby','update-title');
      dialog.innerHTML = '<div class="dialog-head"><h2 id="update-title">Aieyes 更新</h2></div><p id="update-versions"></p><div id="update-notes"></div><p id="update-message" role="status"></p><progress hidden></progress><p id="update-bytes"></p><div class="dialog-foot"><button id="update-later">稍后</button><button id="update-install" class="primary">更新并重启</button></div>';
      document.body.append(dialog);
      dialog.oncancel = event => { event.preventDefault(); if (!busy()) later(); };
      dialog.querySelector('#update-later').onclick = later;
      dialog.querySelector('#update-install').onclick = install;
    }
    renderDialog();
    if (!dialog.open) dialog.showModal();
  }
  async function later() { await run('updates_later'); dialog?.close(); }
  function unsavedForms() {
    if(typeof settingsFormsDirty==='function'&&settingsFormsDirty())return true;
    return [...document.querySelectorAll('#general-form')].some(form => form.dataset.updateSavedValues && form.dataset.updateSavedValues !== JSON.stringify([...new FormData(form)]));
  }
  async function install() {
    if (busy()) return;
    if (status.phase === 'error') { await run('updates_check'); return; }
    if (state.busy || state.settingsSaving || state.serverBusy || state.quotaBusy || state.estimateBusy || state.sharedEstimateOperations?.size || editorSaving || document.querySelector('#quota-dialog')?.dataset.busy || document.querySelector('#editor')?.open || unsavedForms()) {
      status.message = '请先完成当前操作并保存设置，再点击更新。'; renderDialog(); return;
    }
    submitting = true; renderDialog();
    try { await run('updates_install'); }
    finally { submitting = false; updateView(); }
  }
  window.AieyesUpdates = {settingsHTML, bindSettings, get busy() { return installing(); }};
  window.addEventListener('focus', maybePresent);
  document.addEventListener('visibilitychange', maybePresent);
  document.addEventListener('close', () => queueMicrotask(maybePresent), true);
  async function boot() {
    if (!window.__TAURI__) return;
    await window.__TAURI__.event.listen('updates:open',()=>{if(!PANEL)present();});
    const panel=document.querySelector('#panel-update');if(panel)panel.onclick=async()=>{await invoke('desktop_action',{action:'open'});await run('updates_panel_check');};
    await window.__TAURI__.event.listen('updates:status', event => accept(event.payload));
    accept(await invoke('updates_info'));
    if (state.page === 'settings' && state.settingsTab === 'general') renderSettings();
  }
  Promise.resolve(appReady).then(boot).catch(error => { status.message = String(error); status.phase = 'error'; updateView(); });
})();
