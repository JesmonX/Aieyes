// Both surfaces configure the same source opt-in; background sampling belongs to the native app.
window.AieyesAutomatic=(()=>{
  const consent='我确认本数据源只使用订阅账号及其 credits，不使用 API／中转；影响账号额度的所有设备用量均已纳入来源。';
  const mixedConsent='我确认影响订阅额度的所有设备用量均已纳入；API 用量通过计量服务识别，未归属或不完整的区间不参与订阅及 credits 估值。';
  const labels={collecting:'积累样本中',sampling:'自动采样中',waiting:'等待有效样本',waitingIdentity:'等待订阅登录',waitingManual:'等待手动采样结束',disabled:'已关闭'};
  const status=s=>state.dashboard?.sources?.find(row=>row.id===s.id)?.autoValuationStatus;
  const description=s=>{const row=status(s);return s.autoValuation?.enabled?(labels[row?.state]??'等待首个实时快照')+(row?.note?' · '+row.note:''):'已关闭';};
  function html(sources){return sources.filter(s=>s.provider!=='deepseek').map(s=>`<button type="button" data-auto-source="${escapeHTML(s.id)}">自动估值 · ${escapeHTML(s.name)} · ${s.autoValuation?.enabled?'开启':'关闭'}</button><p class="tiny muted" data-auto-status="${escapeHTML(s.id)}">${escapeHTML(description(s))}</p>`).join('');}
  function summary(sources){const enabled=sources.filter(s=>s.autoValuation?.enabled).length;return !sources.length?'请先添加数据目录':enabled?`已开启 ${enabled}/${sources.length}`:'已关闭';}
  function refresh(){
    document.querySelectorAll('[data-auto-status]').forEach(el=>{const s=state.settings?.sources?.find(s=>s.id===el.dataset.autoStatus);if(s)el.textContent=description(s);});
    document.querySelectorAll('[data-auto-machine-status]').forEach(el=>{el.textContent=summary((state.settings?.sources??[]).filter(s=>s.provider===el.dataset.autoProvider&&(s.hostId||'local')===el.dataset.autoMachineStatus));});
  }
  function open(ids,returnProvider=document.querySelector('#quota-dialog')?.dataset.valuationProvider){
    const sources=state.settings.sources.filter(s=>ids.includes(s.id)&&s.provider!=='deepseek');
    const dialog=quotaDialog('数据源自动估值',`<p class="muted">自动识别当前订阅账号，分别积累 5h / 7d 与 credits 的可信样本。无法归属的区间自动跳过，保留最后有效值。</p>${sources.map(s=>`<div class="sampling-row" data-auto-row="${escapeHTML(s.id)}"><strong>${escapeHTML(s.name)}</strong><small>${escapeHTML(s.path)}</small>${s.provider==='codex'?`<label>计费方式<select data-auto-mode="${escapeHTML(s.id)}"><option value="subscription" ${s.autoValuation?.mode!=='mixed'?'selected':''}>仅订阅与 credits</option><option value="mixed" ${s.autoValuation?.mode==='mixed'?'selected':''}>订阅与 API 混用</option></select></label>`:''}<p class="muted" data-auto-status="${escapeHTML(s.id)}">${escapeHTML(description(s))}</p><label class="quota-confirm"><input type="checkbox" data-auto-consent="${escapeHTML(s.id)}"><span data-consent-text>${s.autoValuation?.mode==='mixed'?mixedConsent:consent}</span></label><button type="button" data-auto-save="${escapeHTML(s.id)}" data-enabled="${s.autoValuation?.enabled?'false':'true'}" ${s.autoValuation?.enabled?'':'disabled'}>${s.autoValuation?.enabled?'关闭自动估值':'开启自动估值'}</button>${s.autoValuation?.enabled?`<button type="button" data-auto-apply="${escapeHTML(s.id)}" data-enabled="true" disabled>保存计费方式</button>`:''}</div>`).join('')}`);
    dialog.dataset.autoSources=JSON.stringify(ids);
    if(returnProvider){dialog.dataset.valuationProvider=returnProvider;const back=document.createElement('button');back.type='button';back.textContent='返回估值与采样';back.onclick=()=>openProviderEstimates(returnProvider);dialog.querySelector('.quota-dialog-body').prepend(back);}
    dialog.querySelectorAll('[data-auto-row]').forEach(row=>{
      const check=row.querySelector('[data-auto-consent]'),mode=row.querySelector('[data-auto-mode]');
      check.onchange=()=>row.querySelectorAll('button[data-enabled="true"]').forEach(b=>b.disabled=!check.checked);
      if(mode)mode.onchange=()=>{check.checked=false;row.querySelector('[data-consent-text]').textContent=mode.value==='mixed'?mixedConsent:consent;check.onchange();};
    });
    dialog.querySelectorAll('[data-auto-save],[data-auto-apply]').forEach(button=>button.onclick=async()=>{
      const enabled=button.dataset.enabled==='true',source=state.settings.sources.find(s=>s.id===(button.dataset.autoSave||button.dataset.autoApply)),row=button.closest('[data-auto-row]');
      const buttons=[...dialog.querySelectorAll('button,input,select')];buttons.forEach(b=>b.disabled=true);dialog.dataset.busy='true';
      try{
        const saved=await api('autoEstimates.configure',{sourceId:source.id,enabled,mode:row.querySelector('[data-auto-mode]')?.value||'subscription',confirmed:row.querySelector('[data-auto-consent]').checked,baseConfig:source.autoValuation??null});
        window.AieyesAgentSettings.accept(saved.settings);markSaved();await loadDashboard();delete dialog.dataset.busy;open(ids,returnProvider);
      }catch(error){const el=dialog.querySelector('#quota-tool-error');el.textContent=error.message??String(error);el.hidden=false;buttons.forEach(b=>b.disabled=false);dialog.querySelectorAll('[data-auto-consent]').forEach(c=>c.dispatchEvent(new Event('change')));}
      finally{delete dialog.dataset.busy;}
    });
  }
  document.addEventListener('click',event=>{const button=event.target.closest('[data-auto-source]');if(button)open([button.dataset.autoSource]);});
  return {html,open,refresh,summary};
})();

// Read the full snapshot here: overview filters must never hide management or history.
async function openProviderEstimates(provider) {
  if(state.estimateBusy){notify('采样操作正在保存，请稍后查看。');return;}
  const dialog=quotaDialog((providers[provider]??provider)+' · 估值与采样','<p role="status">正在读取估值与采样记录…</p>');
  const request=crypto.randomUUID();dialog.dataset.loadingRequest=request;
  let snapshot;
  try{snapshot=await api('dashboard.summary');}
  catch(error){if(dialog.open&&dialog.dataset.loadingRequest===request){$('#quota-tool-error').textContent='读取失败：'+String(error);$('#quota-tool-error').hidden=false;const retry=document.createElement('button');retry.textContent='重试读取';retry.onclick=()=>openProviderEstimates(provider);dialog.querySelector('.quota-dialog-body').append(retry);}return;}
  if(!dialog.open||dialog.dataset.loadingRequest!==request)return;
  const sources=state.settings.sources.filter(s=>s.provider===provider),accounts=state.settings.accounts.filter(a=>a.provider===provider&&!a.archived);
  const records=[...(snapshot.quotaEstimates??[]).map(e=>({...e,kind:'quota'})),...(snapshot.creditEstimates??[]).map(e=>({...e,kind:'credits'}))].filter(e=>e.accountKey.startsWith(provider+':')).sort((a,b)=>(b.checkpointAt??b.startedAt)-(a.checkpointAt??a.startedAt));
  const recordHTML=record=>`<div class="sampling-row"><strong>${escapeHTML(historicalAccounts().find(a=>a.provider+':'+a.id===record.accountKey)?.name??record.accountKey)}</strong><span>${record.kind==='credits'?'credits 估值':'5h / 7d 额度估值'} · ${escapeHTML(estimateStatus(record))}</span><small>${date(record.startedAt)} → ${date(record.checkpointAt)}</small><button type="button" data-valuation-record="${records.indexOf(record)}">查看与管理</button></div>`;
  const running=records.filter(e=>e.status!=='completed'&&e.samplingMode!=='automatic'),history=records.filter(e=>e.status==='completed');
  const automatic=sources.length?window.AieyesAutomatic.html(sources):'<p class="muted">添加数据目录后可配置自动估值。</p>';
  const manual=accounts.map(a=>`<div class="list-row"><strong class="row-body">${escapeHTML(a.name)}</strong><button type="button" data-valuation-account="${escapeHTML(a.provider+':'+a.id)}">额度采样</button>${provider==='codex'?`<button type="button" data-valuation-account="${escapeHTML(a.provider+':'+a.id)}" data-kind="credits">credits 采样</button>`:''}</div>`).join('')||'<p class="muted">添加账户后可开始手动采样。</p>';
  quotaDialog((providers[provider]??provider)+' · 估值与采样',`<h3>自动估值</h3><p class="muted">按数据来源分别配置，用于 5h / 7d 与 credits 的 API 等价估值。</p>${automatic}<h3>手动采样</h3>${manual}<h3>进行中的手动采样</h3>${running.map(recordHTML).join('')||'<p class="muted">当前没有进行中的手动采样。</p>'}<h3>已完成的采样历史</h3>${history.map(recordHTML).join('')||'<p class="muted">暂无已完成记录。</p>'}`);
  dialog.dataset.valuationProvider=provider;
  dialog.querySelectorAll('[data-valuation-account]').forEach(button=>button.onclick=()=>openQuotaEstimate(button.dataset.valuationAccount,button.dataset.kind??'weekly','',snapshot));
  dialog.querySelectorAll('[data-valuation-record]').forEach(button=>button.onclick=()=>{const record=records[Number(button.dataset.valuationRecord)];return openQuotaEstimate(record.accountKey,record.kind==='credits'?'credits':'weekly',record.groupId??'',snapshot);});
}
