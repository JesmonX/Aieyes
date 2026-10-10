(() => {
  const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);
  let status = {currentVersion: '', phase:'idle', message:'', automatic:true, prompt:false};
  let dialog = null, submitting = false, preparing = false, cancelled = false, answer = null;
  const busy = () => preparing || submitting || ['checking','downloading','verifying','installing'].includes(status.phase);
  const installing = () => ['downloading','verifying','installing'].includes(status.phase);
  function settingsHTML() {
    return `<div class="card update-settings"><h2>应用更新</h2><p id="update-current">当前版本 ${escape(status.currentVersion ? 'v'+status.currentVersion : '读取中…')}</p><label class="quota-confirm"><input id="updates-automatic" type="checkbox" ${status.automatic?'checked':''}>启动时及每天检查更新</label><div class="actions"><button id="updates" ${busy()?'disabled':''}>${status.phase==='checking'?'检查中…':'检查更新'}</button><button id="update-view" ${status.phase==='available'?'':'hidden'}>查看更新</button></div><p id="update-result" role="status">${escape(status.message)}</p></div>`;
  }
  function hasUpdate() {
    const parts=v=>String(v??'').replace(/^v/,'').split('.').map(Number),latest=parts(status.latestVersion),current=parts(status.currentVersion);
    return latest.length===3 && current.length===3 && latest.every(Number.isFinite) && current.every(Number.isFinite) && latest.some((n,i)=>n>current[i]&&latest.slice(0,i).every((v,j)=>v===current[j]));
  }
  function updateView() {
    if(typeof scheduleRefreshes==='function')scheduleRefreshes();
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
    dialog.querySelector('#update-later').disabled = busy()&&!preparing;
    dialog.querySelector('#update-later').textContent=preparing?'取消等待':'稍后';
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
  async function later() { if(preparing){cancelled=true;answer?.('cancel');return;} await run('updates_later'); dialog?.close(); }
  function unsavedForms() {
    if(typeof settingsFormsDirty==='function')return settingsFormsDirty();
    return [...document.querySelectorAll('#general-form')].some(form => form.dataset.updateSavedValues && form.dataset.updateSavedValues !== JSON.stringify([...new FormData(form)]));
  }
  function decide(message, choices) {
    status.message=message;renderDialog();
    return new Promise(resolve=>{
      const actions=document.createElement('div');actions.className='actions';actions.id='update-preparation-actions';
      answer=value=>{actions.remove();answer=null;resolve(value);};
      for(const [value,label] of choices){const button=document.createElement('button');button.textContent=label;button.onclick=()=>answer?.(value);actions.append(button);}
      dialog.querySelector('#update-message').after(actions);actions.querySelector('button')?.focus();
    });
  }
  function blockers() {
    return [[state.dashboardPending,'概览查询'],[quotaScheduleRead!=null,'刷新调度读取'],[state.busy,'记录同步 / 价格操作'],[state.quotaBusy,'Agent 限额查询'],[state.serverBusy,'服务器采样'],[state.settingsSaving||state.settingsCommitting||editorSaving,'保存配置'],[state.repricing,'重算价格'],[state.estimateBusy||state.sharedEstimateOperations?.size||document.querySelector('#quota-dialog')?.dataset.busy,'采样任务'],[window.AieyesAgyAuth?.busy,'Antigravity 登录授权'],[window.AieyesAgentSettings?.antigravityRefreshing,'Antigravity 登录后刷新'],[window.AieyesCodexAuth?.busy,'Codex 登录授权']].filter(([busy])=>busy).map(([,label])=>label);
  }
  async function prepare() {
    const started=new Map();let deadline=Date.now()+30000;
    while(!cancelled){
      const tasks=blockers();if(!tasks.length)break;
      for(const task of tasks)if(!started.has(task))started.set(task,Date.now());
      status.message='等待当前任务完成：'+tasks.map(task=>task+'（已等待 '+Math.floor((Date.now()-started.get(task))/1000)+' 秒）').join('、');renderDialog();
      if(Date.now()>=deadline){if(await decide(status.message+'。可取消等待，回到对应页面处理。',[['continue','继续等待'],['cancel','取消并返回']])==='cancel')return false;deadline=Date.now()+30000;}
      await new Promise(resolve=>setTimeout(resolve,250));
    }
    if(cancelled)return false;
    if(window.AieyesAgentSettings?.flush&&!await window.AieyesAgentSettings.flush()){status.message='账户设置保存失败，请检查账户页标出的字段。';dialog.close();return false;}
    const editor=document.querySelector('#editor');
    if(editor?.open){
      if(editorSnapshot()!==editorBaseline){
        const choice=await decide('更新前保存“'+document.querySelector('#editor-title').textContent+'”中的更改？',[['save','保存并更新'],['discard','放弃更改并更新'],['cancel','取消']]);
        if(choice==='cancel')return false;
        if(choice==='save'){const form=document.querySelector('#editor-form');if(!form.reportValidity()){dialog.close();return false;}await form.onsubmit({preventDefault(){},target:form});if(editor.open){status.message='编辑器保存失败，请检查显示的错误。';dialog.close();document.querySelector('#editor-error')?.focus();return false;}}
        else closeEditor(true);
      } else closeEditor(true);
    }
    if(unsavedForms()){
      const choice=await decide('更新前保存应用设置中的更改？',[['save','保存并更新'],['discard','放弃更改并更新'],['cancel','取消']]);
      if(choice==='cancel')return false;
      if(choice==='discard')discardAllSettings();
      else try{await saveAllSettings();}catch(error){status.message='设置保存失败：'+String(error);dialog.close();notify(status.message,'error');document.getElementById(error.field)?.focus();return false;}
    }
    return !cancelled;
  }
  async function install() {
    if (busy()) return;
    if (status.phase === 'error') { await run('updates_check'); return; }
    preparing=true;cancelled=false;renderDialog();
    try {
      if(!await prepare()){if(dialog.open){status.message=cancelled?'已取消更新等待；后台刷新已恢复':status.message;renderDialog();}return;}
      preparing=false;submitting=true;renderDialog();await run('updates_install');
    } finally {preparing=false;submitting=false;answer?.('cancel');updateView();}
  }
  window.AieyesUpdates = {settingsHTML, bindSettings, get busy() { return preparing || submitting || installing(); }};
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
