/* One application measurement shared by both webviews; drafts stay local. */
(() => {
  let snapshot = null, busy = false, attempted = 0, revision = 0;
  const label = result => !result ? '连接未测试' : result.status === 'unstable' ? '连接不稳定' : result.status === 'failed' ? '连接失败' : (result.mode === 'direct' ? '直连 ' : result.mode === 'system' ? '系统 ' : '代理 ') + result.averageMs + ' ms';
  const detail = result => result ? result.sites.map(site => site.url+' · '+(site.latencyMs!=null?site.latencyMs+' ms':site.error)).join('\n')+'\n测试于 '+new Date(result.testedAt*1000).toLocaleString() : '测试当前应用连接方式';
  function accept(next) { snapshot=Array.isArray(next?.sites)?next:null; draw(); }
  function draw() { const button=document.querySelector('#panel-latency');if(button){button.textContent=busy?'测试中…':label(snapshot);button.title=detail(snapshot);button.disabled=busy;} }
  async function refresh(force=false) {
    if(busy || (!force && Date.now()-attempted<300000))return;
    busy=true;attempted=Date.now();draw();
    const testedRevision=revision;
    try { const measured=await api('network.test',{force});if(revision===testedRevision)accept(measured); } catch(error) { if(revision===testedRevision&&typeof notify==='function')notify(String(error),'error'); }
    finally { busy=false;draw(); }
  }
  window.AieyesNetwork={label,detail,refresh,invalidate(){revision++;snapshot=null;attempted=0;draw();}};
  Promise.resolve(appReady).then(async()=>{
    await window.__TAURI__.event.listen('desktop:network',({payload})=>{attempted=Date.now();accept(payload);});
    try{accept(await api('network.status'));}catch{accept(null);}
    const button=document.querySelector('#panel-latency');if(button)button.onclick=()=>refresh(true);
    if(PANEL&&document.body.dataset.view==='panel')refresh();
    // Only the main window owns the interval; opening a panel requests the same cached result.
    if(!PANEL)setInterval(()=>{if(state.panelOpen)refresh();},1000);
  }).catch(error=>{if(typeof notify==='function')notify(String(error),'error');});
})();
