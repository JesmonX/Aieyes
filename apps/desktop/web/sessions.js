/* Live session rows, shared by the details window and the floating panel so both
   surfaces stay identical. Fills the static #live-sessions markup of either page. */
(() => {
  // Labels are translated per render so a language switch also reaches the open rows.
  const phases = { idle: "空闲", needsInput: "等待输入", failed: "执行失败", working: "进行中", thinking: "思考中", tool: "执行工具", complete: "已完成", interrupted: "已中断", unknown: "状态待确认" };
  const phaseKey = (key) => Object.hasOwn(phases, key) ? key : 'unknown';
  const phaseLabel = (key) => AieyesI18n.t(phases[phaseKey(key)]);
  const escape = (value) => window.AieyesUI.escapeHTML(value);
  let latest = null;
  // Turn time includes tool and user waits and stops at the terminal event; unknown sessions show none.
  const activePhases = ['working', 'thinking', 'tool', 'needsInput'];
  const elapsed = (s, now = Date.now()) => {
    if (s.phase === 'unknown' || !(s.turnStartedAt > 0)) return null;
    const active = activePhases.includes(s.phase),end = s.turnEndedAt > 0 ? s.turnEndedAt : active ? now : null;
    return end != null && end >= s.turnStartedAt ? (end - s.turnStartedAt) / 1000 : null;
  };
  const elapsedText = (seconds) => {const total = Math.floor(seconds);return total < 3600 ? `${Math.floor(total / 60)}m${String(total % 60).padStart(2, '0')}s` : `${Math.floor(total / 3600)}h${String(Math.floor(total % 3600 / 60)).padStart(2, '0')}m`;};
  const project = (s) => {const name = String(s.workspace ?? '').replace(/[\\/]+$/, '').split(/[\\/]/).pop();return name || null;};
  const updatedTime = (s) => Number.isFinite(s.updatedAt) ? new Date(s.updatedAt * 1000).toLocaleTimeString(AieyesI18n.locale, { hour: '2-digit', minute: '2-digit', ...AieyesI18n.clock }) : '—';
  const updatedText = (s) => AieyesI18n.t("最后更新 {0}", updatedTime(s));
  const identity = (s) => typeof s.nativeSession === 'string' && s.nativeSession ? s.nativeSession.slice(-8) : null;
  const rateText = (n) => Number.isFinite(n) && n > 0 ? (n < 1 ? '<1' : Math.round(n)) + ' tok/s' : null;
  // Unknown parts are omitted, never shown as zero.
  const usageText = (s) => {
    const u = s.usage;if (!u) return '';
    const format = (n) => typeof compact === 'function' ? compact(n) : String(n);
    const rate = rateText(u.outputTokensPerSecond),measured = rateText(u.measuredOutputTokensPerSecond);
    return [AieyesI18n.t("会话 {0}", format(u.sessionTokens)), u.turnTokens != null ? AieyesI18n.t("本轮 {0} Token", format(u.turnTokens)) : null, rate || (measured ? AieyesI18n.t("部分 {0}", measured) : null)].filter(Boolean).join(' · ');
  };
  const usageHelp = (s) => {
    const u = s.usage;if (!u) return AieyesI18n.t("尚未读取到此会话的 Token 记录；不同 Agent 的记录能力和写入时间可能不同");
    const exact = (n) => new Intl.NumberFormat(AieyesI18n.locale, {maximumFractionDigits: 0}).format(n);
    const components = (c) => AieyesI18n.t("输入 {0} · 输出 {1} · 缓存读取 {2} · 缓存写入 {3}", exact(c.input), exact(c.output), exact(c.cacheRead), exact(c.cacheWrite));
    const lines = [AieyesI18n.t("会话累计 {0} Token", exact(u.sessionTokens))];
    if (u.session) lines.push(components(u.session));
    if (u.turnTokens != null) {lines.push(AieyesI18n.t("本轮累计 {0} Token", exact(u.turnTokens)));if (u.turn) lines.push(components(u.turn));}
    if (s.subagentCount > 0) lines.push(AieyesI18n.t("含 {0} 个子 Agent；本轮包含子 Agent 在此轮期间的用量", s.subagentCount));
    lines.push(AieyesI18n.t("输出 Token ÷ 模型请求耗时（含首字等待，不含工具执行和用户等待）"));
    if (rateText(u.measuredOutputTokensPerSecond) && u.turnOutputTokens > 0 && Number.isFinite(u.timedOutputTokens)) lines.push(AieyesI18n.t("部分测速：覆盖本轮 {0}% 的输出 Token；其余输出缺少完整的请求计时", exact(Math.min(100, u.timedOutputTokens / u.turnOutputTokens * 100))));
    else if (!rateText(u.outputTokensPerSecond)) lines.push(AieyesI18n.t("此轮尚无完整的请求计时，暂不显示整轮速度"));
    if (Number.isFinite(u.updatedAtMs)) lines.push(AieyesI18n.t("用量记录时间：{0}", new Date(u.updatedAtMs).toLocaleString(AieyesI18n.locale, AieyesI18n.clock)));
    return lines.join('\n');
  };
  const usageHTML = (s) => `<small class="${s.usage ? 'session-usage' : 'session-usage-unavailable'}" title="${escape(usageHelp(s))}">${escape(usageText(s) || AieyesI18n.t("Token 暂未记录"))}</small>`;
  const detailText = (s, now) => {const time = elapsed(s, now);return [project(s), identity(s) ? '#' + identity(s) : null, time == null ? null : AieyesI18n.t("本轮 {0}", elapsedText(time))].filter(Boolean).concat(time == null ? [updatedText(s)] : []).join(' · ');};
  const threadLink = (s) => s.machineId === 'local' && (s.provider === 'chatgpt-work' || s.provider === 'codex' && ['codex_desktop', 'codex_work_desktop'].includes(s.client)) && typeof s.nativeSession === 'string' && /^[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}$/i.test(s.nativeSession) ? 'codex://threads/' + s.nativeSession : null;
  const canFocus = (s) => s.machineId === 'local' && (Number.isInteger(s.pid) && s.pid > 0 || !!threadLink(s));
  const focusHelp = (s) => threadLink(s) ? AieyesI18n.t("打开此会话") : AieyesI18n.t("切换到会话所在的终端或 IDE");
  // Only running turns tick, and only while the list is visible.
  // A one-second timer runs only while a rendered turn is still counting.
  let tickTimer = 0;
  const tick = () => {
    tickTimer = 0;
    const list = document.querySelector('#session-list'),rows = list ? [...list.querySelectorAll('[data-turn-ticking]')] : [];
    if (!rows.length) return;
    if (!document.hidden && list.closest('details')?.open && !list.closest('[hidden]')) {
      for (const el of rows) {const s = latest?.sessions?.find((row) => row.id === el.dataset.turnTicking);if (s) el.textContent = detailText(s);}
    }
    scheduleTick();
  };
  const scheduleTick = () => {if (!tickTimer && document.querySelector('#session-list [data-turn-ticking]')) tickTimer = setTimeout(tick, 1000);};
  const focusMessages = { session: "已打开 {0} 中的会话", pane: "已定位到 {0} 的分屏", tab: "已定位到 {0} 的标签页", window: "已切换到 {0} 的窗口", application: "已激活 {0}，未定位到具体窗口", requested: "已请求切换到 {0}，尚未确认前台窗口", noWindow: "未找到所属窗口（可能运行在 tmux、screen 或后台服务中）", exited: "会话进程已退出", unsupported: "当前桌面不支持切换窗口", failed: "无法切换到 {0}" };
  let focusing = false;
  async function focusSession(id) {
    const s = latest?.sessions?.find((row) => row.id === id);if (!s || !canFocus(s) || focusing) return;
    focusing = true;
    let result;
    try {result = await window.__TAURI__.core.invoke('focus_session', {pid: s.pid ?? null, processStart: s.processStart ?? 0, provider: s.provider ?? '', nativeSession: s.nativeSession ?? '', client: s.client ?? '', workspace: s.workspace ?? '', machineId: s.machineId});} catch (error) {result = { outcome: 'failed', name: '' };} finally {focusing = false;}
    const message = focusMessages[result?.outcome] ?? focusMessages.failed;
    if (typeof notify === 'function') notify(AieyesI18n.t(message, result?.name || AieyesI18n.t("会话所在应用")), ['session', 'pane', 'tab', 'window', 'application', 'requested'].includes(result?.outcome) ? 'info' : 'error');
  }
  document.addEventListener('click', (event) => {const row = event.target instanceof Element ? event.target.closest('[data-focus-session]') : null;if (row) void focusSession(row.dataset.focusSession);});
  document.addEventListener('keydown', (event) => {const row = event.target instanceof Element ? event.target.closest('[data-focus-session]') : null;if (row && (event.key === 'Enter' || event.key === ' ')) {event.preventDefault();void focusSession(row.dataset.focusSession);}});
  globalThis.AieyesSessions = { visibility() {if (latest) renderLiveSessions(latest);}, tick, elapsed, elapsedText, detailText, usageText, usageHelp, usageHTML, identity, canFocus, focusHelp, threadLink, focusSession, phaseLabel, phaseKey, updatedText, updatedTime, active: (phase) => activePhases.includes(phase), snapshot: () => latest };
  globalThis.renderLiveSessions = (info) => {
    latest = info;
    // The sessions page draws the same snapshot in full.
    window.AieyesSessionsPage?.update();
    const rank = { needsInput: 0, failed: 1, unknown: 2, working: 3, thinking: 4, tool: 5, complete: 6, interrupted: 7, idle: 8 };
    info = { ...info, sessions: (info.sessions ?? []).filter((s) => !state.provider || AieyesUI.sameAgent(s.provider, state.provider)).sort((a, b) => (rank[a.phase] ?? 2) - (rank[b.phase] ?? 2) || b.updatedAt - a.updatedAt) };
    const container = document.querySelector('#live-sessions');

    const summary = document.querySelector('#session-summary');
    const list = document.querySelector('#session-list');
    const warning = document.querySelector('#session-warning');
    if (!summary || !list || !warning) return;
    const active = info.sessions.filter((s) => s.phase !== 'idle');
    const failures = (info.sources || []).filter((s) => s.state !== 'healthy' && (!state.provider || AieyesUI.sameAgent(s.provider, state.provider)));
    const processSources = [...new Map((info.sources || []).filter((s) => typeof s.processRunning === 'boolean' && (!state.provider || AieyesUI.sameAgent(s.provider, state.provider))).map((s) => [s.provider + ':' + s.machineId, s])).values()];
    summary.textContent = active.length ? AieyesI18n.t("{0} 个会话", active.length) : info.unavailable ? AieyesI18n.t("会话状态待确认") : processSources.length ? AieyesI18n.t("IDE 运行状态") : AieyesI18n.t("暂无活跃会话");
    let counts = container.querySelector('.session-counts');if (!counts) {counts = document.createElement('span');counts.className = 'session-counts';container.querySelector('summary > .muted')?.remove();container.querySelector('summary').append(counts);}
    counts.innerHTML = Object.keys(phases).filter((key) => active.some((s) => s.phase === key)).map((key) => `<span data-phase="${key}" data-attention="${['needsInput', 'failed'].includes(key)}"><i class="phase-dot" aria-hidden="true"></i>${phaseLabel(key)} ${active.filter((s) => s.phase === key).length}</span>`).join('');
    if (!container.dataset.persisted) {container.dataset.persisted = 'true';const key = 'aieyes.sessions.' + (PANEL ? 'panel' : 'overview');try {container.open = localStorage.getItem(key) === 'true';} catch (_) {}container.addEventListener('toggle', () => {try {localStorage.setItem(key, String(container.open));} catch (_) {}});}

    const indicator = document.querySelector('#session-indicator');
    if (indicator) {const phase = info.unavailable || failures.length ? 'unknown' : active[0]?.phase ?? 'idle';indicator.dataset.phase = phase;indicator.innerHTML = uiIcon(phaseKey(phase));}

    const rows = active.map((s) => {
      const symbol = uiIcon(phaseKey(s.phase)),label = phaseLabel(s.phase);
      // Local sessions open the terminal or IDE that runs them; SSH sessions are information only.
      const focus = canFocus(s) ? ` role="button" tabindex="0" data-focus-session="${escape(s.id)}" title="${escape(focusHelp(s))}"` : '';
      return `<div class="live-row" data-phase="${escape(s.phase)}"${focus}><span class="phase-mark" data-phase="${escape(s.phase)}" aria-hidden="true">${symbol}</span><div><strong>${escape((window.AieyesContract.providers[s.provider] || s.source || 'Agent') + ' · ' + (s.machine || AieyesI18n.t("本机")))}</strong><small class="session-detail" title="${escape([s.workspace, s.nativeSession, updatedText(s)].filter(Boolean).join('\n'))}"${elapsed(s) != null && !(s.turnEndedAt > 0) ? ` data-turn-ticking="${escape(s.id)}"` : ''}>${escape(detailText(s))}</small>${usageHTML(s)}</div><span>${label}</span>${canFocus(s) ? `<span class="session-jump-mark" aria-hidden="true">${uiIcon('expand')}</span>` : ''}</div>`;
    }).join('') + processSources.map((s) => {
      const provider = window.AieyesContract.providers[s.provider] ?? s.provider;
      const machine = s.machineId === 'local' ? AieyesI18n.t("本机") : state.settings?.hosts?.find((h) => h.id === s.machineId)?.name || s.machineId;
      return `<div class="live-row" data-process-provider="${escape(s.provider)}" data-process-running="${s.processRunning}"><span class="phase-mark" aria-hidden="true">${uiIcon('idle')}</span><div><strong>${escape(provider + ' · ' + machine)}</strong><small>${AieyesI18n.t("对话存储已加密，会话数量未知")}</small></div><span>${AieyesI18n.t(s.processRunning ? "IDE 正在运行" : "IDE 未运行")}</span></div>`;
    }).join('') || `<p class="muted">${AieyesI18n.t("暂无活跃会话")}</p>`;
    if (list._rows !== rows) {list.innerHTML = rows;list._rows = rows;}
    scheduleTick();
    container.hidden = state.page !== 'agent' || !active.length && !processSources.length && !failures.length && !info.unavailable;
    if (failures.length) summary.textContent = AieyesI18n.t("{0} · {1} 个来源异常", summary.textContent, failures.length);
    warning.hidden = !failures.length;
    warning.textContent = failures.map((s) => `${window.AieyesContract.providers[s.provider] ?? s.provider} · ${s.machineId === 'local' ? AieyesI18n.t("本机") : state.settings?.hosts?.find((h) => h.id === s.machineId)?.name || s.machineId}: ${AieyesI18n.message(s.error) || AieyesI18n.t("正在连接")}`).join('; ');
    let notice = document.querySelector('#session-attention');
    const uncertain = info.unavailable || failures.length || info.sessions.some((s) => s.phase === 'unknown');
    if (uncertain) {
      if (!notice) {notice = document.createElement('div');notice.id = 'session-attention';document.querySelector('#live-sessions').after(notice);}
      notice.hidden = state.page !== 'agent';notice.className = 'attention';notice.innerHTML = uiIcon('attention') + `<span>${AieyesI18n.t("待确认：会话状态可能不完整")}</span><button type="button">${AieyesI18n.t("重试读取")}</button>`;
      notice.querySelector('button').onclick = async () => {const button = notice.querySelector('button');button.disabled = true;try {renderLiveSessions(await window.__TAURI__.core.invoke('desktop_info'));} catch (error) {button.disabled = false;notice.querySelector('span').textContent = AieyesI18n.message(String(error));}};
    } else notice?.remove();
  };
})();
