const historicalAccounts = () => [...(state.settings?.accounts??[]), ...(state.settings?.deletedAccounts??[])];
const $ = (s) => document.querySelector(s);
/* The floating window expands into a compact panel that mirrors the macOS menu bar
   popover; the same renderers feed both surfaces. */
const PANEL = Boolean(document.querySelector('#panel'));
const escapeHTML = (s) => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const compact = n => n >= 999995000 ? (n / 1e9).toFixed(2) + 'B' : n >= 999950 ? (n / 1e6).toFixed(2) + 'M' : n >= 999.5 ? (n / 1e3).toFixed(1) + 'K' : Math.round(n || 0).toString();
const money = n => '$' + (n || 0).toFixed(2);
function creditAmount(value) {
  if (value == null || String(value).trim() === '') return '—';
  const match = String(value).trim().match(/^([+-]?)(\d+)(?:\.(\d*))?(?:e([+-]?\d+))?$/i);
  if (!match) return '—';
  const exponent = Number(match[4] || 0), fraction = match[3] || '';
  if (!Number.isSafeInteger(exponent) || Math.abs(exponent) > 1000) return '—';
  let digits = BigInt(match[2] + fraction), shift = 2 + exponent - fraction.length;
  if (shift >= 0) digits *= 10n ** BigInt(shift);
  else { const divisor = 10n ** BigInt(-shift); digits = (digits + divisor / 2n) / divisor; }
  const text = digits.toString().padStart(3, '0');
  return (match[1] === '-' && digits !== 0n ? '-' : '') + text.slice(0, -2) + '.' + text.slice(-2);
}
function creditValue(record) {
  if (!record) return '';
  if (record.status === 'pending') return '待确认';
  return Number.isFinite(record.valuePer1000) ? '1000 credit ≈ '+money(record.valuePer1000)+' USD' : AieyesUI.estimateIssue(record);
}
const pct = n => n == null ? '—' : n.toFixed(1) + '%';
const bytes = n => { if (!Number.isFinite(n) || n < 0) return '—'; let i = 0; while (n >= 1024 && i < 4) { n /= 1024; i++; } return n.toFixed(i ? 1 : 0) + [' B',' KiB',' MiB',' GiB',' TiB'][i]; };
const speed = n => n == null ? '—' : bytes(n) + '/s';
const date = n => n ? new Date(n * 1000).toLocaleString('zh-CN', {month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hourCycle:'h23'}) : '—';
const providers = {codex:'Codex',claude:'Claude Code',antigravity:'Antigravity',agy:'Antigravity',deepseek:'DeepSeek',custom:'自定义'};
function providerMark(provider) {
  const key=provider==='agy'?'antigravity':provider,known=['codex','claude','antigravity','deepseek'].includes(key),name=escapeHTML(providers[provider]??provider);
  return `<span class="provider-mark" data-provider="${escapeHTML(provider)}" title="${name}" aria-label="${name}">${known?`<img src="provider-${key}.png" alt="" width="36" height="36">`:uiIcon('terminal')}</span>`;
}
function providerIdentity(provider) { return `<span class="provider-identity">${providerMark(provider)}<span>${escapeHTML(providers[provider]??provider)}</span></span>`; }
function subscriptionBadge(plan) { const label=AieyesUI.subscription(plan);return label?`<span class="subscription-badge" title="${escapeHTML(plan)}" aria-label="订阅 ${escapeHTML(label)}">${escapeHTML(label)}</span>`:''; }
const palette = ['#6575ed','#39a8a0','#a879d5','#e3a159','#d875a6','#4bbbd0','#7ca768','#817bca'];
const modelHues=new Map();
const color = name => {
  if(!modelHues.has(name)){let hash=2166136261;for(const c of new TextEncoder().encode(name))hash=Math.imul(hash^c,16777619)>>>0;let hue=(hash%3600)/10;
    for(let attempt=0;attempt<24&&[...modelHues.values()].some(h=>Math.min(Math.abs(h-hue),360-Math.abs(h-hue))<20);attempt++)hue=(hue+137.508)%360;
    modelHues.set(name,hue);
  }
  return `hsl(${modelHues.get(name)} 57% ${document.documentElement.dataset.theme==='dark'?74:30}%)`;
};

const groups = {cpu:'CPU',memory:'内存',gpu:'GPU',filesystems:'文件系统',disk:'磁盘 I/O',network:'网络'};
const detailOptions={uptime:'连续运行时间',cpuTimes:'CPU 时间分布',memoryCache:'内存缓存 / Buffer',swap:'Swap',fsAvailable:'文件系统可用空间',fsType:'文件系统类型 / 设备',inodes:'inode',diskIops:'磁盘 IOPS',diskBusy:'磁盘忙碌率',networkTotals:'累计流量',networkErrors:'网络错误 / 丢包',gpuMemory:'GPU 显存',gpuThermals:'GPU 温度 / 功耗'};
function monitoredHosts(settings=state.settings) { return [...(settings?.localMonitor?[{id:'local',name:'本机',target:'本机',...settings.localMonitor}]:[]),...(settings?.hosts??[])]; }
function refreshIndicator(key) { return `<button type="button" class="heading-refresh" data-refresh-status="${key}" aria-label="${refreshLabels[key]}状态" title="${refreshLabels[key]}状态"><span aria-hidden="true">↻</span></button>`; }
function updateHeadingIndicators() { for(const el of document.querySelectorAll('[data-refresh-status]')) { const r=state.refreshState[el.dataset.refreshStatus]??{};el.dataset.busy=String(Boolean(r.busy||(el.dataset.refreshStatus==='scan'&&state.dashboardPending)));el.dataset.error=String(Boolean(r.error||(el.dataset.refreshStatus==='scan'&&state.dashboardError)));el.title=(refreshLabels[el.dataset.refreshStatus]??'刷新')+' · '+(el.dataset.busy==='true'?'进行中':el.dataset.error==='true'?'部分失败，点击查看':r.success?timeLabel(r.success):'尚未刷新');el.setAttribute('aria-label',el.title); } }
function openRefreshStatus(key) {
  let pop=document.querySelector('#refresh-status-popover');if(!pop){pop=document.createElement('div');pop.id='refresh-status-popover';pop.setAttribute('popover','auto');pop.addEventListener('toggle',()=>{if(!pop.matches(':popover-open'))pop.replaceChildren();});document.body.append(pop);}
  pop.dataset.key=key;
  const keys=key==='scan'?['scan','prices']:[key],rows=keys.flatMap(k=>(state.refreshState[k]?.failures??[]).map(r=>({...r,key:k})));
  pop.innerHTML='<strong>'+escapeHTML(refreshLabels[key])+'</strong>'+(rows.length?rows.map((r,i)=>'<div class="error-row"><p>'+escapeHTML((r.name||r.id||r.accountId||'数据源')+' · '+r.error)+'</p><button data-status-retry="'+i+'">重试</button></div>').join(''):'<p>'+escapeHTML(state.refreshState[key]?.busy?'正在刷新…':state.refreshState[key]?.success?'上次成功：'+timeLabel(state.refreshState[key].success):'尚未刷新')+'</p>');
  pop.querySelectorAll('[data-status-retry]').forEach(b=>b.onclick=()=>{const r=rows[Number(b.dataset.statusRetry)];pop.hidePopover();({scan,quotas,prices:syncPrices,hosts:sample})[r.key](r.key==='quotas'?r.accountId:r.id);});pop.showPopover();if(key==='scan'){renderQueryStatus();const query=$('#query-status');pop.append(query);query.hidden=!state.dashboardPending&&!state.dashboardError;}
}
document.addEventListener('click',event=>{const button=event.target.closest('[data-refresh-status]');if(button)openRefreshStatus(button.dataset.refreshStatus);});
document.addEventListener('keydown',event=>{const pop=document.querySelector('#refresh-status-popover');if(event.key==='Escape'&&pop?.matches(':popover-open')){event.preventDefault();event.stopImmediatePropagation();pop.hidePopover();document.querySelector('[data-refresh-status="'+pop.dataset.key+'"]')?.focus({preventScroll:true});}},true);
const state = {page:'agent',settingsTab:'sources',priceSearch:'',settings:null,settingsDraft:null,settingsBaseline:null,settingsCommitting:false,dashboard:null,hosts:[],prices:[],provider:'',sourceId:'',accountKey:'',model:'',days:1,cost:false,busy:false,settingsSaving:false,serverBusy:false,lastScan:0,lastMetrics:0,lastQuota:0,lastSuccessfulUpdate:0,quotaBusy:false,estimateBusy:false,quotaError:'',refreshState:{},savedAt:0,savedSection:'',saveError:'',dashboardRequest:0,panelOpen:false,panelPage:'agent'};
function savePanelView() {
  if(!PANEL)return;
  try{localStorage.setItem('aieyes.panel.view',JSON.stringify(Object.fromEntries(['page','provider','sourceId','accountKey','model','days','cost','serverFilter'].map(k=>[k,state[k]]))));}catch(_){}
}
if(PANEL)try {
  const saved=JSON.parse(localStorage.getItem('aieyes.panel.view')||'{}');
  for(const key of ['provider','sourceId','accountKey','model'])if(typeof saved[key]==='string')state[key]=saved[key];
  if([1,7,30,90,365].includes(saved.days))state.days=saved.days;
  if(['all','errors','paused'].includes(saved.serverFilter))state.serverFilter=saved.serverFilter;state.cost=saved.cost===true;if(saved.page==='servers')state.page='servers';
  document.querySelector('.panel-body').addEventListener('scroll',event=>{try{localStorage.setItem('aieyes.panel.scroll',String(event.target.scrollTop));}catch(_){}},{passive:true});
}catch(_){}
function openDetail() {
  const view=Object.fromEntries(['page','provider','sourceId','accountKey','model','days','cost'].map(k=>[k,state[k]]));
  return window.__TAURI__.core.invoke('desktop_detail',{view}).catch(e=>notify(String(e),'error'));
}
async function api(method, params = {}) {
  try { return await window.__TAURI__.core.invoke('engine_call', {method,params}); }
  finally { if(method==='quotas.refresh'||/^(settings\.(save|patch)|agents\.set|sources\.(configure|remove)|accounts\.(create|connect|delete|restore|archive|save)|codexAuth\.(enable|adopt|profiles\.(bind|remove)|login\.status))/.test(method))invalidateQuotaSchedule(); }
}
let notificationTimer;
function notify(message, kind = 'info') {
  clearTimeout(notificationTimer);const el=$('#message');el.hidden=!message;el.dataset.kind=kind;
  el.innerHTML=message?'<span class="message-text">'+escapeHTML(message)+'</span><button type="button" class="message-close" aria-label="关闭提示">'+uiIcon('x')+'</button>':'';
  el.querySelector('.message-close')?.addEventListener('click',()=>notify(''));
  if(message&&kind==='info')notificationTimer=setTimeout(()=>notify(''),5000);
}
const refreshLabels={scan:'同步记录',quotas:'刷新限额',prices:'同步价格',hosts:'刷新服务器'};
function updateActivity() {
  scheduleRefreshes();
  const el=$('#activity'),fresh=AieyesUI.freshness(state.settings,state.dashboard),data=fresh.timestamp;
  const enabledHosts=monitoredHosts().filter(h=>h.enabled),updatedHosts=enabledHosts.filter(h=>hostStatus(h,state.hosts.find(r=>r.id===h.id))==='正常').length;
  const running=Object.entries(state.refreshState).filter(([,v])=>v.busy).map(([k])=>refreshLabels[k]);
  const failed=state.saveError||fresh.failed||Object.values(state.refreshState).some(v=>v.error)||state.hosts.some(h=>h.error);
  const engineBusy=['scan','quotas','prices'].some(key=>state.refreshState[key]?.busy);
  const status=state.busy||running.length?'busy':failed?'error':fresh.delayed?'warning':data?'success':'info';
  el.dataset.status=status;
  el.textContent=(state.page==='servers'?updatedHosts+'/'+enabledHosts.length+' 台已更新':data?'记录同步于 '+timeLabel(data*1000):fresh.total?'记录尚未全部同步':'未接入用量来源')+' · '+(running.length?running.join('、')+'中…':state.busy?'保存中…':failed?'部分失败':state.page==='servers'?(updatedHosts<enabledHosts.length?'存在延迟或未采样':'采样已更新'):fresh.delayed?fresh.delayed+' 个来源有延迟':fresh.total?'记录已同步':'等待接入');
  el.title=state.page==='servers'?'按每台已启用主机的状态统计；点击刷新服务器':'各来源最后成功同步时间（取最早）；查询生成于 '+date(state.dashboard?.generatedAt)+'；点击同步记录';el.tabIndex=0;el.setAttribute('role','button');el.setAttribute('aria-label',el.textContent+'；'+el.title);window.AieyesNetwork?.draw();
  updateHeadingIndicators();
  for(const button of document.querySelectorAll('[data-refresh]')) {
    const key=button.dataset.refresh,r=state.refreshState[key]??{};
    button.disabled=Boolean(r.busy||((key==='scan'||key==='quotas'||key==='prices')&&(state.busy||state.settingsSaving||engineBusy)));
    button.innerHTML='<span>'+refreshLabels[key]+'</span><small>'+(r.busy?'进行中…':r.error?'失败 · 可重试':r.success?timeLabel(r.success):'尚未刷新')+'</small>';
  }
}
function showFailures(error,key,itemId=null) {
  const failures=error?.failures??(error?[{id:itemId,name:refreshLabels[key]??'操作',error:error.message??String(error)}]:[]);
  const r=state.refreshState[key]??={};
  r.failures=[...(itemId?(r.failures??[]).filter(row=>(row.accountId??row.id)!==itemId):[]),...failures];
  r.error=r.failures.length?'读取失败':null;
  renderFailures();
}
function renderFailures() { updateHeadingIndicators();const pop=document.querySelector('#refresh-status-popover');if(pop?.matches(':popover-open'))openRefreshStatus(pop.dataset.key); }
function beginRefresh(key){state.refreshState[key]={...state.refreshState[key],busy:true};updateActivity();}
function endRefresh(key,error,itemId=null){const r=state.refreshState[key];r.busy=false;if(!error)r.success=Date.now();showFailures(error,key,itemId);updateActivity();}
const timeLabel = stamp => new Date(stamp).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit',hourCycle:'h23'});
let pendingRender=false;
function interactionOpen() {
  return Boolean(($('#token-popover')&&!$('#token-popover').hidden)||$('#editor')?.open||$('#quota-dialog')?.open||$('#panel-more')?.open||($('#panel-menu')&&!$('#panel-menu').hidden)||($('#refresh-menu')&&!$('#refresh-menu').hidden));
}
function flushRender() {
  if(pendingRender&&!interactionOpen()){pendingRender=false;render();}
}
document.addEventListener('click',()=>setTimeout(flushRender,0));
document.addEventListener('keydown',()=>setTimeout(flushRender,0));
window.addEventListener('storage',event=>{
  if(!event.key?.startsWith('quota.expanded.v2.'+(PANEL?'panel.':'detail.')))return;
  const key=event.key.slice(('quota.expanded.v2.'+(PANEL?'panel.':'detail.')).length);
  const detail=document.querySelector('[data-quota="'+CSS.escape(key)+'"] .quota-disclosure');
  if(detail&&detail.dataset.collapsible==='true')detail.open=event.newValue!=='false';
});
// Keep the user's place when live values replace their markup.
function focusSelector(element) {
  if(!element||element===document.body)return null;
  if(element.id)return '#'+CSS.escape(element.id);
  const attributes=['data-refresh-status','data-account-settings','data-heat-day','data-trend-model','data-day-toggle','data-credit-estimate','data-manage-sampling','data-estimate','data-page','data-settings-tab','data-edit','data-remove','data-price','data-account-edit','data-account-options','data-device-command','data-account-login','data-device-manage','data-account-delete','data-account-connect','data-account-restore','data-source-advanced','data-agent-advanced','data-enable','data-gap-map','data-gap-price','data-host-status'];
  for(const attribute of attributes)if(element.hasAttribute(attribute))return `[${attribute}="${CSS.escape(element.getAttribute(attribute))}"]`;
  if(element.tagName==='SUMMARY')for(const attribute of ['data-agent-detail','data-metric'])if(element.parentElement.hasAttribute(attribute))return `[${attribute}="${CSS.escape(element.parentElement.getAttribute(attribute))}"] > summary`;
  return null;
}
function rememberView() {
  return {focus:focusSelector(document.activeElement),window:[window.scrollX,window.scrollY],panel:$('.panel-body')?.scrollTop,
    closed:[...document.querySelectorAll('#content [data-agent-detail]:not([open])')].map(el=>el.dataset.agentDetail),
    opened:[...document.querySelectorAll('#content [data-agent-detail][open],#content [data-metric][open]')].map(el=>[el.hasAttribute('data-metric')?'data-metric':'data-agent-detail',el.dataset.metric??el.dataset.agentDetail]),
    scrollers:[...document.querySelectorAll('#content .daily-table,#content .prices-list')].map(el=>({className:el.className,top:el.scrollTop,left:el.scrollLeft}))};
}
function restoreView(view) {
  for(const key of view.closed??[]){const el=document.querySelector('[data-agent-detail="'+CSS.escape(key)+'"]');if(el&&el.dataset.collapsible!=='false')el.open=false;}
  for(const [attribute,value] of view.opened){const el=document.querySelector(`[${attribute}="${CSS.escape(value)}"]`);if(el){el.open=true;if(el.tagName==='TBODY'){el.setAttribute('open','');el.querySelector('[data-day-toggle]')?.setAttribute('aria-expanded','true');el.querySelectorAll('.day-model').forEach(row=>row.hidden=false);}}}
  if(view.focus)document.querySelector(view.focus)?.focus({preventScroll:true});
  for(const item of view.scrollers){const el=[...document.querySelectorAll('#content .daily-table,#content .prices-list')].find(el=>el.className===item.className);if(el){el.scrollTop=item.top;el.scrollLeft=item.left;}}
  if(view.panel!=null&&$('.panel-body'))$('.panel-body').scrollTop=view.panel;
  window.scrollTo(...view.window);
}
async function openSetup(kind='sources') {
  if(PANEL){try{await window.__TAURI__.core.invoke('desktop_action',{action:kind==='hosts'?'add-host':kind==='quota'?'add-quota':'add-source'});}catch(error){notify(String(error),'error');}return;}
  state.page='settings';state.settingsTab=kind==='hosts'?'hosts':kind==='quota'?'accounts':'sources';render();if(kind==='hosts')editItem();
}
function onboarding() {
  return `<div class="card onboarding"><h2>启用你的第一个 Agent</h2><p class="muted">启用 Agent 并选择机器，在账户中添加登录或 API Key。</p><div class="actions"><button id="add-first-source" class="primary" title="选择 Agent 与机器">设置 Agents</button><button id="add-first-quota">账户限额</button><button id="add-first-host">SSH 主机</button></div></div>`;
}
function bindSetup() { if($('#add-first-source'))$('#add-first-source').onclick=()=>openSetup();if($('#add-first-host'))$('#add-first-host').onclick=()=>openSetup('hosts');if($('#add-first-quota'))$('#add-first-quota').onclick=()=>openSetup('quota'); }
function samplingBanner(d) {
  const records=[...(d.quotaEstimates??[]),...(d.creditEstimates??[])].filter(e=>e.status!=='completed');
  if(!records.length)return '';if(records.some(e=>e.status==='pending'))return attention('待确认：采样需要确认用量来源','管理采样','sampling-attention');
  return '<button class="sampling-banner" data-manage-sampling><span>后台采样 · '+records.filter(e=>e.status==='active').length+' 项进行中'+(records.some(e=>e.status==='pending')?' · 有待确认项':'')+'</span><span>管理采样 '+uiIcon('right')+'</span></button>';
}
function openSampling() {
  if(state.estimateBusy){notify("后台采样操作仍在进行，可从任务入口返回");return;}
  const records=[...(state.dashboard.quotaEstimates??[]),...(state.dashboard.creditEstimates??[])].filter(e=>e.status!=='completed');
  const rows=records.map((e,i)=>'<div class="sampling-row"><strong>'+escapeHTML(historicalAccounts().find(a=>a.provider+':'+a.id===e.accountKey)?.name??e.accountKey)+'</strong><span>'+(e.kind==='credits'?'credit 价值':e.valuationMode==='fiveHour'?'5h / 7d 额度价值':'7d 整周价值')+' · '+estimateStatus(e)+'</span>'+(e.reason?'<p class="error">'+escapeHTML(e.reason)+'</p>':'')+'<div class="actions"><button data-sampling-open="'+i+'">查看与管理</button><button data-sampling-stop="'+i+'">结束采样</button></div></div>').join('');
  quotaDialog('管理采样','<p class="muted">关闭窗口后采样继续。只有结束采样才会停止记录。</p>'+(rows||'<p>当前没有进行中的采样</p>'));
  document.querySelectorAll('[data-sampling-open]').forEach(b=>b.onclick=()=>{const e=records[Number(b.dataset.samplingOpen)];openQuotaEstimate(e.accountKey,e.kind==='credits'?'credits':'weekly',e.groupId??'');});
  document.querySelectorAll('[data-sampling-stop]').forEach(b=>b.onclick=async()=>{const e=records[Number(b.dataset.samplingStop)];if(await quotaAction((e.kind==='credits'?'creditEstimates.':'quotaEstimates.')+'stop',{id:e.id}))openSampling();});
}
function quotaSection(d) {
  const selected=PANEL?AieyesUI.panelAccounts(state.settings,d.quotaOrder??[]):null,visible=a=>!PANEL||selected[a.provider]?.includes(a.provider+':'+(a.accountId??a.id));
  const quotas=(d.quotas??[]).filter(visible),missing=state.settings.accounts.filter(a=>visible(a)&&!a.archived&&a.quotaEnabled&&(!state.provider||a.provider===state.provider)&&(!state.accountKey||state.accountKey===a.provider+':'+a.id)&&!quotas.some(q=>q.provider===a.provider&&q.accountId===a.id));
  if(!quotas.length&&!missing.length)return PANEL&&state.settings.accounts.some(a=>!a.archived&&a.quotaEnabled)?'<p class="muted panel-account-empty">当前面板未显示账户，可在更多中选择；详情保留全部账户。</p>':'';
  return `<div class="section-head quota-heading"><h2 title="实时账户限额 · 不随历史日期或模型筛选变化">实时账户限额 ${refreshIndicator('quotas')} <span class="count">${quotas.length+missing.length}</span></h2><div class="section-actions"><button id="order-quotas" title="调整账户顺序" aria-label="调整账户顺序">${uiIcon('sort')}</button><button id="read-quotas" ${state.quotaBusy||state.refreshState.quotas?.busy?'disabled':''} title="重新查询各账户的实时限额">${state.quotaBusy||state.refreshState.quotas?.busy?'读取中…':'刷新限额'}</button></div></div><div class="quotas">${quotas.map(q=>quotaCard(q)).join('')}${missing.map(a=>{
    const enabled=state.settings.sources.some(src=>src.enabled&&src.provider===a.provider&&(src.accountId===a.id||src.codexHomeId&&window.AieyesAgentSettings.profiles(a).some(r=>r.startsWith(src.codexHomeId+':')))&&(!src.hostId||state.settings.hosts.some(h=>h.id===src.hostId&&h.enabled)));
    const text=!enabled?'关联并启用数据源后可读取限额':state.quotaBusy?'正在读取账户限额…':state.quotaError?'读取失败，可重试刷新限额':'尚未读取限额，可点击刷新限额';
    return `<div class="card quota-placeholder"><h3>${providerMark(a.provider)} ${escapeHTML(a.name)}</h3><p class="muted" role="status">${text}</p>${enabled&&state.quotaError?`<p class="error">${escapeHTML(state.quotaError)}</p>`:''}</div>`;
  }).join('')}</div>`;
}
async function job(label, fn, refreshKey=null, refreshItem=null) {
  if (state.busy || state.settingsSaving || ['scan','quotas','prices'].some(key=>state.refreshState[key]?.busy)) return false;
  const jobFocus=focusSelector(document.activeElement);
  state.busy=true;if(refreshKey)beginRefresh(refreshKey);else updateActivity();
  const buttons=[...document.querySelectorAll('header [data-refresh]')].map(button=>[button,button.disabled]);
  buttons.forEach(([button])=>button.disabled=true);
  let outcome='error',failure=null;const previousMessage=$('#message').textContent;
  try { const result=await fn();if(result===false){outcome='noop';return true;}if(!refreshKey)state.saveError='';outcome='success';state.lastSuccessfulUpdate=Date.now();if($('#message').dataset.kind==='error'&&$('#message').textContent===previousMessage)notify('');return true; }
  catch (error) { failure=error;outcome=error.partial?'partial':'error';if(!refreshKey){state.saveError=error.message??String(error);notify(state.saveError,'error');document.querySelectorAll('.saved-feedback').forEach(el=>{el.textContent='保存失败：'+state.saveError;el.dataset.status='error';});}return false; }
  finally {
    state.busy=false;$('#activity').dataset.status=outcome;
    if(refreshKey&&outcome==='noop'){state.refreshState[refreshKey].busy=false;updateActivity();}else if(refreshKey)endRefresh(refreshKey,failure,refreshItem);else updateActivity();
    buttons.forEach(([button,disabled])=>{if(button.isConnected)button.disabled=disabled;});updateActivity();updateSettingsSaveBar();if(jobFocus&&document.activeElement===document.body){const target=document.querySelector(jobFocus);if(target&&!target.disabled)target.focus({preventScroll:true});else if(jobFocus==='#settings-save-all')$('#settings-save-bar .settings-draft-label')?.focus({preventScroll:true});}
  }
}
const filterKeys=['provider','accountKey','sourceId','model','days'];
const filterSnapshot=()=>Object.fromEntries(filterKeys.map(k=>[k,state[k]]));
function appliedScope(){const f=state.appliedFilters??filterSnapshot();return [f.days===1?'今日':'最近 '+f.days+' 天',providers[f.provider]??'全部 Agent',historicalAccounts().find(a=>a.provider+':'+a.id===f.accountKey)?.name??(f.accountKey==='none'?'未关联账户':'全部账户'),state.settings.sources.find(s=>s.id===f.sourceId)?.name??'全部来源',f.model||'全部模型'].join(' · ');}
async function changeFilters(values={}){Object.assign(state,values);try{await loadDashboard({debounce:true});}catch(_){} }
function renderQueryStatus(){
  let el=$('#query-status');if(!el){el=document.createElement('div');el.id='query-status';el.setAttribute('role','status');document.body.append(el);}
  el.hidden=(!state.dashboardPending&&!state.dashboardError)||!el.closest('#refresh-status-popover')?.matches(':popover-open');updateHeadingIndicators();
  el.innerHTML=(!state.dashboardPending&&!state.dashboardError)?'':'<p>'+escapeHTML(state.dashboardPending?'正在更新，仍显示上一结果：'+appliedScope():'更新失败，仍显示上一结果：'+appliedScope()+'。'+state.dashboardError)+'</p>'+(state.dashboardError?'<button id="query-retry">重试</button><button id="query-restore">恢复已应用筛选</button>':'');
  if($('#query-retry'))$('#query-retry').onclick=()=>changeFilters();
  if($('#query-restore'))$('#query-restore').onclick=()=>changeFilters(state.appliedFilters);
}
let dashboardWork=false,dashboardQueued=false,dashboardReadyAt=0,dashboardTimer, dashboardWaiters=[];
function loadDashboard({debounce=false}={}) {
  ++state.dashboardRequest;dashboardQueued=true;dashboardReadyAt=Date.now()+(debounce?120:0);
  state.dashboardPending=true;state.dashboardError='';if(state.page==='agent')renderQueryStatus();
  const result=new Promise((resolve,reject)=>dashboardWaiters.push({resolve,reject}));
  kickDashboard();return result;
}
function kickDashboard() {
  clearTimeout(dashboardTimer);if(dashboardWork)return;
  const delay=dashboardReadyAt-Date.now();
  if(delay>0){dashboardTimer=setTimeout(kickDashboard,delay);return;}
  if(!dashboardQueued)return;
  dashboardQueued=false;dashboardWork=true;const request=state.dashboardRequest;
  loadDashboardOnce(request).then(()=>finishDashboard(null,request),error=>finishDashboard(error,request));
}
function finishDashboard(error,request) {
  dashboardWork=false;
  if(dashboardQueued){kickDashboard();return;}
  const waiters=dashboardWaiters;dashboardWaiters=[];
  for(const waiter of waiters)if(error&&request===state.dashboardRequest)waiter.reject(error);else waiter.resolve();
}
async function loadDashboardOnce(request) {
  savePanelView();
  if(state.accountKey && state.accountKey!=='none' && !historicalAccounts().some(a=>a.provider+':'+a.id===state.accountKey))state.accountKey='';
  if(state.sourceId && !state.settings.sources.some(s=>s.id===state.sourceId))state.sourceId='';
  const account=historicalAccounts().find(a=>a.provider+':'+a.id===state.accountKey);
  const requested=filterSnapshot();state.dashboardPending=true;state.dashboardError='';if(state.page==='agent')renderQueryStatus();
  try {
  const dashboard = await api('dashboard',{provider:account?.provider || state.provider || null,accountId:state.accountKey==='none'?'':account?.id ?? null,sourceId:state.sourceId || null,model:state.model || null,days:state.days});
  if(request!==state.dashboardRequest)return;
  for(const name of [...new Set([...(dashboard.modelOptions??[]),...(dashboard.models??[]).map(m=>m.key),...(dashboard.dayModels??[]).map(r=>r.model)])].sort())color(name);
  state.dashboard=dashboard;state.appliedFilters=requested;state.dashboardPending=false;state.dashboardError='';updateActivity();
  if (state.page === 'agent') {
    if(interactionOpen())pendingRender=true;else render();
  }
  refreshQuotaDialog();
  }catch(error){if(request===state.dashboardRequest){state.dashboardPending=false;state.dashboardError=error.message??String(error);if(state.page==='agent')renderQueryStatus();}throw error;}
}
function batchError(rows,label) {
  if(!Array.isArray(rows))return null;
  const failures=rows.filter(row=>row.error||row.partial).map(row=>({...row,error:row.error||(row.issues??[]).map(i=>(i.path?i.path+' · ':'')+(i.step!=null?'步骤 '+i.step+' · ':'')+i.message).join('；')||'部分记录需要处理'}));if(!failures.length)return null;
  const partial=failures.length<rows.length;
  const error=new Error(`${label}${partial?'部分失败':'失败'}：${failures.map(row=>`${row.name||row.id||row.accountId||''}${row.name||row.id||row.accountId?' · ':''}${row.error}`).join('；')}`);
  error.partial=partial;error.failures=failures;return error;
}
async function scan(sourceId) {
  if(state.estimateBusy||state.sharedEstimateOperations?.size)return;
  await job('同步记录',async()=>{try{const rows=await api('sources.scan',typeof sourceId==='string'?{sourceId}:{});await loadDashboard();const error=batchError(rows,'同步记录');if(error)throw error;}finally{state.lastScan=Date.now();}},'scan',typeof sourceId==='string'?sourceId:null);
}
function hasQuotaSources() {
  return state.settings.accounts.some(a => !a.archived && a.quotaEnabled && state.settings.sources.some(s => s.enabled && s.provider === a.provider && (s.accountId === a.id || s.codexHomeId && window.AieyesAgentSettings.profiles(a).some(r=>r.startsWith(s.codexHomeId+':'))) && (!s.hostId || state.settings.hosts.some(h => h.id === s.hostId && h.enabled))));
}
async function quotas(accountId,dueOnly=false) {
  if(state.estimateBusy||state.sharedEstimateOperations?.size||state.busy||state.settingsSaving||['scan','quotas','prices'].some(key=>state.refreshState[key]?.busy))return;
  state.quotaBusy=true;if(!dueOnly)state.quotaError='';if(!dueOnly&&state.page==='agent')render();
  try { await job('读取限额',async()=>{try{const rows=await api('quotas.refresh',{...(typeof accountId==='string'?{accountId}:{}),dueOnly});if(dueOnly&&!rows.length)return false;if(rows.some(q=>q.identity&&state.settings.accounts.some(a=>a.id===q.accountId&&a.provider===q.provider&&a.identityKey!==q.identity.key)))await window.AieyesAgentSettings?.syncIdentitySettings();await loadDashboard();const error=batchError(rows,'读取限额');if(error)throw error;}catch(error){state.quotaError=error.failures?'部分账户读取失败，请逐项重试。':error.message??String(error);throw error;}finally{state.lastQuota=Date.now();}},'quotas',typeof accountId==='string'?accountId:null); }
  finally {state.quotaBusy=false;scheduleRefreshes();if(!dueOnly&&state.page==='agent')render();}
}
async function syncPrices(){await job('同步价格',async()=>{await api('prices.sync');state.prices=await api('prices.list');await loadDashboard();if(state.page==='settings')renderSettings();notify('价格已更新');},'prices');}
let lastHostError='';
const hostSampleVersions=new Map(),hostSampleValues=new Map();
function applyHostSamples(payload) {
  const rows=Array.isArray(payload)?payload:payload?.rows??[],attemptedAt=Date.now();
  const byID=new Map(state.hosts.map(row=>[row.id,row]));let changed=false;
  for(const row of rows){
    const old=hostSampleVersions.get(row.id),version=row.sampleVersion,session=row.sampleSession;
    if(session&&old?.session===session&&version<=old.version)continue;
    if(session)hostSampleVersions.set(row.id,{session,version});
    else {const signature=JSON.stringify(row);if(hostSampleValues.get(row.id)===signature)continue;hostSampleValues.set(row.id,signature);}
    const previous=byID.get(row.id);
    byID.set(row.id,{...row,lastAttemptAt:row.lastAttemptAt??attemptedAt,...(row.error?{sample:row.sample??previous?.sample}:{})});changed=true;
  }
  if(!changed)return;
  const allowed=new Set(monitoredHosts().map(h=>h.id));
  state.hosts=[...byID.values()].filter(row=>allowed.has(row.id));
  for(const key of hostSampleVersions.keys())if(!allowed.has(key))hostSampleVersions.delete(key);
  for(const key of hostSampleValues.keys())if(!allowed.has(key))hostSampleValues.delete(key);
  if(!rows.some(row=>row.error)&&lastHostError){if($('#message').textContent===lastHostError)notify('');lastHostError='';}
  state.lastMetrics=attemptedAt;updateActivity();updateHostStatuses();if(state.settings&&state.page==='servers')window.AieyesServers.request();
}
function applyHostFailure(error,hostId=null) {
  const message=error.message??String(error),attemptedAt=Date.now();
  const rows=monitoredHosts().map(host=>{
    const previous=state.hosts.find(row=>row.id===host.id)??{id:host.id};
    if(!host.enabled||(hostId&&host.id!==hostId))return previous;
    const {sampleSession,sampleVersion,...retained}=previous;
    return {...retained,error:message,lastAttemptAt:attemptedAt};
  });
  applyHostSamples(rows);lastHostError=message;
}
async function sample(hostId) {
  if(state.serverBusy||state.refreshState.hosts?.busy)return;state.serverBusy=true;beginRefresh('hosts');let failure=null;
  try{const payload=await api('hosts.sample',{stream:true,...(typeof hostId==='string'?{hostId}:{})}),rows=Array.isArray(payload)?payload:payload.rows;applyHostSamples(rows);failure=batchError(rows,'刷新服务器');}
  catch(error){failure=error;applyHostFailure(error,typeof hostId==='string'?hostId:null);}
  finally{state.lastMetrics=Date.now();state.serverBusy=false;endRefresh('hosts',failure,typeof hostId==='string'?hostId:null);}
}
async function saveSettings(next,{refresh=true}={}) {
  if (state.settingsSaving) throw new Error('设置正在保存，请稍后重试');
  state.settingsSaving = true;
  const previous=state.settings;
  try {
    const saved=await api('settings.patch', {base:previous,settings:next});
    window.AieyesAgentSettings.accept(saved);
    if(JSON.stringify([previous.accounts,previous.sources,previous.hosts,previous.proxy])!==JSON.stringify([next.accounts,next.sources,next.hosts,next.proxy]))state.lastQuota=0;
  } finally { state.settingsSaving = false; scheduleRefreshes(); }
  markSaved();
  if(refresh&&JSON.stringify([previous.accounts,previous.sources,previous.hosts,previous.modelMappings])!==JSON.stringify([next.accounts,next.sources,next.hosts,next.modelMappings]))void loadDashboard().catch(error=>notify('设置已保存，概览刷新失败：'+String(error)));
}
const pendingAPIKeys=new Map(),pendingHostPasswords=new Map(),preparedAPIKeys=new Map();
const configKey=value=>JSON.stringify(value,(_,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.keys(v).sort().map(k=>[k,v[k]])):v);
function draftSettings(){
  if(!state.settingsDraft){state.settingsDraft=structuredClone(state.settings);state.settingsBaseline=configKey(state.settings);}
  return state.settingsDraft;
}
function stageSettings(next){state.settingsDraft=structuredClone(next);for(const id of pendingAPIKeys.keys())if(!next.sources.some(s=>s.id===id&&s.provider==='deepseek'))pendingAPIKeys.delete(id);for(const id of pendingHostPasswords.keys())if(!next.hosts.some(h=>h.id===id&&h.authMode==='password'))pendingHostPasswords.delete(id);updateSettingsSaveBar();}
function settingsChangeCount(){
  captureSettingsForms();if(!state.settings)return 0;
  const draft=state.settingsDraft??state.settings,changed=new Set();
  for(const [list,key] of [['sources','id'],['hosts','id'],['accounts','id']]) {
    const identity=row=>list==='accounts'?row.provider+':'+row.id:row[key];
    const old=new Map(state.settings[list].map(row=>[identity(row),row])),next=new Map(draft[list].map(row=>[identity(row),row]));
    for(const id of new Set([...old.keys(),...next.keys()]))if(JSON.stringify(old.get(id))!==JSON.stringify(next.get(id)))changed.add(list+':'+id);
  }
  for(const id of pendingAPIKeys.keys())changed.add('sources:'+id);for(const id of pendingHostPasswords.keys())changed.add('hosts:'+id);
  const rest=value=>Object.fromEntries(Object.entries(value).filter(([key])=>!['sources','hosts','accounts','modelMappings'].includes(key)));
  const general=settingsForms.get('general-form');if(JSON.stringify(rest(draft))!==JSON.stringify(rest(state.settings))||(general&&JSON.stringify(general.values)!==JSON.stringify(general.baseline)))changed.add('general');
  const mapping=settingsForms.get('mapping-form')?.values;
  if(JSON.stringify(draft.modelMappings)!==JSON.stringify(state.settings.modelMappings)||mapping?.mappingModel?.trim()||mapping?.mappingId?.trim())changed.add('mappings');
  return changed.size;
}
function updateSettingsSaveBar(){
  const count=settingsChangeCount(),bar=$('#settings-save-bar');if(!bar)return;
  bar.dataset.dirty=String(count>0);bar.querySelector('.settings-draft-label').textContent=state.settingsCommitting?'正在保存应用配置…':count?'当前表单有未保存输入':'账户修改自动保存；目录与服务器在编辑器中保存';

}
function collectGeneralDraft(next){
  captureSettingsForms();const values=settingsForms.get('general-form')?.values;if(!values)return next;
  const data=new FormData();for(const [key,value] of Object.entries(values))data.set(key,value);
  next.appearance={theme:data.get("appearanceTheme")||"system",accent:data.get("appearanceAccent")||"indigo"};
  next.proxy=proxyValue(data,'app');next.proxyTestUrls=(data.get('proxyTestUrls')??'').split(',').map(v=>v.trim()).filter(Boolean);
  for(const [key,min,label] of [['refreshSeconds',10,'Agent 限额'],['historyRefreshSeconds',10,'Agent 记录'],['serverForegroundRefreshSeconds',2,'服务器前台'],['serverRefreshSeconds',2,'服务器后台']]){const value=Number(data.get(key));if(!Number.isInteger(value)||value<min||value>86400){const input=$('#field-'+key);input?.setAttribute('aria-invalid','true');input?.focus();const error=new Error(label+' 刷新间隔范围为 '+min+'–86400 秒，草稿已保留');error.field='field-'+key;throw error;}$('#field-'+key)?.removeAttribute('aria-invalid');next[key]=value;}
  return next;
}
async function saveAllSettings({clearForms=true,refresh=true,section=state.settingsTab}={}){
  if(state.settingsCommitting)throw new Error('配置正在保存');
  const next=section==='prices'?mappingDraft(structuredClone(state.settings)):section==='general'?collectGeneralDraft(structuredClone(state.settings)):structuredClone(draftSettings()),preparedHosts=[];
  if(section==='general'&&configKey(next)===configKey(state.settings)&&!pendingAPIKeys.size&&!pendingHostPasswords.size){resetSettingsForm('general-form');updateSettingsSaveBar();return true;}
  const controls=[...document.querySelectorAll('#general-form input,#general-form select,#general-form textarea')].map(c=>[c,c.disabled]);
  controls.forEach(([c])=>c.disabled=true);
  state.settingsCommitting=true;updateSettingsSaveBar();
  try{
    for(const [id,key] of pendingAPIKeys){const source=next.sources.find(s=>s.id===id);if(!source||source.provider!=='deepseek')continue;
      let saved=preparedAPIKeys.get(id);if(!saved||saved.key!==key){const result=await api('credentials.save',{sourceId:id+'-draft-'+crypto.randomUUID(),apiKey:key});if(!result.path)throw new Error('未获得凭据保存路径');saved={key,path:result.path};preparedAPIKeys.set(id,saved);}source.path=saved.path;
    }
    for(const [id,password] of pendingHostPasswords){const host=next.hosts.find(h=>h.id===id);if(!host||host.authMode!=='password')continue;const saved=await api('hosts.credentials.save',{password});if(!saved.passwordRef)throw new Error('未获得密码引用');host.passwordRef=saved.passwordRef;preparedHosts.push(saved.passwordRef);}
    await saveSettings(next,{refresh:false});
    pendingAPIKeys.clear();pendingHostPasswords.clear();preparedAPIKeys.clear();
    const committed=await api('settings.get').catch(()=>next);state.settings=committed;state.settingsDraft=structuredClone(committed);state.settingsBaseline=configKey(committed);
    if(clearForms)settingsForms.delete(section==='prices'?'mapping-form':'general-form');
    else { const general=settingsForms.get('general-form');if(general){general.baseline=structuredClone(general.values);const form=$('#general-form');if(form)form.dataset.draftBaseline=JSON.stringify(general.baseline);} }
    if(refresh)void loadDashboard().catch(error=>notify('配置已保存，概览刷新失败：'+String(error),'error'));
    if(section==='general'&&$('#general-form'))resetSettingsForm('general-form');
    else if(state.page==='settings')renderSettings(!clearForms);
    return true;
  }catch(error){for(const passwordRef of preparedHosts)await api('hosts.credentials.delete',{passwordRef}).catch(()=>{});throw error;}
  finally{state.settingsCommitting=false;scheduleRefreshes();controls.forEach(([c,disabled])=>{if(c.isConnected)c.disabled=disabled;});updateSettingsSaveBar();}
}
function discardAllSettings(){
  state.settingsDraft=null;state.settingsBaseline=null;settingsForms.clear();pendingAPIKeys.clear();pendingHostPasswords.clear();preparedAPIKeys.clear();state.saveError='';renderSettings(false);
}
function savedFeedback(section){return `<span class="saved-feedback" data-save-section="${section}" role="status">${state.savedSection===section&&state.savedAt?'已保存 · '+timeLabel(state.savedAt):''}</span>`;}
function markSaved(){
  state.savedAt=Date.now();state.savedSection=state.settingsTab;state.saveError='';
  const message='已保存 · '+timeLabel(state.savedAt);notify(message);
  document.querySelectorAll('.saved-feedback').forEach(el=>{if(el.dataset.saveSection===state.savedSection){el.textContent=message;delete el.dataset.status;}});
  setTimeout(()=>{if($('#message').dataset.kind==='info'&&$('#message').textContent===message)notify('');},3000);
}

function option(value,label,current) { return `<option value="${escapeHTML(value)}" ${String(value) === String(current) ? 'selected' : ''}>${escapeHTML(label)}</option>`; }
function stat(label,value,detail,icon) {
  const paths={'✧':'M12 3 9 9 3 12l6 3 3 6 3-6 6-3-6-3Z','▱':'M4 7h16v10H4ZM8 10v4m4-4v4m4-4v4','↗':'M5 19 19 5M8 5h11v11','$':'M12 2v20m5-16H9a4 4 0 0 0 0 8h6a4 4 0 0 1 0 8H6'};
  icon=`<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[icon]??paths['✧']}"/></svg>`;
  return `<div class="stat"><div class="stat-label"><span>${label}</span><b>${icon}</b></div><div class="stat-value">${value}</div>${detail?`<small>${detail}</small>`:''}</div>`; }
function render() {
  captureSettingsForms();savePanelView();
  document.querySelectorAll('[data-page]').forEach(b=>{const active=b.dataset.page===state.page;b.classList.toggle('active',active);if(active)b.setAttribute('aria-current','page');else b.removeAttribute('aria-current');});
  const title=$('#title');if(title)title.textContent = {agent:'使用概览',servers:'服务器',settings:'设置'}[state.page];
  for(const id of ['live-sessions']){const el=$('#'+id);if(el)el.hidden=state.page!=='agent';}
  updateActivity();if (!state.settings) return;
  window.AieyesTheme?.apply(state.settings.appearance);
  if (state.page === 'agent') renderAgent(); else if (state.page === 'servers') renderServers(); else renderSettings();
  updateHeadingIndicators();
}
function attention(reason,action,id){return `<div class="attention" role="status">${uiIcon('attention')}<span>${escapeHTML(reason)}</span><button id="${id}" type="button">${action}</button></div>`;}
function bindAttention(){if($('#pricing-attention'))$('#pricing-attention').onclick=openPricing;if($('#sampling-attention'))$('#sampling-attention').onclick=openSampling;}

function creditBalance(q, record, compact=false) {
  const balance=q.credits?.unlimited?'无限':q.credits?.balance!=null?creditAmount(q.credits.balance)+' credits':q.credits?.hasCredits?'数量未知':'—';
  const amount=Number(q.credits?.balance), value=record?.valuePer1000;
  const valid=!q.credits?.unlimited&&q.credits?.balance!=null&&creditAmount(q.credits.balance)!=='—'&&Number.isFinite(amount)&&record?.status!=='pending'&&Number.isFinite(value)&&Number.isFinite(amount*value/1000);
  const full=balance+(valid?' ≈ '+money(Math.round((amount*value/1000+Number.EPSILON*Math.abs(amount*value/1000))*100)/100)+' USD':'');
  return `<span class="credit-balance" title="${escapeHTML('余额 '+full+' · '+(q.creditsUpdatedAt?'更新于 '+date(q.creditsUpdatedAt):'尚未取得 credits 信息'))}"><span class="credit-label">${uiIcon('credit')}余额</span> <strong>${compact?balance.replace(/ credits$/,''):full}</strong></span>`;
}
function creditRow(q,record,history=[]) {
  const amount=AieyesUI.compactMoney(record?.status==='pending'?null:record?.valuePer1000);
  const title=record?`1000 credits ≈ ${money(record.valuePer1000)} USD · ${estimateStatus(record)} · ${date(record.checkpointAt)}`:'尚无有效采样';
  return `<div class="credit-row">${creditBalance(q,record)}</div><button class="credit-estimate-entry estimate-entry" data-credit-estimate="${escapeHTML(q.provider+':'+q.accountId)}" title="${escapeHTML(title)}">${uiIcon('trend')}<span>credits估值（当前1000credits≈${amount}）</span>${uiIcon('right')}</button>${estimateNotice(history,true)}`;
}
function tokenSummary(summary) {
  return `<div class="token-summary"><button type="button" id="token-summary" aria-expanded="false" aria-controls="token-popover" aria-label="总 Token，点击查看详情">${stat('总 Token',compact(summary.total)+' <span class="token-rate">('+pct(cacheRate(summary.tokens))+')</span>','','✧')}<span class="token-hint">点击查看详情</span></button><div id="token-popover" role="region" aria-label="Token 细分" hidden>${cacheCard(summary.tokens).replace('class="stat cache-card"','class="card cache-card"')}</div></div>`;
}
function closeTokenSummary() {
  const popover=$('#token-popover');if(popover)popover.hidden=true;
  $('#token-summary')?.setAttribute('aria-expanded','false');
}
function bindTokenSummary() {
  const button=$('#token-summary');if(!button)return;
  button.onmousedown=event=>event.preventDefault();
  button.onclick=()=>{const popover=$('#token-popover');popover.hidden=!popover.hidden;button.setAttribute('aria-expanded',String(!popover.hidden));};
}
document.addEventListener('pointerdown',event=>{if(!event.target.closest('.token-summary'))closeTokenSummary();});
document.addEventListener('keydown',event=>{if(event.key==='Escape')closeTokenSummary();});

function cacheCard(t){return `<div class="stat cache-card"><div class="token-breakdown">${[['普通输入',t.input],['输出',t.output],['缓存读取',t.cacheRead],['缓存写入',t.cacheWrite]].map(([label,value])=>`<div><span>${label}</span><strong title="${value.toLocaleString()} Token">${compact(value)}</strong></div>`).join('')}<div class="cache-hit"><span title="缓存读取 ÷（普通输入＋缓存读取＋缓存写入）">读取命中率</span><strong>${pct(cacheRate(t))}</strong></div></div></div>`;}

const cacheRate = t => { const input=t.input+t.cacheRead+t.cacheWrite; return input ? t.cacheRead/input*100 : null; };
const missingLabels = gap => [['input','输入'],['output','输出'],['cacheRead','缓存读取'],['cacheWrite','缓存写入']].filter(([k])=>gap.tokens[k]>0).map(([,label])=>label).join('、');
function gapList() {
  const gaps=state.dashboard?.pricingGaps??[];
  return gaps.length ? `<div class="card pricing-gaps"><h2>所选范围 · 待计价模型</h2>${gaps.map((g,i)=>`<div class="list-row"><div class="row-body"><strong>${escapeHTML(g.model)}</strong><small>缺少${missingLabels(g)} · ${compact(g.unpricedTokens)} Token</small></div><button data-gap-map="${i}" title="把日志里的模型名映射到 OpenRouter 模型 ID">映射</button><button data-gap-price="${i}" title="为缺失计价的模型补充价格">补充价格</button></div>`).join('')}</div>` : '';
}
async function openPricing() {
  if(PANEL){window.__TAURI__.core.invoke('desktop_action',{action:'prices'}).catch(e=>notify(String(e)));return;}
  state.page='settings';state.settingsTab='prices';state.prices=await api('prices.list');render();
}
function renderAgent() {
  const d=state.dashboard;if(!d)return;
  if(PANEL){renderAgentPanel(d);return;}
  const view=rememberView();
  const s=d.summary,t=s.tokens,models=d.models;
  const account=historicalAccounts().find(a=>a.provider+':'+a.id===state.accountKey);
  const sources=state.settings.sources.filter(src=>{const ids=d.sources.find(s=>s.id===src.id)?.accountIds??[src.accountId];return (!state.provider||src.provider===state.provider)&&(!state.accountKey||(state.accountKey==='none'?ids.includes(''):src.provider===account?.provider&&ids.includes(account?.id)));});
  const trendModels=[...new Set(d.dayModels.map(r=>r.model))].sort();
  $('#title').textContent='Agent 概览';
  $('#content').innerHTML=`${state.settings.sources.length?'':onboarding()}<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).filter(([key])=>key!=='agy').map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','未关联账户',state.accountKey)}${historicalAccounts().filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select><select id="source" aria-label="数据源">${option('','全部数据源',state.sourceId)}${sources.map(s=>option(s.id,s.name,state.sourceId)).join('')}</select><select id="model" aria-label="模型">${option('','全部模型',state.model)}${[...new Set([...(d.modelOptions??trendModels),...(state.model?[state.model]:[])])].map(m=>option(m,m,state.model)).join('')}</select></div>
  ${s.total>0&&s.pricedTokens<s.total?attention('计价未完成：部分 Token 缺少价格','补充价格','pricing-attention'):''}
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'} ${refreshIndicator('scan')}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':n===365?'最近一年':`最近 ${n} 天`,state.days)).join('')}</select></div>
  <div class="stats">${stat('总 Token',compact(s.total),'','✧')}${stat('API 等价成本'+(s.total>0&&s.pricedTokens<s.total?' <button id="repair-pricing" class="pricing-alert" aria-label="计价未完成，设置模型价格" title="部分 Token 尚未计价">!</button>':''),s.pricedTokens||!s.total?money(s.cost):'—','','$')}</div>${cacheCard(t)}${quotaSection(d)}
  <div class="section-head"><h2>${state.days===1?'近 7 天趋势':'使用趋势'}</h2><div id="cost-mode" class="metric-picker" role="group" aria-label="统计指标"><button type="button" data-cost-mode="false" aria-pressed="${!state.cost}">Token</button><button type="button" data-cost-mode="true" aria-pressed="${state.cost}">API 等价成本</button></div></div>
  <div class="chart-row"><div class="card"><h2>每日用量 · 按模型</h2><canvas id="trend" aria-label="按模型堆叠的每日用量，下方有逐日数据"></canvas><details class="model-legend"><summary>模型图例 · 点击显示或隐藏</summary><div class="model-key">${trendModels.map(m=>`<button data-trend-model="${escapeHTML(m)}" aria-label="显示或隐藏模型 ${escapeHTML(m)}"><i style="background:${color(m)}"></i>${escapeHTML(m)}</button>`).join('')}</div></details></div><div class="card"><h2>所选范围 · 模型分布</h2>${modelDistribution(models)}</div></div>
  ${samplingBanner(d)}<div class="card"><details class="daily" data-agent-detail="daily"><summary><h2>每日明细 · 可选择与复制精确数值</h2></summary>${dailyTable(d.trendDays,d.dayModels)}</details></div>
  <div class="card"><h2>过去 365 天</h2>${heatmapHTML(d.heatmap)}</div>
  `;
  bindSetup();
  $('#provider').onchange=async e=>{state.provider=e.target.value;state.accountKey='';state.sourceId='';state.model='';await changeFilters();};
  $('#account').onchange=async e=>{state.accountKey=e.target.value;state.sourceId='';await changeFilters();};
  $('#source').onchange=async e=>{state.sourceId=e.target.value;await changeFilters();};
  $('#model').onchange=async e=>{state.model=e.target.value;await changeFilters();};
  $('#days').onchange=async e=>{state.days=Number(e.target.value);await changeFilters();};
  document.querySelectorAll('[data-cost-mode]').forEach(button=>button.onclick=()=>{state.cost=button.dataset.costMode==='true';renderAgent();document.querySelector(`[data-cost-mode="${state.cost}"]`)?.focus({preventScroll:true});});
  if($('#read-quotas'))$('#read-quotas').onclick=()=>quotas();
  bindQuotaTools();bindAttention();
  if($('#repair-pricing'))$('#repair-pricing').onclick=openPricing;
  shortenEmptyUsage(d);restoreView(view);
  bindAnalysis();requestAnimationFrame(drawTrend);
}
function panelTrendExpanded(){try{return localStorage.getItem('aieyes.panel.trend')==='true';}catch(_){return false;}}
function renderAgentPanel(d) {
  const view=rememberView(),s=d.summary,t=s.tokens,trendTitle=`近 ${Math.max(7,state.days)} 天趋势与每日明细`;
  $('#content').innerHTML=`${state.settings.sources.length?'':onboarding()}<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).filter(([key])=>key!=='agy').map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','未关联账户',state.accountKey)}${historicalAccounts().filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select></div>
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'} ${refreshIndicator('scan')}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':n===365?'最近一年':`最近 ${n} 天`,state.days)).join('')}</select></div>
  <div class="stats panel-stats">${tokenSummary(s)}${stat('API 等价成本'+(s.total>0&&s.pricedTokens<s.total?' <button id="repair-pricing" class="pricing-alert" aria-label="计价未完成，设置模型价格" title="部分 Token 尚未计价">!</button>':''),s.pricedTokens||!s.total?money(s.cost):'—','','$')}</div>
  ${quotaSection(d)}
  ${samplingBanner(d)}
  <div class="card"><details data-agent-detail="panel-trend" ${panelTrendExpanded()?'open':''}><summary><h2>${trendTitle}</h2></summary><canvas id="trend" class="panel-chart" aria-label="${trendTitle}，按模型的每日用量"></canvas><details class="model-legend"><summary>模型图例 · 点击显示或隐藏</summary><div class="model-key">${[...new Set(d.dayModels.map(r=>r.model))].sort().map(m=>`<button data-trend-model="${escapeHTML(m)}" aria-label="显示或隐藏模型 ${escapeHTML(m)}"><i style="background:${color(m)}"></i>${escapeHTML(m)}</button>`).join('')}</div></details><h3>每日明细</h3>${dailyTable(d.trendDays,d.dayModels)}</details></div>
  <button id="open-detail" class="panel-wide" title="打开完整详情窗口">用量详情</button>`;
  $('#provider').onchange=async e=>{state.provider=e.target.value;state.accountKey='';state.sourceId='';state.model='';await changeFilters();};
  $('#account').onchange=async e=>{state.accountKey=e.target.value;state.sourceId='';await changeFilters();};
  $('#days').onchange=async e=>{state.days=Number(e.target.value);await changeFilters();};
  if($('#read-quotas'))$('#read-quotas').onclick=()=>quotas();
  bindQuotaTools();bindAttention();bindTokenSummary();
  const trend=$('[data-agent-detail=panel-trend]');if(trend){const persist=()=>{if(!trend.isConnected)return;try{localStorage.setItem('aieyes.panel.trend',String(trend.open));}catch(_){}};trend.addEventListener('toggle',persist);trend.querySelector('summary').onclick=event=>{event.preventDefault();trend.open=!trend.open;persist();};}
  if($('#repair-pricing'))$('#repair-pricing').onclick=openPricing;
  $('#open-detail').onclick=openDetail;
  bindSetup();shortenEmptyUsage(d);restoreView(view);
  bindAnalysis();requestAnimationFrame(drawTrend);
}
function shortenEmptyUsage(d) {
  if(d.summary.total||(d.trendDays??[]).some(day=>day.total)||(d.heatmap??[]).some(day=>day.total))return;
  $('#content .stats')?.remove();$('#content > .cache-card')?.remove();
  if(!state.settings.sources.length){$('#content .chart-row')?.remove();$('#trend')?.closest('.card')?.remove();$('#content .daily')?.closest('.card')?.remove();$('#content .heatmap')?.closest('.card')?.remove();$('#cost-mode')?.closest('.section-head')?.remove();return;}
  const trend=$('#trend'),chart=trend?.closest('.chart-row')??trend?.closest('.card');
  if(chart){
    const usage=AieyesUI.usageSources(state.settings),fresh=AieyesUI.freshness(state.settings,d),filtered=Boolean(state.provider||state.accountKey||state.sourceId||state.model),onlyQuota=state.settings.sources.every(s=>s.provider==='deepseek');
    const title=onlyQuota?'此来源提供账户限额':fresh.failed?'记录同步失败':filtered?'没有匹配的用量记录':!fresh.timestamp?'尚未同步用量记录':'此时间范围暂无用量';
    const description=onlyQuota?'可在实时账户区查看余额与限额；接入日志后即可分析用量。':fresh.failed?'请在对应来源查看错误并重试，已有记录仍保留。':filtered?'尝试清除筛选或扩大时间范围。':!fresh.timestamp?'来源已配置，首次同步后将显示统计。':'可以扩大日期范围查看历史，或同步最新日志。';
    const empty=document.createElement('div');empty.className='card empty-usage';empty.innerHTML='<h2>'+title+'</h2><p class="muted">'+description+'</p><div class="actions">'+(onlyQuota?'<button id="empty-add-source">添加日志来源</button>':'<button id="empty-sync">同步记录</button><button id="empty-range">扩大到最近一年</button>'+(filtered?'<button id="empty-clear">清除筛选</button>':''))+'</div>';chart.replaceWith(empty);
    if($('#empty-sync'))$('#empty-sync').onclick=()=>scan();if($('#empty-add-source'))$('#empty-add-source').onclick=()=>openSetup();if($('#empty-range'))$('#empty-range').onclick=()=>PANEL?openDetail():changeFilters({days:365});if($('#empty-clear'))$('#empty-clear').onclick=()=>changeFilters({provider:'',accountKey:'',sourceId:'',model:''});
  }
  $('#cost-mode')?.closest('.section-head')?.remove();
  $('#content .daily')?.closest('.card')?.remove();
  $('#content .heatmap')?.closest('.card')?.remove();
}
function dailyTable(days,rows) {
  return `<div class="daily-table table-scroll"><table><thead><tr><th scope="col">日期 / 模型</th><th scope="col">Token</th><th scope="col">读取命中率</th><th scope="col">已计价成本 USD</th></tr></thead>${[...days].reverse().map(day=>`<tbody data-agent-detail="day:${escapeHTML(day.key)}"><tr class="day"><th scope="row"><button data-day-toggle="${day.key}" aria-expanded="false" aria-label="展开 ${day.key} 的模型明细">${day.key}</button></th><td title="${day.total} Token">${compact(day.total)}</td><td>${pct(cacheRate(day.tokens))}</td><td>${day.pricedTokens||!day.total?day.cost.toFixed(6):'—'}</td></tr>${rows.filter(r=>r.day===day.key).map(r=>`<tr class="day-model" hidden><th scope="row"><span>${escapeHTML(r.model)}</span></th><td title="${r.usage.total} Token">${compact(r.usage.total)}</td><td>${pct(cacheRate(r.usage.tokens))}</td><td>${r.usage.pricedTokens?r.usage.cost.toFixed(6):'—'}</td></tr>`).join('')}</tbody>`).join('')}</table></div>`;
}

function estimateSummary(records, credits=false) {
  const entries=AieyesUI.estimateEntries(records,credits), latest=AieyesUI.canonicalEstimates(records)[0];
  return '<div class="estimate-summary">'+entries.map(e=>`<div><span class="estimate-amount">${escapeHTML(e.label)} ≈ ${money(e.value)} USD</span>${e.historical?`<small>历史采样 · ${date(e.record.checkpointAt)}${e.record.originalEstimateId?' · 已修正':''}</small>`:''}</div>`).join('')+(latest&&(![latest.fiveHourValue,latest.weeklyValue,latest.valuePer1000].some(Number.isFinite)||latest.status==='pending')?`<p class="muted">最新采样 · ${estimateStatus(latest)} · ${escapeHTML(AieyesUI.estimateIssue(latest))}</p>`:'')+'</div>';
}
function estimateNotice(records,credits=false) {
  const latest=AieyesUI.canonicalEstimates(records)[0],entries=AieyesUI.estimateEntries(records,credits),notes=[];
  if(entries.some(e=>e.historical))notes.push('历史采样');
  if(entries.some(e=>e.record.originalEstimateId))notes.push('已修正');
  if(latest&&(!(credits?[latest.valuePer1000]:[latest.fiveHourValue,latest.weeklyValue,latest.weeklyRatioValue]).some(Number.isFinite)||latest.status==='pending'))notes.push('最新采样 · '+estimateStatus(latest)+' · '+AieyesUI.estimateIssue(latest));
  return notes.length?`<div class="estimate-notice muted">${escapeHTML(notes.join(' · '))}</div>`:'';
}
function estimateEntry(q,records,group='') {
  const entries=AieyesUI.estimateEntries(records),latest=AieyesUI.canonicalEstimates(records)[0];
  const title=entries.map(e=>`${e.label} ≈ ${money(e.value)} USD${e.historical?' · 历史采样':''} · ${date(e.record.checkpointAt)}`).join('；') || '开始采样以估算额度价值';
  const label=AieyesUI.estimateLine(records);
  return `<button class="estimate-entry quota-estimate-entry${latest?' sampling-entry':''}" data-estimate="${escapeHTML(q.provider+':'+q.accountId)}" data-estimate-group="${escapeHTML(group)}" title="${escapeHTML(title+'；7d 顺序：容量倍率 / 同期消耗（旧记录为整周）')}">${uiIcon('trend')}<span class="estimate-inline">${escapeHTML(label)}</span>${uiIcon('right')}</button>${estimateNotice(records)}`;
}
function quotaCard(q) {
  const key=q.provider+':'+q.accountId, records=(state.dashboard?.quotaEstimates??[]).filter(e=>e.accountKey===key), credits=(state.dashboard?.creditEstimates??[]).filter(e=>e.accountKey===key), latest=AieyesUI.canonicalEstimates(records)[0], credit=AieyesUI.estimateEntries(credits,true)[0]?.record ?? credits[0];
  const windowHTML=w=>`<div class="quota-window"><div class="between tiny"><span>${escapeHTML(w.name)}</span><strong style="color:${quotaColor(Number.isFinite(w.usedPercent)?resourcePercent(100-w.usedPercent):null)}">剩余 ${pct(Number.isFinite(w.usedPercent)?resourcePercent(100-w.usedPercent):null)}</strong></div>${resourceBar(Number.isFinite(w.usedPercent)?100-w.usedPercent:null,w.name+'剩余额度',true)}<div class="tiny muted quota-reset" data-reset-at="${w.resetsAt??0}" data-window-minutes="${w.windowMinutes??''}" data-used-percent="${Number.isFinite(w.usedPercent)?w.usedPercent:''}">${AieyesUI.quotaReset(w)}</div></div>`;
  const collapsible=true;
  let expanded=true;try{const stored=localStorage.getItem('quota.expanded.v2.'+(PANEL?'panel.':'detail.')+key);if(collapsible&&stored!=null)expanded=stored==='true';}catch(_){}

  let windows='<div class="quota-windows">'+q.windows.map(windowHTML).join('')+'</div>';
  if(['antigravity','agy'].includes(q.provider)){
    const groups=new Map();for(const w of q.windows){const group=w.groupName||w.name.split(' · ').slice(0,-1).join(' · ');if(!groups.has(group))groups.set(group,[]);groups.get(group).push(w);}
    windows=`<div class="agy-groups">${[...groups].map(([group,rows])=>`<div class="agy-group"><strong>${escapeHTML(group)}</strong><div class="agy-windows">${rows.sort((a,b)=>(a.windowMinutes??0)-(b.windowMinutes??0)).map(w=>windowHTML({...w,name:w.windowMinutes===10080?'7d':w.windowMinutes===300?'5h':w.name})).join('')}</div>${estimateEntry(q,records.filter(e=>e.groupId===(rows[0]?.groupId||group)),rows[0]?.groupId||group)}</div>`).join('')}</div>`;
  }
  return `<div class="card quota-card" data-quota="${escapeHTML(key)}"><details class="quota-disclosure" data-collapsible="${collapsible}" data-agent-detail="quota:${escapeHTML(key)}" ${expanded?'open':''}><summary class="quota-title"><h3>${providerMark(q.provider)}<span class="quota-heading-label"><span class="quota-name" title="${escapeHTML(q.name)}">${escapeHTML(q.name)}</span>${q.identity?.email&&q.identity.email!==q.name?`<small class="quota-email" title="${escapeHTML(q.identity.email)}">${escapeHTML(q.identity.email)}</small>`:''}</span></h3>${subscriptionBadge(q.plan)}${q.provider==='codex'?`<span class="quota-credit-summary">${creditBalance(q,credit,true)}</span>`:''}${collapsible?`<span class="quota-chevron" aria-hidden="true">${uiIcon('chevron')}</span>`:''}</summary></details><div class="quota-content"><div class="between tiny muted quota-meta"><span>${escapeHTML(providers[q.provider]??q.provider)}</span><span>${q.origin==='log'?'记录':'更新'} ${date(q.updatedAt)}</span></div>${q.metadataError?`<p class="tiny muted quota-identity-note">${escapeHTML(q.metadataError)}${q.identity?.stale&&q.plan?' · 显示上次订阅':''}</p>`:''}${(q.balances??[]).map(b=>`<div class="quota-window"><div class="between"><span>可用余额</span><strong>${escapeHTML(b.currency)} ${escapeHTML(b.total)}</strong></div><div class="between tiny muted balance-detail"><span>赠送 ${escapeHTML(b.granted)}</span><span>充值 ${escapeHTML(b.toppedUp)}</span></div></div>`).join('')}${q.isAvailable===false?'<p class="error">当前余额不足以调用 API</p>':''}${windows}${q.bankReset?`<details class="bank" data-agent-detail="bank:${escapeHTML(key)}"><summary>Bank Reset · ${q.bankReset.availableCount} 次可用</summary>${(q.bankReset.credits??[]).map(c=>`<div class="between tiny muted"><span>${escapeHTML(c.title??'Reset Credit')}</span><span>到期 ${date(c.expiresAt)}</span></div>`).join('')}</details>`:''}${q.error?`<p class="error quota-error" title="${escapeHTML(q.error)}"><span class="error-detail">${escapeHTML(q.error)}</span><span class="short-error">限额更新失败 · 展开查看</span></p>`:''}${!['antigravity','agy'].includes(q.provider)&&(q.windows.some(w=>w.windowMinutes===300||w.windowMinutes===10080)||latest)?estimateEntry(q,records):''}${q.provider==='codex'?creditRow(q,credit,credits):''}</div></div>`;
}

function drawTrend() {
  const canvas=$('#trend');if(!canvas||!state.dashboard)return;
  const width=canvas.clientWidth,height=canvas.clientHeight,dpr=window.devicePixelRatio||1;canvas.width=width*dpr;canvas.height=height*dpr;
  const ctx=canvas.getContext('2d');ctx.scale(dpr,dpr);
  const days=state.dashboard.trendDays,rows=state.dashboard.dayModels.filter(r=>!hiddenTrendModels.has(r.model)),values=days.map(d=>rows.filter(r=>r.day===d.key).reduce((n,r)=>n+(state.cost?r.usage.cost:r.usage.total),0)),max=Math.max(1,...values);
  const left=58,top=12,bottom=30,plot=height-top-bottom,space=(width-left)/Math.max(1,days.length);
  const style=getComputedStyle(canvas),muted=style.getPropertyValue('--muted');ctx.font=`${style.getPropertyValue('--font-secondary').trim() || '14px'} ${style.fontFamily}`;ctx.fillStyle=muted;ctx.textAlign='right';
  for(let i=0;i<3;i++){const y=top+plot*i/2;ctx.fillText(state.cost?money(max*(1-i/2)):compact(max*(1-i/2)),left-9,y+3);ctx.strokeStyle='rgba(140,145,170,.15)';ctx.beginPath();ctx.moveTo(left,y);ctx.lineTo(width,y);ctx.stroke();}
  days.forEach((d,i)=>{let used=0;const x=left+i*space+space*.2;rows.filter(r=>r.day===d.key).forEach(r=>{const h=(state.cost?r.usage.cost:r.usage.total)/max*plot;ctx.fillStyle=color(r.model);ctx.fillRect(x,top+plot-used-h,Math.max(1,space*.6),h);used+=h;});if(i%Math.max(1,Math.floor(days.length/7))===0){ctx.fillStyle=muted;ctx.textAlign='center';ctx.fillText(d.key.slice(5),x+space*.3,height-5);}});
  canvas.tabIndex=0;
  let selected=Math.min(state.trendDayIndex??0,Math.max(0,days.length-1));
  const describe=i=>{const day=days[i];if(!day)return;state.trendDayIndex=i;const label=day.key+' · '+exactUsage(day);canvas.removeAttribute('title');let reading=$('#trend-reading');if(!reading){reading=document.createElement('p');reading.id='trend-reading';reading.className='muted tiny';reading.setAttribute('aria-live','polite');canvas.after(reading);}reading.textContent=label;canvas.setAttribute('aria-label',label+'；左右键选择日期，回车查看精确值');};
  canvas.onmousemove=e=>{const i=Math.floor((e.offsetX-left)/space);if(days[i])describe(i);};
  canvas.onfocus=()=>describe(selected);
  canvas.onkeydown=e=>{if(['ArrowLeft','ArrowRight','Home','End'].includes(e.key)){e.preventDefault();selected=e.key==='Home'?0:e.key==='End'?days.length-1:Math.max(0,Math.min(days.length-1,(state.trendDayIndex??selected)+(e.key==='ArrowLeft'?-1:1)));describe(selected);}else if(e.key==='Enter'&&days[state.trendDayIndex??selected])showUsageDay(days[state.trendDayIndex??selected]);};
  // A background redraw or metric switch keeps the user's selected date readable.
  if(state.trendDayIndex!=null&&days.length)describe(selected);

}
function resourcePercent(value) { return Number.isFinite(value)?Math.max(0,Math.min(100,value)):null; }
function resourceColor(value) { return value>=90?'var(--resource-high)':value>=70?'var(--resource-warn)':'var(--accent)'; }
function capacityPercent(used,total) { return Number.isFinite(used)&&Number.isFinite(total)&&total>0?used/total*100:null; }
function quotaColor(value) { return value==null?'var(--muted)':value<10?'var(--resource-high)':value<=30?'var(--resource-warn)':'var(--accent)'; }
function phaseColor(phase) { return getComputedStyle(document.documentElement).getPropertyValue('--phase-'+phase).trim() || getComputedStyle(document.documentElement).getPropertyValue('--phase-idle').trim(); }
function resourceBar(value,label,quota=false) {
  const n=resourcePercent(value);
  return `<div class="resource-bar${n==null?' unavailable':''}" role="${n==null?'img':'progressbar'}" aria-label="${escapeHTML(label)}" ${n==null?'':`aria-valuemin="0" aria-valuemax="100" aria-valuenow="${n}"`}><span style="width:${n??0}%;background:${quota?quotaColor(n):resourceColor(n)}"></span></div>`;
}
function resourceRing(label,value,detail='') {
  const n=resourcePercent(value);
  return `<div class="resource-gauge"><div class="resource-ring" role="img" aria-label="${escapeHTML(label)} ${pct(n)}${n>=90?'，高负载':''}"><svg viewBox="0 0 100 100" aria-hidden="true"><circle class="ring-track" cx="50" cy="50" r="42"/><circle class="ring-value" ${n==null||n===0?'hidden':''} cx="50" cy="50" r="42" pathLength="100" stroke-dasharray="${n??0} 100" style="stroke:${resourceColor(n)}"/></svg><strong>${pct(n)}</strong></div><div><h3>${escapeHTML(label)}</h3>${n>=90?'<span class="resource-alert">高负载</span>':''}${detail?`<span class="muted tiny">${escapeHTML(detail)}</span>`:''}</div></div>`;
}
function hostStatus(host,result) {
  if(!host.enabled)return '已暂停';
  if(result?.error)return '连接失败';
  if(!result?.sample)return '等待采样';
  const timestamp=Number(result.sample.timestamp);
  if(!Number.isFinite(timestamp)||timestamp<=0||Date.now()-timestamp*1000>10000)return '数据已延迟';
  return Object.keys(result.sample.errors??{}).length?'部分采集失败':'正常';
}
function updateHostStatuses() {
  if(state.page!=='servers'||!state.settings)return;
  for(const element of document.querySelectorAll('[data-host-status]')){
    const host=monitoredHosts().find(row=>row.id===element.dataset.hostStatus);if(!host)continue;
    const status=hostStatus(host,state.hosts.find(row=>row.id===host.id));element.onclick=()=>sample(host.id);element.title=status+' · 点击刷新服务器';
    if(element.dataset.status!==status){element.dataset.status=status;element.querySelector('[data-host-status-label]').textContent=status;}
  }
}
function hostShows(host,key) { return host.details==null||host.details.includes(key); }
function hostDevices(host,key,rows) {
  if(!(host.metrics??Object.keys(groups)).includes(key))return [];
  const selected=(host.devices??[]).filter(token=>token.startsWith(key+':'));
  return (rows??[]).filter(d=>(!selected.length&&(key!=='filesystems'||AieyesSelect.recommendedFilesystem(d.id,d.type)))||selected.includes(key+':__all__')||(key==='cpu'&&d.id==='cpu')||selected.includes(key+':'+d.id));
}
function usedCapacity(total,available) { return Number.isFinite(total)&&Number.isFinite(available)?total-available:null; }
function uptimeText(seconds) {
  if(!Number.isFinite(seconds)||seconds<0)return '—';
  const minutes=Math.floor(seconds/60);
  if(!Number.isSafeInteger(minutes))return '—';
  return [minutes>=1440?Math.floor(minutes/1440)+' 天':null,minutes>=60?Math.floor(minutes%1440/60)+' 小时':null,minutes%60+' 分'].filter(v=>v!=null).join(' ');
}
function capacityPair(used,total) {
  const scale=Math.max(Number.isFinite(total)?total:0,Number.isFinite(used)?used:0),units=['B','KiB','MiB','GiB','TiB','PiB'];
  const power=scale>0?Math.min(units.length-1,Math.max(0,Math.floor(Math.log(scale)/Math.log(1024)))):0;
  const number=n=>Number.isFinite(n)?(n/1024**power).toFixed(1).replace(/\.0$/,''):'—';
  return number(used)+'/'+number(total)+' '+units[power];
}
function resourceStrip(title,percent,detail='') {
  const value=resourcePercent(percent),label=(detail?detail+' · ':'')+(value==null?'—':Math.round(value)+'%');
  return `<div class="server-resource-strip" title="${escapeHTML(title+' '+label)}"><div class="resource-strip-heading"><span>${escapeHTML(title)}</span><strong${value>=90?' class="error"':''}>${escapeHTML(label)}</strong></div>${resourceBar(value,title+' '+label)}</div>`;
}
function serverSummary(host,sample) {
  const enabled=key=>(host.metrics??Object.keys(groups)).includes(key), memory=sample?.memory;
  const gpus=hostDevices(host,'gpu',sample?.gpu), networks=hostDevices(host,'network',sample?.network);
  const system=(enabled('cpu')?resourceRing('CPU',sample?.cpu?.find(c=>c.id==='cpu')?.utilization):'')+
    (enabled('memory')?resourceRing('内存',capacityPercent(usedCapacity(memory?.total,memory?.available),memory?.total),`${bytes(usedCapacity(memory?.total,memory?.available))} / ${bytes(memory?.total)}`):'');
  const gpuCards=gpus.map(g=>`<div class="server-gpu" data-gpu="${escapeHTML(g.id)}"><div class="server-gpu-title"><strong>GPU ${escapeHTML(g.id)}</strong><span title="${escapeHTML(g.name??'—')}">${escapeHTML(g.name??'—')}</span></div><div class="server-gpu-metrics" data-memory="${hostShows(host,'gpuMemory')}">${resourceStrip('利用率',g.utilization)}${hostShows(host,'gpuMemory')?resourceStrip('显存',capacityPercent(g.memoryUsedMiB,g.memoryTotalMiB),capacityPair(g.memoryUsedMiB==null?null:g.memoryUsedMiB*1048576,g.memoryTotalMiB==null?null:g.memoryTotalMiB*1048576)):''}</div></div>`).join('');
  const filesystems=hostDevices(host,'filesystems',sample?.filesystems),fsSummary=enabled('filesystems')?`<div class="server-filesystems"><span class="muted">文件系统</span><div class="server-filesystem-list">${filesystems.length?filesystems.map(fs=>`<div data-filesystem="${escapeHTML(fs.id)}">${resourceStrip(fs.id,capacityPercent(fs.used,fs.total),capacityPair(fs.used,fs.total))}</div>`).join(''):`<span class="muted">${sample?.filesystems?'无已选挂载点':'—'}</span>`}</div></div>`:'';
  return `<div class="server-summary-metrics">${system?`<div class="server-system-metrics">${system}</div>`:''}${gpuCards?`<div class="server-gpu-list">${gpuCards}</div>`:enabled('gpu')?`<span class="muted">GPU ${sample?.gpu?'无已选设备':'—'}</span>`:''}${fsSummary}<div class="server-summary-facts">${hostShows(host,'uptime')?`<span data-host-uptime>连续运行 <strong>${uptimeText(sample?.uptime)}</strong></span>`:''}${hostShows(host,'networkTotals')&&enabled('network')?(networks.length?networks.map(n=>`<span class="server-network-total" data-network-total="${escapeHTML(n.id)}"><span>${escapeHTML(n.id)} 累计</span><strong>↓ ${bytes(n.rxBytes)} · ↑ ${bytes(n.txBytes)}</strong></span>`).join(''):`<span>累计流量 ${sample?.network?'无已选网卡':'—'}</span>`):''}<span class="server-sample-time muted">采样 ${date(sample?.timestamp)}${!host.enabled?' · 已暂停，保留旧读数':''}</span></div></div>`;
}
function renderServers() { window.AieyesServers.render(); }
function serverGroupContent(key,value,host) {
  const show=key=>hostShows(host,key);
  const row=(name,value)=>`<div class="metric-row"><span>${escapeHTML(name)}</span><strong>${value}</strong></div>`;
  const meter=(name,value,percent)=>row(name,value)+resourceBar(percent,name);
  let content='';
  if(key==='memory')content=row('可用',bytes(value.available))+(show('memoryCache')?row('缓存 / Buffer',`${bytes(value.cached)} / ${bytes(value.buffers)}`):'')+(show('swap')?meter('Swap',`${bytes(usedCapacity(value.swapTotal,value.swapFree))} / ${bytes(value.swapTotal)}`,capacityPercent(usedCapacity(value.swapTotal,value.swapFree),value.swapTotal)):'');
  else content=value.map(d=>{
    if(key==='cpu')return (d.id==='cpu'?'':meter(d.id,pct(resourcePercent(d.utilization)),d.utilization))+(show('cpuTimes')?row(d.id+' user / system',`${pct(d.userPercent)} / ${pct(d.systemPercent)}`)+row('iowait / steal',`${pct(d.iowaitPercent)} / ${pct(d.stealPercent)}`):'');
    if(key==='gpu')return show('gpuThermals')?row(`GPU ${d.id} 温度 / 功耗`,`${d.temperature==null?'—':d.temperature+'°C'} / ${d.powerWatts==null?'—':d.powerWatts+' W'}`):'';
    if(key==='filesystems')return (show('fsAvailable')||show('fsType')||show('inodes')?row(d.id,''):'')+(show('fsAvailable')?row('可用',bytes(d.available)):'')+(show('fsType')?row(d.device??'',escapeHTML(d.type??'')):'')+(show('inodes')?meter('inode',pct(capacityPercent(usedCapacity(d.inodes,d.inodesFree),d.inodes)),capacityPercent(usedCapacity(d.inodes,d.inodesFree),d.inodes)):'');
    if(key==='disk')return row(d.id,`读 ${speed(d.readBytesPerSecond)} · 写 ${speed(d.writeBytesPerSecond)}`)+(show('diskIops')?row('IOPS 读 / 写',`${d.readIops==null?'—':compact(d.readIops)} / ${d.writeIops==null?'—':compact(d.writeIops)}`):'')+(show('diskBusy')?meter('忙碌率',pct(resourcePercent(d.busyMsPerSecond==null?null:d.busyMsPerSecond/10)),d.busyMsPerSecond==null?null:d.busyMsPerSecond/10):'');
    return row(d.id,`↓ ${speed(d.rxBytesPerSecond)} · ↑ ${speed(d.txBytesPerSecond)}`)+(show('networkErrors')?row('错误 / 丢包',`${compact((d.rxErrors??0)+(d.txErrors??0))} / ${compact((d.rxDrops??0)+(d.txDrops??0))}`):'');
  }).join('');
  return content;
}
function serverGroup(key,label,value,host) {
  const content=serverGroupContent(key,value,host);
  if(!content)return '';
  return `<details class="metric-section" data-metric="${escapeHTML(host.id+':'+key)}"><summary>${label}明细</summary><div class="metric-detail">${content}</div></details>`;
}
function field(name,label,value='',placeholder='',type='text') {return `<div class="form-row"><label for="field-${name}">${label}</label><input id="field-${name}" name="${name}" type="${type}" value="${escapeHTML(value)}" placeholder="${escapeHTML(placeholder)}"></div>`;}
function select(name,label,entries,current) {return `<div class="form-row"><label for="field-${name}">${label}</label><select id="field-${name}" name="${name}">${entries.map(([k,v])=>option(k,v,current)).join('')}</select></div>`;}
function textarea(name,label,value='') {return `<div class="form-row"><label for="field-${name}">${label}</label><textarea id="field-${name}" name="${name}">${escapeHTML(value)}</textarea></div>`;}
function priceRows() {
  const query=state.priceSearch.trim().toLocaleLowerCase();
  const prices=state.prices.filter(p=>[p.id,p.name].some(v=>String(v??'').toLocaleLowerCase().includes(query)));
  return prices.map(p=>`<div class="list-row"><div class="row-body">${escapeHTML(p.id)}<small>输入 ${p.input==null?'—':money(p.input*1e6)} · 输出 ${p.output==null?'—':money(p.output*1e6)}</small></div><button data-map-price="${escapeHTML(p.id)}">一键映射</button><button data-price="${escapeHTML(p.id)}" aria-label="编辑 ${escapeHTML(p.id)} 的价格" title="编辑该模型的价格">编辑</button></div>`).join('')||`<div class="empty">${query?'无匹配模型':'同步模型价格'}</div>`;
}
function bindPrices() {
  document.querySelectorAll('[data-map-price]').forEach(b=>b.onclick=()=>{const input=$('#field-mappingId');input.value=b.dataset.mapPrice;$('#mapping-form').scrollIntoView({block:'start',behavior:matchMedia('(prefers-reduced-motion: reduce)').matches?'instant':'smooth'});input.focus({preventScroll:true});});
  document.querySelectorAll('[data-price]').forEach(b=>b.onclick=()=>editPrice(state.prices.find(p=>p.id===b.dataset.price))); }
function sourceError(id) {
  const error=state.dashboard?.sources.find(s=>s.id===id)?.status?.error;
  const source=state.settings.sources.find(s=>s.id===id),status=state.dashboard?.sources.find(s=>s.id===id)?.status;
  return error?`<span class="error" role="status">${escapeHTML(error)}</span>`:`<small>${source?.provider==='deepseek'?'限额查询来源':status?.updatedAt?'记录同步于 '+date(status.updatedAt):'尚未同步用量记录'}</small>`;
}
const settingsForms = new Map();
let generalSaveTimer,generalSaveVersion=0;
function scheduleGeneralSave() {
  const version=++generalSaveVersion;
  clearTimeout(generalSaveTimer);state.generalSavePending=true;
  generalSaveTimer=setTimeout(async()=>{
    if(state.settingsCommitting||state.settingsSaving){scheduleGeneralSave();return;}
    try {await saveAllSettings({section:'general'});} catch(error){notify(error.message??String(error),'error');}
    finally {if(version===generalSaveVersion)state.generalSavePending=false;}
  },250);
}
function captureSettingsForms() {
  for(const form of document.querySelectorAll('#general-form,#mapping-form')) {
    const values=AieyesUI.formValues(form),existing=settingsForms.get(form.id);
    settingsForms.set(form.id,{values,baseline:existing?.baseline??JSON.parse(form.dataset.draftBaseline||JSON.stringify(values))});
  }
}
function settingsFormsDirty(){return settingsChangeCount()>0;}
function bindSettingsDrafts(){
  for(const form of document.querySelectorAll('#general-form,#mapping-form')) {
    const draft=settingsForms.get(form.id);form.dataset.draftBaseline=JSON.stringify(draft?.baseline??AieyesUI.formValues(form));
    if(draft)AieyesUI.restoreForm(form,draft.values);
    const update=()=>{captureSettingsForms();const d=settingsForms.get(form.id);form.dataset.dirty=String(JSON.stringify(d.values)!==JSON.stringify(d.baseline));const label=form.querySelector('.draft-state');if(label)label.textContent=form.dataset.dirty==='true'?'未保存 · 切页保留草稿':'';updateSettingsSaveBar();};
    const label=document.createElement('p');label.className='draft-state';label.setAttribute('role','status');form.append(label);
    form.addEventListener('input',update);form.addEventListener('change',()=>{update();if(form.id==='general-form')scheduleGeneralSave();});update();
  }
}
function resetSettingsForm(id){const form=document.getElementById(id);if(form){const values=AieyesUI.formValues(form);settingsForms.set(id,{values,baseline:values});form.dataset.draftBaseline=JSON.stringify(values);form.dataset.dirty='false';form.querySelector('.draft-state')?.replaceChildren();}else settingsForms.delete(id);}
function mappingDraft(next) {
  captureSettingsForms();const values=settingsForms.get('mapping-form')?.values??{};
  const from=(values.mappingModel??'').trim(),to=(values.mappingId??'').trim();
  if(Boolean(from)!==Boolean(to)){const field=document.getElementById(from?'field-mappingId':'field-mappingModel');field?.setCustomValidity('请补全映射两端');field?.reportValidity();field?.addEventListener('input',()=>field.setCustomValidity(''),{once:true});throw new Error('请补全日志模型名称和 OpenRouter 模型 ID，输入已保留');}
  if(from&&next.modelMappings?.[from]&&next.modelMappings[from]!==to)throw new Error('该模型已有映射；请先明确移除原映射，再添加新值');
  next.modelMappings??={};if(from)next.modelMappings[from]=to;
  return next;
}
function renderSettings(capture=true) {
  if(capture)captureSettingsForms();const view=rememberView();
  if(state.settingsTab==='codexAuth')state.settingsTab='accounts';
  if(state.settingsTab==='connection')state.settingsTab='general';
  const tabs={sources:'Agents',hosts:'服务器',prices:'价格',accounts:'账户',wakeups:'定时唤醒',general:'通用'};
  let body='';const s=draftSettings();
  if(state.settingsTab==='sources')body=window.AieyesAgentSettings.agentsHTML();
  else if(state.settingsTab==='accounts')body=window.AieyesAgentSettings.accountsHTML();
  else if(state.settingsTab==='hosts'){
    const isSource=state.settingsTab==='sources',list=isSource?s.sources:monitoredHosts(s);
    body=`<div class="card">${list.map(item=>`<div class="list-row"><input type="checkbox" role="switch" data-enable="${escapeHTML(item.id)}" ${item.enabled?'checked':''} aria-label="启用 ${escapeHTML(item.name)}">${isSource?providerMark(item.provider):''}<div class="row-body">${escapeHTML(item.name||item.target)}<small>${escapeHTML(isSource?`${providers[item.provider]} · ${item.accountId ? (s.accounts.find(a=>a.id===item.accountId&&a.provider===item.provider)?.name??item.accountId) : item.codexHomeId ? "共享历史" : "无账户"}`:item.target)}</small>${isSource?sourceError(item.id):''}</div><button data-edit="${escapeHTML(item.id)}" aria-label="编辑 ${escapeHTML(item.name||item.target)}" title="编辑此项">编辑</button><button ${item.id==='local'?'hidden':''} data-remove="${escapeHTML(item.id)}" aria-label="移除 ${escapeHTML(item.name)}" title="移除此项">${uiIcon('minus')}</button></div>`).join('')||'<div class="empty">添加第一个'+(isSource?'数据源':'主机')+'</div>'}${savedFeedback(state.settingsTab)}</div><button id="add-item" class="primary" title="新建数据源或服务器">${uiIcon('plus')} 添加${isSource?'数据源':'主机'}</button>`;
  }else if(state.settingsTab==='prices')body='<div class="between"><p class="muted save-model-note">价格与模型映射保存后生效。</p><button id="reprice" type="button" title="保存设置并按当前价格重新计算历史成本">保存并重算</button></div>'+gapList()+`<form id="mapping-form" class="card"><h2>模型映射</h2>${Object.entries(s.modelMappings).map(([from,to])=>`<div class="list-row tiny"><span>${escapeHTML(from)} → ${escapeHTML(to)}</span><button type="button" data-unmap="${escapeHTML(from)}" aria-label="移除 ${escapeHTML(from)} 的模型映射">${uiIcon('minus')}</button></div>`).join('')}${field('mappingModel','日志模型名称')}${field('mappingId','OpenRouter 模型 ID')}<button>添加映射</button></form>`+`<div class="price-toolbar"><input id="price-search" type="search" aria-label="搜索模型" placeholder="搜索模型" value="${escapeHTML(state.priceSearch)}"><button id="sync-prices" title="从项目 Release 同步 OpenRouter 价格表">同步 OpenRouter</button><button id="add-price" title="手动添加一个模型价格">添加价格</button>${savedFeedback('prices')}</div><div class="card prices-list">${priceRows()}</div><span class="muted tiny">USD / 百万 Token</span>`;
  else if(state.settingsTab==='wakeups')body=window.AieyesWakeups?.html()??'<p>正在加载…</p>';
  else body=(window.AieyesDesktop?.settingsHTML() || '')+`<form id="general-form" class="card"><h2>外观</h2>${select('appearanceTheme','主题',[['system','跟随系统'],['light','浅色'],['dark','深色']],s.appearance?.theme??'system')}${select('appearanceAccent','强调色',[['indigo','靛蓝'],['blue','蓝'],['teal','青绿'],['purple','紫']],s.appearance?.accent??'indigo')}<p class="font-notice">界面字体：HarmonyOS Sans SC（鸿蒙黑体）· © 2021 Huawei Device Co., Ltd. <button type="button" id="font-license">字体许可</button></p><h2>网络与刷新</h2>${proxyFields('app',s.proxy)}${field('proxyTestUrls','测试地址（逗号分隔）',(s.proxyTestUrls??['https://api.github.com/rate_limit','https://openrouter.ai']).join(', '))}<h3>刷新</h3>${field('refreshSeconds','Agent 限额间隔（秒）',s.refreshSeconds,'','number')}${field('historyRefreshSeconds','Agent 记录间隔（秒）',s.historyRefreshSeconds??s.refreshSeconds,'','number')}${field('serverForegroundRefreshSeconds','服务器前台间隔（秒）',s.serverForegroundRefreshSeconds??2,'','number')}${field('serverRefreshSeconds','服务器后台间隔（秒）',s.serverRefreshSeconds,'','number')}<p class="muted tiny">选择项即时保存；文本修改后离开输入框保存。</p></form>${window.AieyesUpdates?.settingsHTML() ?? '<div class="card"><p>正在读取更新信息…</p></div>'}`;
  $('#content').innerHTML=`<div class="settings-save-bar" id="settings-save-bar"><p class="settings-draft-label" role="status"></p></div><div class="settings-tabs" role="tablist" aria-label="设置分类">${Object.entries(tabs).map(([k,v])=>`<button id="settings-tab-${k}" data-settings-tab="${k}" role="tab" aria-controls="settings-panel" aria-selected="${state.settingsTab===k}" tabindex="${state.settingsTab===k?0:-1}" class="${state.settingsTab===k?'active':''}">${uiIcon(k)}<span>${v}</span></button>`).join('')}</div><div id="settings-panel" class="settings-block" role="tabpanel" aria-labelledby="settings-tab-${state.settingsTab}">${body}</div>`;
  const fontLicense=$('#font-license');
  if(fontLicense)fontLicense.onclick=async()=>{
    try{const response=await fetch('fonts/LICENSE.txt');if(!response.ok)throw new Error('字体许可读取失败');const license=await response.text();quotaDialog('HarmonyOS Sans 字体许可',`<pre class="font-license-text">${escapeHTML(license)}</pre>`);}
    catch(error){notify(String(error),'error');}
  };
  bindSettingsDrafts();window.AieyesDesktop?.bindSettings();updateSettingsSaveBar();
  if(state.settingsTab==='wakeups')window.AieyesWakeups?.bind();
  if(['sources','accounts'].includes(state.settingsTab))window.AieyesAgentSettings.bind();
  document.querySelectorAll('[data-settings-tab]').forEach(button=>{
    button.onclick=async()=>{
      state.settingsTab=button.dataset.settingsTab;renderSettings();
      try {if(state.settingsTab==='prices'){state.prices=await api('prices.list');if(state.page==='settings'&&state.settingsTab==='prices')renderSettings();}await loadDashboard();}
      catch(error){notify(String(error),'error');}
    };
    button.onkeydown=event=>{
      const buttons=[...document.querySelectorAll('[data-settings-tab]')],index=buttons.indexOf(button);
      const next=event.key==='Home'?0:event.key==='End'?buttons.length-1:event.key==='ArrowRight'?(index+1)%buttons.length:event.key==='ArrowLeft'?(index+buttons.length-1)%buttons.length:null;
      if(next==null)return;event.preventDefault();buttons[next].focus();buttons[next].click();
    };
  });
  document.querySelectorAll('[data-gap-price]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapPrice)];editPrice(state.prices.find(p=>p.id===(g.priceId??g.model))??{id:g.priceId??g.model,name:g.model});});
  document.querySelectorAll('[data-gap-map]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapMap)];$('#field-mappingModel').value=g.model;$('#field-mappingId').value=g.priceId??'';$('#field-mappingId').focus();});
  const items=state.settingsTab==='sources'?s.sources:monitoredHosts(s);
  document.querySelectorAll('[data-edit]').forEach(b=>b.onclick=()=>editItem(items.find(i=>i.id===b.dataset.edit)));
  document.querySelectorAll('[data-enable]').forEach(b=>b.onchange=async()=>{
    const list=state.settingsTab==='sources'?'sources':'hosts',next=structuredClone(draftSettings());
    if(b.dataset.enable==='local')next.localMonitor={...next.localMonitor,enabled:b.checked};else next[list].find(i=>i.id===b.dataset.enable).enabled=b.checked;b.disabled=true;
    try { await window.AieyesAgentSettings.commit(next); }
    catch(error){notify(String(error),'error');} finally { b.checked=(b.dataset.enable==='local'?draftSettings().localMonitor:draftSettings()[list].find(i=>i.id===b.dataset.enable))?.enabled??false;b.disabled=false; }
  });
  document.querySelectorAll('[data-remove]').forEach(button=>button.onclick=()=>{
    const kind=state.settingsTab,id=button.dataset.remove,item=draftSettings()[kind].find(row=>row.id===id);
    const linked=kind==='hosts'?draftSettings().sources.filter(src=>src.hostId===id):[];
    const impact=linked.length?`<p>保存后，以下 ${linked.length} 个数据源将暂停，并需要重新选择服务器：</p><ul>${linked.map(src=>`<li>${escapeHTML(src.name)}</li>`).join('')}</ul>`:'<p>已导入的用量历史会保留。重新连接时需要再次添加配置。</p>';
    showEditor(kind==='hosts'?'移除服务器':'移除数据源',`<p>确认移除「${escapeHTML(item.name||item.target)}」？</p>${impact}`,async()=>{
      const next=structuredClone(state.settings);next[kind]=next[kind].filter(row=>row.id!==id);
      if(kind==='hosts'){
        for(const src of next.sources)if(src.hostId===id){src.hostId=null;src.enabled=false;}
        for(const agent of next.agents??[])agent.machineIds=agent.machineIds.filter(machine=>machine!==id);
      }
      if(kind==='sources')for(const account of next.accounts)if(account.quotaSourceId===id)account.quotaSourceId=null;
      await window.AieyesAgentSettings.commit(next);renderSettings(false);notify(linked.length?`已移除服务器，${linked.length} 个关联数据源将暂停`:'已移除配置，历史记录将保留');
    });
    const submit=$('#editor-form button[type=submit]');submit.textContent='确认移除';submit.classList.add('danger');
    $('#cancel-editor').focus();
  });
  if($('#add-item'))$('#add-item').onclick=()=>editItem();

  if($('#general-form'))$('#general-form').onsubmit=e=>{e.preventDefault();scheduleGeneralSave();};
  window.AieyesUpdates?.bindSettings();
  if($('#sync-prices'))$('#sync-prices').onclick=syncPrices;
  if($('#add-price'))$('#add-price').onclick=()=>editPrice();
  bindPrices();
  if($('#price-search'))$('#price-search').oninput=e=>{state.priceSearch=e.target.value;$('.prices-list').innerHTML=priceRows();bindPrices();};
  if($('#general-form'))bindProxy('app');
  if($('#mapping-form'))$('#mapping-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);if(!f.get('mappingModel')?.trim()||!f.get('mappingId')?.trim()){notify('请输入日志模型名称和 OpenRouter 模型 ID','error');return;}job('保存模型映射',async()=>{const next=structuredClone(state.settings);mappingDraft(next);await saveSettings(next);$('#mapping-form').reset();resetSettingsForm('mapping-form');renderSettings();});};
  document.querySelectorAll('[data-unmap]').forEach(b=>b.onclick=()=>job('移除模型映射',async()=>{const next=structuredClone(state.settings);delete next.modelMappings[b.dataset.unmap];await saveSettings(next);renderSettings(false);}));
  if($('#reprice'))$('#reprice').onclick=()=>job('保存并重算',async()=>{const button=$('#reprice');state.repricing=true;button.disabled=true;button.textContent='保存并重算中…';try{await saveAllSettings({clearForms:false,refresh:false});await api('prices.recalculate');$('#mapping-form').reset();resetSettingsForm('mapping-form');await loadDashboard();renderSettings();notify('已保存并按当前价格重算');}finally{state.repricing=false;const current=$('#reprice');if(current){current.disabled=false;current.textContent='保存并重算';}}});
  restoreView(view);if(state.repricing&&$('#reprice')){$('#reprice').disabled=true;$('#reprice').textContent='保存并重算中…';}
}
let editorSelectors=[],editorSaving=false,editorOpener=null,editorOpenerSelector=null,editorBaseline='',editorExtraSnapshot=()=>null;
const editorSnapshot=()=>JSON.stringify([AieyesUI.formValues($('#editor-form')),editorExtraSnapshot()]);
function clearEditorSelectors(){editorSelectors.forEach(c=>c.destroy());editorSelectors=[];}
function finishEditor(){
  clearEditorSelectors();$('.shell').inert=false;$('#editor-backdrop').hidden=true;document.body.classList.remove('editor-open');
  if(editorOpener||editorOpenerSelector){const target=editorOpener?.isConnected?editorOpener:editorOpenerSelector?document.querySelector(editorOpenerSelector):null;(target??$('#add-item')??$('#add-price')??$('.settings-tabs .active'))?.focus({preventScroll:true});}
  editorOpener=null;editorOpenerSelector=null;updateSettingsSaveBar();
}
function closeEditor(force=false){
  if(editorSaving)return;
  if(!force&&editorSnapshot()!==editorBaseline){
    let guard=$('#editor-discard');if(!guard){guard=document.createElement('div');guard.id='editor-discard';guard.className='discard-guard';guard.setAttribute('role','alert');guard.innerHTML='<p>此编辑器有未保存内容。</p><button type="button" id="editor-continue">继续编辑</button><button type="button" id="editor-confirm-discard" class="danger">放弃更改</button>';$('#editor-form').append(guard);$('#editor-continue').onclick=()=>{guard.remove();$('#editor-fields input')?.focus();};$('#editor-confirm-discard').onclick=()=>closeEditor(true);}$('#editor-continue').focus();return;
  }
  $('#editor').close();finishEditor();
}
function showEditor(title,html,onSubmit){
  if(editorSaving)return;
  $('#editor-discard')?.remove();editorExtraSnapshot=()=>null;editorBaseline='';editorOpener=document.activeElement;editorOpenerSelector=focusSelector(editorOpener);clearEditorSelectors();
  $('#editor').dataset.kind='';$('#editor-title').textContent=title;$('#editor-fields').innerHTML=html;
  $('#editor-error').hidden=true;$('#editor-error').textContent='';
  $('#editor-form button[type=submit]').textContent='保存';$('#editor-form button[type=submit]').classList.remove('danger');
  $('#editor-form').onsubmit=async e=>{
    e.preventDefault();if(editorSaving)return;
    const data=new FormData(e.target),controls=[...e.target.querySelectorAll('button,input,select,textarea')].map(control=>[control,control.disabled]);
    const submit=$('#editor-form button[type=submit]'),label=submit.textContent;
    editorSaving=true;controls.forEach(([control])=>control.disabled=true);submit.textContent='保存中…';
    $('#editor-error').hidden=true;let saved=false;
    try { await onSubmit(data);saved=true; }
    catch(error){$('#editor-error').textContent=error.message??String(error);$('#editor-error').hidden=false;}
    finally {editorSaving=false;controls.forEach(([control,disabled])=>control.disabled=disabled);submit.textContent=label;}
    if(saved)closeEditor(true);
    else {$('#editor-error').tabIndex=-1;$('#editor-error').focus({preventScroll:true});}
  };
  queueMicrotask(()=>{editorBaseline=editorSnapshot();});
  // Keep native-window controls interactive; only the application page is inert.
  $('.shell').inert=true;$('#editor-backdrop').hidden=false;document.body.classList.add('editor-open');$('#editor').show();
  $('#cancel-editor').focus();
}
document.addEventListener('keydown',event=>{
  if(!$('#editor')||!$('#editor').open||event.defaultPrevented)return;
  if(event.key==='Escape'){event.preventDefault();closeEditor();return;}
  if(event.key!=='Tab')return;
  const controls=[...document.querySelectorAll('#titlebar:not([hidden]) button,#editor button,#editor input,#editor select,#editor textarea,#editor summary,#editor a[href],#editor [tabindex]')].filter(element=>{
    if(element.disabled||element.tabIndex<0||element.closest('[hidden],[inert]'))return false;
    for(let parent=element.parentElement;parent;parent=parent.parentElement)if(parent.tagName==='DETAILS'&&!parent.open&&element!==parent.querySelector(':scope > summary'))return false;
    return element.checkVisibility?element.checkVisibility({checkVisibilityCSS:true}):element.getClientRects().length>0&&getComputedStyle(element).visibility!=='hidden';
  });
  if(!controls.length)return;
  const index=controls.indexOf(document.activeElement),next=index<0?(event.shiftKey?controls.length-1:0):(index+(event.shiftKey?-1:1)+controls.length)%controls.length;
  event.preventDefault();controls[next].focus();
});
function editAccount(index) {
  if(index==null)window.AieyesAgentSettings.addAccount();
  else window.AieyesAgentSettings.openDetails(state.settings.accounts[index]);
}
function proxyParts(url) {
  try {
    const u=new URL(url);
    if(['http:','https:','socks5:','socks5h:'].includes(u.protocol)&&!u.username&&!u.password&&(!u.pathname||u.pathname==='/')&&!u.search&&!u.hash){
      return {protocol:u.protocol.slice(0,-1),host:u.hostname.replace(/^\[|\]$/g,''),port:u.port||(u.protocol==='http:'?'80':u.protocol==='https:'?'443':'1080'),url};
    }
  }catch{}
  return {protocol:url?'url':'http',host:'127.0.0.1',port:'7890',url:url??''};
}
function proxyFields(prefix,proxy,inherit=false) {
  const p=proxyParts(proxy?.url??'');
  return select(prefix+'Mode','连接方式',[...(inherit?[['inherit','跟随应用']]:[]),['system','系统代理'],['direct','直连'],['custom','指定代理']],proxy?.mode??(inherit?'inherit':'system'))+`<div id="${prefix}-proxy-address">${select(prefix+'Protocol','协议',[['http','HTTP'],['https','HTTPS'],['socks5','SOCKS5'],['socks5h','SOCKS5H'],['url','自定义 URL']],p.protocol)}<div id="${prefix}-proxy-basic">${field(prefix+'Host','Host',p.host)}${field(prefix+'Port','端口',p.port,'','number')}</div><div id="${prefix}-proxy-url">${field(prefix+'URL','代理 URL',p.url)}</div></div><div class="actions"><button type="button" id="${prefix}-proxy-test">测试连接</button><span id="${prefix}-proxy-result" role="status"></span></div>`;
}
function bindProxy(prefix) {
  const protocol=$('#field-'+prefix+'Protocol');let previous=protocol.value;
  let revision=0;const clearResult=()=>{revision++;$('#'+prefix+'-proxy-result').textContent='';};
  for(const name of [prefix+'Mode',prefix+'Protocol',prefix+'Host',prefix+'Port',prefix+'URL',...(prefix==='app'?['proxyTestUrls']:[])])$('#field-'+name)?.addEventListener('input',clearResult);
  const update=()=>{
    const active=$('#field-'+prefix+'Mode').value==='custom',custom=protocol.value==='url';
    $('#'+prefix+'-proxy-address').hidden=!active;$('#'+prefix+'-proxy-basic').hidden=custom;$('#'+prefix+'-proxy-url').hidden=!custom;
  };
  $('#field-'+prefix+'Mode').onchange=update;
  protocol.onchange=()=>{
    if(protocol.value==='url'&&previous!=='url'){
      try { const form=protocol.closest('form'),f=new FormData(form);f.set(prefix+'Mode','custom');f.set(prefix+'Protocol',previous);$('#field-'+prefix+'URL').value=proxyValue(f,prefix).url; }catch{}
    }
    previous=protocol.value;update();
  };update();
  $('#'+prefix+'-proxy-test').onclick=async()=>{
    const button=$('#'+prefix+'-proxy-test'),result=$('#'+prefix+'-proxy-result');button.disabled=true;result.textContent='测试中…';
    const testedRevision=revision;
    try{const form=protocol.closest('form'),f=new FormData(form);const proxy=proxyValue(f,prefix)??state.settings.proxy;
      const urls=prefix==='app'?f.get('proxyTestUrls')?.split(',').map(s=>s.trim()).filter(Boolean):state.settings.proxyTestUrls;
      const snapshot=await api('network.test',{proxy,...(urls?{urls}:{}),force:true});if(revision===testedRevision){result.textContent=AieyesNetwork.label(snapshot);result.title=AieyesNetwork.detail(snapshot);}
    }catch(error){if(revision===testedRevision)result.textContent=error.message??String(error);}finally{button.disabled=false;}
  };
}
function proxyValue(f,prefix) {
  const mode=f.get(prefix+'Mode');if(mode==='inherit')return null;
  let url=f.get(prefix+'URL')??'';
  if(mode==='custom'&&f.get(prefix+'Protocol')!=='url'){
    let host=f.get(prefix+'Host').trim(),port=Number(f.get(prefix+'Port'));
    if(!host||/[\s/@?#]/.test(host)||!Number.isInteger(port)||port<1||port>65535)throw new Error('请输入有效 Host 和端口');
    host=host.replace(/^\[|\]$/g,'');if(host.includes(':'))host=`[${host}]`;
    url=`${f.get(prefix+'Protocol')}://${host}:${port}`;
  }
  return {mode,url};
}
function editItem(existing){
  if(state.settingsTab==='sources')return window.AieyesAgentSettings.advanced(existing,existing?.provider||'codex');
  const base=structuredClone(state.settings),item=structuredClone(existing??{id:crypto.randomUUID(),name:'',target:'',authMode:'ssh',username:'',passwordRef:'',port:null,identityFile:'',shell:'/bin/bash',preCommand:'',enabled:true,metrics:Object.keys(groups),devices:[],details:Object.keys(detailOptions)});
  let preparedPassword=null;
  const body=field('name','名称',item.name)+field('target','SSH 别名或地址',item.target,'my-server 或 user@host')+field('port','端口',item.port??'','跟随 SSH 配置','number')+select('authMode','登录方式',[['ssh','SSH 配置 / 密钥'],['password','账号密码']],item.authMode??'ssh')+field('username','用户名',item.username??'','跟随地址或 SSH 配置')+`<div id="host-password">${field('password','密码（留空保留）',pendingHostPasswords.get(item.id)??'','','password')}</div><div id="host-key">${field('identityFile','密钥路径',item.identityFile,'跟随 SSH 配置')}</div>`+`<section class="host-selectors"><div id="metric-select"></div><div class="section-head"><h3>设备</h3><button type="button" id="discover-devices" title="通过 SSH 读取远端设备列表">读取设备</button></div><p id="discovery-status" class="error" role="status" hidden></p><div id="device-selects" class="device-selects"></div><div id="detail-select"></div></section><details class="advanced"><summary>高级设置</summary>${field('shell','远程 shell',item.shell)+field('devices','设备表达式',item.devices.join(', '),'network:eth0, gpu:0, filesystems:/')}</details>`;
  showEditor(item.id==='local'?'本机监测':'SSH 主机',body,async f=>{
    const next=structuredClone(base);
    for(const k of ['name','target','identityFile','shell','authMode','username'])item[k]=f.get(k);
    item.port=f.get('port')?Number(f.get('port')):null;item.devices=f.get('devices').split(',').map(s=>s.trim()).filter(Boolean);if(!item.name)item.name=item.target;
    if(item.authMode==='password'&&f.get('password')){
      if(!preparedPassword||preparedPassword.value!==f.get('password'))preparedPassword={value:f.get('password'),...await api('hosts.credentials.save',{password:f.get('password')})};
      item.passwordRef=preparedPassword.passwordRef;
    }
    if(item.id==='local')next.localMonitor={enabled:item.enabled,metrics:item.metrics,devices:item.devices,details:item.details};else {const index=next.hosts.findIndex(h=>h.id===item.id);if(index<0)next.hosts.push(item);else next.hosts[index]=item;}
    await window.AieyesAgentSettings.commit(next,base);renderSettings(false);
  });
  $('#editor').dataset.kind='host';
  if(item.id==='local')for(const key of ['name','target','port','authMode','username','password','identityFile','shell']){const el=$('#field-'+key);if(el)el.closest('.form-row').hidden=true;}
  const selectors=$('.host-selectors'),advanced=document.createElement('details');advanced.className='monitor-options';advanced.open=Boolean(existing);advanced.innerHTML='<summary>监控指标与设备 · 按需选择</summary>';selectors.before(advanced);advanced.append(selectors);
  const connect=$('#discover-devices');advanced.before(connect);connect.textContent='连接并读取设备';
  bindPathSelection(false);bindHostSelectors(item);
  const auth=()=>{$('#host-password').hidden=$('#field-authMode').value!=='password';$('#host-key').hidden=!$('#host-password').hidden;};$('#field-authMode').onchange=auth;auth();
}
function bindPathSelection(source){
  const input=source?$('#field-path'):$('#field-identityFile');if(!input)return;
  const row=input.closest('.form-row'),controls=document.createElement('div');controls.className='path-actions';controls.innerHTML='<button type="button" class="path-pick">选择…</button><button type="button" class="path-check">检测路径</button><span class="path-status" role="status"></span>';row.append(controls);
  const remote=()=>source&&Boolean($('#field-hostId')?.value);
  controls.querySelector('.path-pick').onclick=async()=>{try{const result=await window.__TAURI__.core.invoke('select_local_path',{directory:source&&$('#field-provider')?.value!=='deepseek',initial:input.value});if(result){input.value=result;input.dispatchEvent(new Event('input',{bubbles:true}));}}catch(e){controls.querySelector('.path-status').textContent=String(e);}};
  controls.querySelector('.path-check').onclick=async()=>{const status=controls.querySelector('.path-status');try{status.textContent=await window.__TAURI__.core.invoke('check_local_path',{path:input.value})?'路径存在':'路径不存在，请检查';}catch(e){status.textContent=String(e);}};
  const update=()=>{controls.querySelectorAll('button').forEach(b=>b.hidden=remote());controls.querySelector('.path-status').textContent=remote()?'远程路径 · '+(draftSettings().hosts.find(h=>h.id===$('#field-hostId').value)?.name??'SSH 主机'):'';};$('#field-hostId')?.addEventListener('change',update);update();
}
function bindHostSelectors(item) {
  const selectors=AieyesSelect;editorExtraSnapshot=()=>[item.metrics,item.details];
  let discovered={}, deviceControls=[],discoveryRevision=0;
  const discoveryIdentity=()=>JSON.stringify(['target','port','identityFile','shell','authMode','username','password'].map(k=>$('#field-'+k)?.value));
  for(const key of ['target','port','identityFile','shell','authMode','username','password'])$('#field-'+key)?.addEventListener('input',()=>{discoveryRevision++;discovered={};redrawDevices();$('#discovery-status').hidden=false;$('#discovery-status').textContent='连接配置已更改，请重新读取设备';});
  const tokens=()=>selectors.parse($('#field-devices').value);
  const detailGroup={cpuTimes:'cpu',memoryCache:'memory',swap:'memory',fsAvailable:'filesystems',fsType:'filesystems',inodes:'filesystems',diskIops:'disk',diskBusy:'disk',networkTotals:'network',networkErrors:'network',gpuMemory:'gpu',gpuThermals:'gpu'};
  item.details ??= Object.keys(detailOptions);
  function redrawDevices() {
    deviceControls.forEach(c=>{c.destroy();editorSelectors.splice(editorSelectors.indexOf(c),1);});
    deviceControls=[]; $('#device-selects').replaceChildren();
    for(const [group,title] of Object.entries(groups).filter(([g])=>g!=='memory')) {
      const root=document.createElement('div'); $('#device-selects').append(root);
      const options=()=>{
        const rows=(discovered[group]??[]).filter(d=>group!=='cpu'||d.id!=='cpu');
        const known=new Set(rows.map(d=>d.id));
        return [...rows.map(d=>({id:d.id,type:d.type,label:[d.id,d.type??d.name].filter(Boolean).join(' · ')})),
          ...selectors.deviceIds(tokens(),group).filter(id=>!known.has(id)).map(id=>({id,label:id,unavailable:true}))];
      };
      const control=selectors.mount(root,{title,options,selected:()=>group==='filesystems'&&!tokens().some(s=>s.startsWith('filesystems:'))?options().filter(o=>selectors.recommendedFilesystem(o.id,o.type)).map(o=>o.id):selectors.deviceSelected(tokens(),group,options().map(o=>o.id)),all:()=>tokens().includes(group+':__all__')||(group!=='filesystems'&&!tokens().some(s=>s.startsWith(group+':'))),disabled:()=>!item.metrics.includes(group),
        onChange:(selected,{all})=>{$('#field-devices').value=selectors.writeDevices(tokens(),group,selected,all).join(', ');}});
      deviceControls.push(control);editorSelectors.push(control);
    }
  }
  const metrics=selectors.mount($('#metric-select'),{title:'采集项目',options:()=>Object.entries(groups).map(([id,label])=>({id,label})),selected:()=>item.metrics,
    onChange:values=>{item.metrics=values;deviceControls.forEach(c=>c.refresh());details.refresh();}});
  // Disabled categories retain their saved detail values; only enabled categories are editable.
  const details=selectors.mount($('#detail-select'),{title:'显示细分项',options:()=>Object.entries(detailOptions).filter(([id])=>id==='uptime'||item.metrics.includes(detailGroup[id])).map(([id,label])=>({id,label})),selected:()=>item.details,
    onChange:values=>{item.details=values;}});
  editorSelectors.push(metrics,details);redrawDevices();
  $('#field-devices').onchange=redrawDevices;
  $('#discover-devices').onclick=async()=>{
    const button=$('#discover-devices'), status=$('#discovery-status');
    button.disabled=true;button.textContent='读取中…';status.hidden=true;
    const form=$('#editor-form'), f=new FormData(form), host={...item},identity=discoveryIdentity(),revision=++discoveryRevision;
    for(const key of ['target','identityFile','shell','authMode','username'])host[key]=f.get(key);
    host.port=f.get('port')?Number(f.get('port')):null;
    let temporaryReference;
    try {
      if(host.authMode==='password'&&f.get('password')){const saved=await api('hosts.credentials.save',{password:f.get('password')});temporaryReference=saved.passwordRef;host.passwordRef=temporaryReference;}
      const sample=await api('hosts.discover',{host});
      if(!$('#editor').open||!button.isConnected||revision!==discoveryRevision||identity!==discoveryIdentity())return;
      for(const group of Object.keys(groups))if(Array.isArray(sample[group]))discovered[group]=sample[group];
      redrawDevices();const options=$('.monitor-options');if(options)options.open=true;
      status.textContent=Object.keys(sample.errors??{}).map(k=>(groups[k]??k)+'读取失败').join(' · ');if(!status.textContent)status.textContent='从 '+host.target+' 读取 · '+timeLabel(Date.now());status.hidden=false;
    }catch(error){if(button.isConnected&&revision===discoveryRevision&&identity===discoveryIdentity()){status.textContent=String(error);status.hidden=false;}}
    finally{if(temporaryReference)await api('hosts.credentials.delete',{passwordRef:temporaryReference}).catch(()=>{});button.disabled=false;button.textContent='读取设备';}
  };
}
function editPrice(existing){const price=structuredClone(existing??{id:'',name:'',input:null,output:null,cacheRead:null,cacheWrite:null,fetchedAt:0});
  showEditor('模型价格 · USD / 百万 Token',field('id','模型 ID',price.id)+field('name','显示名称',price.name)+[['input','输入'],['output','输出'],['cacheRead','缓存读取'],['cacheWrite','缓存写入']].map(([k,label])=>field(k,label,price[k]==null?'':price[k]*1e6,'待定价')).join(''),async f=>{price.id=f.get('id');price.name=f.get('name');for(const k of ['input','output','cacheRead','cacheWrite']){const v=f.get(k);price[k]=v===''?null:Number(v)/1e6;if(price[k]!=null&&(!Number.isFinite(price[k])||price[k]<0))throw new Error('价格需要为非负数');}await api('prices.save',{prices:[price]});state.prices=await api('prices.list');await loadDashboard();renderSettings();markSaved();});
  $('#editor').dataset.kind='price';
}
if(!PANEL){
  const aside=document.querySelector('aside'),toggle=document.createElement('button');toggle.id='sidebar-toggle';toggle.title='收起或展开导航';toggle.setAttribute('aria-label','收起或展开导航');toggle.innerHTML=uiIcon('menu');aside?.prepend(toggle);
  const update=()=>{const saved=localStorage.getItem('sidebar.collapsed'),collapsed=saved==null?innerWidth<(document.documentElement.dataset.platform==='windows'?974:1000):saved==='true';document.body.classList.toggle('sidebar-collapsed',collapsed);toggle.setAttribute('aria-expanded',String(!collapsed));toggle.textContent=collapsed?'»':'«';};toggle.onclick=()=>{localStorage.setItem('sidebar.collapsed',String(!document.body.classList.contains('sidebar-collapsed')));update();};addEventListener('resize',update);update();
}
document.querySelectorAll('[data-page]').forEach(button=>button.onclick=()=>{
  state.page=button.dataset.page;render();
  if(PANEL)window.__TAURI__.core.invoke('desktop_panel_page',{page:state.page}).catch(error=>notify(String(error),'error'));
  if(state.page==='servers')sample();
});
if($('#editor')){
  $('#editor').addEventListener('close',()=>{if(!$('#editor').open)finishEditor();});
  $('#editor').addEventListener('cancel',e=>{e.preventDefault();closeEditor();});
  $('#close-editor').onclick=$('#cancel-editor').onclick=()=>closeEditor();
}
if(!PANEL)bindRefreshMenu('refresh','refresh-menu');
$('#activity').onclick=()=>state.page==='servers'?sample():scan();$('#activity').onkeydown=e=>{if(e.key==='Enter'||e.key===' '){e.preventDefault();state.page==='servers'?sample():scan();}};
document.addEventListener('keydown',e=>{if(!(e.ctrlKey||e.metaKey)||$('#editor')?.open||$('#quota-dialog')?.open)return;if(e.key.toLowerCase()==='s'&&state.page==='settings'&&!PANEL){e.preventDefault();if(settingsFormsDirty())job('保存应用配置',()=>saveAllSettings());return;}if(e.key===','){e.preventDefault();if(PANEL)window.__TAURI__.core.invoke('desktop_action',{action:'settings'});else{state.page='settings';render();}}if(e.shiftKey&&e.key.toLowerCase()==='d'){e.preventDefault();if(PANEL)openDetail();else{state.page='agent';render();}}});
// Error text remains selectable; only the explicit close control dismisses it.
new ResizeObserver(()=>{if(state.page==='agent')drawTrend();}).observe($('#content'));
function syncSharedAppearance(appearance) {
  if(!state.settings||!appearance)return;
  const previous=state.settings.appearance??{theme:'system',accent:'indigo'};
  state.settings.appearance={...appearance};
  if(state.settingsDraft){
    const draft=state.settingsDraft.appearance??{...previous};
    for(const key of ['theme','accent'])if(draft[key]===previous[key])draft[key]=appearance[key];
    state.settingsDraft.appearance=draft;state.settingsBaseline=configKey(state.settings);
  }
  const form=$('#general-form');
  if(form){
    const record=settingsForms.get('general-form'), baseline=JSON.parse(form.dataset.draftBaseline||'{}');
    for(const key of ['theme','accent']){
      const name=key==='theme'?'appearanceTheme':'appearanceAccent',input=form.elements.namedItem(name);
      if(input&&input.value===previous[key]){input.value=appearance[key];baseline[name]=appearance[key];if(record){record.values[name]=appearance[key];record.baseline[name]=appearance[key];}}
    }
    form.dataset.draftBaseline=JSON.stringify(baseline);
  }
}
window.addEventListener('aieyes:appearance',event=>{
  // Adopt a sibling window's saved preference before rendering from local state.
  syncSharedAppearance(event.detail);
  if(state.page==='agent'&&state.dashboard){if(interactionOpen())pendingRender=true;else render();}
});
async function toggleTheme() {
  if(!state.settings||state.themeSaving||state.settingsSaving||state.settingsCommitting)return;
  state.themeSaving=true;window.AieyesTheme.setBusy(true);
  try {
    const base=structuredClone(state.settings),next=structuredClone(base);
    next.appearance={...(base.appearance??{accent:'indigo'}),theme:document.documentElement.dataset.theme==='dark'?'light':'dark'};
    const saved=await api('settings.patch',{base,settings:next});
    syncSharedAppearance(saved.appearance);window.AieyesTheme.apply(saved.appearance);
    if($('#message')?.textContent.startsWith('主题保存失败：'))notify('');
  } catch(error) { notify('主题保存失败：'+String(error),'error'); }
  finally { state.themeSaving=false;window.AieyesTheme.setBusy(false); }
}
document.addEventListener('click',event=>{if(event.target.closest('[data-theme-toggle]'))void toggleTheme();});
let externalRefresh=null,externalPending=false,externalSettings=false,externalQuotaCheck=false,externalScanCheck=false;
function refreshExternalData(settings=false,method='') {
  externalPending=true;externalSettings ||= settings;externalQuotaCheck ||= method==='quotas.refresh';externalScanCheck ||= method==='sources.scan';
  if(externalRefresh)return externalRefresh;
  externalRefresh=(async()=>{
    while(externalPending){
      externalPending=false;const readSettings=externalSettings,checkQuotas=externalQuotaCheck,checkScan=externalScanCheck;externalSettings=false;externalQuotaCheck=false;externalScanCheck=false;
      if(readSettings||!state.settings){const saved=await api('settings.get');syncSharedAppearance(saved.appearance);state.settings=saved;window.AieyesTheme?.apply(saved.appearance);}migrateProviderState();
      await loadDashboard();
      if(readSettings&&PANEL&&state.page!=='agent')render();
      const scannedSources=(state.dashboard?.sources??[]).filter(source=>state.settings.sources.some(row=>row.id===source.id&&row.enabled&&(!row.hostId||state.settings.hosts.some(host=>host.id===row.hostId&&host.enabled)))).map(source=>({id:source.id,error:source.status?.error,partial:source.status?.partial,issues:source.status?.issues}));
      // A filtered snapshot is not an action result. Shared feedback owns busy,
      // success time and targeted failures; invalidation must never replace it.
      for(const [key,check,rows] of [['quotas',checkQuotas,state.dashboard?.quotas],['scan',checkScan,scannedSources]]) {
        if(check&&!state.refreshState[key]){const error=batchError(rows,refreshLabels[key]);if(error)showFailures(error,key);}
      }
      if(checkQuotas)state.quotaError=state.refreshState.quotas?.error?'部分账户读取失败，请逐项重试。':'';
      renderFailures();updateActivity();
    }
  })().catch(error=>notify(String(error),'error')).finally(()=>{externalRefresh=null;});
  return externalRefresh;
}
async function listenForSharedState() {
  const events=window.__TAURI__?.event;if(!events?.listen)return;
  state.sharedEstimateOperations=new Set();
  await events.listen('operations:busy',({payload})=>{
    if(payload.busy)state.sharedEstimateOperations.add(payload.operationId);
    else state.sharedEstimateOperations.delete(payload.operationId);scheduleRefreshes();
  });
  await events.listen('desktop:refresh-status',({payload})=>{
    if(!refreshLabels[payload.key])return;
    const previous=state.refreshState[payload.key]??{};
    if(payload.noAttempt){state.refreshState[payload.key]={...previous,busy:false};updateActivity();return;}
    if(payload.busy)state.refreshState[payload.key]={...previous,busy:true};
    else {if(payload.key==='hosts')state.hostRefreshItem=payload.itemId??null;state.refreshState[payload.key]={...previous,busy:false,success:payload.success??previous.success};showFailures(payload.failures?.length?{failures:payload.failures}:null,payload.key,payload.itemId??null);}
    updateActivity();
  });
  await events.listen('desktop:status',({payload})=>{
    state.panelOpen=Boolean(payload.panelOpen);state.panelPage=payload.panelPage??state.panelPage;scheduleRefreshes();
  });
  await events.listen('desktop:settings',()=>{invalidateQuotaSchedule();window.AieyesNetwork?.invalidate();if(PANEL)return refreshExternalData(true);});
  await events.listen('desktop:hosts',({payload})=>{applyHostSamples(payload);});
  await events.listen('desktop:hosts-error',({payload})=>{applyHostFailure(payload,state.hostRefreshItem);});
  await events.listen('desktop:data-changed',({payload})=>{invalidateQuotaSchedule();
    if(PANEL&&['settings.save','settings.patch','agents.set','sources.configure','sources.remove','accounts.connect','accounts.create','accounts.delete','codexAuth.enable','codexAuth.adopt','codexAuth.login.status','codexAuth.profiles.bind','codexAuth.profiles.remove'].includes(payload))return; // desktop:settings owns this invalidation.
    if(PANEL||(!state.busy&&!state.settingsSaving&&!state.settingsCommitting&&!editorSaving))return refreshExternalData(['settings.save','settings.patch','agents.set','sources.configure','sources.remove','accounts.connect','accounts.create','accounts.delete','codexAuth.enable','codexAuth.adopt','codexAuth.login.status','codexAuth.profiles.bind','codexAuth.profiles.remove'].includes(payload),payload);
  });
}
function migrateProviderState() {
  AieyesUI.migrateProviderPreferences(state.settings);
  if(state.provider==='agy')state.provider='antigravity';
  if(state.accountKey?.startsWith('agy:'))state.accountKey=state.settings.accountAliases?.[state.accountKey]??'antigravity:'+state.accountKey.slice(4);
}
async function boot(){
  try{
    if(!window.__TAURI__)throw new Error('通过 Aieyes 桌面应用打开');
    await listenForSharedState();
    const info=await api('hello'),version=$('#app-version');if(version)version.textContent=info.version;
    state.settings=await api('settings.get');migrateProviderState();await loadDashboard();
    if(PANEL){render();updateActivity();requestAnimationFrame(()=>{try{$('.panel-body').scrollTop=Number(localStorage.getItem('aieyes.panel.scroll')||0);}catch(_){}});return;}await scan();if(hasQuotaSources())await quotas(undefined,true);
  }catch(error){notify(String(error),'error');$('#activity').textContent='连接已断开';$('#activity').dataset.status='error';}
}
window.AieyesApp={
  async openSetupInMain(kind){if(!state.settings)await appReady;await openSetup(['hosts','quota'].includes(kind)?kind:'sources');},
  async refreshPanel(){await appReady;await refreshExternalData(true);if(state.page==='servers'&&!state.hosts.length)await sample();},
  applyHostSamples,
  openPanelAccounts,
};
// The panel refreshes on open and on manual action; periodic polling stays in the details window.
const resetConfirmations=new Set();
let refreshTimer,quotaScheduleRead=null,quotaScheduleRevision=0,quotaScheduleKnown=false,quotaDueAt=null,refreshStopped=false;
function invalidateQuotaSchedule() {
  quotaScheduleRevision++;quotaScheduleKnown=false;scheduleRefreshes();
}
async function readQuotaSchedule() {
  if(quotaScheduleRead||PANEL||refreshStopped||!state.settings)return;
  const revision=quotaScheduleRevision;
  quotaScheduleRead=(async()=>{
    try {
      const schedule=await api('quotas.schedule');
      if(revision===quotaScheduleRevision){quotaDueAt=Number.isFinite(schedule?.nextDueAt)?Math.max(Date.now()+1000,schedule.nextDueAt*1000):null;quotaScheduleKnown=true;}
    }catch(_){if(revision===quotaScheduleRevision){quotaDueAt=Date.now()+30000;quotaScheduleKnown=true;}}
  })();
  try{await quotaScheduleRead;}finally{quotaScheduleRead=null;scheduleRefreshes();}
}
function foregroundMetrics(){return (state.page==='servers'&&!document.hidden)||(state.panelOpen&&state.panelPage==='servers');}
function metricsDeadline(){return state.lastMetrics+(foregroundMetrics()?(state.settings.serverForegroundRefreshSeconds??2):state.settings.serverRefreshSeconds)*1000;}
function collectionBlocked(){return state.settingsCommitting||window.AieyesUpdates?.busy||$('#editor')?.open||state.estimateBusy||state.sharedEstimateOperations?.size||$('#quota-dialog')?.dataset.busy||state.busy||state.quotaBusy||state.settingsSaving||['scan','quotas','prices'].some(key=>state.refreshState[key]?.busy);}
function quotaResets(){return (state.dashboard?.quotas??[]).flatMap(q=>q.windows.map(w=>({q,w,key:q.provider+':'+q.accountId+':'+w.resetsAt}))).filter(({w,key})=>w.resetsAt&&!resetConfirmations.has(key));}
function scheduleRefreshes() {
  clearTimeout(refreshTimer);
  if(PANEL||refreshStopped||!state.settings||window.AieyesUpdates?.busy)return;
  const now=Date.now();
  for(const key of ['lastScan','lastMetrics','lastQuota'])if(state[key]>now)state[key]=0;
  if(!quotaScheduleKnown)void readQuotaSchedule();
  const deadlines=[];
  if(!state.serverBusy&&!state.refreshState.hosts?.busy&&monitoredHosts().some(h=>h.enabled))deadlines.push(metricsDeadline());
  if(!collectionBlocked()){
    deadlines.push(state.lastScan+(state.settings.historyRefreshSeconds??state.settings.refreshSeconds)*1000);
    if(quotaScheduleKnown&&quotaDueAt!=null)deadlines.push(quotaDueAt);
    deadlines.push(...quotaResets().map(({w})=>w.resetsAt*1000));
  }
  const next=Math.min(...deadlines.filter(Number.isFinite));
  if(Number.isFinite(next))refreshTimer=setTimeout(runScheduledRefreshes,Math.max(1,next-Date.now()));
}
function runScheduledRefreshes() {
  if(PANEL||refreshStopped||!state.settings||window.AieyesUpdates?.busy)return;
  if(!state.serverBusy&&!state.refreshState.hosts?.busy&&monitoredHosts().some(h=>h.enabled)&&Date.now()>=metricsDeadline())void sample();
  if(!collectionBlocked()){
    const expired=quotaResets().find(({w})=>w.resetsAt*1000<=Date.now());
    if(expired){resetConfirmations.add(expired.key);void quotas(expired.q.accountId);}
    else if(Date.now()>=state.lastScan+(state.settings.historyRefreshSeconds??state.settings.refreshSeconds)*1000)void scan();
    else if(quotaScheduleKnown&&quotaDueAt!=null&&Date.now()>=quotaDueAt){quotaDueAt=null;void quotas(undefined,true);}
  }
  scheduleRefreshes();
}
document.addEventListener('visibilitychange',()=>{if(!document.hidden)invalidateQuotaSchedule();else scheduleRefreshes();});
window.addEventListener('focus',invalidateQuotaSchedule);
document.addEventListener('close',scheduleRefreshes,true);
window.addEventListener('beforeunload',()=>{refreshStopped=true;clearTimeout(refreshTimer);clearTimeout(dashboardTimer);});
// Age visible samples even while an RPC is slow or the panel receives no new events.
setInterval(updateHostStatuses,1000);
function updateResetDisplays(now=Date.now()/1000) { for(const el of document.querySelectorAll('[data-reset-at]'))el.textContent=AieyesUI.quotaReset({resetsAt:Number(el.dataset.resetAt),windowMinutes:Number(el.dataset.windowMinutes),usedPercent:el.dataset.usedPercent?.trim()?Number(el.dataset.usedPercent):undefined},now); }
setInterval(()=>{updateResetDisplays();updateActivity();},30000);
function estimateValues(e){
  const rows=[['本段 5h API 等价值',e.fiveHourValue],['7d 推算 · 同期消耗比例',e.weeklyDirectValue],['7d 推算 · 近期容量倍率',e.weeklyRatioValue]];
  const capacity=e.capacity,old=capacity?.updatedAt&&Date.now()/1000-capacity.updatedAt>14*86400;
  return rows.map(([label,value])=>`<div class="between"><span>${label}</span><strong>${value==null?'—':money(value)+' USD'}</strong></div>`).join('')+`<p class="tiny muted">${capacity?.ratio!=null?'容量倍率 '+capacity.ratio.toFixed(2)+' · '+(old?'历史倍率':capacity.weeklyPercent<5?'倍率样本较少':'近期倍率')+' · '+capacity.samples+' 个样本 · '+date(capacity.updatedAt):'倍率学习中'}</p>`;
}
function estimateStatus(e){return e.status==='active'?(e.fiveHourValue!=null||e.weeklyValue!=null||e.valuePer1000!=null?'可输出估值 · 采样继续':'正在积累样本'):{pending:'需要确认',completed:'已结束'}[e.status]??e.status;}
function quotaDialog(title,body){
  let dialog=$('#quota-dialog');if(!dialog){dialog=document.createElement('dialog');dialog.id='quota-dialog';dialog.setAttribute('aria-labelledby','quota-dialog-title');document.body.append(dialog);}
  dialog.innerHTML=`<div class="dialog-head"><h2 id="quota-dialog-title">${escapeHTML(title)}</h2><button type="button" id="quota-close" aria-label="关闭">${uiIcon('x')}</button></div><div class="quota-dialog-body">${body}<p id="quota-tool-error" class="error" role="alert" hidden></p></div>`;
  delete dialog.dataset.estimateKey;delete dialog.dataset.records;
  delete dialog.dataset.kind;
  $('#quota-close').onclick=()=>dialog.close();if(!dialog.open){dialog.dataset.opener=focusSelector(document.activeElement)??'';dialog.onclose=()=>{flushRender();if(dialog.dataset.opener){const opener=document.querySelector(dialog.dataset.opener);(opener?.getClientRects().length?opener:$('#panel-more > summary'))?.focus({preventScroll:true});}};dialog.showModal();}return dialog;
}
async function quotaAction(action,params){
  if(state.estimateBusy)return false;
  const dialog=$('#quota-dialog'),controls=[...dialog.querySelectorAll('button:not(#quota-close),input,select')].map(el=>[el,el.disabled]);
  dialog.dataset.busy='true';state.estimateBusy=true;controls.forEach(([el])=>el.disabled=true);dialog.oncancel=null;$('#quota-tool-error').hidden=true;
  const operationId=crypto.randomUUID(),started=Date.now();
  state.estimateOperation={key:dialog.dataset.estimateKey,kind:dialog.dataset.estimateKind,group:dialog.dataset.estimateGroup,status:'running'};
  let background=$('#background-task');if(!background){background=document.createElement('button');background.id='background-task';background.setAttribute('role','status');document.body.append(background);}background.hidden=false;background.onclick=()=>{if(state.estimateBusy||state.estimateOperation?.status==='error'){if(!dialog.open)dialog.showModal();}else if(state.estimateOperation?.key){openQuotaEstimate(state.estimateOperation.key,state.estimateOperation.kind,state.estimateOperation.group);background.hidden=true;}else{openSampling();background.hidden=true;}};
  let progress=dialog.querySelector('#quota-progress');if(!progress){progress=document.createElement('p');progress.id='quota-progress';progress.setAttribute('role','status');dialog.querySelector('.quota-dialog-body').append(progress);}
  let stage='等待当前刷新';const update=()=>{progress.textContent=stage+' · '+Math.floor((Date.now()-started)/1000)+' 秒 · 可关闭窗口，后台继续';background.textContent='后台处理中 · '+stage+' · 查看';};update();const timer=setInterval(update,1000);
  let unlisten=()=>{};
  try{unlisten=await window.__TAURI__.event.listen('operations:progress',({payload})=>{if(payload.operationId===operationId){stage=payload.stage;update();}});await api(action,{...params,operationId});stage='读取计算结果';update();await loadDashboard();state.estimateOperation.status='success';background.textContent='后台操作已完成 · 查看结果';return true;}
  catch(error){state.estimateOperation.status='error';background.textContent='后台操作失败 · 查看并重试';$('#quota-tool-error').textContent=error.message??String(error);$('#quota-tool-error').hidden=false;await loadDashboard().catch(()=>{});return false;}
  finally{clearInterval(timer);unlisten();progress.remove();controls.forEach(([el,disabled])=>el.disabled=disabled);dialog.oncancel=null;delete dialog.dataset.busy;state.estimateBusy=false;scheduleRefreshes();}
}
function refreshQuotaDialog(){
  const dialog=$('#quota-dialog'),key=dialog?.dataset.estimateKey;
  if(!dialog?.open||!key||dialog.dataset.busy)return;
  const kind=dialog.dataset.estimateKind??'weekly',group=dialog.dataset.estimateGroup??'';
  const records=JSON.stringify((state.dashboard[kind==='credits'?'creditEstimates':'quotaEstimates']??[]).filter(e=>e.accountKey===key));
  if(records===dialog.dataset.records)return;
  const expanded=dialog.querySelector('details')?.open,focus=document.activeElement?.id;
  openQuotaEstimate(key,kind,group);
  if(expanded&&dialog.querySelector('details'))dialog.querySelector('details').open=true;
  if(focus)document.getElementById(focus)?.focus({preventScroll:true});
}
function hasActiveFilters(){return Boolean(state.provider||state.accountKey||state.sourceId||state.model||state.days!==1);}
function bindQuotaTools(){
  renderQueryStatus();
  $('#content > .active-filters')?.remove();$('#content > .panel-filter-summary')?.remove();
  if(hasActiveFilters()) {
    const filters=document.createElement('div');filters.className='active-filters';
    for(const [key,label] of [['provider',providers[state.provider]],['accountKey',state.accountKey==='none'?'未关联账户':historicalAccounts().find(a=>a.provider+':'+a.id===state.accountKey)?.name??state.accountKey],['sourceId',state.settings.sources.find(s=>s.id===state.sourceId)?.name??state.sourceId],['model',state.model]]) {
      if(!state[key])continue;
      const button=document.createElement('button');button.innerHTML=escapeHTML(label)+uiIcon('x');button.setAttribute('aria-label','清除筛选：'+label);
      button.onclick=()=>{state[key]='';if(key==='provider'){state.accountKey='';state.sourceId='';state.model='';}if(key==='accountKey')state.sourceId='';changeFilters();};filters.append(button);
    }
    if(state.days!==1){const range=document.createElement('button');range.textContent='最近 '+state.days+' 天';range.setAttribute('aria-label','清除时间范围');range.onclick=()=>{state.days=1;changeFilters();};filters.append(range);}
    const clear=document.createElement('button');clear.textContent='清除全部';clear.onclick=()=>{state.provider=state.accountKey=state.sourceId=state.model='';state.days=1;changeFilters();};filters.append(clear);
    if(PANEL){const disclosure=document.createElement('details');disclosure.className='panel-filter-summary';disclosure.innerHTML='<summary>已筛选 '+(filters.children.length-1)+' 项</summary>';disclosure.append(filters);$('#content .filters')?.after(disclosure);}else $('#content .filters')?.after(filters);
  }
  document.querySelectorAll('.quota-disclosure').forEach(detail=>{
    detail.querySelector('summary').onclick=event=>{
      event.preventDefault();if(detail.dataset.collapsible!=='true')return;detail.open=!detail.open;
      try{localStorage.setItem('quota.expanded.v2.'+(PANEL?'panel.':'detail.')+detail.closest('[data-quota]').dataset.quota,String(detail.open));}catch(_){}
    };
  });
  document.querySelectorAll('[data-manage-sampling]').forEach(button=>button.onclick=openSampling);
  if($('#order-quotas'))$('#order-quotas').onclick=openQuotaOrder;
  document.querySelectorAll('[data-credit-estimate]').forEach(b=>b.onclick=()=>openQuotaEstimate(b.dataset.creditEstimate,'credits'));
  document.querySelectorAll('[data-estimate]').forEach(b=>b.onclick=()=>openQuotaEstimate(b.dataset.estimate,"weekly",b.dataset.estimateGroup));
}
function openPanelAccounts(){
  if(!PANEL||state.estimateBusy)return;
  const preference=AieyesUI.panelPreference(), selected=AieyesUI.panelAccounts(state.settings,state.dashboard.quotaOrder??[],preference),accounts=state.settings.accounts.filter(a=>!a.archived&&a.quotaEnabled);
  const groups=[...new Set(accounts.map(a=>a.provider))];
  quotaDialog('面板显示账户','<p class="muted">每个 Agent 可选择 0～5 个账户。仅影响本机面板，详情与采集保留全部账户。</p>'+groups.map(provider=>'<fieldset class="panel-account-group" data-account-provider="'+escapeHTML(provider)+'"><legend>'+providerIdentity(provider)+' <span class="selected-count"></span><button type="button" data-panel-default="'+escapeHTML(provider)+'">恢复默认显示</button></legend>'+accounts.filter(a=>a.provider===provider).map(a=>'<label><input type="checkbox" data-panel-account="'+escapeHTML(a.provider+':'+a.id)+'" '+(selected[provider]?.includes(a.provider+':'+a.id)?'checked':'')+'>'+escapeHTML(a.name)+'</label>').join('')+'</fieldset>').join('')+(accounts.length?'':'<p>尚无可显示限额的账户</p>')+'<div class="dialog-foot"><button type="button" id="panel-accounts-cancel">取消</button><button type="button" class="primary" id="panel-accounts-save">保存面板选择</button></div>');
  $('#quota-dialog').dataset.opener='#panel-more > summary';
  const update=()=>{for(const group of document.querySelectorAll('[data-account-provider]')){const inputs=[...group.querySelectorAll('input')],count=inputs.filter(i=>i.checked).length;group.querySelector('.selected-count').textContent=count+'/5';for(const input of inputs)input.disabled=!input.checked&&count>=5;}};
  document.querySelectorAll('[data-panel-account]').forEach(input=>input.onchange=()=>{const provider=input.closest('[data-account-provider]').dataset.accountProvider;preference.providers[provider]={mode:'custom',keys:[]};update();});update();
  document.querySelectorAll('[data-panel-default]').forEach(button=>button.onclick=()=>{const provider=button.dataset.panelDefault;preference.providers[provider]={mode:'auto',keys:[]};const defaults=AieyesUI.panelAccounts(state.settings,state.dashboard.quotaOrder??[],preference)[provider];button.closest('fieldset').querySelectorAll('input').forEach(input=>input.checked=defaults.includes(input.dataset.panelAccount));update();});
  $('#panel-accounts-cancel').onclick=()=>$('#quota-dialog').close();
  $('#panel-accounts-save').onclick=()=>{for(const group of document.querySelectorAll('[data-account-provider]')){const provider=group.dataset.accountProvider;if(preference.providers[provider]?.mode==='custom')preference.providers[provider].keys=[...group.querySelectorAll('input:checked')].map(i=>i.dataset.panelAccount).slice(0,5);}try{localStorage.setItem('aieyes.panel.accounts.v2',JSON.stringify(preference));}catch(error){$('#quota-tool-error').textContent='无法保存本机选择：'+String(error);$('#quota-tool-error').hidden=false;return;}$('#quota-dialog').close();render();};
  $('#panel-accounts-cancel').focus();
}
function openQuotaOrder(){
  if(state.estimateBusy){notify("后台采样操作仍在进行，可从任务入口返回");return;}
  let keys=[...(state.dashboard.quotaOrder??state.settings.accounts.map(a=>a.provider+':'+a.id))],dragged=null;
  quotaDialog('调整账户顺序','<p class="muted">拖动账户，或使用上下按钮。顺序应用于所有限额面板。</p><div id="quota-order-list"></div><button class="primary" id="quota-order-save">保存顺序</button>');
  function draw(focus){
    const positions=new Map([...document.querySelectorAll('[data-order]')].map(row=>[row.dataset.order,row.getBoundingClientRect().top]));
    $('#quota-order-list').innerHTML=keys.map((key,i)=>`<div class="quota-order-row" draggable="true" data-order="${escapeHTML(key)}"><span aria-hidden="true">${uiIcon('grip')}</span><span class="order-account">${providerMark(key.split(':')[0])}${escapeHTML(historicalAccounts().find(a=>a.provider+':'+a.id===key)?.name??key)}</span><button type="button" data-direction="-1" aria-label="上移账户" ${i===0?'disabled':''}>${uiIcon('up')}</button><button type="button" data-direction="1" aria-label="下移账户" ${i===keys.length-1?'disabled':''}>${uiIcon('down')}</button></div>`).join('');
    for(const row of document.querySelectorAll('[data-order]')){
      row.ondragstart=e=>{document.querySelectorAll('[data-order]').forEach(el=>el.getAnimations().forEach(animation=>animation.cancel()));dragged=row.dataset.order;e.dataTransfer.effectAllowed='move';e.dataTransfer.setData('text/plain',dragged);};row.ondragend=()=>{dragged=null;document.querySelectorAll('.drop-target').forEach(el=>el.classList.remove('drop-target'));};row.ondragover=e=>{e.preventDefault();row.classList.add('drop-target');};row.ondragleave=()=>row.classList.remove('drop-target');
      row.ondrop=e=>{e.preventDefault();if(!dragged||dragged===row.dataset.order)return;const to=keys.indexOf(row.dataset.order);keys.splice(keys.indexOf(dragged),1);keys.splice(to,0,dragged);dragged=null;requestAnimationFrame(()=>draw());};
      row.querySelectorAll('button').forEach(b=>b.onclick=()=>{const i=keys.indexOf(row.dataset.order),j=i+Number(b.dataset.direction);[keys[i],keys[j]]=[keys[j],keys[i]];draw({key:row.dataset.order,direction:b.dataset.direction});});
    }
    if(focus&&!matchMedia('(prefers-reduced-motion: reduce)').matches)for(const row of document.querySelectorAll('[data-order]')){const old=positions.get(row.dataset.order);if(old!=null)row.animate([{transform:`translateY(${old-row.getBoundingClientRect().top}px)`},{transform:'translateY(0)'}],{duration:220,easing:'cubic-bezier(.2,.8,.2,1)'});}
    if(focus){const row=[...document.querySelectorAll('[data-order]')].find(r=>r.dataset.order===focus.key);(row?.querySelector(`button[data-direction="${focus.direction}"]:not(:disabled)`)??row?.querySelector('button:not(:disabled)'))?.focus();}
  }
  draw();$('#quota-order-save').onclick=async()=>{if(await quotaAction('quotas.order.set',{keys})){$('#quota-dialog').close();flushRender();}};
}
function openQuotaEstimate(key,kind="weekly",group=""){
  if(state.estimateBusy){const dialog=$('#quota-dialog');if(dialog&&!dialog.open)dialog.showModal();return;}
  const credits=kind==='credits';
  const q=state.dashboard.quotas.find(q=>q.provider+':'+q.accountId===key);if(!q)return;
  const records=(state.dashboard[credits?'creditEstimates':'quotaEstimates']??[]).filter(e=>e.accountKey===key),current=records.find(e=>e.status!=='completed');
  const windows=AieyesUI.estimateWindows(q),eligible=credits?(q.provider==='codex'&&q.credits?.balance!=null&&!q.credits.unlimited):windows.some(w=>!group||(w.groupId||w.groupName)===group);
  const history=records.filter(e=>e.status==='completed'&&(!group||!e.groupId||e.groupId===group));
  const sources=state.settings.sources.filter(s=>s.enabled&&s.provider===q.provider&&s.accountId===q.accountId&&s.provider!=='deepseek');
  const confirmation='<label class="quota-confirm"><input type="checkbox" id="estimate-confirm">'+(credits?'我确认本段仅消耗 credits，所有设备的用量均已纳入；不混用包含额度、API 或中转。':'我确认采样期间只使用目标订阅，所有设备用量均已纳入所选来源；不混用 API / 中转。')+'</label>';
  const result=e=>`<div class="estimate-result">${e.groupId?`<strong>${escapeHTML(q.windows.find(w=>(w.groupId||w.groupName)===e.groupId)?.groupName||e.groupId)}</strong>`:['antigravity','agy'].includes(q.provider)?'<strong>账户总体 · 旧记录</strong>':''}<div class="between"><strong>${estimateStatus(e)}</strong><strong>${credits?`${creditValue(e)}`:e.valuationMode==='fiveHour'?'5h / 7d':e.weeklyValue!=null?money(e.weeklyValue)+' USD':'—'}</strong></div>${e.valuationMode==='fiveHour'?estimateValues(e):''}<p class="muted">${escapeHTML(e.calculationNote)}</p>${e.originalEstimateId?'<p class="muted">已修正 · 原采样记录保留</p>':e.status==='completed'&&!AieyesUI.estimateEntries([e],credits).length?`<button type="button" data-estimate-repair="${escapeHTML(e.id)}">按原记录修复估值</button>`:''}<details><summary>计算依据</summary><p>${date(e.startedAt)} → ${date(e.checkpointAt)}<br>${escapeHTML(e.windowName)} · 消耗 ${credits?creditAmount(e.consumedCredits)+' credit':pct(e.consumedPercent)} · 样本成本 ${money(e.cost)} USD<br>计价 Token：${compact(e.pricedTokens)} / ${compact(e.totalTokens)}<br>${escapeHTML(e.sourceNames.join('、'))}</p><p class="tiny muted">${credits?'1000 credit 的 API 等价价值 = 样本成本 × 1000 ÷ 消耗 credit':e.valuationMode==='fiveHour'?'5h / 7d 同期价值 = 有效成本 × 100 ÷ 对应消耗百分点；7d 倍率估算 = 5h 价值 × 近期容量倍率':'整周估值 = 样本 API 等价成本 × 100 ÷ 消耗百分点'}；结束后价格与倍率固定。</p>${(e.segments??[]).length?`<p class="tiny muted">已保存 ${e.segments.length} 个有效片段</p>`:''}${e.reason?`<p class="error">${escapeHTML(e.reason)}</p>`:''}${(e.prices??[]).length?`<p class="tiny muted">价格快照：${e.prices.map(p=>escapeHTML(p.id)+' · '+date(p.fetchedAt)).join('；')}</p>`:''}</details></div>`;
  const setup=eligible&&sources.length?`${credits?'':`<label>主要额度池<select id="estimate-window">${windows.map(w=>option(w.id||w.name,w.name,windows.find(x=>(x.groupId||x.groupName)===group)?.id??'')).join('')}</select></label>`}<p>纳入的用量来源</p>${sources.map(s=>`<label class="quota-confirm"><input type="checkbox" name="estimate-source" value="${escapeHTML(s.id)}" checked>${escapeHTML(s.name)}</label>`).join('')}${confirmation}<p class="tiny muted">建议开始后新建会话。至少消耗 ${credits?'5 credits':'5 个百分点'}后输出估值；跨采样边界的累计用量会使本次结果不可用。</p><button class="primary" id="estimate-start" disabled>开始采样</button>`:'<p class="muted">需要可采集 Token 的关联数据源及可靠的额度池映射。请检查数据目录、账户关联和模型映射。</p>';
  quotaDialog(q.name+(credits?' · credit 价值':' · 5h / 7d 额度价值'),`<p class="muted">按本次模型组合的 API 等价成本与额度消耗比例估算，不是可兑换余额。</p>${current?(current.groupId&&group&&current.groupId!==group?'<p class="muted">当前账户正在采样其他模型组；每个账户同时保留一段采样。</p>':'')+result(current)+(current.status==='pending'?confirmation+'<button id="estimate-restart" disabled>确认并开始新一段</button>':'')+`<button class="primary" id="estimate-stop">${current.status==='pending'?'结束并保留有效段':'结束并保存本段结果'}</button>`:setup}${history.length?'<h3>采样历史</h3>'+history.map(result).join(''):''}`);
  $('#quota-dialog').dataset.estimateGroup=group;$('#quota-dialog').dataset.estimateKind=kind;$('#quota-dialog').dataset.estimateKey=key;$('#quota-dialog').dataset.records=JSON.stringify(records);
  if($('#estimate-window'))$('#estimate-window').onchange=()=>{group=windows.find(w=>(w.id||w.name)===$('#estimate-window').value)?.groupId??'';$('#quota-dialog').dataset.estimateGroup=group;};
  const confirm=$('#estimate-confirm');if(confirm)confirm.onchange=()=>{const b=$('#estimate-start')??$('#estimate-restart');if(b)b.disabled=!confirm.checked;};
  const act=async(action,params)=>{if(await quotaAction((credits?'creditEstimates.':'quotaEstimates.')+action,params)){if($('#quota-dialog').open)openQuotaEstimate(key,kind,group);}else {const message=$('#quota-tool-error').textContent;refreshQuotaDialog();$('#quota-tool-error').textContent=message;$('#quota-tool-error').hidden=false;}};
  document.querySelectorAll('[data-estimate-repair]').forEach(button=>button.onclick=()=>act('repair',{id:button.dataset.estimateRepair}));
  if($('#estimate-start'))$('#estimate-start').onclick=()=>act('start',{accountKey:key,windowId:credits?'credits':$('#estimate-window').value,sourceIds:[...document.querySelectorAll('[name=estimate-source]:checked')].map(el=>el.value),confirmed:confirm.checked});
  if($('#estimate-stop'))$('#estimate-stop').onclick=()=>act('stop',{id:current.id});
  if($('#estimate-restart'))$('#estimate-restart').onclick=()=>act('restart',{id:current.id,confirmed:confirm.checked});
}

const appReady=boot();
window.AieyesApp.ready=appReady;

window.addEventListener('aieyes:fonts',()=>requestAnimationFrame(()=>drawTrend()));
