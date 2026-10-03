const $ = (s) => document.querySelector(s);
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
const state = {page:'agent',settingsTab:'accounts',settings:null,dashboard:null,hosts:[],prices:[],provider:'',sourceId:'',accountKey:'',model:'',days:1,cost:false,busy:false,serverBusy:false,lastScan:0,lastMetrics:0,lastQuota:0};
async function api(method, params = {}) { return window.__TAURI__.core.invoke('engine_call', {method,params}); }
function notify(message) { $('#message').textContent = message; $('#message').hidden = !message; }
async function job(label, fn) {
  if (state.busy) return;
  state.busy = true; $('#activity').textContent = label;
  for (const b of document.querySelectorAll('header button')) b.disabled = true;
  try { await fn(); } catch (e) { notify(String(e)); }
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
async function quotas() { await job('读取限额', async () => { await api('quotas.refresh'); state.lastQuota = Date.now(); await loadDashboard(); }); }
async function sample() { if(state.serverBusy)return; state.serverBusy=true; try {
  const rows = await api('hosts.sample');
  state.hosts = rows.map(r => r.error ? {...r,sample:state.hosts.find(p => p.id === r.id)?.sample} : r);
  state.lastMetrics = Date.now(); if (state.page === 'servers') render();
} catch(e) { notify(String(e)); state.lastMetrics=Date.now(); } finally { state.serverBusy=false; } }
async function saveSettings() { await api('settings.save',state.settings); await loadDashboard(); notify('已保存'); }
function option(value,label,current) { return `<option value="${escapeHTML(value)}" ${String(value) === String(current) ? 'selected' : ''}>${escapeHTML(label)}</option>`; }
function stat(label,value,detail,icon) { return `<div class="stat"><div class="stat-label">${label}<b>${icon}</b></div><div class="stat-value">${value}</div><small>${detail}</small></div>`; }
function render() {
  document.querySelectorAll('nav button').forEach(b => b.classList.toggle('active',b.dataset.page === state.page));
  $('#title').textContent = {agent:'使用概览',servers:'服务器',settings:'设置'}[state.page];
  if (!state.settings) return;
  if (state.page === 'agent') renderAgent(); else if (state.page === 'servers') renderServers(); else renderSettings();
}
const cacheRate = t => { const input=t.input+t.cacheRead+t.cacheWrite; return input ? t.cacheRead/input*100 : null; };
const missingLabels = gap => [['input','输入'],['output','输出'],['cacheRead','缓存读取'],['cacheWrite','缓存写入']].filter(([k])=>gap.tokens[k]>0).map(([,label])=>label).join('、');
function gapList() {
  const gaps=state.dashboard?.pricingGaps??[];
  return gaps.length ? `<div class="card pricing-gaps"><h2>所选范围 · 待计价模型</h2><p class="muted tiny">同步价格、映射到已有模型，或补充缺失单价。保存后自动补计缺项。</p>${gaps.map((g,i)=>`<div class="list-row"><div class="row-body"><strong>${escapeHTML(g.model)}</strong><small>缺少${missingLabels(g)} · ${compact(g.unpricedTokens)} Token</small></div><button data-gap-map="${i}">映射</button><button data-gap-price="${i}">补充价格</button></div>`).join('')}</div>` : '';
}
async function openPricing() { state.page='settings';state.settingsTab='prices';state.prices=await api('prices.list');render(); }
function renderAgent() {
  const d=state.dashboard;if(!d)return;
  const s=d.summary,t=s.tokens,models=d.models,sum=models.reduce((n,m)=>n+(state.cost?m.cost:m.total),0);
  let cursor=0;
  const stops=models.map(m=>{const start=cursor;cursor+=sum?(state.cost?m.cost:m.total)/sum*360:0;return `${color(m.key)} ${start}deg ${cursor}deg`;});
  const maxHeat=Math.max(1,...d.heatmap.map(v=>state.cost?v.cost:v.total));
  const lead=d.heatmap.length?(new Date(d.heatmap[0].key+'T12:00:00').getDay()+6)%7:0;
  const account=state.settings.accounts.find(a=>a.provider+':'+a.id===state.accountKey);
  const sources=state.settings.sources.filter(s=>(!state.provider||s.provider===state.provider)&&(!state.accountKey||(state.accountKey==='none'?!s.accountId:s.accountId===account?.id&&s.provider===account?.provider)));
  const trendModels=[...new Set(d.dayModels.map(r=>r.model))].sort();
  $('#title').textContent='Agent 概览';
  $('#content').innerHTML=`<div class="filters"><select id="provider" aria-label="Agent">${option('','全部 Agent',state.provider)}${Object.entries(providers).map(([k,v])=>option(k,v,state.provider)).join('')}</select><select id="account" aria-label="账户">${option('','全部账户',state.accountKey)}${option('none','无账户 / API',state.accountKey)}${state.settings.accounts.filter(a=>!state.provider||a.provider===state.provider).map(a=>option(a.provider+':'+a.id,a.name,state.accountKey)).join('')}</select><select id="source" aria-label="数据源">${option('','全部数据源',state.sourceId)}${sources.map(s=>option(s.id,s.name,state.sourceId)).join('')}</select><select id="model" aria-label="模型">${option('','全部模型',state.model)}${[...new Set([...trendModels,...(state.model?[state.model]:[])])].map(m=>option(m,m,state.model)).join('')}</select></div>
  ${d.quotas.length?`<div class="section-head"><h2>账户限额 <span class="count">${d.quotas.length}</span></h2><button id="read-quotas">刷新限额</button></div><div class="quotas">${d.quotas.map(quotaCard).join('')}</div>`:''}
  <div class="section-head overview-heading"><h2>${state.days===1?'今日概览':'使用概览'}</h2><select id="days" aria-label="时间范围">${[1,7,30,90,365].map(n=>option(n,n===1?'今日':`最近 ${n} 天`,state.days)).join('')}</select></div>
  <div class="stats">${stat('总 Token',compact(s.total),`${s.events} 条使用记录`,'✧')}${stat('API 等价成本',s.pricedTokens?money(s.cost):'—',s.total?`已计价 ${pct(s.pricedTokens/s.total*100)}`:'USD','$')}${stat('缓存命中率',pct(cacheRate(t)),`读取 ${compact(t.cacheRead)} Token`,'▱')}${stat('输出 Token',compact(t.output),`输入 ${compact(t.input+t.cacheRead+t.cacheWrite)}`,'↗')}</div>
  ${d.pricingGaps.length?`<button id="repair-pricing" class="pricing-notice"><span>${d.pricingGaps.length} 个模型待补价 · ${compact(s.total-s.pricedTokens)} Token</span><strong>补齐价格 →</strong></button>`:''}
  <div class="section-head"><h2>${state.days===1?'近 7 天趋势':'使用趋势'}</h2><label class="mode"><input type="checkbox" id="cost-mode" ${state.cost?'checked':''}> 按 API 等价成本</label></div>
  <div class="chart-row"><div class="card"><h2>每日用量 · 按模型</h2><canvas id="trend" aria-label="按模型堆叠的每日用量，下方有逐日数据"></canvas><div class="model-key">${trendModels.map(m=>`<span><i style="background:${color(m)}"></i>${escapeHTML(m)}</span>`).join('')}</div></div><div class="card"><h2>所选范围 · 模型分布</h2><div class="pie-wrap"><div class="donut" style="background:conic-gradient(${stops.length&&sum?stops.join(','):'var(--border) 0deg 360deg'})"><div class="donut-inner">${models.length}<small>模型</small></div></div><div class="legend">${models.map(m=>`<div><span class="color" style="background:${color(m.key)}"></span><span class="model">${escapeHTML(m.key)}</span><strong>${state.cost?money(m.cost):compact(m.total)}</strong></div>`).join('')}</div></div></div></div>
  <div class="card"><details class="daily"><summary><h2>每日模型与缓存命中率</h2><span class="muted tiny">展开 ${d.trendDays.length} 天明细</span></summary><div class="daily-table"><div class="day-head"><span>日期 / 模型</span><span>Token</span><span>缓存命中率</span><span>已计价成本</span></div>${[...d.trendDays].reverse().map(day=>`<details class="day"><summary><span>${day.key}</span><strong>${compact(day.total)}</strong><span>${pct(cacheRate(day.tokens))}</span><span>${day.pricedTokens?money(day.cost):'—'}</span></summary>${d.dayModels.filter(r=>r.day===day.key).map(r=>`<div class="day-model"><span><i style="background:${color(r.model)}"></i>${escapeHTML(r.model)}</span><span>${compact(r.usage.total)}</span><span>${pct(cacheRate(r.usage.tokens))}</span><span>${r.usage.pricedTokens?money(r.usage.cost):'—'}</span></div>`).join('')||'<p class="muted tiny">当日暂无记录</p>'}</details>`).join('')}</div></details></div>
  <div class="card"><h2>过去 365 天</h2><div class="heatmap">${'<span></span>'.repeat(lead)}${d.heatmap.map(day=>{const n=state.cost?day.cost:day.total;return `<span style="background:${n?`color-mix(in srgb,var(--accent) ${20+80*Math.sqrt(n/maxHeat)}%,transparent)`:'var(--border)'}" title="${day.key} · ${state.cost?money(n):compact(n)+' Token'}"></span>`;}).join('')}</div><div class="heat-foot"><span>${d.heatmap[0]?.key??''}</span><span>少 ░ ▒ ▓ 多</span></div></div>
  <div class="card"><h2>数据来源</h2>${d.sources.map(src=>`<div class="list-row"><span class="dot"></span><div class="row-body">${escapeHTML(src.name)}<small>${src.accountId?'账户 · '+escapeHTML(state.settings.accounts.find(a=>a.provider===src.provider&&a.id===src.accountId)?.name??src.accountId):'无账户 / 外接 API'} · ${escapeHTML(src.status?.error??`${src.status?.files??0} 个记录文件`)}</small></div><span class="tiny muted">${date(src.status?.updatedAt)}</span></div>`).join('')||'<div class="empty">在设置中添加第一个数据源</div>'}</div>`;
  $('#provider').onchange=async e=>{state.provider=e.target.value;state.accountKey='';state.sourceId='';state.model='';await loadDashboard();};
  $('#account').onchange=async e=>{state.accountKey=e.target.value;state.sourceId='';await loadDashboard();};
  $('#source').onchange=async e=>{state.sourceId=e.target.value;await loadDashboard();};
  $('#model').onchange=async e=>{state.model=e.target.value;await loadDashboard();};
  $('#days').onchange=async e=>{state.days=Number(e.target.value);await loadDashboard();};
  $('#cost-mode').onchange=e=>{state.cost=e.target.checked;renderAgent();};
  if($('#read-quotas'))$('#read-quotas').onclick=quotas;
  if($('#repair-pricing'))$('#repair-pricing').onclick=openPricing;
  requestAnimationFrame(drawTrend);
}
function quotaCard(q) {
  const linked=state.settings.sources.filter(s=>s.provider===q.provider&&s.accountId===q.accountId),location=linked.find(s=>s.id===q.sourceId)?.name;
  return `<div class="card"><div class="quota-title"><div><h3>${escapeHTML(q.name)}</h3><div class="sub">${escapeHTML(providers[q.provider] ?? q.provider)}${q.plan ? ' · '+escapeHTML(q.plan) : ''}</div></div><span class="tiny muted">${q.origin==='log'?'记录':'更新'} ${date(q.updatedAt)}</span></div>${(q.balances??[]).map(b=>`<div class="quota-window"><div class="between"><span>可用余额</span><strong>${escapeHTML(b.currency)} ${escapeHTML(b.total)}</strong></div><div class="between tiny muted"><span>赠送 ${escapeHTML(b.granted)}</span><span>充值 ${escapeHTML(b.toppedUp)}</span></div></div>`).join('')}${q.isAvailable===false?'<p class="error">当前余额不足以调用 API</p>':''}${q.windows.map(w=>`<div class="quota-window"><div class="between tiny"><span>${escapeHTML(w.name)}</span><strong>剩余 ${pct(Math.max(0,100-w.usedPercent))}</strong></div><div class="track"><span style="width:${Math.min(100,Math.max(0,100-w.usedPercent))}%;${w.usedPercent>=90?'background:#d79a4b':''}"></span></div><div class="between tiny muted"><span>重置于 ${date(w.resetsAt)}</span><span>${w.resetsAt ? w.resetsAt*1000>Date.now() ? `剩余 ${Math.ceil((w.resetsAt*1000-Date.now())/3600000)}h` : '等待同步' : ''}</span></div></div>`).join('')}${state.dashboard.quotas.filter(row=>row.provider===q.provider).length>1&&linked.length>1?`<p class="tiny muted">${linked.length} 个数据源${location?" · 查询自 "+escapeHTML(location):""}</p>`:""}${q.bankReset ? `<details class="bank"><summary>Bank Reset · ${q.bankReset.availableCount} 次可用</summary>${(q.bankReset.credits ?? []).map(c=>`<div class="between tiny muted" style="margin-top:9px"><span>${escapeHTML(c.title ?? 'Reset Credit')}</span><span>${c.expiresAt ? '到期 '+date(c.expiresAt) : '无到期时间'}</span></div>`).join('')}</details>`:''}${q.error ? `<p class="error">${escapeHTML(q.error)}</p>`:''}</div>`;
}
function drawTrend() {
  const canvas=$('#trend');if(!canvas||!state.dashboard)return;
  const width=canvas.clientWidth,height=canvas.clientHeight,dpr=window.devicePixelRatio||1;canvas.width=width*dpr;canvas.height=height*dpr;
  const ctx=canvas.getContext('2d');ctx.scale(dpr,dpr);
  const days=state.dashboard.trendDays,rows=state.dashboard.dayModels,values=days.map(d=>state.cost?d.cost:d.total),max=Math.max(1,...values);
  const left=58,top=12,bottom=30,plot=height-top-bottom,space=(width-left)/Math.max(1,days.length);
  const muted=getComputedStyle(canvas).getPropertyValue('--muted');ctx.font='11px system-ui';ctx.fillStyle=muted;ctx.textAlign='right';
  for(let i=0;i<3;i++){const y=top+plot*i/2;ctx.fillText(state.cost?money(max*(1-i/2)):compact(max*(1-i/2)),left-9,y+3);ctx.strokeStyle='rgba(140,145,170,.15)';ctx.beginPath();ctx.moveTo(left,y);ctx.lineTo(width,y);ctx.stroke();}
  days.forEach((d,i)=>{let used=0;const x=left+i*space+space*.2;rows.filter(r=>r.day===d.key).forEach(r=>{const h=(state.cost?r.usage.cost:r.usage.total)/max*plot;ctx.fillStyle=color(r.model);ctx.fillRect(x,top+plot-used-h,Math.max(1,space*.6),h);used+=h;});if(i%Math.max(1,Math.floor(days.length/7))===0){ctx.fillStyle=muted;ctx.textAlign='center';ctx.fillText(d.key.slice(5),x+space*.3,height-5);}});
  canvas.onmousemove=e=>{const i=Math.floor((e.offsetX-left)/space),day=days[i];canvas.title=day?`${day.key} · 缓存命中率 ${pct(cacheRate(day.tokens))}\n`+rows.filter(r=>r.day===day.key).map(r=>`${r.model}: ${state.cost?money(r.usage.cost):compact(r.usage.total)+' Token'}`).join('\n'):'';};
}
function renderServers() {
  const opened=new Set([...document.querySelectorAll('[data-metric][open]')].map(e=>e.dataset.metric));
  $('#content').innerHTML = `<div class="section-head"><span class="muted">${state.settings.hosts.length} 台主机</span><button id="sample">刷新服务器</button></div>${state.settings.hosts.map(h=>{
    const result=state.hosts.find(r=>r.id===h.id),s=result?.sample,cpu=s?.cpu?.find(c=>c.id==='cpu');
    return `<div class="card"><div class="between"><div><h2 style="margin:0">${escapeHTML(h.name||h.target)}</h2><div class="sub">${escapeHTML(h.target)}</div></div><span class="tiny muted">${h.enabled ? result?.error ? '连接失败' : s ? new Date(s.timestamp*1000).toLocaleTimeString('zh-CN')+(Date.now()-s.timestamp*1000>10000?' · 数据已延迟':'') : '等待采样' : '已暂停'}</span></div>${s ? `<div class="server-summary"><span>CPU ${pct(cpu?.utilization)}</span>${s.memory?`<span>内存 ${bytes(s.memory.total-s.memory.available)} / ${bytes(s.memory.total)}</span>`:''}<span>负载 ${s.load.map(n=>n.toFixed(2)).join(' / ')}</span></div>${Object.entries(groups).filter(([key])=>s[key]).map(([key,label])=>serverGroup(key,label,s[key],h)).join('')}${Object.keys(s.errors).map(k=>`<p class="error">${groups[k]??escapeHTML(k)} · 采集失败</p>`).join('')}`:''}${result?.error?`<p class="error">${escapeHTML(result.error)}</p>`:''}</div>`;
  }).join('')||'<div class="card empty"><strong>连接你的服务器</strong>通过 SSH 查看资源与流量</div>'}`;
  document.querySelectorAll('[data-metric]').forEach(e=>{e.open=opened.has(e.dataset.metric);});
  $('#sample').onclick=sample;
}
function serverGroup(key,label,value,host) {
  const show=key=>host.details==null||host.details.includes(key);
  const row=(name,value)=>`<div><span>${escapeHTML(name)}</span><strong>${value}</strong></div>`;
  let content='';
  if(key==='memory')content=row('可用',bytes(value.available))+(show('memoryCache')?row('缓存 / Buffer',`${bytes(value.cached)} / ${bytes(value.buffers)}`):'')+(show('swap')?row('Swap',`${bytes(value.swapTotal-value.swapFree)} / ${bytes(value.swapTotal)}`):'');
  else content=value.map(d=>{
    if(key==='cpu')return row(d.id,pct(d.utilization))+(show("cpuTimes")?row("user / system",`${pct(d.userPercent)} / ${pct(d.systemPercent)}`)+row("iowait / steal",`${pct(d.iowaitPercent)} / ${pct(d.stealPercent)}`):"");
    if(key==='gpu')return row(d.name??`GPU ${d.id}`,pct(d.utilization))+(show('gpuMemory')?row('显存',`${bytes(d.memoryUsedMiB==null?null:d.memoryUsedMiB*1048576)} / ${bytes(d.memoryTotalMiB==null?null:d.memoryTotalMiB*1048576)}`):'')+(show('gpuThermals')?row('温度 / 功耗',`${d.temperature??'—'}°C / ${d.powerWatts??'—'} W`):'');
    if(key==='filesystems')return row(d.id,`${bytes(d.used)} / ${bytes(d.total)}`)+(show('fsAvailable')?row('可用',bytes(d.available)):'')+(show('fsType')?row(d.device??'',escapeHTML(d.type??'')):'')+(show('inodes')?row('inode',d.inodes?pct((d.inodes-d.inodesFree)/d.inodes*100):'—'):'');
    if(key==='disk')return row(d.id,`读 ${speed(d.readBytesPerSecond)} · 写 ${speed(d.writeBytesPerSecond)}`)+(show('diskIops')?row('IOPS 读 / 写',`${d.readIops==null?'—':compact(d.readIops)} / ${d.writeIops==null?'—':compact(d.writeIops)}`):'')+(show('diskBusy')?row('忙碌率',pct(d.busyMsPerSecond==null?null:Math.min(100,d.busyMsPerSecond/10))):'');
    return row(d.id,`↓ ${speed(d.rxBytesPerSecond)} · ↑ ${speed(d.txBytesPerSecond)}`)+(show('networkTotals')?row('累计接收 / 发送',`${bytes(d.rxBytes)} / ${bytes(d.txBytes)}`):'')+(show('networkErrors')?row('错误 / 丢包',`${compact((d.rxErrors??0)+(d.txErrors??0))} / ${compact((d.rxDrops??0)+(d.txDrops??0))}`):'');
  }).join('');
  return `<details class="metric-section" data-metric="${escapeHTML(host.id+':'+key)}"><summary>${label}<span class="muted">${Array.isArray(value)?value.length+' 项':''}</span></summary><div class="metric-detail">${content}</div></details>`;
}
function field(name,label,value='',placeholder='',type='text') {return `<div class="form-row"><label for="field-${name}">${label}</label><input id="field-${name}" name="${name}" type="${type}" value="${escapeHTML(value)}" placeholder="${escapeHTML(placeholder)}"></div>`;}
function select(name,label,entries,current) {return `<div class="form-row"><label for="field-${name}">${label}</label><select id="field-${name}" name="${name}">${entries.map(([k,v])=>option(k,v,current)).join('')}</select></div>`;}
function textarea(name,label,value='') {return `<div class="form-row"><label for="field-${name}">${label}</label><textarea id="field-${name}" name="${name}">${escapeHTML(value)}</textarea></div>`;}
function renderSettings() {
  const tabs={accounts:'账户',sources:'数据源',hosts:'服务器',connection:'连接',prices:'价格',general:'通用'};
  let body='';const s=state.settings;
  if(state.settingsTab==='accounts') {
    body=`<p class="muted">账户管理身份与限额；数据源管理记录目录。同一账户可关联本机与服务器上的多个目录。</p><div class="card">${s.accounts.map((a,i)=>`<div class="list-row"><div class="row-body"><strong>${escapeHTML(a.name)}</strong><small>${providers[a.provider]} · ${s.sources.filter(src=>src.provider===a.provider&&src.accountId===a.id).length} 个数据源 · ${a.quotaEnabled?'显示限额':'仅统计用量'}</small></div><button data-edit-account="${i}">编辑</button><button data-remove-account="${i}" aria-label="移除账户">−</button></div>`).join('')||'<div class="empty">添加个人账户或工作账户</div>'}</div><button id="add-account" class="primary">＋ 添加账户</button> <button data-add-query="agy">接入 agy</button> <button data-add-query="deepseek">接入 DeepSeek</button><p class="tiny muted">本机两个 Codex 账户：添加两个账户，再分别关联各自的数据目录。外接 API 可直接添加无账户的数据源。</p>`;
  }else if(state.settingsTab==='sources'||state.settingsTab==='hosts'){
    const isSource=state.settingsTab==='sources',list=isSource?s.sources:s.hosts;
    body=`<div class="card">${list.map(item=>`<div class="list-row"><input type="checkbox" data-enable="${escapeHTML(item.id)}" ${item.enabled?'checked':''} aria-label="启用 ${escapeHTML(item.name)}"><div class="row-body">${escapeHTML(item.name||item.target)}<small>${escapeHTML(isSource?`${providers[item.provider]} · ${item.accountId ? "账户："+(s.accounts.find(a=>a.id===item.accountId&&a.provider===item.provider)?.name??item.accountId) : "无账户 / API"} · ${item.path}`:item.target)}</small></div><button data-edit="${escapeHTML(item.id)}">编辑</button><button data-remove="${escapeHTML(item.id)}" aria-label="移除 ${escapeHTML(item.name)}">−</button></div>`).join('')||'<div class="empty">添加第一个'+(isSource?'数据源':'主机')+'</div>'}</div><button id="add-item" class="primary">＋ 添加${isSource?'数据源':'主机'}</button>`;
  }else if(state.settingsTab==='connection')body=`<form id="connection-form" class="card"><h2>应用代理</h2>${select('mode','连接方式',[['system','系统代理'],['direct','直连'],['custom','自定义代理']],s.proxy.mode)}${field('url','代理地址',s.proxy.url,'http://127.0.0.1:7890')}<p class="tiny muted">用于限额查询、模型价格与 GitHub 更新。远程代理在数据源的限额查询前置命令中设置。</p><button class="primary">保存</button></form>`;
  else if(state.settingsTab==='prices')body=gapList()+`<div class="between" style="margin-bottom:15px"><span class="muted tiny">USD / 百万 Token</span><div class="actions"><button id="sync-prices">同步 OpenRouter</button><button id="add-price">添加价格</button></div></div><div class="card prices-list">${state.prices.map(p=>`<div class="list-row"><div class="row-body">${escapeHTML(p.id)}<small>输入 ${p.input==null?'—':money(p.input*1e6)} · 输出 ${p.output==null?'—':money(p.output*1e6)}</small></div><button data-price="${escapeHTML(p.id)}">编辑</button></div>`).join('')||'<div class="empty">同步模型价格</div>'}</div><form id="mapping-form" class="card"><h2>模型映射</h2>${Object.entries(s.modelMappings).map(([from,to])=>`<div class="list-row tiny"><span>${escapeHTML(from)} → ${escapeHTML(to)}</span><button type="button" data-unmap="${escapeHTML(from)}">−</button></div>`).join('')}${field('model','日志模型名称')}${field('id','OpenRouter 模型 ID')}<div class="between"><button>添加映射</button><button id="reprice" type="button">按当前价格重算</button></div></form>`;
  else body=(window.AieyesDesktop?.settingsHTML() || '')+`<form id="general-form" class="card"><h2>刷新与更新</h2>${field('refreshSeconds','Agent 间隔（秒）',s.refreshSeconds,'','number')}${field('serverRefreshSeconds','服务器间隔（秒）',s.serverRefreshSeconds,'','number')}${field('githubRepository','GitHub 仓库',s.githubRepository,'owner/repo')}<div class="between"><button class="primary">保存</button><button type="button" id="updates">检查更新</button></div></form>`;
  $('#content').innerHTML=`<div class="settings-tabs">${Object.entries(tabs).map(([k,v])=>`<button data-settings-tab="${k}" class="${state.settingsTab===k?'active':''}">${v}</button>`).join('')}</div><div class="settings-block">${body}</div>`;
  window.AieyesDesktop?.bindSettings();
  document.querySelectorAll('[data-settings-tab]').forEach(b=>b.onclick=async()=>{state.settingsTab=b.dataset.settingsTab;if(state.settingsTab==='prices')state.prices=await api('prices.list');await loadDashboard();renderSettings();});
  document.querySelectorAll('[data-add-query]').forEach(b=>b.onclick=async()=>{const provider=b.dataset.addQuery,a={id:crypto.randomUUID(),name:providers[provider],provider,quotaEnabled:true,quotaSourceId:null};const next=structuredClone(state.settings);next.accounts.push(a);try{await api('settings.save',next);state.settings=next;state.settingsTab='sources';renderSettings();editItem({id:crypto.randomUUID(),name:providers[provider]+' · 本机',provider,accountId:a.id,path:'',hostId:null,enabled:true,quotaCommand:'',quotaPreCommand:'',codexBinary:'codex',agyBinary:'agy',proxy:null});}catch(e){notify(String(e));}});
  if($('#add-account'))$('#add-account').onclick=()=>editAccount();
  document.querySelectorAll('[data-edit-account]').forEach(b=>b.onclick=()=>editAccount(Number(b.dataset.editAccount)));
  document.querySelectorAll('[data-remove-account]').forEach(b=>b.onclick=()=>job('保存设置',async()=>{const a=s.accounts[Number(b.dataset.removeAccount)];s.accounts.splice(Number(b.dataset.removeAccount),1);for(const src of s.sources)if(src.provider===a.provider&&src.accountId===a.id)src.accountId='';await saveSettings();renderSettings();}));
  document.querySelectorAll('[data-gap-price]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapPrice)];editPrice(state.prices.find(p=>p.id===(g.priceId??g.model))??{id:g.priceId??g.model,name:g.model});});
  document.querySelectorAll('[data-gap-map]').forEach(b=>b.onclick=()=>{const g=state.dashboard.pricingGaps[Number(b.dataset.gapMap)];$('#field-model').value=g.model;$('#field-id').value=g.priceId??'';$('#field-id').focus();});
  const items=state.settingsTab==='sources'?s.sources:s.hosts;
  document.querySelectorAll('[data-edit]').forEach(b=>b.onclick=()=>editItem(items.find(i=>i.id===b.dataset.edit)));
  document.querySelectorAll('[data-enable]').forEach(b=>b.onchange=()=>job('保存设置',async()=>{items.find(i=>i.id===b.dataset.enable).enabled=b.checked;await saveSettings();}));
  document.querySelectorAll('[data-remove]').forEach(b=>b.onclick=()=>job('保存设置',async()=>{
    const id=b.dataset.remove;items.splice(items.findIndex(i=>i.id===id),1);
    if(state.settingsTab==='hosts')for(const src of s.sources)if(src.hostId===id){src.hostId=null;src.enabled=false;}
    if(state.settingsTab==='sources')for(const a of s.accounts)if(a.quotaSourceId===id)a.quotaSourceId=null;
    await saveSettings();renderSettings();
  }));
  if($('#add-item'))$('#add-item').onclick=()=>editItem();
  if($('#connection-form'))$('#connection-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);job('保存设置',async()=>{s.proxy={mode:f.get('mode'),url:f.get('url')};await saveSettings();});};
  if($('#general-form'))$('#general-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);job('保存设置',async()=>{s.refreshSeconds=Number(f.get('refreshSeconds'));s.serverRefreshSeconds=Number(f.get('serverRefreshSeconds'));s.githubRepository=f.get('githubRepository');await saveSettings();});};
  if($('#updates'))$('#updates').onclick=()=>job('检查更新',async()=>{const u=await api('updates.check');notify('最新版本 '+u.version+' · '+u.url);});
  if($('#sync-prices'))$('#sync-prices').onclick=()=>job('同步价格',async()=>{await api('prices.sync');state.prices=await api('prices.list');await loadDashboard();renderSettings();notify('价格已更新');});
  if($('#add-price'))$('#add-price').onclick=()=>editPrice();
  document.querySelectorAll('[data-price]').forEach(b=>b.onclick=()=>editPrice(state.prices.find(p=>p.id===b.dataset.price)));
  if($('#mapping-form'))$('#mapping-form').onsubmit=e=>{e.preventDefault();const f=new FormData(e.target);if(!f.get('model')||!f.get('id'))return;job('保存映射',async()=>{s.modelMappings[f.get('model')]=f.get('id');await saveSettings();renderSettings();});};
  document.querySelectorAll('[data-unmap]').forEach(b=>b.onclick=()=>job('保存映射',async()=>{delete s.modelMappings[b.dataset.unmap];await saveSettings();renderSettings();}));
  if($('#reprice'))$('#reprice').onclick=()=>job('重新计价',async()=>{await api('prices.recalculate');await loadDashboard();renderSettings();notify('已按当前价格重算');});
}
function showEditor(title,html,onSubmit){$('#editor-title').textContent=title;$('#editor-fields').innerHTML=html;$('#editor-form').onsubmit=async e=>{e.preventDefault();try{await onSubmit(new FormData(e.target));$('#editor').close();}catch(err){notify(String(err));}};$('#editor').showModal();}
function editAccount(index) {
  const a=structuredClone(index==null?{id:crypto.randomUUID(),name:'',provider:'codex',quotaEnabled:true,quotaSourceId:null}:state.settings.accounts[index]);
  const linked=state.settings.sources.filter(s=>s.accountId===a.id&&s.provider===a.provider);
  showEditor('账户',field('name','账户名称',a.name,'个人账户 / 工作账户')+select('provider','Agent',Object.entries(providers),a.provider)+select('quotaEnabled','账户限额',[['yes','显示并查询'],['no','仅统计用量']],a.quotaEnabled?'yes':'no')+select('quotaSourceId','优先查询位置',[['','自动 · 优先本机'],...linked.map(s=>[s.id,s.name])],a.quotaSourceId??'')+'<p class="muted tiny">同一账户只显示一份限额。查询失败会尝试其他已关联的数据源。Codex、Claude Code、agy 支持限额读取，DeepSeek 支持余额查询。</p>',async f=>{
    a.name=f.get('name').trim();if(!a.name)throw new Error('请输入账户名称');if(!linked.length)a.provider=f.get('provider');a.quotaEnabled=f.get('quotaEnabled')==='yes';a.quotaSourceId=f.get('quotaSourceId')||null;
    const next=structuredClone(state.settings);if(index==null)next.accounts.push(a);else next.accounts[index]=a;
    await api('settings.save',next);state.settings=next;await loadDashboard();renderSettings();notify('已保存');
  });
  $('#field-provider').disabled=linked.length>0;
  $('#field-provider').onchange=e=>{$('#field-quotaEnabled').value=['codex','claude','agy','deepseek'].includes(e.target.value)?'yes':'no';};
}
function editItem(existing){
  const source=state.settingsTab==='sources';
  const item=structuredClone(existing??(source?{id:crypto.randomUUID(),name:'',provider:'codex',accountId:'',path:'~/.codex',hostId:null,enabled:true,quotaCommand:'',quotaPreCommand:'',codexBinary:'codex',agyBinary:'agy',proxy:null}:{id:crypto.randomUUID(),name:'',target:'',port:null,identityFile:'',shell:'/bin/sh',preCommand:'',enabled:true,metrics:Object.keys(groups),devices:[],details:Object.keys(detailOptions)}));
  const accountOptions=provider=>[['','无账户 / 外接 API'],...state.settings.accounts.filter(a=>a.provider===provider).map(a=>[a.id,a.name])];
  const body=source?field('name','名称',item.name)+select('provider','Agent',Object.entries(providers),item.provider)+select('accountId','关联账户',accountOptions(item.provider),item.accountId)+`<p class="tiny muted">在「账户」中添加身份。同一账户的本机和远程目录选择同一项；本机多个账户使用各自目录。修改关联后需同步记录。</p>`+select('hostId','位置',[['','本机'],...state.settings.hosts.map(h=>[h.id,h.name||h.target])],item.hostId??'')+field('path','数据目录',item.path)+field('codexBinary','Codex 程序',item.codexBinary)+field('agyBinary','agy 程序',item.agyBinary??'agy')+`<div id="deepseek-key">${field('apiKey','DeepSeek API Key（留空保留）','','','password')}<p class="tiny muted">API Key 保存为本机私有凭据文件。未设置时读取 DEEPSEEK_API_KEY 环境变量。</p></div>`+`<div id="remote-quota">${textarea('quotaPreCommand','限额查询前置命令',item.quotaPreCommand)}<p class="tiny muted">例如：export HTTPS_PROXY=http://127.0.0.1:7890<br>在远程限额查询的同一 shell 中执行，用于加载代理环境；留空继承主机前置命令。限额由应用自动读取。</p></div><div id="local-proxy">${select('proxyMode','查询代理',[['inherit','跟随应用'],['system','系统代理'],['direct','直连'],['custom','自定义']],item.proxy?.mode??'inherit')+field('proxyURL','代理地址',item.proxy?.url??'')}</div>`:
    field('name','名称',item.name)+field('target','SSH 别名或地址',item.target,'my-server 或 user@host')+field('port','端口',item.port??'','跟随 SSH 配置','number')+field('identityFile','密钥路径',item.identityFile,'跟随 SSH 配置')+field('shell','远程 shell',item.shell)+textarea('preCommand','远程前置命令',item.preCommand)+`<div class="form-row"><label>采集项目</label><div class="check-group">${Object.entries(groups).map(([k,v])=>`<label><input type="checkbox" name="metrics" value="${k}" ${item.metrics.includes(k)?'checked':''}> ${v}</label>`).join('')}</div></div>`+`<button type="button" id="discover-devices">读取挂载点与设备</button><p id="discovery-status" class="tiny muted"></p><div id="discovered-devices"></div>`+field('devices','指定设备',item.devices.join(', '),'network:eth0, gpu:0, filesystems:/')+`<p class="tiny muted">留空显示全部；读取设备后勾选需要显示的项目，也可手填。</p><h3>显示细分项</h3><div class="check-group">${Object.entries(detailOptions).map(([k,v])=>`<label><input type="checkbox" name="details" value="${k}" ${item.details==null||item.details.includes(k)?'checked':''}> ${v}</label>`).join('')}</div>`;
  showEditor(source?'数据源':'SSH 主机',body,async f=>{
    if(source){for(const k of ['name','provider','accountId','path','codexBinary','agyBinary','quotaPreCommand'])item[k]=f.get(k);item.hostId=f.get('hostId')||null;item.proxy=f.get('proxyMode')==='inherit'?null:{mode:f.get('proxyMode'),url:f.get('proxyURL')};if(!item.name)item.name=providers[item.provider];if(['agy','deepseek'].includes(item.provider)&&!item.accountId)throw new Error('请选择要查询的账户，或从账户页快捷接入');if(item.provider==='deepseek'){item.hostId=null;if(f.get('apiKey')?.trim()){const saved=await api('credentials.save',{sourceId:item.id,apiKey:f.get('apiKey')});item.path=saved.path;}}}
    else{for(const k of ['name','target','identityFile','shell','preCommand'])item[k]=f.get(k);item.port=f.get('port')?Number(f.get('port')):null;item.metrics=f.getAll('metrics');item.details=f.getAll('details');item.devices=f.get('devices').split(',').map(s=>s.trim()).filter(Boolean);if(!item.name)item.name=item.target;}
    const next=structuredClone(state.settings),items=source?next.sources:next.hosts,index=items.findIndex(i=>i.id===item.id);if(index<0)items.push(item);else items[index]=item;
    if(source)for(const a of next.accounts)if(a.quotaSourceId===item.id&&(a.id!==item.accountId||a.provider!==item.provider))a.quotaSourceId=null;
    await api('settings.save',next);state.settings=next;await loadDashboard();renderSettings();notify('已保存');
  });
  if(!source){
    $('#discover-devices').onclick=async()=>{
      const button=$('#discover-devices');button.disabled=true;$('#discovery-status').textContent='读取中…';
      const f=new FormData($('#editor-form')),host={...item};
      for(const k of ['target','identityFile','shell','preCommand'])host[k]=f.get(k);
      host.port=f.get('port')?Number(f.get('port')):null;
      try {
        const sample=await api('hosts.discover',{host});
        const selected=()=>$('#field-devices').value.split(',').map(s=>s.trim()).filter(Boolean);
        const available=Object.entries(groups).filter(([k])=>Array.isArray(sample[k]));
        $('#discovered-devices').innerHTML=available.map(([k,v])=>`<details open><summary>${v}</summary><div class="check-group">${sample[k].filter(d=>k!=='cpu'||d.id!=='cpu').map(d=>`<label><input type="checkbox" data-device-group="${k}" value="${escapeHTML(d.id)}" ${!selected().some(s=>s.startsWith(k+':'))||selected().includes(k+':'+d.id)?'checked':''}> ${escapeHTML(d.id)} ${escapeHTML(d.type??d.name??'')}</label>`).join('')}</div><button type="button" data-all-devices="${k}">显示全部</button></details>`).join('');
        $('#discovery-status').textContent=Object.keys(sample.errors??{}).map(k=>(groups[k]??k)+'采集失败').join(' · ')||'已读取，勾选需要显示的设备';
        document.querySelectorAll('[data-device-group]').forEach(c=>c.onchange=()=>{const group=c.dataset.deviceGroup,values=selected().filter(s=>!s.startsWith(group+':'));const chosen=[...document.querySelectorAll('[data-device-group]')].filter(e=>e.dataset.deviceGroup===group&&e.checked).map(e=>group+':'+e.value);$('#field-devices').value=[...values,...(chosen.length?chosen:[group+':__none__'])].join(', ');});
        document.querySelectorAll('[data-all-devices]').forEach(b=>b.onclick=()=>{const group=b.dataset.allDevices;$('#field-devices').value=selected().filter(s=>!s.startsWith(group+':')).join(', ');document.querySelectorAll('[data-device-group]').forEach(c=>{if(c.dataset.deviceGroup===group)c.checked=true;});});
      } catch(e) { $('#discovery-status').textContent=String(e); }
      finally {button.disabled=false;}
    };
  }
  if(source){
    const update=()=>{const provider=$('#field-provider').value;$('#deepseek-key').hidden=provider!=='deepseek';$('#field-agyBinary').closest('.form-row').hidden=provider!=='agy';$('#field-path').closest('.form-row').hidden=provider==='agy';document.querySelector('label[for=field-path]').textContent=provider==='deepseek'?'API Key 文件（可选）':'数据目录';$('#field-hostId').closest('.form-row').hidden=provider==='deepseek';$('#remote-quota').hidden=!$('#field-hostId').value||!$('#field-accountId').value;$('#local-proxy').hidden=!!$('#field-hostId').value;$('#field-codexBinary').closest('.form-row').hidden=$('#field-provider').value!=='codex';};
    $('#field-hostId').onchange=$('#field-accountId').onchange=update;
    $('#field-provider').onchange=e=>{const provider=e.target.value;if(provider==='deepseek'){$('#field-hostId').value='';$('#field-path').value='';}$('#field-accountId').innerHTML=accountOptions(provider).map(([k,v])=>option(k,v,'')).join('');if(['~/.codex','~/.claude',''].includes($('#field-path').value))$('#field-path').value=provider==='codex'?'~/.codex':provider==='claude'?'~/.claude':'';update();};update();
  }
}
function editPrice(existing){const price=structuredClone(existing??{id:'',name:'',input:null,output:null,cacheRead:null,cacheWrite:null,fetchedAt:0});
  showEditor('模型价格 · USD / 百万 Token',field('id','模型 ID',price.id)+field('name','显示名称',price.name)+[['input','输入'],['output','输出'],['cacheRead','缓存读取'],['cacheWrite','缓存写入']].map(([k,label])=>field(k,label,price[k]==null?'':price[k]*1e6,'待定价')).join(''),async f=>{price.id=f.get('id');price.name=f.get('name');for(const k of ['input','output','cacheRead','cacheWrite']){const v=f.get(k);price[k]=v===''?null:Number(v)/1e6;if(price[k]!=null&&(!Number.isFinite(price[k])||price[k]<0))throw new Error('价格需要为非负数');}await api('prices.save',{prices:[price]});state.prices=await api('prices.list');await loadDashboard();renderSettings();});
}
document.querySelectorAll('[data-page]').forEach(b=>b.onclick=()=>{state.page=b.dataset.page;render();if(state.page==='servers')sample();});
$('#scan').onclick=scan;$('#quota').onclick=quotas;$('#message').onclick=()=>notify('');$('#close-editor').onclick=$('#cancel-editor').onclick=()=>$('#editor').close();
new ResizeObserver(()=>{if(state.page==='agent')drawTrend();}).observe($('#content'));
async function boot(){try{if(!window.__TAURI__)throw new Error('通过 Aieyes 桌面应用打开');state.settings=await api('settings.get');await loadDashboard();await scan();}catch(e){notify(String(e));$('#activity').textContent='连接已断开';}}
setInterval(()=>{
  if(!state.settings||$('#editor').open)return;
  if(!state.serverBusy&&state.settings.hosts.some(h=>h.enabled)&&Date.now()-state.lastMetrics>(state.page==='servers'&&!document.hidden?2000:state.settings.serverRefreshSeconds*1000)){sample();}
  if(state.busy)return;
  if(Date.now()-state.lastScan>state.settings.refreshSeconds*1000){scan();return;}
  if(state.lastQuota&&Date.now()-state.lastQuota>state.settings.refreshSeconds*1000)quotas();
},500);
boot();
