/* Connection credentials stay on the selected device. Switching uses the account preflight. */
window.AieyesCodexAPI = (() => {
  const e = value => escapeHTML(value ?? '');
  const sources = () => (state.settings?.sources ?? []).filter(s => s.provider === 'codex');
  const available = source => Boolean(source&&AieyesUI.sourceEnabled(state.settings,source));
  const connections = () => state.settings?.codexApis ?? [];
  const statuses = new Map(), pending = new Set();
  let busy = false, notice = '', error = '';
  const paint = () => { if (state.page === 'settings' && state.settingsTab === 'integrations' && state.settingsAgent === 'codex') renderSettings(false); };
  const location = source => source ? (source.hostId ? state.settings.hosts.find(h => h.id === source.hostId)?.name || source.hostId : '本机') + ' · ' + source.name + ' · ' + source.path : '来源已移除';
  async function status(sourceId) {
    const value = await api('codexApi.status', {sourceId});
    statuses.set(sourceId, value); return value;
  }
  function html() {
    if (!sources().length && !connections().length) return '';
    return '<section class="card codex-api-settings"><div class="between"><h2>Codex · API 连接</h2><button data-codex-api="add" '+(busy || !sources().length ? 'disabled' : '')+'>添加 API</button></div>'+
      '<p class="muted">添加 API 连接后可切换登录方式，沿用 Codex 的模型、推理强度和网络设置。</p>'+
      (error ? '<div class="error" role="alert">'+e(error)+'</div>' : '')+(notice ? '<p role="status">'+e(notice)+'</p>' : '')+
      connections().map(c => {
        const source=sources().find(s=>s.id===c.sourceId), selected=statuses.get(c.sourceId)?.selected;
        const active=selected?.connectionId===c.id, applied=active&&selected.revision===c.revision;
        return '<div class="api-connection"><div class="list-row"><div class="row-body"><strong>'+e(c.name)+(active?' · 当前 API':'')+
          '</strong><small>'+e(location(source))+'</small><small>'+e(c.baseUrl)+'</small>'+(active&&!applied?'<small>有已保存的更改，切换后生效</small>':'')+(!available(source)?'<small>采集已暂停或目录不可用；恢复后可切换。</small><button data-api-machine-settings>机器设置</button>':'')+
          '</div><button data-codex-api="edit" data-api-id="'+e(c.id)+'" '+(busy?'disabled':'')+'>设置</button></div><div class="actions">'+
          [['probe','测试连接'],['switch',applied?'正在使用':'切换到此 API'],['remove','移除']].map(([action,label])=>
          '<button data-codex-api="'+action+'" data-api-id="'+e(c.id)+'" '+(busy||!source||action==='switch'&&(!available(source)||applied)?'disabled':'')+'>'+label+'</button>').join('')+'</div></div>';
      }).join('')+
      (!connections().length?'<p class="muted">尚未添加 API 连接。密钥保存在所选设备。</p>':'')+
      [...new Set(connections().map(c=>c.sourceId))].map(id=>{
        const selected=statuses.get(id)?.selected;
        return '<div class="list-row"><span class="row-body tiny muted">'+e(location(sources().find(s=>s.id===id)))+' · '+
          (selected?selected.kind==='api'?'当前使用 '+e(selected.name||'API'):'当前使用账户':'状态待检查')+
          '</span><button data-codex-api="refresh" data-api-source="'+e(id)+'" '+(busy?'disabled':'')+'>刷新状态</button>'+
          '<button data-codex-api="account" data-api-source="'+e(id)+'" '+(busy||!available(sources().find(s=>s.id===id))?'disabled':'')+'>切换到已保存账户</button></div>';
      }).join('')+'</section>';
  }
  function bind() {
    for (const id of new Set(connections().map(c=>c.sourceId))) {
      if (statuses.has(id)||pending.has(id)) continue;
      pending.add(id); void status(id).catch(err=>{statuses.set(id,{});error=String(err.message??err);}).finally(()=>{pending.delete(id);paint();});
    }
  }
  function edit(existing) {
    const c={id:existing?.id||'',sourceId:existing?.sourceId||sources()[0]?.id,name:existing?.name||'OpenAI API',baseUrl:existing?.baseUrl||'https://api.openai.com/v1',revision:existing?.revision||''};
    showEditor(existing?'设置 Codex API':'添加 Codex API',
      select('apiSource','保存到设备',sources().map(s=>[s.id,location(s)]),c.sourceId)+
      field('apiName','连接名称',c.name)+field('apiBaseUrl','Base URL',c.baseUrl,'https://api.openai.com/v1')+
      field('apiKey','API Key','',existing?'留空保留已保存的密钥':'填写 API Key','password')+
      '<p class="tiny muted">支持 OpenAI 和兼容 Responses API 的服务。保存后可切换使用。</p>',
      async f=>{
        const connection={...c,sourceId:existing?c.sourceId:f.get('apiSource'),name:f.get('apiName').trim(),baseUrl:f.get('apiBaseUrl').trim()};
        const saved=await api('codexApi.save',{connection,apiKey:f.get('apiKey')});
        document.getElementById('field-apiKey').value='';
        window.AieyesAgentSettings.accept(saved.settings);statuses.delete(connection.sourceId);error='';notice='API 已保存，可检查并切换到此连接。';
        markSaved();paint();void loadDashboard();
      },'api');
    document.getElementById('field-apiSource').disabled=Boolean(existing);
    document.getElementById('field-apiKey').autocomplete='new-password';
  }
  async function switchConnection(sourceId, connection) {
    if(busy)return;
    busy=true;error='';notice='';paint();
    const dialog=quotaDialog('切换 Codex 登录方式','<p role="status">正在检查设备与切换目标…</p>','api');
    let prepared=null, profileId='', inspection=null, working=false, epoch=0;
    const originalClose=dialog.onclose;
    const finish=()=>{if(!working)dialog.close();};
    const setWorking=value=>{working=value;dialog.querySelector('#quota-close').disabled=value;if(value)dialog.dataset.busy='true';else delete dialog.dataset.busy;};
    dialog.onclose=()=>{epoch++;busy=false;delete dialog.dataset.busy;originalClose?.call(dialog);paint();};
    dialog.querySelector('#quota-close').onclick=finish;
    dialog.oncancel=event=>{event.preventDefault();if(!working)finish();};
    async function prepare() {
      const request=++epoch;setWorking(true);prepared=null;
      const body=dialog.querySelector('.quota-dialog-body');
      body.innerHTML='<p role="status">正在检查受影响的进程…</p>';
      try {
        const source=sources().find(s=>s.id===sourceId);
        if(!source?.codexHomeId){await api('codexAuth.enable',{sourceId});window.AieyesAgentSettings.accept(await api('settings.get'));}
        inspection=await api('codexAuth.inspect',{sourceId});
        if(!connection&&!profileId)profileId=inspection.profiles.find(p=>p.identity.key===inspection.storedIdentity)?.id||inspection.profiles[0]?.id||'';
        if(!connection&&!profileId)throw new Error('请先在此机器读取当前登录或登录账户。');
        const target=connection?{kind:'api',connectionId:connection.id,revision:connection.revision}:{kind:'account',profileId};
        const value=await api('codexAuth.switch.prepare',{sourceId,target});
        if(request!==epoch||!dialog.open)return;
        prepared=value;
      }catch(err){if(request===epoch)error=String(err.message??err);}
      finally{if(request===epoch){setWorking(false);draw();}}
    }
    function draw() {
      const rows=prepared?.processes??[], blocked=rows.some(p=>!p.canClose), body=dialog.querySelector('.quota-dialog-body');
      const name=connection?.name||inspection?.profiles.find(p=>p.id===profileId)?.name||'已保存账户';
      body.innerHTML='<p><strong>'+e(location(sources().find(s=>s.id===sourceId)))+' → '+e(name)+'</strong></p>'+
        (!connection&&inspection?.profiles.length?'<label>目标账户<select id="api-switch-profile">'+inspection.profiles.map(p=>'<option value="'+e(p.id)+'" '+(p.id===profileId?'selected':'')+'>'+e(p.name)+'</option>').join('')+'</select></label>':'')+
        '<p class="muted">确认后会关闭下面列出的受影响进程。已确认使用独立 API 配置的实例会保留；请先保存工作。</p>'+
        (error?'<div class="error" role="alert">'+e(error)+'</div>':'')+
        (prepared?rows.length?'<ul>'+rows.map(p=>'<li>'+e(p.name)+' · PID '+p.pid+(p.canClose?'':' · '+e(p.blockingReason))+'</li>').join('')+'</ul>':'<p>没有需要关闭的进程。</p>':'')+
        (prepared?.preservedProcesses?.length?'<p class="tiny muted">将保留 '+prepared.preservedProcesses.length+' 个独立 API 进程。</p>':'')+
        '<div class="actions"><button id="api-switch-cancel">取消</button><button id="api-switch-retry">重新检查</button><button class="primary" id="api-switch-confirm" '+(!prepared||blocked?'disabled':'')+'>'+(rows.length?'关闭并切换':'确认切换')+'</button></div>';
      body.querySelector('#api-switch-cancel').onclick=finish;
      body.querySelector('#api-switch-retry').onclick=()=>{error='';void prepare();};
      const picker=body.querySelector('#api-switch-profile');if(picker)picker.onchange=()=>{profileId=picker.value;error='';void prepare();};
      body.querySelector('#api-switch-confirm').onclick=async()=>{
        if(!prepared||working)return;setWorking(true);body.querySelectorAll('button,select').forEach(b=>b.disabled=true);
        try{
          const result=await api('codexAuth.switch.commit',{sourceId,operationId:prepared.operationId,closeProcesses:rows.length>0});
          if(result.status!=='succeeded')throw new Error(result.error||'切换尚未完成，请重新检查');
          prepared=null;
          notice=result.statusWarning||'登录方式已切换，可重新打开 Codex 并恢复会话。';
          try{
            window.AieyesAgentSettings.accept(await api('settings.get'));await status(sourceId);
            if(await window.AieyesAgentSettings.refreshCodexSource(sourceId)===false)throw new Error('账户状态待刷新');
            await loadDashboard();
          }catch(err){notice='登录方式已切换，状态刷新未完成；可点击刷新状态重试。';statuses.delete(sourceId);}
          setWorking(false);finish();
        }catch(err){error=String(err.message??err);prepared=null;draw();}
        finally{setWorking(false);}
      };
    }
    await prepare();
  }
  async function perform(action,c,sourceId) {
    if(action==='switch'||action==='account')return switchConnection(sourceId,action==='switch'?c:null);
    if(busy)return;busy=true;error='';notice='';paint();
    try{
      if(action==='refresh'){await status(sourceId);notice='已刷新登录方式。';}
      if(action==='probe'){const result=await api('codexApi.probe',{sourceId,id:c.id});notice=result.message+(result.httpStatus?' · HTTP '+result.httpStatus:'');}
    }catch(err){error=String(err.message??err);}finally{busy=false;paint();}
  }
  document.addEventListener('click',event=>{
    if(event.target.closest('[data-api-machine-settings]')){void openSettings({provider:'codex',section:'machines'});return;}
    const button=event.target.closest('[data-codex-api]');if(!button||busy)return;
    const action=button.dataset.codexApi,c=connections().find(v=>v.id===button.dataset.apiId);
    if(action==='add'||action==='edit')return edit(c);
    if(action==='remove'&&c){showEditor('移除 API 连接','<p>移除「'+e(c.name)+'」的连接与密钥，历史用量保留。</p>',async()=>{
      const saved=await api('codexApi.remove',{sourceId:c.sourceId,id:c.id,revision:c.revision});window.AieyesAgentSettings.accept(saved.settings);statuses.delete(c.sourceId);markSaved();paint();
    });return;}
    void perform(action,c,button.dataset.apiSource||c?.sourceId);
  });
  return {html,bind};
})();
