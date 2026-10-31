/* Live session rows, shared by the details window and the floating panel so both
   surfaces stay identical. Fills the static #live-sessions markup of either page. */
(() => {
  // Labels are translated per render so a language switch also reaches the open rows.
  const phases = { idle: "空闲", needsInput: "等待输入", failed: "执行失败", working: "进行中", thinking: "思考中", tool: "执行工具", complete: "已完成", interrupted: "已中断", unknown: "状态待确认" };
  const phaseKey = (key) => Object.hasOwn(phases, key) ? key : 'unknown';
  const phaseLabel = (key) => AieyesI18n.t(phases[phaseKey(key)]);
  const escape = (value) => window.AieyesUI.escapeHTML(value);
  let latest = null;
  globalThis.AieyesSessions = { visibility() {if (latest) renderLiveSessions(latest);} };
  globalThis.renderLiveSessions = (info) => {
    latest = info;
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
    counts.innerHTML = Object.keys(phases).filter((key) => active.some((s) => s.phase === key)).map((key) => `<span data-phase="${key}" data-attention="${['needsInput', 'failed'].includes(key)}">${phaseLabel(key)} ${active.filter((s) => s.phase === key).length}</span>`).join('');
    if (!container.dataset.persisted) {container.dataset.persisted = 'true';const key = 'aieyes.sessions.' + (PANEL ? 'panel' : 'overview');try {container.open = localStorage.getItem(key) === 'true';} catch (_) {}container.addEventListener('toggle', () => {try {localStorage.setItem(key, String(container.open));} catch (_) {}});}

    const indicator = document.querySelector('#session-indicator');
    if (indicator) {const phase = info.unavailable || failures.length ? 'unknown' : active[0]?.phase ?? 'idle';indicator.dataset.phase = phase;indicator.innerHTML = uiIcon(phaseKey(phase));}

    const rows = active.map((s) => {
      const symbol = uiIcon(phaseKey(s.phase)),label = phaseLabel(s.phase);
      return `<div class="live-row" data-phase="${escape(s.phase)}"><span class="phase-mark" data-phase="${escape(s.phase)}" aria-hidden="true">${symbol}</span><div><strong>${escape(({ codex: 'Codex', claude: 'Claude Code', antigravity: 'Antigravity' }[s.provider] || s.source || 'Agent') + ' · ' + (s.machine || AieyesI18n.t("本机")))}</strong><small>${AieyesI18n.t("最后更新 {0}", Number.isFinite(s.updatedAt) ? escape(new Date(s.updatedAt * 1000).toLocaleTimeString(AieyesI18n.locale, { hour: '2-digit', minute: '2-digit', ...AieyesI18n.clock })) : '—')}</small></div><span>${label}</span></div>`;
    }).join('') + processSources.map((s) => {
      const provider = { trae: 'Trae', 'trae-cn': 'Trae CN' }[s.provider] || s.provider;
      const machine = s.machineId === 'local' ? AieyesI18n.t("本机") : state.settings?.hosts?.find((h) => h.id === s.machineId)?.name || s.machineId;
      return `<div class="live-row" data-process-provider="${escape(s.provider)}" data-process-running="${s.processRunning}"><span class="phase-mark" aria-hidden="true">${uiIcon('idle')}</span><div><strong>${escape(provider + ' · ' + machine)}</strong><small>${AieyesI18n.t("对话存储已加密，会话数量未知")}</small></div><span>${AieyesI18n.t(s.processRunning ? "IDE 正在运行" : "IDE 未运行")}</span></div>`;
    }).join('') || `<p class="muted">${AieyesI18n.t("暂无活跃会话")}</p>`;
    if (list._rows !== rows) {list.innerHTML = rows;list._rows = rows;}
    container.hidden = state.page !== 'agent' || !active.length && !processSources.length && !failures.length && !info.unavailable;
    if (failures.length) summary.textContent = AieyesI18n.t("{0} · {1} 个来源异常", summary.textContent, failures.length);
    warning.hidden = !failures.length;
    warning.textContent = failures.map((s) => `${s.provider} · ${s.machineId === 'local' ? AieyesI18n.t("本机") : s.machineId}: ${AieyesI18n.message(s.error) || AieyesI18n.t("正在连接")}`).join('; ');
    let notice = document.querySelector('#session-attention');
    const uncertain = info.unavailable || failures.length || info.sessions.some((s) => s.phase === 'unknown');
    if (uncertain) {
      if (!notice) {notice = document.createElement('div');notice.id = 'session-attention';document.querySelector('#live-sessions').after(notice);}
      notice.hidden = state.page !== 'agent';notice.className = 'attention';notice.innerHTML = uiIcon('attention') + `<span>${AieyesI18n.t("待确认：会话状态可能不完整")}</span><button type="button">${AieyesI18n.t("重试读取")}</button>`;
      notice.querySelector('button').onclick = async () => {const button = notice.querySelector('button');button.disabled = true;try {renderLiveSessions(await window.__TAURI__.core.invoke('desktop_info'));} catch (error) {button.disabled = false;notice.querySelector('span').textContent = AieyesI18n.message(String(error));}};
    } else notice?.remove();
  };
})();
