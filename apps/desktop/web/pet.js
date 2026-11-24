/* Sprite rendering only; the core owns agent-state priority and animation choice. */
(() => {
  const sprite = document.querySelector('#pet-sprite'),ball = document.querySelector('#ball');
  const reduced = matchMedia('(prefers-reduced-motion: reduce)');
  let state = null,start = 0,timer = null,enabled = false,frames = [],assetKey = null,loading = null,scale = 1,painted = null,animationSignature = '';
  const context = sprite?.getContext('2d', { alpha: true });
  const frame = window.AieyesPetAnimation.frame;
  function paint(force = true) {
    if (!state || !sprite) return;
    const index = frame(state, Math.max(0, performance.now() - start), reduced.matches);
    const density = (window.devicePixelRatio || 1) * scale,width = Math.round(132 * density),height = Math.round(143 * density);
    const signature = [assetKey, index, width, height, frames.length].join(':');
    if (!force && signature === painted) return;
    painted = signature;
    if (sprite.width !== width || sprite.height !== height) {sprite.width = width;sprite.height = height;}
    context.clearRect(0, 0, width, height);
    if (frames[index]) {
      context.imageSmoothingEnabled = true;context.imageSmoothingQuality = 'high';
      context.drawImage(frames[index], 0, 0, width, height);
    }
    sprite.dataset.frame = String(index);
  }
  function tick() {
    timer = null;
    paint(false);
    if (enabled && !document.hidden && !reduced.matches) timer = requestAnimationFrame(tick);
  }
  function schedule() {
    paint(false);
    if (enabled && !document.hidden && !reduced.matches) {
      if (timer === null) timer = requestAnimationFrame(tick);
    } else if (timer !== null) {cancelAnimationFrame(timer);timer = null;}
  }
  async function loadAsset(asset) {
    const key = asset?.spritePath;
    if (!key) {loading = null;assetKey = null;frames = [];paint();return;}
    if (key === assetKey || key === loading) return;
    loading = key;
    try {
      const result = await api('pets.current', { includeImage: true });
      if (loading !== key) return;
      if (result.error) throw new Error(result.error);
      const sheet = new Image();sheet.src = result.imageDataUrl;await sheet.decode();
      if (loading !== key) return;
      const { cellWidth, cellHeight, rows } = result.pet;
      // Crop at native resolution before interpolation. Adjacent atlas frames and
      // invisible RGB pixels must not bleed into a scaled frame's transparent edge.
      frames = Array.from({ length: rows * 8 }, (_, index) => {
        const frame = document.createElement('canvas');frame.width = cellWidth;frame.height = cellHeight;
        frame.getContext('2d').drawImage(sheet, index % 8 * cellWidth, Math.floor(index / 8) * cellHeight, cellWidth, cellHeight, 0, 0, cellWidth, cellHeight);
        return frame;
      });
      assetKey = result.pet.spritePath;start = performance.now();sprite.dataset.asset = assetKey;sprite.dataset.rows = String(rows);
      paint();
    } catch (error) {frames = [];delete sprite.dataset.asset;paint();} finally
    {if (loading === key) loading = null;}
  }
  function celebrate() {
    if (!enabled || document.hidden || reduced.matches || !sprite) return;
    sprite.animate([{filter:'brightness(1)'},{filter:'brightness(1.5)'},{filter:'brightness(1)'}], {duration:520, easing:'ease-out'});
  }
  window.__TAURI__?.event?.listen('desktop:celebrate', celebrate).catch(() => {});
  function update(info) {
    const previousBehavior = state?.behavior;
    if (!info.nativePet) void loadAsset(info.petAsset);
    enabled = !!info.petEnabled && !info.nativePet && !info.panelOpen;
    scale = Number.isFinite(info.petScale) ? Math.min(2, Math.max(0.5, info.petScale)) : 1;
    ball.style.setProperty('--pet-scale', String(scale));
    document.body.dataset.pet = String(!!info.petEnabled);
    if (info.pet) {
      const signature = AieyesPetAnimation.signature(info.pet);
      if (!state || animationSignature !== signature) start = performance.now();
      animationSignature = signature;state = info.pet;
    }
    if (state && info.petEnabled) {
      document.body.dataset.petBehavior = state.behavior;
      const phase = { attention: 'needsInput', blocked: 'failed', success: 'complete', rest: 'interrupted', running: 'working', working: 'tool', thinking: 'thinking', unknown: 'unknown', idle: 'idle' }[state.behavior] || 'unknown';
      const label = info.petError ? AieyesI18n.t("请导入桌宠") : info.recovery === 'failed' ? AieyesI18n.t("恢复失败") : info.recovery === 'refreshing' ? AieyesI18n.t("恢复中") : AieyesI18n.message(state.label);
      document.body.style.setProperty('--phase-color', `var(--phase-${info.recovery === 'failed' ? 'failed' : phase})`);
      document.querySelector('#phase').textContent = label;
      const orbitAgents = Array.isArray(info.petOrbit) ? info.petOrbit : [];
      const padX = orbitAgents.length ? 44 : 0, padTop = orbitAgents.length ? 44 : 0;
      ball.style.width = (144 + padX * 2) * scale + 'px';
      ball.style.height = (padTop + 168 + 18 * (label.split('\n').length - 1)) * scale + 'px';
      if (sprite) sprite.style.marginTop = padTop * scale + 'px';
      const orbitLayer = document.querySelector('#pet-orbit-layer');
      if (orbitLayer) {
        const anglesMap = { 1: [-90], 2: [-132, -48], 3: [-155, -90, -25], 4: [-168, -116, -64, -12], 5: [-176, -133, -90, -47, -4], 6: [-180, -144, -108, -72, -36, 0], 7: [-184, -153, -121, -90, -59, -27, 4] };
        const angles = anglesMap[Math.min(7, orbitAgents.length)] || [];
        const icons = window.AieyesContract?.providerIcons || {};
        orbitLayer.innerHTML = orbitAgents.slice(0, 7).map((agent, i) => {
          const rad = angles[i] * Math.PI / 180;
          const cx = (116 + 84 * Math.cos(rad)) * scale, cy = (122 + 76 * Math.sin(rad)) * scale;
          const key = icons[agent.displayProvider];
          const inner = agent.provider === '__overflow__' ? `<span>+${(agent.overflowProviders || []).length}</span>` : (key ? `<img src="provider-${key}.png" alt="">` : '');
          const badge = agent.provider !== '__overflow__' && agent.activeCount > 1 ? `<span class="pet-orbit-badge">${agent.activeCount > 9 ? '9+' : agent.activeCount}</span>` : '';
          return `<span class="pet-orbit-node" data-orbit-provider="${agent.provider}" data-phase="${agent.phase}" style="left:${cx.toFixed(1)}px;top:${cy.toFixed(1)}px;--orbit-delay:${(i * -0.35).toFixed(2)}s">${inner}${badge}</span>`;
        }).join('');
        for (const node of orbitLayer.querySelectorAll('.pet-orbit-node')) {
          node.addEventListener('click', (event) => {
            event.stopPropagation();
            event.preventDefault();
            window.__TAURI__?.core?.invoke('desktop_action', { action: 'orbit-agent:' + node.dataset.orbitProvider }).catch(() => {});
          });
        }
      }
      ball.title = AieyesI18n.t("Aieyes · {0}\n单击打开面板 · 拖动移动 · 右键菜单", label);
      ball.setAttribute('aria-label', ball.title);
    }
    schedule();
    if (previousBehavior && previousBehavior !== 'success' && state?.behavior === 'success') celebrate();
  }
  reduced.addEventListener('change', schedule);document.addEventListener('visibilitychange', schedule);
  window.addEventListener('resize', paint);
  window.addEventListener('pagehide', () => {cancelAnimationFrame(timer);timer = null;});
  window.AieyesPet = { update, frame, celebrate };
})();
