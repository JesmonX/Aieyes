/* Accessible equivalents and interaction for the usage charts. */
const hiddenTrendModels=new Set();
function exactUsage(day){return `${day.total.toLocaleString()} Token · ${day.cost.toFixed(6)} USD · 读取命中率 ${pct(cacheRate(day.tokens))}`;}
function modelDistribution(models){
  const ranked=[...models].sort((a,b)=>(state.cost?b.cost-a.cost:b.total-a.total)||a.key.localeCompare(b.key));
  const sum=ranked.reduce((n,m)=>n+(state.cost?m.cost:m.total),0),top=ranked.slice(0,5),other=ranked.slice(5);
  const slices=[...top,...(other.length?[{key:'其他',total:other.reduce((n,m)=>n+m.total,0),cost:other.reduce((n,m)=>n+m.cost,0)}]:[])];
  let cursor=0;const stops=slices.map(m=>{const start=cursor;cursor+=sum?(state.cost?m.cost:m.total)/sum*360:0;return `${m.key==='其他'?'var(--muted)':color(m.key)} ${start}deg ${cursor}deg`;});
  const row=m=>`<div><span class="color" style="background:${color(m.key)}"></span><span class="model" title="${escapeHTML(m.key)}">${escapeHTML(m.key)}</span><strong title="${state.cost?m.cost.toFixed(6)+' USD':m.total.toLocaleString()+' Token'}">${state.cost?money(m.cost):compact(m.total)} <small>${pct(sum?(state.cost?m.cost:m.total)/sum*100:0)}</small></strong><button class="copy-value" data-copy-value="${escapeHTML(m.key+'\t'+m.total+' Token\t'+m.cost+' USD')}" aria-label="复制 ${escapeHTML(m.key)} 的精确数值">复制</button></div>`;
  return `<div class="pie-wrap"><div class="donut" role="img" aria-label="前五模型与其他，精确数据见右侧列表" style="background:conic-gradient(${sum?stops.join(','):'var(--border) 0deg 360deg'})"><div class="donut-inner">${models.length}<small>模型</small></div></div><div class="legend">${top.map(row).join('')}${other.length?`<details><summary>其他 ${other.length} 个模型 · ${state.cost?money(other.reduce((n,m)=>n+m.cost,0)):compact(other.reduce((n,m)=>n+m.total,0))} · ${pct(sum?other.reduce((n,m)=>n+(state.cost?m.cost:m.total),0)/sum*100:0)}</summary>${other.map(row).join('')}</details>`:''}</div></div>`;
}
function heatmapHTML(days){
  const lead=days.length?(new Date(days[0].key+'T12:00:00').getDay()+6)%7:0,columns=Math.ceil((days.length+lead)/7),max=Math.max(1,...days.map(d=>state.cost?d.cost:d.total));
  const months=days.flatMap((d,i)=>i===0||d.key.slice(8)==='01'?[`<span style="grid-column:${Math.floor((i+lead)/7)+1}">${Number(d.key.slice(5,7))}月</span>`]:[]).join('');
  return `<div class="heat-scroll"><div class="heat-months" style="--weeks:${columns}">${months}</div><div class="heat-layout"><div class="heat-weekdays">${['一','','三','','五','','日'].map(s=>`<span>${s}</span>`).join('')}</div><div class="heatmap" role="group" aria-label="年度用量，方向键选择日期，回车查看当天详情">${'<span></span>'.repeat(lead)}${days.map((d,i)=>{const value=state.cost?d.cost:d.total,label=d.key+' · '+(state.cost?d.cost.toFixed(6)+' USD':d.total.toLocaleString()+' Token');return `<button type="button" data-heat-day="${i}" tabindex="${i===0?0:-1}" aria-label="${label}" title="${label}" style="background:${value?`color-mix(in srgb,var(--accent) ${25+75*Math.sqrt(value/max)}%,var(--solid))`:'var(--surface-sunken)'}"></button>`;}).join('')}</div></div></div><p id="heat-reading" aria-live="polite" class="muted tiny">方向键选择日期，Enter 查看当天详情</p><details class="heat-table"><summary>查看年度数据表（含精确值）</summary><div class="table-scroll"><table><thead><tr><th scope="col">日期</th><th scope="col">Token</th><th scope="col">API 等价成本 USD</th></tr></thead><tbody>${days.map(d=>`<tr><th scope="row">${d.key}</th><td title="${d.total} Token">${compact(d.total)}</td><td>${d.cost.toFixed(6)}</td></tr>`).join('')}</tbody></table></div></details>`;
}
function bindAnalysis(){
  document.querySelectorAll('[data-trend-model]').forEach(b=>{b.setAttribute('aria-pressed',String(!hiddenTrendModels.has(b.dataset.trendModel)));b.onclick=()=>{const name=b.dataset.trendModel;hiddenTrendModels.has(name)?hiddenTrendModels.delete(name):hiddenTrendModels.add(name);b.setAttribute('aria-pressed',String(!hiddenTrendModels.has(name)));drawTrend();};});
  document.querySelectorAll('[data-copy-value]').forEach(b=>b.onclick=async()=>{try{await navigator.clipboard.writeText(b.dataset.copyValue);notify('已复制精确数值');}catch(_){notify('无法写入剪贴板，可直接选择表格文字复制','error');}});
  const cells=[...document.querySelectorAll('[data-heat-day]')];
  const focus=index=>{index=Math.max(0,Math.min(cells.length-1,index));cells.forEach((b,i)=>b.tabIndex=i===index?0:-1);cells[index]?.focus();};
  for(const [index,cell] of cells.entries()){
    cell.onfocus=()=>{$('#heat-reading').textContent=cell.getAttribute('aria-label');};
    cell.onkeydown=e=>{const offset={ArrowLeft:-7,ArrowRight:7,ArrowUp:-1,ArrowDown:1}[e.key];if(offset!=null){e.preventDefault();focus(index+offset);}else if(e.key==='Home'||e.key==='End'){e.preventDefault();focus(e.key==='Home'?0:cells.length-1);}};
    cell.onclick=()=>showUsageDay(state.dashboard.heatmap[index]);
  }
  document.querySelectorAll('[data-day-toggle]').forEach(b=>b.onclick=()=>{const body=b.closest('tbody'),open=b.getAttribute('aria-expanded')!=='true';b.setAttribute('aria-expanded',String(open));body.toggleAttribute('open',open);body.querySelectorAll('.day-model').forEach(row=>row.hidden=!open);});
}
function showUsageDay(day){
  if(state.estimateBusy){notify(day.key+" · "+exactUsage(day));return;}
  quotaDialog(day.key+' · 当天用量',`<p class="exact-usage">${compact(day.total)} Token · ${day.cost.toFixed(6)} USD</p><p>普通输入 ${compact(day.tokens.input)} · 输出 ${compact(day.tokens.output)} · 缓存读取 ${compact(day.tokens.cacheRead)} · 缓存写入 ${compact(day.tokens.cacheWrite)} Token</p><p class="muted">按当前已应用来源、账户和模型筛选。缺价时成本仅含已计价部分。</p>`);
}
