/* Accessible equivalents and interaction for the usage charts. */
const hiddenTrendModels=new Set();
function exactUsage(day){return `${day.total.toLocaleString()} Token · ${day.cost.toFixed(6)} USD · 读取命中率 ${pct(cacheRate(day.tokens))}`;}
function trendUsage(day){return `${compact(day.total)} Token · ${day.cost.toFixed(2)} USD · 读取命中率 ${pct(cacheRate(day.tokens))}`;}
// Annular sectors keep the native chart's gap and small corner radius at every DPI.
function modelRingSVG(slices,total) {
  const style=getComputedStyle(document.documentElement),metric=name=>Number.parseFloat(style.getPropertyValue('--'+name));
  const size=metric('model-chart-size'),outer=size/2,inner=outer*metric('model-chart-inner-ratio');
  const gap=metric('model-chart-angular-inset')*Math.PI/180,rounding=metric('model-chart-corner');
  const point=(radius,angle)=>[outer+radius*Math.cos(angle),outer+radius*Math.sin(angle)].map(n=>Number(n.toFixed(4))).join(' ');
  let angle=-Math.PI/2;
  const paths=slices.flatMap(slice=>{
    const value=state.cost?slice.cost:slice.total;
    if(!Number.isFinite(value)||value<=0||!Number.isFinite(total)||total<=0)return [];
    const span=value/total*Math.PI*2,from=angle,to=angle+span;angle=to;
    let d;
    if(span>=Math.PI*2-1e-6) {
      d=`M ${point(outer,0)} A ${outer} ${outer} 0 1 1 ${point(outer,Math.PI)} A ${outer} ${outer} 0 1 1 ${point(outer,0)} Z M ${point(inner,0)} A ${inner} ${inner} 0 1 0 ${point(inner,Math.PI)} A ${inner} ${inner} 0 1 0 ${point(inner,0)} Z`;
    } else {
      const inset=Math.min(gap,span/2)/2,start=from+inset,end=to-inset;
      const corner=Math.min(rounding,(outer-inner)/2,(end-start)*inner/3),oa=corner/outer,ia=corner/inner;
      d=`M ${point(outer,start+oa)} A ${outer} ${outer} 0 ${end-start-2*oa>Math.PI?1:0} 1 ${point(outer,end-oa)} Q ${point(outer,end)} ${point(outer-corner,end)} L ${point(inner+corner,end)} Q ${point(inner,end)} ${point(inner,end-ia)} A ${inner} ${inner} 0 ${end-start-2*ia>Math.PI?1:0} 0 ${point(inner,start+ia)} Q ${point(inner,start)} ${point(inner+corner,start)} L ${point(outer-corner,start)} Q ${point(outer,start)} ${point(outer,start+oa)} Z`;
    }
    return [`<path d="${d}" fill="${color(slice.key)}" fill-rule="evenodd"/>`];
  });
  const empty=`<circle cx="${outer}" cy="${outer}" r="${(outer+inner)/2}" fill="none" stroke="var(--progress-track)" stroke-width="${outer-inner}"/>`;
  return `<svg viewBox="0 0 ${size} ${size}" aria-hidden="true">${paths.join('')||empty}</svg>`;
}
function modelDistribution(models){
  const ranked=[...models].sort((a,b)=>(state.cost?b.cost-a.cost:b.total-a.total)||a.key.localeCompare(b.key));
  const sum=ranked.reduce((n,m)=>n+(state.cost?m.cost:m.total),0),top=ranked.slice(0,5),other=ranked.slice(5);
  const slices=[...top,...(other.length?[{key:'其他',total:other.reduce((n,m)=>n+m.total,0),cost:other.reduce((n,m)=>n+m.cost,0)}]:[])];
  const row=m=>`<div><span class="model" title="${escapeHTML(m.key)}">${escapeHTML(m.key)}</span><strong title="${state.cost?m.cost.toFixed(6)+' USD':m.total.toLocaleString()+' Token'}">${state.cost?money(m.cost)+' USD':compact(m.total)+' Token'} · ${pct(sum?(state.cost?m.cost:m.total)/sum*100:0)}</strong></div>`;
  return `<div class="pie-wrap"><div class="donut" role="img" aria-label="前五模型与其他，精确数据见右侧列表">${modelRingSVG(slices,sum)}<div class="donut-inner">${models.length}<small>模型</small></div></div><div class="legend">${top.map(row).join('')}${other.length?`<details><summary>其他 ${other.length} 个模型 · ${state.cost?money(other.reduce((n,m)=>n+m.cost,0)):compact(other.reduce((n,m)=>n+m.total,0))} · ${pct(sum?other.reduce((n,m)=>n+(state.cost?m.cost:m.total),0)/sum*100:0)}</summary>${other.map(row).join('')}</details>`:''}</div></div>`;
}
function heatmapHTML(days,offset=0){
  const lead=days.length?(new Date(days[0].key+'T12:00:00').getDay()+6)%7:0,columns=Math.ceil((days.length+lead)/7),max=Math.max(1,...days.map(d=>state.cost?d.cost:d.total));
  const months=days.flatMap((d,i)=>i===0||d.key.slice(8)==='01'?[`<span style="grid-column:${Math.floor((i+lead)/7)+1}">${Number(d.key.slice(5,7))}月</span>`]:[]).join('');
  return `<div class="heat-scroll" style="--weeks:${Math.max(1,columns)}"><div class="heat-months">${months}</div><div class="heat-layout"><div class="heat-weekdays">${['一','','三','','五','','日'].map(s=>`<span>${s}</span>`).join('')}</div><div class="heatmap" role="group" aria-label="年度用量，方向键选择日期，回车查看当天详情">${'<span></span>'.repeat(lead)}${days.map((d,i)=>{const value=state.cost?d.cost:d.total,label=d.key+' · '+(state.cost?d.cost.toFixed(6)+' USD':d.total.toLocaleString()+' Token');return `<button type="button" data-heat-day="${i+offset}" tabindex="${i===0?0:-1}" aria-label="${label}" title="${label}" style="background:${value?`color-mix(in srgb,var(--accent) ${25+75*Math.sqrt(value/max)}%,transparent)`:'color-mix(in srgb,var(--ink) calc(var(--heat-empty-opacity) * 100%),transparent)'}"></button>`;}).join('')}</div></div></div><p id="${offset?'heat-reading-'+offset:'heat-reading'}" aria-live="polite" class="muted tiny heat-reading">方向键选择日期，Enter 查看当天详情</p><div class="heat-legend" aria-label="用量由少到多">零${[0,1,2,3,4].map(n=>`<i style="background:${n?`color-mix(in srgb,var(--accent) ${25+n*18.75}%,transparent)`:'color-mix(in srgb,var(--ink) calc(var(--heat-empty-opacity) * 100%),transparent)'}"></i>`).join('')}多</div><details class="heat-table"><summary>查看年度数据表（含精确值）</summary><div class="table-scroll"><table><thead><tr><th scope="col">日期</th><th scope="col">Token</th><th scope="col">API 等价成本 USD</th></tr></thead><tbody>${days.map(d=>`<tr><th scope="row">${d.key}</th><td title="${d.total} Token">${compact(d.total)}</td><td>${d.cost.toFixed(6)}</td></tr>`).join('')}</tbody></table></div></details>`;
}
function bindAnalysis(){
  document.querySelectorAll('[data-trend-model]').forEach(b=>{b.setAttribute('aria-pressed',String(!hiddenTrendModels.has(b.dataset.trendModel)));b.onclick=()=>{const name=b.dataset.trendModel;hiddenTrendModels.has(name)?hiddenTrendModels.delete(name):hiddenTrendModels.add(name);b.setAttribute('aria-pressed',String(!hiddenTrendModels.has(name)));drawTrend();};});
  bindHeatCells(document);bindYearlyHeatmaps();bindDailyToggles();bindMoreDays();
}
function bindHeatCells(root){
  const cells=[...root.querySelectorAll('[data-heat-day]')];
  const focus=index=>{index=Math.max(0,Math.min(cells.length-1,index));cells.forEach((b,i)=>b.tabIndex=i===index?0:-1);cells[index]?.focus();};
  for(const [index,cell] of cells.entries()){
    cell.onfocus=()=>{const reading=cell.closest('.heat-scroll')?.nextElementSibling;if(reading)reading.textContent=cell.getAttribute('aria-label');};
    cell.onkeydown=e=>{const offset={ArrowLeft:-7,ArrowRight:7,ArrowUp:-1,ArrowDown:1}[e.key];if(offset!=null){e.preventDefault();focus(index+offset);}else if(e.key==='Home'||e.key==='End'){e.preventDefault();focus(e.key==='Home'?0:cells.length-1);}};
    cell.onclick=()=>showUsageDay(state.dashboard.heatmap[Number(cell.dataset.heatDay)]);
  }
}
function bindDailyToggles(){document.querySelectorAll('[data-day-toggle]').forEach(b=>b.onclick=()=>{const body=b.closest('tbody'),open=b.getAttribute('aria-expanded')!=='true';b.setAttribute('aria-expanded',String(open));body.toggleAttribute('open',open);body.querySelectorAll('.day-model').forEach(row=>row.hidden=!open);});}
function yearlyHeatmapHTML(days){
  const years=[...new Set(days.map(day=>day.key.slice(0,4)))].reverse();
  return years.map((year,index)=>`<details class="heat-year" data-heat-year="${year}" data-agent-detail="heat-year:${year}" ${index===0?'open':''}><summary>${year}</summary><div class="heat-year-content"></div></details>`).join('');
}
function bindYearlyHeatmaps(){
  for(const details of document.querySelectorAll('[data-heat-year]')){
    if(details.dataset.bound)continue;details.dataset.bound='true';
    const mount=()=>{const body=details.querySelector('.heat-year-content');if(!details.open){body.replaceChildren();return;}const days=state.dashboard.heatmap,first=days.findIndex(d=>d.key.startsWith(details.dataset.heatYear+'-')),selected=days.filter(d=>d.key.startsWith(details.dataset.heatYear+'-'));body.innerHTML=heatmapHTML(selected,first);bindHeatCells(body);};
    details.ontoggle=mount;mount();
  }
}

function showUsageDay(day){
  if(state.estimateBusy){notify(day.key+" · "+exactUsage(day));return;}
  quotaDialog(day.key+' · 当天用量',`<p class="exact-usage">${compact(day.total)} Token · ${day.cost.toFixed(6)} USD</p><p>普通输入 ${compact(day.tokens.input)} · 输出 ${compact(day.tokens.output)} · 缓存读取 ${compact(day.tokens.cacheRead)} · 缓存写入 ${compact(day.tokens.cacheWrite)} Token</p><p class="muted">按当前已应用来源、账户和模型筛选。缺价时成本仅含已计价部分。</p>`);
}
