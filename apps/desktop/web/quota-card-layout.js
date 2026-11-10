// Measure the collapsed counterpart at its real width, even while the visible card is open.
(() => {
  let frame = 0;
  // Cards that share a layout family settle on one collapsed height.
  const PROVIDER_ALIAS = { agy: 'antigravity', 'chatgpt-work': 'codex' };
  const content = document.querySelector('#content');
  if (!content) return;
  const layer = document.createElement('div');
  layer.className = 'quota-measure-layer';layer.inert = true;layer.setAttribute('aria-hidden', 'true');
  document.body.append(layer);
  // Each card's collapsed height is cached against its width and text; only changed cards are cloned.
  let measured = new WeakMap();
  function measure() {
    frame = 0;
    // Read the live grid before inserting any probes. Measuring after each insertion
    // makes the browser lay out the entire quota grid once for every account.
    const cards = [...content.querySelectorAll('.quota-card[data-provider],.quota-placeholder[data-provider]')]
      .map(card => ({card, width:card.getBoundingClientRect().width})).filter(({width}) => width > 0);
    const maxima = new Map(), probes = [], fragment = document.createDocumentFragment();
    for (const {card, width} of cards) {
      const key = width + '|' + card.textContent, provider = (PROVIDER_ALIAS[card.dataset.provider] ?? card.dataset.provider);
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
      wrapper.append(clone);fragment.append(wrapper);probes.push({card, clone, key, provider});
    }
    // One insertion and one layout for every changed card, followed by writes only.
    if (probes.length) layer.replaceChildren(fragment);
    for (const {card, clone, key, provider} of probes) {
      const height = Math.ceil(clone.getBoundingClientRect().height);measured.set(card, {key, height});
      maxima.set(provider, Math.max(maxima.get(provider) || 0, height));
    }
    if (probes.length) layer.replaceChildren();
    for (const {card} of cards) {
      const provider = (PROVIDER_ALIAS[card.dataset.provider] ?? card.dataset.provider);
      const height = maxima.get(provider) + 'px';
      if (card.style.getPropertyValue('--quota-collapsed-min') !== height) card.style.setProperty('--quota-collapsed-min', height);
    }
  }
  const schedule = () => { if (!frame) frame = requestAnimationFrame(measure); };
  const invalidate = () => { measured = new WeakMap();schedule(); };
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
  new MutationObserver(invalidate).observe(document.documentElement, { attributes:true, attributeFilter:['class','style','lang'] });
  new MutationObserver(invalidate).observe(document.body, { attributes:true, attributeFilter:['class','style','data-view'] });
  document.fonts?.ready.then(invalidate);document.fonts?.addEventListener('loadingdone', invalidate);
  addEventListener('resize', schedule);addEventListener('aieyes:language', invalidate);schedule();
})();
// Masonry for multi-column quota grids. Each item goes to the shortest column and spans rows of a
// small implicit track in proportion to its height, so an expanded card leaves no hole beside it.
// A group occupies only as many columns as it has accounts and reports how far each column reaches, so groups
// after it start under its shorter columns. Only grid lines are written: DOM, reading and Tab order
// are unchanged, and single-column grids (the panel, narrow windows) keep normal flow.
(() => {
  const content = document.querySelector('#content');
  if (!content) return;
  const GRIDS = '.quota-groups,.quotas', UNIT = 4, observed = new Set();
  // Placement is written per column count (--masonry-row-N, --masonry-column-N); workspace.css applies
  // it only while N columns are current, so a resize falls back to plain grid flow until this re-runs.
  const clear = (element, keep = '') => {for (const name of [...element.style]) if (name.startsWith('--masonry-') && !name.endsWith('-' + keep)) element.style.removeProperty(name);};
  const reset = grid => {
    if (!grid.dataset.masonry) return;
    delete grid.dataset.masonry;clear(grid);for (const item of grid.children) clear(item);
  };
  function layout(grid) {
    if (!grid.isConnected) return;
    const style = getComputedStyle(grid), columns = style.display === 'grid' ? style.gridTemplateColumns.split(' ').filter(Boolean).length : 0;
    const items = [...grid.children].filter(item => item.getClientRects().length);
    if (columns < 2 || items.length < 2) {reset(grid);return;}
    // A new column count is not styled yet, so the natural row gap is readable; it becomes part of each span.
    if (grid.dataset.masonry !== String(columns)) {
      reset(grid);grid.style.setProperty('--masonry-gap', (parseFloat(style.rowGap) || 0) + 'px');grid.dataset.masonry = String(columns);
    }
    const gap = parseFloat(grid.style.getPropertyValue('--masonry-gap')) || 0, rows = px => Math.max(1, Math.ceil((px + gap) / UNIT));
    const ROW = '--masonry-row-' + columns, COLUMN = '--masonry-column-' + columns;
    // Read everything first, then write, so the layout is computed once.
    const measured = items.map(item => {
      const box = item.getBoundingClientRect(), width = item.matches('.quota-group-shell') ? Math.min(columns, Number(item.style.getPropertyValue('--quota-account-count')) || columns) : 1;
      let reach = null;
      if (width > 1) {
        const inner = item.querySelector(`.quotas[data-masonry="${width}"]`), area = inner?.getBoundingClientRect();
        if (inner && Math.abs(area.width - box.width) < 1) {
          reach = Array(width).fill(area.top - box.top);
          for (const card of inner.children) if (card.getClientRects().length) {const column = parseInt(card.style.getPropertyValue('--masonry-column-' + width), 10) - 1;if (column >= 0 && column < width) reach[column] = Math.max(reach[column], card.getBoundingClientRect().bottom - box.top);}
        }
      }
      return { item, span: rows(box.height), width, reach };
    });
    const filled = Array(columns).fill(0);
    for (const { item, span, width, reach } of measured) {
      // Choose the lowest contiguous block that fits the group; untouched columns stay available.
      const starts = Array.from({ length: columns - width + 1 }, (_, i) => Math.max(...filled.slice(i, i + width)));
      const start = Math.min(...starts), index = starts.indexOf(start);
      for (let i = 0; i < width; i++) filled[index + i] = start + (reach ? rows(reach[i]) : span);
      const column = width === 1 ? String(index + 1) : `${index + 1} / span ${width}`;
      const row = `${start + 1} / span ${span}`;
      if (item.style.getPropertyValue(ROW) !== row) item.style.setProperty(ROW, row);
      if (column && item.style.getPropertyValue(COLUMN) !== column) item.style.setProperty(COLUMN, column);
    }
  }
  // Only cards are observed. Spans settle inside the callback, before paint, so a growing card
  // never overlaps its neighbour for a frame; the outer group grid follows from the same reads.
  const sizes = new ResizeObserver(rows => {
    const grids = new Set();
    for (const { target } of rows) {
      if (!target.isConnected) {sizes.unobserve(target);observed.delete(target);continue;}
      for (let grid = target.parentElement?.closest(GRIDS); grid && content.contains(grid); grid = grid.parentElement?.closest(GRIDS)) grids.add(grid);
    }
    // Card grids first: their spans set the height of the group grid around them.
    for (const inner of [true, false]) for (const grid of grids) if (grid.matches('.quotas') === inner) layout(grid);
  });
  // Group headings move their cards without resizing them (font loading, language), so they are observed too.
  const WATCHED = '.quotas > *, .quota-group-shell .quota-group-heading';
  const discover = () => {
    // A card removed while its provider group is collapsed already has zero size,
    // so ResizeObserver need not deliver another entry that could release it.
    for (const card of observed) if (!content.contains(card)) {sizes.unobserve(card);observed.delete(card);}
    for (const card of content.querySelectorAll(WATCHED)) if (!observed.has(card)) {observed.add(card);sizes.observe(card);}
  };
  new MutationObserver(discover).observe(content, { childList:true, subtree:true });
  discover();
  const relayout = () => requestAnimationFrame(() => {for (const inner of [true, false]) for (const grid of content.querySelectorAll(GRIDS)) if (grid.matches('.quotas') === inner) layout(grid);});
  document.fonts?.ready.then(relayout);document.fonts?.addEventListener('loadingdone', relayout);addEventListener('aieyes:language', relayout);
})();
