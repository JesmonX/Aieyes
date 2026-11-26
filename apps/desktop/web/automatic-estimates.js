// Source-scoped consent; switches always reflect the last saved configuration.
window.AieyesAutomatic = (() => {
  let consent = AieyesI18n.t("我确认影响订阅额度的所有设备用量均已纳入；按日志识别 API，不确定的区间跳过。");window.addEventListener("aieyes:language", () => {consent = AieyesI18n.t("我确认影响订阅额度的所有设备用量均已纳入；按日志识别 API，不确定的区间跳过。");});
  let labels = { collecting: AieyesI18n.t("积累样本中"), sampling: AieyesI18n.t("自动采样中"), waiting: AieyesI18n.t("等待有效样本"), waitingIdentity: AieyesI18n.t("等待订阅登录"), waitingCoverage: AieyesI18n.t("等待完整目录用量"), waitingManual: AieyesI18n.t("等待手动采样结束"), disabled: AieyesI18n.t("已关闭") };window.addEventListener("aieyes:language", () => {labels = { collecting: AieyesI18n.t("积累样本中"), sampling: AieyesI18n.t("自动采样中"), waiting: AieyesI18n.t("等待有效样本"), waitingIdentity: AieyesI18n.t("等待订阅登录"), waitingCoverage: AieyesI18n.t("等待完整目录用量"), waitingManual: AieyesI18n.t("等待手动采样结束"), disabled: AieyesI18n.t("已关闭") };});
  const status = (s) => state.dashboard?.sources?.find((row) => row.id === s.id)?.autoValuationStatus;
  const description = (s) => {const row = status(s);return !AieyesUI.sourceEnabled(state.settings, s) ? AieyesI18n.t("采集已暂停") : !s.autoValuation?.enabled ? AieyesI18n.t("已关闭") : (labels[row?.state] ?? AieyesI18n.t("等待首个实时快照")) + (row?.note ? ' · ' + row.note : '');};
  const intro = (provider) => provider === 'codex' ? AieyesI18n.t("估算 5 小时 / 7 天额度及 credits 的 API 等价价值。已暂停的机器不参与实时估值；暂停期间请勿在这些机器上使用同一账户。") : AieyesI18n.t("估算 5 小时 / 7 天额度的 API 等价价值");
  let saving = false;
  function html(sources) {return sources.filter((s) => AieyesUI.supports(s.provider, 'valuation', state.settings)).map((s) => `<section class="sampling-row" data-auto-row="${escapeHTML(s.id)}"><div class="between"><h4>${escapeHTML(s.name)}</h4><input type="checkbox" role="switch" data-auto-save="${escapeHTML(s.id)}" aria-label="${AieyesI18n.t("自动估值 · {0}", escapeHTML(s.name))}" ${s.autoValuation?.enabled ? 'checked' : ''} ${saving ? 'disabled' : ''}></div><small>${escapeHTML(s.path)}</small><p class="tiny muted" data-auto-status="${escapeHTML(s.id)}">${escapeHTML(description(s))}</p><div class="error" data-auto-error hidden></div></section>`).join('');}
  function summary(sources) {const enabled = sources.filter((s) => s.autoValuation?.enabled),active = enabled.filter((s) => AieyesUI.sourceEnabled(state.settings, s));return !sources.length ? AieyesI18n.t("未配置目录") : !sources.some((s) => AieyesUI.sourceEnabled(state.settings, s)) ? AieyesI18n.t("已暂停") : !enabled.length ? AieyesI18n.t("已关闭") : !active.length ? AieyesI18n.t("已暂停") : AieyesI18n.t("已开启 ") + active.length + '/' + sources.length;}
  function refresh() {
    document.querySelectorAll('[data-auto-status]').forEach((el) => {const s = state.settings?.sources?.find((s) => s.id === el.dataset.autoStatus);if (s) el.textContent = description(s);});
    document.querySelectorAll('[data-auto-save]').forEach((el) => {const source = state.settings?.sources?.find((s) => s.id === el.dataset.autoSave);el.checked = source?.autoValuation?.enabled === true;el.disabled = saving;});
    document.querySelectorAll('[data-auto-machine-status]').forEach((el) => {el.textContent = AieyesI18n.t("估值 ") + summary((state.settings?.sources ?? []).filter((s) => s.provider === el.dataset.autoProvider && (s.hostId || 'local') === el.dataset.autoMachineStatus));});
  }
  function confirm() {return new Promise((resolve) => {
      const dialog = document.createElement('dialog');dialog.id = 'auto-consent';dialog.setAttribute('aria-label', AieyesI18n.t("开启自动估值"));
      dialog.innerHTML = `<h2>${AieyesI18n.t("开启自动估值？")}</h2><p>${consent}</p><div class="actions dialog-actions"><button data-cancel>${AieyesI18n.t("取消")}</button><button data-confirm class="primary">${AieyesI18n.t("确认开启")}</button></div>`;
      const finish = (value) => {dialog.close();dialog.remove();resolve(value);};
      dialog.oncancel = (event) => {event.preventDefault();finish(false);};dialog.querySelector('[data-cancel]').onclick = () => finish(false);dialog.querySelector('[data-confirm]').onclick = () => finish(true);
      document.body.append(dialog);dialog.showModal();
    });}
  async function toggle(input) {
    const source = state.settings.sources.find((s) => s.id === input.dataset.autoSave);if (!source) return;
    const enabled = input.checked;input.checked = source.autoValuation?.enabled === true;
    if (saving) return;
    if (enabled && !(await confirm())) return;
    saving = true;refresh();
    const row = input.closest('[data-auto-row]'),error = row?.querySelector('[data-auto-error]'),dialog = input.closest('dialog');if (error) error.hidden = true;
    if (dialog) dialog.dataset.busy = 'true';
    try {
      const result = await api('autoEstimates.configure', { sourceId: source.id, enabled, confirmed: enabled, baseConfig: source.autoValuation ?? null });
      window.AieyesAgentSettings.accept(result.settings);markSaved();await loadDashboard();
    } catch (cause) {if (error) {AieyesUI.showError(error, cause.message ?? String(cause), AieyesI18n.t("估值设置失败"));error.hidden = false;} else notify(String(cause), 'error');} finally
    {saving = false;if (dialog) delete dialog.dataset.busy;refresh();}
  }
  function open(ids, returnProvider = document.querySelector('#quota-dialog')?.dataset.valuationProvider) {
    const sources = state.settings.sources.filter((s) => ids.includes(s.id) && AieyesUI.supports(s.provider, 'valuation', state.settings));
    const dialog = quotaDialog(AieyesI18n.t("数据源自动估值"), '<p class="muted">' + intro(sources.some((s) => s.provider === 'codex') ? 'codex' : '') + '</p>' + html(sources), 'automatic');
    dialog.dataset.autoSources = JSON.stringify(ids);
    if (returnProvider) {dialog.dataset.valuationProvider = returnProvider;const back = document.createElement('button');back.type = 'button';back.textContent = AieyesI18n.t("返回估值与采样");back.onclick = () => openProviderEstimates(returnProvider);dialog.querySelector('.quota-dialog-body').prepend(back);}
  }
  document.addEventListener('change', (event) => {if (event.target.matches('[data-auto-save]')) void toggle(event.target);});
  document.addEventListener('click', (event) => {const button = event.target.closest('[data-auto-source]');if (button) open([button.dataset.autoSource]);});
  return { html, open, refresh, summary, intro };
})();

const estimateHistorySelection = new Map();
async function loadEstimateHistory(slot, accountKey, credits, group) {
  const dialog = slot.closest('dialog');
  const active = () => slot.isConnected && dialog.open;
  const selectionKey = JSON.stringify([accountKey, credits, group]);
  const request = crypto.randomUUID();
  slot.dataset.historyRequest = request;
  const state = slot.querySelector('[data-history-state]');
  const rollingChart = slot.querySelector('[data-history-chart="rolling"]');
  state.textContent = AieyesI18n.t("正在读取估值历史…");
  rollingChart.replaceChildren();
  try {
    const rows = await api('estimates.history', { accountKey, includeRolling: true, includeDaily: false });
    if (!active() || slot.dataset.historyRequest !== request) return;
    const aliases = {
      'gemini models': 'gemini', 'gemini 4 argon': 'gemini-4-argon',
      'claude and gpt models': 'claude-gpt', 'claudes & gpts': 'claude-gpt',
    };
    const normalizedGroup = aliases[group.toLowerCase()] ?? group.toLowerCase();
    const series = (Array.isArray(rows) ? rows : []).filter((row) =>
      row.samplingMode === 'rolling' && (row.period === 'credits') === credits && row.points?.length
      && (!normalizedGroup || row.group === normalizedGroup));
    state.textContent = series.length ? '' : AieyesI18n.t("暂无可绘制的估值历史");
    if (series.length) renderEstimateHistory(rollingChart, series, selectionKey);
  } catch (error) {
    if (!active() || slot.dataset.historyRequest !== request) return;
    state.textContent = AieyesI18n.t("估值记录读取失败");
    const retry = document.createElement('button');
    retry.type = 'button';
    retry.textContent = AieyesI18n.t("重试读取");
    retry.onclick = () => loadEstimateHistory(slot, accountKey, credits, group);
    state.append(' ', retry);
  }
}

function renderEstimateHistory(container, series, selectionKey) {
  const identity = (row) => JSON.stringify([row.samplingMode, row.period, row.group, row.plan]);
  const chooseLabel = (row) => [
    { fiveHour: '5h', weekly: AieyesI18n.t("7d 直接估值"), weeklyRatio: AieyesI18n.t("7d 容量推算"), credits: '1000 credits' }[row.period],
    row.group, row.plan,
  ].filter(Boolean).join(' · ');
  const saved = estimateHistorySelection.get(selectionKey);
  const picked = series.findIndex(row => identity(row) === saved?.series);
  const newest = series.reduce((best, row, i) => row.points.at(-1).day > series[best].points.at(-1).day ? i : best, 0);
  container.innerHTML = `
    <select aria-label="${escapeHTML(AieyesI18n.t("估值序列"))}" ${series.length === 1 ? 'disabled' : ''}>
      ${series.map((row, i) => option(i, chooseLabel(row), picked < 0 ? newest : picked)).join('')}
    </select>
    <div data-history-plot></div>`;
  const select = container.querySelector('select');
  const plot = container.querySelector('[data-history-plot]');
  const draw = () => {
    const row = series[Number(select.value)];
    const points = row.points;
    const dayNumber = (p) => Date.parse(p.day + 'T00:00:00Z') / 86400000;
    const dates = points.map(dayNumber);
    const span = dates.at(-1) - dates[0];
    const max = points.reduce((value, p) => Math.max(value, p.value), 0) || 1;
    const x = (i) => span ? 8 + (dates[i] - dates[0]) / span * 464 : 240;
    const y = (i) => 116 - points[i].value / max * 108;
    const indices = points.map((_, i) => i);
    const positions = indices.map(i => `${x(i)} ${y(i)}`);
    const solid = [`M${positions[0]}`];
    const gaps = [];
    for (let i = 1; i < points.length; i++) {
      if (dates[i] === dates[i - 1] + 1) {
        solid.push(`L${positions[i]}`);
      } else {
        solid.push(`M${positions[i]}`);
        gaps.push(`M${positions[i - 1]} L${positions[i]}`);
      }
    }
    plot.innerHTML = `
      <div class="estimate-history-graph">
        <div class="estimate-history-scale"><span>${escapeHTML(money(max))}</span><span>$0</span></div>
        <svg viewBox="0 0 480 124" preserveAspectRatio="none" role="img" aria-label="${escapeHTML(AieyesI18n.t("滚动估值历史"))}">
          <path class="estimate-history-grid" d="M0 8H480 M0 62H480 M0 116H480"/>
          ${gaps.length ? `<path class="estimate-history-gap" d="${gaps.join(' ')}"/>` : ''}
          <path class="estimate-history-line" d="${solid.join(' ')}"/>
          <path data-history-guide class="estimate-history-guide"/>
          <circle data-history-dot class="estimate-history-point" r="4"/>
        </svg>
        <div class="estimate-history-axis muted"><span>${points[0].day}</span>${span ? `<span>${points.at(-1).day}</span>` : ''}</div>
      </div>
      ${points.length > 1 ? `<input aria-label="${escapeHTML(AieyesI18n.t("所选日期"))}" type="range" min="0" max="${points.length - 1}" step="1">` : ''}
      <output data-history-reading></output>
      ${gaps.length ? `<p class="tiny muted">${AieyesI18n.t("虚线跨过没有估值的日期。")}</p>` : ''}
      <p class="tiny muted">${AieyesI18n.t("每天使用截至当日的近 90 天有效自动样本，样本权重每 7 天减半。")}</p>
      ${row.period === 'weeklyRatio' ? `<p class="tiny muted">${AieyesI18n.t("7d 容量推算取当日 5h 滚动估值乘以同套餐的容量倍率。")}</p>` : ''}`;
    const svg = plot.querySelector('svg');
    const guide = plot.querySelector('[data-history-guide]');
    const dot = plot.querySelector('[data-history-dot]');
    const slider = plot.querySelector('input');
    const reading = plot.querySelector('output');
    const show = (i) => {
      const p = points[i];
      guide.setAttribute('d', `M${x(i)} 8V116`);
      dot.setAttribute('cx', x(i));dot.setAttribute('cy', y(i));
      reading.textContent = `${p.day} · ${money(p.value)} USD · ${AieyesI18n.t("{0} 个样本", p.sampleCount)}`;
      if (slider) {slider.value = i;slider.setAttribute('aria-valuetext', reading.textContent);}
      estimateHistorySelection.set(selectionKey, { series: identity(row), day: p.day });
    };
    if (slider) slider.oninput = () => show(Number(slider.value));
    svg.onpointermove = (event) => {
      const box = svg.getBoundingClientRect();
      const at = 480 * (event.clientX - box.left) / box.width;
      show(indices.reduce((best, i) => Math.abs(x(i) - at) < Math.abs(x(best) - at) ? i : best, 0));
    };
    const previous = estimateHistorySelection.get(selectionKey);
    const index = previous?.series === identity(row) ? points.findIndex(p => p.day === previous.day) : -1;
    show(index < 0 ? points.length - 1 : index);
  };
  select.onchange = draw;
  draw();
}

const manualSamplingExpanded = new Map();
// Management always uses the unfiltered snapshot. Collapsed manual rows are not mounted.
async function openProviderEstimates(provider) {
  if (state.estimateBusy) {notify(AieyesI18n.t("采样操作正在保存，请稍后查看。"));return;}
  const dialog = quotaDialog(AieyesI18n.t("{0} · 估值与采样", providers[provider] ?? provider), `<p role="status">${AieyesI18n.t("正在读取估值与采样记录…")}</p>`, 'valuation');
  const request = crypto.randomUUID();dialog.dataset.loadingRequest = request;
  let snapshot;
  try {snapshot = await api('dashboard.summary');}
  catch (error) {if (dialog.open && dialog.dataset.loadingRequest === request) {AieyesUI.showError($('#quota-tool-error'), String(error), AieyesI18n.t("估值记录读取失败"));$('#quota-tool-error').hidden = false;const retry = document.createElement('button');retry.textContent = AieyesI18n.t("重试读取");retry.onclick = () => openProviderEstimates(provider);dialog.querySelector('.quota-dialog-body').append(retry);}return;}
  if (!dialog.open || dialog.dataset.loadingRequest !== request) return;
  const sources = state.settings.sources.filter((s) => s.provider === provider),accounts = state.settings.accounts.filter((a) => a.provider === provider && !a.archived);
  const records = [...(snapshot.quotaEstimates ?? []).map((e) => ({ ...e, kind: 'quota' })), ...(snapshot.creditEstimates ?? []).map((e) => ({ ...e, kind: 'credits' }))].filter((e) => e.accountKey.startsWith(provider + ':')).sort((a, b) => (b.checkpointAt ?? b.startedAt) - (a.checkpointAt ?? a.startedAt));
  const recordHTML = (record) => `<div class="sampling-row"><strong>${escapeHTML(historicalAccounts().find((a) => a.provider + ':' + a.id === record.accountKey)?.name ?? record.accountKey)}</strong><span>${record.kind === 'credits' ? AieyesI18n.t("credits 估值") : escapeHTML(AieyesI18n.t("{0} 额度估值", AieyesUI.estimatePeriods(snapshot.quotas?.find(q => q.provider + ':' + q.accountId === record.accountKey), [record], record.groupId)).trim())} · ${escapeHTML(estimateStatus(record))}</span><small>${date(record.startedAt)} → ${date(record.checkpointAt)}</small>${record.rollingExcludedReason ? `<small class="muted">${escapeHTML(AieyesI18n.message(record.rollingExcludedReason))}</small>` : ''}<button type="button" data-valuation-record="${records.indexOf(record)}">${AieyesI18n.t("查看与管理")}</button></div>`;
  const running = records.filter((e) => e.status !== 'completed' && e.samplingMode !== 'automatic'),history = records.filter((e) => e.status === 'completed' && e.samplingMode !== 'automatic'),automaticHistory = records.filter((e) => e.samplingMode === 'automatic');
  const automatic = sources.length ? window.AieyesAutomatic.html(sources) : `<p class="muted">${AieyesI18n.t("添加数据目录后可配置自动估值。")}</p>`;
  const manual = () => accounts.map((a) => `<div class="list-row"><strong class="row-body">${escapeHTML(a.name)}</strong><button type="button" data-valuation-account="${escapeHTML(a.provider + ':' + a.id)}">${AieyesI18n.t("额度采样")}</button>${provider === 'codex' ? `<button type="button" data-valuation-account="${escapeHTML(a.provider + ':' + a.id)}" data-kind="credits">${AieyesI18n.t("credits 采样")}</button>` : ''}</div>`).join('') || `<p class="muted">${AieyesI18n.t("添加账户后可开始手动采样。")}</p>`;
  quotaDialog(AieyesI18n.t("{0} · 估值与采样", providers[provider] ?? provider), `<h3>${AieyesI18n.t("自动估值")}</h3><p class="muted">${AieyesAutomatic.intro(provider)}</p>${automatic}<details id="manual-sampling" ${manualSamplingExpanded.get(provider) ? 'open' : ''}><summary>${AieyesI18n.t("手动采样（进行中 {0}）", running.length)}</summary><div data-manual-body></div></details><h3>${AieyesI18n.t("自动采样历史")}</h3>${automaticHistory.map(recordHTML).join('') || `<p class="muted">${AieyesI18n.t("暂无记录。")}</p>`}`, 'valuation');
  dialog.dataset.valuationProvider = provider;
  const bindRecords = () => {
    dialog.querySelectorAll('[data-valuation-account]').forEach((button) => button.onclick = () => openQuotaEstimate(button.dataset.valuationAccount, button.dataset.kind ?? 'weekly', '', snapshot));
    dialog.querySelectorAll('[data-valuation-record]').forEach((button) => button.onclick = () => {const record = records[Number(button.dataset.valuationRecord)];return openQuotaEstimate(record.accountKey, record.kind === 'credits' ? 'credits' : 'weekly', record.groupId ?? '', snapshot);});
  };
  const details = dialog.querySelector('#manual-sampling');
  const mountManual = () => {manualSamplingExpanded.set(provider, details.open);details.querySelector('[data-manual-body]').innerHTML = details.open ? manual() + `<h3>${AieyesI18n.t("进行中的手动采样")}</h3>${running.map(recordHTML).join('') || `<p class="muted">${AieyesI18n.t("当前没有进行中的手动采样。")}</p>`}<h3>${AieyesI18n.t("手动采样历史")}</h3>${history.map(recordHTML).join('') || `<p class="muted">${AieyesI18n.t("暂无已完成记录。")}</p>`}` : '';bindRecords();};
  details.ontoggle = mountManual;mountManual();
}
