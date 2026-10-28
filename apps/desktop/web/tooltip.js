/* Hint bubbles. Every control that already documents itself with `title` gets the
   same translucent bubble instead of the OS tooltip. Delegated from the document so
   re-rendered markup keeps working, and shown in the top layer so a bubble can never
   widen the page, be clipped by a scrolling panel or hide behind a modal dialog.
   `title` stays on the control: it is parked only while the pointer rests on it, so
   the OS tooltip stays quiet without costing icon-only controls their accessible
   name or the language switch its translation target. */
(() => {
  if (globalThis.AieyesHint) return;
  const DELAY = 520, EDGE = 10, GAP = 9, MIN_ROOM = 26;
  let bubble = null, current = null, pending = null, timer = 0, hideTimer = 0, description = null, parked = null;

  const prefersReduced = () => window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

  function node() {
    if (bubble && bubble.isConnected) return bubble;
    bubble = document.createElement('div');
    bubble.className = 'hint';
    bubble.setAttribute('role', 'tooltip');bubble.id='aieyes-tooltip';
    if (typeof bubble.showPopover === 'function') bubble.popover = 'manual';
    bubble.hidden = true;
    document.body.appendChild(bubble);
    return bubble;
  }
  const hintOf = (el) => (el.getAttribute('title') || el.dataset.hint || '').trim();
  function target(event) {
    if (!(event.target instanceof Element)) return null;
    let el = event.target.closest('[title],[data-hint]');
    // The hovered control's own title is parked; a hint nested inside it still wins.
    if (parked?.el.contains(event.target) && (!el || el.contains(parked.el))) el = parked.el;
    // The pet window is too small for an in-WebView tooltip; retain the OS tooltip.
    if(el?.id==='ball')return null;
    return el && hintOf(el) ? el : null;
  }
  // Text a sighted user already sees on the control; visually hidden labels do not count.
  function visibleText(el) {
    let out = '';
    const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
    for (let text = walker.nextNode(); text; text = walker.nextNode()) {
      const box = text.parentElement?.getBoundingClientRect();
      if (box && box.width > 1 && box.height > 1) out += text.nodeValue;
    }
    return out.replace(/\s+/g, ' ').trim();
  }
  const accessibleName = (el) => (el.getAttribute('aria-label') || el.textContent || '').replace(/\s+/g, ' ').trim();
  // A bubble over an open menu or one that repeats the visible label only gets in the way.
  const redundant = (el, text) => el.getAttribute('aria-expanded') === 'true' || el.tagName === 'SUMMARY' && el.parentElement?.open || visibleText(el) === text;
  // While parked the text lives in data-hint, then both attributes are put back.
  function park(el) {
    if (parked?.el === el) return;
    unpark();
    const text = el.getAttribute('title');
    if (!text) return;
    parked = { el, hint: el.getAttribute('data-hint') };
    el.dataset.hint = text;
    el.removeAttribute('title');
  }
  function unpark() {
    const record = parked;
    parked = null;
    if (!record?.el.isConnected) return;
    if (!record.el.hasAttribute('title')) record.el.setAttribute('title', record.el.dataset.hint);
    if (record.hint === null) record.el.removeAttribute('data-hint');else record.el.setAttribute('data-hint', record.hint);
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
    clearTimeout(hideTimer);
    clearTimeout(timer); timer = 0; pending = null;
    if (current) {
      if(description===null)current.removeAttribute('aria-describedby');else current.setAttribute('aria-describedby',description);
      current = null;
    }
    if (bubble && !bubble.hidden) {
      bubble.dataset.show = 'false';
      const el = bubble;
      hideTimer = setTimeout(() => {
        if (el.dataset.show !== 'false') return;
        el.hidden = true;
        if (el.matches?.(':popover-open')) el.hidePopover();
      }, prefersReduced() ? 0 : 160);
    }
  }
  function show(el) {
    clearTimeout(hideTimer);
    if (!el.isConnected) { hide(); return; }
    const text = hintOf(el);
    if (!text || redundant(el, text)) { hide(); return; }
    current = el;description=el.getAttribute('aria-describedby');
    // A hint that only restates the name would be announced twice.
    if (text !== accessibleName(el)) el.setAttribute('aria-describedby', [description,'aieyes-tooltip'].filter(Boolean).join(' '));
    const tip = node();
    tip.textContent = text;
    tip.hidden = false;
    if (tip.popover && !tip.matches(':popover-open')) try { tip.showPopover(); } catch (_) { hide(); return; }
    place(el);
    requestAnimationFrame(() => { if (current === el) tip.dataset.show = 'true'; });
  }
  function arm(el) {
    if (current === el) return;
    hide();
    if (!el) return;
    // Suppress the OS tooltip before its own delay starts, not after ours fires.
    park(el);
    pending = el;
    timer = setTimeout(() => { timer = 0; pending = null; show(el); }, DELAY);
  }
  document.addEventListener('pointerover', event => {
    const el = target(event);
    if (!el) { if (current && !current.contains(event.target)) hide(); if (parked && !parked.el.contains(event.target)) unpark(); return; }
    if (el === current || el.contains(event.relatedTarget)) return;
    arm(el);
  });
  document.addEventListener('pointerout', event => {
    if (parked && !parked.el.contains(event.relatedTarget) && parked.el.contains(event.target)) unpark();
    const active = current || pending;
    if (!active || active.contains(event.relatedTarget)) return;
    if (active.contains(event.target)) hide();
  });
  // Keyboard focus never shows the OS tooltip, so focus leaves `title` alone and
  // only keyboard navigation (not a click) raises the bubble.
  // Focus can arrive while another popover is still opening; show the bubble once that settles.
  document.addEventListener('focusin', event => { const el = target(event); if (el && event.target.matches(':focus-visible')) queueMicrotask(() => { if (document.activeElement === event.target) { hide();show(el); } }); });
  document.addEventListener('focusout', hide);
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('scroll', hide, true);
  window.addEventListener('blur', () => { hide();unpark(); });
  document.addEventListener('keydown', event => { if (event.key === 'Escape') hide(); });
  window.addEventListener('resize', () => { if (current) place(current); });
  // Controls can update their title while hovered (refresh status, live values).
  // The hovered control keeps it parked; everyone else keeps the attribute.
  new MutationObserver(records => {
    for (const record of records) {
      const el = record.target;
      if (record.type !== 'attributes' || !el.hasAttribute('title')) continue;
      if (parked?.el === el) { el.dataset.hint = el.getAttribute('title');el.removeAttribute('title'); }
      if (current === el) { node().textContent = hintOf(el); place(el); }
    }
    if (parked && !parked.el.isConnected) parked = null;
    if ((current && !current.isConnected) || (pending && !pending.isConnected)) hide();
  }).observe(document.documentElement, {subtree:true, childList:true, attributes:true, attributeFilter:['title']});
  globalThis.AieyesHint = { hide };
})();
