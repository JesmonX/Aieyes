/* Compact panel and Linux pet. Windows pet input lives in native_pet.rs. */
(() => {
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const ball = document.querySelector('#ball');
  const panel = document.querySelector('#panel');
  let labels = { idle: AieyesI18n.t("空闲"), working: AieyesI18n.t("进行中"), thinking: AieyesI18n.t("思考中"), tool: AieyesI18n.t("执行工具"), complete: AieyesI18n.t("已完成"), interrupted: AieyesI18n.t("已中断"), unknown: AieyesI18n.t("待确认"), needsInput: AieyesI18n.t("等待输入"), failed: AieyesI18n.t("执行失败") };window.addEventListener("aieyes:language", () => {labels = { idle: AieyesI18n.t("空闲"), working: AieyesI18n.t("进行中"), thinking: AieyesI18n.t("思考中"), tool: AieyesI18n.t("执行工具"), complete: AieyesI18n.t("已完成"), interrupted: AieyesI18n.t("已中断"), unknown: AieyesI18n.t("待确认"), needsInput: AieyesI18n.t("等待输入"), failed: AieyesI18n.t("执行失败") };});
  const SUPPRESS = 250;
  let start = null,dragged = false,info = null;
  let panelOpen = false,pinned = false,suppress = 0,orbitAgent = null,lastOrbitEntry = null,orbitTimer = 0;
  let orbitHeight = null,orbitLayoutFrame = 0,orbitFocusAsked = false;

  function isDarkTheme() {
    const t = document.documentElement.dataset.theme;
    if (t === 'dark') return true;
    if (t === 'light') return false;
    return !!window.matchMedia?.('(prefers-color-scheme: dark)').matches;
  }
  function orbitProviderIcon(provider) {
    const dark = isDarkTheme();
    const icons = window.AieyesContract?.providerIcons || {};
    const darkIcons = window.AieyesContract?.providerDarkIcons || {};
    const mono = window.AieyesContract?.monochromeIcons || [];
    const key = (dark && darkIcons[provider]) || icons[provider];
    return key ? { src: `provider-${key}.png`, mono: mono.includes(provider) } : null;
  }
  function orbitProviderName(provider) {
    if (provider === '__overflow__') return AieyesI18n.t("其他运行中 Agent");
    return window.AieyesContract?.providers?.[provider] || provider;
  }
  function orbitRingSVG(ring, provider) {
    const r = 13.5, cx = 16, cy = 16, sw = 2.4;
    const polar = (deg) => { const rad = deg * Math.PI / 180; return [cx + r * Math.cos(rad), cy + r * Math.sin(rad)]; };
    const arcPath = (startDeg, sweepDeg) => {
      if (Math.abs(sweepDeg) >= 359.5) return `M ${cx} ${cy - r} A ${r} ${r} 0 1 1 ${cx - 0.01} ${cy - r}`;
      const [sx, sy] = polar(startDeg), [ex, ey] = polar(startDeg + sweepDeg);
      const large = Math.abs(sweepDeg) > 180 ? 1 : 0, sweepFlag = sweepDeg >= 0 ? 1 : 0;
      return `M ${sx.toFixed(2)} ${sy.toFixed(2)} A ${r} ${r} 0 ${large} ${sweepFlag} ${ex.toFixed(2)} ${ey.toFixed(2)}`;
    };
    const tone = (rem) => rem < 10 ? 'var(--danger)' : rem <= 30 ? 'var(--warn)' : `var(--category-${provider}, var(--accent))`;
    const track = 'color-mix(in srgb, var(--ink) 18%, transparent)';
    if (!ring || ring.kind === 'empty') {
      return `<svg viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="${r}" fill="none" stroke="${track}" stroke-width="1.6" stroke-dasharray="1.5 3"/></svg>`;
    }
    if (ring.kind === 'coin') {
      const c = ring.low ? 'var(--danger)' : `var(--category-${provider}, var(--accent))`;
      return `<svg viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="${r}" fill="none" stroke="${c}" stroke-opacity="0.65" stroke-width="1.8"/></svg>`;
    }
    if (ring.kind === 'single') {
      const rem = Math.max(0, Math.min(100, Number(ring.top?.remaining ?? 0)));
      const fg = rem > 0 ? `<path d="${arcPath(270, Math.max(1, 360 * rem / 100))}" fill="none" stroke="${tone(rem)}" stroke-width="${sw}" stroke-linecap="round"/>` : '';
      return `<svg viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="${r}" fill="none" stroke="${track}" stroke-width="${sw}"/>${fg}</svg>`;
    }
    const half = ((sw / 2 + 0.8) / r) * 180 / Math.PI, span = 180 - 2 * half;
    const topRem = Math.max(0, Math.min(100, Number(ring.top?.remaining ?? 0)));
    const botRem = Math.max(0, Math.min(100, Number(ring.bottom?.remaining ?? 0)));
    const topFg = topRem > 0 ? `<path d="${arcPath(180 + half, Math.max(1, span * topRem / 100))}" fill="none" stroke="${tone(topRem)}" stroke-width="${sw}" stroke-linecap="round"/>` : '';
    const botFg = botRem > 0 ? `<path d="${arcPath(180 - half, -Math.max(1, span * botRem / 100))}" fill="none" stroke="${tone(botRem)}" stroke-width="${sw}" stroke-linecap="round"/>` : '';
    return `<svg viewBox="0 0 32 32" aria-hidden="true"><path d="${arcPath(180 + half, span)}" fill="none" stroke="${track}" stroke-width="${sw}" stroke-linecap="round"/><path d="${arcPath(180 - half, -span)}" fill="none" stroke="${track}" stroke-width="${sw}" stroke-linecap="round"/>${topFg}${botFg}</svg>`;
  }
  function stopOrbitTick() { if (orbitTimer) { clearInterval(orbitTimer); orbitTimer = 0; } }
  /* The shell sizes the popover window to an interaction card: report the height its content needs,
     or 0 when no card is shown so the shell falls back to its own session-list size. */
  function reportOrbitLayout() {
    if (orbitLayoutFrame) return;
    orbitLayoutFrame = requestAnimationFrame(() => {
      orbitLayoutFrame = 0;
      const pop = document.querySelector('#orbit-popover');
      const body = pop?.querySelector('.orbit-body');
      if (!panelOpen || !orbitAgent || !body) return;
      const card = document.querySelector('#orbit-interaction');
      // A card that is still loading has no size worth resizing the window for.
      if (card && !card.hidden && card.querySelector('.orbit-iloading')) return;
      let height = 0;
      if (card && !card.hidden) {
        const last = [...body.children].filter((el) => !el.hidden && el.offsetHeight > 0).pop();
        const content = last ? last.getBoundingClientRect().bottom - body.getBoundingClientRect().top + body.scrollTop + (parseFloat(getComputedStyle(body).paddingBottom) || 0) : 0;
        height = Math.ceil((pop.querySelector('.orbit-head')?.offsetHeight || 0) + content + (pop.querySelector('.orbit-foot')?.offsetHeight || 0));
      }
      if (height === orbitHeight) return;
      orbitHeight = height;
      invoke('desktop_orbit_popover', { height }).catch(() => {});
    });
  }
  function renderOrbitPopover(next) {
    const pop = document.querySelector('#orbit-popover');
    if (!pop) return;
    if (!panelOpen || !orbitAgent) { pop.hidden = true; stopOrbitTick(); return; }
    pop.hidden = false;
    const esc = (v) => window.AieyesUI ? window.AieyesUI.escapeHTML(v) : String(v ?? '');
    const entries = Array.isArray(next?.petOrbit) ? next.petOrbit : [];
    let entry = entries.find((item) => item.provider === orbitAgent);
    if (entry) lastOrbitEntry = entry;
    else if (lastOrbitEntry?.provider === orbitAgent) entry = lastOrbitEntry;
    else entry = { provider: orbitAgent, displayProvider: orbitAgent, activeCount: 0, overflowProviders: [], quota: null };

    const isOverflow = entry.provider === '__overflow__';
    const headIcon = document.querySelector('#orbit-head-icon');
    const iconInfo = !isOverflow ? orbitProviderIcon(entry.displayProvider) : null;
    headIcon.innerHTML = isOverflow
      ? `<svg viewBox="0 0 32 32" aria-hidden="true"><circle cx="16" cy="16" r="13.5" fill="none" stroke="var(--accent)" stroke-opacity="0.55" stroke-width="2"/></svg><span style="font-size:11px;font-weight:600;position:relative;z-index:1">+${(entry.overflowProviders || []).length}</span>`
      : orbitRingSVG(entry.quota?.ring, entry.displayProvider) + (iconInfo ? `<img src="${esc(iconInfo.src)}" alt="" class="${iconInfo.mono ? 'mono' : ''}">` : '');

    document.querySelector('#orbit-title').textContent = orbitProviderName(entry.displayProvider);
    document.querySelector('#orbit-subtitle').textContent = entry.quota?.accountName || (isOverflow ? AieyesI18n.t("还有 {0} 个运行中 Agent", (entry.overflowProviders || []).length) : AieyesI18n.t("进行中会话"));
    document.querySelector('#orbit-quota').textContent = entry.quota?.summary || (isOverflow ? '' : AieyesI18n.t("暂无额度"));

    const sessionsApi = window.AieyesSessions;
    const rawSessions = Array.isArray(next?.sessions) ? next.sessions : [];
    const active = rawSessions.filter((s) => {
      if (!sessionsApi?.active?.(s.phase)) return false;
      if (isOverflow) {
        const alias = window.AieyesContract?.agentAliases?.[s.provider] || s.provider;
        return (entry.overflowProviders || []).includes(alias);
      }
      return window.AieyesUI ? window.AieyesUI.sameAgent(s.provider, entry.provider) : s.provider === entry.provider;
    }).sort((a, b) => {
      const ra = a.phase === 'needsInput' ? 0 : 1, rb = b.phase === 'needsInput' ? 0 : 1;
      if (ra !== rb) return ra - rb;
      const sa = Number(a.turnStartedAt || 0), sb = Number(b.turnStartedAt || 0);
      return sa !== sb ? sb - sa : String(a.id).localeCompare(String(b.id));
    });

    const interactionShown = !!window.AieyesOrbitInteraction?.update(next, entry, active);
    const listSessions = interactionShown ? active.filter((s) => s.phase !== 'needsInput') : active;
    const list = document.querySelector('#orbit-session-list');
    if (!active.length) {
      list.innerHTML = `<div class="orbit-empty"><span>${esc(AieyesI18n.t("当前会话已结束"))}</span><small>${esc(AieyesI18n.t("可打开会话页查看最近结束记录"))}</small></div>`;
    } else if (!listSessions.length) {
      list.innerHTML = '';
    } else {
      const heading = interactionShown ? `<div class="orbit-subhead">${esc(AieyesI18n.t("其他进行中会话"))}</div>` : '';
      list.innerHTML = heading + listSessions.map((s) => {
        const key = sessionsApi.phaseKey(s.phase), label = sessionsApi.phaseLabel(s.phase);
        const symbol = typeof uiIcon === 'function' ? uiIcon(key) : '';
        const focus = sessionsApi.canFocus(s) ? ` role="button" tabindex="0" data-focus-session="${esc(s.id)}" title="${esc(sessionsApi.focusHelp(s))}"` : '';
        const title = (isOverflow ? `${orbitProviderName(s.provider)} · ` : '') + (s.machine || AieyesI18n.t("本机"));
        const row = `<div class="live-row" data-phase="${esc(s.phase)}"${focus}><span class="phase-mark" data-phase="${esc(s.phase)}" aria-hidden="true">${symbol}</span><div><strong>${esc(title)}</strong><small class="session-detail" data-orbit-tick="${esc(s.id)}">${esc(sessionsApi.detailText(s))}</small>${sessionsApi.usageHTML(s)}</div><span>${esc(label)}</span>${sessionsApi.canFocus(s) ? `<span class="session-jump-mark" aria-hidden="true">${typeof uiIcon === 'function' ? uiIcon('expand') : ''}</span>` : ''}</div>`;
        // A running session that takes a follow-up or an interrupt gets its own button beside the row.
        const follow = window.AieyesOrbitInteraction?.followUpHTML?.(s) || '';
        return follow ? `<div class="orbit-row">${row}${follow}</div>` : row;
      }).join('');
    }
    document.querySelector('#orbit-count').textContent = active.length ? AieyesI18n.t("{0} 个进行中", active.length) : AieyesI18n.t("空闲");
    document.querySelector('#orbit-open-sessions').textContent = AieyesI18n.t("全部会话");
    document.querySelector('#orbit-close').textContent = AieyesI18n.t("关闭");
    stopOrbitTick();
    if (active.length && !document.hidden) {
      orbitTimer = setInterval(() => {
        if (!panelOpen || !orbitAgent || document.hidden) { stopOrbitTick(); return; }
        for (const el of document.querySelectorAll('#orbit-session-list [data-orbit-tick]')) {
          const s = info?.sessions?.find((row) => row.id === el.dataset.orbitTick);
          if (s && window.AieyesSessions) el.textContent = window.AieyesSessions.detailText(s);
        }
      }, 1000);
    }
    reportOrbitLayout();
  }

  function failure(error) {
    document.querySelector('#phase').textContent = AieyesI18n.t("连接异常");
    document.body.dataset.active = 'false';
    ball.title = AieyesI18n.t("{0}；右键打开菜单", String(error));
    if (panelOpen && typeof notify === 'function') notify(String(error), 'error');
  }
  async function action(name) {
    try {return await invoke('desktop_action', { action: name });} catch (error) {failure(error);}
  }
  function closeMenu(restoreFocus = false) {
    const menu = document.querySelector('#panel-menu');
    if (!menu || menu.hidden) return;
    menu.hidden = true;
    document.querySelector('#panel-refresh')?.setAttribute('aria-expanded', 'false');
    if (restoreFocus) document.querySelector('#panel-refresh')?.focus({ preventScroll: true });
  }
  function applyPanel(next) {
    const was = panelOpen;
    const wasOrbit = orbitAgent;
    const focused = document.activeElement;
    panelOpen = !!next.panelOpen;
    pinned = !!next.panelPinned;
    if (next.orbitAgent !== undefined) orbitAgent = next.orbitAgent || null;
    if (!panelOpen) orbitAgent = null;
    document.body.dataset.view = panelOpen ? (orbitAgent ? 'orbit' : 'panel') : 'ball';
    if (info) window.AieyesPet?.update({ ...info, panelOpen, orbitAgent });
    // The shell forgets the reported height and the passive state whenever the popover changes.
    if (!panelOpen || orbitAgent !== wasOrbit) {orbitHeight = null;orbitFocusAsked = false;}
    // A follow-up card and a picked request belong to the agent they were opened for.
    if (panelOpen && wasOrbit && orbitAgent && orbitAgent !== wasOrbit) window.AieyesOrbitInteraction?.reset();
    renderOrbitPopover(info || next);
    const pin = document.querySelector('#panel-pin');
    if (pin) {
      pin.dataset.pinned = String(pinned);
      pin.setAttribute('aria-pressed', String(pinned));
      pin.textContent = pinned ? AieyesI18n.t("取消固定面板") : AieyesI18n.t("固定面板");
      pin.title = pinned ? AieyesI18n.t("取消固定") : AieyesI18n.t("固定面板");
      pin.setAttribute('aria-label', pin.title);
    }
    if (was && !panelOpen) {
      stopOrbitTick();
      window.AieyesOrbitInteraction?.reset();
      window.AieyesSingleSelect?.closeActive(false);
      window.AieyesMotion?.panel(false);
      closeMenu();
      const more = document.querySelector('#panel-more');
      if (more) more.open = false;
      if (typeof closeTokenSummary === 'function') closeTokenSummary();
      if (!info?.nativePet && panel.contains(focused)) ball.focus({ preventScroll: true });
    }
    if ((!was && panelOpen) || (panelOpen && wasOrbit && !orbitAgent)) {
      if (!orbitAgent) {
        window.AieyesNetwork?.refresh();
        if (typeof render === 'function') render();
      }
      window.AieyesMotion?.panel(true);
      requestAnimationFrame(() => {if (!orbitAgent && typeof drawTrend === 'function') drawTrend();invoke('desktop_panel_ready', { generation: window.AIEYES_SHELL_GENERATION ?? 0 }).catch(failure);});
      // The Linux panel window takes keyboard focus; park it on the panel itself so opening shows no
      // focus ring or tooltip, and Tab starts at the first control. Only a popover that opened by
      // itself is left alone: orbitAuto says nothing about the full panel.
      if (info?.platform === 'linux' && !(orbitAgent && (next?.orbitAuto || info?.orbitAuto))) requestAnimationFrame(() => {const target = orbitAgent ? document.querySelector('#orbit-popover') : panel;if (target) {target.tabIndex = -1;target.focus({ preventScroll: true });}});
      // Mouse-opened floating surfaces preserve the other application's keyboard focus.
      const refresh = !orbitAgent && window.AieyesApp?.refreshPanel;
      if (refresh) refresh().catch((error) => failure(error));
    }
  }
  async function setPanel(open) {
    if (panelOpen === open) return;
    try {applyPanel(await invoke('desktop_panel', { open }));}
    catch (error) {failure(error);return;}
  }

  ball.addEventListener('pointerdown', (event) => {
    if (event.button !== 0) return;
    start = { x: event.clientX, y: event.clientY };
    dragged = false;
  });
  ball.addEventListener('pointermove', async (event) => {
    if (!start || !(event.buttons & 1) || dragged) return;
    if (Math.hypot(event.clientX - start.x, event.clientY - start.y) < 5) return;
    dragged = true;start = null;
    try {await window.__TAURI__.window.getCurrentWindow().startDragging();}
    catch (error) {dragged = false;start = null;failure(error);return;}
    // Keep the panel closed right after a drag; the ball settles onto a nearby edge.
    suppress = Date.now() + SUPPRESS;
    action('snap');
    setTimeout(() => {
      dragged = false;
    }, SUPPRESS);
  });
  ball.addEventListener('pointerup', () => {start = null;});
  ball.addEventListener('pointercancel', () => {start = null;});
  ball.addEventListener('click', (event) => {
    if (dragged && event.detail !== 0) return;
    dragged = false;
    if (Date.now() < suppress && event.detail !== 0) return;
    setPanel(!panelOpen, false);
  });
  document.addEventListener('contextmenu', (event) => {
    if (event.target?.closest?.('input,textarea,[contenteditable=true]')) return;
    event.preventDefault();action('menu');
  });
  document.addEventListener('pointerup', () => {start = null;});
  document.addEventListener('pointercancel', () => {start = null;dragged = false;});
  ball.addEventListener('keydown', (event) => {
    if (event.shiftKey && event.key === 'F10' || event.key === 'ContextMenu') {event.preventDefault();action('menu');}
  });

  document.addEventListener('keydown', (event) => {
    if (event.defaultPrevented || document.querySelector('#quota-dialog')?.open) return;
    // Escape that cancels an IME candidate list belongs to the input method, not to the panel.
    if (event.isComposing || event.keyCode === 229) return;
    if (event.key === 'Escape' && panelOpen) {
      event.preventDefault();
      const more = document.querySelector('#panel-more');
      if (more?.open) {more.open = false;more.querySelector('summary')?.focus({ preventScroll: true });} else
      if (!document.querySelector('#panel-menu').hidden) closeMenu(true);else
      setPanel(false);
    }
  });
  document.addEventListener('pointerdown', (event) => {
    const menu = document.querySelector('#panel-menu');
    const wrap = event.target instanceof Element ? event.target.closest('.panel-refresh-wrap') : null;
    if (menu && !menu.hidden && !wrap) closeMenu();
    const more = document.querySelector('#panel-more');
    if (more?.open && !more.contains(event.target)) more.open = false;
    // An auto-opened Linux popover is shown without keyboard focus; the first press inside claims it.
    if (info?.platform === 'linux' && panelOpen && orbitAgent && info?.orbitAuto && !orbitFocusAsked &&
      event.target instanceof Element && event.target.closest('#orbit-popover')) {
      orbitFocusAsked = true;
      invoke('desktop_orbit_popover', { focus: true }).catch(() => {});
    }
  });
  window.addEventListener('aieyes:orbit-layout', reportOrbitLayout);
  window.addEventListener('aieyes:orbit-followup', () => {if (panelOpen && orbitAgent && info) renderOrbitPopover(info);});
  window.addEventListener('resize', reportOrbitLayout);

  bindRefreshMenu('panel-refresh', 'panel-menu');
  document.querySelector('#panel-pin')?.addEventListener('click', async () => {
    try {applyPanel(await invoke('desktop_panel_pin', { pinned: !pinned }));} catch (error) {failure(error);}
  });
  document.querySelector('#panel-detail').addEventListener('click', () => {setPanel(false);if (typeof openDetail === 'function') openDetail();else action('open');});
  document.querySelector('#panel-settings').addEventListener('click', () => {setPanel(false);action('settings');});
  document.querySelector('#panel-add-host')?.addEventListener('click', () => openSetup('hosts'));
  document.querySelector('#panel-accounts')?.addEventListener('click', () => {const more = document.querySelector('#panel-more');if (more) more.open = false;window.AieyesApp?.openPanelAccounts();});
  document.querySelector('#panel-quit').addEventListener('click', () => action('quit'));
  document.querySelector('#orbit-open-sessions')?.addEventListener('click', () => action('open-sessions'));
  document.querySelector('#orbit-close')?.addEventListener('click', () => action('close-panel'));
  document.querySelector('#panel-more')?.addEventListener('toggle', (event) => {
    if (event.target.open) closeMenu();
  });

  window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener('change', () => {if (info) update(info, false);});
  function update(next, applyOpen = true) {
    info = next;
    window.AieyesI18n?.setHourCycle(next.hourCycle);
    document.body.dataset.phase = next.phase || 'idle';
    document.body.dataset.active = String(next.activeCount > 0);
    document.body.dataset.platform = next.platform || '';
    document.documentElement.dataset.platform = next.platform || '';
    document.documentElement.dataset.material = next.materials?.floating || 'opaque';
    document.documentElement.dataset.corners = next.windowCorners || 'round';
    window.AieyesTheme?.loadFonts?.();
    document.querySelector('#phase').textContent = next.recovery === 'failed' ? AieyesI18n.t("恢复失败") : next.recovery === 'refreshing' ? AieyesI18n.t("恢复中") : next.unavailable ? AieyesI18n.t("状态不全") : labels[next.phase] || AieyesI18n.t("空闲");
    const count = document.querySelector('#count');
    count.textContent = next.activeCount > 99 ? '99+' : String(next.activeCount);
    count.hidden = !next.activeCount || !next.showCount;
    count.setAttribute('aria-label', AieyesI18n.t("{0} 个活跃会话", next.activeCount));
    if (typeof phaseColor === 'function') document.body.style.setProperty('--phase-color', phaseColor(next.phase || 'idle'));
    ball.title = AieyesI18n.t("Aieyes · {0}\n单击打开面板 · 拖动移动 · 右键菜单", AieyesI18n.message(next.summary));
    ball.setAttribute('aria-label', ball.title);
    window.AieyesPet?.update(next);
    if (typeof renderLiveSessions === 'function') renderLiveSessions(next);
    if (applyOpen && (next.panelOpen !== undefined || next.panelPinned !== undefined || next.orbitAgent !== undefined)) applyPanel(next);
    else if (panelOpen && orbitAgent) renderOrbitPopover(next);
  }
  async function boot() {
    try {
      await window.__TAURI__.event.listen('desktop:status', (event) => update(event.payload));
      await window.__TAURI__.event.listen('desktop:error', (event) => failure(event.payload));
      await window.__TAURI__.event.listen('desktop:panel', (event) => applyPanel(event.payload));
      const initial = await invoke('desktop_info');
      document.body.dataset.nativePet = String(!!initial.nativePet);
      update(initial);
      if (typeof appReady !== 'undefined') await appReady;
      await window.AieyesTheme?.loadFonts?.();
      await document.fonts?.ready;
      await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      await invoke('desktop_panel_ready', { generation: window.AIEYES_SHELL_GENERATION ?? 0 });
    } catch (error) {failure(error);}
  }
  boot();
})();
