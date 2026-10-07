/* Compact panel and Linux capsule. Windows capsule input lives in capsule.rs. */
(() => {
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const ball = document.querySelector('#ball');
  const panel = document.querySelector('#panel');
  const labels = {idle:'空闲',working:'进行中',thinking:'思考中',tool:'执行工具',complete:'已完成',interrupted:'已中断',unknown:'待确认'};
  const SUPPRESS = 250;
  let start = null, dragged = false, info = null;
  let panelOpen = false, pinned = false, suppress = 0;

  function failure(error) {
    document.querySelector('#phase').textContent = '连接异常';
    document.body.dataset.active = 'false';
    ball.title = String(error) + '；右键打开菜单';
    if (panelOpen && typeof notify === 'function') notify(String(error));
  }
  async function action(name) {
    try { return await invoke('desktop_action', { action: name }); } catch (error) { failure(error); }
  }
  function closeMenu(restoreFocus = false) {
    const menu = document.querySelector('#panel-menu');
    if (!menu || menu.hidden) return;
    menu.hidden = true;
    document.querySelector('#panel-refresh')?.setAttribute('aria-expanded', 'false');
    if (restoreFocus) document.querySelector('#panel-refresh')?.focus({preventScroll:true});
  }
  function applyPanel(next) {
    const was = panelOpen;
    const focused = document.activeElement;
    panelOpen = !!next.panelOpen;
    pinned = !!next.panelPinned;
    document.body.dataset.view = panelOpen ? 'panel' : 'ball';
    const pin = document.querySelector('#panel-pin');
    if (pin) {
      pin.dataset.pinned = String(pinned);
      pin.setAttribute('aria-pressed', String(pinned));
      pin.textContent = pinned ? "取消固定面板" : "固定面板";
      pin.title = pinned ? '取消固定' : '固定面板';
      pin.setAttribute('aria-label', pin.title);
    }
    if (was && !panelOpen) {
      closeMenu();
      const more = document.querySelector('#panel-more');
      if (more) more.open = false;
      if(typeof closeTokenSummary==='function')closeTokenSummary();
      if (panel.contains(focused)) ball.focus({preventScroll:true});
    }
    if (!was && panelOpen) {
      window.AieyesNetwork?.refresh();
      if (typeof render === 'function') render();
      requestAnimationFrame(() => { if (typeof drawTrend === 'function') drawTrend(); });
      // Mouse-opened floating surfaces preserve the other application's keyboard focus.
      const refresh=window.AieyesApp?.refreshPanel;
      if(refresh)refresh().catch(error=>failure(error));
    }
  }
  async function setPanel(open) {
    if (panelOpen === open) return;
    try { applyPanel(await invoke('desktop_panel', { open })); }
    catch (error) { failure(error); return; }
  }

  ball.addEventListener('pointerdown', event => {
    if (event.button !== 0) return;
    start = { x: event.clientX, y: event.clientY };
    dragged = false;
  });
  ball.addEventListener('pointermove', async event => {
    if (!start || !(event.buttons & 1) || dragged) return;
    if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < 5) return;
    dragged = true; start = null;
    try { await window.__TAURI__.window.getCurrentWindow().startDragging(); }
    catch (error) { dragged=false;start=null;failure(error); return; }
    // Keep the panel closed right after a drag; the ball settles onto a nearby edge.
    suppress = Date.now() + SUPPRESS;
    action('snap');
    setTimeout(() => {
      dragged = false;
    }, SUPPRESS);
  });
  ball.addEventListener('pointerup', () => { start = null; });
  ball.addEventListener('pointercancel', () => { start = null; });
  ball.addEventListener('click', event => {
    if (dragged && event.detail !== 0) return;
    dragged = false;
    if(Date.now()<suppress&&event.detail!==0)return;
    setPanel(!panelOpen, false);
  });
  document.addEventListener('contextmenu', event => {
    if(event.target?.closest?.('input,textarea,[contenteditable=true]'))return;
    event.preventDefault();action('menu');
  });
  document.addEventListener('pointerup',()=>{start=null;});
  document.addEventListener('pointercancel',()=>{start=null;dragged=false;});
  ball.addEventListener('keydown', event => {
    if ((event.shiftKey && event.key === 'F10') || event.key === 'ContextMenu') { event.preventDefault(); action('menu'); }
  });

  document.addEventListener('keydown', event => {
    if (event.defaultPrevented || document.querySelector('#quota-dialog')?.open) return;
    if (event.key === 'Escape' && panelOpen) {
      event.preventDefault();
      const more = document.querySelector('#panel-more');
      if (more?.open) { more.open = false; more.querySelector('summary')?.focus({preventScroll:true}); }
      else if (!document.querySelector('#panel-menu').hidden) closeMenu(true);
      else setPanel(false);
    }
  });
  document.addEventListener('pointerdown', event => {
    const menu = document.querySelector('#panel-menu');
    const wrap = event.target instanceof Element ? event.target.closest('.panel-refresh-wrap') : null;
    if (menu && !menu.hidden && !wrap) closeMenu();
    const more = document.querySelector('#panel-more');
    if (more?.open && !more.contains(event.target)) more.open = false;
  });

  bindRefreshMenu('panel-refresh','panel-menu');
  document.querySelector('#panel-pin')?.addEventListener('click', async () => {
    try { applyPanel(await invoke('desktop_panel_pin', { pinned: !pinned })); } catch (error) { failure(error); }
  });
  document.querySelector('#panel-detail').addEventListener('click', () => { setPanel(false); if(typeof openDetail==='function')openDetail();else action('open'); });
  document.querySelector('#panel-settings').addEventListener('click', () => { setPanel(false); action('settings'); });
  document.querySelector('#panel-accounts')?.addEventListener('click',()=>{const more=document.querySelector('#panel-more');if(more)more.open=false;window.AieyesApp?.openPanelAccounts();});
  document.querySelector('#panel-quit').addEventListener('click', () => action('quit'));
  document.querySelector('#panel-more')?.addEventListener('toggle', event => {
    if (event.target.open) closeMenu();
  });

  window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener('change',()=>{if(info)update(info,false);});
  function update(next, applyOpen = true) {
    info = next;
    document.body.dataset.phase = next.phase || 'idle';
    document.body.dataset.active = String(next.activeCount > 0);
    document.body.dataset.platform = next.platform || '';
    document.querySelector('#phase').textContent = next.recovery==='failed'?'恢复失败':next.recovery==='refreshing'?'恢复中':next.unavailable ? '状态不全' : (labels[next.phase] || '空闲');
    const count = document.querySelector('#count');
    count.textContent = next.activeCount > 99 ? '99+' : String(next.activeCount);
    count.hidden = !next.activeCount || !next.showCount;
    count.setAttribute('aria-label',next.activeCount+' 个活跃会话');
    if(typeof phaseColor==='function')document.body.style.setProperty('--phase-color',phaseColor(next.phase||'idle'));
    ball.title = `Aieyes · ${next.summary}\n单击打开面板 · 拖动移动 · 右键菜单`;
    ball.setAttribute('aria-label', ball.title);
    if (typeof renderLiveSessions === 'function') renderLiveSessions(next);
    if (applyOpen && (next.panelOpen !== undefined || next.panelPinned !== undefined)) applyPanel(next);
  }
  async function boot() {
    try {
      await window.__TAURI__.event.listen('desktop:status', event => update(event.payload));
      await window.__TAURI__.event.listen('desktop:error', event => failure(event.payload));
      await window.__TAURI__.event.listen('desktop:panel', event => applyPanel(event.payload));
      const initial=await invoke('desktop_info');
      document.body.dataset.nativeCapsule=String(!!initial.nativeCapsule);
      update(initial);
      if(typeof appReady!=='undefined')await appReady;
      await invoke('desktop_panel_ready',{generation:window.AIEYES_SHELL_GENERATION??0});
    } catch (error) { failure(error); }
  }
  boot();
})();
