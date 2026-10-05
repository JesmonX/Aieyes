/* Hint bubbles. Every control that already documents itself with `title` gets the
   same translucent bubble instead of the OS tooltip. Delegated from the document so
   re-rendered markup keeps working, and positioned with `fixed` so a bubble can never
   widen the page or be clipped by a scrolling panel. */
(() => {
  if (globalThis.AieyesHint) return;
  const DELAY = 520, EDGE = 10, GAP = 9, MIN_ROOM = 26;
  let bubble = null, current = null, timer = 0, text = '';

  const prefersReduced = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  function node() {
    if (bubble && bubble.isConnected) return bubble;
    bubble = document.createElement('div');
    bubble.className = 'hint';
    bubble.setAttribute('role', 'tooltip');
    bubble.hidden = true;
    document.body.appendChild(bubble);
    return bubble;
  }
  function target(event) {
    const el = event.target instanceof Element ? event.target.closest('[title]') : null;
    // The capsule is too small for an in-WebView tooltip; retain the OS tooltip.
    if(el?.id==='ball')return null;
    return el && el.title.trim() ? el : null;
  }
  function place(el) {
    const box = el.getBoundingClientRect(), tip = node().getBoundingClientRect();
    const below = window.innerHeight - box.bottom - GAP;
    const up = below < tip.height + MIN_ROOM && box.top - GAP - tip.height > EDGE;
    const left = Math.min(Math.max(EDGE, box.left + box.width / 2 - tip.width / 2), window.innerWidth - tip.width - EDGE);
    const top = up ? box.top - GAP - tip.height : Math.min(box.bottom + GAP, window.innerHeight - tip.height - EDGE);
    node().style.left = `${Math.round(left)}px`;
    node().style.top = `${Math.round(Math.max(EDGE, top))}px`;
  }
  function hide() {
    clearTimeout(timer); timer = 0;
    if (current) {
      // Only restore a value we removed and nobody has replaced since.
      if (!current.hasAttribute('title') && text) current.setAttribute('title', text);
      current = null;
    }
    if (bubble && !bubble.hidden) {
      bubble.dataset.show = 'false';
      const el = bubble;
      setTimeout(() => { if (el.dataset.show === 'false') el.hidden = true; }, prefersReduced() ? 0 : 160);
    }
  }
  function show(el) {
    text = el.getAttribute('title');
    if (!text || !text.trim()) { hide(); return; }
    current = el;
    el.removeAttribute('title');            // suppress the native tooltip while ours is up
    const tip = node();
    tip.textContent = text.trim();
    tip.hidden = false;
    place(el);
    requestAnimationFrame(() => { if (current === el) tip.dataset.show = 'true'; });
  }
  function arm(el) {
    if (current === el) return;
    hide();
    if (!el) return;
    timer = setTimeout(() => { timer = 0; show(el); }, DELAY);
  }
  document.addEventListener('pointerover', event => {
    const el = target(event);
    if (!el) { if (current && !current.contains(event.target)) hide(); return; }
    if (el === current || el.contains(event.relatedTarget)) return;
    arm(el);
  });
  document.addEventListener('pointerout', event => {
    if (!current || current.contains(event.relatedTarget)) return;
    if (current.contains(event.target) || current === event.target) hide();
  });
  document.addEventListener('focusin', event => { const el = target(event); if (el) arm(el); });
  document.addEventListener('focusout', hide);
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('scroll', hide, true);
  window.addEventListener('blur', hide);
  document.addEventListener('keydown', event => { if (event.key === 'Escape') hide(); });
  window.addEventListener('resize', () => { if (current) place(current); });
  globalThis.AieyesHint = { hide };
})();
