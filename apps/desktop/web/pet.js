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
  function update(info) {
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
      ball.style.height = (168 + 18 * (label.split('\n').length - 1)) * scale + 'px';
      ball.title = AieyesI18n.t("Aieyes · {0}\n单击打开面板 · 拖动移动 · 右键菜单", label);
      ball.setAttribute('aria-label', ball.title);
    }
    schedule();
  }
  reduced.addEventListener('change', schedule);document.addEventListener('visibilitychange', schedule);
  window.addEventListener('resize', paint);
  window.addEventListener('pagehide', () => {cancelAnimationFrame(timer);timer = null;});
  window.AieyesPet = { update, frame };
})();
