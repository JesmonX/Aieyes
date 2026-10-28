// Measure the collapsed counterpart at its real width, even while the visible card is open.
(() => {
  let frame = 0;
  const content = document.querySelector('#content');
  if (!content) return;
  const layer = document.createElement('div');
  layer.className = 'quota-measure-layer';layer.inert = true;layer.setAttribute('aria-hidden', 'true');
  document.body.append(layer);
  // Each card's collapsed height is cached against its width and text; only changed cards are cloned.
  const measured = new WeakMap();
  function measure() {
    frame = 0;
    const cards = [...content.querySelectorAll('.quota-card[data-provider],.quota-placeholder[data-provider]')].filter(card => card.getBoundingClientRect().width > 0);
    const maxima = new Map();
    for (const card of cards) {
      const width = card.getBoundingClientRect().width, key = width + '|' + card.textContent, provider = card.dataset.provider === 'agy' ? 'antigravity' : card.dataset.provider;
      const cached = measured.get(card);
      if (cached?.key === key) {maxima.set(provider, Math.max(maxima.get(provider) || 0, cached.height));continue;}
      const clone = card.cloneNode(true);
      clone.querySelector('.quota-disclosure')?.removeAttribute('open');
      clone.style.setProperty('--quota-collapsed-min', '0px');
      clone.style.width = width + 'px';
      clone.removeAttribute('id');clone.querySelectorAll('[id]').forEach(node => node.removeAttribute('id'));
      const wrapper = document.createElement('div');
      wrapper.className = document.body.dataset.view === 'panel' ? 'panel-body' : '';
      wrapper.style.cssText = 'padding:0;overflow:visible;display:block;width:' + width + 'px';
      wrapper.append(clone);layer.replaceChildren(wrapper);
      const height = Math.ceil(clone.getBoundingClientRect().height);measured.set(card, { key, height });
      maxima.set(provider, Math.max(maxima.get(provider) || 0, height));
    }
    layer.replaceChildren();
    for (const card of cards) {
      const provider = card.dataset.provider === 'agy' ? 'antigravity' : card.dataset.provider;
      const height = maxima.get(provider) + 'px';
      if (card.style.getPropertyValue('--quota-collapsed-min') !== height) card.style.setProperty('--quota-collapsed-min', height);
    }
  }
  const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
  // Live values elsewhere in #content (servers, sessions) change constantly; only quota cards need measuring.
  const QUOTA = '.quota-card,.quota-placeholder';
  const touchesQuota = record => {
    const target = record.target.nodeType === 1 ? record.target : record.target.parentElement;
    return !!target?.closest(QUOTA) || [...record.addedNodes, ...record.removedNodes].some(node => node.nodeType === 1 && (node.matches(QUOTA) || node.querySelector(QUOTA)));
  };
  new MutationObserver(records => { if (records.some(touchesQuota)) schedule(); }).observe(content, { childList:true, subtree:true, characterData:true });
  content.addEventListener('toggle', schedule, true);
  let width = -1;
  new ResizeObserver(() => { if (content.clientWidth !== width) { width = content.clientWidth;schedule(); } }).observe(content);
  new MutationObserver(schedule).observe(document.documentElement, { attributes:true, attributeFilter:['class','style','lang'] });
  new MutationObserver(schedule).observe(document.body, { attributes:true, attributeFilter:['class','style','data-view'] });
  document.fonts?.ready.then(schedule);document.fonts?.addEventListener('loadingdone', schedule);
  addEventListener('resize', schedule);addEventListener('aieyes:language', schedule);schedule();
})();
