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
const state = {page:'agent',settingsTab:'sources',priceSearch:'',settings:null,dashboard:null,hosts:[],prices:[],provider:'',sourceId:'',accountKey:'',model:'',days:1,cost:false,busy:false,settingsSaving:false,serverBusy:false,lastScan:0,lastMetrics:0,lastQuota:0};
async function api(method, params = {}) { return window.__TAURI__.core.invoke('engine_call', {method,params}); }
function notify(message) { $('#message').textContent = message; $('#message').hidden = !message; }
async function job(label, fn) {
  if (state.busy || state.settingsSaving) return false;
  state.busy = true; $('#activity').textContent = label;
  for (const b of document.querySelectorAll('header button')) b.disabled = true;
  try { await fn(); return true; } catch (e) { notify(String(e)); return false; }
  finally { state.busy = false; $('#activity').textContent = '更新于 ' + new Date().toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'}); for (const b of document.querySelectorAll('header button')) b.disabled = false; }
}
async function loadDashboard() {
  if(state.accountKey && state.accountKey!=='none' && !state.settings.accounts.some(a=>a.provider+':'+a.id===state.accountKey))state.accountKey='';
  if(state.sourceId && !state.settings.sources.some(s=>s.id===state.sourceId))state.sourceId='';
  const account=state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey);
  state.dashboard = await api('dashboard',{provider:account?.provider || state.provider || null,accountId:state.accountKey==='none'?'':account?.id ?? null,sourceId:state.sourceId || null,model:state.model || null,days:state.days});
  if (state.page === 'agent') render();
}
async function scan() { await job('同步记录', async () => { await api('sources.scan'); state.lastScan = Date.now(); await loadDashboard(); }); }
function hasQuotaSources() {
  return state.settings.accounts.some(a => !a.archived && a.quotaEnabled && state.settings.sources.some(s => s.enabled && s.provider === a.provider && s.accountId === a.id && (!s.hostId || state.settings.hosts.some(h => h.id === s.hostId && h.enabled))));
}
async function quotas() { await job('读取限额', async () => { try { await api('quotas.refresh'); await loadDashboard(); } finally { state.lastQuota = Date.now(); } }); }
async function syncPrices(){await job('同步价格',async()=>{await api('prices.sync');state.prices=await api('prices.list');await loadDashboard();if(state.page==='settings')renderSettings();notify('价格已更新');});}
async function sample() { if(state.serverBusy)return; state.serverBusy=true; try {
  const rows = await api('hosts.sample');
  state.hosts = rows.map(r => r.error ? {...r,sample:state.hosts.find(p => p.id === r.id)?.sample} : r);
  state.lastMetrics = Date.now(); if (state.page === 'servers') render();
} catch(e) { notify(String(e)); state.lastMetrics=Date.now(); } finally { state.serverBusy=false; } }
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
  document.querySelectorAll('[data-page]').forEach(b => b.classList.toggle('active',b.dataset.page === state.page));
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
  const opened=new Set([...document.querySelectorAll('[data-agent-detail][open]')].map(e=>e.dataset.agentDetail));
  const focused=document.activeElement,focusId=focused?.id,focusDetail=focused?.tagName==='SUMMARY'?focused.parentElement.dataset.agentDetail:null;
  const scroll={x:window.scrollX,y:window.scrollY};
  const s=d.summary,t=s.tokens,models=d.models,sum=models.reduce((n,m)=>n+(state.cost?m.cost:m.total),0);
  let cursor=0;
  const stops=models.map(m=>{const start=cursor;cursor+=sum?(state.cost?m.cost:m.total)/sum*360:0;return `${color(m.key)} ${start}deg ${cursor}deg`;});
  const maxHeat=Math.max(1,...d.heatmap.map(v=>state.cost?v.cost:v.total));
  const lead=d.heatmap.length?(new Date(d.heatmap[0].key+'T12:00:00').getDay()+6)%7:0;
  const account=state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey);
  const sources=state.settings.sources.filter(src=>{const ids=d.sources.find(s=>s.id===src.id)?.accountIds??[src.accountId];return (!state.provider||src.provider===state.provider)&&(!state.accountKey||(state.accountKey==='none'?ids.includes(''):src.provider===account?.provider&&ids.includes(account?.id)));});
  const trendModels=[...new Set(d.dayModels.map(r=>r.model))].sort();
  $('#title').textContent='Agent 概览';
  $('#content').innerHTML=`<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','无账户 / API',state.accountKey)}${state.settings.accounts.filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select><select id="source" aria-label="数据源">${option('','全部数据源',state.sourceId)}${sources.map(s=>option(s.id,s.name,state.sourceId)).join('')}</select><select id="model" aria-label="模型">${option('','全部模型',state.model)}${[...new Set([...trendModels,...(state.model?[state.model]:[])])].map(m=>option(m,m,state.model)).join('')}</select></div>
  ${d.quotas.length?`<div class="section-head"><h2>账户限额 <span class="count">${d.quotas.length}</span></h2><button id="read-quotas" title="重新查询各账户的实时限额">刷新限额</button></div><div class="quotas">${d.quotas.map(quotaCard).join('')}</div>`:''}
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':`最近 ${n} 天`,state.days)).join('')}</select></div>
  <div class="stats">${stat('总 Token',compact(s.total),'','✧')}${stat('API 等价成本'+(s.total>0&&s.pricedTokens<s.total?' <button id="repair-pricing" class="pricing-alert" aria-label="计价未完成，设置模型价格" title="部分 Token 尚未计价">!</button>':''),s.pricedTokens?money(s.cost):'—','','$')}${stat('缓存命中率',pct(cacheRate(t)),'','▱')}<div class="stat token-card"><div class="stat-label">Token 明细</div><div class="token-breakdown">${[['输入',t.input],['输出',t.output],['缓存',t.cacheRead+t.cacheWrite]].map(([label,value])=>`<div><span>${label}</span><strong>${compact(value)}</strong></div>`).join('')}</div></div></div>
  <div class="section-head"><h2>${state.days===1?'近 7 天趋势':'使用趋势'}</h2><label class="mode"><input type="checkbox" id="cost-mode" ${state.cost?'checked':''}> 按 API 等价成本</label></div>
  <div class="chart-row"><div class="card"><h2>每日用量 · 按模型</h2><canvas id="trend" aria-label="按模型堆叠的每日用量，下方有逐日数据"></canvas><div class="model-key">${trendModels.map(m=>`<span><i style="background:${color(m)}"></i>${escapeHTML(m)}</span>`).join('')}</div></div><div class="card"><h2>所选范围 · 模型分布</h2><div class="pie-wrap"><div class="donut" style="background:conic-gradient(${stops.length&&sum?stops.join(','):'var(--border) 0deg 360deg'})"><div class="donut-inner">${models.length}<small>模型</small></div></div><div class="legend">${models.map(m=>`<div><span class="color" style="background:${color(m.key)}"></span><span class="model">${escapeHTML(m.key)}</span><strong>${state.cost?money(m.cost):compact(m.total)}</strong></div>`).join('')}</div></div></div></div>
  <div class="card"><details class="daily" data-agent-detail="daily"><summary><h2>每日模型与缓存命中率</h2></summary>${dailyTable(d.trendDays,d.dayModels)}</details></div>
  <div class="card"><h2>过去 365 天</h2><div class="heatmap">${'<span></span>'.repeat(lead)}${d.heatmap.map(day=>{const n=state.cost?day.cost:day.total;return `<span style="background:${n?`color-mix(in srgb,var(--accent) ${20+80*Math.sqrt(n/maxHeat)}%,transparent)`:'var(--border)'}" title="${day.key} · ${state.cost?money(n):compact(n)+' Token'}"></span>`;}).join('')}</div><div class="heat-foot"><span>${d.heatmap[0]?.key??''}</span><span>少 ░ ▒ ▓ 多</span></div></div>
  ${state.settings.sources.length?'':'<div class="card empty"><button id="add-first-source" class="primary" title="新建第一个数据源">添加数据源</button></div>'} `;
  if($('#add-first-source'))$('#add-first-source').onclick=()=>{state.page='settings';state.settingsTab='sources';render();editItem();};
  $('#provider').onchange=async e=>{state.provider=e.target.value;state.accountKey='';state.sourceId='';state.model='';await loadDashboard();};
  $('#account').onchange=async e=>{state.accountKey=e.target.value;state.sourceId='';await loadDashboard();};
  $('#source').onchange=async e=>{state.sourceId=e.target.value;await loadDashboard();};
  $('#model').onchange=async e=>{state.model=e.target.value;await loadDashboard();};
  $('#days').onchange=async e=>{state.days=Number(e.target.value);await loadDashboard();};
  $('#cost-mode').onchange=e=>{state.cost=e.target.checked;renderAgent();};
  if($('#read-quotas'))$('#read-quotas').onclick=quotas;
  if($('#repair-pricing'))$('#repair-pricing').onclick=openPricing;
  document.querySelectorAll('[data-agent-detail]').forEach(e=>{e.open=opened.has(e.dataset.agentDetail);if(e.dataset.agentDetail===focusDetail)e.querySelector('summary').focus({preventScroll:true});});
  if(focusId)document.getElementById(focusId)?.focus({preventScroll:true});
  window.scrollTo(scroll.x,scroll.y);
  requestAnimationFrame(drawTrend);
}
function renderAgentPanel(d) {
  const s=d.summary,t=s.tokens;
  $('#content').innerHTML=`<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','无账户 / API',state.accountKey)}${state.settings.accounts.filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name+(a.archived?'（已归档）':''),state.accountKey)).join('')}</select></div>
  ${d.quotas.length?`<div class="section-head"><h2>账户限额 <span class="count">${d.quotas.length}</span></h2><button id="read-quotas" title="重新查询各账户的实时限额">刷新</button></div><div class="quotas">${d.quotas.map(quotaCard).join('')}</div>`:''}
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':`最近 ${n} 天`,state.days)).join('')}</select></div>
  <div class="stats panel-stats">${stat('总 Token',compact(s.total),'','✧')}${stat('API 等价成本'+(s.total>0&&s.pricedTokens<s.total?' <button id="repair-pricing" class="pricing-alert" aria-label="计价未完成，设置模型价格" title="部分 Token 尚未计价">!</button>':''),s.pricedTokens?money(s.cost):'—','','$')}</div>
  <div class="card panel-cache"><div class="between"><span class="muted">缓存命中率</span><strong>${pct(cacheRate(t))}</strong></div><div class="token-breakdown">${[['输入',t.input],['输出',t.output],['缓存',t.cacheRead+t.cacheWrite]].map(([label,value])=>`<div><span>${label}</span><strong>${compact(value)}</strong></div>`).join('')}</div></div>
  <div class="card"><h2>近 7 天用量</h2><canvas id="trend" class="panel-chart" aria-label="近 7 天按模型的每日用量"></canvas><div class="model-key">${[...new Set(d.dayModels.map(r=>r.model))].sort().map(m=>`<span><i style="background:${color(m)}"></i>${escapeHTML(m)}</span>`).join('')}</div><details class="daily" data-agent-detail="daily"><summary><h2>每日明细</h2></summary>${dailyTable(d.trendDays,d.dayModels)}</details></div>
  <button id="open-detail" class="panel-wide" title="打开完整详情窗口">用量详情</button>`;
  $('#provider').onchange=async e=>{state.provider=e.target.value;state.accountKey='';state.sourceId='';state.model='';await loadDashboard();};
  $('#account').onchange=async e=>{state.accountKey=e.target.value;state.sourceId='';await loadDashboard();};
  $('#days').onchange=async e=>{state.days=Number(e.target.value);await loadDashboard();};
  if($('#read-quotas'))$('#read-quotas').onclick=quotas;
  if($('#repair-pricing'))$('#repair-pricing').onclick=openPricing;
  $('#open-detail').onclick=()=>window.__TAURI__.core.invoke('desktop_action',{action:'open'}).catch(e=>notify(String(e)));
  const opened=new Set([...document.querySelectorAll('[data-agent-detail][open]')].map(e=>e.dataset.agentDetail));
  document.querySelectorAll('[data-agent-detail]').forEach(e=>{e.open=opened.has(e.dataset.agentDetail);});
  requestAnimationFrame(drawTrend);
}
function dailyTable(days,rows) {
  return `<div class="daily-table"><div class="day-head"><span>日期 / 模型</span><span>Token</span><span>缓存命中率</span><span>已计价成本</span></div>${[...days].reverse().map(day=>`<details class="day" data-agent-detail="day:${escapeHTML(day.key)}"><summary><span>${day.key}</span><strong>${compact(day.total)}</strong><span>${pct(cacheRate(day.tokens))}</span><span>${day.pricedTokens?money(day.cost):'—'}</span></summary>${rows.filter(r=>r.day===day.key).map(r=>`<div class="day-model"><span><i style="background:${color(r.model)}"></i>${escapeHTML(r.model)}</span><span>${compact(r.usage.total)}</span><span>${pct(cacheRate(r.usage.tokens))}</span><span>${r.usage.pricedTokens?money(r.usage.cost):'—'}</span></div>`).join('')||'<p class="muted tiny">当日暂无记录</p>'}</details>`).join('')}</div>`;
}
function quotaCard(q) {
  return `<div class="card"><div class="quota-title"><div><h3>${escapeHTML(q.name)}</h3><div class="sub">${escapeHTML(providers[q.provider] ?? q.provider)}${q.plan ? ' · '+escapeHTML(q.plan) : ''}</div></div><span class="tiny muted">${q.origin==='log'?'记录':'更新'} ${date(q.updatedAt)}</span></div>${(q.balances??[]).map(b=>`<div class="quota-window"><div class="between"><span>可用余额</span><strong>${escapeHTML(b.currency)} ${escapeHTML(b.total)}</strong></div><div class="between tiny muted"><span>赠送 ${escapeHTML(b.granted)}</span><span>充值 ${escapeHTML(b.toppedUp)}</span></div></div>`).join('')}${q.isAvailable===false?'<p class="error">当前余额不足以调用 API</p>':''}${q.windows.map(w=>`<div class="quota-window"><div class="between tiny"><span>${escapeHTML(w.name)}</span><strong>剩余 ${pct(Math.max(0,100-w.usedPercent))}</strong></div><div class="track"><span style="width:${Math.min(100,Math.max(0,100-w.usedPercent))}%;${w.usedPercent>=90?'background:#d79a4b':''}"></span></div><div class="between tiny muted"><span>重置于 ${date(w.resetsAt)}</span><span>${w.resetsAt ? w.resetsAt*1000>Date.now() ? `剩余 ${Math.ceil((w.resetsAt*1000-Date.now())/3600000)}h` : '等待同步' : ''}</span></div></div>`).join('')}${q.bankReset ? `<details class="bank" data-agent-detail="bank:${escapeHTML(q.provider+':'+q.accountId)}"><summary>Bank Reset · ${q.bankReset.availableCount} 次可用</summary>${(q.bankReset.credits ?? []).map(c=>`<div class="between tiny muted" style="margin-top:9px"><span>${escapeHTML(c.title ?? 'Reset Credit')}</span><span>${c.expiresAt ? '到期 '+date(c.expiresAt) : '无到期时间'}</span></div>`).join('')}</details>`:''}${q.error ? `<p class="error">${escapeHTML(q.error)}</p>`:''}</div>`;
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
  if(Date.now()-result.sample.timestamp*1000>10000)return '数据延迟';
  return Object.keys(result.sample.errors??{}).length?'部分采集失败':'正常';
}
function renderServers() {
  const opened=new Set([...document.querySelectorAll('[data-metric][open]')].map(e=>e.dataset.metric));
  $('#content').innerHTML = `<div class="section-head"><span class="muted">${state.settings.hosts.length} 台主机</span><button id="sample" title="立即采样所有已启用的服务器">刷新服务器</button></div>${state.settings.hosts.map(h=>{
    const result=state.hosts.find(r=>r.id===h.id),s=result?.sample,cpu=s?.cpu?.find(c=>c.id==='cpu'),status=hostStatus(h,result);
    return `<div class="card server-card"><div class="between"><div><h2 style="margin:0">${escapeHTML(h.name||h.target)}</h2><div class="sub">${escapeHTML(h.target)}</div></div><span class="host-status" data-status="${status}"><i></i>${status}</span></div>${s ? `<div class="server-gauges">${s.cpu?resourceRing('CPU',cpu?.utilization):''}${s.memory?resourceRing('内存',capacityPercent(s.memory.total-s.memory.available,s.memory.total),`${bytes(s.memory.total-s.memory.available)} / ${bytes(s.memory.total)}`):''}${(s.gpu??[]).map(g=>resourceRing(`GPU ${g.id}`,g.utilization,g.name??'')).join('')}</div>${Object.entries(groups).filter(([key])=>s[key]).map(([key,label])=>serverGroup(key,label,s[key],h)).join('')}<div class="between tiny muted"><span>负载 ${s.load.map(n=>n.toFixed(2)).join(' / ')}</span><span>${date(s.timestamp)}</span></div>${Object.keys(s.errors??{}).map(k=>`<p class="error">${groups[k]??escapeHTML(k)} · 采集失败</p>`).join('')}`:''}${result?.error?`<p class="error">${escapeHTML(result.error)}</p>`:''}</div>`;
  }).join('')||'<div class="card empty"><button id="add-first-host" class="primary" title="新建第一台服务器">添加服务器</button></div>'}`;
  document.querySelectorAll('[data-metric]').forEach(e=>{e.open=opened.has(e.dataset.metric);});
  $('#sample').onclick=sample;
  if($('#add-first-host'))$('#add-first-host').onclick=()=>{state.page='settings';state.settingsTab='hosts';render();editItem();};
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
  return prices.map(p=>`<div class="list-row"><div class="row-body">${escapeHTML(p.id)}<small>输入 ${p.input==null?'—':money(p.input*1e6)} · 输出 ${p.output==null?'—':money(p.output*1e6)}</small></div><button data-price="${escapeHTML(p.id)}" title="编辑该模型的价格">编辑</button></div>`).join('')||`<div class="empty">${query?'无匹配模型':'同步模型价格'}</div>`;
}
function bindPrices() { document.querySelectorAll('[data-price]').forEach(b=>b.onclick=()=>editPrice(state.prices.find(p=>p.id===b.dataset.price))); }
function sourceError(id) {
  const error=state.dashboard?.sources.find(s=>s.id===id)?.status?.error;
  return error?`<span class="error" role="status">${escapeHTML(error)}</span>`:'';
}
function accountRows(accounts) {
  return accounts.map(a=>{const i=state.settings.accounts.indexOf(a);return `<div class="list-row"><div class="row-body">${escapeHTML(a.name)}<small>${escapeHTML(providers[a.provider]??a.provider)}</small></div><button data-edit-account="${i}" title="编辑账户">编辑</button>${a.archived?`<button data-restore-account="${i}" title="把归档账户恢复为正常状态">恢复</button>`:`<button data-remove-account="${i}" aria-label="归档账户" title="归档账户并保留历史">归档</button>`}</div>`;}).join('');
}
function renderSettings() {
  if(state.settingsTab==='accounts')state.settingsTab='sources';
  if(state.settingsTab==='connection')state.settingsTab='general';
  const tabs={sources:'数据源',hosts:'服务器',prices:'价格',general:'通用'};
  let body='';const s=state.settings;
  if(state.settingsTab==='sources'||state.settingsTab==='hosts'){
    const isSource=state.settingsTab==='sources',list=isSource?s.sources:s.hosts;
    body=`<div class="card">${list.map(item=>`<div class="list-row"><input type="checkbox" data-enable="${escapeHTML(item.id)}" ${item.enabled?'checked':''} aria-label="启用 ${escapeHTML(item.name)}"><div class="row-body">${escapeHTML(item.name||item.target)}<small>${escapeHTML(isSource?`${providers[item.provider]} · ${item.accountId ? (s.accounts.find(a=>a.id===item.accountId&&a.provider===item.provider)?.name??item.accountId) : "无账户"}`:item.target)}</small>${isSource?sourceError(item.id):''}</div><button data-edit="${escapeHTML(item.id)}" title="编辑此项">编辑</button><button data-remove="${escapeHTML(item.id)}" aria-label="移除 ${escapeHTML(item.name)}" title="移除此项">−</button></div>`).join('')||'<div class="empty">添加第一个'+(isSource?'数据源':'主机')+'</div>'}</div><button id="add-item" class="primary" title="新建数据源或服务器">＋ 添加${isSource?'数据源':'主机'}</button>`;
    if(isSource){
      const orphan=s.accounts.filter(a=>!a.archived&&!s.sources.some(src=>src.accountId===a.id&&src.provider===a.provider)),archived=s.accounts.filter(a=>a.archived);
      if(orphan.length)body+=`<details class="card account-history"><summary>未关联账户</summary>${accountRows(orphan)}</details>`;
      if(archived.length)body+=`<details class="card account-history"><summary>已归档账户</summary>${accountRows(archived)}</details>`;
    }
  }else if(state.settingsTab==='prices')body=gapList()+`<div class="price-toolbar"><input id="price-search" type="search" aria-label="搜索模型" placeholder="搜索模型" value="${escapeHTML(state.priceSearch)}"><button id="sync-prices" title="从项目 Release 同步 OpenRouter 价格表">同步 OpenRouter</button><button id="add-price" title="手动添加一个模型价格">添加价格</button></div><div class="card prices-list">${priceRows()}</div><form id="mapping-form" class="card"><h2>模型映射</h2>${Object.entries(s.modelMappings).map(([from,to])=>`<div class="list-row tiny"><span>${escapeHTML(from)} → ${escapeHTML(to)}</span><button type="button" data-unmap="${escapeHTML(from)}">−</button></div>`).join('')}${field('model','日志模型名称')}${field('id','OpenRouter 模型 ID')}<div class="between"><button>添加映射</button><button id="reprice" type="button" title="按当前价格重新计算历史成本">按当前价格重算</button></div></form><span class="muted tiny">USD / 百万 Token</span>`;
  else body=(window.AieyesDesktop?.settingsHTML() || '')+`<form id="connection-form" class="card"><h2>连接</h2>${proxyFields('app',s.proxy)}<button class="primary">保存</button></form><form id="general-form" class="card"><h2>刷新</h2>${field('refreshSeconds','Agent 间隔（秒）',s.refreshSeconds,'','number')}${field('serverRefreshSeconds','服务器间隔（秒）',s.serverRefreshSeconds,'','number')}<button class="primary">保存</button></form><div class="card"><button id="updates" title="检查 GitHub Release 是否有新版本">检查更新</button><span id="update-result" role="status"></span></div>`;
  $('#content').innerHTML=`<div class="settings-tabs">${Object.entries(tabs).map(([k,v])=>`<button data-settings-tab="${k}" class="${state.settingsTab===k?'active':''}">${v}</button>`).join('')}</div><div class="settings-block">${body}</div>`;
  window.AieyesDesktop?.bindSettings();
  document.querySelectorAll('[data-settings-tab]').forEach(b=>b.onclick=async()=>{state.settingsTab=b.dataset.settingsTab;if(state.settingsTab==='prices')state.prices=await api('prices.list');await loadDashboard();renderSettings();});
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
  document.querySelectorAll('[data-gap-map]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapMap)];$('#field-model').value=g.model;$('#field-id').value=g.priceId??'';$('#field-id').focus();});
  const items=state.settingsTab==='sources'?s.sources:s.hosts;
  document.querySelectorAll('[data-edit]').forEach(b=>b.onclick=()=>editItem(items.find(i=>i.id===b.dataset.edit)));
  document.querySelectorAll('[data-enable]').forEach(b=>b.onchange=async()=>{
    const list=state.settingsTab==='sources'?'sources':'hosts',next=structuredClone(state.settings);
    next[list].find(i=>i.id===b.dataset.enable).enabled=b.checked;b.disabled=true;
    try { await job('保存设置',()=>saveSettings(next)); }
    finally { b.checked=state.settings[list].find(i=>i.id===b.dataset.enable)?.enabled??false;b.disabled=false; }
  });
  document.querySelectorAll('[data-remove]').forEach(b=>b.onclick=()=>job('保存设置',async()=>{
    const id=b.dataset.remove,next=structuredClone(state.settings),list=state.settingsTab==='sources'?next.sources:next.hosts;
    list.splice(list.findIndex(i=>i.id===id),1);
    if(state.settingsTab==='hosts')for(const src of next.sources)if(src.hostId===id){src.hostId=null;src.enabled=false;}
    if(state.settingsTab==='sources')for(const a of next.accounts)if(a.quotaSourceId===id)a.quotaSourceId=null;
    await saveSettings(next);renderSettings();
  }));
  if($('#add-item'))$('#add-item').onclick=()=>editItem();
  if($('#connection-form'))$('#connection-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);job('保存设置',async()=>{const next=structuredClone(state.settings);next.proxy=proxyValue(f,'app');await saveSettings(next);});};
  if($('#general-form'))$('#general-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);job('保存设置',async()=>{const next=structuredClone(state.settings);next.refreshSeconds=Number(f.get('refreshSeconds'));next.serverRefreshSeconds=Number(f.get('serverRefreshSeconds'));await saveSettings(next);});};
  if($('#updates'))$('#updates').onclick=()=>job('检查更新',async()=>{const u=await api('updates.check');const target=new URL(u.url);if(target.origin!=='https://github.com'||!target.pathname.startsWith('/JesmonX/Aieyes/releases/'))throw new Error('更新地址无效');$('#update-result').innerHTML=` ${escapeHTML(u.version)} · <a href="${escapeHTML(target.href)}" target="_blank" rel="noopener noreferrer">发布页</a>`;$('#update-result a').onclick=e=>{e.preventDefault();job('打开发布页',()=>api('updates.open'));};});
  if($('#sync-prices'))$('#sync-prices').onclick=syncPrices;
  if($('#add-price'))$('#add-price').onclick=()=>editPrice();
  bindPrices();
  if($('#price-search'))$('#price-search').oninput=e=>{state.priceSearch=e.target.value;$('.prices-list').innerHTML=priceRows();bindPrices();};
  if($('#connection-form'))bindProxy('app');
  if($('#mapping-form'))$('#mapping-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);if(!f.get('model')||!f.get('id'))return;job('保存映射',async()=>{const next=structuredClone(state.settings);next.modelMappings[f.get('model')]=f.get('id');await saveSettings(next);renderSettings();});};
  document.querySelectorAll('[data-unmap]').forEach(b=>b.onclick=()=>job('保存映射',async()=>{const next=structuredClone(state.settings);delete next.modelMappings[b.dataset.unmap];await saveSettings(next);renderSettings();}));
  if($('#reprice'))$('#reprice').onclick=()=>job('重新计价',async()=>{await api('prices.recalculate');await loadDashboard();renderSettings();notify('已按当前价格重算');});
}
let editorSelectors=[],editorSaving=false,editorOpener=null;
function clearEditorSelectors(){editorSelectors.forEach(c=>c.destroy());editorSelectors=[];}
function finishEditor(){clearEditorSelectors();$('.shell').inert=false;$('#editor-backdrop').hidden=true;document.body.classList.remove('editor-open');if(editorOpener?.isConnected)editorOpener.focus({preventScroll:true});}
function closeEditor(){if(editorSaving)return;$('#editor').close();finishEditor();}
function showEditor(title,html,onSubmit){
  if(editorSaving)return;
  editorOpener=document.activeElement;clearEditorSelectors();
  $('#editor-title').textContent=title;$('#editor-fields').innerHTML=html;
  $('#editor-error').hidden=true;$('#editor-error').textContent='';
  $('#editor-form button[type=submit]').textContent='保存';
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
  $('#editor-fields').querySelector('input:not([disabled]),select:not([disabled]),textarea:not([disabled])')?.focus();
}
document.addEventListener('keydown',event=>{
  if(!$('#editor')||!$('#editor').open||event.defaultPrevented)return;
  if(event.key==='Escape'){event.preventDefault();closeEditor();return;}
  if(event.key!=='Tab')return;
  const controls=[...document.querySelectorAll('#titlebar:not([hidden]) button,#editor button,#editor input,#editor select,#editor textarea')].filter(e=>!e.disabled&&e.getClientRects().length);
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
  const item=structuredClone(existing??(source?{id:crypto.randomUUID(),name:'',provider:'codex',accountId:'',path:'~/.codex',hostId:null,enabled:true,quotaCommand:'',quotaPreCommand:'',codexBinary:'codex',agyBinary:'agy',proxy:null}:{id:crypto.randomUUID(),name:'',target:'',port:null,identityFile:'',shell:'/bin/sh',preCommand:'',enabled:true,metrics:Object.keys(groups),devices:[],details:Object.keys(detailOptions)}));
  const accountOptions=provider=>[['__new__','新建账户'],...state.settings.accounts.filter(a=>a.provider===provider&&(!a.archived||a.id===item.accountId)).map(a=>[a.id,a.name+(a.archived?'（已归档）':'')])];
  const account=state.settings.accounts.find(a=>a.id===item.accountId&&a.provider===item.provider);
  const body=source?field('name','名称',item.name)+select('provider','Agent',Object.entries(providers),item.provider)+`<label class="account-toggle"><input type="checkbox" name="isAccount" id="field-isAccount" ${item.accountId?'checked':''}>作为账户</label><div id="account-fields">${select('accountId','账户',accountOptions(item.provider),item.accountId||'__new__')}${field('accountName','账户名称',account?.name??'')}${select('quotaEnabled','账户限额',[['yes','显示并查询'],['no','仅统计用量']],account?.quotaEnabled?'yes':'no')}${select('accountArchived','账户状态',[['no','正常'],['yes','已归档']],account?.archived?'yes':'no')}${select('quotaSourceId','优先查询位置',[['','自动 · 优先本机']],account?.quotaSourceId??'')}</div>`+select('hostId','位置',[['','本机'],...state.settings.hosts.map(h=>[h.id,h.name||h.target])],item.hostId??'')+field('path','数据目录',item.path)+field('codexBinary','Codex 程序',item.codexBinary)+field('agyBinary','agy 程序',item.agyBinary??'agy')+`<div id="deepseek-key">${field('apiKey','API Key（留空保留）','','','password')}</div><div id="remote-quota">${textarea('quotaPreCommand','限额查询前置命令',item.quotaPreCommand)}</div><div id="local-proxy">${proxyFields('source',item.proxy,true)}</div>`:
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
      const provider=e.target.value;if(provider==='deepseek'){$('#field-hostId').value='';$('#field-path').value='';}$('#field-apiKey').value='';
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
document.querySelectorAll('[data-page]').forEach(b=>b.onclick=()=>{state.page=b.dataset.page;render();if(state.page==='servers')sample();});
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
async function boot(){try{if(!window.__TAURI__)throw new Error('通过 Aieyes 桌面应用打开');const info=await api('hello');const version=$('#app-version');if(version)version.textContent=info.version;state.settings=await api('settings.get');await loadDashboard();if(PANEL)return;await scan();if(hasQuotaSources())await quotas();}catch(e){notify(String(e));$('#activity').textContent='连接已断开';}}
// The panel refreshes on open and on manual action; periodic polling stays in the details window.
if(!PANEL)setInterval(()=>{
  if(!state.settings||$('#editor').open)return;
  if(!state.serverBusy&&state.settings.hosts.some(h=>h.enabled)&&Date.now()-state.lastMetrics>(state.page==='servers'&&!document.hidden?2000:state.settings.serverRefreshSeconds*1000)){sample();}
  if(state.busy||state.settingsSaving)return;
  if(hasQuotaSources()&&(!state.lastQuota||Date.now()-state.lastQuota>state.settings.refreshSeconds*1000)){quotas();return;}
  if(Date.now()-state.lastScan>state.settings.refreshSeconds*1000){scan();return;}
},500);
boot();
