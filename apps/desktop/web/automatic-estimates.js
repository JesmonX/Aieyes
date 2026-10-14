// Source-scoped consent; switches always reflect the last saved configuration.
window.AieyesAutomatic=(()=>{
  const consent='我确认影响订阅额度的所有设备用量均已纳入；按日志识别 API，不确定的区间跳过。';
  const labels={collecting:'积累样本中',sampling:'自动采样中',waiting:'等待有效样本',waitingIdentity:'等待订阅登录',waitingManual:'等待手动采样结束',disabled:'已关闭'};
  const status=s=>state.dashboard?.sources?.find(row=>row.id===s.id)?.autoValuationStatus;
  const description=s=>{const row=status(s);return !AieyesUI.sourceEnabled(state.settings,s)?'采集已暂停':!s.autoValuation?.enabled?'已关闭':(labels[row?.state]??'等待首个实时快照')+(row?.note?' · '+row.note:'');};
  const intro=provider=>provider==='codex'?'自动估值5h/7d限额和credits的价值':'自动估值5h/7d限额的价值';
  let saving=false;
  function html(sources){return sources.filter(s=>s.provider!=='deepseek').map(s=>`<section class="sampling-row" data-auto-row="${escapeHTML(s.id)}"><div class="between"><h4>${escapeHTML(s.name)}</h4><input type="checkbox" role="switch" data-auto-save="${escapeHTML(s.id)}" aria-label="自动估值 · ${escapeHTML(s.name)}" ${s.autoValuation?.enabled?'checked':''} ${saving?'disabled':''}></div><small>${escapeHTML(s.path)}</small><p class="tiny muted" data-auto-status="${escapeHTML(s.id)}">${escapeHTML(description(s))}</p><p class="error" data-auto-error hidden></p></section>`).join('');}
  function summary(sources){const enabled=sources.filter(s=>s.autoValuation?.enabled),active=enabled.filter(s=>AieyesUI.sourceEnabled(state.settings,s));return !sources.length?'未配置目录':!sources.some(s=>AieyesUI.sourceEnabled(state.settings,s))?'已暂停':!enabled.length?'已关闭':!active.length?'已暂停':'已开启 '+active.length+'/'+sources.length;}
  function refresh(){
    document.querySelectorAll('[data-auto-status]').forEach(el=>{const s=state.settings?.sources?.find(s=>s.id===el.dataset.autoStatus);if(s)el.textContent=description(s);});
    document.querySelectorAll('[data-auto-save]').forEach(el=>{const source=state.settings?.sources?.find(s=>s.id===el.dataset.autoSave);el.checked=source?.autoValuation?.enabled===true;el.disabled=saving;});
    document.querySelectorAll('[data-auto-machine-status]').forEach(el=>{el.textContent='估值 '+summary((state.settings?.sources??[]).filter(s=>s.provider===el.dataset.autoProvider&&(s.hostId||'local')===el.dataset.autoMachineStatus));});
  }
  function confirm(){return new Promise(resolve=>{
    const dialog=document.createElement('dialog');dialog.id='auto-consent';dialog.setAttribute('aria-label','开启自动估值');
    dialog.innerHTML=`<h2>开启自动估值？</h2><p>${consent}</p><div class="actions"><button data-cancel>取消</button><button data-confirm class="primary">确认开启</button></div>`;
    const finish=value=>{dialog.close();dialog.remove();resolve(value);};
    dialog.oncancel=event=>{event.preventDefault();finish(false);};dialog.querySelector('[data-cancel]').onclick=()=>finish(false);dialog.querySelector('[data-confirm]').onclick=()=>finish(true);
    document.body.append(dialog);dialog.showModal();
  });}
  async function toggle(input){
    const source=state.settings.sources.find(s=>s.id===input.dataset.autoSave);if(!source)return;
    const enabled=input.checked;input.checked=source.autoValuation?.enabled===true;
    if(saving)return;
    if(enabled&&!await confirm())return;
    saving=true;refresh();
    const row=input.closest('[data-auto-row]'),error=row?.querySelector('[data-auto-error]'),dialog=input.closest('dialog');if(error)error.hidden=true;
    if(dialog)dialog.dataset.busy='true';
    try{
      const result=await api('autoEstimates.configure',{sourceId:source.id,enabled,confirmed:enabled,baseConfig:source.autoValuation??null});
      window.AieyesAgentSettings.accept(result.settings);markSaved();await loadDashboard();
    }catch(cause){if(error){error.textContent=cause.message??String(cause);error.hidden=false;}else notify(String(cause),'error');}
    finally{saving=false;if(dialog)delete dialog.dataset.busy;refresh();}
  }
  function open(ids,returnProvider=document.querySelector('#quota-dialog')?.dataset.valuationProvider){
    const sources=state.settings.sources.filter(s=>ids.includes(s.id)&&s.provider!=='deepseek');
    const dialog=quotaDialog('数据源自动估值','<p class="muted">'+intro(sources.some(s=>s.provider==='codex')?'codex':'')+'</p>'+html(sources),'automatic');
    dialog.dataset.autoSources=JSON.stringify(ids);
    if(returnProvider){dialog.dataset.valuationProvider=returnProvider;const back=document.createElement('button');back.type='button';back.textContent='返回估值与采样';back.onclick=()=>openProviderEstimates(returnProvider);dialog.querySelector('.quota-dialog-body').prepend(back);}
  }
  document.addEventListener('change',event=>{if(event.target.matches('[data-auto-save]'))void toggle(event.target);});
  document.addEventListener('click',event=>{const button=event.target.closest('[data-auto-source]');if(button)open([button.dataset.autoSource]);});
  return {html,open,refresh,summary,intro};
})();

const manualSamplingExpanded=new Map();
// Management always uses the unfiltered snapshot. Collapsed manual rows are not mounted.
async function openProviderEstimates(provider) {
  if(state.estimateBusy){notify('采样操作正在保存，请稍后查看。');return;}
  const dialog=quotaDialog((providers[provider]??provider)+' · 估值与采样','<p role="status">正在读取估值与采样记录…</p>','valuation');
  const request=crypto.randomUUID();dialog.dataset.loadingRequest=request;
  let snapshot;
  try{snapshot=await api('dashboard.summary');}
  catch(error){if(dialog.open&&dialog.dataset.loadingRequest===request){$('#quota-tool-error').textContent='读取失败：'+String(error);$('#quota-tool-error').hidden=false;const retry=document.createElement('button');retry.textContent='重试读取';retry.onclick=()=>openProviderEstimates(provider);dialog.querySelector('.quota-dialog-body').append(retry);}return;}
  if(!dialog.open||dialog.dataset.loadingRequest!==request)return;
  const sources=state.settings.sources.filter(s=>s.provider===provider),accounts=state.settings.accounts.filter(a=>a.provider===provider&&!a.archived);
  const records=[...(snapshot.quotaEstimates??[]).map(e=>({...e,kind:'quota'})),...(snapshot.creditEstimates??[]).map(e=>({...e,kind:'credits'}))].filter(e=>e.accountKey.startsWith(provider+':')).sort((a,b)=>(b.checkpointAt??b.startedAt)-(a.checkpointAt??a.startedAt));
  const recordHTML=record=>`<div class="sampling-row"><strong>${escapeHTML(historicalAccounts().find(a=>a.provider+':'+a.id===record.accountKey)?.name??record.accountKey)}</strong><span>${record.kind==='credits'?'credits 估值':escapeHTML((AieyesUI.estimatePeriods(snapshot.quotas?.find(q=>q.provider+':'+q.accountId===record.accountKey),[record],record.groupId)+' 额度估值').trim())} · ${escapeHTML(estimateStatus(record))}</span><small>${date(record.startedAt)} → ${date(record.checkpointAt)}</small><button type="button" data-valuation-record="${records.indexOf(record)}">查看与管理</button></div>`;
  const running=records.filter(e=>e.status!=='completed'&&e.samplingMode!=='automatic'),history=records.filter(e=>e.status==='completed'&&e.samplingMode!=='automatic'),automaticHistory=records.filter(e=>e.samplingMode==='automatic');
  const automatic=sources.length?window.AieyesAutomatic.html(sources):'<p class="muted">添加数据目录后可配置自动估值。</p>';
  const manual=()=>accounts.map(a=>`<div class="list-row"><strong class="row-body">${escapeHTML(a.name)}</strong><button type="button" data-valuation-account="${escapeHTML(a.provider+':'+a.id)}">额度采样</button>${provider==='codex'?`<button type="button" data-valuation-account="${escapeHTML(a.provider+':'+a.id)}" data-kind="credits">credits 采样</button>`:''}</div>`).join('')||'<p class="muted">添加账户后可开始手动采样。</p>';
  quotaDialog((providers[provider]??provider)+' · 估值与采样',`<h3>自动估值</h3><p class="muted">${AieyesAutomatic.intro(provider)}</p>${automatic}<details id="manual-sampling" ${manualSamplingExpanded.get(provider)?'open':''}><summary>手动采样（进行中 ${running.length}）</summary><div data-manual-body></div></details><h3>自动采样历史</h3>${automaticHistory.map(recordHTML).join('')||'<p class="muted">暂无记录。</p>'}`,'valuation');
  dialog.dataset.valuationProvider=provider;
  const bindRecords=()=>{
    dialog.querySelectorAll('[data-valuation-account]').forEach(button=>button.onclick=()=>openQuotaEstimate(button.dataset.valuationAccount,button.dataset.kind??'weekly','',snapshot));
    dialog.querySelectorAll('[data-valuation-record]').forEach(button=>button.onclick=()=>{const record=records[Number(button.dataset.valuationRecord)];return openQuotaEstimate(record.accountKey,record.kind==='credits'?'credits':'weekly',record.groupId??'',snapshot);});
  };
  const details=dialog.querySelector('#manual-sampling');
  const mountManual=()=>{manualSamplingExpanded.set(provider,details.open);details.querySelector('[data-manual-body]').innerHTML=details.open?manual()+`<h3>进行中的手动采样</h3>${running.map(recordHTML).join('')||'<p class="muted">当前没有进行中的手动采样。</p>'}<h3>手动采样历史</h3>${history.map(recordHTML).join('')||'<p class="muted">暂无已完成记录。</p>'}`:'';bindRecords();};
  details.ontoggle=mountManual;mountManual();
}
