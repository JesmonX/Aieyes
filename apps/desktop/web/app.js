const $ = (s) => document.querySelector(s);
/* The floating window expands into a compact panel that mirrors the macOS menu bar
   popover; the same renderers feed both surfaces. */
const PANEL = window.AIEYES_PANEL === true;
const escapeHTML = (s) => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const compact = n => n >= 1e9 ? (n / 1e9).toFixed(2) + 'B' : n >= 1e6 ? (n / 1e6).toFixed(2) + 'M' : n >= 1e3 ? (n / 1e3).toFixed(1) + 'K' : Math.round(n || 0).toString();
const money = n => '$' + (n || 0).toFixed(2);
const pct = n => n == null ? '—' : n.toFixed(1) + '%';
const bytes = n => { if (n == null) return '—'; let i = 0; while (n >= 1024 && i < 4) { n /= 1024; i++; } return n.toFixed(i ? 1 : 0) + [' B',' KiB',' MiB',' GiB',' TiB'][i]; };
const speed = n => n == null ? '—' : bytes(n) + '/s';
const date = n => n ? new Date(n * 1000).toLocaleString('zh-CN', {month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'}) : '—';
const providers = {codex:'Codex',claude:'Claude Code',antigravity:'Antigravity',agy:'agy',deepseek:'DeepSeek',custom:'自定义'};
const palette = ['#6575ed','#39a8a0','#a879d5','#e3a159','#d875a6','#4bbbd0','#7ca768','#817bca'];
const color = name => { let hash=2166136261; for(const c of new TextEncoder().encode(name))hash=Math.imul(hash^c,16777619)>>>0; return `hsl(${(hash%3600)/10} 57% 50%)`; };
const groups = {cpu:'CPU',memory:'内存',gpu:'GPU',filesystems:'文件系统',disk:'磁盘 I/O',network:'网络'};
const detailOptions={cpuTimes:'CPU 时间分布',memoryCache:'内存缓存 / Buffer',swap:'Swap',fsAvailable:'文件系统可用空间',fsType:'文件系统类型 / 设备',inodes:'inode',diskIops:'磁盘 IOPS',diskBusy:'磁盘忙碌率',networkTotals:'累计流量',networkErrors:'网络错误 / 丢包',gpuMemory:'GPU 显存',gpuThermals:'GPU 温度 / 功耗'};
const state = {page:'agent',settingsTab:'sources',priceSearch:'',settings:null,dashboard:null,hosts:[],prices:[],provider:'',sourceId:'',accountKey:'',model:'',days:1,cost:false,busy:false,settingsSaving:false,serverBusy:false,lastScan:0,lastMetrics:0,lastQuota:0,lastSuccessfulUpdate:0,quotaBusy:false,quotaError:'',dashboardRequest:0,panelOpen:false,panelPage:'agent'};
async function api(method, params = {}) { return window.__TAURI__.core.invoke('engine_call', {method,params}); }
function notify(message, kind = 'info') { const el=$('#message');el.textContent=message;el.hidden=!message;el.dataset.kind=kind; }
const timeLabel = stamp => new Date(stamp).toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'});
// Keep the user's place when live values replace their markup.
function focusSelector(element) {
  if(!element||element===document.body)return null;
  if(element.id)return '#'+CSS.escape(element.id);
  const attributes=['data-estimate','data-page','data-settings-tab','data-edit','data-remove','data-price','data-edit-account','data-remove-account','data-restore-account','data-enable','data-gap-map','data-gap-price'];
  for(const attribute of attributes)if(element.hasAttribute(attribute))return `[${attribute}="${CSS.escape(element.getAttribute(attribute))}"]`;
  if(element.tagName==='SUMMARY')for(const attribute of ['data-agent-detail','data-metric'])if(element.parentElement.hasAttribute(attribute))return `[${attribute}="${CSS.escape(element.parentElement.getAttribute(attribute))}"] > summary`;
  return null;
}
function rememberView() {
  return {focus:focusSelector(document.activeElement),window:[window.scrollX,window.scrollY],panel:$('.panel-body')?.scrollTop,
    opened:[...document.querySelectorAll('#content [data-agent-detail][open],#content [data-metric][open]')].map(el=>[el.hasAttribute('data-metric')?'data-metric':'data-agent-detail',el.dataset.metric??el.dataset.agentDetail]),
    scrollers:[...document.querySelectorAll('#content .daily-table,#content .prices-list')].map(el=>({className:el.className,top:el.scrollTop,left:el.scrollLeft}))};
}
function restoreView(view) {
  for(const [attribute,value] of view.opened){const el=document.querySelector(`[${attribute}="${CSS.escape(value)}"]`);if(el)el.open=true;}
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
function quotaSection(d) {
  const quotas=d.quotas??[],missing=state.settings.accounts.filter(a=>!a.archived&&a.quotaEnabled&&(!state.provider||a.provider===state.provider)&&(!state.accountKey||state.accountKey===a.provider+':'+a.id)&&!quotas.some(q=>q.provider===a.provider&&q.accountId===a.id));
  if(!quotas.length&&!missing.length)return '';
  return `<div class="section-head"><h2>账户限额 <span class="count">${quotas.length+missing.length}</span></h2><button id="order-quotas" title="调整账户顺序">排序</button><button id="read-quotas" ${state.quotaBusy?'disabled':''} title="重新查询各账户的实时限额">${state.quotaBusy?'读取中…':'刷新限额'}</button></div><div class="quotas">${quotas.map(quotaCard).join('')}${missing.map(a=>{
    const enabled=state.settings.sources.some(src=>src.enabled&&src.provider===a.provider&&src.accountId===a.id&&(!src.hostId||state.settings.hosts.some(h=>h.id===src.hostId&&h.enabled)));
    const text=!enabled?'关联并启用数据源后可读取限额':state.quotaBusy?'正在读取账户限额…':state.quotaError?'读取失败，可重试刷新限额':'尚未读取限额，可点击刷新限额';
    return `<div class="card quota-placeholder"><h3>${escapeHTML(a.name)}</h3><p class="muted" role="status">${text}</p>${enabled&&state.quotaError?`<p class="error">${escapeHTML(state.quotaError)}</p>`:''}</div>`;
  }).join('')}</div>`;
}
async function job(label, fn) {
  if (state.busy || state.settingsSaving) return false;
  state.busy=true;$('#activity').textContent=label;$('#activity').dataset.status='busy';
  const buttons=[...document.querySelectorAll('header button')].map(button=>[button,button.disabled]);
  buttons.forEach(([button])=>button.disabled=true);
  let outcome='error';const previousMessage=$('#message').textContent;
  try { await fn();outcome='success';state.lastSuccessfulUpdate=Date.now();if($('#message').dataset.kind==='error'&&$('#message').textContent===previousMessage)notify('');return true; }
  catch (error) { outcome=error.partial?'partial':'error';notify(error.message??String(error),'error');return false; }
  finally {
    state.busy=false;$('#activity').dataset.status=outcome;
    $('#activity').textContent=outcome==='success'?'更新于 '+timeLabel(state.lastSuccessfulUpdate):(outcome==='partial'?'部分失败':'操作失败')+(state.lastSuccessfulUpdate?' · 上次成功 '+timeLabel(state.lastSuccessfulUpdate):'');
    buttons.forEach(([button,disabled])=>button.disabled=disabled);
  }
}
async function loadDashboard() {
  if(state.accountKey && state.accountKey!=='none' && !state.settings.accounts.some(a=>a.provider+':'+a.id===state.accountKey))state.accountKey='';
  if(state.sourceId && !state.settings.sources.some(s=>s.id===state.sourceId))state.sourceId='';
  const account=state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey);
  const request=++state.dashboardRequest;
  const dashboard = await api('dashboard',{provider:account?.provider || state.provider || null,accountId:state.accountKey==='none'?'':account?.id ?? null,sourceId:state.sourceId || null,model:state.model || null,days:state.days});
  if(request!==state.dashboardRequest)return;
  state.dashboard=dashboard;
  if (state.page === 'agent') render();
  refreshQuotaDialog();
}
function batchError(rows,label) {
  if(!Array.isArray(rows))return null;
  const failures=rows.filter(row=>row.error);if(!failures.length)return null;
  const partial=failures.length<rows.length;
  const error=new Error(`${label}${partial?'部分失败':'失败'}：${failures.map(row=>`${row.name||row.id||row.accountId||''}${row.name||row.id||row.accountId?' · ':''}${row.error}`).join('；')}`);
  error.partial=partial;return error;
}
async function scan() {
  await job('同步记录',async()=>{try{const rows=await api('sources.scan');await loadDashboard();const error=batchError(rows,'同步记录');if(error)throw error;}finally{state.lastScan=Date.now();}});
}
function hasQuotaSources() {
  return state.settings.accounts.some(a => !a.archived && a.quotaEnabled && state.settings.sources.some(s => s.enabled && s.provider === a.provider && s.accountId === a.id && (!s.hostId || state.settings.hosts.some(h => h.id === s.hostId && h.enabled))));
}
async function quotas() {
  if(state.busy||state.settingsSaving)return;
  state.quotaBusy=true;state.quotaError='';if(state.page==='agent')render();
  try { await job('读取限额',async()=>{try{const rows=await api('quotas.refresh');await loadDashboard();const error=batchError(rows,'读取限额');if(error)throw error;}catch(error){state.quotaError=String(error);throw error;}finally{state.lastQuota=Date.now();}}); }
  finally {state.quotaBusy=false;if(state.page==='agent')render();}
}
async function syncPrices(){await job('同步价格',async()=>{await api('prices.sync');state.prices=await api('prices.list');await loadDashboard();if(state.page==='settings')renderSettings();notify('价格已更新');});}
let lastHostError='';
function applyHostSamples(rows) {
  const attemptedAt=Date.now();
  state.hosts=rows.map(row=>({...row,lastAttemptAt:row.lastAttemptAt??attemptedAt,
    ...(row.error?{sample:row.sample??state.hosts.find(previous=>previous.id===row.id)?.sample}:{})}));
  if(!rows.some(row=>row.error)&&lastHostError){if($('#message').textContent===lastHostError)notify('');lastHostError='';}
  state.lastMetrics=attemptedAt;if(state.settings&&state.page==='servers')render();
}
function applyHostFailure(error) {
  const message=error.message??String(error),attemptedAt=Date.now();
  const rows=(state.settings?.hosts??[]).map(host=>{
    const previous=state.hosts.find(row=>row.id===host.id)??{id:host.id};
    return host.enabled?{...previous,error:message,lastAttemptAt:attemptedAt}:previous;
  });
  applyHostSamples(rows);lastHostError=message;notify(message,'error');
}
async function sample() {
  if(state.serverBusy)return;state.serverBusy=true;
  try{applyHostSamples(await api('hosts.sample'));}
  catch(error){applyHostFailure(error);}
  finally{state.serverBusy=false;}
}
async function saveSettings(next) {
  if (state.settingsSaving) throw new Error('设置正在保存，请稍后重试');
  state.settingsSaving = true;
  try {
    await api('settings.save', next);
    state.settings = next;
    state.lastQuota = 0;
  } finally { state.settingsSaving = false; }
  notify('已保存');
  try { await loadDashboard(); } catch (error) { notify('设置已保存，概览刷新失败：' + String(error)); }
}
function option(value,label,current) { return `<option value="${escapeHTML(value)}" ${String(value) === String(current) ? 'selected' : ''}>${escapeHTML(label)}</option>`; }
function stat(label,value,detail,icon) {
  const paths={'✧':'M12 3 9 9 3 12l6 3 3 6 3-6 6-3-6-3Z','▱':'M4 7h16v10H4ZM8 10v4m4-4v4m4-4v4','↗':'M5 19 19 5M8 5h11v11','$':'M12 2v20m5-16H9a4 4 0 0 0 0 8h6a4 4 0 0 1 0 8H6'};
  icon=`<svg viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[icon]??paths['✧']}"/></svg>`;
  return `<div class="stat"><div class="stat-label"><span>${label}</span><b>${icon}</b></div><div class="stat-value">${value}</div>${detail?`<small>${detail}</small>`:''}</div>`; }
function render() {
  document.querySelectorAll('[data-page]').forEach(b=>{const active=b.dataset.page===state.page;b.classList.toggle('active',active);if(active)b.setAttribute('aria-current','page');else b.removeAttribute('aria-current');});
  const title=$('#title');if(title)title.textContent = {agent:'使用概览',servers:'服务器',settings:'设置'}[state.page];
  for(const id of ['scan','quota','live-sessions']){const el=$('#'+id);if(el)el.hidden=state.page!=='agent';}
  if (!state.settings) return;
  if (state.page === 'agent') renderAgent(); else if (state.page === 'servers') renderServers(); else renderSettings();
}
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
  const s=d.summary,t=s.tokens,models=d.models,sum=models.reduce((n,m)=>n+(state.cost?m.cost:m.total),0);
  let cursor=0;
  const stops=models.map(m=>{const start=cursor;cursor+=sum?(state.cost?m.cost:m.total)/sum*360:0;return `${color(m.key)} ${start}deg ${cursor}deg`;});
  const maxHeat=Math.max(1,...d.heatmap.map(v=>state.cost?v.cost:v.total));
  const lead=d.heatmap.length?(new Date(d.heatmap[0].key+'T12:00:00').getDay()+6)%7:0;
  const account=state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey);
  const sources=state.settings.sources.filter(src=>{const ids=d.sources.find(s=>s.id===src.id)?.accountIds??[src.accountId];return (!state.provider||src.provider===state.provider)&&(!state.accountKey||(state.accountKey==='none'?ids.includes(''):src.provider===account?.provider&&ids.includes(account?.id)));});
  const trendModels=[...new Set(d.dayModels.map(r=>r.model))].sort();
  $('#title').textContent='Agent 概览';
  $('#content').innerHTML=`${state.settings.sources.length?'':onboarding()}<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','未关联账户',state.accountKey)}${state.settings.accounts.filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select><select id="source" aria-label="数据源">${option('','全部数据源',state.sourceId)}${sources.map(s=>option(s.id,s.name,state.sourceId)).join('')}</select><select id="model" aria-label="模型">${option('','全部模型',state.model)}${[...new Set([...(d.modelOptions??trendModels),...(state.model?[state.model]:[])])].map(m=>option(m,m,state.model)).join('')}</select></div>
  ${quotaSection(d)}
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':`最近 ${n} 天`,state.days)).join('')}</select></div>
  <div class="stats">${stat('总 Token',compact(s.total),'','✧')}${stat('API 等价成本'+(s.total>0&&s.pricedTokens<s.total?' <button id="repair-pricing" class="pricing-alert" aria-label="计价未完成，设置模型价格" title="部分 Token 尚未计价">!</button>':''),s.pricedTokens?money(s.cost):'—','','$')}${stat('缓存命中率',pct(cacheRate(t)),'','▱')}<div class="stat token-card"><div class="stat-label">Token 明细</div><div class="token-breakdown">${[['输入',t.input],['输出',t.output],['缓存',t.cacheRead+t.cacheWrite]].map(([label,value])=>`<div><span>${label}</span><strong>${compact(value)}</strong></div>`).join('')}</div></div></div>
  <div class="section-head"><h2>${state.days===1?'近 7 天趋势':'使用趋势'}</h2><label class="mode"><input type="checkbox" id="cost-mode" ${state.cost?'checked':''}> 按 API 等价成本</label></div>
  <div class="chart-row"><div class="card"><h2>每日用量 · 按模型</h2><canvas id="trend" aria-label="按模型堆叠的每日用量，下方有逐日数据"></canvas><div class="model-key">${trendModels.map(m=>`<span><i style="background:${color(m)}"></i>${escapeHTML(m)}</span>`).join('')}</div></div><div class="card"><h2>所选范围 · 模型分布</h2><div class="pie-wrap"><div class="donut" style="background:conic-gradient(${stops.length&&sum?stops.join(','):'var(--border) 0deg 360deg'})"><div class="donut-inner">${models.length}<small>模型</small></div></div><div class="legend">${models.map(m=>`<div><span class="color" style="background:${color(m.key)}"></span><span class="model">${escapeHTML(m.key)}</span><strong>${state.cost?money(m.cost):compact(m.total)}</strong></div>`).join('')}</div></div></div></div>
  <div class="card"><details class="daily" data-agent-detail="daily"><summary><h2>每日模型与缓存命中率</h2></summary>${dailyTable(d.trendDays,d.dayModels)}</details></div>
  <div class="card"><h2>过去 365 天</h2><div class="heatmap">${'<span></span>'.repeat(lead)}${d.heatmap.map(day=>{const n=state.cost?day.cost:day.total;return `<span style="background:${n?`color-mix(in srgb,var(--accent) ${20+80*Math.sqrt(n/maxHeat)}%,transparent)`:'var(--border)'}" title="${day.key} · ${state.cost?money(n):compact(n)+' Token'}"></span>`;}).join('')}</div><div class="heat-foot"><span>${d.heatmap[0]?.key??''}</span><span>少 ░ ▒ ▓ 多</span></div></div>
  `;
  bindSetup();
  $('#provider').onchange=async e=>{state.provider=e.target.value;state.accountKey='';state.sourceId='';state.model='';await loadDashboard();};
  $('#account').onchange=async e=>{state.accountKey=e.target.value;state.sourceId='';await loadDashboard();};
  $('#source').onchange=async e=>{state.sourceId=e.target.value;await loadDashboard();};
  $('#model').onchange=async e=>{state.model=e.target.value;await loadDashboard();};
  $('#days').onchange=async e=>{state.days=Number(e.target.value);await loadDashboard();};
  $('#cost-mode').onchange=e=>{state.cost=e.target.checked;renderAgent();};
  if($('#read-quotas'))$('#read-quotas').onclick=quotas;
  bindQuotaTools();
  if($('#repair-pricing'))$('#repair-pricing').onclick=openPricing;
  shortenEmptyUsage(d);restoreView(view);
  requestAnimationFrame(drawTrend);
}
function renderAgentPanel(d) {
  const view=rememberView(),s=d.summary,t=s.tokens,trendTitle=`近 ${Math.max(7,state.days)} 天用量`;
  $('#content').innerHTML=`${state.settings.sources.length?'':onboarding()}<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','未关联账户',state.accountKey)}${state.settings.accounts.filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select></div>
  ${quotaSection(d)}
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':`最近 ${n} 天`,state.days)).join('')}</select></div>
  <div class="stats panel-stats">${stat('总 Token',compact(s.total),'','✧')}${stat('API 等价成本'+(s.total>0&&s.pricedTokens<s.total?' <button id="repair-pricing" class="pricing-alert" aria-label="计价未完成，设置模型价格" title="部分 Token 尚未计价">!</button>':''),s.pricedTokens?money(s.cost):'—','','$')}</div>
  <div class="card panel-cache"><div class="between"><span class="muted">缓存命中率</span><strong>${pct(cacheRate(t))}</strong></div><div class="token-breakdown panel-token-breakdown">${[['输入',t.input],['输出',t.output],['缓存',t.cacheRead+t.cacheWrite]].map(([label,value])=>`<div><span>${label}</span><strong>${compact(value)}</strong></div>`).join('')}</div></div>
  <div class="card"><h2>${trendTitle}</h2><canvas id="trend" class="panel-chart" aria-label="${trendTitle}，按模型的每日用量"></canvas><div class="model-key">${[...new Set(d.dayModels.map(r=>r.model))].sort().map(m=>`<span><i style="background:${color(m)}"></i>${escapeHTML(m)}</span>`).join('')}</div><details class="daily" data-agent-detail="daily"><summary><h2>每日明细</h2></summary>${dailyTable(d.trendDays,d.dayModels)}</details></div>
  <button id="open-detail" class="panel-wide" title="打开完整详情窗口">用量详情</button>`;
  $('#provider').onchange=async e=>{state.provider=e.target.value;state.accountKey='';state.sourceId='';state.model='';await loadDashboard();};
  $('#account').onchange=async e=>{state.accountKey=e.target.value;state.sourceId='';await loadDashboard();};
  $('#days').onchange=async e=>{state.days=Number(e.target.value);await loadDashboard();};
  if($('#read-quotas'))$('#read-quotas').onclick=quotas;
  bindQuotaTools();
  if($('#repair-pricing'))$('#repair-pricing').onclick=openPricing;
  $('#open-detail').onclick=()=>window.__TAURI__.core.invoke('desktop_action',{action:'open'}).catch(e=>notify(String(e)));
  bindSetup();shortenEmptyUsage(d);restoreView(view);
  requestAnimationFrame(drawTrend);
}
function shortenEmptyUsage(d) {
  if(d.summary.total||(d.trendDays??[]).some(day=>day.total)||(d.heatmap??[]).some(day=>day.total))return;
  const trend=$('#trend'),chart=trend?.closest('.chart-row')??trend?.closest('.card');
  if(chart){const empty=document.createElement('div');empty.className='card empty-usage';empty.innerHTML='<h2>暂无用量记录</h2><p class="muted">'+(state.settings.sources.length?'当前筛选范围没有记录。可调整筛选，或同步记录后再查看。':'添加数据源后，使用趋势和模型明细会显示在这里。')+'</p>';chart.replaceWith(empty);}
  $('#cost-mode')?.closest('.section-head')?.remove();
  $('#content .daily')?.closest('.card')?.remove();
  $('#content .heatmap')?.closest('.card')?.remove();
}
function dailyTable(days,rows) {
  return `<div class="daily-table"><div class="day-head"><span>日期 / 模型</span><span>Token</span><span>缓存命中率</span><span>已计价成本</span></div>${[...days].reverse().map(day=>`<details class="day" data-agent-detail="day:${escapeHTML(day.key)}"><summary><span>${day.key}</span><strong>${compact(day.total)}</strong><span>${pct(cacheRate(day.tokens))}</span><span>${day.pricedTokens?money(day.cost):'—'}</span></summary>${rows.filter(r=>r.day===day.key).map(r=>`<div class="day-model"><span><i style="background:${color(r.model)}"></i>${escapeHTML(r.model)}</span><span>${compact(r.usage.total)}</span><span>${pct(cacheRate(r.usage.tokens))}</span><span>${r.usage.pricedTokens?money(r.usage.cost):'—'}</span></div>`).join('')||'<p class="muted tiny">当日暂无记录</p>'}</details>`).join('')}</div>`;
}
function quotaCard(q) {
  const key=q.provider+':'+q.accountId, latest=(state.dashboard?.quotaEstimates??[]).find(e=>e.accountKey===key);
  const windowHTML=w=>`<div class="quota-window"><div class="between tiny"><span>${escapeHTML(w.name)}</span><strong>剩余 ${pct(Math.max(0,100-w.usedPercent))}</strong></div>${resourceBar(100-w.usedPercent,w.name+'剩余额度')}<div class="tiny muted quota-reset">重置 ${date(w.resetsAt)}${w.resetsAt&&w.resetsAt*1000<=Date.now()?' · 等待同步':''}</div></div>`;
  let windows=q.windows.map(windowHTML).join('');
  if(q.provider==='agy'){
    const groups=new Map();for(const w of q.windows){const group=w.groupName||w.name.split(' · ').slice(0,-1).join(' · ');if(!groups.has(group))groups.set(group,[]);groups.get(group).push(w);}
    windows=`<details class="agy-details" data-agent-detail="agy:${escapeHTML(key)}"><summary>模型组额度 <span class="muted tiny">展开重置时间</span></summary></details><div class="agy-groups">${[...groups].map(([group,rows])=>`<div class="agy-group"><strong>${escapeHTML(group)}</strong><div class="agy-windows">${rows.sort((a,b)=>(a.windowMinutes??0)-(b.windowMinutes??0)).map(w=>windowHTML({...w,name:w.windowMinutes===10080?'7d':w.windowMinutes===300?'5h':w.name})).join('')}</div></div>`).join('')}</div>`;
  }
  return `<div class="card quota-card" data-quota="${escapeHTML(key)}"><div class="quota-title"><div><h3>${escapeHTML(q.name)}</h3><div class="sub">${escapeHTML(providers[q.provider]??q.provider)}${q.plan?' · '+escapeHTML(q.plan):''}</div></div><span class="tiny muted">${q.origin==='log'?'记录':'更新'} ${date(q.updatedAt)}</span></div>${(q.balances??[]).map(b=>`<div class="quota-window"><div class="between"><span>可用余额</span><strong>${escapeHTML(b.currency)} ${escapeHTML(b.total)}</strong></div><div class="between tiny muted"><span>赠送 ${escapeHTML(b.granted)}</span><span>充值 ${escapeHTML(b.toppedUp)}</span></div></div>`).join('')}${q.provider==='codex'?`<div class="quota-window"><div class="between"><span>Codex credits</span><strong>${q.credits?.unlimited?'无限':q.credits?.balance!=null?escapeHTML(q.credits.balance):q.credits?.hasCredits?'有余额 · 数量未知':'—'}</strong></div><span class="tiny muted">${q.creditsUpdatedAt?date(q.creditsUpdatedAt):'尚未取得 credits 信息'}${q.error?' · 更新失败':''}</span></div><button class="estimate-entry" data-credit-estimate="${escapeHTML(key)}">估算 500 / 1000 credits 的 API 价值 ›</button>`:''}${q.isAvailable===false?'<p class="error">当前余额不足以调用 API</p>':''}${windows}${q.bankReset?`<details class="bank" data-agent-detail="bank:${escapeHTML(key)}"><summary>Bank Reset · ${q.bankReset.availableCount} 次可用</summary>${(q.bankReset.credits??[]).map(c=>`<div class="between tiny muted"><span>${escapeHTML(c.title??'Reset Credit')}</span><span>到期 ${date(c.expiresAt)}</span></div>`).join('')}</details>`:''}${q.error?`<p class="error">${escapeHTML(q.error)}</p>`:''}${(q.provider!=='agy'&&q.windows.some(w=>w.windowMinutes===10080))||latest?`<button class="estimate-entry" data-estimate="${escapeHTML(key)}"><span>${latest?'7d 整周估值':'估算整周价值'}</span><strong>${latest?.weeklyValue!=null?money(latest.weeklyValue)+' USD'+(latest.status==='pending'?' · 待确认':''):latest?estimateStatus(latest):'›'}</strong></button>`:''}</div>`;
}

function drawTrend() {
  const canvas=$('#trend');if(!canvas||!state.dashboard)return;
  const width=canvas.clientWidth,height=canvas.clientHeight,dpr=window.devicePixelRatio||1;canvas.width=width*dpr;canvas.height=height*dpr;
  const ctx=canvas.getContext('2d');ctx.scale(dpr,dpr);
  const days=state.dashboard.trendDays,rows=state.dashboard.dayModels,values=days.map(d=>state.cost?d.cost:d.total),max=Math.max(1,...values);
  const left=58,top=12,bottom=30,plot=height-top-bottom,space=(width-left)/Math.max(1,days.length);
  const style=getComputedStyle(canvas),muted=style.getPropertyValue('--muted');ctx.font=`13px ${style.fontFamily}`;ctx.fillStyle=muted;ctx.textAlign='right';
  for(let i=0;i<3;i++){const y=top+plot*i/2;ctx.fillText(state.cost?money(max*(1-i/2)):compact(max*(1-i/2)),left-9,y+3);ctx.strokeStyle='rgba(140,145,170,.15)';ctx.beginPath();ctx.moveTo(left,y);ctx.lineTo(width,y);ctx.stroke();}
  days.forEach((d,i)=>{let used=0;const x=left+i*space+space*.2;rows.filter(r=>r.day===d.key).forEach(r=>{const h=(state.cost?r.usage.cost:r.usage.total)/max*plot;ctx.fillStyle=color(r.model);ctx.fillRect(x,top+plot-used-h,Math.max(1,space*.6),h);used+=h;});if(i%Math.max(1,Math.floor(days.length/7))===0){ctx.fillStyle=muted;ctx.textAlign='center';ctx.fillText(d.key.slice(5),x+space*.3,height-5);}});
  canvas.onmousemove=e=>{const i=Math.floor((e.offsetX-left)/space),day=days[i];canvas.title=day?`${day.key} · 缓存命中率 ${pct(cacheRate(day.tokens))}\n`+rows.filter(r=>r.day===day.key).map(r=>`${r.model}: ${state.cost?money(r.usage.cost):compact(r.usage.total)+' Token'}`).join('\n'):'';};
}
function resourcePercent(value) { return Number.isFinite(value)?Math.max(0,Math.min(100,value)):null; }
function resourceColor(value) { return value>=90?'var(--resource-high)':value>=70?'var(--resource-warn)':'var(--accent)'; }
function capacityPercent(used,total) { return Number.isFinite(used)&&Number.isFinite(total)&&total>0?used/total*100:null; }
function resourceBar(value,label) {
  const n=resourcePercent(value);
  return `<div class="resource-bar${n==null?' unavailable':''}" role="${n==null?'img':'progressbar'}" aria-label="${escapeHTML(label)}" ${n==null?'':`aria-valuemin="0" aria-valuemax="100" aria-valuenow="${n}"`}><span style="width:${n??0}%;background:${resourceColor(n)}"></span></div>`;
}
function resourceRing(label,value,detail='') {
  const n=resourcePercent(value);
  return `<div class="resource-gauge"><div class="resource-ring" role="img" aria-label="${escapeHTML(label)} ${pct(n)}"><svg viewBox="0 0 100 100" aria-hidden="true"><circle class="ring-track" cx="50" cy="50" r="42"/><circle class="ring-value" ${n==null||n===0?'hidden':''} cx="50" cy="50" r="42" pathLength="100" stroke-dasharray="${n??0} 100" style="stroke:${resourceColor(n)}"/></svg><strong>${pct(n)}</strong></div><div><h3>${escapeHTML(label)}</h3>${detail?`<span class="muted tiny">${escapeHTML(detail)}</span>`:''}</div></div>`;
}
function hostStatus(host,result) {
  if(!host.enabled)return '已暂停';
  if(result?.error)return '连接失败';
  if(!result?.sample)return '等待采样';
  const timestamp=Number(result.sample.timestamp);
  if(!Number.isFinite(timestamp)||timestamp<=0||Date.now()-timestamp*1000>10000)return '数据延迟';
  return Object.keys(result.sample.errors??{}).length?'部分采集失败':'正常';
}
function updateHostStatuses() {
  if(state.page!=='servers'||!state.settings)return;
  for(const element of document.querySelectorAll('[data-host-status]')){
    const host=state.settings.hosts.find(row=>row.id===element.dataset.hostStatus);if(!host)continue;
    const status=hostStatus(host,state.hosts.find(row=>row.id===host.id));
    if(element.dataset.status!==status){element.dataset.status=status;element.querySelector('[data-host-status-label]').textContent=status;}
  }
}
function renderServers() {
  const view=rememberView();
  $('#content').innerHTML = `<div class="section-head"><span class="muted">${state.settings.hosts.length} 台主机</span><button id="sample" title="立即采样所有已启用的服务器">刷新服务器</button></div>${state.settings.hosts.map(h=>{
    const result=state.hosts.find(r=>r.id===h.id),s=result?.sample,cpu=s?.cpu?.find(c=>c.id==='cpu'),status=hostStatus(h,result);
    return `<div class="card server-card"><div class="between"><div><h2 style="margin:0">${escapeHTML(h.name||h.target)}</h2><div class="sub">${escapeHTML(h.target)}</div></div><span class="host-status" data-host-status="${escapeHTML(h.id)}" data-status="${status}"><i aria-hidden="true"></i><span data-host-status-label>${status}</span></span></div>${s ? `<div class="server-gauges">${s.cpu?resourceRing('CPU',cpu?.utilization):''}${s.memory?resourceRing('内存',capacityPercent(s.memory.total-s.memory.available,s.memory.total),`${bytes(s.memory.total-s.memory.available)} / ${bytes(s.memory.total)}`):''}${(s.gpu??[]).map(g=>resourceRing(`GPU ${g.id}`,g.utilization,g.name??'')).join('')}</div>${Object.entries(groups).filter(([key])=>s[key]).map(([key,label])=>serverGroup(key,label,s[key],h)).join('')}<div class="between tiny muted"><span>负载 ${s.load.map(n=>n.toFixed(2)).join(' / ')}</span><span>${date(s.timestamp)}</span></div>${Object.keys(s.errors??{}).map(k=>`<p class="error">${groups[k]??escapeHTML(k)} · 采集失败</p>`).join('')}`:''}${result?.error?`<p class="error">${escapeHTML(result.error)}</p>`:''}</div>`;
  }).join('')||'<div class="card empty"><button id="add-first-host" class="primary" title="新建第一台服务器">添加服务器</button></div>'}`;
  restoreView(view);
  $('#sample').onclick=sample;
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
  return prices.map(p=>`<div class="list-row"><div class="row-body">${escapeHTML(p.id)}<small>输入 ${p.input==null?'—':money(p.input*1e6)} · 输出 ${p.output==null?'—':money(p.output*1e6)}</small></div><button data-price="${escapeHTML(p.id)}" aria-label="编辑 ${escapeHTML(p.id)} 的价格" title="编辑该模型的价格">编辑</button></div>`).join('')||`<div class="empty">${query?'无匹配模型':'同步模型价格'}</div>`;
}
function bindPrices() { document.querySelectorAll('[data-price]').forEach(b=>b.onclick=()=>editPrice(state.prices.find(p=>p.id===b.dataset.price))); }
function sourceError(id) {
  const error=state.dashboard?.sources.find(s=>s.id===id)?.status?.error;
  return error?`<span class="error" role="status">${escapeHTML(error)}</span>`:'';
}
function accountRows(accounts) {
  return accounts.map(a=>{const i=state.settings.accounts.indexOf(a);return `<div class="list-row"><div class="row-body">${escapeHTML(a.name)}<small>${escapeHTML(providers[a.provider]??a.provider)}</small></div><button data-edit-account="${i}" aria-label="编辑账户 ${escapeHTML(a.name)}" title="编辑账户">编辑</button>${a.archived?`<button data-restore-account="${i}" aria-label="恢复账户 ${escapeHTML(a.name)}" title="把归档账户恢复为正常状态">恢复</button>`:`<button data-remove-account="${i}" aria-label="归档账户 ${escapeHTML(a.name)}" title="归档账户并保留历史">归档</button>`}</div>`;}).join('');
}
function renderSettings() {
  const view=rememberView();
  if(state.settingsTab==='accounts')state.settingsTab='sources';
  if(state.settingsTab==='connection')state.settingsTab='general';
  const tabs={sources:'数据源',hosts:'服务器',prices:'价格',wakeups:'定时唤醒',general:'通用'};
  let body='';const s=state.settings;
  if(state.settingsTab==='sources'||state.settingsTab==='hosts'){
    const isSource=state.settingsTab==='sources',list=isSource?s.sources:s.hosts;
    body=`<div class="card">${list.map(item=>`<div class="list-row"><input type="checkbox" data-enable="${escapeHTML(item.id)}" ${item.enabled?'checked':''} aria-label="启用 ${escapeHTML(item.name)}"><div class="row-body">${escapeHTML(item.name||item.target)}<small>${escapeHTML(isSource?`${providers[item.provider]} · ${item.accountId ? (s.accounts.find(a=>a.id===item.accountId&&a.provider===item.provider)?.name??item.accountId) : "无账户"}`:item.target)}</small>${isSource?sourceError(item.id):''}</div><button data-edit="${escapeHTML(item.id)}" aria-label="编辑 ${escapeHTML(item.name||item.target)}" title="编辑此项">编辑</button><button data-remove="${escapeHTML(item.id)}" aria-label="移除 ${escapeHTML(item.name)}" title="移除此项">−</button></div>`).join('')||'<div class="empty">添加第一个'+(isSource?'数据源':'主机')+'</div>'}</div><button id="add-item" class="primary" title="新建数据源或服务器">＋ 添加${isSource?'数据源':'主机'}</button>`;
    if(isSource){
      const orphan=s.accounts.filter(a=>!a.archived&&!s.sources.some(src=>src.accountId===a.id&&src.provider===a.provider)),archived=s.accounts.filter(a=>a.archived);
      if(orphan.length)body+=`<details class="card account-history"><summary>未关联账户</summary>${accountRows(orphan)}</details>`;
      if(archived.length)body+=`<details class="card account-history"><summary>已归档账户</summary>${accountRows(archived)}</details>`;
    }
  }else if(state.settingsTab==='prices')body=gapList()+`<div class="price-toolbar"><input id="price-search" type="search" aria-label="搜索模型" placeholder="搜索模型" value="${escapeHTML(state.priceSearch)}"><button id="sync-prices" title="从项目 Release 同步 OpenRouter 价格表">同步 OpenRouter</button><button id="add-price" title="手动添加一个模型价格">添加价格</button></div><div class="card prices-list">${priceRows()}</div><form id="mapping-form" class="card"><h2>模型映射</h2>${Object.entries(s.modelMappings).map(([from,to])=>`<div class="list-row tiny"><span>${escapeHTML(from)} → ${escapeHTML(to)}</span><button type="button" data-unmap="${escapeHTML(from)}" aria-label="移除 ${escapeHTML(from)} 的模型映射">−</button></div>`).join('')}${field('mappingModel','日志模型名称')}${field('mappingId','OpenRouter 模型 ID')}<div class="between"><button>添加映射</button><button id="reprice" type="button" title="按当前价格重新计算历史成本">按当前价格重算</button></div></form><span class="muted tiny">USD / 百万 Token</span>`;
  else if(state.settingsTab==='wakeups')body=window.AieyesWakeups?.html()??'<p>正在加载…</p>';
  else body=(window.AieyesDesktop?.settingsHTML() || '')+`<form id="connection-form" class="card"><h2>连接</h2>${proxyFields('app',s.proxy)}<button class="primary">保存</button></form><form id="general-form" class="card"><h2>刷新</h2>${field('refreshSeconds','Agent 间隔（秒）',s.refreshSeconds,'','number')}${field('serverRefreshSeconds','服务器间隔（秒）',s.serverRefreshSeconds,'','number')}<button class="primary">保存</button></form>${window.AieyesUpdates?.settingsHTML() ?? '<div class="card"><p>正在读取更新信息…</p></div>'}`;
  $('#content').innerHTML=`<div class="settings-tabs" role="tablist" aria-label="设置分类">${Object.entries(tabs).map(([k,v])=>`<button id="settings-tab-${k}" data-settings-tab="${k}" role="tab" aria-controls="settings-panel" aria-selected="${state.settingsTab===k}" tabindex="${state.settingsTab===k?0:-1}" class="${state.settingsTab===k?'active':''}">${v}</button>`).join('')}</div><div id="settings-panel" class="settings-block" role="tabpanel" aria-labelledby="settings-tab-${state.settingsTab}">${body}</div>`;
  window.AieyesDesktop?.bindSettings();
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
    showEditor('归档账户',`<p>归档「${escapeHTML(a.name)}」并保留历史？</p>`,async()=>{
      const next=structuredClone(state.settings);next.accounts.find(row=>row.id===a.id&&row.provider===a.provider).archived=true;
      await saveSettings(next);renderSettings();
    });
    $('#editor-form button[type=submit]').textContent='归档并保留历史';
  });
  document.querySelectorAll('[data-restore-account]').forEach(b=>b.onclick=()=>job('恢复账户',async()=>{
    const next=structuredClone(state.settings);next.accounts[Number(b.dataset.restoreAccount)].archived=false;
    await saveSettings(next);renderSettings();
  }));
  document.querySelectorAll('[data-gap-price]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapPrice)];editPrice(state.prices.find(p=>p.id===(g.priceId??g.model))??{id:g.priceId??g.model,name:g.model});});
  document.querySelectorAll('[data-gap-map]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapMap)];$('#field-mappingModel').value=g.model;$('#field-mappingId').value=g.priceId??'';$('#field-mappingId').focus();});
  const items=state.settingsTab==='sources'?s.sources:s.hosts;
  document.querySelectorAll('[data-edit]').forEach(b=>b.onclick=()=>editItem(items.find(i=>i.id===b.dataset.edit)));
  document.querySelectorAll('[data-enable]').forEach(b=>b.onchange=async()=>{
    const list=state.settingsTab==='sources'?'sources':'hosts',next=structuredClone(state.settings);
    next[list].find(i=>i.id===b.dataset.enable).enabled=b.checked;b.disabled=true;
    try { await job('保存设置',()=>saveSettings(next)); }
    finally { b.checked=state.settings[list].find(i=>i.id===b.dataset.enable)?.enabled??false;b.disabled=false; }
  });
  document.querySelectorAll('[data-remove]').forEach(button=>button.onclick=()=>{
    const kind=state.settingsTab,id=button.dataset.remove,item=state.settings[kind].find(row=>row.id===id);
    const linked=kind==='hosts'?state.settings.sources.filter(src=>src.hostId===id):[];
    const impact=linked.length?`<p>以下 ${linked.length} 个数据源将暂停，并需要重新选择服务器：</p><ul>${linked.map(src=>`<li>${escapeHTML(src.name)}</li>`).join('')}</ul>`:'<p>已导入的用量历史会保留。重新连接时需要再次添加配置。</p>';
    showEditor(kind==='hosts'?'移除服务器':'移除数据源',`<p>确认移除「${escapeHTML(item.name||item.target)}」？</p>${impact}`,async()=>{
      const next=structuredClone(state.settings);next[kind]=next[kind].filter(row=>row.id!==id);
      if(kind==='hosts')for(const src of next.sources)if(src.hostId===id){src.hostId=null;src.enabled=false;}
      if(kind==='sources')for(const account of next.accounts)if(account.quotaSourceId===id)account.quotaSourceId=null;
      await saveSettings(next);renderSettings();notify(linked.length?`已移除服务器，${linked.length} 个关联数据源已暂停`:'已移除，历史记录已保留');
    });
    const submit=$('#editor-form button[type=submit]');submit.textContent='确认移除';submit.classList.add('danger');
    $('#cancel-editor').focus();
  });
  if($('#add-item'))$('#add-item').onclick=()=>editItem();
  if($('#connection-form'))$('#connection-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);job('保存设置',async()=>{const next=structuredClone(state.settings);next.proxy=proxyValue(f,'app');await saveSettings(next);e.target.dataset.updateSavedValues=JSON.stringify([...new FormData(e.target)]);});};
  if($('#general-form'))$('#general-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);job('保存设置',async()=>{const next=structuredClone(state.settings);next.refreshSeconds=Number(f.get('refreshSeconds'));next.serverRefreshSeconds=Number(f.get('serverRefreshSeconds'));await saveSettings(next);e.target.dataset.updateSavedValues=JSON.stringify([...new FormData(e.target)]);});};
  window.AieyesUpdates?.bindSettings();
  if($('#sync-prices'))$('#sync-prices').onclick=syncPrices;
  if($('#add-price'))$('#add-price').onclick=()=>editPrice();
  bindPrices();
  if($('#price-search'))$('#price-search').oninput=e=>{state.priceSearch=e.target.value;$('.prices-list').innerHTML=priceRows();bindPrices();};
  if($('#connection-form'))bindProxy('app');
  if($('#mapping-form'))$('#mapping-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);if(!f.get('mappingModel')?.trim()||!f.get('mappingId')?.trim()){notify('请输入日志模型名称和 OpenRouter 模型 ID','error');return;}job('保存映射',async()=>{const next=structuredClone(state.settings);next.modelMappings[f.get('mappingModel').trim()]=f.get('mappingId').trim();await saveSettings(next);renderSettings();});};
  document.querySelectorAll('[data-unmap]').forEach(b=>b.onclick=()=>job('保存映射',async()=>{const next=structuredClone(state.settings);delete next.modelMappings[b.dataset.unmap];await saveSettings(next);renderSettings();}));
  if($('#reprice'))$('#reprice').onclick=()=>job('重新计价',async()=>{await api('prices.recalculate');await loadDashboard();renderSettings();notify('已按当前价格重算');});
  restoreView(view);
}
let editorSelectors=[],editorSaving=false,editorOpener=null,editorOpenerSelector=null;
function clearEditorSelectors(){editorSelectors.forEach(c=>c.destroy());editorSelectors=[];}
function finishEditor(){
  clearEditorSelectors();$('.shell').inert=false;$('#editor-backdrop').hidden=true;document.body.classList.remove('editor-open');
  if(editorOpener||editorOpenerSelector){const target=editorOpener?.isConnected?editorOpener:editorOpenerSelector?document.querySelector(editorOpenerSelector):null;(target??$('#add-item')??$('#add-price')??$('.settings-tabs .active'))?.focus({preventScroll:true});}
  editorOpener=null;editorOpenerSelector=null;
}
function closeEditor(){if(editorSaving)return;$('#editor').close();finishEditor();}
function showEditor(title,html,onSubmit){
  if(editorSaving)return;
  editorOpener=document.activeElement;editorOpenerSelector=focusSelector(editorOpener);clearEditorSelectors();
  $('#editor-title').textContent=title;$('#editor-fields').innerHTML=html;
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
    if(saved)closeEditor();
    else {$('#editor-error').tabIndex=-1;$('#editor-error').focus({preventScroll:true});}
  };
  // Keep native-window controls interactive; only the application page is inert.
  $('.shell').inert=true;$('#editor-backdrop').hidden=false;document.body.classList.add('editor-open');$('#editor').show();
  ($('#editor-fields').querySelector('input:not([disabled]),select:not([disabled]),textarea:not([disabled])')??$('#cancel-editor')).focus();
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
  const a=structuredClone(index==null?{id:crypto.randomUUID(),name:'',provider:'codex',quotaEnabled:true,quotaSourceId:null}:state.settings.accounts[index]);
  const linked=state.settings.sources.filter(s=>s.accountId===a.id&&s.provider===a.provider);
  showEditor('账户',field('name','账户名称',a.name,'个人账户 / 工作账户')+select('provider','Agent',Object.entries(providers),a.provider)+select('quotaEnabled','账户限额',[['yes','显示并查询'],['no','仅统计用量']],a.quotaEnabled?'yes':'no')+select('quotaSourceId','优先查询位置',[['','自动 · 优先本机'],...linked.map(s=>[s.id,s.name])],a.quotaSourceId??'')+'',async f=>{
    a.name=f.get('name').trim();if(!a.name)throw new Error('请输入账户名称');if(!linked.length)a.provider=f.get('provider');a.quotaEnabled=f.get('quotaEnabled')==='yes';a.quotaSourceId=f.get('quotaSourceId')||null;
    const next=structuredClone(state.settings);if(index==null)next.accounts.push(a);else next.accounts[index]=a;
    await saveSettings(next);renderSettings();
  });
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
  return select(prefix+'Mode','连接方式',[...(inherit?[['inherit','跟随应用']]:[]),['system','系统代理'],['direct','直连'],['custom','指定代理']],proxy?.mode??(inherit?'inherit':'system'))+`<div id="${prefix}-proxy-address">${select(prefix+'Protocol','协议',[['http','HTTP'],['https','HTTPS'],['socks5','SOCKS5'],['socks5h','SOCKS5H'],['url','自定义 URL']],p.protocol)}<div id="${prefix}-proxy-basic">${field(prefix+'Host','Host',p.host)}${field(prefix+'Port','端口',p.port,'','number')}</div><div id="${prefix}-proxy-url">${field(prefix+'URL','代理 URL',p.url)}</div></div>`;
}
function bindProxy(prefix) {
  const protocol=$('#field-'+prefix+'Protocol');let previous=protocol.value;
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
  const item=structuredClone(existing??(source?{id:crypto.randomUUID(),name:'',provider:'codex',accountId:'',path:'~/.codex',hostId:null,enabled:true,quotaCommand:'',quotaPreCommand:'',codexBinary:'codex',agyBinary:'agy',proxy:null}:{id:crypto.randomUUID(),name:'',target:'',port:null,identityFile:'',shell:'/bin/bash',preCommand:'',enabled:true,metrics:Object.keys(groups),devices:[],details:Object.keys(detailOptions)}));
  const accountOptions=provider=>[['__new__','新建账户'],...state.settings.accounts.filter(a=>a.provider===provider&&(!a.archived||a.id===item.accountId)).map(a=>[a.id,a.name+(a.archived?'（已归档）':'')])];
  const account=state.settings.accounts.find(a=>a.id===item.accountId&&a.provider===item.provider);
  const body=source?field('name','名称',item.name)+select('provider','Agent',Object.entries(providers),item.provider)+`<label class="account-toggle"><input type="checkbox" name="isAccount" id="field-isAccount" ${item.accountId?'checked':''} aria-describedby="account-help">关联账户与限额</label><p id="account-help" class="muted tiny account-help">数据源用于读取记录；关联账户后可查询限额或余额。同一账户的名称与限额设置由关联它的数据源共用。</p><div id="account-fields">${select('accountId','账户',accountOptions(item.provider),item.accountId||'__new__')}${field('accountName','账户名称',account?.name??'')}${select('quotaEnabled','账户限额',[['yes','显示并查询'],['no','仅统计用量']],account?.quotaEnabled?'yes':'no')}${select('accountArchived','账户状态',[['no','正常'],['yes','已归档']],account?.archived?'yes':'no')}${select('quotaSourceId','优先查询位置',[['','自动 · 优先本机']],account?.quotaSourceId??'')}</div>`+select('hostId','位置',[['','本机'],...state.settings.hosts.map(h=>[h.id,h.name||h.target])],item.hostId??'')+field('path','数据目录',item.path)+field('codexBinary','Codex 程序',item.codexBinary)+field('agyBinary','agy 程序',item.agyBinary??'agy')+`<div id="deepseek-key">${field('apiKey','API Key（留空保留）','','','password')}</div><div id="remote-quota">${textarea('quotaPreCommand','限额查询前置命令',item.quotaPreCommand)}</div><div id="local-proxy">${proxyFields('source',item.proxy,true)}</div>`:
    field('name','名称',item.name)+field('target','SSH 别名或地址',item.target,'my-server 或 user@host')+field('port','端口',item.port??'','跟随 SSH 配置','number')+field('identityFile','密钥路径',item.identityFile,'跟随 SSH 配置')+`<section class="host-selectors"><div id="metric-select"></div><div class="section-head"><h3>设备</h3><button type="button" id="discover-devices" title="通过 SSH 读取远端设备列表">读取设备</button></div><p id="discovery-status" class="error" role="status" hidden></p><div id="device-selects" class="device-selects"></div><div id="detail-select"></div></section><details class="advanced"><summary>高级设置</summary>${field('shell','远程 shell',item.shell)+textarea('preCommand','远程前置命令',item.preCommand)+field('devices','设备表达式',item.devices.join(', '),'network:eth0, gpu:0, filesystems:/')}</details>`;
  showEditor(source?'数据源':'SSH 主机',body,async f=>{
    const next=structuredClone(state.settings);
    if(source){
      for(const k of ['name','provider','path','codexBinary','agyBinary','quotaPreCommand'])item[k]=f.get(k);
      item.accountId='';
      if(f.has('isAccount')){
        const selected=f.get('accountId');let a=next.accounts.find(a=>a.id===selected&&a.provider===item.provider);
        if(!a){if(selected!=='__new__')throw new Error('请选择账户');a={id:crypto.randomUUID(),provider:item.provider,archived:false};next.accounts.push(a);}
        a.name=f.get('accountName').trim()||item.name.trim()||providers[item.provider];
        a.quotaEnabled=f.get('quotaEnabled')==='yes';a.archived=f.get('accountArchived')==='yes';a.quotaSourceId=f.get('quotaSourceId')||null;item.accountId=a.id;
      }
      item.hostId=f.get('hostId')||null;item.proxy=proxyValue(f,'source');if(!item.name)item.name=providers[item.provider];
      if(item.provider==='deepseek'){item.hostId=null;if(f.get('apiKey')?.trim()){const saved=await api('credentials.save',{sourceId:item.id,apiKey:f.get('apiKey')});item.path=saved.path;}}
    }else{for(const k of ['name','target','identityFile','shell','preCommand'])item[k]=f.get(k);item.port=f.get('port')?Number(f.get('port')):null;item.devices=f.get('devices').split(',').map(s=>s.trim()).filter(Boolean);if(!item.name)item.name=item.target;}
    const items=source?next.sources:next.hosts,index=items.findIndex(i=>i.id===item.id);if(index<0)items.push(item);else items[index]=item;
    if(source)for(const a of next.accounts)if(a.quotaSourceId===item.id&&(a.id!==item.accountId||a.provider!==item.provider))a.quotaSourceId=null;
    await saveSettings(next);renderSettings();
  });
  if(!source)bindHostSelectors(item);
  else {
    bindProxy('source');
    const update=()=>{
      const provider=$('#field-provider').value,asAccount=$('#field-isAccount').checked;
      $('#account-fields').hidden=!asAccount;
      const accountId=$('#field-accountId').value,linked=state.settings.sources.filter(src=>src.id!==item.id&&src.provider===provider&&src.accountId===accountId);
      $('#account-help').textContent=['agy','deepseek'].includes(provider)?'此服务通过关联账户显示限额或余额。取消关联后不会查询账户信息。':'数据源用于读取记录；关联账户后可查询限额，并汇总同一账户的用量。';
      if(asAccount&&linked.length)$('#account-help').textContent+=` 当前账户还关联 ${linked.length} 个数据源，修改账户名称或限额设置会同时生效。`;
      $('#deepseek-key').hidden=provider!=='deepseek';$('#field-agyBinary').closest('.form-row').hidden=provider!=='agy';$('#field-path').closest('.form-row').hidden=provider==='agy';
      document.querySelector('label[for=field-path]').textContent=provider==='deepseek'?'API Key 文件（可选）':'数据目录';$('#field-hostId').closest('.form-row').hidden=provider==='deepseek';
      $('#remote-quota').hidden=!$('#field-hostId').value||!asAccount;$('#local-proxy').hidden=!!$('#field-hostId').value;$('#field-codexBinary').closest('.form-row').hidden=provider!=='codex';
      $('#field-quotaSourceId').closest('.form-row').hidden=$('#field-quotaEnabled').value!=='yes';
    };
    const selectAccount=()=>{
      const provider=$('#field-provider').value,id=$('#field-accountId').value,a=state.settings.accounts.find(a=>a.id===id&&a.provider===provider);
      $('#field-accountName').value=a?.name??'';$('#field-accountArchived').value=a?.archived?'yes':'no';$('#field-quotaEnabled').value=(a?a.quotaEnabled:['codex','claude','agy','deepseek'].includes(provider))?'yes':'no';
      const linked=state.settings.sources.filter(s=>s.provider===provider&&s.accountId===id&&s.id!==item.id);
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
function bindHostSelectors(item) {
  const selectors=AieyesSelect;
  let discovered={}, deviceControls=[];
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
    const form=$('#editor-form'), f=new FormData(form), host={...item};
    for(const key of ['target','identityFile','shell','preCommand'])host[key]=f.get(key);
    host.port=f.get('port')?Number(f.get('port')):null;
    try {
      const sample=await api('hosts.discover',{host});
      if(!$('#editor').open||!button.isConnected)return;
      for(const group of Object.keys(groups))if(Array.isArray(sample[group]))discovered[group]=sample[group];
      redrawDevices();
      status.textContent=Object.keys(sample.errors??{}).map(k=>(groups[k]??k)+'读取失败').join(' · ');status.hidden=!status.textContent;
    }catch(error){if(button.isConnected){status.textContent=String(error);status.hidden=false;}}
    finally{button.disabled=false;button.textContent='读取设备';}
  };
}
function editPrice(existing){const price=structuredClone(existing??{id:'',name:'',input:null,output:null,cacheRead:null,cacheWrite:null,fetchedAt:0});
  showEditor('模型价格 · USD / 百万 Token',field('id','模型 ID',price.id)+field('name','显示名称',price.name)+[['input','输入'],['output','输出'],['cacheRead','缓存读取'],['cacheWrite','缓存写入']].map(([k,label])=>field(k,label,price[k]==null?'':price[k]*1e6,'待定价')).join(''),async f=>{price.id=f.get('id');price.name=f.get('name');for(const k of ['input','output','cacheRead','cacheWrite']){const v=f.get(k);price[k]=v===''?null:Number(v)/1e6;if(price[k]!=null&&(!Number.isFinite(price[k])||price[k]<0))throw new Error('价格需要为非负数');}await api('prices.save',{prices:[price]});state.prices=await api('prices.list');await loadDashboard();renderSettings();});
}
document.querySelectorAll('[data-page]').forEach(button=>button.onclick=()=>{
  state.page=button.dataset.page;render();
  if(PANEL)window.__TAURI__.core.invoke('desktop_panel_page',{page:state.page}).catch(error=>notify(String(error),'error'));
  if(state.page==='servers')sample();
});
if($('#editor')){
  $('#editor').addEventListener('close',()=>{if(!$('#editor').open)finishEditor();});
  $('#editor').addEventListener('cancel',clearEditorSelectors);
  $('#close-editor').onclick=$('#cancel-editor').onclick=()=>closeEditor();
}
if($('#scan'))$('#scan').onclick=scan;
if($('#quota'))$('#quota').onclick=quotas;
$('#message').onclick=()=>notify('');
new ResizeObserver(()=>{if(state.page==='agent')drawTrend();}).observe($('#content'));
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change',()=>{if(state.page==='agent')requestAnimationFrame(drawTrend);});
let externalRefresh=null,externalPending=false,externalSettings=false,externalQuotaCheck=false,externalScanCheck=false,externalMutation=false;
function refreshExternalData(settings=false,method='') {
  externalPending=true;externalSettings ||= settings;externalQuotaCheck ||= method==='quotas.refresh';externalScanCheck ||= method==='sources.scan';externalMutation ||= Boolean(method);
  if(externalRefresh)return externalRefresh;
  externalRefresh=(async()=>{
    while(externalPending){
      externalPending=false;const readSettings=externalSettings,checkQuotas=externalQuotaCheck,checkScan=externalScanCheck,mutation=externalMutation;externalSettings=false;externalQuotaCheck=false;externalScanCheck=false;externalMutation=false;
      if(readSettings||!state.settings)state.settings=await api('settings.get');
      await loadDashboard();
      if(readSettings&&PANEL&&state.page!=='agent')render();
      const scannedSources=(state.dashboard?.sources??[]).filter(source=>state.settings.sources.some(row=>row.id===source.id&&row.enabled&&(!row.hostId||state.settings.hosts.some(host=>host.id===row.hostId&&host.enabled)))).map(source=>({id:source.id,error:source.status?.error}));
      const error=(checkQuotas?batchError(state.dashboard?.quotas,'读取限额'):null)??(checkScan?batchError(scannedSources,'同步记录'):null);
      if(checkQuotas)state.quotaError=error?.message??'';
      if(!state.busy){
        if(error){notify(error.message,'error');$('#activity').dataset.status=error.partial?'partial':'error';$('#activity').textContent=error.partial?'部分读取失败':'读取失败';}
        else if(mutation){state.lastSuccessfulUpdate=Date.now();$('#activity').dataset.status='success';$('#activity').textContent='更新于 '+timeLabel(state.lastSuccessfulUpdate);}
        else if(!['error','partial'].includes($('#activity').dataset.status)){$('#activity').dataset.status='info';$('#activity').textContent='已读取概览';}
      }
    }
  })().catch(error=>notify(String(error),'error')).finally(()=>{externalRefresh=null;});
  return externalRefresh;
}
async function listenForSharedState() {
  const events=window.__TAURI__?.event;if(!events?.listen)return;
  await events.listen('desktop:status',({payload})=>{
    state.panelOpen=Boolean(payload.panelOpen);state.panelPage=payload.panelPage??state.panelPage;
  });
  await events.listen('desktop:settings',()=>{if(PANEL)return refreshExternalData(true);});
  await events.listen('desktop:hosts',({payload})=>{if(PANEL)applyHostSamples(payload);});
  await events.listen('desktop:hosts-error',({payload})=>{if(PANEL)applyHostFailure(payload);});
  await events.listen('desktop:data-changed',({payload})=>{
    if(PANEL&&payload==='settings.save')return; // desktop:settings owns this invalidation.
    if(PANEL||(!state.busy&&!state.settingsSaving&&!editorSaving))return refreshExternalData(payload==='settings.save'&&PANEL,payload);
  });
}
async function boot(){
  try{
    if(!window.__TAURI__)throw new Error('通过 Aieyes 桌面应用打开');
    await listenForSharedState();
    const info=await api('hello'),version=$('#app-version');if(version)version.textContent=info.version;
    state.settings=await api('settings.get');await loadDashboard();
    if(PANEL){$('#activity').textContent='已读取概览';return;}await scan();if(hasQuotaSources())await quotas();
  }catch(error){notify(String(error),'error');$('#activity').textContent='连接已断开';$('#activity').dataset.status='error';}
}
window.AieyesApp={
  async openSetupInMain(kind){if(!state.settings)await appReady;await openSetup(['hosts','quota'].includes(kind)?kind:'sources');},
  async refreshPanel(){await appReady;await refreshExternalData(true);if(state.page==='servers'&&!state.hosts.length)await sample();},
  applyHostSamples,
};
// The panel refreshes on open and on manual action; periodic polling stays in the details window.
if(!PANEL)setInterval(()=>{
  if(!state.settings)return;
  if(!state.serverBusy&&state.settings.hosts.some(h=>h.enabled)&&Date.now()-state.lastMetrics>((state.page==='servers'&&!document.hidden)||(state.panelOpen&&state.panelPage==='servers')?2000:state.settings.serverRefreshSeconds*1000)){sample();}
  if(window.AieyesUpdates?.busy||$('#editor').open||$('#quota-dialog')?.dataset.busy||state.busy||state.settingsSaving)return;
  if(hasQuotaSources()&&(!state.lastQuota||Date.now()-state.lastQuota>state.settings.refreshSeconds*1000)){quotas();return;}
  if(Date.now()-state.lastScan>state.settings.refreshSeconds*1000){scan();return;}
},500);
// Age visible samples even while an RPC is slow or the panel receives no new events.
setInterval(updateHostStatuses,1000);
function estimateStatus(e){return {active:'采样中',pending:'待确认',completed:'已结束'}[e.status]??e.status;}
function quotaDialog(title,body){
  let dialog=$('#quota-dialog');if(!dialog){dialog=document.createElement('dialog');dialog.id='quota-dialog';dialog.setAttribute('aria-labelledby','quota-dialog-title');document.body.append(dialog);}
  dialog.innerHTML=`<div class="dialog-head"><h2 id="quota-dialog-title">${escapeHTML(title)}</h2><button type="button" id="quota-close" aria-label="关闭">×</button></div><div class="quota-dialog-body">${body}<p id="quota-tool-error" class="error" role="alert" hidden></p></div>`;
  delete dialog.dataset.estimateKey;delete dialog.dataset.records;
  $('#quota-close').onclick=()=>dialog.close();if(!dialog.open){dialog.dataset.opener=focusSelector(document.activeElement)??'';dialog.onclose=()=>{if(dialog.dataset.opener)document.querySelector(dialog.dataset.opener)?.focus({preventScroll:true});};dialog.showModal();}return dialog;
}
async function quotaAction(action,params){
  const dialog=$('#quota-dialog'),controls=[...dialog.querySelectorAll('button,input,select')].map(el=>[el,el.disabled]);
  dialog.dataset.busy='true';controls.forEach(([el])=>el.disabled=true);dialog.oncancel=e=>e.preventDefault();$('#quota-tool-error').hidden=true;
  try{await api(action,params);await loadDashboard();return true;}
  catch(error){$('#quota-tool-error').textContent=error.message??String(error);$('#quota-tool-error').hidden=false;await loadDashboard();return false;}
  finally{controls.forEach(([el,disabled])=>el.disabled=disabled);dialog.oncancel=null;delete dialog.dataset.busy;}
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
function bindQuotaTools(){
  document.querySelectorAll('.agy-details').forEach(detail=>{detail.ontoggle=()=>{detail.querySelector('summary .muted').textContent=detail.open?'收起重置时间':'展开重置时间';};});
  if($('#order-quotas'))$('#order-quotas').onclick=openQuotaOrder;
  document.querySelectorAll('[data-credit-estimate]').forEach(b=>b.onclick=()=>openQuotaEstimate(b.dataset.creditEstimate,'credits'));
  document.querySelectorAll('[data-estimate]').forEach(b=>b.onclick=()=>openQuotaEstimate(b.dataset.estimate));
}
function openQuotaOrder(){
  let keys=[...(state.dashboard.quotaOrder??state.settings.accounts.map(a=>a.provider+':'+a.id))],dragged=null;
  quotaDialog('调整账户顺序','<p class="muted">拖动账户，或使用上下按钮。顺序应用于所有限额面板。</p><div id="quota-order-list"></div><button class="primary" id="quota-order-save">保存顺序</button>');
  function draw(focus){
    const positions=new Map([...document.querySelectorAll('[data-order]')].map(row=>[row.dataset.order,row.getBoundingClientRect().top]));
    $('#quota-order-list').innerHTML=keys.map((key,i)=>`<div class="quota-order-row" draggable="true" data-order="${escapeHTML(key)}"><span aria-hidden="true">☰</span><span>${escapeHTML(state.settings.accounts.find(a=>a.provider+':'+a.id===key)?.name??key)}</span><button type="button" data-direction="-1" aria-label="上移账户" ${i===0?'disabled':''}>↑</button><button type="button" data-direction="1" aria-label="下移账户" ${i===keys.length-1?'disabled':''}>↓</button></div>`).join('');
    for(const row of document.querySelectorAll('[data-order]')){
      row.ondragstart=e=>{document.querySelectorAll('[data-order]').forEach(el=>el.getAnimations().forEach(animation=>animation.cancel()));dragged=row.dataset.order;e.dataTransfer.effectAllowed='move';e.dataTransfer.setData('text/plain',dragged);};row.ondragend=()=>{dragged=null;document.querySelectorAll('.drop-target').forEach(el=>el.classList.remove('drop-target'));};row.ondragover=e=>{e.preventDefault();row.classList.add('drop-target');};row.ondragleave=()=>row.classList.remove('drop-target');
      row.ondrop=e=>{e.preventDefault();if(!dragged||dragged===row.dataset.order)return;const to=keys.indexOf(row.dataset.order);keys.splice(keys.indexOf(dragged),1);keys.splice(to,0,dragged);dragged=null;requestAnimationFrame(()=>draw());};
      row.querySelectorAll('button').forEach(b=>b.onclick=()=>{const i=keys.indexOf(row.dataset.order),j=i+Number(b.dataset.direction);[keys[i],keys[j]]=[keys[j],keys[i]];draw({key:row.dataset.order,direction:b.dataset.direction});});
    }
    if(focus&&!matchMedia('(prefers-reduced-motion: reduce)').matches)for(const row of document.querySelectorAll('[data-order]')){const old=positions.get(row.dataset.order);if(old!=null)row.animate([{transform:`translateY(${old-row.getBoundingClientRect().top}px)`},{transform:'translateY(0)'}],{duration:220,easing:'cubic-bezier(.2,.8,.2,1)'});}
    if(focus){const row=[...document.querySelectorAll('[data-order]')].find(r=>r.dataset.order===focus.key);(row?.querySelector(`button[data-direction="${focus.direction}"]:not(:disabled)`)??row?.querySelector('button:not(:disabled)'))?.focus();}
  }
  draw();$('#quota-order-save').onclick=async()=>{if(await quotaAction('quotas.order.set',{keys}))$('#quota-dialog').close();};
}
function openQuotaEstimate(key,kind="weekly"){
  const credits=kind==='credits';
  const q=state.dashboard.quotas.find(q=>q.provider+':'+q.accountId===key);if(!q)return;
  const records=(state.dashboard[credits?'creditEstimates':'quotaEstimates']??[]).filter(e=>e.accountKey===key),current=records.find(e=>e.status!=='completed');
  const windows=q.windows.filter(w=>w.windowMinutes===10080),eligible=credits?(q.provider==='codex'&&q.credits?.balance!=null&&!q.credits.unlimited):q.provider!=='agy'&&(windows.length===1||(q.provider==='claude'&&windows.some(w=>w.id==='seven_day'||w.name==='7d')));
  const sources=state.settings.sources.filter(s=>s.enabled&&s.provider===q.provider&&s.accountId===q.accountId&&!['agy','deepseek'].includes(s.provider));
  const confirmation='<label class="quota-confirm"><input type="checkbox" id="estimate-confirm">'+(credits?'我确认本段仅消耗 credits，所有设备的用量均已纳入；不混用包含额度、API 或中转。':'我确认采样期间只使用目标订阅，所有设备用量均已纳入所选来源；不混用 API / 中转。')+'</label>';
  const result=e=>`<div class="estimate-result"><div class="between"><strong>${estimateStatus(e)}</strong><strong>${credits?`500 credits：${e.valuePer500!=null?money(e.valuePer500)+' USD':'—'}<br>1000 credits：${e.valuePer1000!=null?money(e.valuePer1000)+' USD':'—'}`:e.weeklyValue!=null?money(e.weeklyValue)+' USD':'—'}</strong></div><p class="muted">${escapeHTML(e.calculationNote)}</p><details><summary>计算依据</summary><p>${date(e.startedAt)} → ${date(e.checkpointAt)}<br>${escapeHTML(e.windowName)} · 消耗 ${credits?escapeHTML(e.consumedCredits)+' credits':pct(e.consumedPercent)} · 样本成本 ${money(e.cost)} USD<br>计价 Token：${compact(e.pricedTokens)} / ${compact(e.totalTokens)}<br>${escapeHTML(e.sourceNames.join('、'))}</p><p class="tiny muted">${credits?'每 N credits 估值 = 样本成本 × N ÷ 消耗 credits':'整周估值 = 样本 API 等价成本 × 100 ÷ 消耗百分点'}；结束后价格依据固定。</p>${e.reason?`<p class="error">${escapeHTML(e.reason)}</p>`:''}${(e.prices??[]).length?`<p class="tiny muted">价格快照：${e.prices.map(p=>escapeHTML(p.id)+' · '+date(p.fetchedAt)).join('；')}</p>`:''}</details></div>`;
  const setup=eligible&&sources.length?`${credits?'':`<label>7d 额度池<select id="estimate-window">${windows.filter(w=>windows.length===1||w.id==='seven_day'||w.name==='7d').map(w=>option(w.id||w.name,w.name,'')).join('')}</select></label>`}<p>纳入的用量来源</p>${sources.map(s=>`<label class="quota-confirm"><input type="checkbox" name="estimate-source" value="${escapeHTML(s.id)}" checked>${escapeHTML(s.name)}</label>`).join('')}${confirmation}<p class="tiny muted">建议开始后新建会话。至少消耗 ${credits?'5 credits':'5 个百分点'}后输出估值；跨采样边界的累计用量会使本次结果不可用。</p><button class="primary" id="estimate-start" disabled>开始采样</button>`:'<p class="muted">需要可采集 Token 的关联数据源及可靠的额度池映射。agy 限额查询本身不提供用量历史，此额度池暂不支持估值。</p>';
  quotaDialog(q.name+(credits?' · Credits 价值':' · 7d 整周价值'),`<p class="muted">按本次模型组合的 API 等价成本与额度消耗比例估算，不是可兑换余额。</p>${current?result(current)+(current.status==='pending'?confirmation+'<button id="estimate-restart" disabled>确认并开始新一段</button>':'')+`<button class="primary" id="estimate-stop">${current.status==='pending'?'结束并保留有效段':'结束采样并计算'}</button>`:setup}${records.some(e=>e.status==='completed')?'<h3>采样历史</h3>'+records.filter(e=>e.status==='completed').map(result).join(''):''}`);
  $('#quota-dialog').dataset.estimateKind=kind;$('#quota-dialog').dataset.estimateKey=key;$('#quota-dialog').dataset.records=JSON.stringify(records);
  const confirm=$('#estimate-confirm');if(confirm)confirm.onchange=()=>{const b=$('#estimate-start')??$('#estimate-restart');if(b)b.disabled=!confirm.checked;};
  const act=async(action,params)=>{if(await quotaAction((credits?'creditEstimates.':'quotaEstimates.')+action,params))openQuotaEstimate(key,kind);else {const message=$('#quota-tool-error').textContent;refreshQuotaDialog();$('#quota-tool-error').textContent=message;$('#quota-tool-error').hidden=false;}};
  if($('#estimate-start'))$('#estimate-start').onclick=()=>act('start',{accountKey:key,windowId:credits?'credits':$('#estimate-window').value,sourceIds:[...document.querySelectorAll('[name=estimate-source]:checked')].map(el=>el.value),confirmed:confirm.checked});
  if($('#estimate-stop'))$('#estimate-stop').onclick=()=>act('stop',{id:current.id});
  if($('#estimate-restart'))$('#estimate-restart').onclick=()=>act('restart',{id:current.id,confirmed:confirm.checked});
}

const appReady=boot();
