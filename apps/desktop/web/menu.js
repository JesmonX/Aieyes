/* Menu buttons built on <details data-menu> (§5.3): the <summary> is the trigger and the element
   after it is the menu. Items are not tab stops: ArrowUp/ArrowDown (wrapping), Home/End and the
   first letter move between them; Esc closes the menu and returns focus to the trigger; Tab, focus
   or a click elsewhere, and choosing an item close it. Delegated from the document, so menus that
   are re-rendered with innerHTML keep working. */
(() => {
  if (globalThis.AieyesMenu) return;
  const MENU = 'details[data-menu]';
  const triggerOf = (menu) => menu.querySelector(':scope > summary');
  const bodyOf = (menu) => triggerOf(menu)?.nextElementSibling ?? null;
  const items = (menu) => [...(bodyOf(menu)?.querySelectorAll('button,a[href]') ?? [])].filter((el) => !el.disabled && el.getClientRects().length);
  function prepare(menu) {
    const trigger = triggerOf(menu),body = bodyOf(menu);
    if (!trigger || !body) return;
    trigger.setAttribute('aria-haspopup', 'menu');trigger.setAttribute('aria-expanded', String(menu.open));
    body.setAttribute('role', 'menu');
    for (const item of body.querySelectorAll('button,a[href]')) {if (!item.getAttribute('role')?.startsWith('menuitem')) item.setAttribute('role', 'menuitem');item.tabIndex = -1;}
  }
  function close(menu, restore = false) {
    if (!menu?.open) return;
    menu.open = false;triggerOf(menu)?.setAttribute('aria-expanded', 'false');
    if (restore) triggerOf(menu)?.focus({ preventScroll: true });
  }
  const closeAll = (except = null) => document.querySelectorAll(MENU + '[open]').forEach((menu) => {if (menu !== except) close(menu);});
  function open(menu) {
    closeAll(menu);
    if (!menu.open) menu.open = true;
    prepare(menu);
  }
  document.addEventListener('toggle', (event) => {
    const menu = event.target;
    if (!(menu instanceof HTMLDetailsElement) || !menu.matches(MENU)) return;
    prepare(menu);
    if (menu.open) closeAll(menu);
  }, true);
  document.addEventListener('keydown', (event) => {
    const menu = event.target instanceof Element ? event.target.closest(MENU) : null;
    if (!menu) return;
    if (event.key === 'Escape' && menu.open) {event.preventDefault();event.stopImmediatePropagation();close(menu, true);return;}
    if (event.key === 'Tab') {close(menu);return;}
    const list = items(menu),index = list.indexOf(document.activeElement);
    if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
      event.preventDefault();open(menu);
      const all = items(menu),last = all.length - 1;
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? last : index < 0 ? event.key === 'ArrowUp' ? last : 0 : (index + (event.key === 'ArrowUp' ? -1 : 1) + all.length) % all.length;
      all[next]?.focus({ preventScroll: true });
      return;
    }
    // Type-ahead: the next item whose label starts with the typed character.
    if (menu.open && index >= 0 && event.key.length === 1 && /\S/.test(event.key) && !event.ctrlKey && !event.metaKey && !event.altKey) {
      const key = event.key.toLocaleLowerCase(),rest = [...list.slice(index + 1), ...list.slice(0, index + 1)];
      const hit = rest.find((el) => el.textContent.trim().toLocaleLowerCase().startsWith(key));
      if (hit) {event.preventDefault();hit.focus({ preventScroll: true });}
    }
  }, true);
  document.addEventListener('focusout', (event) => {
    const menu = event.target instanceof Element ? event.target.closest(MENU) : null;
    if (menu?.open && !(event.relatedTarget instanceof Node && menu.contains(event.relatedTarget)) && event.relatedTarget) close(menu);
  });
  document.addEventListener('pointerdown', (event) => closeAll(event.target instanceof Element ? event.target.closest(MENU) : null), true);
  // Choosing an item closes the menu before the item's own handler runs, so a handler that re-renders
  // the row does not reopen it. Focus returns to the trigger unless the action moved it elsewhere.
  document.addEventListener('click', (event) => {
    const item = event.target instanceof Element ? event.target.closest(MENU + ' [role^=menuitem]') : null;
    if (!item) return;
    const menu = item.closest(MENU),trigger = triggerOf(menu);
    close(menu);
    setTimeout(() => {
      const active = document.activeElement;
      if ((!active || active === document.body || active.closest(MENU + ':not([open])')) && trigger?.isConnected) trigger.focus({ preventScroll: true });
    }, 0);
  }, true);
  globalThis.AieyesMenu = { close, closeAll, prepare };
})();
