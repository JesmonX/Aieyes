/* Shared selection rules and accessible, persistent checkbox popovers. */
(() => {
  const unique = values => [...new Set(values)];
  const parse = text => unique(String(text).split(',').map(s => s.trim()).filter(Boolean));
  function deviceIds(tokens, group) {
    return tokens.filter(s => s.startsWith(group + ':')).map(s => s.slice(group.length + 1))
      .filter(id => id !== '__none__' && id !== '__all__' && !(group === 'cpu' && id === 'cpu'));
  }
  function deviceSelected(tokens, group, ids) {
    if(tokens.includes(group+':__all__'))return [...ids];
    return tokens.some(s => s.startsWith(group + ':')) ? deviceIds(tokens, group) : [...ids].filter(id=>group!=='filesystems'||recommendedFilesystem(id));
  }
  function writeDevices(tokens, group, selected, all = false) {
    const remaining = tokens.filter(s => !s.startsWith(group + ':'));
    const ids = unique(selected).filter(id => id !== '__none__' && id !== '__all__' && !(group === 'cpu' && id === 'cpu'));
    return [...remaining, ...(all ? (group==='filesystems'?['filesystems:__all__']:[]) : (ids.length ? ids : ['__none__']).map(id => group + ':' + id))];
  }
  function applySelection(selected, targets, action) {
    const result = new Set(selected);
    for (const id of targets) {
      if (action === 'all' || (action === 'invert' && !result.has(id))) result.add(id);
      else result.delete(id);
    }
    return [...result];
  }
  function filterOptions(options, query) {
    const term = query.trim().toLocaleLowerCase();
    return options.filter(o => `${o.id} ${o.label}`.toLocaleLowerCase().includes(term));
  }
  let active = null, sequence = 0;
  function mount(root, config) {
    const doc = root.ownerDocument;
    const trigger = doc.createElement('button');
    trigger.type = 'button'; trigger.className = 'multi-trigger';
    const panel = doc.createElement('div');
    panel.className = 'multi-panel'; panel.id = `multi-panel-${++sequence}`;
    panel.hidden = true; panel.setAttribute('role', 'group'); panel.setAttribute('aria-label', config.title);
    // A top-layer popover escapes both the modal's scrolling area and glass layers.
    if (typeof panel.showPopover === 'function') panel.setAttribute('popover', 'manual');
    trigger.setAttribute('aria-controls', panel.id); trigger.setAttribute('aria-expanded', 'false');
    const search = doc.createElement('input');
    search.type = 'search'; search.placeholder = '搜索'; search.setAttribute('aria-label', `搜索${config.title}`);
    const toolbar = doc.createElement('div'); toolbar.className = 'multi-toolbar';
    const list = doc.createElement('div'); list.className = 'multi-list';
    const count = doc.createElement('span'); count.className = 'multi-count'; count.setAttribute('aria-live', 'polite');
    const buttons = ['all', 'clear', 'invert'].map((action, index) => {
      const button = doc.createElement('button'); button.type = 'button'; button.textContent = ['全选','清空','反选'][index];
      button.onclick = () => {
        const targets = filterOptions(config.options(), search.value).map(o => o.id);
        config.onChange(applySelection(config.selected(), targets, action), {all: action === 'all' && !search.value.trim()});
        refresh();
      };
      toolbar.append(button); return button;
    });
    toolbar.append(count); panel.append(search, toolbar, list);
    root.replaceChildren(trigger);
    (root.closest('dialog') || doc.body).append(panel);
    let rows = [];
    function renderOptions() {
      list.replaceChildren();
      rows = filterOptions(config.options(), search.value).map(option => {
        const label = doc.createElement('label'); label.className = 'multi-option';
        const input = doc.createElement('input'); input.type = 'checkbox'; input.value = option.id;
        const text = doc.createElement('span'); text.textContent = option.label;
        input.onchange = () => {
          config.onChange(applySelection(config.selected(), [option.id], input.checked ? 'all' : 'clear'), {all:false});
          refresh();
        };
        label.append(input, text);
        if (option.unavailable) { const hint = doc.createElement('small'); hint.textContent = '暂不可用'; label.append(hint); }
        list.append(label); return input;
      });
      if (!rows.length) { const empty = doc.createElement('p'); empty.className = 'multi-empty'; empty.textContent = '没有匹配项'; list.append(empty); }
      refresh();
    }
    function refresh() {
      const options = config.options(), selected = new Set(config.selected());
      const total = options.length, n = options.filter(o => selected.has(o.id)).length;
      const isAll = config.all ? config.all() : total > 0 && n === total;
      const summary = isAll ? '全部' : n ? `已选 ${n} / ${total}` : '未选择';
      trigger.textContent = `${config.title} · ${summary}`;
      trigger.title = `${config.title}：${summary}`;
      trigger.disabled = Boolean(config.disabled?.());
      if (trigger.disabled && active === controller) close(false);
      for (const row of rows) row.checked = selected.has(row.value);
      count.textContent = `${n} / ${total}`;
      buttons.forEach((b, i) => { b.textContent = ['全选','清空','反选'][i] + (search.value.trim() ? '结果' : ''); b.disabled = !rows.length; });
    }
    function position() {
      const rect = trigger.getBoundingClientRect(), width = Math.min(380, window.innerWidth - 24);
      panel.style.width = `${width}px`;
      panel.style.left = `${Math.max(12, Math.min(rect.left, window.innerWidth - width - 12))}px`;
      const below = window.innerHeight - rect.bottom - 12;
      const topMargin = doc.body.dataset.platform === 'windows' ? 52 : 12;
      const above = rect.top - topMargin;
      const height = Math.min(360, Math.max(above, below));
      panel.style.maxHeight = `${height}px`;
      panel.style.top = `${below >= Math.min(360, above) ? rect.bottom + 4 : Math.max(topMargin, rect.top - height - 4)}px`;
    }
    function outside(event) { if (!panel.contains(event.target) && !trigger.contains(event.target)) close(false); }
    function keydown(event) {
      if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); close(true); }
    }
    function focusout(event) { if (!panel.contains(event.target) && event.target !== trigger) close(false); }
    function open() {
      active?.close(false); active = controller; search.value = ''; panel.hidden = false;
      renderOptions(); position();
      if (panel.showPopover) panel.showPopover();
      trigger.setAttribute('aria-expanded', 'true'); search.focus();
      doc.addEventListener('pointerdown', outside, true); doc.addEventListener('keydown', keydown, true);
      doc.addEventListener('focusin', focusout); window.addEventListener('resize', reposition);
      doc.addEventListener('scroll', reposition, true);
    }
    function reposition(event) { if (!panel.contains(event.target)) position(); }
    function close(focus = true) {
      if (panel.hidePopover && panel.matches(':popover-open')) panel.hidePopover();
      panel.hidden = true; trigger.setAttribute('aria-expanded', 'false');
      doc.removeEventListener('pointerdown', outside, true); doc.removeEventListener('keydown', keydown, true);
      doc.removeEventListener('focusin', focusout); window.removeEventListener('resize', reposition);
      doc.removeEventListener('scroll', reposition, true);
      if (active === controller) active = null;
      if (focus) trigger.focus();
    }
    const controller = {refresh, close, destroy() {close(false); panel.remove();}};
    trigger.onclick = () => panel.hidden ? open() : close();
    search.oninput = renderOptions;
    refresh(); return controller;
  }
  function recommendedFilesystem(id,type='') {
    const path=id.toLowerCase();return !['tmpfs','devtmpfs','squashfs','overlay','proc','procfs','sysfs','devfs','autofs','cgroup','cgroup2'].includes(type.toLowerCase())&&!['/snap','/var/lib/snapd/snap','/run','/efi','/boot/efi','/dev','/proc','/sys','/system/volumes/preboot','/system/volumes/vm','/system/volumes/update','/system/volumes/xarts','/system/volumes/iscpreboot','/system/volumes/hardware'].some(p=>path===p||path.startsWith(p+'/'));
  }
  globalThis.AieyesSelect = {recommendedFilesystem,parse, deviceIds, deviceSelected, writeDevices, applySelection, filterOptions, mount};
})();
