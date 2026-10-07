/* Hint bubbles. Every control that already documents itself with `title` gets the
   same translucent bubble instead of the OS tooltip. Delegated from the document so
   re-rendered markup keeps working, and positioned with `fixed` so a bubble can never
   widen the page or be clipped by a scrolling panel. */
(() => {
  if (globalThis.AieyesHint) return;
  const DELAY = 520, EDGE = 10, GAP = 9, MIN_ROOM = 26;
  let bubble = null, current = null, pending = null, timer = 0, text = '', description = null;

  const prefersReduced = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  function node() {
    if (bubble && bubble.isConnected) return bubble;
    bubble = document.createElement('div');
    bubble.className = 'hint';
    bubble.setAttribute('role', 'tooltip');bubble.id='aieyes-tooltip';
    bubble.hidden = true;
    document.body.appendChild(bubble);
    return bubble;
  }
  function target(event) {
    const el = event.target instanceof Element ? event.target.closest('[title],[data-hint]') : null;
    // The capsule is too small for an in-WebView tooltip; retain the OS tooltip.
    if(el?.id==='ball')return null;
    return el && (el.getAttribute('title') || el.dataset.hint || '').trim() ? el : null;
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
    clearTimeout(timer); timer = 0; pending = null;
    if (current) {
      if(description===null)current.removeAttribute('aria-describedby');else current.setAttribute('aria-describedby',description);
      current = null;
    }
    if (bubble && !bubble.hidden) {
      bubble.dataset.show = 'false';
      const el = bubble;
      setTimeout(() => { if (el.dataset.show === 'false') el.hidden = true; }, prefersReduced() ? 0 : 160);
    }
  }
  function show(el) {
    if (!el.isConnected) { hide(); return; }
    text = el.getAttribute('title') || el.dataset.hint;
    if (!text || !text.trim()) { hide(); return; }
    current = el;description=el.getAttribute('aria-describedby');el.setAttribute('aria-describedby', [description,'aieyes-tooltip'].filter(Boolean).join(' '));
    el.dataset.hint = text;
    el.removeAttribute('title');
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
    // Suppress the OS tooltip before its own delay starts, not after ours fires.
    el.dataset.hint = el.getAttribute('title') || el.dataset.hint;
    el.removeAttribute('title');
    pending = el;
    timer = setTimeout(() => { timer = 0; pending = null; show(el); }, DELAY);
  }
  document.addEventListener('pointerover', event => {
    const el = target(event);
    if (!el) { if (current && !current.contains(event.target)) hide(); return; }
    if (el === current || el.contains(event.relatedTarget)) return;
    arm(el);
  });
  document.addEventListener('pointerout', event => {
    const active = current || pending;
    if (!active || active.contains(event.relatedTarget)) return;
    if (active.contains(event.target)) hide();
  });
  document.addEventListener('focusin', event => { const el = target(event); if (el) { hide();show(el); } });
  document.addEventListener('focusout', hide);
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('scroll', hide, true);
  window.addEventListener('blur', hide);
  document.addEventListener('keydown', event => { if (event.key === 'Escape') hide(); });
  window.addEventListener('resize', () => { if (current) place(current); });
  // Controls can update their title while hovered (refresh status, live values).
  // Keep one owner of the hint and never allow the native title to reappear.
  new MutationObserver(records => {
    for (const record of records) {
      const el = record.target;
      if (record.type === 'attributes' && el.hasAttribute('title') && el.dataset.hint !== undefined && el.id !== 'ball') {
        el.dataset.hint = el.getAttribute('title');
        el.removeAttribute('title');
        if (current === el) { text = el.dataset.hint; node().textContent = text; place(el); }
      }
    }
    if ((current && !current.isConnected) || (pending && !pending.isConnected)) hide();
  }).observe(document.documentElement, {subtree:true, childList:true, attributes:true, attributeFilter:['title']});
  globalThis.AieyesHint = { hide };
})();
