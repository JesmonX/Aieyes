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
const bytes = n => { if (n == null) return '—'; let i = 0; while (n >= 1024 && i < 4) { n /= 1024; i++; } return n.toFixed(i ? 1 : 0) + [' B',' KiB',' MiB',' GiB',' TiB'][i]; };
const speed = n => n == null ? '—' : bytes(n) + '/s';
const date = n => n ? new Date(n * 1000).toLocaleString('zh-CN', {month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit',hourCycle:'h23'}) : '—';
const providers = {codex:'Codex',claude:'Claude Code',antigravity:'Antigravity',agy:'agy',deepseek:'DeepSeek',custom:'自定义'};
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
const detailOptions={cpuTimes:'CPU 时间分布',memoryCache:'内存缓存 / Buffer',swap:'Swap',fsAvailable:'文件系统可用空间',fsType:'文件系统类型 / 设备',inodes:'inode',diskIops:'磁盘 IOPS',diskBusy:'磁盘忙碌率',networkTotals:'累计流量',networkErrors:'网络错误 / 丢包',gpuMemory:'GPU 显存',gpuThermals:'GPU 温度 / 功耗'};
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
async function api(method, params = {}) { return window.__TAURI__.core.invoke('engine_call', {method,params}); }
let notificationTimer;
function notify(message, kind = 'info') {
  clearTimeout(notificationTimer);const el=$('#message');el.hidden=!message;el.dataset.kind=kind;
  el.innerHTML=message?'<span class="message-text">'+escapeHTML(message)+'</span><button type="button" class="message-close" aria-label="关闭提示">'+uiIcon('x')+'</button>':'';
  el.querySelector('.message-close')?.addEventListener('click',()=>notify(''));
  if(message&&kind==='info')notificationTimer=setTimeout(()=>notify(''),5000);
}
const refreshLabels={scan:'同步记录',quotas:'刷新限额',prices:'同步价格',hosts:'刷新服务器'};
function updateActivity() {
  const el=$('#activity'),fresh=AieyesUI.freshness(state.settings,state.dashboard),data=fresh.timestamp;
  const enabledHosts=(state.settings?.hosts??[]).filter(h=>h.enabled),updatedHosts=enabledHosts.filter(h=>hostStatus(h,state.hosts.find(r=>r.id===h.id))==='正常').length;
  const running=Object.entries(state.refreshState).filter(([,v])=>v.busy).map(([k])=>refreshLabels[k]);
  const failed=state.saveError||fresh.failed||Object.values(state.refreshState).some(v=>v.error)||state.hosts.some(h=>h.error);
  const engineBusy=['scan','quotas','prices'].some(key=>state.refreshState[key]?.busy);
  const status=state.busy||running.length?'busy':failed?'error':fresh.delayed?'warning':data?'success':'info';
  el.dataset.status=status;
  el.textContent=(state.page==='servers'?updatedHosts+'/'+enabledHosts.length+' 台已更新':data?'记录同步于 '+timeLabel(data*1000):fresh.total?'记录尚未全部同步':'未接入用量来源')+' · '+(running.length?running.join('、')+'中…':state.busy?'保存中…':failed?'部分失败':state.page==='servers'?(updatedHosts<enabledHosts.length?'存在延迟或未采样':'采样已更新'):fresh.delayed?fresh.delayed+' 个来源有延迟':fresh.total?'记录已同步':'等待接入');
  el.title=state.page==='servers'?'按每台已启用主机的状态统计；点击刷新服务器':'各来源最后成功同步时间（取最早）；查询生成于 '+date(state.dashboard?.generatedAt)+'；点击同步记录';el.tabIndex=0;el.setAttribute('role','button');el.setAttribute('aria-label',el.textContent+'；'+el.title);window.AieyesNetwork?.draw();
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
function renderFailures() {
  const rows=Object.entries(state.refreshState).flatMap(([key,r])=>(r.failures??[]).map(row=>({...row,key})));
  const el=$('#message');if(!rows.length){if(el.dataset.kind==='error')notify('');return;}
  clearTimeout(notificationTimer);
  el.hidden=false;el.dataset.kind='error';
  el.innerHTML='<button type="button" class="message-close" aria-label="关闭提示">'+uiIcon('x')+'</button><div class="error-list">'+rows.map((row,i)=>'<div class="error-row"><div><strong>'+escapeHTML(row.name||state.settings?.sources.find(s=>s.id===row.id)?.name||row.id||row.accountId||'数据源')+'</strong><p>'+escapeHTML(row.error)+'</p></div><button type="button" data-retry="'+row.key+'" data-failure="'+i+'" aria-label="重试 '+escapeHTML(row.name||row.id||row.accountId||refreshLabels[row.key])+'">重试</button></div>').join('')+'</div>';
  el.querySelector('.message-close').onclick=()=>notify('');
  el.querySelectorAll('[data-retry]').forEach(button=>button.onclick=event=>{event.stopPropagation();const row=rows[Number(button.dataset.failure)];({scan,quotas,prices:syncPrices,hosts:sample})[row.key]?.(row.key==='quotas'?row.accountId:row.id);});
}
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
  const attributes=['data-account-settings','data-heat-day','data-trend-model','data-copy-value','data-day-toggle','data-credit-estimate','data-manage-sampling','data-estimate','data-page','data-settings-tab','data-edit','data-remove','data-price','data-edit-account','data-remove-account','data-restore-account','data-enable','data-gap-map','data-gap-price','data-host-status'];
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
  state.page='settings';state.settingsTab=kind==='hosts'?'hosts':'sources';render();editItem();
  if(kind==='quota'){$('#field-provider').value='deepseek';$('#field-provider').dispatchEvent(new Event('change'));}
}
function onboarding() {
  return `<div class="card onboarding"><h2>连接你的第一个数据源</h2><p class="muted">从本机日志查看用量，关联账户查询余额与限额，或连接 SSH 主机。</p><div class="actions"><button id="add-first-source" class="primary" title="添加本机 Agent 日志数据源">本机日志</button><button id="add-first-quota">账户限额</button><button id="add-first-host">SSH 主机</button></div></div>`;
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
  const rows=records.map((e,i)=>'<div class="sampling-row"><strong>'+escapeHTML(state.settings.accounts.find(a=>a.provider+':'+a.id===e.accountKey)?.name??e.accountKey)+'</strong><span>'+(e.kind==='credits'?'credit 价值':e.valuationMode==='fiveHour'?'5h / 7d 额度价值':'7d 整周价值')+' · '+estimateStatus(e)+'</span>'+(e.reason?'<p class="error">'+escapeHTML(e.reason)+'</p>':'')+'<div class="actions"><button data-sampling-open="'+i+'">查看与管理</button><button data-sampling-stop="'+i+'">结束采样</button></div></div>').join('');
  quotaDialog('管理采样','<p class="muted">关闭窗口后采样继续。只有结束采样才会停止记录。</p>'+(rows||'<p>当前没有进行中的采样</p>'));
  document.querySelectorAll('[data-sampling-open]').forEach(b=>b.onclick=()=>{const e=records[Number(b.dataset.samplingOpen)];openQuotaEstimate(e.accountKey,e.kind==='credits'?'credits':'weekly');});
  document.querySelectorAll('[data-sampling-stop]').forEach(b=>b.onclick=async()=>{const e=records[Number(b.dataset.samplingStop)];if(await quotaAction((e.kind==='credits'?'creditEstimates.':'quotaEstimates.')+'stop',{id:e.id}))openSampling();});
}
function quotaSection(d) {
  const selected=PANEL?AieyesUI.panelAccounts(state.settings,d.quotaOrder??[]):null,visible=a=>!PANEL||selected[a.provider]?.includes(a.provider+':'+(a.accountId??a.id));
  const quotas=(d.quotas??[]).filter(visible),missing=state.settings.accounts.filter(a=>visible(a)&&!a.archived&&a.quotaEnabled&&(!state.provider||a.provider===state.provider)&&(!state.accountKey||state.accountKey===a.provider+':'+a.id)&&!quotas.some(q=>q.provider===a.provider&&q.accountId===a.id));
  if(!quotas.length&&!missing.length)return PANEL&&state.settings.accounts.some(a=>!a.archived&&a.quotaEnabled)?'<p class="muted panel-account-empty">当前面板未显示账户，可在更多中选择；详情保留全部账户。</p>':'';
  return `<div class="section-head quota-heading"><h2 title="实时账户限额 · 不随历史日期或模型筛选变化">实时账户限额 <span class="count">${quotas.length+missing.length}</span></h2><div class="section-actions"><button id="order-quotas" title="调整账户顺序" aria-label="调整账户顺序">${uiIcon('sort')}</button><button id="read-quotas" ${state.quotaBusy||state.refreshState.quotas?.busy?'disabled':''} title="重新查询各账户的实时限额">${state.quotaBusy||state.refreshState.quotas?.busy?'读取中…':'刷新限额'}</button></div></div><div class="quotas">${quotas.map(q=>quotaCard(q)).join('')}${missing.map(a=>{
    const enabled=state.settings.sources.some(src=>src.enabled&&src.provider===a.provider&&src.accountId===a.id&&(!src.hostId||state.settings.hosts.some(h=>h.id===src.hostId&&h.enabled)));
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
  try { await fn();if(!refreshKey)state.saveError='';outcome='success';state.lastSuccessfulUpdate=Date.now();if($('#message').dataset.kind==='error'&&$('#message').textContent===previousMessage)notify('');return true; }
  catch (error) { failure=error;outcome=error.partial?'partial':'error';if(!refreshKey){state.saveError=error.message??String(error);notify(state.saveError,'error');document.querySelectorAll('.saved-feedback').forEach(el=>{el.textContent='保存失败：'+state.saveError;el.dataset.status='error';});}return false; }
  finally {
    state.busy=false;$('#activity').dataset.status=outcome;
    if(refreshKey)endRefresh(refreshKey,failure,refreshItem);else updateActivity();
    buttons.forEach(([button,disabled])=>{if(button.isConnected)button.disabled=disabled;});updateActivity();updateSettingsSaveBar();if(jobFocus&&document.activeElement===document.body){const target=document.querySelector(jobFocus);if(target&&!target.disabled)target.focus({preventScroll:true});else if(jobFocus==='#settings-save-all')$('#settings-save-bar .settings-draft-label')?.focus({preventScroll:true});}
  }
}
const filterKeys=['provider','accountKey','sourceId','model','days'];
const filterSnapshot=()=>Object.fromEntries(filterKeys.map(k=>[k,state[k]]));
function appliedScope(){const f=state.appliedFilters??filterSnapshot();return [f.days===1?'今日':'最近 '+f.days+' 天',providers[f.provider]??'全部 Agent',state.settings.accounts.find(a=>a.provider+':'+a.id===f.accountKey)?.name??(f.accountKey==='none'?'未关联账户':'全部账户'),state.settings.sources.find(s=>s.id===f.sourceId)?.name??'全部来源',f.model||'全部模型'].join(' · ');}
async function changeFilters(values={}){Object.assign(state,values);try{await loadDashboard();}catch(_){} }
function renderQueryStatus(){
  let el=$('#query-status');if(!el){el=document.createElement('div');el.id='query-status';el.setAttribute('role','status');$('#content .filters')?.after(el);}
  el.hidden=!state.dashboardPending&&!state.dashboardError;
  el.innerHTML=el.hidden?'':'<p>'+escapeHTML(state.dashboardPending?'正在更新，仍显示上一结果：'+appliedScope():'更新失败，仍显示上一结果：'+appliedScope()+'。'+state.dashboardError)+'</p>'+(state.dashboardError?'<button id="query-retry">重试</button><button id="query-restore">恢复已应用筛选</button>':'');
  if($('#query-retry'))$('#query-retry').onclick=()=>changeFilters();
  if($('#query-restore'))$('#query-restore').onclick=()=>changeFilters(state.appliedFilters);
}
async function loadDashboard() {
  savePanelView();
  if(state.accountKey && state.accountKey!=='none' && !state.settings.accounts.some(a=>a.provider+':'+a.id===state.accountKey))state.accountKey='';
  if(state.sourceId && !state.settings.sources.some(s=>s.id===state.sourceId))state.sourceId='';
  const account=state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey);
  const request=++state.dashboardRequest,requested=filterSnapshot();state.dashboardPending=true;state.dashboardError='';if(state.page==='agent')renderQueryStatus();
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
  const failures=rows.filter(row=>row.error);if(!failures.length)return null;
  const partial=failures.length<rows.length;
  const error=new Error(`${label}${partial?'部分失败':'失败'}：${failures.map(row=>`${row.name||row.id||row.accountId||''}${row.name||row.id||row.accountId?' · ':''}${row.error}`).join('；')}`);
  error.partial=partial;error.failures=failures;return error;
}
async function scan(sourceId) {
  if(state.estimateBusy||state.sharedEstimateOperations?.size)return;
  await job('同步记录',async()=>{try{const rows=await api('sources.scan',typeof sourceId==='string'?{sourceId}:{});await loadDashboard();const error=batchError(rows,'同步记录');if(error)throw error;}finally{state.lastScan=Date.now();}},'scan',typeof sourceId==='string'?sourceId:null);
}
function hasQuotaSources() {
  return state.settings.accounts.some(a => !a.archived && a.quotaEnabled && state.settings.sources.some(s => s.enabled && s.provider === a.provider && s.accountId === a.id && (!s.hostId || state.settings.hosts.some(h => h.id === s.hostId && h.enabled))));
}
async function quotas(accountId) {
  if(state.estimateBusy||state.sharedEstimateOperations?.size||state.busy||state.settingsSaving||['scan','quotas','prices'].some(key=>state.refreshState[key]?.busy))return;
  state.quotaBusy=true;state.quotaError='';if(state.page==='agent')render();
  try { await job('读取限额',async()=>{try{const rows=await api('quotas.refresh',typeof accountId==='string'?{accountId}:{});await loadDashboard();const error=batchError(rows,'读取限额');if(error)throw error;}catch(error){state.quotaError=error.failures?'部分账户读取失败，请逐项重试。':error.message??String(error);throw error;}finally{state.lastQuota=Date.now();}},'quotas',typeof accountId==='string'?accountId:null); }
  finally {state.quotaBusy=false;if(state.page==='agent')render();}
}
async function syncPrices(){await job('同步价格',async()=>{await api('prices.sync');state.prices=await api('prices.list');await loadDashboard();if(state.page==='settings')renderSettings();notify('价格已更新');},'prices');}
let lastHostError='';
function applyHostSamples(rows) {
  const attemptedAt=Date.now();
  state.hosts=rows.map(row=>({...row,lastAttemptAt:row.lastAttemptAt??attemptedAt,
    ...(row.error?{sample:row.sample??state.hosts.find(previous=>previous.id===row.id)?.sample}:{})}));
  if(!rows.some(row=>row.error)&&lastHostError){if($('#message').textContent===lastHostError)notify('');lastHostError='';}
  state.lastMetrics=attemptedAt;updateActivity();if(state.settings&&state.page==='servers')render();
}
function applyHostFailure(error,hostId=null) {
  const message=error.message??String(error),attemptedAt=Date.now();
  const rows=(state.settings?.hosts??[]).map(host=>{
    const previous=state.hosts.find(row=>row.id===host.id)??{id:host.id};
    return host.enabled&&(!hostId||host.id===hostId)?{...previous,error:message,lastAttemptAt:attemptedAt}:previous;
  });
  applyHostSamples(rows);lastHostError=message;notify(message,'error');
}
async function sample(hostId) {
  if(state.serverBusy||state.refreshState.hosts?.busy)return;state.serverBusy=true;beginRefresh('hosts');let failure=null;
  try{const rows=await api('hosts.sample',typeof hostId==='string'?{hostId}:{});applyHostSamples(typeof hostId==='string'?[...state.hosts.filter(h=>h.id!==hostId),...rows]:rows);failure=batchError(rows,'刷新服务器');}
  catch(error){failure=error;applyHostFailure(error,typeof hostId==='string'?hostId:null);}
  finally{state.serverBusy=false;endRefresh('hosts',failure,typeof hostId==='string'?hostId:null);}
}
async function saveSettings(next,{refresh=true}={}) {
  if (state.settingsSaving) throw new Error('设置正在保存，请稍后重试');
  state.settingsSaving = true;
  const previous=state.settings;
  try {
    await api('settings.save', next);
    state.settings = next;
    window.AieyesTheme?.apply(next.appearance);
    if(JSON.stringify([previous.accounts,previous.sources,previous.hosts,previous.proxy])!==JSON.stringify([next.accounts,next.sources,next.hosts,next.proxy]))state.lastQuota=0;
  } finally { state.settingsSaving = false; }
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
  bar.dataset.dirty=String(count>0);bar.querySelector('.settings-draft-label').textContent=state.settingsCommitting?'正在保存应用配置…':count?'应用配置 · 未保存 '+count+' 项':'应用配置 · 已保存';
  $('#settings-save-all').disabled=!count||state.settingsCommitting||state.busy||editorSaving;
  $('#settings-discard-all').disabled=!count||state.settingsCommitting||state.busy||editorSaving;
}
function collectGeneralDraft(next){
  captureSettingsForms();const values=settingsForms.get('general-form')?.values;if(!values)return next;
  const data=new FormData();for(const [key,value] of Object.entries(values))data.set(key,value);
  next.appearance={theme:data.get("appearanceTheme")||"system",accent:data.get("appearanceAccent")||"indigo"};
  next.proxy=proxyValue(data,'app');next.proxyTestUrls=(data.get('proxyTestUrls')??'').split(',').map(v=>v.trim()).filter(Boolean);
  for(const [key,min,label] of [['refreshSeconds',10,'Agent'],['serverRefreshSeconds',2,'服务器']]){const value=Number(data.get(key));if(!Number.isInteger(value)||value<min||value>86400)throw new Error(label+' 刷新间隔范围为 '+min+'–86400 秒，草稿已保留');next[key]=value;}
  return next;
}
async function saveAllSettings({clearForms=true,refresh=true}={}){
  if(state.settingsCommitting)throw new Error('配置正在保存');
  const next=mappingDraft(collectGeneralDraft(structuredClone(draftSettings()))),preparedHosts=[];
  state.settingsCommitting=true;updateSettingsSaveBar();
  try{
    const current=await api('settings.get');
    if(configKey(current)!==state.settingsBaseline){state.settings=structuredClone(current);throw new Error('配置已在其他窗口修改。草稿已保留，请放弃草稿后重新编辑，避免覆盖其他更改。');}
    for(const [id,key] of pendingAPIKeys){const source=next.sources.find(s=>s.id===id);if(!source||source.provider!=='deepseek')continue;
      let saved=preparedAPIKeys.get(id);if(!saved||saved.key!==key){const result=await api('credentials.save',{sourceId:id+'-draft-'+crypto.randomUUID(),apiKey:key});if(!result.path)throw new Error('未获得凭据保存路径');saved={key,path:result.path};preparedAPIKeys.set(id,saved);}source.path=saved.path;
    }
    for(const [id,password] of pendingHostPasswords){const host=next.hosts.find(h=>h.id===id);if(!host||host.authMode!=='password')continue;const saved=await api('hosts.credentials.save',{password});if(!saved.passwordRef)throw new Error('未获得密码引用');host.passwordRef=saved.passwordRef;preparedHosts.push(saved.passwordRef);}
    await saveSettings(next,{refresh:false});
    pendingAPIKeys.clear();pendingHostPasswords.clear();preparedAPIKeys.clear();
    const committed=await api('settings.get').catch(()=>next);state.settings=committed;state.settingsDraft=structuredClone(committed);state.settingsBaseline=configKey(committed);
    if(clearForms)settingsForms.clear();
    else { const general=settingsForms.get('general-form');if(general){general.baseline=structuredClone(general.values);const form=$('#general-form');if(form)form.dataset.draftBaseline=JSON.stringify(general.baseline);} }
    if(refresh)await loadDashboard().catch(error=>notify('配置已保存，概览刷新失败：'+String(error),'error'));
    if(state.page==='settings')renderSettings(!clearForms);
    return true;
  }catch(error){for(const passwordRef of preparedHosts)await api('hosts.credentials.delete',{passwordRef}).catch(()=>{});throw error;}
  finally{state.settingsCommitting=false;updateSettingsSaveBar();}
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
function creditRow(q, record) {
  return `<div class="credit-row">${creditBalance(q,record)}<button class="credit-estimate-entry" data-credit-estimate="${escapeHTML(q.provider+':'+q.accountId)}">${record?estimateStatus(record)+' · ':''}估值 ${uiIcon('right')}</button></div>`;
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
  const account=state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey);
  const sources=state.settings.sources.filter(src=>{const ids=d.sources.find(s=>s.id===src.id)?.accountIds??[src.accountId];return (!state.provider||src.provider===state.provider)&&(!state.accountKey||(state.accountKey==='none'?ids.includes(''):src.provider===account?.provider&&ids.includes(account?.id)));});
  const trendModels=[...new Set(d.dayModels.map(r=>r.model))].sort();
  $('#title').textContent='Agent 概览';
  $('#content').innerHTML=`${state.settings.sources.length?'':onboarding()}<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','未关联账户',state.accountKey)}${state.settings.accounts.filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select><select id="source" aria-label="数据源">${option('','全部数据源',state.sourceId)}${sources.map(s=>option(s.id,s.name,state.sourceId)).join('')}</select><select id="model" aria-label="模型">${option('','全部模型',state.model)}${[...new Set([...(d.modelOptions??trendModels),...(state.model?[state.model]:[])])].map(m=>option(m,m,state.model)).join('')}</select></div>
  ${s.total>0&&s.pricedTokens<s.total?attention('计价未完成：部分 Token 缺少价格','补充价格','pricing-attention'):''}
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':n===365?'最近一年':`最近 ${n} 天`,state.days)).join('')}</select></div>
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
  $('#content').innerHTML=`${state.settings.sources.length?'':onboarding()}<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','未关联账户',state.accountKey)}${state.settings.accounts.filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select></div>
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':n===365?'最近一年':`最近 ${n} 天`,state.days)).join('')}</select></div>
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
    const usage=AieyesUI.usageSources(state.settings),fresh=AieyesUI.freshness(state.settings,d),filtered=Boolean(state.provider||state.accountKey||state.sourceId||state.model),onlyQuota=state.settings.sources.every(s=>['agy','deepseek'].includes(s.provider));
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
  return `<div class="daily-table table-scroll"><table><thead><tr><th scope="col">日期 / 模型</th><th scope="col">Token</th><th scope="col">读取命中率</th><th scope="col">已计价成本 USD</th></tr></thead>${[...days].reverse().map(day=>`<tbody data-agent-detail="day:${escapeHTML(day.key)}"><tr class="day"><th scope="row"><button data-day-toggle="${day.key}" aria-expanded="false" aria-label="展开 ${day.key} 的模型明细">${day.key}</button></th><td title="${day.total} Token">${compact(day.total)}</td><td>${pct(cacheRate(day.tokens))}</td><td>${day.pricedTokens||!day.total?day.cost.toFixed(6):'—'}</td></tr>${rows.filter(r=>r.day===day.key).map(r=>`<tr class="day-model" hidden><th scope="row"><span>${escapeHTML(r.model)}</span><button data-copy-value="${escapeHTML(r.model+'\t'+r.usage.total+' Token\t'+r.usage.cost+' USD')}" aria-label="复制 ${escapeHTML(r.model)}">复制</button></th><td title="${r.usage.total} Token">${compact(r.usage.total)}</td><td>${pct(cacheRate(r.usage.tokens))}</td><td>${r.usage.pricedTokens?r.usage.cost.toFixed(6):'—'}</td></tr>`).join('')}</tbody>`).join('')}</table></div>`;
}

function estimateSummary(records, credits=false) {
  const entries=AieyesUI.estimateEntries(records,credits), latest=AieyesUI.canonicalEstimates(records)[0];
  return '<div class="estimate-summary">'+entries.map(e=>`<div><span class="estimate-amount">${escapeHTML(e.label)} ≈ ${money(e.value)} USD</span>${e.historical?`<small>历史采样 · ${date(e.record.checkpointAt)}${e.record.originalEstimateId?' · 已修正':''}</small>`:''}</div>`).join('')+(latest&&(![latest.fiveHourValue,latest.weeklyValue,latest.valuePer1000].some(Number.isFinite)||latest.status==='pending')?`<p class="muted">最新采样 · ${estimateStatus(latest)} · ${escapeHTML(AieyesUI.estimateIssue(latest))}</p>`:'')+'</div>';
}
function quotaCard(q) {
  const key=q.provider+':'+q.accountId, records=(state.dashboard?.quotaEstimates??[]).filter(e=>e.accountKey===key), credits=(state.dashboard?.creditEstimates??[]).filter(e=>e.accountKey===key), latest=AieyesUI.canonicalEstimates(records)[0], credit=AieyesUI.estimateEntries(credits,true)[0]?.record ?? credits[0];
  const windowHTML=w=>`<div class="quota-window"><div class="between tiny"><span>${escapeHTML(w.name)}</span><strong style="color:${quotaColor(Number.isFinite(w.usedPercent)?resourcePercent(100-w.usedPercent):null)}">剩余 ${pct(Number.isFinite(w.usedPercent)?resourcePercent(100-w.usedPercent):null)}</strong></div>${resourceBar(Number.isFinite(w.usedPercent)?100-w.usedPercent:null,w.name+'剩余额度',true)}<div class="tiny muted quota-reset" data-reset-at="${w.resetsAt??0}" data-window-minutes="${w.windowMinutes??''}">${AieyesUI.quotaReset(w)}</div></div>`;
  const collapsible=true;
  let expanded=true;try{const stored=localStorage.getItem('quota.expanded.v2.'+(PANEL?'panel.':'detail.')+key);if(collapsible&&stored!=null)expanded=stored==='true';}catch(_){}

  let windows='<div class="quota-windows">'+q.windows.map(windowHTML).join('')+'</div>';
  if(q.provider==='agy'){
    const groups=new Map();for(const w of q.windows){const group=w.groupName||w.name.split(' · ').slice(0,-1).join(' · ');if(!groups.has(group))groups.set(group,[]);groups.get(group).push(w);}
    windows=`<div class="agy-groups">${[...groups].map(([group,rows])=>`<div class="agy-group"><strong>${escapeHTML(group)}</strong><div class="agy-windows">${rows.sort((a,b)=>(a.windowMinutes??0)-(b.windowMinutes??0)).map(w=>windowHTML({...w,name:w.windowMinutes===10080?'7d':w.windowMinutes===300?'5h':w.name})).join('')}</div></div>`).join('')}</div>`;
  }
  return `<div class="card quota-card" data-quota="${escapeHTML(key)}"><details class="quota-disclosure" data-collapsible="${collapsible}" data-agent-detail="quota:${escapeHTML(key)}" ${expanded?'open':''}><summary class="quota-title"><h3>${providerMark(q.provider)}<span class="quota-name" title="${escapeHTML(q.name)}">${escapeHTML(q.name)}</span></h3>${subscriptionBadge(q.plan)}${q.provider==='codex'?`<span class="quota-credit-summary">${creditBalance(q,credit,true)}</span>`:''}${collapsible?`<span class="quota-chevron" aria-hidden="true">${uiIcon('chevron')}</span>`:''}</summary></details><div class="quota-content"><div class="between tiny muted quota-meta"><span>${escapeHTML(providers[q.provider]??q.provider)}</span><span>${q.origin==='log'?'记录':'更新'} ${date(q.updatedAt)}</span></div>${(q.balances??[]).map(b=>`<div class="quota-window"><div class="between"><span>可用余额</span><strong>${escapeHTML(b.currency)} ${escapeHTML(b.total)}</strong></div><div class="between tiny muted balance-detail"><span>赠送 ${escapeHTML(b.granted)}</span><span>充值 ${escapeHTML(b.toppedUp)}</span></div></div>`).join('')}${q.isAvailable===false?'<p class="error">当前余额不足以调用 API</p>':''}${windows}${q.bankReset?`<details class="bank" data-agent-detail="bank:${escapeHTML(key)}"><summary>Bank Reset · ${q.bankReset.availableCount} 次可用</summary>${(q.bankReset.credits??[]).map(c=>`<div class="between tiny muted"><span>${escapeHTML(c.title??'Reset Credit')}</span><span>到期 ${date(c.expiresAt)}</span></div>`).join('')}</details>`:''}${q.error?`<p class="error quota-error" title="${escapeHTML(q.error)}"><span class="error-detail">${escapeHTML(q.error)}</span><span class="short-error">限额更新失败 · 展开查看</span></p>`:''}${(q.provider!=='agy'&&q.windows.some(w=>w.windowMinutes===300||w.windowMinutes===10080))||latest?`<button class="estimate-entry${latest?' sampling-entry':''}" data-estimate="${escapeHTML(key)}"><span>${uiIcon('trend')} ${latest?.valuationMode==='fiveHour'?'5h / 7d 估值':latest?'7d 整周估值':'估算额度价值'}</span><strong></strong>${uiIcon('right')}</button>`:''}${estimateSummary(records)}${q.provider==='codex'?creditRow(q,credit)+estimateSummary(credits,true):''}</div></div>`;
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
    const host=state.settings.hosts.find(row=>row.id===element.dataset.hostStatus);if(!host)continue;
    const status=hostStatus(host,state.hosts.find(row=>row.id===host.id));element.onclick=()=>sample(host.id);element.title=status+' · 点击刷新服务器';
    if(element.dataset.status!==status){element.dataset.status=status;element.querySelector('[data-host-status-label]').textContent=status;}
  }
}
function renderServers() {
  const view=rememberView();
  $('#content').innerHTML = `<div class="section-head"><span class="muted">${state.settings.hosts.length} 台主机</span><select id="server-filter" aria-label="服务器筛选">${[['all','全部'],['errors','异常'],['paused','已暂停']].map(([key,label])=>option(key,label,state.serverFilter??'all')).join('')}</select><button id="sample" title="立即采样所有已启用的服务器">刷新服务器</button></div>${state.settings.hosts.filter(h=>!state.serverFilter||state.serverFilter==='all'||(state.serverFilter==='paused'?!h.enabled:h.enabled&&hostStatus(h,state.hosts.find(r=>r.id===h.id))!=='正常')).map(h=>{
    const result=state.hosts.find(r=>r.id===h.id),s=result?.sample,cpu=s?.cpu?.find(c=>c.id==='cpu'),status=hostStatus(h,result);
    return `<details class="card server-card" data-agent-detail="host:${escapeHTML(h.id)}"><summary class="server-summary"><div class="between"><div><h2 style="margin:0">${escapeHTML(h.name||h.target)}</h2><div class="sub">${escapeHTML(h.target)}</div></div><button type="button" class="host-status" data-host-status="${escapeHTML(h.id)}" data-status="${status}"><i aria-hidden="true"></i><span data-host-status-label>${status}</span></button></div><div class="server-summary-metrics">${(h.metrics??Object.keys(groups)).includes('cpu')?'<span>CPU <strong>'+pct(cpu?.utilization)+'</strong></span>':''}${(h.metrics??Object.keys(groups)).includes('memory')?'<span>内存 <strong>'+pct(s?.memory?capacityPercent(s.memory.total-s.memory.available,s.memory.total):null)+'</strong></span>':''}${(h.metrics??Object.keys(groups)).includes('gpu')?'<span>GPU <strong>'+pct(s?.gpu?.some(g=>g.utilization!=null)?Math.max(...s.gpu.flatMap(g=>g.utilization==null?[]:[g.utilization])):null)+'</strong> / '+(s?.gpu?.length??0)+' 卡</span>':''}<span class="muted">采样 ${date(s?.timestamp)}${!h.enabled?' · 已暂停，保留旧读数':''}</span></div></summary>${s ? `<div class="server-gauges">${s.cpu?resourceRing('CPU',cpu?.utilization):''}${s.memory?resourceRing('内存',capacityPercent(s.memory.total-s.memory.available,s.memory.total),`${bytes(s.memory.total-s.memory.available)} / ${bytes(s.memory.total)}`):''}${(s.gpu??[]).map(g=>resourceRing(`GPU ${g.id}`,g.utilization,g.name??'')).join('')}</div>${Object.entries(groups).filter(([key])=>s[key]).map(([key,label])=>serverGroup(key,label,s[key],h)).join('')}<div class="between tiny muted"><span>负载 ${s.load.map(n=>n.toFixed(2)).join(' / ')}</span><span>${date(s.timestamp)}</span></div>${Object.keys(s.errors??{}).map(k=>`<p class="error">${groups[k]??escapeHTML(k)} · 采集失败</p>`).join('')}`:''}${result?.error?`<p class="error">${escapeHTML(result.error)}</p>`:''}</details>`;
  }).join('')||'<div class="card empty"><button id="add-first-host" class="primary" title="新建第一台服务器">添加服务器</button></div>'}`;
  restoreView(view);
  $('#sample').onclick=sample;$('#server-filter').onchange=e=>{state.serverFilter=e.target.value;savePanelView();renderServers();};updateHostStatuses();
  bindSetup();
}
function serverGroup(key,label,value,host) {
  const show=key=>host.details==null||host.details.includes(key);
  const row=(name,value)=>`<div class="metric-row"><span>${escapeHTML(name)}</span><strong>${value}</strong></div>`;
  const meter=(name,value,percent)=>row(name,value)+resourceBar(percent,name);
  let content='';
  if(key==='memory')content=row('可用',bytes(value.available))+(show('memoryCache')?row('缓存 / Buffer',`${bytes(value.cached)} / ${bytes(value.buffers)}`):'')+(show('swap')?meter('Swap',`${bytes(value.swapTotal-value.swapFree)} / ${bytes(value.swapTotal)}`,capacityPercent(value.swapTotal-value.swapFree,value.swapTotal)):'');
  else content=value.map(d=>{
    if(key==='cpu')return meter(d.id,pct(resourcePercent(d.utilization)),d.utilization)+(show('cpuTimes')?row('user / system',`${pct(d.userPercent)} / ${pct(d.systemPercent)}`)+row('iowait / steal',`${pct(d.iowaitPercent)} / ${pct(d.stealPercent)}`):'');
    if(key==='gpu')return meter(d.name??`GPU ${d.id}`,pct(resourcePercent(d.utilization)),d.utilization)+(show('gpuMemory')?meter('显存',`${bytes(d.memoryUsedMiB==null?null:d.memoryUsedMiB*1048576)} / ${bytes(d.memoryTotalMiB==null?null:d.memoryTotalMiB*1048576)}`,capacityPercent(d.memoryUsedMiB,d.memoryTotalMiB)):'')+(show('gpuThermals')?row('温度 / 功耗',`${d.temperature==null?'—':d.temperature+'°C'} / ${d.powerWatts==null?'—':d.powerWatts+' W'}`):'');
    if(key==='filesystems')return meter(d.id,`${bytes(d.used)} / ${bytes(d.total)}`,capacityPercent(d.used,d.total))+(show('fsAvailable')?row('可用',bytes(d.available)):'')+(show('fsType')?row(d.device??'',escapeHTML(d.type??'')):'')+(show('inodes')?meter('inode',pct(capacityPercent(d.inodes-d.inodesFree,d.inodes)),capacityPercent(d.inodes-d.inodesFree,d.inodes)):'');
    if(key==='disk')return row(d.id,`读 ${speed(d.readBytesPerSecond)} · 写 ${speed(d.writeBytesPerSecond)}`)+(show('diskIops')?row('IOPS 读 / 写',`${d.readIops==null?'—':compact(d.readIops)} / ${d.writeIops==null?'—':compact(d.writeIops)}`):'')+(show('diskBusy')?meter('忙碌率',pct(resourcePercent(d.busyMsPerSecond==null?null:d.busyMsPerSecond/10)),d.busyMsPerSecond==null?null:d.busyMsPerSecond/10):'');
    return row(d.id,`↓ ${speed(d.rxBytesPerSecond)} · ↑ ${speed(d.txBytesPerSecond)}`)+(show('networkTotals')?row('累计接收 / 发送',`${bytes(d.rxBytes)} / ${bytes(d.txBytes)}`):'')+(show('networkErrors')?row('错误 / 丢包',`${compact((d.rxErrors??0)+(d.txErrors??0))} / ${compact((d.rxDrops??0)+(d.txDrops??0))}`):'');
  }).join('');
  return `<details class="metric-section" data-metric="${escapeHTML(host.id+':'+key)}"><summary>${label}</summary><div class="metric-detail">${content}</div></details>`;
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
  return error?`<span class="error" role="status">${escapeHTML(error)}</span>`:`<small>${['agy','deepseek'].includes(source?.provider)?'限额查询来源':status?.updatedAt?'记录同步于 '+date(status.updatedAt):'尚未同步用量记录'}</small>`;
}
function accountRows(accounts) {
  return accounts.map(a=>{const i=draftSettings().accounts.indexOf(a);return `<div class="list-row">${providerMark(a.provider)}<div class="row-body">${escapeHTML(a.name)}<small>${escapeHTML(providers[a.provider]??a.provider)}</small></div><button data-edit-account="${i}" aria-label="编辑账户 ${escapeHTML(a.name)}" title="编辑账户">编辑</button>${a.archived?`<button data-restore-account="${i}" aria-label="恢复账户 ${escapeHTML(a.name)}" title="把归档账户恢复为正常状态">恢复</button>`:`<button data-remove-account="${i}" aria-label="归档账户 ${escapeHTML(a.name)}" title="归档账户并保留历史">归档</button>`}</div>`;}).join('');
}
const settingsForms = new Map();
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
    form.addEventListener('input',update);form.addEventListener('change',update);update();
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
  if(state.settingsTab==='accounts')state.settingsTab='sources';
  if(state.settingsTab==='connection')state.settingsTab='general';
  const tabs={sources:'数据源',hosts:'服务器',prices:'价格',wakeups:'定时唤醒',general:'通用'};
  let body='';const s=draftSettings();
  if(state.settingsTab==='sources'||state.settingsTab==='hosts'){
    const isSource=state.settingsTab==='sources',list=isSource?s.sources:s.hosts;
    body=`<div class="card">${list.map(item=>`<div class="list-row"><input type="checkbox" role="switch" data-enable="${escapeHTML(item.id)}" ${item.enabled?'checked':''} aria-label="启用 ${escapeHTML(item.name)}">${isSource?providerMark(item.provider):''}<div class="row-body">${escapeHTML(item.name||item.target)}<small>${escapeHTML(isSource?`${providers[item.provider]} · ${item.accountId ? (s.accounts.find(a=>a.id===item.accountId&&a.provider===item.provider)?.name??item.accountId) : "无账户"}`:item.target)}</small>${isSource?sourceError(item.id):''}</div><button data-edit="${escapeHTML(item.id)}" aria-label="编辑 ${escapeHTML(item.name||item.target)}" title="编辑此项">编辑</button><button data-remove="${escapeHTML(item.id)}" aria-label="移除 ${escapeHTML(item.name)}" title="移除此项">${uiIcon('minus')}</button></div>`).join('')||'<div class="empty">添加第一个'+(isSource?'数据源':'主机')+'</div>'}${savedFeedback(state.settingsTab)}</div><button id="add-item" class="primary" title="新建数据源或服务器">${uiIcon('plus')} 添加${isSource?'数据源':'主机'}</button>`;
    if(isSource){
      const parser=document.createElement('div');parser.innerHTML=body;
      const rows=[...parser.querySelectorAll('.list-row')],card=parser.querySelector('.card');card.replaceChildren();
      for(const account of s.accounts.filter(a=>!a.archived)){
        const group=document.createElement('section');group.className='account-group';const linked=s.sources.filter(src=>src.provider===account.provider&&src.accountId===account.id);
        group.innerHTML='<div class="section-head"><div><h3>'+providerMark(account.provider)+' '+escapeHTML(account.name)+'</h3><p class="muted tiny">共用 '+linked.length+' 个来源 · 账户名称与限额设置共同生效</p></div><button data-edit-account="'+s.accounts.indexOf(account)+'" aria-label="编辑账户 '+escapeHTML(account.name)+'">编辑账户</button></div>';
        for(const source of linked){const row=rows.find(row=>row.querySelector('[data-edit]')?.dataset.edit===source.id);if(row)group.append(row);}card.append(group);
      }
      const ungrouped=rows.filter(row=>!row.isConnected&&!row.closest('.account-group'));if(ungrouped.length){const group=document.createElement('section');group.className='account-group';group.innerHTML='<h3>未关联账户的来源</h3>';group.append(...ungrouped);card.append(group);}
      body=parser.innerHTML;
      const orphan=s.accounts.filter(a=>!a.archived&&!s.sources.some(src=>src.accountId===a.id&&src.provider===a.provider)),archived=s.accounts.filter(a=>a.archived);
      if(orphan.length)body+=`<details class="card account-history"><summary>未关联账户</summary>${accountRows(orphan)}</details>`;
      if(archived.length)body+=`<details class="card account-history"><summary>已归档账户</summary>${accountRows(archived)}</details>`;
    }
  }else if(state.settingsTab==='prices')body='<div class="between"><p class="muted save-model-note">价格条目与同步立即生效；模型映射随顶部保存全部提交。</p><button id="reprice" type="button" title="保存设置并按当前价格重新计算历史成本">保存并重算</button></div>'+gapList()+`<form id="mapping-form" class="card"><h2>模型映射</h2>${Object.entries(s.modelMappings).map(([from,to])=>`<div class="list-row tiny"><span>${escapeHTML(from)} → ${escapeHTML(to)}</span><button type="button" data-unmap="${escapeHTML(from)}" aria-label="移除 ${escapeHTML(from)} 的模型映射">${uiIcon('minus')}</button></div>`).join('')}${field('mappingModel','日志模型名称')}${field('mappingId','OpenRouter 模型 ID')}<button>添加映射</button></form>`+`<div class="price-toolbar"><input id="price-search" type="search" aria-label="搜索模型" placeholder="搜索模型" value="${escapeHTML(state.priceSearch)}"><button id="sync-prices" title="从项目 Release 同步 OpenRouter 价格表">同步 OpenRouter</button><button id="add-price" title="手动添加一个模型价格">添加价格</button>${savedFeedback('prices')}</div><div class="card prices-list">${priceRows()}</div><span class="muted tiny">USD / 百万 Token</span>`;
  else if(state.settingsTab==='wakeups')body=window.AieyesWakeups?.html()??'<p>正在加载…</p>';
  else body=(window.AieyesDesktop?.settingsHTML() || '')+`<form id="general-form" class="card"><h2>外观</h2>${select('appearanceTheme','主题',[['system','跟随系统'],['light','浅色'],['dark','深色']],s.appearance?.theme??'system')}${select('appearanceAccent','强调色',[['indigo','靛蓝'],['blue','蓝'],['teal','青绿'],['purple','紫']],s.appearance?.accent??'indigo')}<h2>网络与刷新</h2>${proxyFields('app',s.proxy)}${field('proxyTestUrls','测试地址（逗号分隔）',(s.proxyTestUrls??['https://api.github.com/rate_limit','https://openrouter.ai']).join(', '))}<h3>刷新</h3>${field('refreshSeconds','Agent 间隔（秒）',s.refreshSeconds,'','number')}${field('serverRefreshSeconds','服务器间隔（秒）',s.serverRefreshSeconds,'','number')}<p class="muted tiny">修改保留为草稿，请使用顶部保存全部。</p></form>${window.AieyesUpdates?.settingsHTML() ?? '<div class="card"><p>正在读取更新信息…</p></div>'}`;
  $('#content').innerHTML=`<div class="settings-save-bar" id="settings-save-bar"><div><p class="settings-draft-label" role="status" tabindex="-1"></p><p class="muted tiny">编辑器更新草稿，顶部保存全部；价格、定时唤醒与本机显示偏好独立保存。</p></div><button type="button" id="settings-discard-all">放弃更改</button><button type="button" class="primary" id="settings-save-all">保存应用配置</button></div><div class="settings-tabs" role="tablist" aria-label="设置分类">${Object.entries(tabs).map(([k,v])=>`<button id="settings-tab-${k}" data-settings-tab="${k}" role="tab" aria-controls="settings-panel" aria-selected="${state.settingsTab===k}" tabindex="${state.settingsTab===k?0:-1}" class="${state.settingsTab===k?'active':''}">${uiIcon(k)}<span>${v}</span></button>`).join('')}</div><div id="settings-panel" class="settings-block" role="tabpanel" aria-labelledby="settings-tab-${state.settingsTab}">${body}</div>`;
  bindSettingsDrafts();window.AieyesDesktop?.bindSettings();updateSettingsSaveBar();
  $('#settings-save-all').onclick=()=>job('保存应用配置',()=>saveAllSettings());
  $('#settings-discard-all').onclick=()=>{showEditor('放弃配置更改','<p>放弃全部未保存的配置草稿？当前运行配置继续生效。</p>',async()=>discardAllSettings());$('#editor-form button[type=submit]').textContent='放弃更改';$('#editor-form button[type=submit]').classList.add('danger');};
  if(state.settingsTab==='wakeups')window.AieyesWakeups?.bind();
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
  document.querySelectorAll('[data-edit-account]').forEach(b=>b.onclick=()=>editAccount(Number(b.dataset.editAccount)));
  document.querySelectorAll('[data-remove-account]').forEach(b=>b.onclick=()=>{
    const a=s.accounts[Number(b.dataset.removeAccount)];
    showEditor('归档账户',`<p>归档「${escapeHTML(a.name)}」并保留历史？保存全部后生效，可从已归档账户恢复。</p>`,async()=>{
      const next=structuredClone(draftSettings());next.accounts.find(row=>row.id===a.id&&row.provider===a.provider).archived=true;
      stageSettings(next);renderSettings();
    });
    $('#editor-form button[type=submit]').textContent='归档并保留历史';$('#editor-form button[type=submit]').classList.add('danger');$('#cancel-editor').focus();
  });
  document.querySelectorAll('[data-restore-account]').forEach(b=>b.onclick=()=>job('恢复账户草稿',async()=>{
    const next=structuredClone(draftSettings());next.accounts[Number(b.dataset.restoreAccount)].archived=false;
    stageSettings(next);renderSettings();
  }));
  document.querySelectorAll('[data-gap-price]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapPrice)];editPrice(state.prices.find(p=>p.id===(g.priceId??g.model))??{id:g.priceId??g.model,name:g.model});});
  document.querySelectorAll('[data-gap-map]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapMap)];$('#field-mappingModel').value=g.model;$('#field-mappingId').value=g.priceId??'';$('#field-mappingId').focus();});
  const items=state.settingsTab==='sources'?s.sources:s.hosts;
  document.querySelectorAll('[data-edit]').forEach(b=>b.onclick=()=>editItem(items.find(i=>i.id===b.dataset.edit)));
  document.querySelectorAll('[data-enable]').forEach(b=>b.onchange=async()=>{
    const list=state.settingsTab==='sources'?'sources':'hosts',next=structuredClone(draftSettings());
    next[list].find(i=>i.id===b.dataset.enable).enabled=b.checked;b.disabled=true;
    try { await job('更新草稿',()=>stageSettings(next)); }
    finally { b.checked=draftSettings()[list].find(i=>i.id===b.dataset.enable)?.enabled??false;b.disabled=false; }
  });
  document.querySelectorAll('[data-remove]').forEach(button=>button.onclick=()=>{
    const kind=state.settingsTab,id=button.dataset.remove,item=draftSettings()[kind].find(row=>row.id===id);
    const linked=kind==='hosts'?draftSettings().sources.filter(src=>src.hostId===id):[];
    const impact=linked.length?`<p>保存全部后，以下 ${linked.length} 个数据源将暂停，并需要重新选择服务器：</p><ul>${linked.map(src=>`<li>${escapeHTML(src.name)}</li>`).join('')}</ul>`:'<p>已导入的用量历史会保留。重新连接时需要再次添加配置。</p>';
    showEditor(kind==='hosts'?'移除服务器':'移除数据源',`<p>确认移除「${escapeHTML(item.name||item.target)}」？</p>${impact}`,async()=>{
      const next=structuredClone(draftSettings());next[kind]=next[kind].filter(row=>row.id!==id);
      if(kind==='hosts')for(const src of next.sources)if(src.hostId===id){src.hostId=null;src.enabled=false;}
      if(kind==='sources')for(const account of next.accounts)if(account.quotaSourceId===id)account.quotaSourceId=null;
      stageSettings(next);renderSettings();notify(linked.length?`待保存：移除服务器，${linked.length} 个关联数据源将暂停`:'待保存：移除配置，历史记录将保留');
    });
    const submit=$('#editor-form button[type=submit]');submit.textContent='确认移除';submit.classList.add('danger');
    $('#cancel-editor').focus();
  });
  if($('#add-item'))$('#add-item').onclick=()=>editItem();

  if($('#general-form'))$('#general-form').onsubmit=e=>{e.preventDefault();updateSettingsSaveBar();};
  window.AieyesUpdates?.bindSettings();
  if($('#sync-prices'))$('#sync-prices').onclick=syncPrices;
  if($('#add-price'))$('#add-price').onclick=()=>editPrice();
  bindPrices();
  if($('#price-search'))$('#price-search').oninput=e=>{state.priceSearch=e.target.value;$('.prices-list').innerHTML=priceRows();bindPrices();};
  if($('#general-form'))bindProxy('app');
  if($('#mapping-form'))$('#mapping-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);if(!f.get('mappingModel')?.trim()||!f.get('mappingId')?.trim()){notify('请输入日志模型名称和 OpenRouter 模型 ID','error');return;}job('更新映射草稿',async()=>{const next=structuredClone(draftSettings());mappingDraft(next);stageSettings(next);$('#mapping-form').reset();resetSettingsForm('mapping-form');renderSettings();});};
  document.querySelectorAll('[data-unmap]').forEach(b=>b.onclick=()=>job('更新映射草稿',async()=>{const next=structuredClone(draftSettings());delete next.modelMappings[b.dataset.unmap];stageSettings(next);renderSettings();}));
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
  const a=structuredClone(index==null?{id:crypto.randomUUID(),name:'',provider:'codex',quotaEnabled:true,quotaSourceId:null}:draftSettings().accounts[index]);
  const linked=draftSettings().sources.filter(s=>s.accountId===a.id&&s.provider===a.provider);
  showEditor('账户',field('name','账户名称',a.name,'个人账户 / 工作账户')+select('provider','Agent',Object.entries(providers),a.provider)+select('quotaEnabled','账户限额',[['yes','显示并查询'],['no','仅统计用量']],a.quotaEnabled?'yes':'no')+select('quotaSourceId','优先查询位置',[['','自动 · 优先本机'],...linked.map(s=>[s.id,s.name])],a.quotaSourceId??'')+'',async f=>{
    a.name=f.get('name').trim();if(!a.name)throw new Error('请输入账户名称');if(!linked.length)a.provider=f.get('provider');a.quotaEnabled=f.get('quotaEnabled')==='yes';a.quotaSourceId=f.get('quotaSourceId')||null;
    const next=structuredClone(draftSettings());if(index==null)next.accounts.push(a);else next.accounts[index]=a;
    stageSettings(next);renderSettings();
  });
  $('#editor').dataset.kind='account';$('#editor-form button[type=submit]').textContent='更新草稿';
  $('#field-provider').disabled=linked.length>0;
  $('#field-provider').onchange=e=>{$('#field-quotaEnabled').value=['codex','claude','agy','deepseek'].includes(e.target.value)?'yes':'no';};
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
  const source=state.settingsTab==='sources';
  const item=structuredClone(existing??(source?{id:crypto.randomUUID(),name:'',provider:'codex',accountId:'',path:'~/.codex',hostId:null,enabled:true,quotaCommand:'',quotaPreCommand:'',codexBinary:'codex',agyBinary:'agy',proxy:null}:{id:crypto.randomUUID(),name:'',target:'',authMode:'ssh',username:'',passwordRef:'',port:null,identityFile:'',shell:'/bin/bash',preCommand:'',enabled:true,metrics:Object.keys(groups),devices:[],details:Object.keys(detailOptions)}));
  const accountOptions=provider=>[['__new__','新建账户'],...draftSettings().accounts.filter(a=>a.provider===provider&&(!a.archived||a.id===item.accountId)).map(a=>[a.id,a.name+(a.archived?'（已归档）':'')])];
  const account=draftSettings().accounts.find(a=>a.id===item.accountId&&a.provider===item.provider);
  const body=source?field('name','名称',item.name)+select('provider','Agent',Object.entries(providers),item.provider)+`<label class="account-toggle"><input type="checkbox" name="isAccount" id="field-isAccount" ${item.accountId?'checked':''} aria-describedby="account-help">关联账户与限额</label><p id="account-help" class="muted tiny account-help">数据源用于读取记录；关联账户后可查询限额或余额。同一账户的名称与限额设置由关联它的数据源共用。</p><div id="account-fields"><h3>账户与限额</h3>${select('accountId','账户',accountOptions(item.provider),item.accountId||'__new__')}${field('accountName','账户名称',account?.name??'')}${select('quotaEnabled','账户限额',[['yes','显示并查询'],['no','仅统计用量']],account?.quotaEnabled?'yes':'no')}${select('accountArchived','账户状态',[['no','正常'],['yes','已归档']],account?.archived?'yes':'no')}${select('quotaSourceId','优先查询位置',[['','自动 · 优先本机']],account?.quotaSourceId??'')}</div>`+select('hostId','位置',[['','本机'],...draftSettings().hosts.map(h=>[h.id,h.name||h.target])],item.hostId??'')+field('path','数据目录',item.path)+field('codexBinary','Codex 程序',item.codexBinary)+field('agyBinary','agy 程序',item.agyBinary??'agy')+`<div id="deepseek-key">${field('apiKey','API Key（留空保留）',pendingAPIKeys.get(item.id)??'','','password')}</div><div id="remote-quota">${textarea('quotaPreCommand','限额查询前置命令',item.quotaPreCommand)}</div><div id="local-proxy">${proxyFields('source',item.proxy,true)}</div>`:
    field('name','名称',item.name)+field('target','SSH 别名或地址',item.target,'my-server 或 user@host')+field('port','端口',item.port??'','跟随 SSH 配置','number')+select('authMode','登录方式',[['ssh','SSH 配置 / 密钥'],['password','账号密码']],item.authMode??'ssh')+field('username','用户名',item.username??'','跟随地址或 SSH 配置')+`<div id="host-password">${field('password','密码（留空保留）',pendingHostPasswords.get(item.id)??'','','password')}</div><div id="host-key">${field('identityFile','密钥路径',item.identityFile,'跟随 SSH 配置')}</div>`+`<section class="host-selectors"><div id="metric-select"></div><div class="section-head"><h3>设备</h3><button type="button" id="discover-devices" title="通过 SSH 读取远端设备列表">读取设备</button></div><p id="discovery-status" class="error" role="status" hidden></p><div id="device-selects" class="device-selects"></div><div id="detail-select"></div></section><details class="advanced"><summary>高级设置</summary>${field('shell','远程 shell',item.shell)+textarea('preCommand','远程前置命令',item.preCommand)+field('devices','设备表达式',item.devices.join(', '),'network:eth0, gpu:0, filesystems:/')}</details>`;
  showEditor(source?'数据源':'SSH 主机',body,async f=>{
    const next=structuredClone(draftSettings());
    if(source){
      for(const k of ['name','provider','path','codexBinary','agyBinary','quotaPreCommand'])item[k]=f.get(k);
      item.accountId='';
      if(f.has('isAccount')){
        const selected=f.get('accountId');let a=next.accounts.find(a=>a.id===selected&&a.provider===item.provider);
        if(!a){if(selected!=='__new__')throw new Error('请选择账户');a={id:crypto.randomUUID(),provider:item.provider,archived:false};next.accounts.push(a);}
        a.name=f.get('accountName').trim()||item.name.trim()||providers[item.provider];
        a.quotaEnabled=f.get('quotaEnabled')==='yes';a.archived=f.get('accountArchived')==='yes';a.quotaSourceId=f.get('quotaSourceId')||null;item.accountId=a.id;
      }
      if(!['agy','deepseek'].includes(item.provider)&&!item.path?.trim())throw new Error('请输入数据目录');
      item.hostId=f.get('hostId')||null;item.proxy=proxyValue(f,'source');if(!item.name)item.name=providers[item.provider];
      if(item.provider==='deepseek'){item.hostId=null;if(f.get('apiKey')?.trim())pendingAPIKeys.set(item.id,f.get('apiKey').trim());}
    }else{for(const k of ['name','target','identityFile','shell','preCommand','authMode','username'])item[k]=f.get(k);if(item.authMode==='password'&&f.get('password'))pendingHostPasswords.set(item.id,f.get('password'));item.port=f.get('port')?Number(f.get('port')):null;item.devices=f.get('devices').split(',').map(s=>s.trim()).filter(Boolean);if(!item.name)item.name=item.target;}
    const items=source?next.sources:next.hosts,index=items.findIndex(i=>i.id===item.id);if(index<0)items.push(item);else items[index]=item;
    if(source)for(const a of next.accounts)if(a.quotaSourceId===item.id&&(a.id!==item.accountId||a.provider!==item.provider))a.quotaSourceId=null;
    stageSettings(next);renderSettings();
  });
  $('#editor').dataset.kind=source?'source':'host';$('#editor-form button[type=submit]').textContent='更新草稿';
  if(source){
    const providerRow=$('#field-provider').closest('.form-row'),hostRow=$('#field-hostId').closest('.form-row'),pathRow=$('#field-path').closest('.form-row');providerRow.after(hostRow);hostRow.after(pathRow);
    const advanced=document.createElement('details');advanced.className='advanced';advanced.innerHTML='<summary>高级连接选项 · CLI / 代理 / 前置命令</summary>';$('#editor-fields').append(advanced);
    for(const el of [$('#field-codexBinary').closest('.form-row'),$('#field-agyBinary').closest('.form-row'),$('#remote-quota'),$('#local-proxy')])advanced.append(el);
  }else{
    const selectors=$('.host-selectors'),advanced=document.createElement('details');advanced.className='monitor-options';advanced.open=Boolean(existing);advanced.innerHTML='<summary>监控指标与设备 · 按需选择</summary>';selectors.before(advanced);advanced.append(selectors);
    const connect=$('#discover-devices');advanced.before(connect);connect.textContent='连接并读取设备';
  }
  bindPathSelection(source);

  if(!source){bindHostSelectors(item);const auth=()=>{$('#host-password').hidden=$('#field-authMode').value!=='password';$('#host-key').hidden=!$('#host-password').hidden;};$('#field-authMode').onchange=auth;auth();}
  else {
    bindProxy('source');
    const update=()=>{
      const provider=$('#field-provider').value,asAccount=$('#field-isAccount').checked;
      $('#account-fields').hidden=!asAccount;
      const accountId=$('#field-accountId').value,linked=draftSettings().sources.filter(src=>src.id!==item.id&&src.provider===provider&&src.accountId===accountId);
      $('#account-help').textContent=['agy','deepseek'].includes(provider)?'此服务通过关联账户显示限额或余额。取消关联后不会查询账户信息。':'数据源用于读取记录；关联账户后可查询限额，并汇总同一账户的用量。';
      if(asAccount&&linked.length)$('#account-help').textContent+=` 当前账户还关联 ${linked.length} 个数据源，修改账户名称或限额设置会同时生效。`;
      $('#deepseek-key').hidden=provider!=='deepseek';$('#field-agyBinary').closest('.form-row').hidden=provider!=='agy';$('#field-path').closest('.form-row').hidden=provider==='agy';
      document.querySelector('label[for=field-path]').textContent=provider==='deepseek'?'API Key 文件（可选）':'数据目录';$('#field-hostId').closest('.form-row').hidden=provider==='deepseek';
      $('#remote-quota').hidden=!$('#field-hostId').value||!asAccount;$('#local-proxy').hidden=!!$('#field-hostId').value;$('#field-codexBinary').closest('.form-row').hidden=provider!=='codex';
      $('#field-quotaSourceId').closest('.form-row').hidden=$('#field-quotaEnabled').value!=='yes';
    };
    const selectAccount=()=>{
      const provider=$('#field-provider').value,id=$('#field-accountId').value,a=draftSettings().accounts.find(a=>a.id===id&&a.provider===provider);
      $('#field-accountName').value=a?.name??'';$('#field-accountArchived').value=a?.archived?'yes':'no';$('#field-quotaEnabled').value=(a?a.quotaEnabled:['codex','claude','agy','deepseek'].includes(provider))?'yes':'no';
      const linked=draftSettings().sources.filter(s=>s.provider===provider&&s.accountId===id&&s.id!==item.id);
      $('#field-quotaSourceId').innerHTML=[['','自动 · 优先本机'],...linked.map(s=>[s.id,s.name]),[item.id,item.name||'此数据源']].map(([k,v])=>option(k,v,a?.quotaSourceId??'')).join('');update();
    };
    $('#field-isAccount').onchange=$('#field-hostId').onchange=$('#field-quotaEnabled').onchange=update;$('#field-accountId').onchange=selectAccount;
    $('#field-provider').onchange=e=>{
      const provider=e.target.value;if(!existing&&['agy','deepseek'].includes(provider))$('#field-isAccount').checked=true;if(provider==='deepseek'){$('#field-hostId').value='';$('#field-path').value='';}$('#field-apiKey').value='';
      $('#field-accountId').innerHTML=accountOptions(provider).map(([k,v])=>option(k,v,'__new__')).join('');
      if(['~/.codex','~/.claude',''].includes($('#field-path').value))$('#field-path').value=provider==='codex'?'~/.codex':provider==='claude'?'~/.claude':'';selectAccount();
    };selectAccount();
  }
}
function bindPathSelection(source){
  const input=source?$('#field-path'):$('#field-identityFile');if(!input)return;
  const row=input.closest('.form-row'),controls=document.createElement('div');controls.className='path-actions';controls.innerHTML='<button type="button" class="path-pick">选择…</button><button type="button" class="path-check">检测路径</button><span class="path-status" role="status"></span>';row.append(controls);
  const remote=()=>source&&Boolean($('#field-hostId')?.value);
  controls.querySelector('.path-pick').onclick=async()=>{try{const result=await window.__TAURI__.core.invoke('select_local_path',{directory:source&&$('#field-provider').value!=='deepseek',initial:input.value});if(result){input.value=result;input.dispatchEvent(new Event('input',{bubbles:true}));}}catch(e){controls.querySelector('.path-status').textContent=String(e);}};
  controls.querySelector('.path-check').onclick=async()=>{const status=controls.querySelector('.path-status');try{status.textContent=await window.__TAURI__.core.invoke('check_local_path',{path:input.value})?'路径存在':'路径不存在，请检查';}catch(e){status.textContent=String(e);}};
  const update=()=>{controls.querySelectorAll('button').forEach(b=>b.hidden=remote());controls.querySelector('.path-status').textContent=remote()?'远程路径 · '+(draftSettings().hosts.find(h=>h.id===$('#field-hostId').value)?.name??'SSH 主机'):'';};$('#field-hostId')?.addEventListener('change',update);update();
}
function bindHostSelectors(item) {
  const selectors=AieyesSelect;editorExtraSnapshot=()=>[item.metrics,item.details];
  let discovered={}, deviceControls=[],discoveryRevision=0;
  const discoveryIdentity=()=>JSON.stringify(['target','port','identityFile','shell','preCommand','authMode','username','password'].map(k=>$('#field-'+k)?.value));
  for(const key of ['target','port','identityFile','shell','preCommand','authMode','username','password'])$('#field-'+key)?.addEventListener('input',()=>{discoveryRevision++;discovered={};redrawDevices();$('#discovery-status').hidden=false;$('#discovery-status').textContent='连接配置已更改，请重新读取设备';});
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
        return [...rows.map(d=>({id:d.id,label:[d.id,d.type??d.name].filter(Boolean).join(' · ')})),
          ...selectors.deviceIds(tokens(),group).filter(id=>!known.has(id)).map(id=>({id,label:id,unavailable:true}))];
      };
      const control=selectors.mount(root,{title,options,selected:()=>selectors.deviceSelected(tokens(),group,options().map(o=>o.id)),all:()=>!tokens().some(s=>s.startsWith(group+':')),disabled:()=>!item.metrics.includes(group),
        onChange:(selected,{all})=>{$('#field-devices').value=selectors.writeDevices(tokens(),group,selected,all).join(', ');}});
      deviceControls.push(control);editorSelectors.push(control);
    }
  }
  const metrics=selectors.mount($('#metric-select'),{title:'采集项目',options:()=>Object.entries(groups).map(([id,label])=>({id,label})),selected:()=>item.metrics,
    onChange:values=>{item.metrics=values;deviceControls.forEach(c=>c.refresh());details.refresh();}});
  // Disabled categories retain their saved detail values; only enabled categories are editable.
  const details=selectors.mount($('#detail-select'),{title:'显示细分项',options:()=>Object.entries(detailOptions).filter(([id])=>item.metrics.includes(detailGroup[id])).map(([id,label])=>({id,label})),selected:()=>item.details,disabled:()=>!item.metrics.length,
    onChange:values=>{item.details=values;}});
  editorSelectors.push(metrics,details);redrawDevices();
  $('#field-devices').onchange=redrawDevices;
  $('#discover-devices').onclick=async()=>{
    const button=$('#discover-devices'), status=$('#discovery-status');
    button.disabled=true;button.textContent='读取中…';status.hidden=true;
    const form=$('#editor-form'), f=new FormData(form), host={...item},identity=discoveryIdentity(),revision=++discoveryRevision;
    for(const key of ['target','identityFile','shell','preCommand','authMode','username'])host[key]=f.get(key);
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
  const update=()=>{const saved=localStorage.getItem('sidebar.collapsed'),collapsed=saved==null?innerWidth<1000:saved==='true';document.body.classList.toggle('sidebar-collapsed',collapsed);toggle.setAttribute('aria-expanded',String(!collapsed));toggle.textContent=collapsed?'»':'«';};toggle.onclick=()=>{localStorage.setItem('sidebar.collapsed',String(!document.body.classList.contains('sidebar-collapsed')));update();};addEventListener('resize',update);update();
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
window.addEventListener('aieyes:appearance',()=>{if(state.page==='agent'){if(interactionOpen())pendingRender=true;else render();}});
let externalRefresh=null,externalPending=false,externalSettings=false,externalQuotaCheck=false,externalScanCheck=false;
function refreshExternalData(settings=false,method='') {
  externalPending=true;externalSettings ||= settings;externalQuotaCheck ||= method==='quotas.refresh';externalScanCheck ||= method==='sources.scan';
  if(externalRefresh)return externalRefresh;
  externalRefresh=(async()=>{
    while(externalPending){
      externalPending=false;const readSettings=externalSettings,checkQuotas=externalQuotaCheck,checkScan=externalScanCheck;externalSettings=false;externalQuotaCheck=false;externalScanCheck=false;
      if(readSettings||!state.settings)state.settings=await api('settings.get');
      await loadDashboard();
      if(readSettings&&PANEL&&state.page!=='agent')render();
      const scannedSources=(state.dashboard?.sources??[]).filter(source=>state.settings.sources.some(row=>row.id===source.id&&row.enabled&&(!row.hostId||state.settings.hosts.some(host=>host.id===row.hostId&&host.enabled)))).map(source=>({id:source.id,error:source.status?.error}));
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
    else state.sharedEstimateOperations.delete(payload.operationId);
  });
  await events.listen('desktop:refresh-status',({payload})=>{
    if(!refreshLabels[payload.key])return;
    const previous=state.refreshState[payload.key]??{};
    if(payload.busy)state.refreshState[payload.key]={...previous,busy:true};
    else {if(payload.key==='hosts')state.hostRefreshItem=payload.itemId??null;state.refreshState[payload.key]={...previous,busy:false,success:payload.success??previous.success};showFailures(payload.failures?.length?{failures:payload.failures}:null,payload.key,payload.itemId??null);}
    updateActivity();
  });
  await events.listen('desktop:status',({payload})=>{
    state.panelOpen=Boolean(payload.panelOpen);state.panelPage=payload.panelPage??state.panelPage;
  });
  await events.listen('desktop:settings',()=>{window.AieyesNetwork?.invalidate();if(PANEL)return refreshExternalData(true);});
  await events.listen('desktop:hosts',({payload})=>{applyHostSamples(state.hostRefreshItem?[...state.hosts.filter(row=>row.id!==state.hostRefreshItem),...payload]:payload);});
  await events.listen('desktop:hosts-error',({payload})=>{applyHostFailure(payload,state.hostRefreshItem);});
  await events.listen('desktop:data-changed',({payload})=>{
    if(PANEL&&payload==='settings.save')return; // desktop:settings owns this invalidation.
    if(PANEL||(!state.busy&&!state.settingsSaving&&!state.settingsCommitting&&!editorSaving))return refreshExternalData(payload==='settings.save',payload);
  });
}
async function boot(){
  try{
    if(!window.__TAURI__)throw new Error('通过 Aieyes 桌面应用打开');
    await listenForSharedState();
    const info=await api('hello'),version=$('#app-version');if(version)version.textContent=info.version;
    state.settings=await api('settings.get');await loadDashboard();
    if(PANEL){render();updateActivity();requestAnimationFrame(()=>{try{$('.panel-body').scrollTop=Number(localStorage.getItem('aieyes.panel.scroll')||0);}catch(_){}});return;}await scan();if(hasQuotaSources())await quotas();
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
if(!PANEL)setInterval(()=>{
  if(!state.settings)return;
  if(!state.serverBusy&&state.settings.hosts.some(h=>h.enabled)&&Date.now()-state.lastMetrics>((state.page==='servers'&&!document.hidden)||(state.panelOpen&&state.panelPage==='servers')?2000:state.settings.serverRefreshSeconds*1000)){sample();}
  if(state.settingsCommitting||window.AieyesUpdates?.busy||$('#editor').open||state.estimateBusy||state.sharedEstimateOperations?.size||$('#quota-dialog')?.dataset.busy||state.busy||state.settingsSaving)return;
  const expired=state.dashboard?.quotas.flatMap(q=>q.windows.map(w=>({q,w,key:q.provider+':'+q.accountId+':'+w.resetsAt}))).find(({w,key})=>w.resetsAt&&w.resetsAt*1000<=Date.now()&&!resetConfirmations.has(key));
  if(expired){resetConfirmations.add(expired.key);quotas(expired.q.accountId);return;}
  if(hasQuotaSources()&&(!state.lastQuota||Date.now()-state.lastQuota>state.settings.refreshSeconds*1000)){quotas();return;}
  if(Date.now()-state.lastScan>state.settings.refreshSeconds*1000){scan();return;}
},500);
// Age visible samples even while an RPC is slow or the panel receives no new events.
setInterval(updateHostStatuses,1000);
function updateResetDisplays(now=Date.now()/1000) { for(const el of document.querySelectorAll('[data-reset-at]'))el.textContent=AieyesUI.quotaReset({resetsAt:Number(el.dataset.resetAt),windowMinutes:Number(el.dataset.windowMinutes)},now); }
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
  state.estimateOperation={key:dialog.dataset.estimateKey,kind:dialog.dataset.estimateKind,status:'running'};
  let background=$('#background-task');if(!background){background=document.createElement('button');background.id='background-task';background.setAttribute('role','status');document.body.append(background);}background.hidden=false;background.onclick=()=>{if(state.estimateBusy||state.estimateOperation?.status==='error'){if(!dialog.open)dialog.showModal();}else if(state.estimateOperation?.key){openQuotaEstimate(state.estimateOperation.key,state.estimateOperation.kind);background.hidden=true;}else{openSampling();background.hidden=true;}};
  let progress=dialog.querySelector('#quota-progress');if(!progress){progress=document.createElement('p');progress.id='quota-progress';progress.setAttribute('role','status');dialog.querySelector('.quota-dialog-body').append(progress);}
  let stage='等待当前刷新';const update=()=>{progress.textContent=stage+' · '+Math.floor((Date.now()-started)/1000)+' 秒 · 可关闭窗口，后台继续';background.textContent='后台处理中 · '+stage+' · 查看';};update();const timer=setInterval(update,1000);
  let unlisten=()=>{};
  try{unlisten=await window.__TAURI__.event.listen('operations:progress',({payload})=>{if(payload.operationId===operationId){stage=payload.stage;update();}});await api(action,{...params,operationId});stage='读取计算结果';update();await loadDashboard();state.estimateOperation.status='success';background.textContent='后台操作已完成 · 查看结果';return true;}
  catch(error){state.estimateOperation.status='error';background.textContent='后台操作失败 · 查看并重试';$('#quota-tool-error').textContent=error.message??String(error);$('#quota-tool-error').hidden=false;await loadDashboard().catch(()=>{});return false;}
  finally{clearInterval(timer);unlisten();progress.remove();controls.forEach(([el,disabled])=>el.disabled=disabled);dialog.oncancel=null;delete dialog.dataset.busy;state.estimateBusy=false;}
}
function refreshQuotaDialog(){
  const dialog=$('#quota-dialog'),key=dialog?.dataset.estimateKey;
  if(!dialog?.open||!key||dialog.dataset.busy)return;
  const kind=dialog.dataset.estimateKind??'weekly';
  const records=JSON.stringify((state.dashboard[kind==='credits'?'creditEstimates':'quotaEstimates']??[]).filter(e=>e.accountKey===key));
  if(records===dialog.dataset.records)return;
  const expanded=dialog.querySelector('details')?.open,focus=document.activeElement?.id;
  openQuotaEstimate(key,kind);
  if(expanded&&dialog.querySelector('details'))dialog.querySelector('details').open=true;
  if(focus)document.getElementById(focus)?.focus({preventScroll:true});
}
function hasActiveFilters(){return Boolean(state.provider||state.accountKey||state.sourceId||state.model||state.days!==1);}
function bindQuotaTools(){
  renderQueryStatus();
  $('#content > .active-filters')?.remove();$('#content > .panel-filter-summary')?.remove();
  if(hasActiveFilters()) {
    const filters=document.createElement('div');filters.className='active-filters';
    for(const [key,label] of [['provider',providers[state.provider]],['accountKey',state.accountKey==='none'?'未关联账户':state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey)?.name??state.accountKey],['sourceId',state.settings.sources.find(s=>s.id===state.sourceId)?.name??state.sourceId],['model',state.model]]) {
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
  document.querySelectorAll('[data-estimate]').forEach(b=>b.onclick=()=>openQuotaEstimate(b.dataset.estimate));
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
    $('#quota-order-list').innerHTML=keys.map((key,i)=>`<div class="quota-order-row" draggable="true" data-order="${escapeHTML(key)}"><span aria-hidden="true">${uiIcon('grip')}</span><span class="order-account">${providerMark(key.split(':')[0])}${escapeHTML(state.settings.accounts.find(a=>a.provider+':'+a.id===key)?.name??key)}</span><button type="button" data-direction="-1" aria-label="上移账户" ${i===0?'disabled':''}>${uiIcon('up')}</button><button type="button" data-direction="1" aria-label="下移账户" ${i===keys.length-1?'disabled':''}>${uiIcon('down')}</button></div>`).join('');
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
function openQuotaEstimate(key,kind="weekly"){
  if(state.estimateBusy){const dialog=$('#quota-dialog');if(dialog&&!dialog.open)dialog.showModal();return;}
  const credits=kind==='credits';
  const q=state.dashboard.quotas.find(q=>q.provider+':'+q.accountId===key);if(!q)return;
  const records=(state.dashboard[credits?'creditEstimates':'quotaEstimates']??[]).filter(e=>e.accountKey===key),current=records.find(e=>e.status!=='completed');
  const allFive=q.windows.filter(w=>w.windowMinutes===300),allWeek=q.windows.filter(w=>w.windowMinutes===10080),windows=allFive.length===1?allFive:q.provider==='claude'&&allFive.some(w=>w.id==='five_hour')?allFive.filter(w=>w.id==='five_hour'):allWeek.length===1?allWeek:allWeek.filter(w=>q.provider==='claude'&&w.id==='seven_day'),eligible=credits?(q.provider==='codex'&&q.credits?.balance!=null&&!q.credits.unlimited):q.provider!=='agy'&&windows.length>0;
  const sources=state.settings.sources.filter(s=>s.enabled&&s.provider===q.provider&&s.accountId===q.accountId&&!['agy','deepseek'].includes(s.provider));
  const confirmation='<label class="quota-confirm"><input type="checkbox" id="estimate-confirm">'+(credits?'我确认本段仅消耗 credits，所有设备的用量均已纳入；不混用包含额度、API 或中转。':'我确认采样期间只使用目标订阅，所有设备用量均已纳入所选来源；不混用 API / 中转。')+'</label>';
  const result=e=>`<div class="estimate-result"><div class="between"><strong>${estimateStatus(e)}</strong><strong>${credits?`${creditValue(e)}`:e.valuationMode==='fiveHour'?'5h / 7d':e.weeklyValue!=null?money(e.weeklyValue)+' USD':'—'}</strong></div>${e.valuationMode==='fiveHour'?estimateValues(e):''}<p class="muted">${escapeHTML(e.calculationNote)}</p>${e.originalEstimateId?'<p class="muted">已修正 · 原采样记录保留</p>':e.status==='completed'&&!AieyesUI.estimateEntries([e],credits).length?`<button type="button" data-estimate-repair="${escapeHTML(e.id)}">按原记录修复估值</button>`:''}<details><summary>计算依据</summary><p>${date(e.startedAt)} → ${date(e.checkpointAt)}<br>${escapeHTML(e.windowName)} · 消耗 ${credits?creditAmount(e.consumedCredits)+' credit':pct(e.consumedPercent)} · 样本成本 ${money(e.cost)} USD<br>计价 Token：${compact(e.pricedTokens)} / ${compact(e.totalTokens)}<br>${escapeHTML(e.sourceNames.join('、'))}</p><p class="tiny muted">${credits?'1000 credit 的 API 等价价值 = 样本成本 × 1000 ÷ 消耗 credit':e.valuationMode==='fiveHour'?'5h / 7d 同期价值 = 有效成本 × 100 ÷ 对应消耗百分点；7d 倍率估算 = 5h 价值 × 近期容量倍率':'整周估值 = 样本 API 等价成本 × 100 ÷ 消耗百分点'}；结束后价格与倍率固定。</p>${(e.segments??[]).length?`<p class="tiny muted">已保存 ${e.segments.length} 个有效片段</p>`:''}${e.reason?`<p class="error">${escapeHTML(e.reason)}</p>`:''}${(e.prices??[]).length?`<p class="tiny muted">价格快照：${e.prices.map(p=>escapeHTML(p.id)+' · '+date(p.fetchedAt)).join('；')}</p>`:''}</details></div>`;
  const setup=eligible&&sources.length?`${credits?'':`<label>主要额度池<select id="estimate-window">${windows.filter(w=>windows.length===1||w.id==='seven_day'||w.name==='7d').map(w=>option(w.id||w.name,w.name,'')).join('')}</select></label>`}<p>纳入的用量来源</p>${sources.map(s=>`<label class="quota-confirm"><input type="checkbox" name="estimate-source" value="${escapeHTML(s.id)}" checked>${escapeHTML(s.name)}</label>`).join('')}${confirmation}<p class="tiny muted">建议开始后新建会话。至少消耗 ${credits?'5 credits':'5 个百分点'}后输出估值；跨采样边界的累计用量会使本次结果不可用。</p><button class="primary" id="estimate-start" disabled>开始采样</button>`:'<p class="muted">需要可采集 Token 的关联数据源及可靠的额度池映射。agy 限额查询本身不提供用量历史，此额度池暂不支持估值。</p>';
  quotaDialog(q.name+(credits?' · credit 价值':' · 5h / 7d 额度价值'),`<p class="muted">按本次模型组合的 API 等价成本与额度消耗比例估算，不是可兑换余额。</p>${current?result(current)+(current.status==='pending'?confirmation+'<button id="estimate-restart" disabled>确认并开始新一段</button>':'')+`<button class="primary" id="estimate-stop">${current.status==='pending'?'结束并保留有效段':'结束并保存本段结果'}</button>`:setup}${records.some(e=>e.status==='completed')?'<h3>采样历史</h3>'+records.filter(e=>e.status==='completed').map(result).join(''):''}`);
  $('#quota-dialog').dataset.estimateKind=kind;$('#quota-dialog').dataset.estimateKey=key;$('#quota-dialog').dataset.records=JSON.stringify(records);
  const confirm=$('#estimate-confirm');if(confirm)confirm.onchange=()=>{const b=$('#estimate-start')??$('#estimate-restart');if(b)b.disabled=!confirm.checked;};
  const act=async(action,params)=>{if(await quotaAction((credits?'creditEstimates.':'quotaEstimates.')+action,params)){if($('#quota-dialog').open)openQuotaEstimate(key,kind);}else {const message=$('#quota-tool-error').textContent;refreshQuotaDialog();$('#quota-tool-error').textContent=message;$('#quota-tool-error').hidden=false;}};
  document.querySelectorAll('[data-estimate-repair]').forEach(button=>button.onclick=()=>act('repair',{id:button.dataset.estimateRepair}));
  if($('#estimate-start'))$('#estimate-start').onclick=()=>act('start',{accountKey:key,windowId:credits?'credits':$('#estimate-window').value,sourceIds:[...document.querySelectorAll('[name=estimate-source]:checked')].map(el=>el.value),confirmed:confirm.checked});
  if($('#estimate-stop'))$('#estimate-stop').onclick=()=>act('stop',{id:current.id});
  if($('#estimate-restart'))$('#estimate-restart').onclick=()=>act('restart',{id:current.id,confirmed:confirm.checked});
}

const appReady=boot();
window.AieyesApp.ready=appReady;
