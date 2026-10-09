/* Credentials remain in the selected home; this view only receives profile metadata. */
(() => {
  const e = value => escapeHTML(value ?? '');
  let intent = 'login', needsRefresh = false, targetAccountId = '', sourceId = '', info = null, busy = false, error = '', notice = '', prepared = null, operation = null, timer;
  const savedSources = () => (state.settings?.sources ?? []).filter(s => s.provider === 'codex' && s.enabled);
  const selected = () => savedSources().find(s => s.id === sourceId);
  const targetAccount = () => state.settings.accounts.find(a=>a.provider==='codex'&&a.id===targetAccountId);
  const paint = () => { if (state.page === 'settings' && state.settingsTab === 'accounts') renderSettings(false); };
  async function call(method, params = {}) { return api('codexAuth.' + method, {sourceId,accountId:targetAccountId, ...params}); }
  async function settingsChanged() {
    window.AieyesAgentSettings.accept(await api('settings.get'));
    void loadDashboard();
  }
  async function refresh() {
    const requestedSource=sourceId, requestedAccount=targetAccountId;
    const loaded = await call('inspect');
    if(sourceId!==requestedSource||targetAccountId!==requestedAccount){needsRefresh=true;return;}
    info=loaded;
    const account=state.settings.accounts.find(a=>a.provider==='codex'&&a.id===targetAccountId);
    if(intent==='switch'&&account){const profile=visibleProfiles().find(p=>!p.current&&window.AieyesAgentSettings.profiles(account).includes(selected()?.codexHomeId+':'+p.id));if(profile)prepared=await call('switch.prepare',{profileId:profile.id});}
  }
  async function run(body) {
    if (busy) return;
    busy = true; error = ''; notice = ''; paint();
    try { await body(); } catch (err) { error = err.message ?? String(err); }
    finally { busy = false; paint(); }
  }
  function remember() {
    if (operation) localStorage.setItem('aieyes.codexLogin', JSON.stringify({sourceId, operationId:operation.operationId,accountId:targetAccountId}));
    else localStorage.removeItem('aieyes.codexLogin');
  }
  async function poll() {
    clearTimeout(timer);
    if (!operation) return;
    try {
      operation = await call('login.status', {operationId:operation.operationId,accountId:targetAccountId});
      if (['succeeded','failed','cancelled'].includes(operation.status)) {
        if (operation.status === 'succeeded') { notice = operation.accountArchived?'登录已保存，该账户已归档，可在账户页恢复。':operation.quotaEnabled===false?'登录成功，该账户的额度查询已暂停。':'登录成功，已启用此账号的额度查询。'; await settingsChanged(); await refresh(); }
        else error = operation.error || '登录已结束';
        operation = null; remember();
      }
    } catch (err) { error = err.message ?? String(err); }
    paint(); if (operation) timer = setTimeout(poll, 1800);
  }
  function visibleProfiles(){
    const a=state.settings.accounts.find(a=>a.provider==='codex'&&a.id===targetAccountId);
    if(!a)return info?.profiles??[];
    return (info?.profiles??[]).filter(p=>{const ref=selected()?.codexHomeId+':'+p.id;return a.identityKey?a.identityKey===p.identity.key:window.AieyesAgentSettings.profiles(a).includes(ref)||!state.settings.accounts.some(other=>window.AieyesAgentSettings.profiles(other).includes(ref));});
  }
  function html() {
    const sources = savedSources();
    if (!sourceId || !sources.some(s => s.id === sourceId)) { sourceId = sources[0]?.id ?? ''; info = null; }
    const source = selected(), account=targetAccount(), dirty = settingsFormsDirty();
    return `<div class="card"><h2>${intent==='switch'?'切换账户':'Codex 登录'}</h2><p class="muted">${account?'在所选机器登录「'+e(account.name)+'」，请使用同一个账号。':'登录或读取成功后自动添加账号，以邮箱命名；已添加的同一账号会自动识别。'}切换会改变机器当前使用的账号。</p>${sources.length?`<label>运行位置<select id="auth-source" ${busy||operation?'disabled':''}>${sources.map(s=>`<option value="${e(s.id)}" ${s.id===sourceId?'selected':''}>${e(s.name)} · ${s.hostId?'SSH':'本机'}</option>`).join('')}</select></label><p class="tiny muted">${e(source.path)}</p>`:'<p>请先在 Agents 启用 Codex 并选择机器。</p>'}${error?`<p class="error" role="alert">${e(error)} <button data-auth="refresh" ${busy?'disabled':''}>重试读取</button></p>`:''}${notice?`<p role="status">${e(notice)}</p>`:''}${busy?'<p role="status">正在处理…</p>':''}
    <fieldset ${busy||dirty||!source||operation?'disabled':''} style="border:0;padding:0"><div class="actions">${source&&intent==='login'?`<button data-auth="adopt">读取当前登录账号</button><button data-auth="login">${account?'登录此账号':'登录新账号'}</button>`:''}</div>
    ${info?`<p class="tiny muted">${e(info.version||'CLI 版本未知')} · 认证存储 ${e(info.storageMode)} · 检测到 ${info.processes?.length??0} 个相关进程</p>`:''}
    ${visibleProfiles().map(p=>`<div class="list-row"><div class="row-body"><strong>${e(p.name)}</strong>${p.current?' · 当前日常账号':''}<small>${e(p.identity.email)} · ${e(p.identity.plan)}</small></div>${intent!=='login'||state.settings.accounts.some(a=>window.AieyesAgentSettings.profiles(a).includes(source.codexHomeId+':'+p.id))?'':`<button data-profile="${e(p.id)}" data-auth="bind">添加此账号</button>`}${intent==='login'?`<button data-profile="${e(p.id)}" data-auth="reauth">重新登录</button>`:''}${p.current?'':(intent==='switch'?`<button data-profile="${e(p.id)}" data-auth="switch">切换到此账户</button>`:`<button data-profile="${e(p.id)}" data-auth="remove">移除</button>`)}</div>`).join('')}
    ${prepared?`<div class="card"><h3>确认切换账号</h3><p>${prepared.processes.length?'需要先关闭以下 Codex 进程：':'未检测到运行中的 Codex。'}</p><ul>${prepared.processes.map(p=>`<li>${e(p.name)} · PID ${p.pid}${p.canClose?'':' · 无法确认归属，请手动关闭'}</li>`).join('')}</ul><p>切换后请重新打开 Codex 并恢复会话。</p><button data-auth="cancelSwitch">取消</button><button data-auth="commit" ${prepared.processes.some(p=>!p.canClose)?'disabled':''}>${prepared.processes.length?'关闭并切换':'确认切换'}</button></div>`:''}</fieldset>
    ${operation?`<div class="card" role="status"><h3>等待授权</h3>${operation.userCode?`<p>设备码：<strong>${e(operation.userCode)}</strong> <button data-auth="copyCode">复制设备码</button></p>`:''}${operation.authUrl?'<button data-auth="open">打开登录页面</button><button data-auth="copyUrl">复制登录链接</button>':'<p>正在准备登录…</p>'}<button data-auth="cancelLogin">取消登录</button><p class="tiny muted">请在浏览器完成授权。登录最多等待 10 分钟。</p></div>`:''}</div>`;
  }
  async function ensureManaged() {if(!selected()?.codexHomeId){await call('enable');await settingsChanged();}}
  function login(profileId) {
    const p = info?.profiles.find(p=>p.id===profileId);
    showEditor(p?'重新登录':'添加 Codex 账号',`<p>登录后自动以邮箱命名，可在账户设置中修改名称。</p><p>将在${selected().hostId?'服务器':'本机'}保存登录。${p?'请使用原账号。':''}</p>`, async () => {
      await ensureManaged();
      prepared = null; operation = await call('login.start',{profileId:profileId??null,deviceCode:!!selected().hostId});
      remember(); poll();
    });
  }
  function bind() {
    if(needsRefresh&&!busy&&!operation&&sourceId){needsRefresh=false;queueMicrotask(()=>run(refresh));}
    const selector = document.getElementById('auth-source');
    if (selector) selector.onchange = () => { sourceId=selector.value; info=null; prepared=null; run(refresh); };
    document.querySelectorAll('[data-auth]').forEach(button => button.onclick = () => {
      const action = button.dataset.auth, profileId = button.dataset.profile;
      if (action==='login'||action==='reauth') return login(profileId);
      if (action==='bind') return run(async()=>{await call('profiles.bind',{profileId,accountId:targetAccountId});await settingsChanged();await refresh();});
      if (action==='remove') return showEditor('移除账号档案','<p>删除此位置保存的登录凭据，历史记录保留。</p>',async()=>{await call('profiles.remove',{profileId});await settingsChanged();await refresh();paint();});
      run(async()=>{
        if (action==='refresh') await refresh();
        if (action==='adopt') { await ensureManaged();await call('adopt',{accountId:targetAccountId});await settingsChanged(); await refresh(); }
        if (action==='switch') prepared=await call('switch.prepare',{profileId});
        if (action==='cancelSwitch') prepared=null;
        if (action==='commit') { const r=await call('switch.commit',{operationId:prepared.operationId,closeProcesses:prepared.processes.length>0});prepared=null;notice=r.message??'账号已切换，请重新打开 Codex。';await refresh(); }
        if (action==='cancelLogin') await call('login.cancel',{operationId:operation.operationId,accountId:targetAccountId});
        if (action==='copyUrl'||action==='copyCode'){await navigator.clipboard.writeText(action==='copyUrl'?operation.authUrl:operation.userCode);notice='已复制';}
        if (action==='open') await api('codexAuth.openUrl',{url:operation.authUrl});
      });
    });
    if (!operation) { try { const saved=JSON.parse(localStorage.getItem('aieyes.codexLogin')||'null');if(saved&&savedSources().some(s=>s.id===saved.sourceId)){sourceId=saved.sourceId;targetAccountId=saved.accountId||'';operation={operationId:saved.operationId};poll();} } catch { localStorage.removeItem('aieyes.codexLogin'); } }
  }
  window.AieyesCodexAuth = {html,bind,open(accountId='',selectedSource='',action='login'){if(!operation){intent=action;needsRefresh=true;targetAccountId=accountId;if(selectedSource)sourceId=selectedSource;info=null;prepared=null;error='';notice='';}}};
})();
