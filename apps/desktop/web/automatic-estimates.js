// Both surfaces configure the same source opt-in; background sampling belongs to the native app.
window.AieyesAutomatic=(()=>{
  const consent='我确认本数据源只使用订阅账号及其 credits，不使用 API／中转；影响账号额度的所有设备用量均已纳入来源。';
  const labels={collecting:'积累样本中',sampling:'自动采样中',waiting:'等待有效样本',waitingIdentity:'等待订阅登录',waitingManual:'等待手动采样结束',disabled:'已关闭'};
  const status=s=>state.dashboard?.sources?.find(row=>row.id===s.id)?.autoValuationStatus;
  const description=s=>{const row=status(s);return s.autoValuation?.enabled?(labels[row?.state]??'等待首个实时快照')+(row?.note?' · '+row.note:''):'已关闭';};
  function html(sources){return sources.filter(s=>s.provider!=='deepseek').map(s=>`<button type="button" data-auto-source="${escapeHTML(s.id)}">自动估值 · ${escapeHTML(s.name)} · ${s.autoValuation?.enabled?'开启':'关闭'}</button><p class="tiny muted" data-auto-status="${escapeHTML(s.id)}">${escapeHTML(description(s))}</p>`).join('');}
  function refresh(){document.querySelectorAll('[data-auto-status]').forEach(el=>{const s=state.settings?.sources?.find(s=>s.id===el.dataset.autoStatus);if(s)el.textContent=description(s);});}
  function open(ids){
    const sources=state.settings.sources.filter(s=>ids.includes(s.id)&&s.provider!=='deepseek');
    const dialog=quotaDialog('数据源自动估值',`<p class="muted">自动识别当前订阅账号，分别积累 5h / 7d 与 credits 的可信样本。无法归属的区间自动跳过，保留最后有效值。</p><label class="quota-confirm"><input type="checkbox" id="auto-estimate-consent">${consent}</label>${sources.map(s=>`<div class="sampling-row"><strong>${escapeHTML(s.name)}</strong><small>${escapeHTML(s.path)}</small><p class="muted" data-auto-status="${escapeHTML(s.id)}">${escapeHTML(description(s))}</p><button type="button" data-auto-save="${escapeHTML(s.id)}" data-enabled="${s.autoValuation?.enabled?'false':'true'}" ${s.autoValuation?.enabled?'':'disabled'}>${s.autoValuation?.enabled?'关闭自动估值':'开启自动估值'}</button></div>`).join('')}`);
    dialog.dataset.autoSources=JSON.stringify(ids);
    dialog.querySelector('#auto-estimate-consent').onchange=event=>dialog.querySelectorAll('[data-auto-save][data-enabled="true"]').forEach(b=>b.disabled=!event.target.checked);
    dialog.querySelectorAll('[data-auto-save]').forEach(button=>button.onclick=async()=>{
      const enabled=button.dataset.enabled==='true',source=state.settings.sources.find(s=>s.id===button.dataset.autoSave);
      const buttons=[...dialog.querySelectorAll('button,input')];buttons.forEach(b=>b.disabled=true);dialog.dataset.busy='true';
      try{
        const saved=await api('autoEstimates.configure',{sourceId:source.id,enabled,confirmed:dialog.querySelector('#auto-estimate-consent').checked,baseConfig:source.autoValuation??null});
        window.AieyesAgentSettings.accept(saved.settings);markSaved();await loadDashboard();delete dialog.dataset.busy;open(ids);
      }catch(error){const el=dialog.querySelector('#quota-tool-error');el.textContent=error.message??String(error);el.hidden=false;buttons.forEach(b=>b.disabled=false);dialog.querySelector('#auto-estimate-consent').dispatchEvent(new Event('change'));}
      finally{delete dialog.dataset.busy;}
    });
  }
  document.addEventListener('click',event=>{const button=event.target.closest('[data-auto-source]');if(button)open([button.dataset.autoSource]);});
  return {html,open,refresh};
})();
