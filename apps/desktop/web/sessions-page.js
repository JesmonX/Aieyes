/* Sessions detail page: what is running now and what ended recently, drawn from the
   desktop:status snapshot that also fills the overview strip. Lifecycle metadata only —
   prompts, tool arguments and replies are never read or shown. */
(() => {
  const storeKey = 'aieyes.sessions.provider';
  let selection = 'all',root = null,tickTimer = 0;
  try {selection = localStorage.getItem(storeKey) || 'all';} catch (_) {}
  const escape = (value) => AieyesUI.escapeHTML(value);
  const providerName = (provider) => window.AieyesContract.providers[provider] ?? provider;
  const machineName = (id) => id === 'local' ? AieyesI18n.t("本机") : state.settings?.hosts?.find((h) => h.id === id)?.name || id;
  const project = (s) => {const name = String(s.workspace ?? '').replace(/[\\/]+$/, '').split(/[\\/]/).pop();return name || null;};
  const active = (phase) => AieyesSessions.active(phase);
  // Waiting sessions lead; the rest keep a stable order within a turn so rows do not jump every second.
  const rank = (phase) => phase === 'needsInput' ? 0 : phase === 'unknown' ? 2 : 1;
  const byId = (a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  // "本轮 3m12s", or the last update time when no turn timing is known.
  const timing = (s, now) => {
    const seconds = AieyesSessions.elapsed(s, now);
    if (seconds == null) return AieyesSessions.updatedText(s);
    const turn = AieyesI18n.t("本轮 {0}", AieyesSessions.elapsedText(seconds));
    return active(s.phase) ? turn : `${turn} · ${AieyesSessions.updatedTime(s)}`;
  };
  const patch = (element, html) => {if (element._html !== html) {element._html = html;element.innerHTML = html;}};
  function rowHTML(s, now) {
    // SSH sessions are information only: no local window owns them.
    const focusable = AieyesSessions.canFocus(s),focus = AieyesSessions.focusHelp(s),identity = AieyesSessions.identity(s);
    const ticking = AieyesSessions.elapsed(s, now) != null && !(s.turnEndedAt > 0) && active(s.phase);
    return `<div class="live-row session-row" data-phase="${escape(s.phase)}"><span class="phase-mark" data-phase="${escape(s.phase)}" aria-hidden="true">${uiIcon(AieyesSessions.phaseKey(s.phase))}</span><div class="session-info"><strong title="${escape(s.workspace ?? '')}" data-user-text>${escape(project(s) ?? providerName(s.provider))}</strong><small>${escape(providerName(s.provider))} · <span data-user-text>${escape(s.machine || machineName(s.machineId))}</span>${identity ? ` · <span title="${escape(s.nativeSession)}" data-user-text>#${escape(identity)}</span>` : ''}</small>${AieyesSessions.usageHTML(s)}</div><div class="session-state"><span class="session-phase-label" data-phase="${escape(s.phase)}">${escape(AieyesSessions.phaseLabel(s.phase))}</span><small class="session-timing" data-session-timing="${escape(s.id)}"${ticking ? ' data-ticking' : ''}></small></div>${focusable ? `<button type="button" class="session-focus" data-focus-session="${escape(s.id)}" title="${escape(focus)}" aria-label="${escape(focus)}">${uiIcon('expand')}</button>` : ''}</div>`;
  }
  const listHTML = (key, title, rows, empty, now) => `<section class="card session-list" data-session-list="${key}"><h2>${escape(title)}${rows.length ? `<span class="count">${rows.length}</span>` : ''}</h2>${rows.length ? rows.map((s) => rowHTML(s, now)).join('') : `<p class="muted">${escape(empty)}</p>`}</section>`;
  const tileHTML = (tone, icon, label, value) => `<div class="stat" data-session-tile="${tone}"><div class="stat-label"><span>${escape(label)}</span><b>${uiIcon(icon)}</b></div><div class="stat-value">${value}</div></div>`;
  function fillTimings(now = Date.now()) {
    const sessions = AieyesSessions.snapshot()?.sessions ?? [];
    for (const el of root?.querySelectorAll('[data-session-timing]') ?? []) {
      const s = sessions.find((row) => row.id === el.dataset.sessionTiming),text = s ? timing(s, now) : '';
      if (el.textContent !== text) el.textContent = text;
    }
  }
  // A one-second timer runs only while a rendered turn is still counting.
  function tick() {
    tickTimer = 0;
    if (state.page !== 'sessions' || !root?.isConnected) return;
    if (!document.hidden) fillTimings();
    scheduleTick();
  }
  const scheduleTick = () => {if (!tickTimer && root?.querySelector('[data-ticking]')) tickTimer = setTimeout(tick, 1000);};
  function render() {
    const content = document.querySelector('#content');if (!content) return;
    if (!root?.isConnected) {
      root = document.createElement('div');root.className = 'session-page';
      root.innerHTML = '<div class="agent-selector session-selector" role="group" data-session-filters hidden></div><div data-session-body></div>';
      content.replaceChildren(root);
      root.firstElementChild.onclick = (event) => {
        const button = event.target.closest('[data-session-provider]');if (!button) return;
        selection = button.dataset.sessionProvider;try {localStorage.setItem(storeKey, selection);} catch (_) {}
        update();root.querySelector(`[data-session-provider="${CSS.escape(selection)}"]`)?.focus({ preventScroll: true });
      };
      root.lastElementChild.onclick = (event) => {if (event.target.closest('[data-session-connect]')) void openSettings({ category: 'integrations' });};
    }
    update();
    window.AieyesMotion?.didRender('sessions');
  }
  function update() {
    if (state.page !== 'sessions' || !root?.isConnected) return;
    const info = AieyesSessions.snapshot() ?? {},now = Date.now();
    const sessions = (info.sessions ?? []).filter((s) => s.phase !== 'idle');
    const providers = [...new Set(sessions.map((s) => AieyesUI.agentProvider(s.provider)))].sort();
    // A remembered filter for an agent with no sessions right now falls back to everything.
    const chosen = selection === 'all' || providers.includes(selection) ? selection : 'all';
    const matches = (provider) => chosen === 'all' || AieyesUI.sameAgent(provider, chosen);
    const visible = sessions.filter((s) => matches(s.provider));
    const running = visible.filter((s) => active(s.phase) || s.phase === 'unknown').sort((a, b) => rank(a.phase) - rank(b.phase) || (b.turnStartedAt ?? 0) - (a.turnStartedAt ?? 0) || byId(a, b));
    const ended = visible.filter((s) => !active(s.phase) && s.phase !== 'unknown').sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || byId(a, b));
    const failures = (info.sources ?? []).filter((s) => s.state !== 'healthy' && matches(s.provider));
    const processes = [...new Map((info.sources ?? []).filter((s) => typeof s.processRunning === 'boolean' && matches(s.provider)).map((s) => [s.provider + ':' + s.machineId, s])).values()];
    const filters = root.querySelector('[data-session-filters]'),chip = (key, label) => `<button type="button" data-session-provider="${escape(key)}" aria-pressed="${chosen === key}"${key === 'all' ? '' : ` style="--category:var(--category-${escape(key)},var(--accent))"`}>${escape(label)}</button>`;
    filters.hidden = providers.length < 2;filters.setAttribute('aria-label', AieyesI18n.t("选择 Agent"));
    patch(filters, providers.length < 2 ? '' : `<div class="tab-scroller">${chip('all', AieyesI18n.t("全部"))}${providers.map((key) => chip(key, providerName(key))).join('')}</div>`);
    let body;
    if (info.unavailable) body = `<div class="card session-unavailable" role="status">${uiIcon('warning')}<span>${AieyesI18n.t("会话读取失败")}</span></div>`;
    else if (!visible.length && !failures.length && !processes.length) body = `<div class="card">${AieyesUI.emptyStateHTML({ icon: 'terminal', title: AieyesI18n.t("暂无会话动态"), message: AieyesI18n.t("在终端或 IDE 中启动 Agent 后，会话会显示在这里"), action: state.settings?.sources?.length ? '' : `<button type="button" class="primary" data-session-connect>${AieyesI18n.t("接入账户")}</button>` })}</div>`;
    else body = (visible.length ? `<div class="stats session-stats">${tileHTML('running', 'working', AieyesI18n.t("进行中"), running.filter((s) => active(s.phase) && s.phase !== 'needsInput').length)}${tileHTML('waiting', 'needsInput', AieyesI18n.t("等待输入"), running.filter((s) => s.phase === 'needsInput').length)}${tileHTML('ended', 'complete', AieyesI18n.t("最近结束"), ended.length)}</div>` : '') + listHTML('running', AieyesI18n.t("进行中"), running, AieyesI18n.t("当前没有进行中的会话"), now) + listHTML('ended', AieyesI18n.t("最近结束"), ended, AieyesI18n.t("暂无最近结束的会话"), now);
    if (processes.length) body += `<section class="card session-list" data-session-list="processes"><h2>${AieyesI18n.t("IDE 运行状态")}</h2>${processes.map((s) => `<div class="live-row" data-process-provider="${escape(s.provider)}" data-process-running="${s.processRunning}"><span class="phase-mark" aria-hidden="true">${uiIcon('computer')}</span><div><strong>${escape(providerName(s.provider))} · <span data-user-text>${escape(machineName(s.machineId))}</span></strong><small>${AieyesI18n.t("对话存储已加密，会话数量未知")}</small></div><span class="muted">${AieyesI18n.t(s.processRunning ? "IDE 正在运行" : "IDE 未运行")}</span></div>`).join('')}</section>`;
    if (failures.length) body += `<section class="card session-list session-failures" data-session-list="failures"><h2>${AieyesI18n.t("{0} 个来源异常", failures.length)}</h2>${failures.map((s) => `<p class="session-failure">${uiIcon('warning')}<span>${escape(providerName(s.provider))} · <span data-user-text>${escape(machineName(s.machineId))}</span>: ${escape(AieyesI18n.message(s.error) || AieyesI18n.t("正在连接"))}</span></p>`).join('')}</section>`;
    patch(root.querySelector('[data-session-body]'), body);
    fillTimings(now);scheduleTick();
  }
  window.AieyesSessionsPage = { render, update };
})();
