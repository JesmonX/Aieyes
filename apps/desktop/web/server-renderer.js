/* Retained host cards and viewport-sized device lists; shared by both windows. */
(() => {
  let root=null,frame=0;
  const cards=new Map(),lists=new Set(),expansion=new Map();
  const template=document.createElement('template');
  function patch(target,html){
    if(target._html===html)return;target._html=html;template.innerHTML=html;
    const sync=(a,b)=>{
      if(a.nodeType!==b.nodeType||a.nodeName!==b.nodeName){a.replaceWith(b.cloneNode(true));return;}
      if(a.nodeType===Node.TEXT_NODE){if(a.nodeValue!==b.nodeValue)a.nodeValue=b.nodeValue;return;}
      if(a.nodeType!==Node.ELEMENT_NODE)return;
      for(const attr of [...a.attributes])if(!b.hasAttribute(attr.name))a.removeAttribute(attr.name);
      for(const attr of b.attributes)if(a.getAttribute(attr.name)!==attr.value)a.setAttribute(attr.name,attr.value);
      children(a,b);
    };
    const children=(a,b)=>{const old=[...a.childNodes],next=[...b.childNodes];for(let i=0;i<Math.max(old.length,next.length);i++){if(!next[i])old[i].remove();else if(!old[i])a.append(next[i].cloneNode(true));else sync(old[i],next[i]);}};
    children(target,template.content);
  }
  const visible=el=>{const r=el.getBoundingClientRect();return r.bottom>=-200&&r.top<=innerHeight+200;};
  function request(){if(!frame&&!document.hidden)frame=requestAnimationFrame(()=>{frame=0;if(state.page==='servers')render();else clear();});}
  function clear(){for(const list of lists)list.dispose();cards.clear();root=null;}
  class DeviceList {
    constructor(element,key,host){this.el=element;this.key=key;this.host=host;this.rows=[];this.heights=new Map();this.nodes=new Map();this.before=document.createElement('div');this.after=document.createElement('div');this.before.className=this.after.className='metric-spacer';element.replaceChildren(this.before,this.after);element.setAttribute('role','list');element.setAttribute('aria-label',groups[key]+'设备');this.observer=new ResizeObserver(entries=>{let changed=false;for(const entry of entries){const id=entry.target.dataset.deviceID,h=entry.borderBoxSize?.[0]?.blockSize??entry.contentRect.height;if(h>0&&Math.abs((this.heights.get(id)??this.estimate())-h)>1){this.heights.set(id,h);changed=true;}}if(changed)request();});lists.add(this);}
    estimate(){return this.key==='cpu'?60:72;}
    update(rows,host){this.rows=rows;this.host=host;this.paint();}
    paint(){
      if(!this.el.isConnected){this.dispose();return;}
      if(!this.el.closest('details')?.open)return;
      const top=this.el.getBoundingClientRect().top,low=-top-200,high=innerHeight-top+200,offsets=[0];
      for(const row of this.rows)offsets.push(offsets.at(-1)+(this.heights.get(row.id)??this.estimate()));
      let first=0,last=this.rows.length;
      if(last>80){while(first<last&&offsets[first+1]<low)first++;last=first;while(last<this.rows.length&&offsets[last]<high)last++;}
      const wanted=new Set(this.rows.slice(first,last).map(r=>r.id));
      for(const [id,node]of this.nodes)if(!wanted.has(id)){this.observer.unobserve(node);node.remove();this.nodes.delete(id);}
      this.before.style.height=offsets[first]+'px';this.after.style.height=(offsets.at(-1)-offsets[last])+'px';
      let anchor=this.before;
      for(let i=first;i<last;i++){
        const row=this.rows[i];let node=this.nodes.get(row.id);
        if(!node){node=document.createElement('div');node.className='metric-device';node.dataset.deviceID=row.id;node.setAttribute('role','listitem');this.nodes.set(row.id,node);this.observer.observe(node);}
        node.setAttribute('aria-posinset',String(i+1));node.setAttribute('aria-setsize',String(this.rows.length));
        patch(node,serverGroupContent(this.key,[row],this.host));if(anchor.nextSibling!==node)anchor.after(node);anchor=node;
      }
      const ids=new Set(this.rows.map(r=>r.id));for(const id of this.heights.keys())if(!ids.has(id))this.heights.delete(id);
    }
    dispose(){this.observer.disconnect();lists.delete(this);this.nodes.clear();}
  }
  function paintCard(card,host,result){
    const signature=JSON.stringify([host,result?.sampleSession&&result?.sampleVersion!=null?[result.sampleSession,result.sampleVersion]:result]);if(card.signature===signature)return;
    card.signature=signature;
    const sample=result?.sample;
    patch(card.heading,`<div><h2 style="margin:0">${escapeHTML(host.name||host.target)}</h2><div class="sub">${escapeHTML(host.target)}</div></div><button type="button" class="host-status" data-host-status="${escapeHTML(host.id)}"><i aria-hidden="true"></i><span data-host-status-label></span></button>`);
    patch(card.summary,serverSummary(host,sample));
    if(!card.el.open)return;
    const enabled=(host.metrics??Object.keys(groups)).filter(key=>sample?.[key]);
    for(const [key,group]of card.groups)if(!enabled.includes(key)){group.list?.dispose();group.el.remove();card.groups.delete(key);}
    for(const key of enabled){
      let group=card.groups.get(key);
      if(!group){const el=document.createElement('details');el.className='metric-section';el.dataset.metric=host.id+':'+key;el.innerHTML=`<summary>${groups[key]}明细</summary><div class="metric-detail"></div>`;card.details.append(el);group={el,content:el.lastElementChild};card.groups.set(key,group);el.open=expansion.get('group:'+host.id+':'+key)??false;el.addEventListener('toggle',()=>{expansion.set('group:'+host.id+':'+key,el.open);card.signature=null;if(!el.open){group.list?.dispose();group.list=null;group.content.replaceChildren();}request();});}
      if(!group.el.open)continue;
      if(key==='memory')patch(group.content,serverGroupContent(key,sample[key],host));
      else {group.list??=new DeviceList(group.content,key,host);group.list.update(hostDevices(host,key,sample[key]).filter(row=>key!=='cpu'||row.id!=='cpu'||(host.details??[]).includes('cpuTimes')),host);}
    }
    patch(card.feedback,`${sample?`<div class="tiny muted">负载 ${(sample.load??[]).map(n=>n.toFixed(2)).join(' / ')}</div>`:''}${Object.keys(sample?.errors??{}).filter(k=>(host.metrics??Object.keys(groups)).includes(k)).map(k=>`<p class="error">${groups[k]??escapeHTML(k)} · 采集失败</p>`).join('')}${result?.error?`<p class="error">${escapeHTML(result.error)}</p>`:''}`);
  }
  function render(){
    const content=document.querySelector('#content');if(!content)return;
    if(!root?.isConnected){clear();root=document.createElement('div');root.className='server-list';content.replaceChildren(root);const head=document.createElement('div');head.className='section-head';head.innerHTML=`<span class="muted" data-server-count></span><select id="server-filter" aria-label="服务器筛选">${[['all','全部'],['errors','异常'],['paused','已暂停']].map(([key,label])=>option(key,label,state.serverFilter??'all')).join('')}</select><button id="sample" title="立即采样所有已启用的服务器">刷新服务器</button>`;content.prepend(head);head.querySelector('#sample').onclick=()=>sample();head.querySelector('#server-filter').onchange=e=>{state.serverFilter=e.target.value;savePanelView();render();};}
    const hosts=monitoredHosts();for(const key of expansion.keys())if(!hosts.some(h=>key==='host:'+h.id||key.startsWith('group:'+h.id+':')))expansion.delete(key);const byID=new Map(state.hosts.map(row=>[row.id,row]));
    patch(document.querySelector('[data-server-count]'),hosts.length+' 台主机 '+refreshIndicator('hosts'));
    const selected=hosts.filter(h=>!state.serverFilter||state.serverFilter==='all'||(state.serverFilter==='paused'?!h.enabled:h.enabled&&hostStatus(h,byID.get(h.id))!=='正常')),ids=new Set(selected.map(h=>h.id));
    for(const [id,card]of cards)if(!ids.has(id)){for(const g of card.groups.values())g.list?.dispose();card.el.remove();cards.delete(id);}
    let previous=null;
    for(const host of selected){
      let card=cards.get(host.id);
      if(!card){const el=document.createElement('details');el.className='card server-card';el.dataset.agentDetail='host:'+host.id;el.innerHTML='<summary class="server-summary"><div class="between server-heading"></div><div data-server-summary></div></summary><div data-server-details></div><div data-server-feedback></div>';card={el,heading:el.querySelector('.server-heading'),summary:el.querySelector('[data-server-summary]'),details:el.querySelector('[data-server-details]'),feedback:el.querySelector('[data-server-feedback]'),groups:new Map()};cards.set(host.id,card);el.open=expansion.get('host:'+host.id)??false;el.addEventListener('toggle',()=>{expansion.set('host:'+host.id,el.open);card.signature=null;if(!el.open){for(const g of card.groups.values())g.list?.dispose();card.groups.clear();card.details.replaceChildren();card.feedback.replaceChildren();}request();});}
      if(previous?previous.nextSibling!==card.el:root.firstChild!==card.el){if(previous)previous.after(card.el);else root.prepend(card.el);}previous=card.el;
      if(visible(card.el)||!card.signature)paintCard(card,host,byID.get(host.id));
    }
    let empty=root.querySelector('.empty');if(selected.length)empty?.remove();else if(!empty){empty=document.createElement('div');empty.className='card empty';empty.innerHTML='<button id="add-first-host" class="primary">添加服务器</button>';root.append(empty);bindSetup();}
    for(const list of lists)list.paint();updateHostStatuses();
  }
  document.addEventListener('scroll',request,{capture:true,passive:true});window.addEventListener('resize',request);
  document.addEventListener('visibilitychange',()=>{if(!document.hidden)request();});
  new MutationObserver(()=>{if(root&&!root.isConnected)clear();}).observe(document.body,{childList:true,subtree:true});
  window.AieyesServers={render,request};
})();
