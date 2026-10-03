(() => {
  const ball = document.querySelector('#ball');
  const labels = {working:'进行中',thinking:'思考中',tool:'执行工具',complete:'已完成',interrupted:'已中断',unknown:'待确认'};
  let start = null, dragged = false;
  function failure(error) {
    document.querySelector('#phase').textContent = '连接异常';
    document.body.dataset.active = 'false';
    ball.title = String(error) + '；右键打开菜单';
  }
  async function action(name) {
    try { await window.__TAURI__.core.invoke('desktop_action', {action:name}); } catch (error) { failure(error); }
  }
  function update(info) {
    document.body.dataset.phase = info.phase || 'idle';
    document.body.dataset.active = String(info.activeCount > 0);
    document.querySelector('#phase').textContent = info.unavailable ? '状态不全' : (labels[info.phase] || '空闲');
    const count = document.querySelector('#count');
    count.textContent = info.activeCount > 99 ? '99+' : String(info.activeCount);
    count.hidden = !info.activeCount;
    ball.title = `Aieyes · ${info.summary}\n单击打开 · 拖动移动 · 右键菜单`;
    ball.setAttribute('aria-label', ball.title);
  }
  ball.addEventListener('pointerdown', event => {
    if (event.button !== 0) return;
    start = {x:event.clientX,y:event.clientY}; dragged = false;
  });
  ball.addEventListener('pointermove', async event => {
    if (!start || !(event.buttons & 1) || dragged) return;
    if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < 5) return;
    dragged = true; start = null;
    try { await window.__TAURI__.window.getCurrentWindow().startDragging(); } catch (error) { failure(error); }
  });
  ball.addEventListener('pointerup', () => { start = null; });
  ball.addEventListener('pointercancel', () => { start = null; });
  ball.addEventListener('click', event => { if (!dragged || event.detail === 0) action('open'); });
  ball.addEventListener('contextmenu', event => { event.preventDefault(); action('menu'); });
  ball.addEventListener('keydown', event => {
    if ((event.shiftKey && event.key === 'F10') || event.key === 'ContextMenu') { event.preventDefault(); action('menu'); }
  });
  async function boot() {
    try {
      await window.__TAURI__.event.listen('desktop:status', event => update(event.payload));
      await window.__TAURI__.event.listen('desktop:error', event => failure(event.payload));
      update(await window.__TAURI__.core.invoke('desktop_info'));
    } catch (error) { failure(error); }
  }
  boot();
})();
