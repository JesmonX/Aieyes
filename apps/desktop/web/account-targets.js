/* Shared account/machine projection; credential handling stays in the core. */
(() => {
  const e = v => AieyesUI.escapeHTML(v ?? '');
  let dialog, accountKey = '', rows = [], errors = [], checking = false, embedded = false, generation = 0;
  const eligible = key => {
    const a = state.settings?.accounts.find(a => a.provider+':'+a.id === key && !a.archived);
    return a?.provider === 'codex' && state.settings.agents?.some(g => g.provider === 'codex' && g.enabled && g.machineIds.some(id => id === 'local' || state.settings.hosts.some(h => h.id === id && h.enabled)));
  };
  async function refresh(key, changed) {
    let targets = await api('accounts.targets', key ? {accountKey:key} : {});
    changed?.(targets, []);
    const sources = [...new Set(targets.filter(t => t.provider === 'codex').map(t => t.sourceId))];
    const issues = [];
    await Promise.all(sources.map(async sourceId => {
      try {
        const result = await api('codexAuth.reconcile', {sourceId, accountId:key?.replace(/^codex:/,'') || ''});
        window.AieyesAgentSettings.accept(result.settings);
        issues.push(...result.issues.map(i => i.message));
      } catch (error) { issues.push((targets.find(t=>t.sourceId===sourceId)?.machineName || sourceId) + '：' + String(error.message ?? error)); }
      targets = await api('accounts.targets', key ? {accountKey:key} : {});
      changed?.(targets, [...issues]);
    }));
    return targets;
  }
  function paint() {
    if (!dialog?.open) return;
    const title = state.settings.accounts.find(a=>a.provider+':'+a.id===accountKey)?.name ?? '账号';
    dialog.innerHTML = `<div class="dialog-head"><h2>切换为此账号 · ${e(title)}</h2></div><div class="account-target-body">${embedded?window.AieyesCodexAuth.html():`${checking?'<p role="status">正在检查各机器的登录…</p>':''}${errors.map(v=>`<p class="error">${e(v)}</p>`).join('')}${rows.map((t,i)=>`<div class="list-row"><div class="row-body"><strong>${e(t.machineName)}</strong><small>${e(t.path)}</small><small>${e(t.reason || (t.current===true?'正在使用':t.credential===true?'已登录，尚未选用':t.credential===false?'未登录':'登录状态待确认'))}</small></div>${t.current===true?'':`<button data-account-target="${i}" ${checking&&!t.canSwitch?'disabled':''}>${t.canSwitch?'切换':t.credential===false?'登录':'检查 / 登录'}</button>`}</div>`).join('')}`}</div><div class="dialog-foot">${embedded?'<button id="target-back">返回机器列表</button>':'<button id="target-refresh">重新检查</button>'}<button id="target-close">完成</button></div>`;
    dialog.querySelector('#target-close').disabled = embedded && window.AieyesCodexAuth.busy;
    dialog.querySelector('#target-close').onclick = () => { generation++; dialog.close(); embedded=false; };
    if (embedded) {
      dialog.querySelector('#target-back').disabled = window.AieyesCodexAuth.busy;
      dialog.querySelector('#target-back').onclick = () => { embedded=false; reload(); };
      window.AieyesCodexAuth.bind();
    } else {
      dialog.querySelector('#target-refresh').disabled=checking;
      dialog.querySelector('#target-refresh').onclick=reload;
      dialog.querySelectorAll('[data-account-target]').forEach(button=>button.onclick=()=>{
        const t=rows[Number(button.dataset.accountTarget)];
        window.AieyesCodexAuth.open(accountKey.replace(/^codex:/,''),t.sourceId,t.canSwitch?'switch':'login');
        embedded=true;paint();
      });
    }
  }
  async function reload() {
    const revision=++generation;checking=true;errors=[];paint();
    try { await refresh(accountKey,(next,issues)=>{if(revision===generation){rows=next;errors=issues;paint();}}); }
    catch(error){if(revision===generation)errors=[String(error.message??error)];}
    finally {if(revision===generation){checking=false;paint();}}
  }
  function open(key) {
    if(!eligible(key))return;
    accountKey=key;embedded=false;rows=[];
    if(!dialog){dialog=document.createElement('dialog');dialog.id='account-targets';dialog.className='editor-dialog';document.body.append(dialog);dialog.oncancel=event=>{if(embedded&&window.AieyesCodexAuth.busy)event.preventDefault();else{generation++;embedded=false;}};}
    dialog.showModal();reload();
  }
  window.AieyesAccountTargets={eligible,open,refresh,paintAuth(){if(dialog?.open&&embedded){paint();return true;}return false;}};
})();
