/* Agent setup and account connections. Credentials never enter rendered markup. */
(() => {
  const e = value => escapeHTML(value ?? '');
  let directorySelection = null, saveQueue = Promise.resolve(), pendingSaves = 0;
  const accountDrafts = new Map(), accountErrors = new Map();
  let showCodex = false, detailKey = '', statusRows = [], statusLoading = false, statusLoaded = false, cleanupRows = [], syncMessage = '', syncRunning = false, refreshTimer;
  let statusRevision = 0;
  const sourceStatusRequests = new Map(), sourceStatusTasks = new Map();
  const antigravityRefreshTasks = new Map();
  const accountKey = a => a.provider+':'+a.id;
  const machineId = s => s.hostId||'local';
  const deviceName = id => id==='local'?'本机':state.settings.hosts.find(h=>h.id===id)?.name||state.settings.hosts.find(h=>h.id===id)?.target||'已移除设备';
  function machines(provider, paused=false) {
    const config=configurations().find(a=>a.provider===provider);
    const active=(config?.enabled?config.machineIds:[]).filter(id=>id==='local'||state.settings.hosts.some(h=>h.id===id&&h.enabled));
    return [...new Set(paused?[...active,...state.settings.sources.filter(s=>s.provider===provider).map(machineId)]:active)].sort((a,b)=>a==='local'?-1:b==='local'?1:a.localeCompare(b));
  }
  const statuses=(a,machine)=>statusRows.filter(r=>r.accountKey===accountKey(a)&&r.machineId===machine);
  const loggedIn = row => row.current===true||row.credential===true;
  const identityText=row=>row?.identity ? (row.current===false?'身份不一致 · 当前登录 ':'正在使用 ')+row.identity.email+' · '+(row.identity.subscription||'订阅未知')+(row.identity.stale?' · 订阅待更新':'')+(row.error?' · 上次检查':'') : '';
  const statusText=rows=>{
    const identified=rows.find(r=>r.identity);if(identified)return identityText(identified);
    if(rows.some(r=>r.authenticated===true))return '登录有效 · 账户身份待确认';
    const login=rows.some(loggedIn)?'已登录':rows.length&&rows.every(r=>r.current===false&&r.credential===false)?'未登录':'登录待确认';
    const current=rows.some(r=>r.current===true)?'正在使用':rows.length&&rows.every(r=>r.current===false)?'未使用':'使用状态待确认';
    return login+' · '+current+(rows.some(r=>r.error)?' · 待确认':'');
  };
  function counts(a){const ids=machines(a.provider);return '已登录 '+ids.filter(id=>statuses(a,id).some(loggedIn)).length+'/'+ids.length+' · 使用中 '+ids.filter(id=>statuses(a,id).some(r=>r.current===true)).length+'/'+ids.length+(ids.some(id=>{const rows=statuses(a,id);return !rows.length||rows.some(r=>r.current==null||r.credential==null||r.error);})?' · 有设备待确认':'');}
  const repaint=()=>{if(state.page==='settings'&&state.settingsTab==='accounts'&&!document.getElementById('editor')?.open&&!document.activeElement?.matches('[data-account-field],[data-command-field]'))renderSettings(false);};
  async function refreshStatuses(refresh=true){
    if(statusLoading)return;statusLoading=true;
    const revision=statusRevision;
    try{
      const loaded=await api('accounts.status.get');if(revision===statusRevision)statusRows=Array.isArray(loaded)?loaded:[];
      cleanupRows=await api('accounts.cleanup.list');if(!Array.isArray(cleanupRows))cleanupRows=[];
      statusLoaded=true;repaint();
      if(refresh)for(const a of state.settings.accounts.filter(a=>!a.archived))for(const s of state.settings.sources.filter(s=>s.provider===a.provider&&s.enabled&&machines(a.provider).includes(machineId(s)))){
        if(state.page!=='settings'||state.settingsTab!=='accounts')return;
        try{const row=await api('accounts.status.refresh',{accountKey:accountKey(a),sourceId:s.id});if(revision===statusRevision){statusRows=statusRows.filter(r=>!(r.accountKey===row.accountKey&&r.sourceId===row.sourceId));statusRows.push(row);repaint();}}catch{/* Settings changed while this device was being checked. */}
      }
    }catch(error){notify('设备状态读取失败：'+String(error),'error');}
    finally{statusLoading=false;await syncIdentitySettings();repaint();}
  }
  async function syncIdentitySettings(reportFailure=false) {
    if(pendingSaves||state.settingsSaving)return;
    const before=configKey(state.settings),saved=await api('settings.get').catch(error=>{if(reportFailure)notify('登录已确认，账户设置刷新失败：'+String(error),'error');return null;});
    if(!saved)return;
    if(before===configKey(state.settings)&&!pendingSaves&&!state.settingsSaving){
      const clean=!state.settingsDraft||configKey(state.settingsDraft)===before;
      state.settings=saved;if(clean){state.settingsDraft=structuredClone(saved);state.settingsBaseline=configKey(saved);}
    }
  }
  async function refreshAntigravitySource(sourceId, status=null) {
    const source=state.settings.sources.find(s=>s.id===sourceId&&s.provider==='antigravity');
    if(!source)throw new Error('账户连接已变化，请重新检查');
    const account=state.settings.accounts.find(a=>a.provider==='antigravity'&&a.id===source.accountId);
    if(!account)throw new Error('账户连接已变化，请重新检查');
    const revision=++statusRevision;
    const row=status??await api('accounts.status.refresh',{accountKey:accountKey(account),sourceId});
    if(!status&&row.error)throw new Error('设备状态更新失败：'+row.error);
    if(revision!==statusRevision||row.sourceId!==sourceId||row.accountKey!==accountKey(account)||row.machineId!==machineId(source)||!state.settings.sources.some(s=>s.id===sourceId&&s.accountId===account.id&&s.enabled))throw new Error('账户连接已变化，请重新检查');
    statusRows=statusRows.filter(r=>!(r.accountKey===row.accountKey&&r.sourceId===row.sourceId));statusRows.push(row);repaint();
    if(!antigravityRefreshTasks.has(sourceId)){
      const work=(async()=>{await syncIdentitySettings(true);repaint();await quotas(account.id);await loadDashboard();})()
        .catch(error=>notify('登录已确认，后续刷新失败：'+String(error),'error'))
        .finally(()=>antigravityRefreshTasks.delete(sourceId));
      antigravityRefreshTasks.set(sourceId,work);
    }
  }
  async function refreshCodexSource(sourceId) {
    sourceStatusRequests.set(sourceId, ++statusRevision);
    if(sourceStatusTasks.has(sourceId))return sourceStatusTasks.get(sourceId);
    const work=(async()=>{
      while(true){
        const requested=sourceStatusRequests.get(sourceId), rows=[];
        let succeeded=true;
        if(!state.settings.sources.some(s=>s.id===sourceId&&s.provider==='codex'&&s.enabled))return false;
        for(const a of state.settings.accounts.filter(a=>a.provider==='codex')){
          try{
            const row=await api('accounts.status.refresh',{accountKey:accountKey(a),sourceId});
            rows.push(row);if(row.error)succeeded=false;
          }catch{succeeded=false;}
        }
        if(requested!==sourceStatusRequests.get(sourceId))continue;
        for(const row of rows){
          if(!state.settings.accounts.some(a=>accountKey(a)===row.accountKey))continue;
          statusRows=statusRows.filter(r=>!(r.accountKey===row.accountKey&&r.sourceId===row.sourceId));statusRows.push(row);
        }
        repaint();return succeeded;
      }
    })().finally(()=>sourceStatusTasks.delete(sourceId));
    sourceStatusTasks.set(sourceId,work);return work;
  }
  async function syncDeployments(){if(syncRunning)return;syncRunning=true;try{const r=await api('accounts.deployments.sync');syncMessage=r.failedTasks?.length?'配置已保存，部分唤醒部署待同步，可重试。':'';}catch(error){syncMessage='配置已保存，唤醒部署待同步：'+String(error);}finally{syncRunning=false;repaint();}}
  function backgroundRefresh(){clearTimeout(refreshTimer);refreshTimer=setTimeout(()=>{void loadDashboard().catch(error=>notify('配置已保存，刷新失败：'+String(error),'error'));},300);void syncDeployments();}

  const configurations = () => state.settings.agents ?? Object.keys(providers).filter(p=>p!=='agy').map(provider=>({provider,enabled:state.settings.sources.some(s=>s.provider===provider&&s.enabled),machineIds:[...new Set(state.settings.sources.filter(s=>s.provider===provider&&s.enabled).map(s=>s.hostId||'local'))]}));
  const machineName = s => s.hostId ? state.settings.hosts.find(h=>h.id===s.hostId)?.name || s.hostId : '本机';
  const profiles = a => [...new Set([a.quotaProfileId,...(a.connections??[]).map(c=>c.profileId)].filter(Boolean))];
  const linked = (a,s) => s.provider===a.provider&&(s.accountId===a.id||s.codexHomeId&&profiles(a).some(r=>r.startsWith(s.codexHomeId+':')));
  function accept(saved) {
    const added=saved.accounts.filter(a=>!state.settings.accounts.some(old=>old.provider===a.provider&&old.id===a.id));
    const pref=AieyesUI.panelPreference();
    for(const a of added){const choice=pref.providers[a.provider];if(choice?.mode==='custom'&&!a.archived&&a.quotaEnabled){const keys=AieyesUI.panelAccounts(saved,[],pref)[a.provider]??[];if(keys.length<5&&!keys.includes(a.provider+':'+a.id))choice.keys=[...keys,a.provider+':'+a.id];}}
    if(added.length){try{localStorage.setItem('aieyes.panel.accounts.v2',JSON.stringify(pref));}catch{notify('账户已保存，本机面板偏好暂时无法写入','error');}}
    statusLoaded=false;state.settings=saved;state.settingsDraft=structuredClone(saved);state.settingsBaseline=configKey(saved);state.lastQuota=0;
    window.AieyesTheme?.apply(saved.appearance);
  }
  async function commit(next, base=state.settings) {
    if(state.settingsSaving)throw new Error('正在保存，请稍后重试');
    state.settingsSaving=true;
    try { accept(await api('settings.patch',{base,settings:next}));markSaved(); }
    finally { state.settingsSaving=false; scheduleRefreshes(); }
    backgroundRefresh();
  }
  async function run(action) {
    if(state.settingsSaving)return;
    try { await action(); } catch(error) { notify(error.message??String(error),'error'); }
    finally { if(state.page==='settings')renderSettings(false); }
  }
  function agentsHTML() {
    if(directorySelection){const {provider,machine}=directorySelection;return `<button id="directories-back">返回 Agents</button><section class="card"><h2>${e(providers[provider])} · ${e(deviceName(machine))}</h2>${state.settings.sources.filter(s=>s.provider===provider&&machineId(s)===machine).map(s=>`<div class="list-row"><code class="row-body">${e(s.path)}</code><button data-source-advanced="${e(s.id)}">编辑目录</button><button class="danger" data-source-remove="${e(s.id)}">移除目录配置</button></div>`).join('')}<button data-source-add="${e(provider)}">添加目录</button><p class="muted">只配置采集位置，不移动文件。远程 Shell 位于服务器设置，前置命令位于账户设置。</p></section>`;}
    return configurations().map(a=>`<section class="card agent-setup"><div class="between"><h2>${providerMark(a.provider)} ${e(providers[a.provider])}</h2><label><input type="checkbox" role="switch" data-agent-enable="${e(a.provider)}" ${state.settingsSaving?'disabled':''} ${a.enabled?'checked':''}>启用</label></div>${a.provider==='deepseek'?'<p class="muted">在账户中添加 API Key，通过本机查询余额。</p>':`<fieldset class="agent-machines"><legend>所在机器 · 可多选</legend>${[{id:'local',name:'本机',enabled:true},...state.settings.hosts].map(h=>`<div class="list-row"><label class="row-body"><input type="checkbox" role="switch" data-agent-machine="${e(a.provider)}" ${state.settingsSaving||!a.enabled?'disabled':''} value="${e(h.id)}" ${a.machineIds.includes(h.id)?'checked':''}>${e(h.name||h.target)}${h.enabled?'':' · 服务器已停用'}</label><button data-machine-directories="${e(a.provider)}" data-machine="${e(h.id)}">目录设置</button></div>`).join('')}</fieldset>${a.machineIds.length?'':'<p class="muted">未选择机器，暂不采集。</p>'}${state.settings.hosts.length?'':'<button data-agent-host>添加服务器</button>'}`}</section>`).join('');
  }
  function accountsHTML() {
    if(showCodex)return '<button id="accounts-back">返回账户</button>'+window.AieyesCodexAuth.html();
    const detail=state.settings.accounts.find(a=>accountKey(a)===detailKey);
    if(detail)return detailsHTML(detail);
    const row=a=>`<div class="list-row"><div class="row-body"><strong>${e(a.name)}</strong><small>${e(counts(a))}</small>${statusRows.find(r=>r.accountKey===accountKey(a)&&r.current===true&&r.identity)?`<small class="account-identity">${e(identityText(statusRows.find(r=>r.accountKey===accountKey(a)&&r.current===true&&r.identity)))}</small>`:''}</div><button data-account-edit="${e(accountKey(a))}">设置</button></div>`;
    const groups=configurations().filter(config=>config.enabled||state.settings.accounts.some(a=>a.provider===config.provider&&!a.archived));
    return `<div class="card"><div class="between"><button id="account-status-refresh" ${statusLoading?'disabled':''}>${statusLoading?'检查中…':'刷新设备状态'}</button></div><p class="muted">一个账户就是一个实际账号，可在多台机器登录。“已登录”表示机器保存了该账号的登录；“使用中”表示当前选用该账号。离线状态以上次检查为准。</p>${syncHTML()}</div>`+groups.map(config=>`<section class="card account-agent-group"><div class="between"><h2>${providerMark(config.provider)} ${e(providers[config.provider])}</h2><button data-account-add="${e(config.provider)}" class="primary" ${config.enabled?'':'disabled'}>添加账户</button></div>${state.settings.accounts.filter(a=>a.provider===config.provider&&!a.archived).map(row).join('')||'<p class="muted">尚未添加账户</p>'}<div class="account-machine-summary">${machines(config.provider).map(id=>{
      const sources=state.settings.sources.filter(s=>s.provider===config.provider&&machineId(s)===id&&s.enabled);
      const rows=statusRows.filter(r=>r.machineId===id&&r.accountKey.startsWith(config.provider+':')&&sources.some(s=>s.id===r.sourceId));
      const names=state.settings.accounts.filter(a=>a.provider===config.provider&&rows.some(r=>r.accountKey===accountKey(a)&&r.current===true)).map(a=>a.name);
      const stateText=rows.some(r=>r.identity)?identityText(rows.find(r=>r.identity)):rows.some(r=>r.authenticated===true)?'登录有效 · 账户身份待确认':names.length?'正在使用 '+names.join('、'):rows.some(loggedIn)?'已登录，尚未选用':!rows.length||rows.some(r=>r.current==null||r.credential==null||r.error)?'登录状态待确认':'未登录';
      const source=sources.find(s=>rows.some(r=>r.sourceId===s.id&&(r.current===true||r.credential===true)))||sources[0];
      return `<div class="list-row"><span>${e(deviceName(id))}</span><span class="row-body muted">${e(stateText)}</span>${config.provider==='antigravity'&&source?`<button data-agy-login="${e(source.id)}">登录 / 检查</button>`:''}${config.provider==='codex'&&source?(rows.some(r=>r.current===true||r.credential===true)?`<button data-device-manage="${e(source.id)}" data-intent="switch">切换账户</button>`:`<button data-device-manage="${e(source.id)}" data-intent="login">登录</button>`):''}</div>`;
    }).join('')}</div></section>`).join('')+
    (state.settings.accounts.some(a=>a.archived)?`<details class="card" data-agent-detail="archived-accounts"><summary>已归档账户</summary>${state.settings.accounts.filter(a=>a.archived).map(a=>`<div class="list-row"><span class="row-body">${e(providers[a.provider])} · ${e(a.name)}</span><button data-account-restore="${e(accountKey(a))}">恢复</button><button class="danger" data-account-delete="${e(accountKey(a))}">删除账户</button></div>`).join('')}</details>`:'')+
    (cleanupRows.length?`<details class="card" data-agent-detail="abandoned-tasks"><summary>已解除管理的远端任务（可能仍在运行）</summary>${cleanupRows.map(r=>`<h3>${e(r.name)}</h3>${r.tasks.map(t=>`<p>${e(t.name)} · ${e(t.machine)} · <code>${e(t.root)}</code></p>`).join('')}`).join('')}</details>`:'');
  }
  function syncHTML(){return syncMessage?`<p class="error">${e(syncMessage)} <button data-deployment-sync>重试同步</button></p>`:'';}
  function legacyCommand(s){return s.quotaPreCommand||state.settings.hosts.find(h=>h.id===s.hostId)?.preCommand||'';}
  function deviceCommand(a,id){const configured=a.deviceSettings?.find(d=>d.machineId===id);if(configured)return configured.preCommand;const values=[...new Set(state.settings.sources.filter(s=>s.provider===a.provider&&machineId(s)===id).map(legacyCommand))];return values.length===1?values[0]:'';}
  const draftValue=(a,field,fallback)=>accountDrafts.get(accountKey(a)+':'+field)??fallback;
  function queueAccountField(a,field,value){
    const key=accountKey(a), draftKey=key+':'+field;
    accountDrafts.set(draftKey,value);pendingSaves++;
    saveQueue=saveQueue.catch(()=>{}).then(async()=>{
      try {
        if(field==='name'&&!String(value).trim())throw new Error('请输入账户名称');
        if(field==='quotaRefreshSeconds'&&value!==''&&(!/^\d+$/.test(String(value))||Number(value)<30||Number(value)>86400))throw new Error('查询间隔范围为 30–86400 秒');
        const next=structuredClone(state.settings), row=next.accounts.find(a=>accountKey(a)===key);if(!row)throw new Error('账户已移除');
        if(field.startsWith('command:')){const machine=field.slice(8);row.deviceSettings=(row.deviceSettings??[]).filter(d=>d.machineId!==machine);row.deviceSettings.push({machineId:machine,preCommand:value});}
        else row[field]=field==='quotaRefreshSeconds'?(value===''?null:Number(value)):field==='name'?value.trim():value;
        await commit(next);accountErrors.delete(draftKey);
        if(accountDrafts.get(draftKey)===value)accountDrafts.delete(draftKey);
      }catch(error){accountErrors.set(draftKey,error.message??String(error));notify(error.message??String(error),'error');}
      finally{pendingSaves--;repaint();}
    });return saveQueue;
  }
  function detailsHTML(a){
    const selected=AieyesUI.panelAccounts(state.settings,state.dashboard?.quotaOrder??[]),isSelected=selected[a.provider]?.includes(accountKey(a));
    return `<div class="between"><button id="account-detail-back">返回账户</button><button id="account-status-refresh" ${statusLoading?'disabled':''}>${statusLoading?'检查中…':'刷新设备状态'}</button></div><section class="card account-config"><h2>${providerMark(a.provider)} ${e(a.name)}</h2><p>${e(counts(a))}</p>${state.dashboard?.quotas?.find(q=>q.provider===a.provider&&q.accountId===a.id)?.error?`<p class="error">额度查询：${e(state.dashboard.quotas.find(q=>q.provider===a.provider&&q.accountId===a.id).error)}</p>`:''}<label>优先查询位置<select data-account-priority="${e(accountKey(a))}">${option('','自动 · 优先本机',draftValue(a,'quotaSourceId',a.quotaSourceId)||'')}${state.settings.sources.filter(s=>linked(a,s)).map(s=>option(s.id,deviceName(machineId(s))+' · '+s.name+(s.enabled?'':' · 已暂停'),draftValue(a,'quotaSourceId',a.quotaSourceId)||'')).join('')}</select></label><div class="actions"><label><input type="checkbox" data-account-quota="${e(accountKey(a))}" ${draftValue(a,'quotaEnabled',a.quotaEnabled)?'checked':''}>显示并查询额度</label><label><input type="checkbox" data-account-panel="${e(accountKey(a))}" ${isSelected?'checked':''} ${!a.quotaEnabled?'disabled':''}>在面板显示</label><button data-account-panel-manage>调整面板选择</button>${a.provider!=='codex'?`<button data-account-connect="${e(accountKey(a))}">连接机器</button>`:''}</div><label>查询间隔（秒）<input id="account-interval" data-account-field="quotaRefreshSeconds" data-account="${e(accountKey(a))}" type="number" min="30" max="86400" value="${e(draftValue(a,'quotaRefreshSeconds',a.quotaRefreshSeconds??''))}" placeholder="沿用全局 · ${state.settings.refreshSeconds} 秒"></label><label>账户名称<input id="account-name" data-account-field="name" data-account="${e(accountKey(a))}" value="${e(draftValue(a,'name',a.name))}"></label><label><input type="checkbox" data-account-archive="${e(accountKey(a))}" ${draftValue(a,'archived',a.archived)?'checked':''}>归档账户</label><p class="muted tiny">选择项即时保存；文本离开输入框保存。${pendingSaves?'正在保存…':''}</p>${[...accountErrors].filter(([key])=>key.startsWith(accountKey(a)+':')).map(([key,message])=>`<p class="error">${e(message)} <button data-account-retry="${e(key)}">重试保存</button></p>`).join('')}${syncHTML()}</section>`+
    machines(a.provider,true).map(id=>{
      const sources=state.settings.sources.filter(s=>s.provider===a.provider&&machineId(s)===id),active=machines(a.provider).includes(id),rows=statuses(a,id);
      const legacy=[...new Set(sources.map(legacyCommand))],conflict=legacy.length>1&&!a.deviceSettings?.some(d=>d.machineId===id);
      return `<section class="card"><h3>${e(deviceName(id))}${active?'':' · 已暂停'}</h3><p class="muted">${e(statusText(rows))}</p>${sources.map(s=>{const r=rows.find(r=>r.sourceId===s.id);return `<div class="list-row"><div class="row-body">${e(s.name)}<small>${e(s.path)}</small>${r?.checkedAt?`<small>检查于 ${e(new Date(r.checkedAt*1000).toLocaleString())}</small>`:''}${r?.error||r?.note?`<small>${e(r.error||r.note)}</small>`:''}</div>${a.provider==='antigravity'&&s.accountId===a.id?`<button data-agy-login="${e(s.id)}" ${active&&s.enabled?'':'disabled'}>登录 / 检查</button>`:''}${a.provider==='codex'?(r?.current===true?'<span class="muted">正在使用</span>':r?.credential===true?`<button data-account-login="${e(accountKey(a))}" data-source="${e(s.id)}" data-intent="switch" ${active&&s.enabled?'':'disabled'}>切换到此账户</button>`:`<button data-account-login="${e(accountKey(a))}" data-source="${e(s.id)}" data-intent="login" ${active&&s.enabled?'':'disabled'}>登录</button>`):''}</div>`;}).join('')}${id!=='local'?`<details data-agent-detail="command:${e(accountKey(a))}:${e(id)}"><summary>前置命令</summary><p class="tiny muted">用于此账户的授权、切换、额度查询和定时唤醒；历史同步、监控不执行。清空后不执行。</p>${conflict?'<p class="error">旧目录命令不同，暂分别保留。编辑完成后统一使用新命令。</p>'+sources.map(s=>`<p>${e(s.name)}：<code>${e(legacyCommand(s))}</code></p>`).join(''):''}<label>账户前置命令<textarea id="account-command-${e(id)}" data-command-field="${e(id)}" data-account="${e(accountKey(a))}">${e(draftValue(a,'command:'+id,deviceCommand(a,id)))}</textarea></label></details>`:''}</section>`;
    }).join('');
  }
  async function deleteAccount(a){
    let preview;try{preview=await api('accounts.deletion.preview',{accountKey:accountKey(a)});}catch(error){notify(String(error),'error');return;}
    const tasks=preview.tasks??[];
    showEditor('删除归档账户「'+a.name+'」','<p>删除账户配置和关联任务；历史记录及机器凭证保留，不退出 Agent。</p><ul>'+tasks.map(t=>`<li>${e(t.name)} · ${e(t.machine)}</li>`).join('')+'</ul>'+ (tasks.length?'<label><input type="checkbox" name="force">强制删除（设备已弃用或无法连接）</label><p class="error">强制删除无法保证远端任务停止。将解除本地管理，不再自动重试；未清理位置保留供查看。</p>':'')+'<div id="account-delete-failures"></div>',async f=>{
      const result=await api('accounts.delete',{accountKey:accountKey(a),force:f.has('force')});
      if(!result.deleted){$('#account-delete-failures').innerHTML=(result.failedTasks??[]).map(t=>`<p class="error">${e(t.name)} · ${e(t.error)}</p>`).join('');throw new Error('部分任务清理失败，账户仍保留；可重试或勾选强制删除。');}
      accept(result.settings);cleanupRows=await api('accounts.cleanup.list');if(!Array.isArray(cleanupRows))cleanupRows=[];repaint();backgroundRefresh();
    });$('#editor-form button[type=submit]').textContent='确认删除';
  }
  function advanced(existing, provider) {
    const base=structuredClone(state.settings), item=structuredClone(existing??{id:crypto.randomUUID(),provider,name:providers[provider],accountId:'',path:{codex:'~/.codex',claude:'~/.claude',antigravity:'~/.gemini/antigravity-cli',custom:'~/.aieyes'}[provider]||'',hostId:directorySelection?.machine==='local'?null:directorySelection?.machine??null,enabled:true,codexBinary:'codex',agyBinary:'agy',quotaCommand:'',quotaPreCommand:'',proxy:null});
    showEditor(providers[item.provider]+' · 目录设置',select('hostId','机器',[['','本机'],...base.hosts.map(h=>[h.id,h.name||h.target])],item.hostId||'')+field('path',item.provider==='deepseek'?'API Key 文件':'数据目录',item.path)+(item.codexHomeId?'<p class="muted">更换目录将解除旧位置的账户连接；原文件和凭证保留，请在账户中读取新目录的登录信息。</p>':'')+(item.provider==='codex'?field('codexBinary','Codex 程序',item.codexBinary):'')+(item.provider==='antigravity'?field('agyBinary','agy 程序',item.agyBinary):'')+(item.provider==='custom'?textarea('quotaCommand','限额查询命令',item.quotaCommand||''):'')+proxyFields('source',item.proxy,true),async f=>{
      item.path=f.get('path').trim();if(!['antigravity','deepseek'].includes(item.provider)&&!item.path)throw new Error('请输入数据目录');
      for(const key of ['codexBinary','agyBinary','quotaCommand'])if(f.has(key))item[key]=f.get(key);
      item.proxy=proxyValue(f,'source');
      accept(await api('sources.configure',{source:item,...(existing?{baseSource:existing}:{})}));markSaved();backgroundRefresh();renderSettings(false);
    });
    $('#field-hostId').disabled=true;bindProxy('source');bindPathSelection(true);
  }
  function removeDirectory(source){showEditor('移除目录配置','<p>仅解除此目录的采集和账户连接；文件、登录凭证和历史记录保留。</p><code>'+e(source.path)+'</code>',async()=>{accept(await api('sources.remove',{sourceId:source.id,baseSource:source}));markSaved();backgroundRefresh();renderSettings(false);});$('#editor-form button[type=submit]').textContent='移除配置';}
  function editAccount(account){detailKey=accountKey(account);showCodex=false;renderSettings(false);}
  function addAccount(account, selectedProvider) {
    const enabled=configurations().filter(a=>a.enabled), initial=account?.provider||selectedProvider||enabled[0]?.provider;
    if(!initial)return;
    if(initial==='codex'){detailKey=account?accountKey(account):'';showCodex=true;window.AieyesCodexAuth.open(account?.id||'');renderSettings(false);return;}
    let secret=null, secretSourceId=crypto.randomUUID();
    showEditor(providers[initial]+' · '+(account?'连接机器':'添加账户'),`<input id="field-provider" type="hidden" name="provider" value="${e(initial)}">`+'<div id="account-connection-fields"></div>',async f=>{
      const provider=f.get('provider')||account?.provider, accountId=account?.id||'';
      const name=account?.name||providers[provider]+' 账户',base=structuredClone(state.settings);
      if(provider==='deepseek'){
        const key=f.get('apiKey')?.trim();if(!key)throw new Error('请输入 API Key');
        const next=structuredClone(base),id=accountId||crypto.randomUUID();
        if(!accountId)next.accounts.push({id,provider,name,quotaEnabled:true,archived:false,connections:[]});
        let source=next.sources.find(s=>s.provider===provider&&s.accountId===id);
        if(!source){source={id:secretSourceId,provider,name:name||'DeepSeek',accountId:id,hostId:null,enabled:true,path:'',quotaCommand:'',quotaPreCommand:'',codexBinary:'codex',agyBinary:'agy',proxy:null};next.sources.push(source);}
        if(!secret||secret.key!==key){const result=await api('credentials.save',{sourceId:secretSourceId+'-'+crypto.randomUUID(),apiKey:key});secret={key,path:result.path};}
        source.path=secret.path;await commit(next,base);
      } else {const result=await api('accounts.connect',{provider,accountId,name,sourceIds:f.getAll('sourceIds')});accept(result.settings);markSaved();backgroundRefresh();}
      renderSettings(false);
    });
    const draw=()=>{
      const provider=$('#field-provider').value;
      $('#account-connection-fields').innerHTML='<p class="muted">'+(provider==='deepseek'?'添加账号的 API Key，之后可修改显示名称。':'读取所选机器上已有账号的登录。')+'</p>'+(provider==='deepseek'?field('apiKey','API Key','','','password'):'<fieldset id="account-source-options"><legend>已有登录的机器 · 可多选</legend></fieldset>');
      const drawSources=()=>{
        const id=account?.id||'';
        if($('#account-source-options'))$('#account-source-options').innerHTML=state.settings.sources.filter(s=>s.enabled&&s.provider===provider&&(!s.accountId||s.accountId===id)).map(s=>`<label><input type="checkbox" name="sourceIds" value="${e(s.id)}">${e(s.name)} · ${e(machineName(s))}</label>${provider==='antigravity'?`<button type="button" data-agy-login="${e(s.id)}">登录 / 检查</button>`:''}`).join('')||'<p>没有可用位置。请先在 Agents 选择机器；同一目录只能连接一个当前登录账户。</p>';
      };drawSources();
      $('#editor-form button[type=submit]').textContent=account?'保存连接':'添加账号';
    };draw();
  }
  function bind() {
    document.querySelectorAll('[data-agent-enable],[data-agent-machine]').forEach(input=>input.onchange=()=>run(async()=>{
      const provider=input.dataset.agentEnable||input.dataset.agentMachine,a=configurations().find(a=>a.provider===provider);
      const params=input.dataset.agentEnable?{provider,enabled:input.checked,...(input.checked&&!a.machineIds.length?{machineIds:['local']}:{})}:{provider,machineIds:[...document.querySelectorAll('[data-agent-machine]')].filter(i=>i.dataset.agentMachine===provider&&i.checked).map(i=>i.value)};
      state.settingsSaving=true;document.querySelectorAll('[data-agent-enable],[data-agent-machine]').forEach(i=>i.disabled=true);
      try{accept(await api('agents.set',params));markSaved();backgroundRefresh();}finally{state.settingsSaving=false;scheduleRefreshes();}
    }));
    if($('#directories-back'))$('#directories-back').onclick=()=>{directorySelection=null;renderSettings(false);};
    document.querySelectorAll('[data-machine-directories]').forEach(b=>b.onclick=()=>{directorySelection={provider:b.dataset.machineDirectories,machine:b.dataset.machine};renderSettings(false);});
    document.querySelectorAll('[data-source-remove]').forEach(b=>b.onclick=()=>removeDirectory(state.settings.sources.find(s=>s.id===b.dataset.sourceRemove)));
    document.querySelectorAll('[data-source-advanced]').forEach(b=>b.onclick=()=>advanced(state.settings.sources.find(s=>s.id===b.dataset.sourceAdvanced)));
    document.querySelectorAll('[data-source-add]').forEach(b=>b.onclick=()=>advanced(null,b.dataset.sourceAdd));
    document.querySelectorAll('[data-agent-host]').forEach(b=>b.onclick=()=>{state.settingsTab='hosts';renderSettings(false);editItem();});
    if($('#accounts-back'))$('#accounts-back').onclick=()=>{showCodex=false;statusLoaded=false;renderSettings(false);};
    if($('#account-detail-back'))$('#account-detail-back').onclick=()=>{detailKey='';renderSettings(false);};
    if($('#account-status-refresh'))$('#account-status-refresh').onclick=()=>refreshStatuses(true);
    document.querySelectorAll('[data-deployment-sync]').forEach(b=>b.onclick=syncDeployments);
    if(state.settingsTab==='accounts'&&!showCodex&&!statusLoaded&&!statusLoading)void refreshStatuses(true);
    document.querySelectorAll('[data-account-add]').forEach(b=>b.onclick=()=>addAccount(null,b.dataset.accountAdd));
    const account=key=>state.settings.accounts.find(a=>a.provider+':'+a.id===key);
    document.querySelectorAll('[data-account-edit]').forEach(b=>b.onclick=()=>{detailKey=b.dataset.accountEdit;renderSettings(false);});
    document.querySelectorAll('[data-account-options]').forEach(b=>b.onclick=()=>editAccount(account(b.dataset.accountOptions)));
    document.querySelectorAll('[data-account-delete]').forEach(b=>b.onclick=()=>deleteAccount(account(b.dataset.accountDelete)));
    document.querySelectorAll('[data-account-field],[data-command-field]').forEach(input=>{
      const a=account(input.dataset.account),field=input.dataset.accountField||'command:'+input.dataset.commandField,key=accountKey(a)+':'+field;
      input.oninput=()=>accountDrafts.set(key,input.value);
      input.onchange=()=>queueAccountField(a,field,input.value);
    });
    document.querySelectorAll('[data-account-retry]').forEach(b=>b.onclick=()=>{const key=b.dataset.accountRetry,a=account(detailKey),field=key.slice(detailKey.length+1);void queueAccountField(a,field,accountDrafts.get(key));});
    document.querySelectorAll('[data-account-archive]').forEach(b=>b.onchange=()=>queueAccountField(account(b.dataset.accountArchive),'archived',b.checked));
    document.querySelectorAll('[data-account-priority]').forEach(b=>b.onchange=()=>queueAccountField(account(b.dataset.accountPriority),'quotaSourceId',b.value||null));
    document.querySelectorAll('[data-account-quota]').forEach(b=>b.onchange=()=>queueAccountField(account(b.dataset.accountQuota),'quotaEnabled',b.checked));
    document.querySelectorAll('[data-account-login]').forEach(b=>b.onclick=async()=>{if(!await flush())return;showCodex=true;window.AieyesCodexAuth.open(account(b.dataset.accountLogin).id,b.dataset.source,b.dataset.intent);renderSettings(false);});
    document.querySelectorAll('[data-device-manage]').forEach(b=>b.onclick=()=>{showCodex=true;window.AieyesCodexAuth.open('',b.dataset.deviceManage,b.dataset.intent);renderSettings(false);});
    document.querySelectorAll('[data-account-connect]').forEach(b=>b.onclick=()=>{const a=account(b.dataset.accountConnect);if(a.provider==='codex'){showCodex=true;window.AieyesCodexAuth.open(a.id);renderSettings(false);}else addAccount(a);});
    document.querySelectorAll('[data-account-restore]').forEach(b=>b.onclick=()=>run(async()=>{const next=structuredClone(state.settings),a=next.accounts.find(a=>a.provider+':'+a.id===b.dataset.accountRestore);a.archived=false;await commit(next);}));
    document.querySelectorAll('[data-account-panel]').forEach(i=>i.onchange=()=>{const a=account(i.dataset.accountPanel),pref=AieyesUI.panelPreference(),keys=(AieyesUI.panelAccounts(state.settings,state.dashboard?.quotaOrder??[],pref)[a.provider]??[]).filter(k=>k!==i.dataset.accountPanel);if(i.checked&&keys.length<5)keys.push(i.dataset.accountPanel);pref.providers[a.provider]={mode:'custom',keys};try{localStorage.setItem('aieyes.panel.accounts.v2',JSON.stringify(pref));renderSettings(false);}catch(error){i.checked=!i.checked;notify('无法保存面板选择：'+error,'error');}});
    document.querySelectorAll('[data-account-panel-manage]').forEach(b=>b.onclick=openPanelAccounts);
    if(showCodex)window.AieyesCodexAuth.bind();
  }
  async function flush(){
    document.activeElement?.blur();await saveQueue;
    for(const [key,value] of [...accountDrafts]){const a=state.settings.accounts.find(a=>key.startsWith(accountKey(a)+':'));if(a)await queueAccountField(a,key.slice(accountKey(a).length+1),value);}
    if(accountErrors.size){notify('账户设置尚未保存，请修正输入或重试。','error');return false;}return true;
  }
  window.AieyesAgentSettings={flush,agentsHTML,accountsHTML,bind,commit,accept,advanced,editAccount,addAccount,profiles,linked,refreshStatuses,refreshCodexSource,refreshAntigravitySource,syncIdentitySettings,syncDeployments,get antigravityRefreshing(){return antigravityRefreshTasks.size>0;},openDetails(a){detailKey=accountKey(a);showCodex=false;state.page='settings';state.settingsTab='accounts';render();}};
})();

// A separate modal keeps the connection form intact during the CLI's authorization flow.
window.AieyesAgyAuth={busy:false};
document.addEventListener('click',event=>{const button=event.target.closest('[data-agy-login]');if(button)openAntigravityLogin(button.dataset.agyLogin);});
async function openAntigravityLogin(sourceId) {
  if(document.querySelector('#agy-login'))return;
  const dialog=document.createElement('dialog');dialog.id='agy-login';dialog.setAttribute('aria-label','Antigravity 登录');
  dialog.innerHTML='<h2>Antigravity 登录</h2><p>复用此机器的现有登录，或完成浏览器授权。登录后显示邮箱与订阅，登录凭据保存在原机器。</p><p id="agy-login-status" role="status"></p><p id="agy-login-identity"></p><p id="agy-login-metadata" class="muted"></p><div id="agy-auth-url" hidden><button type="button" id="agy-open-url">打开授权网页</button><button type="button" id="agy-copy-url">复制授权网址</button><label>浏览器返回的授权码<input id="agy-code" type="password" autocomplete="off"></label><button type="button" id="agy-submit-code">提交授权码</button></div><p id="agy-login-error" class="error"></p><div class="actions"><button id="agy-trust" hidden>确认临时目录</button><button id="agy-verify">验证登录</button><button id="agy-retry" hidden>重新登录 / 检查</button><button id="agy-close">完成</button></div>';
  document.body.append(dialog);dialog.showModal();
  let session=null,sending=false,closed=false,refreshedSession=null;
  const active=()=>!session||['checking','starting','authorizing','verifying'].includes(session.phase);
  function draw(){AieyesAgyAuth.busy=active()||sending;dialog.querySelector('#agy-login-status').textContent=session?.message||'正在检查现有登录…';dialog.querySelector('#agy-login-identity').textContent=session?.identity?[session.identity.email,session.identity.subscription||'订阅未知'].join(' · '):'';dialog.querySelector('#agy-login-metadata').textContent=session?.metadataError||'';dialog.querySelector('#agy-auth-url').hidden=!session?.authUrl;dialog.querySelector('#agy-trust').hidden=session?.workspaceConfirmationRequired!==true;dialog.querySelector('#agy-trust').disabled=sending;dialog.querySelector('#agy-submit-code').disabled=sending;dialog.querySelector('#agy-retry').disabled=sending;dialog.querySelector('#agy-retry').hidden=active();dialog.querySelector('#agy-verify').disabled=sending||!session||['checking','verifying'].includes(session.phase);dialog.querySelector('#agy-close').disabled=sending;dialog.querySelector('#agy-close').textContent=active()?'取消授权':'完成';}
  async function call(method,extra={}){if(sending)return;sending=true;if(['start','verify'].includes(method))refreshedSession=null;draw();try{session=await api('agyAuth.'+method,{sourceId,id:session?.id,...extra});dialog.querySelector('#agy-login-error').textContent='';draw();if(session?.authenticated&&refreshedSession!==session.id){await window.AieyesAgentSettings.refreshAntigravitySource(sourceId,session.accountStatus);refreshedSession=session.id;}}catch(error){dialog.querySelector('#agy-login-error').textContent=String(error);}finally{sending=false;draw();}}
  async function close(){if(sending)return;if(active()&&session)await call('cancel');closed=true;AieyesAgyAuth.busy=false;dialog.close();dialog.remove();}
  dialog.oncancel=e=>{e.preventDefault();close();};dialog.querySelector('#agy-close').onclick=close;
  dialog.querySelector('#agy-verify').onclick=()=>call('verify');dialog.querySelector('#agy-retry').onclick=()=>call('start');dialog.querySelector('#agy-trust').onclick=()=>call('confirmWorkspace');
  dialog.querySelector('#agy-submit-code').onclick=()=>{const input=dialog.querySelector('#agy-code'),code=input.value;input.value='';call('submitCode',{code});};
  dialog.querySelector('#agy-open-url').onclick=()=>call('openUrl');
  dialog.querySelector('#agy-copy-url').onclick=async()=>{try{await navigator.clipboard.writeText(session.authUrl);}catch(error){dialog.querySelector('#agy-login-error').textContent='复制失败：'+String(error);}};
  draw();await call('start');
  while(!closed){await new Promise(resolve=>setTimeout(resolve,1000));if(!closed&&!sending&&session&&(active()||(session.authenticated&&refreshedSession!==session.id)))await call('status');}
}
