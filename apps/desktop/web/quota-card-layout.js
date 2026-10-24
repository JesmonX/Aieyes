// Measure the collapsed counterpart at its real width, even while the visible card is open.
(() => {
  let frame = 0;
  const content = document.querySelector('#content');
  if (!content) return;
  const layer = document.createElement('div');
  layer.className = 'quota-measure-layer';layer.inert = true;layer.setAttribute('aria-hidden', 'true');
  document.body.append(layer);
  function measure() {
    frame = 0;
    const cards = [...content.querySelectorAll('.quota-card[data-provider],.quota-placeholder[data-provider]')].filter(card => card.getBoundingClientRect().width > 0);
    const maxima = new Map();
    for (const card of cards) {
      const clone = card.cloneNode(true), width = card.getBoundingClientRect().width;
      clone.querySelector('.quota-disclosure')?.removeAttribute('open');
      clone.style.setProperty('--quota-collapsed-min', '0px');
      clone.style.width = width + 'px';
      clone.removeAttribute('id');clone.querySelectorAll('[id]').forEach(node => node.removeAttribute('id'));
      const wrapper = document.createElement('div');
      wrapper.className = document.body.dataset.view === 'panel' ? 'panel-body' : '';
      wrapper.style.cssText = 'padding:0;overflow:visible;display:block;width:' + width + 'px';
      wrapper.append(clone);layer.replaceChildren(wrapper);
      const provider = card.dataset.provider === 'agy' ? 'antigravity' : card.dataset.provider;
      maxima.set(provider, Math.max(maxima.get(provider) || 0, Math.ceil(clone.getBoundingClientRect().height)));
    }
    layer.replaceChildren();
    for (const card of cards) {
      const provider = card.dataset.provider === 'agy' ? 'antigravity' : card.dataset.provider;
      const height = maxima.get(provider) + 'px';
      if (card.style.getPropertyValue('--quota-collapsed-min') !== height) card.style.setProperty('--quota-collapsed-min', height);
    }
  }
  const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
  new MutationObserver(schedule).observe(content, { childList:true, subtree:true, characterData:true });
  content.addEventListener('toggle', schedule, true);
  let width = -1;
  new ResizeObserver(() => { if (content.clientWidth !== width) { width = content.clientWidth;schedule(); } }).observe(content);
  new MutationObserver(schedule).observe(document.documentElement, { attributes:true, attributeFilter:['class','style','lang'] });
  new MutationObserver(schedule).observe(document.body, { attributes:true, attributeFilter:['class','style','data-view'] });
  document.fonts?.ready.then(schedule);document.fonts?.addEventListener('loadingdone', schedule);
  addEventListener('resize', schedule);addEventListener('aieyes:language', schedule);schedule();
})();
