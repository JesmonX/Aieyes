/* A Windows select-only combobox. The HTML select remains the sole form/value
   owner; the enhancement can be removed without migrating any application state. */
(() => {
  const root = document.documentElement,contrast = matchMedia('(forced-colors: active)');
  const instances = new Map();
  let active = null,sequence = 0,observing = false;
  const supported = () => root.dataset.platform === 'windows' && !contrast.matches && typeof HTMLElement.prototype.showPopover === 'function';
  const enabledOption = (option) => !option.disabled && !option.hidden && !option.closest('optgroup[disabled],optgroup[hidden]');
  const textOf = (option) => option?.label ?? '';

  function mount(select) {
    if (!supported() || !(select instanceof HTMLSelectElement) || select.multiple || select.size > 1 || !select.isConnected) return null;
    if (instances.has(select)) return instances.get(select);
    const original = { tabindex: select.getAttribute('tabindex'), hidden: select.getAttribute('aria-hidden') };
    const label = select.getAttribute('aria-label') || [...select.labels].map((label) => {
      const copy = label.cloneNode(true);copy.querySelectorAll('select,button,input,[aria-hidden=true]').forEach((node) => node.remove());return copy.textContent.trim();
    }).filter(Boolean).join(' ') || select.name || AieyesI18n.t("选择");
    const id = select.id ? `${select.id}-choice` : `win-choice-${++sequence}`;
    const wrapper = document.createElement('span');wrapper.className = 'win-select';
    const trigger = document.createElement('button');trigger.type = 'button';trigger.className = 'win-select-trigger';trigger.id = id;
    trigger.setAttribute('role', 'combobox');trigger.setAttribute('aria-haspopup', 'listbox');trigger.setAttribute('aria-expanded', 'false');
    trigger.setAttribute('aria-label', label);
    const caption = document.createElement('span');caption.className = 'win-select-label';trigger.append(caption);
    const menu = document.createElement('div');menu.className = 'win-select-menu';menu.id = `${id}-options`;
    menu.setAttribute('role', 'listbox');menu.setAttribute('aria-label', label);menu.setAttribute('popover', 'manual');
    trigger.setAttribute('aria-controls', menu.id);
    const validation = document.createElement('span');validation.className = 'win-select-validation';validation.id = `${id}-error`;validation.setAttribute('aria-live', 'polite');
    select.before(wrapper);wrapper.append(select, trigger, validation);
    select.setAttribute('tabindex', '-1');select.setAttribute('aria-hidden', 'true');
    const abort = new AbortController(),on = (node, type, fn, options = {}) => node.addEventListener(type, fn, { ...options, signal: abort.signal });
    const properties = new Map();
    let rows = [],index = -1,buffer = '',lastTyped = 0,invalid = false,disposed = false,repositionFrame = 0;

    function options() {return [...select.options].filter((option) => !option.hidden && !option.closest('optgroup[hidden]'));}
    function attributes() {
      for (const name of ['aria-labelledby', 'aria-describedby', 'aria-required']) {
        const value = select.getAttribute(name);if (value !== null) trigger.setAttribute(name, value);else trigger.removeAttribute(name);
      }
      if (select.required) trigger.setAttribute('aria-required', 'true');
      wrapper.style.width = select.style.width;
      wrapper.style.maxWidth = select.style.maxWidth;
      if (select.hasAttribute('aria-label')) trigger.setAttribute('aria-label', select.getAttribute('aria-label'));
      trigger.tabIndex = original.tabindex === null ? 0 : Number(original.tabindex);
    }
    function refresh() {
      if (disposed) return;
      if (select.multiple || select.size > 1) {destroy();return;}
      attributes();
      const option = select.selectedOptions[0];caption.textContent = option ? textOf(option) : AieyesI18n.t("无可选项");
      trigger.disabled = select.matches(':disabled') || !options().some(enabledOption);
      if (select.validity.valid) invalid = false;
      trigger.setAttribute('aria-invalid', String(invalid || select.getAttribute('aria-invalid') === 'true'));
      validation.textContent = invalid ? select.validationMessage : '';
      if (invalid) trigger.setAttribute('aria-errormessage', validation.id);else trigger.removeAttribute('aria-errormessage');
      trigger.title = invalid ? select.validationMessage : select.title || caption.textContent;
      if (active === controller) {
        if (trigger.disabled || !select.isConnected) close(false);else
        {const previous = rows[index]?.option;draw(previous);position();}
      }
    }
    function draw(preferred) {
      const fragment = document.createDocumentFragment();let group = null;
      rows = options().map((option, i) => {
        const parent = option.parentElement;
        if (parent.tagName === 'OPTGROUP' && parent !== group) {
          const heading = document.createElement('div');heading.className = 'win-select-group';heading.textContent = parent.label;heading.setAttribute('role', 'presentation');fragment.append(heading);group = parent;
        }
        const row = document.createElement('div');row.className = 'win-select-option';row.id = `${id}-option-${i}`;
        row.setAttribute('role', 'option');row.setAttribute('aria-disabled', String(!enabledOption(option)));
        row.dataset.selected = String(option.selected);row.title = textOf(option);
        const text = document.createElement('span');text.textContent = textOf(option);row.append(text);fragment.append(row);
        return { option, row };
      });
      menu.replaceChildren(fragment);
      index = rows.findIndex((item) => item.option === (preferred ?? select.selectedOptions[0]) && enabledOption(item.option));
      if (index < 0) index = rows.findIndex((item) => enabledOption(item.option));
      highlight(index, false);
    }
    function highlight(next, scroll = true) {
      index = next;
      for (let i = 0; i < rows.length; i++) {
        rows[i].row.dataset.active = String(i === index);
        rows[i].row.setAttribute('aria-selected', String(i === index));
      }
      if (index < 0) {trigger.removeAttribute('aria-activedescendant');return;}
      trigger.setAttribute('aria-activedescendant', rows[index].row.id);
      if (scroll) {
        const row = rows[index].row,top = row.offsetTop,bottom = top + row.offsetHeight;
        if (top < menu.scrollTop) menu.scrollTop = top;else
        if (bottom > menu.scrollTop + menu.clientHeight) menu.scrollTop = bottom - menu.clientHeight;
      }
    }
    function position() {
      if (active !== controller) return;
      const rect = trigger.getBoundingClientRect(),titlebar = document.querySelector('#titlebar:not([hidden])');
      const topMargin = titlebar ? titlebar.getBoundingClientRect().bottom + 12 : 12;
      if (rect.bottom < topMargin || rect.top > innerHeight || !trigger.getClientRects().length) {close(false);return;}
      const width = Math.min(innerWidth - 24, Math.max(rect.width, Math.min(320, Math.max(160, menu.scrollWidth))));
      menu.style.width = `${width}px`;
      const below = innerHeight - rect.bottom - 16,above = rect.top - topMargin - 4;
      menu.style.maxHeight = `${Math.max(28, Math.min(234, Math.max(above, below)))}px`;
      const height = menu.offsetHeight,down = below >= height || below >= above;
      menu.style.left = `${Math.max(12, Math.min(rect.left, innerWidth - width - 12))}px`;
      menu.style.top = `${Math.max(topMargin, Math.min(down ? rect.bottom + 4 : rect.top - height - 4, innerHeight - height - 12))}px`;
      menu.style.setProperty('--win-menu-origin', down ? 'top center' : 'bottom center');
      menu.style.setProperty('--win-menu-offset', down ? '4px' : '-4px');
    }
    function reposition(event) {
      if (active !== controller || event?.type === 'scroll' && menu.contains(event.target) || repositionFrame) return;
      repositionFrame = requestAnimationFrame(() => {repositionFrame = 0;position();});
    }
    function open() {
      if (disposed || trigger.disabled || !supported()) return;
      active?.close(false);window.AieyesSelect?.closeActive?.();window.AieyesMotion?.finishMenuExits();active = controller;
      (select.closest('dialog') || document.body).append(menu);
      menu.removeAttribute('aria-hidden');
      buffer = '';draw();
      try {menu.showPopover();} catch (_) {active = null;destroy();return;}
      position();trigger.setAttribute('aria-expanded', 'true');trigger.focus({ preventScroll: true });highlight(index);
    }
    function close(focus = true) {
      if (active !== controller) return;
      active = null;
      menu.setAttribute('aria-hidden', 'true');
      if (menu.matches(':popover-open')) menu.hidePopover();
      trigger.setAttribute('aria-expanded', 'false');trigger.removeAttribute('aria-activedescendant');
      cancelAnimationFrame(repositionFrame);repositionFrame = 0;buffer = '';
      if (focus && trigger.isConnected && !trigger.disabled) trigger.focus({ preventScroll: true });
      queueMicrotask(() => window.AieyesApp?.flushRender?.());
    }
    function commit() {
      const option = rows[index]?.option;
      if (!option || !enabledOption(option)) {close();return;}
      const next = [...select.options].indexOf(option),changed = select.selectedIndex !== next;
      close(false);select.selectedIndex = next;invalid = false;refresh();
      if (changed) {select.dispatchEvent(new Event('input', { bubbles: true }));select.dispatchEvent(new Event('change', { bubbles: true }));}
    }
    function move(delta) {
      const choices = rows.flatMap((row, i) => enabledOption(row.option) ? [i] : []),current = choices.indexOf(index);
      if (choices.length) highlight(choices[Math.max(0, Math.min(choices.length - 1, current + delta))]);
    }
    function type(character) {
      const now = performance.now();buffer = now - lastTyped < 700 ? buffer + character : character;lastTyped = now;
      const query = buffer.toLocaleLowerCase(),cycling = [...query].every((c) => c === query[0]),term = cycling ? query[0] : query;
      const start = cycling ? index + 1 : 0;
      for (let i = 0; i < rows.length; i++) {
        const next = (start + i) % rows.length;
        if (enabledOption(rows[next].option) && textOf(rows[next].option).trim().toLocaleLowerCase().startsWith(term)) {highlight(next);return;}
      }
    }
    function keydown(event) {
      if (event.target !== trigger || event.ctrlKey || event.metaKey || event.isComposing) return;
      const wasOpen = active === controller,key = event.key;
      if (key === 'Escape' && wasOpen) {event.preventDefault();event.stopImmediatePropagation();close();return;}
      if (key === 'Tab' && wasOpen) {commit();return;}
      if (['ArrowDown', 'ArrowUp', 'Home', 'End', 'Enter', ' ', 'PageDown', 'PageUp'].includes(key)) {
        event.preventDefault();event.stopPropagation();
        if (!wasOpen) open();
        if (active !== controller) return;
        if (wasOpen && (['Enter', ' '].includes(key) || key === 'ArrowUp' && event.altKey)) {commit();return;}
        if (key === 'Home') move(-rows.length);else if (key === 'End') move(rows.length);else
        if (wasOpen && !event.altKey) move(key === 'ArrowDown' ? 1 : key === 'ArrowUp' ? -1 : key === 'PageDown' ? 8 : key === 'PageUp' ? -8 : 0);
      } else if (key.length === 1 && !event.altKey) {event.preventDefault();if (!wasOpen) open();if (active === controller) type(key);}
    }
    function destroy() {
      if (disposed) return;
      const focused = document.activeElement === trigger;close(false);disposed = true;abort.abort();observer.disconnect();resize.disconnect();
      cancelAnimationFrame(repositionFrame);menu.remove();
      for (const [name, descriptor] of properties) if (descriptor) Object.defineProperty(select, name, descriptor);else delete select[name];
      for (const [name, value] of [['tabindex', original.tabindex], ['aria-hidden', original.hidden]]) if (value === null) select.removeAttribute(name);else select.setAttribute(name, value);
      if (wrapper.parentNode) {wrapper.before(select);wrapper.remove();}
      instances.delete(select);
      if (focused && select.isConnected) select.focus({ preventScroll: true });
    }
    const controller = { select, trigger, menu, refresh, open, close, destroy };instances.set(select, controller);
    // Observe local property assignments as well as browser-generated changes.
    // No prototype patch, polling loop or synthetic change event is involved.
    for (const name of ['value', 'selectedIndex']) {
      const descriptor = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, name);
      properties.set(name, Object.getOwnPropertyDescriptor(select, name));
      Object.defineProperty(select, name, { configurable: true, get() {return descriptor.get.call(this);}, set(value) {descriptor.set.call(this, value);refresh();} });
    }
    const observer = new MutationObserver(refresh);observer.observe(select, { attributes: true, childList: true, subtree: true, characterData: true, attributeFilter: ['disabled', 'required', 'label', 'selected', 'hidden', 'multiple', 'size', 'style', 'title', 'aria-label', 'aria-labelledby', 'aria-describedby', 'aria-required', 'aria-invalid'] });
    const resize = new ResizeObserver(reposition);resize.observe(wrapper);
    on(trigger, 'click', () => active === controller ? close() : open());
    on(select, 'focus', () => trigger.focus({ preventScroll: true }));
    on(select, 'input', refresh);on(select, 'change', refresh);
    on(select, 'invalid', (event) => {event.preventDefault();invalid = true;refresh();trigger.focus({ preventScroll: true });});
    on(document, 'reset', (event) => {if (event.target === select.form) queueMicrotask(() => {invalid = false;refresh();});}, { capture: true });
    on(document, 'keydown', keydown, { capture: true });
    on(document, 'pointerdown', (event) => {if (active === controller && !wrapper.contains(event.target) && !menu.contains(event.target)) close(false);}, { capture: true });
    on(document, 'focusin', (event) => {if (active === controller && event.target !== trigger && !menu.contains(event.target)) close(false);});
    on(menu, 'pointerdown', (event) => event.preventDefault());
    on(menu, 'pointermove', (event) => {const next = rows.findIndex((row) => row.row === event.target.closest('.win-select-option'));if (next >= 0 && enabledOption(rows[next].option) && next !== index) highlight(next, false);});
    on(menu, 'click', (event) => {const next = rows.findIndex((row) => row.row === event.target.closest('.win-select-option'));if (next >= 0 && enabledOption(rows[next].option)) {index = next;commit();}});
    on(menu, 'toggle', (event) => {if (event.newState === 'closed' && active === controller) close(false);});
    on(window, 'resize', reposition);on(document, 'scroll', reposition, { capture: true, passive: true });
    refresh();return controller;
  }
  function mountAll(scope = document) {
    if (!supported()) return;
    if (scope instanceof HTMLSelectElement) mount(scope);
    scope.querySelectorAll?.('select').forEach(mount);
  }
  function refresh(scope = document) {
    mountAll(scope);
    for (const [select, controller] of instances) {if (!select.isConnected) controller.destroy();else if (scope === select || scope.contains(select)) controller.refresh();}
  }
  // Discover only added subtrees. Ordinary text/status changes don't scan or
  // replace the page, and discarded editors release all their listeners.
  const discovery = new MutationObserver((records) => {
    if (!supported()) return;
    for (const record of records) for (const node of record.addedNodes) if (node.nodeType === 1 && !node.closest('.win-select-menu,.win-select')) mountAll(node);
    if (records.some((record) => record.removedNodes.length)) for (const [select, controller] of instances) if (!select.isConnected) controller.destroy();
    const fieldsets = new Set();
    for (const record of records) if (record.type === 'attributes') {
      if (record.target.tagName === 'FIELDSET') fieldsets.add(record.target);else
      if (record.target instanceof HTMLSelectElement && !instances.has(record.target)) mount(record.target);
    }
    for (const fieldset of fieldsets) refresh(fieldset);
  });
  function update() {
    if (supported()) {
      if (!observing && document.body) {
        mountAll();
        discovery.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['disabled', 'multiple', 'size'] });observing = true;
      }
    } else {discovery.disconnect();observing = false;for (const controller of [...instances.values()]) controller.destroy();}
  }
  new MutationObserver(update).observe(root, { attributes: true, attributeFilter: ['data-platform'] });
  contrast.addEventListener('change', update);
  document.addEventListener('DOMContentLoaded', update);
  window.AieyesSingleSelect = { mount, mountAll, refresh, visible: (select) => instances.get(select)?.trigger ?? select, isOpen: () => Boolean(active), closeActive: (focus = false) => active?.close(focus), destroy(scope = document) {for (const [select, controller] of instances) if (scope === select || scope.contains(select)) controller.destroy();} };
  update();
})();
