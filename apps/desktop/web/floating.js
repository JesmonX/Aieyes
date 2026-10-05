/* The floating ball and the compact panel it expands into. The panel mirrors the
   macOS menu bar popover: hover or click opens it, dragging snaps the ball to a
   work-area edge, and the native menu can hide the ball temporarily. */
(() => {
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const ball = document.querySelector('#ball');
  const panel = document.querySelector('#panel');
  const labels = {working:'进行中',thinking:'思考中',tool:'执行工具',complete:'已完成',interrupted:'已中断',unknown:'待确认'};
  const HOVER_DELAY = 350, LEAVE_DELAY = 450, SUPPRESS = 650;
  let start = null, dragged = false, info = null;
  let panelOpen = false, pinned = false, hoverOpened = false, sticky = false, hovering = false;
  let hoverTimer = 0, leaveTimer = 0, hoverWatch = 0, outsideSince = null, suppress = 0;

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
  function watchHover() {
    clearTimeout(hoverWatch);
    if (!panelOpen || !hoverOpened || sticky || pinned) return;
    hoverWatch = setTimeout(async () => {
      try {
        const inside = await invoke('desktop_panel_cursor_inside');
        if (!panelOpen || !hoverOpened || sticky || pinned) return;
        if (inside) outsideSince = null;
        else if (outsideSince === null) outsideSince = Date.now();
        else if (Date.now() - outsideSince >= LEAVE_DELAY) { setPanel(false); return; }
      } catch (_) { /* Pointer events remain the fallback on unsupported desktops. */ }
      watchHover();
    }, 150);
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
      pin.title = pinned ? '取消固定' : '固定面板';
      pin.setAttribute('aria-label', pin.title);
    }
    if (was && !panelOpen) {
      // Do not immediately reopen from the hover that may survive the window shrink.
      suppress = Date.now() + SUPPRESS;
      hoverOpened = sticky = false;
      clearTimeout(leaveTimer);
      clearTimeout(hoverWatch);
      outsideSince = null;
      closeMenu();
      const more = document.querySelector('#panel-more');
      if (more) more.open = false;
      if (panel.contains(focused)) ball.focus({preventScroll:true});
    }
    if (!was && panelOpen) {
      clearTimeout(hoverTimer);
      if (typeof render === 'function') render();
      requestAnimationFrame(() => { if (typeof drawTrend === 'function') drawTrend(); });
      if (sticky) panel.querySelector('.panel-tabs button.active')?.focus({preventScroll:true});
      watchHover();
    }
  }
  async function setPanel(open, hover) {
    if (panelOpen === open) return;
    if (open) { hoverOpened = !!hover; sticky = !hover; }
    try { applyPanel(await invoke('desktop_panel', { open })); }
    catch (error) { failure(error); return; }
    if (open && panelOpen) {
      const refresh = window.AieyesApp?.refreshPanel || (typeof loadDashboard === 'function' ? loadDashboard : null);
      if (refresh) refresh().catch(error => notify(String(error)));
    }
  }
  function scheduleHover() {
    clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      if (!hovering || panelOpen || start || dragged || Date.now() < suppress) return;
      if (info?.hidden) return;
      setPanel(true, true);
    }, HOVER_DELAY);
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
    clearTimeout(hoverTimer);
    try { await window.__TAURI__.window.getCurrentWindow().startDragging(); }
    catch (error) { failure(error); return; }
    // Keep the panel closed right after a drag; the ball settles onto a nearby edge.
    suppress = Date.now() + SUPPRESS;
    action('snap');
    setTimeout(() => {
      dragged = false;
      if (hovering && !panelOpen) scheduleHover();
    }, SUPPRESS);
  });
  ball.addEventListener('pointerup', () => { start = null; });
  ball.addEventListener('pointercancel', () => { start = null; });
  ball.addEventListener('pointerenter', () => { hovering = true; scheduleHover(); });
  ball.addEventListener('pointerleave', () => { hovering = false; clearTimeout(hoverTimer); });
  ball.addEventListener('click', event => {
    if (dragged && event.detail !== 0) return;
    dragged = false;
    setPanel(true, false);
  });
  ball.addEventListener('contextmenu', event => { event.preventDefault(); action('menu'); });
  ball.addEventListener('keydown', event => {
    if ((event.shiftKey && event.key === 'F10') || event.key === 'ContextMenu') { event.preventDefault(); action('menu'); }
  });

  panel.addEventListener('pointerenter', () => { clearTimeout(leaveTimer); outsideSince = null; });
  panel.addEventListener('pointerleave', () => {
    if (!panelOpen || sticky || pinned) return;
    clearTimeout(leaveTimer);
    leaveTimer = setTimeout(async () => {
      if (!panelOpen || !hoverOpened || sticky || pinned) return;
      try { if (await invoke('desktop_panel_cursor_inside')) return; } catch (_) {}
      if (panelOpen && hoverOpened && !sticky && !pinned) setPanel(false);
    }, LEAVE_DELAY);
  });
  panel.addEventListener('pointerdown', () => { sticky = true; clearTimeout(hoverWatch); });
  document.addEventListener('focusin', () => { if (panelOpen) { sticky = true; clearTimeout(hoverWatch); } });
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

  document.querySelector('#panel-refresh').addEventListener('click', () => {
    const menu = document.querySelector('#panel-menu');
    menu.hidden = !menu.hidden;
    const more = document.querySelector('#panel-more');
    if (more) more.open = false;
    document.querySelector('#panel-refresh').setAttribute('aria-expanded', String(!menu.hidden));
  });
  document.querySelector('#panel-refresh').addEventListener('keydown', event => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const menu = document.querySelector('#panel-menu');
      menu.hidden = false;
      document.querySelector('#panel-refresh').setAttribute('aria-expanded', 'true');
      const buttons = [...menu.querySelectorAll('button')];
      buttons[event.key === 'ArrowDown' ? 0 : buttons.length - 1]?.focus();
    }
  });
  document.querySelector('#panel-menu').addEventListener('keydown', event => {
    const buttons = [...document.querySelector('#panel-menu').querySelectorAll('button')];
    const index = buttons.indexOf(document.activeElement);
    let next;
    if (event.key === 'ArrowDown') next = (index + 1) % buttons.length;
    if (event.key === 'ArrowUp') next = (index + buttons.length - 1) % buttons.length;
    if (event.key === 'Home') next = 0;
    if (event.key === 'End') next = buttons.length - 1;
    if (next !== undefined) { event.preventDefault(); buttons[next]?.focus(); }
  });
  document.querySelector('#panel-menu').addEventListener('click', event => {
    const button = event.target.closest('[data-refresh]');
    if (!button) return;
    closeMenu();
    const runs = { scan, quotas, prices: syncPrices, hosts: sample };
    const run = runs[button.dataset.refresh];
    if (typeof run === 'function') run();
  });
  document.querySelector('#panel-pin').addEventListener('click', async () => {
    try { applyPanel(await invoke('desktop_panel_pin', { pinned: !pinned })); } catch (error) { failure(error); }
  });
  document.querySelector('#panel-detail').addEventListener('click', () => { setPanel(false); action('open'); });
  document.querySelector('#panel-settings').addEventListener('click', () => { setPanel(false); action('settings'); });
  document.querySelector('#panel-quit').addEventListener('click', () => action('quit'));
  document.querySelector('#panel-more')?.addEventListener('toggle', event => {
    if (event.target.open) closeMenu();
  });

  function update(next) {
    info = next;
    document.body.dataset.phase = next.phase || 'idle';
    document.body.dataset.active = String(next.activeCount > 0);
    document.body.dataset.platform = next.platform || '';
    document.querySelector('#phase').textContent = next.unavailable ? '状态不全' : (labels[next.phase] || '空闲');
    const count = document.querySelector('#count');
    count.textContent = next.activeCount > 99 ? '99+' : String(next.activeCount);
    count.hidden = !next.activeCount;
    ball.title = `Aieyes · ${next.summary}\n单击或悬停打开面板 · 拖动移动 · 右键菜单`;
    ball.setAttribute('aria-label', ball.title);
    if (typeof renderLiveSessions === 'function') renderLiveSessions(next);
    if (next.panelOpen !== undefined || next.panelPinned !== undefined) applyPanel(next);
  }
  async function boot() {
    try {
      await window.__TAURI__.event.listen('desktop:status', event => update(event.payload));
      await window.__TAURI__.event.listen('desktop:error', event => failure(event.payload));
      await window.__TAURI__.event.listen('desktop:panel', event => applyPanel(event.payload));
      update(await invoke('desktop_info'));
    } catch (error) { failure(error); }
  }
  boot();
})();
