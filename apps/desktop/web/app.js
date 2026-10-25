const historicalAccounts = () => [...(state.settings?.accounts ?? []), ...(state.settings?.deletedAccounts ?? [])];
const filterAccounts = () => [...(state.settings?.accounts ?? []), ...(state.settings?.historyShowDeletedAccounts ? state.settings?.deletedAccounts ?? [] : [])];
const accountFilterName = (a) => a.name + (state.settings.accounts.some((current) => current.provider === a.provider && current.id === a.id) ? a.archived ? AieyesI18n.t("（已归档）") : '' : AieyesI18n.t("（已删除）"));
const $ = (s) => document.querySelector(s);
document.querySelectorAll('[data-ui-icon]').forEach((element) => element.innerHTML = uiIcon(element.dataset.uiIcon));
/* The floating window expands into a compact panel that mirrors the macOS menu bar
   popover; the same renderers feed both surfaces. */
const PANEL = Boolean(document.querySelector('#panel'));
const escapeHTML = (value) => window.AieyesUI.escapeHTML(value);
const compact = (n) => n >= 999995000 ? (n / 1e9).toFixed(2) + 'B' : n >= 999950 ? (n / 1e6).toFixed(2) + 'M' : n >= 999.5 ? (n / 1e3).toFixed(1) + 'K' : Math.round(n || 0).toString();
const money = (n) => '$' + (n || 0).toFixed(2);
function creditAmount(value) {
  if (value == null || String(value).trim() === '') return '—';
  const match = String(value).trim().match(/^([+-]?)(\d+)(?:\.(\d*))?(?:e([+-]?\d+))?$/i);
  if (!match) return '—';
  const exponent = Number(match[4] || 0),fraction = match[3] || '';
  if (!Number.isSafeInteger(exponent) || Math.abs(exponent) > 1000) return '—';
  let digits = BigInt(match[2] + fraction),shift = 2 + exponent - fraction.length;
  if (shift >= 0) digits *= 10n ** BigInt(shift);else
  {const divisor = 10n ** BigInt(-shift);digits = (digits + divisor / 2n) / divisor;}
  const text = digits.toString().padStart(3, '0');
  return (match[1] === '-' && digits !== 0n ? '-' : '') + text.slice(0, -2) + '.' + text.slice(-2);
}
function creditValue(record) {
  if (!record) return '';
  if (record.status === 'pending') return AieyesI18n.t("待确认");
  return Number.isFinite(record.valuePer1000) ? '1000 credit ≈ ' + money(record.valuePer1000) + ' USD' : AieyesUI.estimateIssue(record);
}
const pct = (n) => n == null ? '—' : n.toFixed(1) + '%';
const bytes = (n) => {if (!Number.isFinite(n) || n < 0) return '—';let i = 0;while (n >= 1024 && i < 4) {n /= 1024;i++;}return n.toFixed(i ? 1 : 0) + [' B', ' KiB', ' MiB', ' GiB', ' TiB'][i];};
const speed = (n) => n == null ? '—' : bytes(n) + '/s';
const date = (n) => n ? new Date(n * 1000).toLocaleString(AieyesI18n.locale, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }) : '—';
const providers = window.AieyesContract.providers;
function providerMark(provider) {
  const icon = window.AieyesContract.providerIcons[provider],name = escapeHTML(providers[provider] ?? provider);
  return `<span class="provider-mark${icon ? icon.monochrome ? ' provider-monochrome' : '' : ' provider-fallback'}" data-provider="${escapeHTML(provider)}" title="${name}" aria-label="${name}">${icon ? `<img src="provider-${icon.asset}.png" class="${icon.darkAsset ? 'provider-light' : ''}" alt="" width="36" height="36">${icon.darkAsset ? `<img src="provider-${icon.darkAsset}.png" class="provider-dark" alt="" width="36" height="36">` : ''}` : uiIcon('terminal')}</span>`;
}
function providerIdentity(provider) {return `<span class="provider-identity">${providerMark(provider)}<span>${escapeHTML(providers[provider] ?? provider)}</span></span>`;}
function subscriptionBadge(plan) {const label = AieyesUI.subscription(plan);return label ? `<span class="subscription-badge" title="${escapeHTML(plan)}" aria-label="${AieyesI18n.t("订阅 {0}", escapeHTML(label))}">${escapeHTML(label)}</span>` : '';}
// Keep category slots across filtering and appearance changes. Probe collisions
// until a preset is full; larger category sets reuse colors with named legends.
const modelSlots = new Map(),modelUsedSlots = new Map();
function chartColors(preset = document.documentElement.dataset.chartPalette, contrast = document.documentElement.dataset.chartContrast, theme = document.documentElement.dataset.theme) {
  return (window.AieyesChartPalettes.find((p) => p.id === preset) ?? window.AieyesChartPalettes[0]).variants[(theme === 'dark' ? 'dark' : 'light') + '-' + (contrast === 'low' ? 'low' : 'high')].map((c) => '#' + c);
}
const color = (name, preset = document.documentElement.dataset.chartPalette || 'classic') => {
  const colors = chartColors(preset);
  if (!modelSlots.has(preset)) {modelSlots.set(preset, new Map());modelUsedSlots.set(preset, new Set());}
  const slots = modelSlots.get(preset),used = modelUsedSlots.get(preset);
  if (!slots.has(name)) {
    let hash = 2166136261;for (const c of new TextEncoder().encode(name)) hash = Math.imul(hash ^ c, 16777619) >>> 0;
    let slot = hash % colors.length;
    for (let attempt = 0; attempt < colors.length && used.has(slot); attempt++) slot = (slot + 1) % colors.length;
    slots.set(name, slot);used.add(slot);
  }
  return colors[slots.get(name)];
};
function chartPalettePreview() {
  const form = $('#general-form');if (!form) return '';
  const preset = form.elements.namedItem('appearanceChartPalette')?.value ?? 'classic',contrast = form.elements.namedItem('appearanceChartContrast')?.value ?? 'high';
  return ['light', 'dark'].map((theme) => `<div class="chart-palette-sample" data-preview-theme="${theme}" role="img" aria-label="${AieyesI18n.t("{0}背景柱形图配色预览", theme === 'dark' ? AieyesI18n.t("深色") : AieyesI18n.t("浅色"))}"><span>${theme === 'dark' ? AieyesI18n.t("深色") : AieyesI18n.t("浅色")}</span><div class="chart-palette-bars">${chartColors(preset, contrast, theme).map((c, i) => `<i style="background:${c};height:${[54, 82, 66, 94, 44, 74, 60, 88, 50, 70][i]}%"></i>`).join('')}</div></div>`).join('');
}
function updateChartPalettePreview() {const preview = $('#chart-palette-preview');if (preview) preview.innerHTML = chartPalettePreview();}


let groups = { cpu: 'CPU', memory: AieyesI18n.t("内存"), gpu: 'GPU', filesystems: AieyesI18n.t("文件系统"), disk: AieyesI18n.t("磁盘 I/O"), network: AieyesI18n.t("网络") };window.addEventListener("aieyes:language", () => {groups = { cpu: 'CPU', memory: AieyesI18n.t("内存"), gpu: 'GPU', filesystems: AieyesI18n.t("文件系统"), disk: AieyesI18n.t("磁盘 I/O"), network: AieyesI18n.t("网络") };});
let detailOptions = { uptime: AieyesI18n.t("连续运行时间"), cpuTimes: AieyesI18n.t("CPU 时间分布"), memoryCache: AieyesI18n.t("内存缓存 / Buffer"), swap: 'Swap', fsAvailable: AieyesI18n.t("文件系统可用空间"), fsType: AieyesI18n.t("文件系统类型 / 设备"), inodes: 'inode', diskIops: AieyesI18n.t("磁盘 IOPS"), diskBusy: AieyesI18n.t("磁盘忙碌率"), networkTotals: AieyesI18n.t("累计流量"), networkErrors: AieyesI18n.t("网络错误 / 丢包"), gpuMemory: AieyesI18n.t("GPU 显存"), gpuThermals: AieyesI18n.t("GPU 温度 / 功耗") };window.addEventListener("aieyes:language", () => {detailOptions = { uptime: AieyesI18n.t("连续运行时间"), cpuTimes: AieyesI18n.t("CPU 时间分布"), memoryCache: AieyesI18n.t("内存缓存 / Buffer"), swap: 'Swap', fsAvailable: AieyesI18n.t("文件系统可用空间"), fsType: AieyesI18n.t("文件系统类型 / 设备"), inodes: 'inode', diskIops: AieyesI18n.t("磁盘 IOPS"), diskBusy: AieyesI18n.t("磁盘忙碌率"), networkTotals: AieyesI18n.t("累计流量"), networkErrors: AieyesI18n.t("网络错误 / 丢包"), gpuMemory: AieyesI18n.t("GPU 显存"), gpuThermals: AieyesI18n.t("GPU 温度 / 功耗") };});
function configuredHosts(settings = state.settings) {return [...(settings?.localMonitor ? [{ id: 'local', name: AieyesI18n.t("本机"), target: AieyesI18n.t("本机"), ...settings.localMonitor }] : []), ...(settings?.hosts ?? [])];}
function monitoredHosts(settings = state.settings) {return configuredHosts(settings).map((host) => ({ ...host, enabled: host.id === 'local' ? host.enabled !== false : host.monitoringEnabled !== false }));}
function refreshIndicator(key) {return `<button type="button" class="heading-refresh" data-refresh-status="${key}" aria-label="${AieyesI18n.t("{0}状态", refreshLabels[key])}" title="${AieyesI18n.t("{0}状态", refreshLabels[key])}"><span aria-hidden="true">${uiIcon('info')}</span></button>`;}
function updateHeadingIndicators() {for (const el of document.querySelectorAll('[data-refresh-status]')) {const r = state.refreshState[el.dataset.refreshStatus] ?? {};el.dataset.busy = String(Boolean(r.busy || el.dataset.refreshStatus === 'scan' && state.dashboardPending));el.dataset.error = String(Boolean(r.error || el.dataset.refreshStatus === 'scan' && state.dashboardError));el.title = (refreshLabels[el.dataset.refreshStatus] ?? AieyesI18n.t("刷新")) + AieyesI18n.t("状态 · ") + (el.dataset.busy === 'true' ? AieyesI18n.t("进行中") : el.dataset.error === 'true' ? AieyesI18n.t("部分失败") : r.success ? AieyesI18n.t("已更新 · ") + timeLabel(r.success) : AieyesI18n.t("尚未刷新")) + AieyesI18n.t("；点击查看详情");el.setAttribute('aria-label', el.title);el.firstElementChild.innerHTML = uiIcon(el.dataset.busy === 'true' ? 'refresh' : el.dataset.error === 'true' ? 'warning' : r.success ? 'checked' : 'info');}}
function openRefreshStatus(key) {
  let pop = document.querySelector('#refresh-status-popover');if (!pop) {pop = document.createElement('div');pop.id = 'refresh-status-popover';pop.setAttribute('popover', 'auto');pop.addEventListener('toggle', () => {if (!pop.matches(':popover-open')) pop.replaceChildren();});document.body.append(pop);}
  pop.dataset.key = key;
  const keys = key === 'scan' ? ['scan', 'prices'] : [key],rows = keys.flatMap((k) => (state.refreshState[k]?.failures ?? []).filter((r) => !r.dismissed).map((r) => ({ ...r, key: k, failure: r })));
  pop.innerHTML = '<strong>' + escapeHTML(refreshLabels[key]) + '</strong>' + (rows.length ? rows.map((r, i) => '<div class="error-row"><p>' + escapeHTML(AieyesI18n.message((r.name || r.id || r.accountId || AieyesI18n.t("数据源")) + ' · ' + r.error)) + '</p><button data-status-retry="' + i + AieyesI18n.t("\">重试</button><button data-status-dismiss=\"") + i + AieyesI18n.t("\">关闭</button></div>")).join('') : '<p>' + escapeHTML(AieyesI18n.message(state.refreshState[key]?.busy ? AieyesI18n.t("正在刷新…") : keys.some((k) => state.refreshState[k]?.error) ? AieyesI18n.t("本次错误提示已关闭") : state.refreshState[key]?.success ? AieyesI18n.t("上次成功：") + timeLabel(state.refreshState[key].success) : AieyesI18n.t("尚未刷新"))) + '</p>');
  pop.querySelectorAll('[data-status-dismiss]').forEach((b) => b.onclick = () => {rows[Number(b.dataset.statusDismiss)].failure.dismissed = true;openRefreshStatus(key);});
  pop.querySelectorAll('[data-status-retry]').forEach((b) => b.onclick = () => {const r = rows[Number(b.dataset.statusRetry)];pop.hidePopover();({ scan, rescan, quotas, prices: syncPrices, hosts: sample })[r.key](r.key === 'quotas' ? r.accountId : r.id);});pop.showPopover();if (key === 'scan') {renderQueryStatus();const query = $('#query-status');pop.append(query);query.hidden = !state.dashboardPending && !state.dashboardError;}
}
document.addEventListener('click', (event) => {const button = event.target.closest('[data-refresh-status]');if (button) openRefreshStatus(button.dataset.refreshStatus);});
document.addEventListener('keydown', (event) => {const pop = document.querySelector('#refresh-status-popover');if (event.key === 'Escape' && pop?.matches(':popover-open')) {event.preventDefault();event.stopImmediatePropagation();pop.hidePopover();document.querySelector('[data-refresh-status="' + pop.dataset.key + '"]')?.focus({ preventScroll: true });}}, true);
const state = { page: 'agent', settingsTab: 'integrations', settingsAgent: 'codex', generalTab: 'appearance', priceSearch: '', settings: null, settingsDraft: null, settingsBaseline: null, settingsCommitting: false, dashboard: null, hosts: [], prices: [], provider: '', sourceId: '', accountKey: '', model: '', days: 1, dateRange: null, cost: false, busy: false, settingsSaving: false, serverBusy: false, lastScan: 0, lastMetrics: 0, lastQuota: 0, lastSuccessfulUpdate: 0, quotaBusy: false, estimateBusy: false, quotaError: '', refreshState: {}, savedAt: 0, savedSection: '', saveError: '', dashboardRequest: 0, panelOpen: false, panelPage: 'agent' };
try {const provider = localStorage.getItem('aieyes.settings.agent');if (providers[provider]) state.settingsAgent = provider;} catch (_) {}
const settingsCategory = (category) => ({ sources: 'integrations', accounts: 'integrations', codexAuth: 'integrations', servers: 'hosts', connection: 'general' })[category] ?? category;
async function openSettings(target = {}) {
  if (PANEL) {try {await window.__TAURI__.core.invoke('desktop_action', { action: 'settings-target:' + JSON.stringify(target) });} catch (error) {notify(String(error), 'error');}return;}
  if (!state.settings) await appReady;
  rememberPage();viewPreferences.save(dashboardScope(), state);captureSettingsForms();state.page = 'settings';state.settingsTab = settingsCategory(target.category ?? 'integrations');
  if (state.settingsTab === 'integrations') window.AieyesAgentSettings.navigate(target);
  if (state.settingsTab === 'general' && target.section) state.generalTab = target.section;
  render();
  if (state.settingsTab === 'hosts' && (target.hostId || target.section === 'add-host')) {const host = target.hostId ? configuredHosts().find((h) => h.id === target.hostId) : undefined;if (target.hostId && !host) notify(AieyesI18n.t("此服务器已移除，请重新选择。"), 'error');else {editItem(host);if (target.section === 'shell') {const field = $('#field-shell');if (field) {field.closest('details').open = true;field.focus();}}}}
  if (state.settingsTab === 'integrations' && target.section) {const section = document.getElementById('agent-section-' + target.section);if (section) {section.open = true;section.scrollIntoView({ block: 'start' });}}
  if (state.settingsTab === 'integrations' && target.accountKey && target.machineId && target.section === 'command') {
    const field = [...document.querySelectorAll('[data-command-field]')].find((el) => el.dataset.commandField === target.machineId && el.dataset.account === target.accountKey);
    if (field) {field.closest('details').open = true;field.scrollIntoView({ block: 'center' });field.focus({ preventScroll: true });} else
    notify(AieyesI18n.t("此账户的目标机器已变化，请在账户设备列表重新选择。"), 'error');
  }
  if (state.settingsTab === 'prices') {try {state.prices = await api('prices.list');if (state.page === 'settings' && state.settingsTab === 'prices') renderSettings();} catch (error) {notify(String(error), 'error');}}
}
function savePanelView() {
  if (!PANEL) return;
  try {localStorage.setItem('aieyes.panel.view', JSON.stringify(Object.fromEntries(['page', 'provider', 'sourceId', 'accountKey', 'model', 'days', 'dateRange', 'cost', 'serverFilter', 'serverSelection'].map((k) => [k, state[k]]))));} catch (_) {}
}
const viewPreferences = AieyesViewState.create(localStorage),dashboardCache = new Map(),appliedFilterCache = new Map();
let currentDashboardScope = PANEL ? 'panel' : 'overview';
const dashboardScope = () => currentDashboardScope;
const viewKey = () => PANEL ? 'panel:' + state.page : state.page === 'settings' ? 'settings:' + state.settingsTab + ':' + (state.settingsTab === 'general' ? state.generalTab : state.settingsAgent) : state.page;
Object.assign(state, viewPreferences.read(dashboardScope()));
if (PANEL) try {const saved = JSON.parse(localStorage.getItem('aieyes.panel.view') || '{}');if (saved.page === 'servers') state.page = 'servers';if (['all', 'errors', 'paused'].includes(saved.serverFilter)) state.serverFilter = saved.serverFilter;} catch (_) {}
try {state.serverSelection = localStorage.getItem('aieyes.server.selection.' + (PANEL ? 'panel' : 'detail')) || 'all';} catch (_) {}
let renderedViewKey = null;
function rememberPage() {if (renderedViewKey) viewPreferences.savePosition(renderedViewKey, rememberView());}
function syncNavigation() {
  for (const b of document.querySelectorAll('aside nav button')) {
    const active = b.hasAttribute('data-guide-open') ? state.page === 'setup' : b.dataset.generalPage ? state.page === 'settings' && state.settingsTab === 'general' && b.dataset.generalPage === state.generalTab : b.dataset.settingsTab ? state.page === 'settings' && b.dataset.settingsTab === state.settingsTab : b.dataset.page === state.page;
    b.classList.toggle('active', active);if (active) b.setAttribute('aria-current', 'page');else b.removeAttribute('aria-current');
  }
}
async function navigatePage(page, context) {
  if (!['agent', 'analysis', 'servers'].includes(page) || PANEL && page === 'analysis') return;
  rememberPage();viewPreferences.save(dashboardScope(), state);
  captureSettingsForms();state.page = page;
  if (page !== 'servers') {
    currentDashboardScope = PANEL ? 'panel' : page === 'analysis' ? 'analysis' : 'overview';
    Object.assign(state, viewPreferences.read(dashboardScope()), context ? AieyesViewState.normalize({ ...viewPreferences.read(dashboardScope()), ...context }) : {});
    viewPreferences.save(dashboardScope(), state);state.dashboard = dashboardCache.get(dashboardScope()) ?? null;state.appliedFilters = appliedFilterCache.get(dashboardScope()) ?? filterSnapshot();state.dashboardError = '';state.restorePosition = viewKey();
    render();try {await loadDashboard();} catch (_) {}
  } else {render();void sample();}
  if (PANEL) window.__TAURI__.core.invoke('desktop_panel_page', { page }).catch((error) => notify(String(error), 'error'));
}
function openAnalysis(accountKey = state.accountKey, provider = state.provider) {
  const view = { page: 'analysis', provider: provider || accountKey?.split(':')[0] || '', accountKey: '', sourceId: '', model: '' };
  if (PANEL) return window.__TAURI__.core.invoke('desktop_detail', { view }).catch((e) => notify(String(e), 'error'));
  return navigatePage('analysis', view);
}
function openDetail() {
  return window.__TAURI__.core.invoke('desktop_detail', { view: { page: state.page === 'servers' ? 'servers' : 'agent' } }).catch((e) => notify(String(e), 'error'));
}
const ownedOperations = new Map(),runtimeVersions = {},querySnapshots = new Map(),snapshotStatusVersions = new Map();
let localDataEpoch = 0,localUsageEpoch = 0,lastDashboardVersion = '';
let resumeSharedRefresh = () => {};
const clearUsageSnapshots = () => {querySnapshots.clear();snapshotStatusVersions.clear();};
const usageVersion = () => JSON.stringify([localUsageEpoch, ...['settings', 'usage', 'prices'].map((key) => runtimeVersions[key] ?? 0), new Date().toDateString(), Intl.DateTimeFormat().resolvedOptions().timeZone]);
const dashboardVersion = () => JSON.stringify([localDataEpoch, ...['settings', 'usage', 'quotas', 'estimates', 'prices', 'sources'].map((key) => runtimeVersions[key] ?? 0), new Date().toDateString(), Intl.DateTimeFormat().resolvedOptions().timeZone]);
async function api(method, params = {}) {
  const mutation = window.AieyesContract.methods[method]?.invalidates.length || ['pets.select', 'pets.import'].includes(method);
  const operationId = mutation ? params.operationId || crypto.randomUUID() : null;
  if (operationId) {ownedOperations.set(operationId, Date.now());if (ownedOperations.size > 256) ownedOperations.delete(ownedOperations.keys().next().value);}
  try {const result = await window.__TAURI__.core.invoke('engine_call', { method, params, ...(operationId ? { operationId } : {}) });return AieyesUI.canonicalPayload(result, result?.accountAliases ?? state.settings?.accountAliases ?? {});} finally
  {if (mutation) {localDataEpoch++;if (!state.revisionEvents || window.AieyesContract.methods[method]?.invalidates.includes('settings') || ['sources.scan', 'sources.rescan', 'prices.save', 'prices.sync', 'prices.recalculate'].includes(method)) {localUsageEpoch++;clearUsageSnapshots();}}if (method === 'quotas.refresh' || window.AieyesContract.methods[method]?.invalidates.includes('settings')) invalidateQuotaSchedule();}
}
let notificationTimer;
function notify(message, kind = 'info', source = '') {
  clearTimeout(notificationTimer);const el = $('#message');el.hidden = !message;el.dataset.kind = kind;el.dataset.source = source;
  el.innerHTML = message ? '<div class="message-text">' + (kind === 'error' ? AieyesUI.errorHTML(message) : escapeHTML(AieyesI18n.message(message))) + AieyesI18n.t("</div><button type=\"button\" class=\"message-close\" aria-label=\"关闭提示\">") + uiIcon('x') + '</button>' : '';
  el.querySelector('.message-close')?.addEventListener('click', () => notify(''));
  if (message && kind === 'info') notificationTimer = setTimeout(() => notify(''), 5000);
}
let refreshLabels = { scan: AieyesI18n.t("同步记录"), rescan: AieyesI18n.t("全量重扫"), quotas: AieyesI18n.t("刷新额度"), prices: AieyesI18n.t("同步价格"), hosts: AieyesI18n.t("刷新服务器") };window.addEventListener("aieyes:language", () => {refreshLabels = { scan: AieyesI18n.t("同步记录"), rescan: AieyesI18n.t("全量重扫"), quotas: AieyesI18n.t("刷新额度"), prices: AieyesI18n.t("同步价格"), hosts: AieyesI18n.t("刷新服务器") };});
function updateActivity() {
  resumeSharedRefresh();
  scheduleRefreshes();
  const el = $('#activity'),fresh = AieyesUI.freshness(state.settings, state.dashboard),data = fresh.timestamp;
  const enabledHosts = monitoredHosts().filter((h) => h.enabled),updatedHosts = enabledHosts.filter((h) => hostStatus(h, state.hosts.find((r) => r.id === h.id)) === AieyesI18n.t('正常')).length;
  const running = Object.entries(state.refreshState).filter(([, v]) => v.busy).map(([k]) => refreshLabels[k]);
  const failed = state.saveError || fresh.failed || Object.values(state.refreshState).some((v) => v.error) || state.hosts.some((h) => h.error);
  const engineBusy = ['scan', 'rescan', 'prices'].some((key) => state.refreshState[key]?.busy);
  const status = state.busy || running.length ? 'busy' : failed ? 'error' : fresh.delayed ? 'warning' : data ? 'success' : 'info';
  el.dataset.status = status;
  el.textContent = (state.page === 'servers' ? updatedHosts + '/' + enabledHosts.length + AieyesI18n.t(" 台已更新") : data ? AieyesI18n.t("记录同步于 ") + timeLabel(data * 1000) : fresh.total ? AieyesI18n.t("记录尚未全部同步") : AieyesI18n.t("未接入用量来源")) + ' · ' + (running.length ? running.join('、') + AieyesI18n.t("中…") : state.busy ? AieyesI18n.t("保存中…") : failed ? AieyesI18n.t("部分失败") : state.page === 'servers' ? updatedHosts < enabledHosts.length ? AieyesI18n.t("存在延迟或未采样") : AieyesI18n.t("采样已更新") : fresh.delayed ? fresh.delayed + AieyesI18n.t(" 个来源有延迟") : fresh.total ? AieyesI18n.t("记录已同步") : AieyesI18n.t("等待接入"));
  el.title = state.page === 'servers' ? AieyesI18n.t("按每台已启用主机的状态统计；点击刷新服务器") : AieyesI18n.t("各来源最后成功同步时间（取最早）；查询生成于 ") + date(state.dashboard?.generatedAt) + AieyesI18n.t("；点击同步记录");el.tabIndex = 0;el.setAttribute('role', 'button');el.setAttribute('aria-label', el.textContent + '；' + el.title);window.AieyesNetwork?.draw();
  updateHeadingIndicators();
  for (const button of document.querySelectorAll('[data-refresh]')) {
    const key = button.dataset.refresh,r = state.refreshState[key] ?? {};
    button.disabled = Boolean((key === 'hosts' ? hostRefreshBusy() : r.busy) || state.settingsSaving || (key === 'quotas' ? state.quotaBusy : (key === 'scan' || key === 'prices') && (state.busy || engineBusy)));
    button.innerHTML = '<span>' + refreshLabels[key] + '</span><small>' + (r.busy ? AieyesI18n.t("进行中…") : r.error ? AieyesI18n.t("失败 · 可重试") : r.success ? timeLabel(r.success) : AieyesI18n.t("尚未刷新")) + '</small>';
  }
  updateRescanStatus();
}
function showFailures(error, key, itemId = null) {
  const failures = error?.failures ?? (error ? [{ id: itemId, name: refreshLabels[key] ?? AieyesI18n.t("操作"), error: error.message ?? String(error) }] : []);
  const r = state.refreshState[key] ??= {};
  r.failures = [...(itemId ? (r.failures ?? []).filter((row) => (row.accountId ?? row.id) !== itemId) : []), ...failures];
  r.error = r.failures.length ? AieyesI18n.t("读取失败") : null;
  renderFailures();
}
function renderFailures() {updateHeadingIndicators();const pop = document.querySelector('#refresh-status-popover');if (pop?.matches(':popover-open')) openRefreshStatus(pop.dataset.key);}
function beginRefresh(key) {state.refreshState[key] = { ...state.refreshState[key], busy: true };updateActivity();}
function endRefresh(key, error, itemId = null) {const r = state.refreshState[key];r.busy = false;if (!error) r.success = Date.now();showFailures(error, key, itemId);updateActivity();}
const timeLabel = (stamp) => new Date(stamp).toLocaleTimeString(AieyesI18n.locale, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
let pendingRender = false;
function interactionOpen() {
  return Boolean($('#quota-failures-popover')?.matches(':popover-open') || window.AieyesSingleSelect?.isOpen() || window.AieyesSelect?.isOpen?.() || $('#token-popover') && !$('#token-popover').hidden || $('#editor')?.open || $('#quota-dialog')?.open || $('#panel-more')?.open || $('#panel-menu') && !$('#panel-menu').hidden || $('#refresh-menu') && !$('#refresh-menu').hidden || $('.machine-menu[open]'));
}
function flushRender() {
  if (pendingRender && !interactionOpen()) {pendingRender = false;render();}
}
document.addEventListener('click', () => setTimeout(flushRender, 0));
document.addEventListener('keydown', () => setTimeout(flushRender, 0));
window.addEventListener('storage', (event) => {
  if (event.key === 'aieyes.panel.accounts.v2') {if (state.dashboard) render();return;}
  if (!event.key?.startsWith('quota.expanded.v2.' + (PANEL ? 'panel.' : 'detail.'))) return;
  const key = event.key.slice(('quota.expanded.v2.' + (PANEL ? 'panel.' : 'detail.')).length);
  const detail = document.querySelector('[data-quota="' + CSS.escape(key) + '"] .quota-disclosure');
  if (detail && detail.dataset.collapsible === 'true') detail.open = event.newValue !== 'false';
});
// Keep the user's place when live values replace their markup.
function focusSelector(element) {
  if (!element || element === document.body) return null;
  if (element.id) return '#' + CSS.escape(element.id);
  const attributes = ['data-quota-failures', 'data-quota-failure-retry', 'data-quota-failure-settings', 'data-overview-agent', 'data-view-analysis', 'data-switch-account', 'data-general-page', 'data-refresh-status', 'data-account-settings', 'data-heat-day', 'data-trend-model', 'data-day-toggle', 'data-credit-estimate', 'data-manage-sampling', 'data-estimate', 'data-page', 'data-settings-tab', 'data-edit', 'data-remove', 'data-price', 'data-account-edit', 'data-account-options', 'data-device-command', 'data-account-login', 'data-device-manage', 'data-account-delete', 'data-account-connect', 'data-account-restore', 'data-source-advanced', 'data-agent-advanced', 'data-enable', 'data-gap-map', 'data-gap-price', 'data-host-status'];
  for (const attribute of attributes) if (element.hasAttribute(attribute)) return `[${attribute}="${CSS.escape(element.getAttribute(attribute))}"]`;
  if (element.tagName === 'SUMMARY') for (const attribute of ['data-agent-detail', 'data-metric', 'data-persist-disclosure']) if (element.parentElement.hasAttribute(attribute)) return `[${attribute}="${CSS.escape(element.parentElement.getAttribute(attribute))}"] > summary`;
  return null;
}
function rememberView() {
  return { focus: focusSelector(document.activeElement), window: [window.scrollX, window.scrollY], panel: $('.panel-body')?.scrollTop,
    closed: [...document.querySelectorAll('#content [data-agent-detail]:not(.quota-disclosure):not([open])')].map((el) => el.dataset.agentDetail),
    opened: [...document.querySelectorAll('#content [data-agent-detail]:not(.quota-disclosure)[open],#content [data-metric][open]')].map((el) => [el.hasAttribute('data-metric') ? 'data-metric' : 'data-agent-detail', el.dataset.metric ?? el.dataset.agentDetail]),
    scrollers: [...document.querySelectorAll('#content .daily-table,#content .prices-list')].map((el) => ({ className: el.className, top: el.scrollTop, left: el.scrollLeft })) };
}
function restoreView(view) {
  window.AieyesSingleSelect?.mountAll($('#content'));
  for (const key of view.closed ?? []) {const el = document.querySelector('[data-agent-detail="' + CSS.escape(key) + '"]');if (el && el.dataset.collapsible !== 'false') el.open = false;}
  for (const [attribute, value] of view.opened) {const el = document.querySelector(`[${attribute}="${CSS.escape(value)}"]`);if (el) {el.open = true;if (el.tagName === 'TBODY') {el.setAttribute('open', '');el.querySelector('[data-day-toggle]')?.setAttribute('aria-expanded', 'true');el.querySelectorAll('.day-model').forEach((row) => row.hidden = false);}}}
  if (view.focus) document.querySelector(view.focus)?.focus({ preventScroll: true });
  for (const item of view.scrollers) {const el = [...document.querySelectorAll('#content .daily-table,#content .prices-list')].find((el) => el.className === item.className);if (el) {el.scrollTop = item.top;el.scrollLeft = item.left;}}
  if (view.panel != null && $('.panel-body')) $('.panel-body').scrollTop = view.panel;
  window.scrollTo(...view.window);
}
async function openSetup(kind = 'sources') {
  if (PANEL) {try {await window.__TAURI__.core.invoke('desktop_action', { action: kind === 'hosts' ? 'add-host' : kind === 'quota' ? 'add-quota' : 'add-source' });} catch (error) {notify(String(error), 'error');}return;}
  await openSettings(kind === 'hosts' ? { category: 'hosts', section: 'add-host' } : { category: 'integrations', section: kind === 'quota' ? 'accounts' : 'machines' });
}
function onboarding() {
  return `<div class="card onboarding"><h2>${AieyesI18n.t("启用你的第一个 Agent")}</h2><p class="muted">${AieyesI18n.t("在 Agent 接入中启用 Agent、添加账户并选择机器。")}</p><div class="actions"><button id="add-first-source" class="primary" title="${AieyesI18n.t("选择 Agent 与机器")}">${AieyesI18n.t("Agent 接入")}</button><button id="add-first-quota">${AieyesI18n.t("账户额度")}</button><button id="add-first-host">${AieyesI18n.t("SSH 主机")}</button><button type="button" data-guide-open>${AieyesI18n.t("配置向导")}</button></div></div>`;
}
function bindSetup() {if ($('#add-first-source')) $('#add-first-source').onclick = () => openSetup();if ($('#add-first-host')) $('#add-first-host').onclick = () => openSetup('hosts');if ($('#add-first-quota')) $('#add-first-quota').onclick = () => openSetup('quota');}
function samplingBanner(d) {
  const records = [...(d.quotaEstimates ?? []), ...(d.creditEstimates ?? [])].filter((e) => e.status !== 'completed');
  if (!records.length) return '';if (records.some((e) => e.status === 'pending')) return attention(AieyesI18n.t("待确认：采样需要确认用量来源"), AieyesI18n.t("管理采样"), 'sampling-attention');
  return AieyesI18n.t("<button class=\"sampling-banner\" data-manage-sampling><span>后台采样 · ") + records.filter((e) => e.status === 'active').length + AieyesI18n.t(" 项进行中") + (records.some((e) => e.status === 'pending') ? AieyesI18n.t(" · 有待确认项") : '') + AieyesI18n.t("</span><span>管理采样 ") + uiIcon('right') + '</span></button>';
}
function openSampling() {
  if (state.estimateBusy) {notify(AieyesI18n.t("后台采样操作仍在进行，可从任务入口返回"));return;}
  const records = [...(state.dashboard.quotaEstimates ?? []), ...(state.dashboard.creditEstimates ?? [])].filter((e) => e.status !== 'completed');
  const rows = records.map((e, i) => '<div class="sampling-row"><strong>' + escapeHTML(historicalAccounts().find((a) => a.provider + ':' + a.id === e.accountKey)?.name ?? e.accountKey) + '</strong><span>' + (e.kind === 'credits' ? AieyesI18n.t("credit 价值") : e.valuationMode === 'fiveHour' ? AieyesI18n.t("5h / 7d 额度价值") : AieyesI18n.t("7d 整周价值")) + ' · ' + estimateStatus(e) + '</span>' + (e.reason ? '<div class="error">' + escapeHTML(AieyesI18n.message(e.reason)) + '</div>' : '') + '<div class="actions"><button data-sampling-open="' + i + AieyesI18n.t("\">查看与管理</button><button data-sampling-stop=\"") + i + AieyesI18n.t("\">结束采样</button></div></div>")).join('');
  quotaDialog(AieyesI18n.t("管理采样"), AieyesI18n.t("<p class=\"muted\">关闭窗口后继续采样，点击“结束采样”停止。</p>") + (rows || AieyesI18n.t("<p>当前没有进行中的采样</p>")));
  document.querySelectorAll('[data-sampling-open]').forEach((b) => b.onclick = () => {const e = records[Number(b.dataset.samplingOpen)];openQuotaEstimate(e.accountKey, e.kind === 'credits' ? 'credits' : 'weekly', e.groupId ?? '');});
  document.querySelectorAll('[data-sampling-stop]').forEach((b) => b.onclick = async () => {const e = records[Number(b.dataset.samplingStop)];if (await quotaAction((e.kind === 'credits' ? 'creditEstimates.' : 'quotaEstimates.') + 'stop', { id: e.id })) openSampling();});
}
function disclosureOpen(key, fallback = true) {try {const value = localStorage.getItem('aieyes.disclosure.v3.' + (PANEL ? 'panel.' : 'detail.') + key);return value == null ? fallback : value === 'true';} catch (_) {return fallback;}}
function bindRememberedDisclosures() {
  for (const el of document.querySelectorAll('[data-persist-disclosure]')) {
    if (el.dataset.persistBound) continue;el.dataset.persistBound = 'true';
    el.addEventListener('toggle', () => {if (el.isConnected) try {localStorage.setItem('aieyes.disclosure.v3.' + (PANEL ? 'panel.' : 'detail.') + el.dataset.persistDisclosure, String(el.open));} catch (_) {}});
  }
}
function quotaNeedsAttention(q) {return typeof q?.error === 'string' && q.error.trim().length > 0;}
function quotaGroupFailures(provider) {
  const group = document.querySelector('.quota-group[data-provider="' + CSS.escape(provider) + '"]');
  const visible = new Set([...(group?.querySelectorAll('[data-quota]') ?? [])].map((node) => node.dataset.quota));
  const accounts = new Set((state.settings?.accounts ?? []).filter((a) => a.quotaEnabled && !a.archived).map((a) => a.provider + ':' + a.id));
  const quotas = new Map((state.dashboard?.quotas ?? []).map((q) => [q.provider + ':' + q.accountId, q]));
  return [...visible].filter((key) => accounts.has(key)).map((key) => quotas.get(key)).filter(quotaNeedsAttention);
}
function quotaFailureFocus(provider) {
  const button = document.querySelector('[data-quota-failures="' + CSS.escape(provider) + '"]');
  (button && !button.hidden ? button : document.querySelector('[data-persist-disclosure="quota-group:' + CSS.escape(provider) + '"] > summary'))?.focus({ preventScroll: true });
}
function closeQuotaFailures() {
  const pop = $('#quota-failures-popover');if (!pop?.matches(':popover-open')) return;
  const provider = pop.dataset.provider;pop.hidePopover();flushRender();quotaFailureFocus(provider);
}
function positionQuotaFailures() {
  const pop = $('#quota-failures-popover');if (!pop?.matches(':popover-open')) return;
  const body = $('.panel-body')?.getBoundingClientRect(),top = body?.top ?? 0,bottom = body?.bottom ?? innerHeight;
  const anchor = document.querySelector('[data-quota-failures="' + CSS.escape(pop.dataset.provider) + '"]')?.getBoundingClientRect();
  pop.style.maxHeight = Math.max(80, bottom - top - 16) + 'px';
  pop.style.left = Math.max(8, Math.min(innerWidth - pop.offsetWidth - 8, anchor?.left ?? 8)) + 'px';
  const below = (anchor?.bottom ?? top) + 6,above = (anchor?.top ?? bottom) - pop.offsetHeight - 6;
  pop.style.top = Math.max(top + 8, Math.min(bottom - pop.offsetHeight - 8, below + pop.offsetHeight <= bottom - 8 ? below : above)) + 'px';
}
function updateQuotaFailurePopover() {
  const pop = $('#quota-failures-popover');if (!pop?.matches(':popover-open')) return;
  const rows = quotaGroupFailures(pop.dataset.provider),list = pop.querySelector('[data-quota-failure-list]'),scroll = pop.scrollTop;
  const keys = new Set(rows.map((q) => q.provider + ':' + q.accountId));
  for (const row of list.querySelectorAll('[data-quota-failure-row]')) if (!keys.has(row.dataset.quotaFailureRow)) {
    if (row.contains(document.activeElement)) pop.querySelector('[data-quota-failure-close]').focus({ preventScroll: true });row.remove();
  }
  for (const q of rows) {
    const key = q.provider + ':' + q.accountId;
    let row = [...list.children].find((el) => el.dataset.quotaFailureRow === key);
    if (!row) {
      row = document.createElement('section');row.className = 'quota-failure-row';row.dataset.quotaFailureRow = key;
      row.innerHTML = '<strong></strong><div class="quota-failure-reason"></div><div class="actions"><button type="button" data-quota-failure-retry="' + escapeHTML(key) + AieyesI18n.t("\">重试查询</button><button type=\"button\" data-quota-failure-settings=\"") + escapeHTML(key) + AieyesI18n.t("\">账户设置</button></div>");
      row.querySelector('[data-quota-failure-retry]').onclick = () => {void quotas(q.accountId);};
      row.querySelector('[data-quota-failure-settings]').onclick = () => {closeQuotaFailures();void openSettings({ provider: q.provider, accountKey: key, section: 'accounts' });};
      list.append(row);
    }
    row.querySelector('strong').textContent = q.name;
    const reason = AieyesUI.errorPresentation(q.error, AieyesI18n.t("额度查询失败")),reasonNode = row.querySelector('.quota-failure-reason'),text = reason.details || reason.summary;
    if (reasonNode.textContent !== text) reasonNode.textContent = text;
    const retry = row.querySelector('[data-quota-failure-retry]');retry.disabled = quotaCollectionBlocked();retry.textContent = retry.disabled ? AieyesI18n.t("查询中…") : AieyesI18n.t("重试查询");
  }
  pop.querySelector('[data-quota-failure-empty]').hidden = rows.length > 0;
  pop.scrollTop = scroll;positionQuotaFailures();
}
function openQuotaFailures(provider) {
  let pop = $('#quota-failures-popover');
  if (pop?.matches(':popover-open') && pop.dataset.provider === provider) {closeQuotaFailures();return;}
  if (!pop) {
    pop = document.createElement('div');pop.id = 'quota-failures-popover';pop.setAttribute('popover', 'auto');pop.setAttribute('role', 'dialog');pop.setAttribute('aria-labelledby', 'quota-failures-title');
    pop.innerHTML = AieyesI18n.t("<div class=\"dialog-head\"><strong id=\"quota-failures-title\"></strong><button type=\"button\" data-quota-failure-close aria-label=\"关闭查询失败列表\">") + uiIcon('x') + AieyesI18n.t("</button></div><div data-quota-failure-list></div><p data-quota-failure-empty role=\"status\" hidden>当前没有查询失败的账户</p>");
    pop.querySelector('[data-quota-failure-close]').onclick = closeQuotaFailures;
    pop.addEventListener('toggle', () => {for (const button of document.querySelectorAll('[data-quota-failures]')) button.setAttribute('aria-expanded', String(pop.matches(':popover-open') && button.dataset.quotaFailures === pop.dataset.provider));if (!pop.matches(':popover-open')) flushRender();});
    document.body.append(pop);
  }
  pop.dataset.provider = provider;pop.querySelector('#quota-failures-title').textContent = (providers[provider] ?? provider) + AieyesI18n.t(" · 额度查询失败");
  pop.showPopover();updateQuotaFailurePopover();pop.querySelector('[data-quota-failure-close]').focus({ preventScroll: true });
}
document.addEventListener('click', (event) => {const button = event.target.closest('[data-quota-failures]');if (button) {event.preventDefault();openQuotaFailures(button.dataset.quotaFailures);}});
document.addEventListener('keydown', (event) => {
  const button = event.target.closest('[data-quota-failures]');
  if (button && (event.key === 'Enter' || event.key === ' ')) {event.preventDefault();event.stopImmediatePropagation();if (!event.repeat) openQuotaFailures(button.dataset.quotaFailures);return;}
  if (event.key === 'Escape' && $('#quota-failures-popover')?.matches(':popover-open')) {event.preventDefault();event.stopImmediatePropagation();closeQuotaFailures();}
}, true);
window.addEventListener('resize', positionQuotaFailures);
document.addEventListener('scroll', positionQuotaFailures, true);
function updateQuotaGroupAttention() {
  for (const group of document.querySelectorAll('.quota-group')) {
    const count = quotaGroupFailures(group.dataset.provider).length;
    const badge = group.querySelector('.group-attention');if (!badge) continue;
    badge.hidden = !count;badge.querySelector('[data-quota-attention]').textContent = count + AieyesI18n.t(" 个账户查询失败");
    if (!count && document.activeElement === badge) quotaFailureFocus(group.dataset.provider);
  }
  updateQuotaFailurePopover();
}
function quotaSection(d) {
  const selected = PANEL ? AieyesUI.panelAccounts(state.settings, d.quotaOrder ?? []) : null;
  const visible = (a) => (!state.provider || AieyesUI.sameAgent(a.provider, state.provider)) && (!state.accountKey || state.accountKey === a.provider + ':' + (a.accountId ?? a.id)) && (!PANEL || selected[a.provider]?.includes(a.provider + ':' + (a.accountId ?? a.id)));
  const accounts = state.settings.accounts.filter((a) => !a.archived && a.quotaEnabled && AieyesUI.supports(a.provider, 'quota', state.settings) && visible(a));
  const order = d.quotaOrder ?? [];accounts.sort((a, b) => (order.indexOf(a.provider + ':' + a.id) < 0 ? Infinity : order.indexOf(a.provider + ':' + a.id)) - (order.indexOf(b.provider + ':' + b.id) < 0 ? Infinity : order.indexOf(b.provider + ':' + b.id)));
  if (!accounts.length) return `<div class="card empty-quota"><h2>${state.provider ? AieyesI18n.t("此 Agent 暂无显示账户") : AieyesI18n.t("尚未显示账户")}</h2><p class="muted">${PANEL ? AieyesI18n.t("可在面板更多菜单中选择账户。") : AieyesI18n.t("添加账户后查看额度与余额。")}</p><button data-add-quota>${AieyesI18n.t("接入账户")}</button></div>`;
  const groups = [...new Set(accounts.map((a) => AieyesUI.agentProvider(a.provider)))];
  return `<div class="section-head quota-heading"><h2>${AieyesI18n.t("实时账户额度 {0} ", refreshIndicator('quotas'))}<span class="count">${accounts.length}</span></h2><div class="section-actions"><button id="order-quotas" title="${AieyesI18n.t("调整账户顺序")}" aria-label="${AieyesI18n.t("调整账户顺序")}">${uiIcon('sort')}</button><button id="read-quotas" ${state.quotaBusy || state.refreshState.quotas?.busy ? 'disabled' : ''}>${state.quotaBusy ? AieyesI18n.t("读取中…") : AieyesI18n.t("刷新额度")}</button></div></div><div class="quota-groups">` + groups.map((provider) => {
    const rows = accounts.filter((a) => AieyesUI.sameAgent(a.provider, provider)),quotas = rows.map((a) => (d.quotas ?? []).find((q) => q.provider === a.provider && q.accountId === a.id));
    const attention = quotas.filter(quotaNeedsAttention).length;
    return `<details class="quota-group" data-single-account="${rows.length === 1}" data-provider="${escapeHTML(provider)}" data-persist-disclosure="quota-group:${provider}" ${disclosureOpen('quota-group:' + provider) ? 'open' : ''}><summary class="quota-group-heading"><span class="category-dot"></span><strong>${escapeHTML(providers[provider])}</strong><span class="muted">${AieyesI18n.t("{0} 个账户", rows.length)}</span><button type="button" class="group-attention" data-quota-failures="${escapeHTML(provider)}" aria-haspopup="dialog" aria-expanded="false" aria-controls="quota-failures-popover" title="${AieyesI18n.t("查看失败原因与重试")}" ${attention ? '' : 'hidden'}>${uiIcon('warning')}<span data-quota-attention>${AieyesI18n.t("{0} 个账户查询失败", attention)}</span></button></summary><div class="quotas">${rows.map((a, i) => quotas[i] ? quotaCard(quotas[i], accounts.length) : `<div class="card quota-placeholder" data-provider="${escapeHTML(provider)}" data-quota-placeholder="${escapeHTML(a.provider + ':' + a.id)}"><h3>${providerMark(provider)} ${escapeHTML(a.name)}</h3><p class="muted" role="status">${state.quotaBusy ? AieyesI18n.t("正在读取账户额度…") : state.quotaError ? AieyesI18n.t("读取失败，可重试刷新额度") : AieyesI18n.t("尚未读取额度，可点击刷新额度")}</p></div>`).join('')}</div></details>`;
  }).join('') + '</div>';
}
async function job(label, fn, refreshKey = null, refreshItem = null) {
  const quotaJob = refreshKey === 'quotas';
  if (state.settingsSaving || (quotaJob ? state.refreshState.quotas?.busy : state.busy || ['scan', 'rescan', 'prices'].some((key) => state.refreshState[key]?.busy))) return false;
  const jobFocus = focusSelector(document.activeElement);
  if (!quotaJob) state.busy = true;if (refreshKey) beginRefresh(refreshKey);else updateActivity();
  const buttons = [...document.querySelectorAll('header [data-refresh]')].map((button) => [button, button.disabled]);
  buttons.forEach(([button]) => {if (quotaJob ? button.dataset.refresh === 'quotas' : button.dataset.refresh !== 'quotas') button.disabled = true;});
  let outcome = 'error',failure = null;const previousMessage = $('#message').textContent;
  try {const result = await fn();if (result === false) {outcome = 'noop';return true;}if (!refreshKey) state.saveError = '';outcome = 'success';state.lastSuccessfulUpdate = Date.now();if ($('#message').dataset.kind === 'error' && $('#message').textContent === previousMessage) notify('');return true;}
  catch (error) {failure = error;outcome = error.partial ? 'partial' : 'error';if (!refreshKey) {state.saveError = error.message ?? String(error);notify(state.saveError, 'error');document.querySelectorAll('.saved-feedback').forEach((el) => {AieyesUI.showError(el, state.saveError, AieyesI18n.t("保存失败，输入已保留"));el.dataset.status = 'error';});}return false;} finally
  {
    if (!quotaJob) state.busy = false;$('#activity').dataset.status = outcome;
    if (refreshKey && outcome === 'noop') {state.refreshState[refreshKey].busy = false;updateActivity();} else if (refreshKey) endRefresh(refreshKey, failure, refreshItem);else updateActivity();
    buttons.forEach(([button, disabled]) => {if (button.isConnected) button.disabled = disabled;});updateActivity();updateSettingsSaveBar();if (jobFocus && document.activeElement === document.body) {const target = document.querySelector(jobFocus);if (target && !target.disabled) target.focus({ preventScroll: true });else if (jobFocus === '#settings-save-all') $('#settings-save-bar .settings-draft-label')?.focus({ preventScroll: true });}
  }
}
const filterKeys = ['provider', 'accountKey', 'sourceId', 'model', 'days', 'dateRange'];
const filterSnapshot = () => structuredClone(Object.fromEntries(filterKeys.map((k) => [k, state[k]])));
function appliedScope() {const f = state.appliedFilters ?? filterSnapshot();return [rangeLabel(f), providers[f.provider] ?? AieyesI18n.t("全部 Agent"), historicalAccounts().find((a) => a.provider + ':' + a.id === f.accountKey)?.name ?? (f.accountKey === 'none' ? AieyesI18n.t("未关联账户") : AieyesI18n.t("全部账户")), state.settings.sources.find((s) => s.id === f.sourceId)?.name ?? AieyesI18n.t("全部来源"), f.model || AieyesI18n.t("全部模型")].join(' · ');}
async function changeFilters(values = {}) {Object.assign(state, values);Object.assign(state, AieyesViewState.normalize(state, dashboardScope()));try {viewPreferences.save(dashboardScope(), state);savePanelView();await loadDashboard({ debounce: true });} catch (_) {}}
function renderQueryStatus() {
  let el = $('#query-status');if (!el) {el = document.createElement('div');el.id = 'query-status';el.setAttribute('role', 'status');document.body.append(el);}
  el.hidden = !state.dashboardPending && !state.dashboardError || !el.closest('#refresh-status-popover')?.matches(':popover-open');updateHeadingIndicators();
  el.innerHTML = !state.dashboardPending && !state.dashboardError ? '' : '<p>' + escapeHTML(state.dashboardPending ? AieyesI18n.t("正在更新，仍显示上一结果：") + appliedScope() : AieyesI18n.t("更新失败，仍显示上一结果：") + appliedScope() + '。' + state.dashboardError) + '</p>' + (state.dashboardError ? AieyesI18n.t("<button id=\"query-retry\">重试</button><button id=\"query-restore\">恢复已应用筛选</button>") : '');
  if ($('#query-retry')) $('#query-retry').onclick = () => changeFilters();
  if ($('#query-restore')) $('#query-restore').onclick = () => changeFilters(state.appliedFilters);
}
let dashboardWork = false,dashboardQueued = false,dashboardReadyAt = 0,dashboardTimer,dashboardWaiters = [];
function loadDashboard({ debounce = false } = {}) {
  if (state.revisionEvents && (document.hidden || PANEL && !state.panelOpen)) {state.dashboardDirty = true;return Promise.resolve();}
  ++state.dashboardRequest;dashboardQueued = true;dashboardReadyAt = Date.now() + (debounce ? 120 : 0);
  state.dashboardPending = true;state.dashboardError = '';if (['agent', 'analysis'].includes(state.page)) renderQueryStatus();
  const result = new Promise((resolve, reject) => dashboardWaiters.push({ resolve, reject }));
  kickDashboard();return result;
}
function kickDashboard() {
  clearTimeout(dashboardTimer);if (dashboardWork) return;
  const delay = dashboardReadyAt - Date.now();
  if (delay > 0) {dashboardTimer = setTimeout(kickDashboard, delay);return;}
  if (!dashboardQueued) return;
  dashboardQueued = false;dashboardWork = true;const request = state.dashboardRequest;
  loadDashboardOnce(request).then(() => finishDashboard(null, request), (error) => finishDashboard(error, request));
}
function finishDashboard(error, request) {
  dashboardWork = false;
  if (dashboardQueued) {kickDashboard();return;}
  const waiters = dashboardWaiters;dashboardWaiters = [];
  for (const waiter of waiters) if (error && request === state.dashboardRequest) waiter.reject(error);else waiter.resolve();
}
async function loadDashboardOnce(request) {
  void window.AieyesAgentSettings?.ensureStatuses('codex');
  migrateProviderState();Object.assign(state, AieyesViewState.normalize(state, dashboardScope()));savePanelView();
  if (state.accountKey && state.accountKey !== 'none' && !filterAccounts().some((a) => a.provider + ':' + a.id === state.accountKey)) state.accountKey = '';
  if (state.sourceId && !state.settings.sources.some((s) => s.id === state.sourceId)) state.sourceId = '';
  const account = historicalAccounts().find((a) => a.provider + ':' + a.id === state.accountKey);
  const requested = filterSnapshot(),requestedScope = dashboardScope();state.dashboardPending = true;state.dashboardError = '';if (['agent', 'analysis'].includes(state.page)) renderQueryStatus();
  try {
    const params = { estimateHistory: 'display', provider: state.provider || null, sourceId: state.sourceId || null, model: state.model || null, days: state.days, dateRange: state.dateRange };
    const version = dashboardVersion(),usage = usageVersion(),key = JSON.stringify([usage, params]);
    const cacheable = state.revisionEvents && Object.keys(runtimeVersions).length > 0;
    let dashboard;
    if (cacheable && querySnapshots.has(key)) {
      dashboard = querySnapshots.get(key);
      if (snapshotStatusVersions.get(key) !== version) {
        const status = await api('dashboard.status', Object.fromEntries(['provider', 'accountId', 'sourceId'].map((k) => [k, params[k]])));
        dashboard = { ...dashboard, ...Object.fromEntries(['quotas', 'quotaOrder', 'quotaEstimates', 'creditEstimates', 'automaticEstimateSummaries', 'sources', 'priceUpdatedAt'].filter((k) => k in status).map((k) => [k, status[k]])) };
      }
    } else dashboard = await api('dashboard', params);
    if (cacheable && usage === usageVersion() && version === dashboardVersion() && JSON.stringify(dashboard).length <= 2 * 1024 * 1024) {if (querySnapshots.size >= 4 && !querySnapshots.has(key)) clearUsageSnapshots();querySnapshots.set(key, dashboard);snapshotStatusVersions.set(key, version);}
    lastDashboardVersion = version;
    if (request !== state.dashboardRequest) return;
    for (const name of [...new Set([...(dashboard.modelOptions ?? []), ...(dashboard.models ?? []).map((m) => m.key), ...(dashboard.dayModels ?? []).map((r) => r.model)])].sort()) for (const preset of window.AieyesChartPalettes) color(name, preset.id);
    dashboardCache.set(requestedScope, dashboard);appliedFilterCache.set(requestedScope, requested);state.dashboard = dashboard;state.appliedFilters = requested;state.dashboardPending = false;state.dashboardError = '';updateActivity();
    if (['agent', 'analysis'].includes(state.page)) {
      if (interactionOpen()) pendingRender = true;else render();
    }
    updateQuotaGroupAttention();refreshQuotaDialog();
  } catch (error) {if (request === state.dashboardRequest) {state.dashboardPending = false;state.dashboardError = error.message ?? String(error);if (['agent', 'analysis'].includes(state.page)) renderQueryStatus();}throw error;}
}
function batchError(rows, label) {
  if (!Array.isArray(rows)) return null;
  const failures = rows.map((row) => ({ ...row, error: AieyesUI.syncFailure(row) })).filter((row) => row.error);if (!failures.length) return null;
  const partial = failures.length < rows.length;
  const error = new Error(`${label}${partial ? AieyesI18n.t("部分失败") : AieyesI18n.t("失败")}：${failures.map((row) => `${row.name || row.id || row.accountId || ''}${row.name || row.id || row.accountId ? ' · ' : ''}${row.error}`).join('；')}`);
  error.partial = partial;error.failures = failures;return error;
}
async function scan(sourceId) {
  if (state.estimateBusy || state.sharedEstimateOperations?.size) return;
  await job(AieyesI18n.t("同步记录"), async () => {try {const rows = await api('sources.scan', typeof sourceId === 'string' ? { sourceId } : {});await loadDashboard();const error = batchError(rows, AieyesI18n.t("同步记录"));if (error) throw error;} finally {state.lastScan = Date.now();}}, 'scan', typeof sourceId === 'string' ? sourceId : null);
}
function updateRescanStatus() {
  const button = $('#history-rescan'),status = $('#history-rescan-status');
  if (button) {button.disabled = collectionBlocked();button.textContent = state.rescanning ? AieyesI18n.t("全量重扫中…") : AieyesI18n.t("全量重扫");}
  if (status) {status.textContent = state.rescanning ? AieyesI18n.t("正在重新读取完整日志…") : state.rescanMessage ?? '';status.hidden = !status.textContent;status.classList.toggle('error', Boolean(state.rescanFailed));}
}
async function rescan(sourceId) {
  if (collectionBlocked()) return;
  state.rescanning = true;state.rescanMessage = '';state.rescanFailed = false;
  try {
    await job(AieyesI18n.t("全量重扫"), async () => {
      try {
        const rows = await api('sources.rescan', typeof sourceId === 'string' ? { sourceId } : {});
        await loadDashboard();
        const error = batchError(rows, AieyesI18n.t("全量重扫"));if (error) throw error;
        const unknown = rows.reduce((sum, row) => sum + (row.issues ?? []).filter((issue) => issue.code === 'unknownModel').length, 0);
        const warning = unknown ? AieyesI18n.t(" {0} 条调用的模型身份待确认，Token 已保留、暂不计价。", unknown) : '';
        state.rescanMessage = rows.length ? AieyesI18n.t("全量重扫完成，已更新历史统计（{0} 个数据源）。", rows.length) + warning : AieyesI18n.t("没有可重扫的已启用数据源。");
        return rows.length ? undefined : false;
      } catch (error) {state.rescanFailed = true;state.rescanMessage = error.message ?? String(error);throw error;} finally
      {state.lastScan = Date.now();}
    }, 'rescan', typeof sourceId === 'string' ? sourceId : null);
  } finally {state.rescanning = false;updateRescanStatus();}
}
function hasQuotaSources() {
  return state.settings.accounts.some((a) => !a.archived && a.quotaEnabled && state.settings.sources.some((s) => AieyesUI.sourceEnabled(state.settings, s) && s.provider === a.provider && (s.accountId === a.id || s.codexHomeId && window.AieyesAgentSettings.profiles(a).some((r) => r.startsWith(s.codexHomeId + ':')))));
}
async function quotas(accountId, dueOnly = false) {
  if (quotaCollectionBlocked()) return;
  state.quotaBusy = true;if (!dueOnly) state.quotaError = '';if (!dueOnly && state.page === 'agent') {if (interactionOpen()) pendingRender = true;else render();}updateQuotaGroupAttention();
  try {await job(AieyesI18n.t("读取额度"), async () => {try {const rows = await api('quotas.refresh', { ...(typeof accountId === 'string' ? { accountId } : {}), dueOnly });applyQuotaSamples(rows);if (dueOnly && !rows.length) return false;if (rows.some((q) => q.identity && state.settings.accounts.some((a) => a.id === q.accountId && a.provider === q.provider && a.identityKey !== q.identity.key))) await window.AieyesAgentSettings?.syncIdentitySettings();await loadDashboard();const error = batchError(rows, AieyesI18n.t("读取额度"));if (error) throw error;} catch (error) {state.quotaError = error.failures ? AieyesI18n.t("部分账户读取失败，请逐项重试。") : error.message ?? String(error);throw error;} finally {state.lastQuota = Date.now();}}, 'quotas', typeof accountId === 'string' ? accountId : null);} finally
  {state.quotaBusy = false;scheduleRefreshes();if (!dueOnly && state.page === 'agent') {if (interactionOpen()) pendingRender = true;else render();}updateQuotaGroupAttention();}
}
async function referencePrices(model = null, name = '') {
  await job(AieyesI18n.t("智能引用价格"), async () => {
    await saveAllSettings({ refresh: false, section: 'prices' });
    const result = await api('prices.reference', model ? { model, name } : {});
    window.AieyesAgentSettings.accept(result.settings);
    await loadDashboard();if (state.page === 'settings') renderSettings(false);
    notify(AieyesI18n.t("已智能引用 {0} 个模型{1}{2}。按参考价格估值，历史成本可点“保存并重算”更新。", result.matched, result.unmatched.length ? '；' + result.unmatched.length + AieyesI18n.t(" 个未匹配") : '', result.skipped ? '；' + result.skipped + AieyesI18n.t(" 个已有价格或映射") : ''));
  });
}
async function syncPrices() {await job(AieyesI18n.t("同步价格"), async () => {await api('prices.sync');state.prices = await api('prices.list');await loadDashboard();if (state.page === 'settings') renderSettings();notify(AieyesI18n.t("价格已更新"));}, 'prices');}
function applyQuotaSamples(rows) {
  if (!state.dashboard || !Array.isArray(rows)) return;
  const changed = [];
  for (let row of rows) {
    const key = row.provider + ':' + row.accountId;
    if (!state.settings?.accounts.some((a) => a.provider + ':' + a.id === key && !a.archived)) continue;
    const index = state.dashboard.quotas.findIndex((q) => q.provider + ':' + q.accountId === key),old = state.dashboard.quotas[index];
    if (row.error && old) row = { ...old, error: row.error };else if (old?.updatedAt > row.updatedAt) continue;
    if (JSON.stringify(row) === JSON.stringify(old)) continue;
    if (index >= 0) state.dashboard.quotas[index] = row;else if ((!state.provider || AieyesUI.sameAgent(state.provider, row.provider)) && (!state.accountKey || state.accountKey === key)) state.dashboard.quotas.push(row);
    changed.push(row);
  }
  if (state.page === 'agent') {
    const view = rememberView();let replaced = false;
    for (const row of changed) {const key = CSS.escape(row.provider + ':' + row.accountId),node = document.querySelector('[data-quota="' + key + '"],[data-quota-placeholder="' + key + '"]');if (node) {node.outerHTML = quotaCard(row, state.dashboard.quotas.length);replaced = true;}}
    if (replaced) {updateQuotaGroupAttention();bindQuotaTools();restoreView(view);}
  }
}
let lastHostError = '';
const hostSampleVersions = new Map(),hostSampleValues = new Map();
let hostMonitorConfiguration = '',hostMonitorTask = null,hostMonitorRetryAt = 0,hostMonitorVersion = null,hostMonitorRequestBusy = false;
let hostMonitorBusy = new Set();
function hostRefreshBusy(id = null) {return state.hostMonitoring ? id ? hostMonitorBusy.has(id) : hostMonitorRequestBusy : Boolean(state.serverBusy || state.refreshState.hosts?.busy);}
function applyHostMonitor(snapshot) {
  if (!snapshot || !Array.isArray(snapshot.rows)) return;
  if (hostMonitorVersion?.session === snapshot.sampleSession && hostMonitorVersion.version >= snapshot.version) return;
  hostMonitorVersion = { session: snapshot.sampleSession, version: snapshot.version };
  hostMonitorBusy = new Set([...(snapshot.inFlight ?? []), ...(snapshot.pending ?? [])]);
  state.serverBusy = hostMonitorBusy.size > 0;
  const feedback = state.refreshState.hosts ??= {};feedback.busy = state.serverBusy;
  applyHostSamples(snapshot.rows);
  if (!state.serverBusy) {
    if (snapshot.rows.some((row) => row.sampleVersion)) feedback.success = Date.now();
    showFailures(batchError(snapshot.rows, AieyesI18n.t("刷新服务器")), 'hosts');
  }
  updateActivity();updateHostStatuses();
}
function configureHostMonitor() {
  if (PANEL || !state.hostMonitoring || !state.settings || refreshStopped || hostMonitorTask || Date.now() < hostMonitorRetryAt) return hostMonitorTask;
  const intervalSeconds = foregroundMetrics() ? state.settings.serverForegroundRefreshSeconds ?? 2 : state.settings.serverRefreshSeconds;
  const paused = Boolean(window.AieyesUpdates?.busy);
  const configuration = JSON.stringify([monitoredHosts(), intervalSeconds, paused]);
  if (configuration === hostMonitorConfiguration) return;
  hostMonitorTask = api('hosts.monitor.configure', { intervalSeconds, paused }).then((snapshot) => {
    hostMonitorConfiguration = configuration;hostMonitorRetryAt = 0;applyHostMonitor(snapshot);
  }).catch((error) => {hostMonitorRetryAt = Date.now() + 5000;notify(String(error), 'error');}).finally(() => {hostMonitorTask = null;scheduleRefreshes();});
  return hostMonitorTask;
}
function applyHostSamples(payload) {
  const rows = Array.isArray(payload) ? payload : payload?.rows ?? [],attemptedAt = Date.now();
  const byID = new Map(state.hosts.map((row) => [row.id, row]));let changed = false;
  for (const row of rows) {
    const old = hostSampleVersions.get(row.id),version = row.sampleVersion,session = row.sampleSession;
    if (session && old?.session === session && version <= old.version) continue;
    if (session) hostSampleVersions.set(row.id, { session, version });else
    {const signature = JSON.stringify(row);if (hostSampleValues.get(row.id) === signature) continue;hostSampleValues.set(row.id, signature);}
    const previous = byID.get(row.id);
    byID.set(row.id, { ...row, lastAttemptAt: row.lastAttemptAt ?? attemptedAt, ...(row.error ? { sample: row.sample ?? previous?.sample } : {}) });changed = true;
  }
  if (!changed) return;
  const allowed = new Set(monitoredHosts().map((h) => h.id));
  state.hosts = [...byID.values()].filter((row) => allowed.has(row.id));
  for (const key of hostSampleVersions.keys()) if (!allowed.has(key)) hostSampleVersions.delete(key);
  for (const key of hostSampleValues.keys()) if (!allowed.has(key)) hostSampleValues.delete(key);
  if (!rows.some((row) => row.error) && lastHostError) {if ($('#message').textContent === lastHostError) notify('');lastHostError = '';}
  state.lastMetrics = attemptedAt;updateActivity();updateHostStatuses();if (state.settings && state.page === 'servers') window.AieyesServers.request();
}
function applyHostFailure(error, hostId = null) {
  const message = error.message ?? String(error),attemptedAt = Date.now();
  const rows = monitoredHosts().map((host) => {
    const previous = state.hosts.find((row) => row.id === host.id) ?? { id: host.id };
    if (!host.enabled || hostId && host.id !== hostId) return previous;
    const { sampleSession, sampleVersion, ...retained } = previous;
    return { ...retained, error: message, lastAttemptAt: attemptedAt };
  });
  applyHostSamples(rows);lastHostError = message;
}
async function sample(hostId) {
  if (state.hostMonitoring) {
    if (window.AieyesUpdates?.busy || hostMonitorRequestBusy) return;
    hostMonitorRequestBusy = true;
    try {hostMonitorRetryAt = 0;await configureHostMonitor();applyHostMonitor(await api('hosts.monitor.refresh', typeof hostId === 'string' ? { hostId } : {}));}
    catch (error) {applyHostFailure(error, typeof hostId === 'string' ? hostId : null);showFailures(error, 'hosts', typeof hostId === 'string' ? hostId : null);} finally
    {hostMonitorRequestBusy = false;updateActivity();}
    return;
  }
  if (state.serverBusy || state.refreshState.hosts?.busy) return;state.serverBusy = true;beginRefresh('hosts');let failure = null;
  try {const payload = await api('hosts.sample', { stream: true, ...(typeof hostId === 'string' ? { hostId } : {}) }),rows = Array.isArray(payload) ? payload : payload.rows;applyHostSamples(rows);failure = batchError(rows, AieyesI18n.t("刷新服务器"));}
  catch (error) {failure = error;applyHostFailure(error, typeof hostId === 'string' ? hostId : null);} finally
  {state.lastMetrics = Date.now();state.serverBusy = false;endRefresh('hosts', failure, typeof hostId === 'string' ? hostId : null);}
}
async function saveSettings(next, { refresh = true, section = state.settingsTab, submittedForms = null } = {}) {
  if (state.settingsSaving) throw new Error(AieyesI18n.t("设置正在保存，请稍后重试"));
  state.settingsSaving = true;
  const previous = state.settings;
  try {
    const saved = await api('settings.patch', { base: previous, settings: next });
    window.AieyesAgentSettings.accept(saved, submittedForms);
    if (JSON.stringify([previous.accounts, previous.sources, previous.hosts, previous.proxy]) !== JSON.stringify([next.accounts, next.sources, next.hosts, next.proxy])) state.lastQuota = 0;
  } finally {state.settingsSaving = false;scheduleRefreshes();}
  markSaved(section);
  if (refresh && JSON.stringify([previous.accounts, previous.sources, previous.hosts, previous.modelMappings]) !== JSON.stringify([next.accounts, next.sources, next.hosts, next.modelMappings])) void loadDashboard().catch((error) => notify(AieyesI18n.t("设置已保存，概览刷新失败：") + String(error)));
}
const configKey = (value) => JSON.stringify(value, (_, v) => v && typeof v === 'object' && !Array.isArray(v) ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, v[k]])) : v);
// Rebase only edits made since the last saved configuration. Account/login
// updates can arrive while a different settings form still has a draft.
function mergeSettingsDraft(base, draft, saved) {
  if (configKey(draft) === configKey(base)) return structuredClone(saved);
  if (configKey(saved) === configKey(base)) return structuredClone(draft);
  const object = (value) => value && typeof value === 'object' && !Array.isArray(value);
  if (object(base) && object(draft) && object(saved)) {
    const merged = {};
    for (const key of new Set([...Object.keys(base), ...Object.keys(draft), ...Object.keys(saved)])) {
      const value = mergeSettingsDraft(base[key], draft[key], saved[key]);
      if (value !== undefined) merged[key] = value;
    }
    return merged;
  }
  if ([base, draft, saved].every(Array.isArray)) {
    const rows = [...base, ...draft, ...saved],identity = (row) => row?.id != null ? (row.provider ?? '') + ':' + row.id : row?.machineId ?? row?.provider;
    if (rows.length && rows.every((row) => object(row) && identity(row) != null)) {
      const previous = new Map(base.map((row) => [identity(row), row])),edits = new Map(draft.map((row) => [identity(row), row])),latest = new Map(saved.map((row) => [identity(row), row]));
      return [...new Set([...saved.map(identity), ...draft.map(identity)])].flatMap((key) => {
        const value = mergeSettingsDraft(previous.get(key), edits.get(key), latest.get(key));
        return value === undefined ? [] : [value];
      });
    }
  }
  return structuredClone(draft);
}
function acceptSettings(saved, submittedForms = null) {
  AieyesUI.observePanelAccounts(saved, state.dashboard?.quotaOrder ?? [], state.settings);
  captureSettingsForms();
  state.settingsDraft = mergeSettingsDraft(state.settings, state.settingsDraft ?? state.settings, saved);
  state.settings = saved;state.settingsBaseline = configKey(saved);
  rebaseGeneralForms(saved, new Map(submittedForms ?? []));
  if (state.page === 'setup') window.AieyesSetup?.render();
}
function draftSettings() {
  if (!state.settingsDraft) {state.settingsDraft = structuredClone(state.settings);state.settingsBaseline = configKey(state.settings);}
  return state.settingsDraft;
}
function settingsChangeCount() {
  captureSettingsForms();if (!state.settings) return 0;
  const draft = state.settingsDraft ?? state.settings,changed = new Set();
  for (const [list, key] of [['sources', 'id'], ['hosts', 'id'], ['accounts', 'id']]) {
    const identity = (row) => list === 'accounts' ? row.provider + ':' + row.id : row[key];
    const old = new Map(state.settings[list].map((row) => [identity(row), row])),next = new Map(draft[list].map((row) => [identity(row), row]));
    for (const id of new Set([...old.keys(), ...next.keys()])) if (JSON.stringify(old.get(id)) !== JSON.stringify(next.get(id))) changed.add(list + ':' + id);
  }
  const rest = (value) => Object.fromEntries(Object.entries(value).filter(([key]) => !['sources', 'hosts', 'accounts', 'modelMappings'].includes(key)));
  const general = [...settingsForms].filter(([key]) => key.startsWith('general-form:')).some(([, form]) => configKey(form.values) !== configKey(form.baseline));if (JSON.stringify(rest(draft)) !== JSON.stringify(rest(state.settings)) || general) changed.add('general');
  const mapping = settingsForms.get('mapping-form')?.values;
  if (JSON.stringify(draft.modelMappings) !== JSON.stringify(state.settings.modelMappings) || mapping?.mappingModel?.trim() || mapping?.mappingId?.trim()) changed.add('mappings');
  return changed.size;
}
function updateSettingsSaveBar() {
  const count = settingsChangeCount(),bar = $('#settings-save-bar');if (!bar) return;
  bar.dataset.dirty = String(count > 0);bar.dataset.status = state.saveError ? 'error' : state.settingsCommitting ? 'saving' : 'saved';bar.querySelector('.settings-draft-label').textContent = state.saveError ? AieyesI18n.t("保存失败：") + state.saveError : state.settingsCommitting ? AieyesI18n.t("正在保存…") : count ? AieyesI18n.t("当前表单有未保存输入") : state.savedAt ? AieyesI18n.t("已保存 · ") + timeLabel(state.savedAt) : '';
  if (state.saveError) AieyesUI.showError(bar.querySelector('.settings-draft-label'), state.saveError, AieyesI18n.t("保存失败，输入已保留"));
  bar.hidden = !bar.querySelector('.settings-draft-label').textContent;
  const retry = bar.querySelector('#settings-retry-save');if (retry) {retry.hidden = !state.saveError;retry.disabled = state.settingsCommitting;}
}
function collectGeneralDraft(next, forms) {
  if (!forms) {captureSettingsForms();forms = [...settingsForms].filter(([key]) => key.startsWith('general-form:'));}
  for (const [formKey, { values, baseline }] of forms) {
    const changed = (key) => Object.hasOwn(values, key) && values[key] !== baseline[key];
    const data = new FormData();for (const [key, value] of Object.entries(values)) data.set(key, value);
    for (const [field, key] of [['needsInputSound', 'needsInput'], ['completeSound', 'complete']]) if (changed(field)) next.sessionSounds = { needsInput: true, complete: true, ...next.sessionSounds, [key]: data.get(field) === 'true' };
    for (const [field, key] of [['unexpectedResetNotification', 'unexpected'], ['naturalResetNotification', 'natural']]) if (changed(field)) next.quotaResetNotifications = { unexpected: true, natural: false, ...next.quotaResetNotifications, [key]: data.get(field) === 'true' };
    if (changed('historyShowDeletedAccounts')) next.historyShowDeletedAccounts = data.get('historyShowDeletedAccounts') === 'true';
    if (changed('preciseCounting')) next.preciseCounting = data.get('preciseCounting') === 'true';
    for (const [field, key] of [['appearanceLanguage', 'language'], ['appearanceTheme', 'theme'], ['appearanceAccent', 'accent'], ['appearanceChartPalette', 'chartPalette'], ['appearanceChartContrast', 'chartContrast']]) if (changed(field)) next.appearance = { theme: 'system', accent: 'indigo', chartPalette: 'classic', chartContrast: 'high', ...next.appearance, [key]: values[field] };
    if (Object.hasOwn(values, 'appMode') && Object.keys(values).some((key) => key.startsWith('app') && changed(key))) next.proxy = proxyValue(data, 'app');
    if (changed('proxyTestUrls')) next.proxyTestUrls = values.proxyTestUrls.split(',').map((v) => v.trim()).filter(Boolean);
    for (const [key, min, label] of [['refreshSeconds', 5, AieyesI18n.t("额度查询")], ['historyRefreshSeconds', 10, AieyesI18n.t("Agent 记录")], ['serverForegroundRefreshSeconds', 2, AieyesI18n.t("服务器前台")], ['serverRefreshSeconds', 2, AieyesI18n.t("服务器后台")]]) {
      if (!changed(key)) continue;const value = Number(values[key]);
      if (!Number.isInteger(value) || value < min || value > 86400) {state.page = 'settings';state.settingsTab = 'general';state.generalTab = formKey.split(':')[1];renderSettings(false);const input = $('#field-' + key);input?.setAttribute('aria-invalid', 'true');input?.focus();const error = new Error(label + AieyesI18n.t(" 刷新间隔范围为 ") + min + AieyesI18n.t("–86400 秒，草稿已保留"));error.field = 'field-' + key;throw error;}
      $('#field-' + key)?.removeAttribute('aria-invalid');next[key] = value;
    }
  }
  return next;
}
async function saveAllSettings({ clearForms = true, refresh = true, section = state.settingsTab } = {}) {
  if (state.settingsCommitting) throw new Error(AieyesI18n.t("配置正在保存"));
  captureSettingsForms();
  const generalForms = structuredClone([...settingsForms].filter(([key]) => key.startsWith('general-form:')));
  const next = section === 'prices' ? mappingDraft(structuredClone(state.settings)) : section === 'general' ? collectGeneralDraft(structuredClone(state.settings), generalForms) : structuredClone(draftSettings());
  if (section === 'general' && configKey(next) === configKey(state.settings)) {state.saveError = '';settleGeneralForms(generalForms);updateSettingsSaveBar();return true;}
  state.settingsCommitting = true;updateSettingsSaveBar();
  try {
    await saveSettings(next, { refresh: false, section, submittedForms: section === 'general' ? generalForms : null });
    captureSettingsForms();
    if (section === 'general') settleGeneralForms(generalForms);else
    if (clearForms && section === 'prices') settingsForms.delete('mapping-form');
    if (refresh) void loadDashboard().catch((error) => notify(AieyesI18n.t("配置已保存，概览刷新失败：") + String(error), 'error'));
    if (state.page === 'settings' && section !== 'general') renderSettings(false);
    return true;
  } finally {state.settingsCommitting = false;scheduleRefreshes();updateSettingsSaveBar();}
}
function settleGeneralForms(submittedForms) {
  captureSettingsForms();
  rebaseGeneralForms(state.settings, new Map(submittedForms));
  updateChartPalettePreview();
}
function rebaseGeneralForms(saved, submitted = new Map()) {
  for (const [key, draft] of settingsForms) {
    if (!key.startsWith('general-form:')) continue;
    const template = document.createElement('template');template.innerHTML = generalSettingsHTML(saved, key.slice('general-form:'.length));
    const fresh = template.content.querySelector('#general-form');if (!fresh) continue;
    const baseline = AieyesUI.formValues(fresh),values = { ...baseline },before = submitted.get(key)?.values ?? draft.baseline;
    for (const [field, value] of Object.entries(draft.values)) if (value !== before[field]) values[field] = value;
    const dirty = configKey(values) !== configKey(baseline);
    if (dirty) settingsForms.set(key, { values, baseline });else settingsForms.delete(key);
    const form = $('#general-form');if (!form || settingsFormKey(form) !== key) continue;
    AieyesUI.restoreForm(form, values);form.dataset.draftBaseline = JSON.stringify(baseline);form.dataset.dirty = String(dirty);
    const label = form.querySelector('.draft-state');if (label) label.textContent = dirty ? AieyesI18n.t("未保存 · 切页保留草稿") : '';
  }
}
function discardAllSettings() {
  state.settingsDraft = null;state.settingsBaseline = null;settingsForms.clear();state.saveError = '';renderSettings(false);
}
function savedFeedback(section) {return `<span class="saved-feedback" data-save-section="${section}" role="status">${state.savedSection === section && state.savedAt ? AieyesI18n.t("已保存 · ") + timeLabel(state.savedAt) : ''}</span>`;}
function markSaved(section = state.settingsTab) {
  state.savedAt = Date.now();state.savedSection = section;if (section === 'general') state.saveError = '';
  const message = AieyesI18n.t("已保存 · ") + timeLabel(state.savedAt);
  document.querySelectorAll('.saved-feedback').forEach((el) => {if (el.dataset.saveSection === state.savedSection) {el.textContent = AieyesI18n.message(message);delete el.dataset.status;}});
  updateSettingsSaveBar();
}

function option(value, label, current) {return `<option value="${escapeHTML(value)}" ${String(value) === String(current) ? 'selected' : ''}>${escapeHTML(label)}</option>`;}
function stat(label, value, detail, icon) {
  icon = uiIcon({ '✧': 'sparkle', '▱': 'terminal', '↗': 'trend', '$': 'money' }[icon] ?? 'sparkle');
  return `<div class="stat"><div class="stat-label"><span>${label}</span><b>${icon}</b></div><div class="stat-value">${value}</div>${detail ? `<small>${detail}</small>` : ''}</div>`;}
function render() {
  window.AieyesMotion?.captureNavigation();
  captureSettingsForms();savePanelView();
  const nextKey = viewKey(),changing = renderedViewKey !== nextKey;
  document.querySelectorAll('[data-page]').forEach((b) => {const active = b.dataset.page === state.page;b.classList.toggle('active', active);if (active) b.setAttribute('aria-current', 'page');else b.removeAttribute('aria-current');});
  const title = $('#title');if (title) title.textContent = { agent: AieyesI18n.t("Agent 总览"), analysis: AieyesI18n.t("用量分析"), servers: AieyesI18n.t("服务器"), settings: AieyesI18n.t("设置"), setup: AieyesI18n.t("配置向导") }[state.page];
  for (const id of ['live-sessions']) {const el = $('#' + id);if (el) el.hidden = state.page !== 'agent';}
  updateActivity();if (!state.settings) return;
  window.AieyesTheme?.apply(state.settings.appearance);
  if (state.page === 'agent') renderOverview();else if (state.page === 'analysis') renderAnalysis();else if (state.page === 'servers') renderServers();else if (state.page === 'setup') window.AieyesSetup?.render();else renderSettings();
  window.AieyesSetup?.returnBar();syncNavigation();window.AieyesSessions?.visibility();
  renderedViewKey = nextKey;if (changing || state.restorePosition === nextKey) {restoreView(viewPreferences.position(nextKey));if (state.page === 'settings' || state.page === 'servers' || state.dashboard) state.restorePosition = null;}
  updateHeadingIndicators();
}
function attention(reason, action, id) {return `<div class="attention" role="status">${uiIcon('attention')}<span>${escapeHTML(AieyesI18n.message(reason))}</span><button id="${id}" type="button">${action}</button></div>`;}
function bindAttention() {if ($('#pricing-attention')) $('#pricing-attention').onclick = openPricing;if ($('#sampling-attention')) $('#sampling-attention').onclick = openSampling;}

function creditBalance(q, record, compact = false) {
  const balance = q.credits?.unlimited ? AieyesI18n.t("无限") : q.credits?.balance != null ? creditAmount(q.credits.balance) + ' credits' : q.credits?.hasCredits ? AieyesI18n.t("数量未知") : '—';
  const amount = Number(q.credits?.balance),value = record?.valuePer1000;
  const valid = !q.credits?.unlimited && q.credits?.balance != null && creditAmount(q.credits.balance) !== '—' && Number.isFinite(amount) && record?.status !== 'pending' && Number.isFinite(value) && Number.isFinite(amount * value / 1000);
  const full = balance + (valid ? ' ≈ ' + money(Math.round((amount * value / 1000 + Number.EPSILON * Math.abs(amount * value / 1000)) * 100) / 100) + ' USD' : '');
  return `<span class="credit-balance" title="${escapeHTML(AieyesI18n.t("余额 ") + full + ' · ' + (q.creditsUpdatedAt ? AieyesI18n.t("更新于 ") + date(q.creditsUpdatedAt) : AieyesI18n.t("尚未取得 credits 信息")))}"><span class="credit-label">${AieyesI18n.t("{0}余额", uiIcon('credit'))}</span> <strong>${compact ? balance.replace(/ credits$/, '') : full}</strong></span>`;
}
function creditRow(q, record, history = []) {
  const records = history.length ? history : record ? [record] : [],entry = AieyesUI.estimateEntries(records, true)[0];
  const title = entry ? `1000 credits ≈ ${money(entry.value)} USD · ${estimateStatus(entry.record)} · ${date(entry.record.checkpointAt)}` : AieyesI18n.t("开始采样以估算 credits 价值");
  return `<div class="credit-row">${creditBalance(q, record)}</div><button class="credit-estimate-entry estimate-entry" data-credit-estimate="${escapeHTML(q.provider + ':' + q.accountId)}" title="${escapeHTML(title)}">${uiIcon('trend')}<span>${escapeHTML(AieyesUI.creditEstimateLine(records))}</span>${uiIcon('right')}</button>${estimateNotice(history, true)}`;
}
function tokenSummary(summary, inline = false) {
  return `<div class="token-summary"><button type="button" id="token-summary" title="${AieyesI18n.t("总 Token：{0}；缓存读取命中率 = 缓存读取 ÷（普通输入＋缓存读取＋缓存写入），不包含输出 Token。点击查看明细。", summary.total.toLocaleString(AieyesI18n.locale))}" aria-expanded="false" aria-controls="token-popover" aria-label="${AieyesI18n.t("总 Token，点击查看详情")}">${inline ? 'Token <strong>' + compact(summary.total) + '</strong>' + uiIcon('chevron') : stat(AieyesI18n.t("总 Token"), compact(summary.total) + ' <span class="token-rate">(' + pct(cacheRate(summary.tokens)) + ')</span>', '', '✧')}${inline ? '' : AieyesI18n.t("<span class=\"token-hint\">点击查看详情</span>")}</button><div id="token-popover" popover="manual" role="region" aria-label="${AieyesI18n.t("Token 细分")}" hidden>${cacheCard(summary.tokens).replace('class="stat cache-card"', 'class="card cache-card"')}</div></div>`;
}
function closeTokenSummary() {
  const popover = $('#token-popover');if (popover) {if (popover.matches(':popover-open')) popover.hidePopover();popover.hidden = true;}
  $('#token-summary')?.setAttribute('aria-expanded', 'false');
}
function positionTokenSummary() {
  const popover = $('#token-popover'),button = $('#token-summary');if (!popover || popover.hidden || !button) return;
  const anchor = button.getBoundingClientRect(),area = $('.panel-body')?.getBoundingClientRect(),top = area?.top ?? 0,bottom = area?.bottom ?? innerHeight;
  popover.style.maxHeight = Math.max(120, bottom - top - 16) + 'px';
  popover.style.left = Math.max(8, Math.min(innerWidth - popover.offsetWidth - 8, anchor.left)) + 'px';
  popover.style.top = Math.max(top + 8, anchor.bottom + popover.offsetHeight + 6 <= bottom ? anchor.bottom + 6 : anchor.top - popover.offsetHeight - 6) + 'px';
}
function bindTokenSummary() {
  const button = $('#token-summary');if (!button) return;
  button.onmousedown = (event) => event.preventDefault();
  button.onclick = () => {const popover = $('#token-popover');if (!popover.hidden) {closeTokenSummary();return;}popover.hidden = false;popover.showPopover();positionTokenSummary();button.setAttribute('aria-expanded', 'true');};
}
document.addEventListener('scroll', positionTokenSummary, { capture: true, passive: true });
window.addEventListener('resize', positionTokenSummary);
document.addEventListener('pointerdown', (event) => {if (!event.target.closest('.token-summary')) closeTokenSummary();});
document.addEventListener('keydown', (event) => {if (event.key === 'Escape') closeTokenSummary();});

function cacheCard(t) {return `<div class="stat cache-card"><div class="token-breakdown">${[[AieyesI18n.t("普通输入"), t.input], [AieyesI18n.t("输出"), t.output], [AieyesI18n.t("缓存读取"), t.cacheRead], [AieyesI18n.t("缓存写入"), t.cacheWrite]].map(([label, value]) => `<div><span>${label}</span><strong title="${value.toLocaleString(AieyesI18n.locale)} Token">${compact(value)}</strong></div>`).join('')}<div class="cache-hit"><span title="${AieyesI18n.t("缓存读取 ÷（普通输入＋缓存读取＋缓存写入）")}">${AieyesI18n.t("读取命中率")}</span><strong>${pct(cacheRate(t))}</strong></div></div></div>`;}

const nativeCreditRows = (usage) => Object.entries(usage?.nativeCredits ?? {}).filter(([, row]) => row.requests > 0);
const hasNativeCredits = (usage) => nativeCreditRows(usage).length > 0;
const hasDisplayCredits = (usage) => nativeCreditRows(usage).some(([provider]) => provider !== 'qoder');
const creditCounter = (value) => Number(value).toLocaleString(AieyesI18n.locale, { maximumFractionDigits: 6 });
const equivalentCostAvailable = (usage) => usage.pricedTokens > 0 || !usage.total && !hasNativeCredits(usage);
function nativeCreditText(usage) {
  return nativeCreditRows(usage).filter(([provider]) => provider !== 'qoder').map(([provider, row]) => AieyesI18n.t("{0} 原生记录 {1} Credits · {2} {3}", providers[provider] ?? provider, creditCounter(row.credits), row.requests, provider === 'codebuddy' ? AieyesI18n.t("条计量") : AieyesI18n.t("次请求"))).join('；');
}
function nativeCreditDetail(row) {
  const parts = [['billable', 'billableRequests', AieyesI18n.t("标记可计费")], ['nonBillable', 'nonBillableRequests', AieyesI18n.t("标记不计费")], ['unknownBillability', 'unknownBillabilityRequests', AieyesI18n.t("计费状态未提供")]].filter(([, key]) => row[key] > 0).map(([key, count, label]) => AieyesI18n.t("{0} {1}（{2} 次）", label, creditCounter(row[key]), row[count]));
  if (row.originalCreditsRequests > 0) parts.push(AieyesI18n.t("折扣前已记录 {0}（{1}/{2} 次请求）", creditCounter(row.originalCredits), row.originalCreditsRequests, row.requests));
  return parts.join(' · ');
}
function nativeCreditsHTML(usage, models = [], inline = false) {
  usage = { ...usage, nativeCredits: Object.fromEntries(nativeCreditRows(usage).filter(([provider]) => provider !== 'qoder')) };
  if (!hasNativeCredits(usage)) return '';
  return `<div class="${inline ? 'native-credits' : 'card native-credits'}" data-native-credits><strong>${escapeHTML(nativeCreditText(usage))}</strong>${inline ? '' : nativeCreditRows(usage).map(([provider, row]) => `<p class="tiny muted">${escapeHTML(providers[provider] ?? provider)}：${escapeHTML(nativeCreditDetail(row))}</p>`).join('')}<p class="tiny muted">${AieyesI18n.t("来自原生计量记录；不代表已扣费金额或账户剩余额度。{0}", !usage.total ? AieyesI18n.t("原生记录没有可用的 Token 统计，API 等价成本暂不可计算。") : '')}</p>${!inline && models.some(hasDisplayCredits) ? `<details><summary>${AieyesI18n.t("按模型查看 Credits")}</summary>${models.filter(hasDisplayCredits).map((row) => `<p><span>${escapeHTML(modelName(row.key))}</span> · ${escapeHTML(nativeCreditText(row))}</p>`).join('')}</details>` : ''}</div>`;
}

const cacheRate = (t) => {const input = t.input + t.cacheRead + t.cacheWrite;return input ? t.cacheRead / input * 100 : null;};
const missingLabels = (gap) => [['input', AieyesI18n.t("输入")], ['output', AieyesI18n.t("输出")], ['cacheRead', AieyesI18n.t("缓存读取")], ['cacheWrite', AieyesI18n.t("缓存写入")]].filter(([k]) => gap.tokens[k] > 0).map(([, label]) => label).join('、');
function gapList() {
  const gaps = state.dashboard?.pricingGaps ?? [];
  return gaps.length ? `<div class="card pricing-gaps"><h2>${AieyesI18n.t("待计价模型")}</h2>${gaps.map((g, i) => `<div class="list-row"><div class="row-body"><strong>${escapeHTML(modelName(g.model))}</strong><small>${AieyesI18n.t("缺少{0} · {1} Token", missingLabels(g), compact(g.unpricedTokens))}</small></div><button data-gap-reference="${i}">${AieyesI18n.t("智能引用")}</button><button data-gap-map="${i}" title="${AieyesI18n.t("将日志模型名对应到 OpenRouter 模型 ID")}">${AieyesI18n.t("映射")}</button><button data-gap-price="${i}" title="${AieyesI18n.t("为缺失计价的模型补充价格")}">${AieyesI18n.t("补充价格")}</button></div>`).join('')}</div>` : '';
}
async function openPricing() {
  if (PANEL) {window.__TAURI__.core.invoke('desktop_action', { action: 'prices' }).catch((e) => notify(String(e)));return;}
  await openSettings({ category: 'prices' });
}
function rangeLabel(value = state) {return value.dateRange?.kind === 'all' ? AieyesI18n.t("所有") : value.dateRange?.kind === 'custom' ? value.dateRange.start + AieyesI18n.t(" 至 ") + value.dateRange.end : value.days === 1 ? AieyesI18n.t("今日") : AieyesI18n.t("最近 ") + value.days + AieyesI18n.t(" 天");}
function localISODate(date = new Date()) {return [date.getFullYear(), String(date.getMonth() + 1).padStart(2, '0'), String(date.getDate()).padStart(2, '0')].join('-');}
function rangeSelectHTML() {
  const selected = state.dateRange?.kind ?? state.days;
  return `<div class="range-controls"><select id="days" aria-label="${AieyesI18n.t("时间范围")}">${[1, 7, 30, 90, 365].map((n) => option(n, n === 1 ? AieyesI18n.t("今日") : n === 365 ? AieyesI18n.t("最近一年") : AieyesI18n.t("最近 {0} 天", n), selected)).join('')}${option('all', AieyesI18n.t("所有"), selected)}${option('custom', AieyesI18n.t("自定义日期"), selected)}</select>${state.dateRange?.kind === 'custom' ? `<div class="custom-date-controls"><label>${AieyesI18n.t("开始")}<input id="range-start" type="date" value="${escapeHTML(state.dateRange.start)}" aria-label="${AieyesI18n.t("开始日期")}"></label><label>${AieyesI18n.t("结束")}<input id="range-end" type="date" value="${escapeHTML(state.dateRange.end)}" aria-label="${AieyesI18n.t("结束日期")}"></label><button type="button" id="range-apply">${AieyesI18n.t("应用日期")}</button></div>` : ''}</div>`;
}
function bindRangeControl() {
  $('#days').onchange = async (event) => {
    const value = event.target.value;
    if (value === 'all') state.dateRange = { kind: 'all' };else
    if (value === 'custom') {const start = new Date();start.setDate(start.getDate() - 6);state.dateRange = { kind: 'custom', start: localISODate(start), end: localISODate() };} else
    {state.dateRange = null;state.days = Number(value);}
    await changeFilters();
  };
  if ($('#range-apply')) $('#range-apply').onclick = async () => {
    const start = $('#range-start'),end = $('#range-end');
    if (!start.value || !end.value || !start.checkValidity() || !end.checkValidity() || start.value > end.value) {notify(AieyesI18n.t("请输入有效日期，开始日期不能晚于结束日期。"), 'error');return;}
    await changeFilters({ dateRange: { kind: 'custom', start: start.value, end: end.value } });
  };
}
function renderAnalysis() {
  const d = state.dashboard;if (!d) {$('#content').innerHTML = AieyesI18n.t("<p class=\"muted\" role=\"status\">正在读取用量…</p>");return;}
  window.AieyesMotion?.captureNavigation();
  if (PANEL) {renderAgentPanel(d);return;}
  const view = rememberView();
  const s = d.summary,t = s.tokens,models = d.models;
  const account = historicalAccounts().find((a) => a.provider + ':' + a.id === state.accountKey);
  const sources = state.settings.sources.filter((src) => {const ids = d.sources.find((s) => s.id === src.id)?.accountIds ?? [src.accountId];return (!state.provider || AieyesUI.sameAgent(src.provider, state.provider)) && (!state.accountKey || (state.accountKey === 'none' ? ids.includes('') : src.provider === account?.provider && ids.includes(account?.id)));});
  const trendModels = [...new Set(d.dayModels.map((r) => r.model))].sort();
  $('#title').textContent = AieyesI18n.t("用量分析");
  $('#content').innerHTML = `${state.settings.sources.length ? '' : onboarding()}<div class="filters"><select id="provider" aria-label="Agent">${option('', AieyesI18n.t("全部 Agent"), state.provider)}${window.AieyesContract.agentProviders.map((k) => option(k, providers[k], state.provider)).join('')}</select><select id="source" aria-label="${AieyesI18n.t("数据源")}">${option('', AieyesI18n.t("全部数据源"), state.sourceId)}${sources.map((s) => option(s.id, s.name, state.sourceId)).join('')}</select><select id="model" aria-label="${AieyesI18n.t("模型")}">${option('', AieyesI18n.t("全部模型"), state.model)}${[...new Set([...(d.modelOptions ?? trendModels), ...(state.model ? [state.model] : [])])].map((m) => option(m, modelName(m), state.model)).join('')}</select></div>
  ${s.total > 0 && s.pricedTokens < s.total ? attention(AieyesI18n.t("计价未完成：部分 Token 缺少价格"), AieyesI18n.t("补充价格"), 'pricing-attention') : ''}
  <div class="section-head overview-heading"><h2>${!state.dateRange && state.days === 1 ? AieyesI18n.t("今日概览") : AieyesI18n.t("使用概览")} ${refreshIndicator('scan')}</h2>${rangeSelectHTML()}</div>
  <div class="stats">${stat(AieyesI18n.t("总 Token"), compact(s.total), '', '✧')}${stat(AieyesI18n.t("API 等价成本") + (s.total > 0 && s.pricedTokens < s.total ? AieyesI18n.t(" <button id=\"repair-pricing\" class=\"pricing-alert\" aria-label=\"计价未完成，设置模型价格\" title=\"部分 Token 尚未计价\">!</button>") : ''), equivalentCostAvailable(s) ? money(s.cost) : '—', '', '$')}</div>${cacheCard(t)}${nativeCreditsHTML(s, models)}
  <div class="section-head"><h2>${!state.dateRange && state.days === 1 ? AieyesI18n.t("近 7 天趋势") : AieyesI18n.t("使用趋势")}</h2><div id="cost-mode" class="metric-picker" role="group" aria-label="${AieyesI18n.t("统计指标")}"><button type="button" data-cost-mode="false" aria-pressed="${!state.cost}">Token</button><button type="button" data-cost-mode="true" aria-pressed="${state.cost}">${AieyesI18n.t("API 等价成本")}</button></div></div>
  <div class="chart-row"><div class="card"><h2>${AieyesI18n.t("每日用量 · 按模型")}</h2><canvas id="trend" aria-label="${AieyesI18n.t("按模型堆叠的每日用量，下方有逐日数据")}"></canvas><details class="model-legend"><summary>${AieyesI18n.t("模型图例 · 点击显示或隐藏")}</summary><div class="model-key">${trendModels.map((m) => `<button data-trend-model="${escapeHTML(m)}" aria-label="${AieyesI18n.t("显示或隐藏模型 {0}", escapeHTML(modelName(m)))}"><i style="background:${color(m)}"></i>${escapeHTML(modelName(m))}</button>`).join('')}</div></details></div><div class="card"><h2>${AieyesI18n.t("所选范围 · 模型分布")}</h2>${modelDistribution(models)}</div></div>
  ${samplingBanner(d)}<div class="card"><details class="daily" data-agent-detail="daily"><summary><h2>${AieyesI18n.t("每日明细 · 可选择与复制精确数值")}</h2></summary>${dailyTable(d.trendDays, d.dayModels)}</details></div>
  <div class="card"><h2>${state.dateRange ? AieyesI18n.t("所选日期用量") : AieyesI18n.t("过去 365 天")}</h2>${state.dateRange ? yearlyHeatmapHTML(d.heatmap) : heatmapHTML(d.heatmap)}</div>
  `;
  bindSetup();
  $('#provider').onchange = async (e) => {state.provider = e.target.value;state.accountKey = '';state.sourceId = '';state.model = '';await changeFilters();};
  $('#source').onchange = async (e) => {state.sourceId = e.target.value;await changeFilters();};
  $('#model').onchange = async (e) => {state.model = e.target.value;await changeFilters();};
  bindRangeControl();
  document.querySelectorAll('[data-cost-mode]').forEach((button) => button.onclick = () => {state.cost = button.dataset.costMode === 'true';viewPreferences.save(dashboardScope(), state);renderAnalysis();document.querySelector(`[data-cost-mode="${state.cost}"]`)?.focus({ preventScroll: true });});
  if ($('#read-quotas')) $('#read-quotas').onclick = () => quotas();
  bindQuotaTools();bindAttention();
  if ($('#repair-pricing')) $('#repair-pricing').onclick = openPricing;
  shortenEmptyUsage(d);restoreView(view);
  bindAnalysis();requestAnimationFrame(drawTrend);
  window.AieyesMotion?.didRender('analysis');
}
function renderOverview() {
  const d = state.dashboard;if (!d) {$('#content').innerHTML = AieyesI18n.t("<p class=\"muted\" role=\"status\">正在读取账户…</p>");return;}
  const view = rememberView(),s = d.summary;
  const enabledProviders = AieyesUI.overviewAgentProviders(state.settings);
  if (state.provider && !enabledProviders.includes(state.provider)) {void changeFilters({ provider: '', accountKey: '' });return;}
  const visibleProviders = enabledProviders.map((key) => [key, providers[key]]);
  const agentTabs = `<div class="agent-selector" role="group" aria-label="${AieyesI18n.t("选择 Agent")}"><button data-overview-agent="" aria-pressed="${!state.provider}">${AieyesI18n.t("总览")}</button>${visibleProviders.map(([key, label]) => `<button data-overview-agent="${key}" data-provider="${key}" aria-pressed="${key === state.provider}">${escapeHTML(label)}</button>`).join('')}</div>`;
  $('#content').innerHTML = `${state.settings.sources.length || state.settings.accounts.length ? '' : onboarding()}${agentTabs}<div class="filters overview-filters"><button id="open-detail" class="analysis-link">${AieyesI18n.t("查看分析 {0}", uiIcon('right'))}</button></div><div class="overview-usage card"><div class="between"><span class="muted">${AieyesI18n.t("今日用量 {0}", refreshIndicator('scan'))}</span><span>${s.total > 0 && s.pricedTokens < s.total ? AieyesI18n.t("<button type=\"button\" id=\"repair-pricing\" class=\"pricing-alert\" aria-label=\"计价未完成，设置模型价格\" title=\"计价未完成：部分模型缺少价格，点击设置模型价格\">!</button>") : ''}</span></div><div class="usage-inline">${tokenSummary(s, true)}<span>${AieyesI18n.t("API 等价成本 ")}<strong>${equivalentCostAvailable(s) ? money(s.cost) : '—'}</strong></span></div>${nativeCreditsHTML(s, [], true)}</div>${quotaSection(d)}${samplingBanner(d)}`;
  document.querySelectorAll('[data-overview-agent]').forEach((button) => button.onclick = () => changeFilters({ provider: button.dataset.overviewAgent, accountKey: '', sourceId: '', model: '' }));
  $('#open-detail').onclick = () => openAnalysis();
  if ($('#read-quotas')) $('#read-quotas').onclick = () => quotas();
  document.querySelectorAll('[data-add-quota]').forEach((b) => b.onclick = () => openSetup('quota'));
  if ($('#repair-pricing')) $('#repair-pricing').onclick = openPricing;
  bindQuotaTools();bindAttention();bindTokenSummary();bindSetup();restoreView(view);window.AieyesSessions?.visibility();
  window.AieyesMotion?.didRender('agent');
}
function renderAgentPanel() {renderOverview();}
function shortenEmptyUsage(d) {
  if (d.summary.total || hasNativeCredits(d.summary) || (d.trendDays ?? []).some((day) => day.total || hasNativeCredits(day)) || (d.heatmap ?? []).some((day) => day.total || hasNativeCredits(day))) return;
  $('#content .stats')?.remove();$('#content > .cache-card')?.remove();
  if (!state.settings.sources.length) {$('#content .chart-row')?.remove();$('#trend')?.closest('.card')?.remove();$('#content .daily')?.closest('.card')?.remove();$('#content .heatmap')?.closest('.card')?.remove();$('#cost-mode')?.closest('.section-head')?.remove();return;}
  const trend = $('#trend'),chart = trend?.closest('.chart-row') ?? trend?.closest('.card');
  if (chart) {
    const usage = AieyesUI.usageSources(state.settings),fresh = AieyesUI.freshness(state.settings, d),filtered = Boolean(state.provider || state.accountKey || state.sourceId || state.model),onlyQuota = state.settings.sources.every((s) => s.provider === 'deepseek');
    const title = onlyQuota ? AieyesI18n.t("此来源提供账户额度") : fresh.failed ? AieyesI18n.t("记录同步失败") : filtered ? AieyesI18n.t("没有匹配的用量记录") : !fresh.timestamp ? AieyesI18n.t("尚未同步用量记录") : AieyesI18n.t("此时间范围暂无用量");
    const description = onlyQuota ? AieyesI18n.t("连接账户查看额度与余额；添加日志目录后分析用量。") : fresh.failed ? AieyesI18n.t("请在对应来源查看错误并重试，已有记录仍保留。") : filtered ? AieyesI18n.t("尝试清除筛选或扩大时间范围。") : !fresh.timestamp ? AieyesI18n.t("来源已配置，首次同步后将显示统计。") : AieyesI18n.t("可以扩大日期范围查看历史，或同步最新日志。");
    const empty = document.createElement('div');empty.className = 'card empty-usage';empty.innerHTML = '<h2>' + title + '</h2><p class="muted">' + description + '</p><div class="actions">' + (onlyQuota ? AieyesI18n.t("<button id=\"empty-add-source\">添加日志来源</button>") : AieyesI18n.t("<button id=\"empty-sync\">同步记录</button><button id=\"empty-range\">扩大到最近一年</button>") + (filtered ? AieyesI18n.t("<button id=\"empty-clear\">清除筛选</button>") : '')) + '</div>';chart.replaceWith(empty);
    if ($('#empty-sync')) $('#empty-sync').onclick = () => scan();if ($('#empty-add-source')) $('#empty-add-source').onclick = () => openSetup();if ($('#empty-range')) $('#empty-range').onclick = () => {state.days = 365;state.dateRange = null;savePanelView();return PANEL ? openDetail() : changeFilters({ days: 365, dateRange: null });};if ($('#empty-clear')) $('#empty-clear').onclick = () => changeFilters({ provider: '', accountKey: '', sourceId: '', model: '' });
  }
  $('#cost-mode')?.closest('.section-head')?.remove();
  $('#content .daily')?.closest('.card')?.remove();
  $('#content .heatmap')?.closest('.card')?.remove();
}
let dailyState = null;
function dailyRowsHTML(days, byDay) {
  return days.map((day) => `<tbody data-agent-detail="day:${escapeHTML(day.key)}"><tr class="day"><th scope="row"><button data-day-toggle="${day.key}" aria-expanded="false" aria-label="${AieyesI18n.t("展开 {0} 的模型明细", day.key)}">${day.key}</button>${hasDisplayCredits(day) ? `<small class="muted">${escapeHTML(nativeCreditText(day))}</small>` : ''}</th><td title="${day.total} Token">${compact(day.total)}</td><td>${pct(cacheRate(day.tokens))}</td><td>${equivalentCostAvailable(day) ? day.cost.toFixed(6) : '—'}</td></tr>${(byDay.get(day.key) ?? []).map((r) => `<tr class="day-model" hidden><th scope="row"><span>${escapeHTML(modelName(r.model))}</span>${hasDisplayCredits(r.usage) ? `<small class="muted">${escapeHTML(nativeCreditText(r.usage))}</small>` : ''}</th><td title="${r.usage.total} Token">${compact(r.usage.total)}</td><td>${pct(cacheRate(r.usage.tokens))}</td><td>${equivalentCostAvailable(r.usage) ? r.usage.cost.toFixed(6) : '—'}</td></tr>`).join('')}</tbody>`).join('');
}
function dailyTable(days, rows) {
  const sorted = [...days].reverse(),byDay = new Map();for (const row of rows) {if (!byDay.has(row.day)) byDay.set(row.day, []);byDay.get(row.day).push(row);}
  const count = state.dateRange ? Math.min(31, sorted.length) : sorted.length;dailyState = { days: sorted, byDay, count };
  return `<div class="daily-table table-scroll"><table><thead><tr><th scope="col">${AieyesI18n.t("日期 / 模型")}</th><th scope="col">Token</th><th scope="col">${AieyesI18n.t("读取命中率")}</th><th scope="col">${AieyesI18n.t("已计价成本 USD")}</th></tr></thead>${dailyRowsHTML(sorted.slice(0, count), byDay)}</table>${count < sorted.length ? AieyesI18n.t("<button data-more-days>加载更多日期</button>") : ''}</div>`;
}
function bindMoreDays() {
  const button = document.querySelector('[data-more-days]');if (!button || !dailyState) return;const data = dailyState;
  button.onclick = () => {const next = Math.min(data.count + 31, data.days.length);button.previousElementSibling.insertAdjacentHTML('beforeend', dailyRowsHTML(data.days.slice(data.count, next), data.byDay));data.count = next;if (next === data.days.length) button.remove();bindDailyToggles();};
}


function estimateNotice(records, credits = false, q = null, group = '') {
  const latest = AieyesUI.canonicalEstimates(records)[0],entries = AieyesUI.estimateEntries(records, credits, q, group),notes = [];
  if (latest?.samplingMode === 'rolling') notes.push(AieyesI18n.t("滚动估值 · ") + latest.sampleCount + AieyesI18n.t(" 个有效片段"));
  if (entries.some((e) => e.historical)) notes.push(AieyesI18n.t("历史采样"));
  if (entries.some((e) => e.record.originalEstimateId)) notes.push(AieyesI18n.t("已修正"));
  if (latest && (!(credits ? [latest.valuePer1000] : [latest.fiveHourValue, latest.weeklyValue, latest.weeklyRatioValue]).some(Number.isFinite) || latest.status === 'pending')) notes.push(AieyesI18n.t("最新采样 · ") + estimateStatus(latest) + ' · ' + AieyesUI.estimateIssue(latest));
  return notes.length ? `<div class="estimate-notice muted">${escapeHTML(notes.join(' · '))}</div>` : '';
}
function estimateEntry(q, records, group = '') {
  const entries = AieyesUI.estimateEntries(records, false, q, group),latest = AieyesUI.canonicalEstimates(records)[0];
  const title = entries.map((e) => `${e.label} ≈ ${money(e.value)} USD${e.historical ? AieyesI18n.t(" · 历史采样") : ''} · ${date(e.record.checkpointAt)}`).join('；') || AieyesI18n.t("开始采样以估算额度价值");
  const label = AieyesUI.estimateLine(records, q, group);
  return `<button class="estimate-entry quota-estimate-entry${latest ? ' sampling-entry' : ''}" data-estimate="${escapeHTML(q.provider + ':' + q.accountId)}" data-estimate-group="${escapeHTML(group)}" title="${escapeHTML(AieyesI18n.message(title + '；' + (latest?.samplingMode === 'rolling' ? latest.calculationNote + '；' : '') + AieyesUI.estimateHelp(q, records, group)))}">${uiIcon('trend')}<span class="estimate-inline">${escapeHTML(label)}</span>${uiIcon('right')}</button>${estimateNotice(records, false, q, group)}`;
}
function extraUsageHTML(wallet) {
  if (!wallet) return '';
  const amount = (value) => value == null ? AieyesI18n.t("未提供") : wallet.currency + ' ' + value;
  const limit = wallet.monthlyLimitEnabled === true ? AieyesI18n.t("月度上限 ") + amount(wallet.monthlyLimit) : wallet.monthlyLimitEnabled === false ? AieyesI18n.t("月度上限未启用") : AieyesI18n.t("月度上限状态未提供");
  return `<div class="quota-window extra-usage-wallet" data-extra-usage><div class="between"><span>${AieyesI18n.t("额外用量余额")}</span><strong>${escapeHTML(amount(wallet.balance))}</strong></div><div class="between tiny muted balance-detail"><span>${AieyesI18n.t("钱包总额")}</span><span>${escapeHTML(amount(wallet.total))}</span></div><div class="between tiny muted balance-detail"><span>${AieyesI18n.t("本月扣费")}</span><span>${escapeHTML(amount(wallet.monthlyUsed))}</span></div><div class="tiny muted balance-detail">${escapeHTML(limit)}</div><div class="tiny muted balance-detail">${AieyesI18n.t("钱包与订阅额度分别计算")}</div></div>`;
}
function quotaCard(q, visibleAccounts = 1) {
  const key = q.provider + ':' + q.accountId,records = [...(state.dashboard?.quotaEstimates ?? []), ...(state.dashboard?.automaticEstimateSummaries ?? []).filter((e) => e.kind === 'quota')].filter((e) => e.accountKey === key),credits = [...(state.dashboard?.creditEstimates ?? []), ...(state.dashboard?.automaticEstimateSummaries ?? []).filter((e) => e.kind === 'credits')].filter((e) => e.accountKey === key),latest = AieyesUI.canonicalEstimates(records)[0],credit = AieyesUI.estimateEntries(credits, true)[0]?.record ?? AieyesUI.canonicalEstimates(credits)[0];
  const windowHTML = (w) => `<div class="quota-window"><div class="between tiny"><span>${escapeHTML(AieyesI18n.message(w.name))}</span><strong style="color:${quotaColor(Number.isFinite(w.usedPercent) ? resourcePercent(100 - w.usedPercent) : null)}">${AieyesI18n.t("剩余 {0}", pct(Number.isFinite(w.usedPercent) ? resourcePercent(100 - w.usedPercent) : null))}</strong></div>${resourceBar(Number.isFinite(w.usedPercent) ? 100 - w.usedPercent : null, AieyesI18n.message(w.name) + AieyesI18n.t("剩余额度"), true)}<div class="tiny muted quota-reset" data-reset-at="${w.resetsAt ?? 0}" data-window-minutes="${w.windowMinutes ?? ''}" data-used-percent="${Number.isFinite(w.usedPercent) ? w.usedPercent : ''}">${AieyesUI.quotaReset(w)}</div></div>`;
  const collapsible = true;
  let expanded = false;try {const stored = localStorage.getItem('quota.expanded.v2.' + (PANEL ? 'panel.' : 'detail.') + key);if (collapsible && stored != null) expanded = stored === 'true';} catch (_) {}

  let windows = '<div class="quota-windows">' + q.windows.map(windowHTML).join('') + '</div>';
  if (q.provider === 'antigravity') {
    const groups = new Map();for (const w of q.windows) {const group = w.groupName || w.name.split(' · ').slice(0, -1).join(' · ');if (!groups.has(group)) groups.set(group, []);groups.get(group).push(w);}
    windows = `<div class="agy-groups">${[...groups].map(([group, rows]) => `<div class="agy-group"><strong>${escapeHTML(group)}</strong><div class="agy-windows">${rows.sort((a, b) => (a.windowMinutes ?? 0) - (b.windowMinutes ?? 0)).map((w) => windowHTML({ ...w, name: w.windowMinutes === window.AieyesContract.windows.week ? '7d' : w.windowMinutes === window.AieyesContract.windows.fiveHour ? '5h' : w.name })).join('')}</div>${estimateEntry(q, records.filter((e) => e.groupId === (rows[0]?.groupId || group)), rows[0]?.groupId || group)}</div>`).join('')}</div>`;
  }
  return `<div class="card quota-card" data-provider="${escapeHTML(q.provider)}" data-quota="${escapeHTML(key)}"><details class="quota-disclosure" data-collapsible="${collapsible}" data-agent-detail="quota:${escapeHTML(key)}" ${expanded ? 'open' : ''}><summary class="quota-title"><h3>${providerMark(q.provider)}<span class="quota-heading-label"><span class="quota-name" title="${escapeHTML(q.name)}">${escapeHTML(q.name)}</span>${q.identity?.email && q.identity.email !== q.name ? `<small class="quota-email" title="${escapeHTML(q.identity.email)}">${escapeHTML(q.identity.email)}</small>` : ''}</span></h3>${q.plan ? `<span class="quota-subscription">${subscriptionBadge(q.plan)}</span>` : ''}${collapsible ? `<span class="quota-chevron" aria-hidden="true">${uiIcon('chevron')}</span>` : ''}${q.provider === 'codex' ? `<span class="quota-collapsed-summary">${creditBalance(q, credit, true)}<span>Bank Reset <strong>${q.bankReset?.availableCount ?? '—'}</strong>${AieyesI18n.t(" 次可用")}</span></span>` : ''}</summary></details><div class="quota-content">${window.AieyesAccountTargets?.eligible(key) ? `<div class="account-switch-row"><button class="account-switch-entry" data-switch-account="${escapeHTML(key)}">${AieyesI18n.t("{0}切换为此账户", uiIcon('switch'))}</button><span class="account-device-counts" data-device-counts="${escapeHTML(key)}">${window.AieyesAgentSettings?.countsHTML(key) ?? ''}</span></div>` : q.provider === 'codex' ? AieyesI18n.t("<div class=\"account-switch-row\"><span class=\"muted tiny\">暂无可用机器</span><button data-account-machine-settings>机器设置</button></div>") : ''}<div class="between tiny muted quota-meta"><span>${escapeHTML(providers[q.provider] ?? q.provider)}</span><span>${q.origin === 'log' ? AieyesI18n.t("记录") : AieyesI18n.t("更新")} ${date(q.updatedAt)}</span></div>${q.metadataError ? `<p class="tiny muted quota-identity-note">${escapeHTML(AieyesI18n.message(q.metadataError))}${q.identity?.stale && q.plan ? AieyesI18n.t(" · 显示上次订阅") : ''}</p>` : ''}${(q.balances ?? []).map((b) => `<div class="quota-window"><div class="between"><span>${AieyesI18n.t("可用余额")}</span><strong>${escapeHTML(b.currency)} ${escapeHTML(b.total)}</strong></div><div class="between tiny muted balance-detail"><span>${AieyesI18n.t("赠送 {0}", escapeHTML(b.granted))}</span><span>${AieyesI18n.t("充值 {0}", escapeHTML(b.toppedUp))}</span></div></div>`).join('')}${extraUsageHTML(q.extraUsage)}${q.isAvailable === false ? AieyesI18n.t("<div class=\"error\">当前余额不足以调用 API</div>") : ''}${windows}${q.bankReset ? `<details class="bank" data-agent-detail="bank:${escapeHTML(key)}"><summary>${AieyesI18n.t("Bank Reset · {0} 次可用", q.bankReset.availableCount)}</summary>${(q.bankReset.credits ?? []).map((c) => `<div class="between tiny muted"><span>${escapeHTML(c.title ?? 'Reset Credit')}</span><span>${AieyesI18n.t("到期 {0}", date(c.expiresAt))}</span></div>`).join('')}</details>` : ''}${q.error ? `<div class="error quota-error"><div class="error-detail">${AieyesUI.errorHTML(q.error, AieyesI18n.t("额度查询失败"))}</div><span class="short-error">${AieyesI18n.t("额度更新失败 · 展开查看")}</span></div>` : ''}${q.provider !== 'antigravity' && (q.windows.some((w) => w.windowMinutes === window.AieyesContract.windows.fiveHour || w.windowMinutes === window.AieyesContract.windows.week) || latest) ? estimateEntry(q, records) : ''}${q.provider === 'codex' ? creditRow(q, credit, credits) : ''}<button class="quota-analysis" data-view-analysis="${escapeHTML(key)}">${AieyesI18n.t("查看分析 {0}", uiIcon('right'))}</button></div></div>`;
}

function drawTrend() {
  const canvas = $('#trend');if (!canvas || !state.dashboard) return;
  const width = canvas.clientWidth,height = canvas.clientHeight,dpr = window.devicePixelRatio || 1;canvas.width = width * dpr;canvas.height = height * dpr;
  const ctx = canvas.getContext('2d');ctx.scale(dpr, dpr);
  const days = state.dashboard.trendDays,rows = state.dashboard.dayModels.filter((r) => !hiddenTrendModels.has(r.model)),byDay = new Map();for (const row of rows) {if (!byDay.has(row.day)) byDay.set(row.day, []);byDay.get(row.day).push(row);}const values = days.map((d) => (byDay.get(d.key) ?? []).reduce((n, r) => n + (state.cost ? r.usage.cost : r.usage.total), 0)),max = Math.max(1, ...values);
  const style = getComputedStyle(canvas),metric = (name) => Number.parseFloat(style.getPropertyValue('--' + name)),muted = style.getPropertyValue('--muted');
  const left = metric('chart-plot-left'),top = metric('chart-plot-top'),bottom = metric('chart-plot-bottom'),plot = height - top - bottom,space = (width - left) / Math.max(1, days.length);
  ctx.font = `${style.getPropertyValue('--font-secondary').trim() || '14px'} ${style.fontFamily}`;ctx.fillStyle = muted;ctx.textAlign = 'right';
  for (let i = 0; i < 3; i++) {const y = top + plot * i / 2;ctx.fillText(state.cost ? money(max * (1 - i / 2)) : compact(max * (1 - i / 2)), left - metric('chart-label-gap'), y + 3);ctx.strokeStyle = style.color;ctx.beginPath();ctx.moveTo(left, y);ctx.lineTo(width, y);ctx.stroke();}
  let labelRight = -Infinity;
  days.forEach((d, i) => {
    let used = 0;const x = left + i * space + space * .2;
    (byDay.get(d.key) ?? []).forEach((r) => {const h = (state.cost ? r.usage.cost : r.usage.total) / max * plot;ctx.fillStyle = color(r.model);ctx.fillRect(x, top + plot - used - h, Math.max(1, space * .6), h);used += h;});
    if (i % Math.max(1, Math.floor(days.length / 7)) === 0) {
      const label = d.key.slice(5),labelWidth = ctx.measureText(label).width;
      if (labelWidth + 2 > width) return;
      // Keep the complete date inside the canvas and leave room between nearby ticks.
      const labelX = Math.max(labelWidth / 2 + 1, Math.min(width - labelWidth / 2 - 1, x + space * .3));
      if (labelX - labelWidth / 2 >= labelRight + metric('chart-label-gap')) {
        ctx.fillStyle = muted;ctx.textAlign = 'center';ctx.fillText(label, labelX, height - 5);
        labelRight = labelX + labelWidth / 2;
      }
    }
  });
  if (Number.isInteger(state.trendDayIndex) && days[state.trendDayIndex]) {const x = left + (state.trendDayIndex + .5) * space;ctx.save();ctx.strokeStyle = style.getPropertyValue('--accent');ctx.lineWidth = 1;ctx.setLineDash([3]);ctx.beginPath();ctx.moveTo(x, top);ctx.lineTo(x, height - bottom);ctx.stroke();ctx.restore();}
  canvas.tabIndex = 0;
  let selected = Math.min(state.trendDayIndex ?? 0, Math.max(0, days.length - 1));
  const describe = (i) => {const day = days[i];if (!day) return;const changed = state.trendDayIndex !== i;state.trendDayIndex = i;const label = day.key + ' · ' + trendUsage(day);canvas.removeAttribute('title');let reading = $('#trend-reading');if (!reading) {reading = document.createElement('p');reading.id = 'trend-reading';reading.className = 'muted tiny';reading.setAttribute('aria-live', 'polite');canvas.after(reading);}reading.textContent = label;canvas.setAttribute('aria-label', label + AieyesI18n.t("；左右键选择日期，回车查看精确值"));if (changed) drawTrend();};
  canvas.onmousemove = (e) => {const i = Math.floor((e.offsetX - left) / space);if (days[i]) describe(i);};
  canvas.onfocus = () => describe(selected);
  canvas.onkeydown = (e) => {if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) {e.preventDefault();selected = e.key === 'Home' ? 0 : e.key === 'End' ? days.length - 1 : Math.max(0, Math.min(days.length - 1, (state.trendDayIndex ?? selected) + (e.key === 'ArrowLeft' ? -1 : 1)));describe(selected);} else if (e.key === 'Enter' && days[state.trendDayIndex ?? selected]) showUsageDay(days[state.trendDayIndex ?? selected]);};
  // A background redraw or metric switch keeps the user's selected date readable.
  if (state.trendDayIndex != null && days.length) describe(selected);

}
function resourcePercent(value) {return Number.isFinite(value) ? Math.max(0, Math.min(100, value)) : null;}
function resourceColor(value) {return value >= 90 ? 'var(--resource-high)' : value >= 70 ? 'var(--resource-warn)' : 'var(--category,var(--accent))';}
function capacityPercent(used, total) {return Number.isFinite(used) && Number.isFinite(total) && total > 0 ? used / total * 100 : null;}
function quotaColor(value) {return value == null ? 'var(--muted)' : value < 10 ? 'var(--resource-high)' : value <= 30 ? 'var(--resource-warn)' : 'var(--accent)';}
function phaseColor(phase) {return getComputedStyle(document.documentElement).getPropertyValue('--phase-' + phase).trim() || getComputedStyle(document.documentElement).getPropertyValue('--phase-idle').trim();}
function resourceBar(value, label, quota = false) {
  const n = resourcePercent(value);
  return `<div class="resource-bar${n == null ? ' unavailable' : ''}" role="${n == null ? 'img' : 'progressbar'}" aria-label="${escapeHTML(label)}" ${n == null ? '' : `aria-valuemin="0" aria-valuemax="100" aria-valuenow="${n}"`}><span style="width:${n ?? 0}%;background:${quota ? quotaColor(n) : resourceColor(n)}"></span></div>`;
}
function resourceRing(label, value, detail = '') {
  const n = resourcePercent(value);
  return `<div class="resource-gauge"><div class="resource-ring" role="img" aria-label="${escapeHTML(label)} ${pct(n)}${n >= 90 ? AieyesI18n.t("，高负载") : ''}"><svg viewBox="0 0 100 100" aria-hidden="true"><circle class="ring-track" cx="50" cy="50" r="50"/><circle class="ring-value" ${n == null || n === 0 ? 'hidden' : ''} cx="50" cy="50" r="50" pathLength="100" stroke-dasharray="${n ?? 0} 100" style="stroke:${resourceColor(n)}"/></svg><strong>${pct(n)}</strong></div><div><h3>${escapeHTML(label)}</h3>${n >= 90 ? AieyesI18n.t("<span class=\"resource-alert\">高负载</span>") : ''}${detail ? `<span class="muted tiny">${escapeHTML(detail)}</span>` : ''}</div></div>`;
}
function hostStatus(host, result) {
  if (!host.enabled) return AieyesI18n.t("已暂停");
  if (result?.error) return AieyesI18n.t("连接失败");
  if (!result?.sample) return AieyesI18n.t("等待采样");
  const timestamp = Number(result.sample.timestamp);
  if (!Number.isFinite(timestamp) || timestamp <= 0 || Date.now() - timestamp * 1000 > 10000) return AieyesI18n.t("数据已延迟");
  return Object.keys(result.sample.errors ?? {}).length ? AieyesI18n.t("部分采集失败") : AieyesI18n.t("正常");
}
function updateHostStatuses() {
  if (state.page !== 'servers' || !state.settings) return;
  for (const element of document.querySelectorAll('[data-host-status]')) {
    const host = monitoredHosts().find((row) => row.id === element.dataset.hostStatus);if (!host) continue;
    const status = hostStatus(host, state.hosts.find((row) => row.id === host.id));element.onclick = () => sample(host.id);element.title = status + AieyesI18n.t(" · 点击刷新服务器");element.disabled = hostRefreshBusy(host.id);
    if (element.dataset.status !== status) {element.dataset.status = status;element.querySelector('[data-host-status-label]').textContent = status;}
  }
}
function hostShows(host, key) {return host.details == null || host.details.includes(key);}
function hostDevices(host, key, rows) {
  if (!(host.metrics ?? Object.keys(groups)).includes(key)) return [];
  const selected = (host.devices ?? []).filter((token) => token.startsWith(key + ':'));
  return (rows ?? []).filter((d) => !selected.length && (key !== 'filesystems' || AieyesSelect.recommendedFilesystem(d.id, d.type)) || selected.includes(key + ':__all__') || key === 'cpu' && d.id === 'cpu' || selected.includes(key + ':' + d.id));
}
function usedCapacity(total, available) {return Number.isFinite(total) && Number.isFinite(available) ? total - available : null;}
function uptimeText(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const minutes = Math.floor(seconds / 60);
  if (!Number.isSafeInteger(minutes)) return '—';
  return [minutes >= 1440 ? Math.floor(minutes / 1440) + AieyesI18n.t(" 天") : null, minutes >= 60 ? Math.floor(minutes % 1440 / 60) + AieyesI18n.t(" 小时") : null, minutes % 60 + AieyesI18n.t(" 分")].filter((v) => v != null).join(' ');
}
function capacityPair(used, total) {
  const scale = Math.max(Number.isFinite(total) ? total : 0, Number.isFinite(used) ? used : 0),units = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
  const power = scale > 0 ? Math.min(units.length - 1, Math.max(0, Math.floor(Math.log(scale) / Math.log(1024)))) : 0;
  const number = (n) => Number.isFinite(n) ? (n / 1024 ** power).toFixed(1).replace(/\.0$/, '') : '—';
  return number(used) + '/' + number(total) + ' ' + units[power];
}
function resourceStrip(title, percent, detail = '') {
  const value = resourcePercent(percent),label = (detail ? detail + ' · ' : '') + (value == null ? '—' : Math.round(value) + '%');
  return `<div class="server-resource-strip" title="${escapeHTML(title + ' ' + label)}"><div class="resource-strip-heading"><span>${escapeHTML(title)}</span><strong${value >= 90 ? ' class="error"' : ''}>${escapeHTML(label)}</strong></div>${resourceBar(value, title + ' ' + label)}</div>`;
}
function resourceBlock(label, percent, detail = '', kind = 'cpu') {
  const value = resourcePercent(percent);
  return `<div class="resource-block" data-resource="${kind}"><div class="between"><span>${escapeHTML(label)}</span>${value >= 90 ? `<span class="error" title="${AieyesI18n.t("高负载")}" aria-label="${AieyesI18n.t("高负载")}">${uiIcon('warning')}</span>` : ''}</div><strong>${value == null ? '—' : Math.round(value) + '%'}</strong>${resourceBar(value, label)}${detail ? `<small>${escapeHTML(detail)}</small>` : ''}</div>`;
}
function serverSummary(host, sample) {
  const enabled = (key) => (host.metrics ?? Object.keys(groups)).includes(key),memory = sample?.memory;
  const gpus = enabled('gpu') ? hostDevices(host, 'gpu', sample?.gpu) : [],networks = enabled('network') ? hostDevices(host, 'network', sample?.network) : [];
  const system = (enabled('cpu') ? resourceBlock('CPU', sample?.cpu?.find((c) => c.id === 'cpu')?.utilization, '', 'cpu') : '') + (
  enabled('memory') ? resourceBlock(AieyesI18n.t("内存"), capacityPercent(usedCapacity(memory?.total, memory?.available), memory?.total), capacityPair(usedCapacity(memory?.total, memory?.available), memory?.total), 'memory') : '');
  const gpuCards = gpus.map((g) => `<div class="server-gpu" data-resource="gpu" data-gpu="${escapeHTML(g.id)}"><div class="server-gpu-title"><strong>GPU ${escapeHTML(g.id)}</strong><span title="${escapeHTML(g.name ?? '—')}">${escapeHTML(g.name ?? '—')}</span></div><div class="server-gpu-metrics" data-memory="${hostShows(host, 'gpuMemory')}">${resourceStrip(AieyesI18n.t("利用率"), g.utilization)}${hostShows(host, 'gpuMemory') ? resourceStrip(AieyesI18n.t("显存"), capacityPercent(g.memoryUsedMiB, g.memoryTotalMiB), capacityPair(g.memoryUsedMiB == null ? null : g.memoryUsedMiB * 1048576, g.memoryTotalMiB == null ? null : g.memoryTotalMiB * 1048576)) : ''}</div></div>`).join('');
  const filesystems = hostDevices(host, 'filesystems', sample?.filesystems),fsSummary = enabled('filesystems') ? `<div class="server-filesystems" data-resource="storage"><span class="muted">${AieyesI18n.t("文件系统")}</span><div class="server-filesystem-list balanced-grid">${filesystems.length ? filesystems.map((fs) => `<div data-filesystem="${escapeHTML(fs.id)}">${resourceStrip(fs.id, capacityPercent(fs.used, fs.total), capacityPair(fs.used, fs.total))}</div>`).join('') : `<span class="muted">${sample?.filesystems ? AieyesI18n.t("无已选挂载点") : '—'}</span>`}</div></div>` : '';
  return `<div class="server-summary-metrics">${system ? `<div class="server-system-metrics">${system}</div>` : ''}${gpuCards ? `<div class="server-gpu-list balanced-grid">${gpuCards}</div>` : enabled('gpu') ? `<span class="muted">GPU ${sample?.gpu ? AieyesI18n.t("无已选设备") : '—'}</span>` : ''}${fsSummary}${enabled('network') ? `<div class="server-network-rates balanced-grid" data-resource="network">${networks.length ? networks.map((n) => `<div><span>${escapeHTML(n.id)}</span><strong>↓ ${speed(n.rxBytesPerSecond)} · ↑ ${speed(n.txBytesPerSecond)}</strong></div>`).join('') : AieyesI18n.t("<span>网络 —</span>")}</div>` : ''}<div class="server-summary-facts">${hostShows(host, 'uptime') ? `<span data-host-uptime>${AieyesI18n.t("连续运行 ")}<strong>${uptimeText(sample?.uptime)}</strong></span>` : ''}${hostShows(host, 'networkTotals') && enabled('network') ? networks.length ? networks.map((n) => `<span class="server-network-total" data-network-total="${escapeHTML(n.id)}"><span>${AieyesI18n.t("{0} 累计", escapeHTML(n.id))}</span><strong>↓ ${bytes(n.rxBytes)} · ↑ ${bytes(n.txBytes)}</strong></span>`).join('') : `<span>${AieyesI18n.t("累计流量 {0}", sample?.network ? AieyesI18n.t("无已选网卡") : '—')}</span>` : ''}<span class="server-sample-time muted">${AieyesI18n.t("采样 {0}{1}", date(sample?.timestamp), !host.enabled ? AieyesI18n.t(" · 已暂停，保留旧读数") : '')}</span></div></div>`;
}
function renderServers() {window.AieyesMotion?.captureNavigation();window.AieyesServers.render();window.AieyesSingleSelect?.mountAll($('#content'));window.AieyesMotion?.didRender('servers');}
function serverGroupContent(key, value, host) {
  const show = (key) => hostShows(host, key);
  const counterTotal = (received, sent) => Number.isFinite(received) && Number.isFinite(sent) && received >= 0 && sent >= 0 && Number.isFinite(received + sent) ? compact(received + sent) : '—';
  const row = (name, value) => `<div class="metric-row"><span>${escapeHTML(name)}</span><strong>${value}</strong></div>`;
  const meter = (name, value, percent) => row(name, value) + resourceBar(percent, name);
  let content = '';
  if (key === 'memory') content = row(AieyesI18n.t("可用"), bytes(value.available)) + (show('memoryCache') ? row(AieyesI18n.t("缓存 / Buffer"), `${bytes(value.cached)} / ${bytes(value.buffers)}`) : '') + (show('swap') ? meter('Swap', `${bytes(usedCapacity(value.swapTotal, value.swapFree))} / ${bytes(value.swapTotal)}`, capacityPercent(usedCapacity(value.swapTotal, value.swapFree), value.swapTotal)) : '');else
  content = value.map((d) => {
    if (key === 'cpu') return (d.id === 'cpu' ? '' : meter(d.id, pct(resourcePercent(d.utilization)), d.utilization)) + (show('cpuTimes') ? row(d.id + ' user / system', `${pct(d.userPercent)} / ${pct(d.systemPercent)}`) + row('iowait / steal', `${pct(d.iowaitPercent)} / ${pct(d.stealPercent)}`) : '');
    if (key === 'gpu') return show('gpuThermals') ? row(AieyesI18n.t("GPU {0} 温度 / 功耗", d.id), `${d.temperature == null ? '—' : d.temperature + '°C'} / ${d.powerWatts == null ? '—' : d.powerWatts + ' W'}`) : '';
    if (key === 'filesystems') return (show('fsAvailable') || show('fsType') || show('inodes') ? row(d.id, '') : '') + (show('fsAvailable') ? row(AieyesI18n.t("可用"), bytes(d.available)) : '') + (show('fsType') ? row(d.device ?? '', escapeHTML(d.type ?? '')) : '') + (show('inodes') ? meter('inode', pct(capacityPercent(usedCapacity(d.inodes, d.inodesFree), d.inodes)), capacityPercent(usedCapacity(d.inodes, d.inodesFree), d.inodes)) : '');
    if (key === 'disk') return row(d.id, AieyesI18n.t("读 {0} · 写 {1}", speed(d.readBytesPerSecond), speed(d.writeBytesPerSecond))) + (show('diskIops') ? row(AieyesI18n.t("IOPS 读 / 写"), `${d.readIops == null ? '—' : compact(d.readIops)} / ${d.writeIops == null ? '—' : compact(d.writeIops)}`) : '') + (show('diskBusy') ? meter(AieyesI18n.t("忙碌率"), pct(resourcePercent(d.busyMsPerSecond == null ? null : d.busyMsPerSecond / 10)), d.busyMsPerSecond == null ? null : d.busyMsPerSecond / 10) : '');
    return row(d.id, `↓ ${speed(d.rxBytesPerSecond)} · ↑ ${speed(d.txBytesPerSecond)}`) + (show('networkErrors') ? row(AieyesI18n.t("错误 / 丢包"), `${counterTotal(d.rxErrors, d.txErrors)} / ${counterTotal(d.rxDrops, d.txDrops)}`) : '');
  }).join('');
  return content;
}
function field(name, label, value = '', placeholder = '', type = 'text') {const size = type === 'number' || /^(port|.*Port|.*Seconds|.*Interval|input|output|cacheRead|cacheWrite)$/.test(name) ? 'short' : /^(username|name)$/.test(name) ? 'medium' : 'long';return `<div class="form-row" data-field-size="${size}"> <label for="field-${name}">${label}</label><input id="field-${name}" name="${name}" type="${type}" value="${escapeHTML(value)}" placeholder="${escapeHTML(placeholder)}"></div>`;}
function select(name, label, entries, current) {return `<div class="form-row" data-field-size="${/Protocol$/.test(name) ? 'protocol' : 'choice'}"> <label for="field-${name}">${label}</label><select id="field-${name}" name="${name}">${entries.map(([k, v]) => option(k, v, current)).join('')}</select></div>`;}
function textarea(name, label, value = '') {return `<div class="form-row"><label for="field-${name}">${label}</label><textarea id="field-${name}" name="${name}">${escapeHTML(value)}</textarea></div>`;}
const PRICE_ROW_HEIGHT = 64;
let priceCache = { source: null, query: '', rows: [] };
function matchingPrices() {
  const query = state.priceSearch.trim().toLocaleLowerCase();
  if (priceCache.source !== state.prices || priceCache.query !== query) priceCache = { source: state.prices, query, rows: state.prices.filter((p) => [p.id, p.name].some((v) => String(v ?? '').toLocaleLowerCase().includes(query))) };
  return priceCache.rows;
}
function priceRows(top = 0) {
  const prices = matchingPrices(),first = Math.max(0, Math.floor(top / PRICE_ROW_HEIGHT) - 2),last = Math.min(prices.length, first + 9);
  return prices.length ? `<div class="price-list-spacer" style="height:${prices.length * PRICE_ROW_HEIGHT}px">` + prices.slice(first, last).map((p, offset) => `<div class="list-row price-virtual-row" style="top:${(first + offset) * PRICE_ROW_HEIGHT}px"><div class="row-body" title="${escapeHTML(p.id)}">${escapeHTML(p.id)}<small>${AieyesI18n.t("输入 {0} · 输出 {1}", p.input == null ? '—' : money(p.input * 1e6), p.output == null ? '—' : money(p.output * 1e6))}</small></div><button data-map-price="${escapeHTML(p.id)}">${AieyesI18n.t("一键映射")}</button><button data-price="${escapeHTML(p.id)}" aria-label="${AieyesI18n.t("编辑 {0} 的价格", escapeHTML(p.id))}">${AieyesI18n.t("编辑")}</button></div>`).join('') + '</div>' : `<div class="empty">${priceCache.query ? AieyesI18n.t("无匹配模型") : AieyesI18n.t("同步模型价格")}</div>`;
}
function bindPriceViewport() {
  const viewport = document.querySelector('.prices-list');if (!viewport) return;let scheduled = false,lastStart = -1;
  viewport.onscroll = () => {if (scheduled) return;scheduled = true;requestAnimationFrame(() => {scheduled = false;if (!viewport.isConnected) return;const top = viewport.scrollTop,start = Math.floor(top / PRICE_ROW_HEIGHT);if (start === lastStart) return;lastStart = start;viewport.innerHTML = priceRows(top);viewport.scrollTop = top;bindPrices();});};
}
function existingMappingRows() {return Object.entries(draftSettings().modelMappings).map(([from, to]) => `<div class="list-row tiny"><span class="row-body">${escapeHTML(from)} → ${escapeHTML(to)}</span><button type="button" data-mapping-edit="${escapeHTML(from)}">${AieyesI18n.t("编辑")}</button><button type="button" data-unmap="${escapeHTML(from)}" aria-label="${AieyesI18n.t("移除 {0} 的模型映射", escapeHTML(from))}">${uiIcon('minus')}</button></div>`).join('');}
function bindExistingMappings() {
  const details = document.querySelector('#existing-mappings');if (!details) return;
  const mount = () => {
    const body = details.querySelector('[data-mapping-rows]');body.innerHTML = details.open ? existingMappingRows() : '';
    body.querySelectorAll('[data-mapping-edit]').forEach((button) => button.onclick = () => {
      const name = button.dataset.mappingEdit,baseline = structuredClone(state.settings);
      showEditor(AieyesI18n.t("编辑模型映射"), field('from', AieyesI18n.t("日志模型名称"), name) + field('to', AieyesI18n.t("OpenRouter 模型 ID"), baseline.modelMappings[name]), async (form) => {
        const from = form.get('from').trim(),to = form.get('to').trim();if (!from || !to) throw new Error(AieyesI18n.t("请补全映射两端"));if (from !== name && baseline.modelMappings[from] != null) throw new Error(AieyesI18n.t("日志模型名称已存在"));
        const next = structuredClone(baseline);delete next.modelMappings[name];next.modelMappings[from] = to;await window.AieyesAgentSettings.commit(next, baseline);renderSettings(false);
      });
    });
    body.querySelectorAll('[data-unmap]').forEach((button) => button.onclick = () => job(AieyesI18n.t("移除模型映射"), async () => {const next = structuredClone(state.settings);delete next.modelMappings[button.dataset.unmap];await saveSettings(next);renderSettings(false);}));
  };
  details.ontoggle = mount;mount();
}
function bindPrices() {
  document.querySelectorAll('[data-map-price]').forEach((b) => b.onclick = () => {const input = $('#field-mappingId');input.value = b.dataset.mapPrice;$('#mapping-form').scrollIntoView({ block: 'start', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' });input.focus({ preventScroll: true });});
  document.querySelectorAll('[data-price]').forEach((b) => b.onclick = () => editPrice(state.prices.find((p) => p.id === b.dataset.price)));}
const settingsForms = new Map();
const settingsFormKey = (form) => form.dataset.draftKey || form.id;
let generalSaveTimer,generalSaveVersion = 0;
function scheduleGeneralSave() {
  const version = ++generalSaveVersion;
  clearTimeout(generalSaveTimer);state.generalSavePending = true;
  generalSaveTimer = setTimeout(async () => {
    if (state.settingsCommitting || state.settingsSaving) {scheduleGeneralSave();return;}
    try {await saveAllSettings({ section: 'general' });} catch (error) {state.saveError = error.message ?? String(error);updateSettingsSaveBar();} finally
    {if (version === generalSaveVersion) state.generalSavePending = false;}
  }, 250);
}
function captureSettingsForms() {
  for (const form of document.querySelectorAll('#general-form,#mapping-form')) {
    const key = settingsFormKey(form),values = AieyesUI.formValues(form),existing = settingsForms.get(key);
    settingsForms.set(key, { values, baseline: existing?.baseline ?? JSON.parse(form.dataset.draftBaseline || JSON.stringify(values)) });
  }
}
function settingsFormsDirty() {return settingsChangeCount() > 0;}
function bindSettingsDrafts() {
  for (const form of document.querySelectorAll('#general-form,#mapping-form')) {
    const key = settingsFormKey(form),draft = settingsForms.get(key);form.dataset.draftBaseline = JSON.stringify(draft?.baseline ?? AieyesUI.formValues(form));
    if (draft) AieyesUI.restoreForm(form, draft.values);
    const update = () => {captureSettingsForms();const d = settingsForms.get(key);form.dataset.dirty = String(JSON.stringify(d.values) !== JSON.stringify(d.baseline));const label = form.querySelector('.draft-state');if (label) label.textContent = form.dataset.dirty === 'true' ? AieyesI18n.t("未保存 · 切页保留草稿") : '';updateSettingsSaveBar();};
    const label = document.createElement('p');label.className = 'draft-state';label.setAttribute('role', 'status');form.append(label);
    form.addEventListener('input', update);form.addEventListener('change', () => {update();if (form.id === 'general-form') scheduleGeneralSave();});update();
  }
}
function resetSettingsForm(id) {const form = document.getElementById(id);if (form) {const values = AieyesUI.formValues(form);settingsForms.set(settingsFormKey(form), { values, baseline: values });form.dataset.draftBaseline = JSON.stringify(values);form.dataset.dirty = 'false';form.querySelector('.draft-state')?.replaceChildren();} else settingsForms.delete(id);}
function mappingDraft(next) {
  captureSettingsForms();const values = settingsForms.get('mapping-form')?.values ?? {};
  const from = (values.mappingModel ?? '').trim(),to = (values.mappingId ?? '').trim();
  if (Boolean(from) !== Boolean(to)) {const field = document.getElementById(from ? 'field-mappingId' : 'field-mappingModel');field?.setCustomValidity(AieyesI18n.t("请补全映射两端"));field?.reportValidity();field?.addEventListener('input', () => field.setCustomValidity(''), { once: true });throw new Error(AieyesI18n.t("请补全日志模型名称和 OpenRouter 模型 ID，输入已保留"));}
  if (from && next.modelMappings?.[from] && next.modelMappings[from] !== to) throw new Error(AieyesI18n.t("该模型已有映射；请在已有映射中编辑"));
  next.modelMappings ??= {};if (from) next.modelMappings[from] = to;
  return next;
}
function settingsSidebar(items, current, attribute, label) {
  return `<nav class="settings-sidebar" aria-label="${escapeHTML(label)}">${items.map(([key, label]) => `<button type="button" ${attribute}="${escapeHTML(key)}" ${key === current ? 'class="active" aria-current="page"' : ''}>${escapeHTML(label)}</button>`).join('')}</nav>`;
}
function bindSettingsSidebar(attribute, onSelect) {
  const buttons = [...document.querySelectorAll('#content [' + attribute + ']')];
  for (const button of buttons) {
    button.onclick = () => {captureSettingsForms();onSelect(button.getAttribute(attribute));};
    button.onkeydown = (event) => {const index = buttons.indexOf(button),next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : event.key === 'ArrowDown' ? (index + 1) % buttons.length : event.key === 'ArrowUp' ? (index + buttons.length - 1) % buttons.length : null;if (next == null) return;event.preventDefault();buttons[next].click();document.querySelector('[' + attribute + '="' + CSS.escape(buttons[next].getAttribute(attribute)) + '"]')?.focus();};
  }
}
let generalPages = [['appearance', AieyesI18n.t("外观")], ['desktop', AieyesI18n.t("桌宠与会话")], ['network', AieyesI18n.t("网络代理")], ['refresh', AieyesI18n.t("刷新与历史")], ['notifications', AieyesI18n.t("通知")], ['updates', AieyesI18n.t("更新与关于")]];window.addEventListener("aieyes:language", () => {generalPages = [['appearance', AieyesI18n.t("外观")], ['desktop', AieyesI18n.t("桌宠与会话")], ['network', AieyesI18n.t("网络代理")], ['refresh', AieyesI18n.t("刷新与历史")], ['notifications', AieyesI18n.t("通知")], ['updates', AieyesI18n.t("更新与关于")]];});
function generalSettingsHTML(s, tab = state.generalTab) {
  if (!generalPages.some(([key]) => key === tab)) {tab = 'appearance';if (arguments.length < 2) state.generalTab = tab;}
  let fields = '',body = '';
  if (tab === 'appearance') fields = `<h2>${AieyesI18n.t("外观")}</h2>${select('appearanceLanguage', AieyesI18n.t('语言'), [['zh-CN','简体中文'], ['en','English']], s.appearance?.language ?? 'zh-CN')}${select('appearanceTheme', AieyesI18n.t("主题"), [['system', AieyesI18n.t("跟随系统")], ['light', AieyesI18n.t("浅色")], ['dark', AieyesI18n.t("深色")]], s.appearance?.theme ?? 'system')}${select('appearanceAccent', AieyesI18n.t("强调色"), [['indigo', AieyesI18n.t("靛蓝")], ['blue', AieyesI18n.t("蓝")], ['teal', AieyesI18n.t("青绿")], ['purple', AieyesI18n.t("紫")]], s.appearance?.accent ?? 'indigo')}${select('appearanceChartPalette', AieyesI18n.t("图表配色"), window.AieyesChartPalettes.map((p) => [p.id, p.name]), s.appearance?.chartPalette ?? 'classic')}${select('appearanceChartContrast', AieyesI18n.t("图表对比度"), [['high', AieyesI18n.t("高对比度")], ['low', AieyesI18n.t("低对比度 · 柔和")]], s.appearance?.chartContrast ?? 'high')}<div id="chart-palette-preview" class="chart-palette-preview"></div><p class="muted tiny">${AieyesI18n.t("图表配色随明暗主题调整。高对比度适合小色块，低对比度更柔和。")}</p><p class="font-notice">${AieyesI18n.t("界面字体：HarmonyOS Sans SC（鸿蒙黑体）· © 2021 Huawei Device Co., Ltd. ")}<button type="button" id="font-license">${AieyesI18n.t("字体许可")}</button></p>`;
  if (tab === 'network') fields = `<h2>${AieyesI18n.t("网络代理")}</h2>${proxyFields('app', s.proxy)}${field('proxyTestUrls', AieyesI18n.t("测试地址（逗号分隔）"), (s.proxyTestUrls ?? ['https://api.github.com/rate_limit', 'https://openrouter.ai']).join(', '))}`;
  if (tab === 'refresh') {
    fields = `<h2>${AieyesI18n.t("刷新与历史")}</h2>${field('refreshSeconds', AieyesI18n.t("额度查询间隔（秒）"), s.refreshSeconds, '', 'number')}${field('historyRefreshSeconds', AieyesI18n.t("日志同步间隔（秒）"), s.historyRefreshSeconds ?? Math.max(10, s.refreshSeconds), '', 'number')}${field('serverForegroundRefreshSeconds', AieyesI18n.t("服务器前台间隔（秒）"), s.serverForegroundRefreshSeconds ?? 2, '', 'number')}${field('serverRefreshSeconds', AieyesI18n.t("服务器后台间隔（秒）"), s.serverRefreshSeconds, '', 'number')}<p class="muted tiny">${AieyesI18n.t("用量按 Agent 统计，已删除账户的历史用量仍计入总量。")}</p>`;
    fields += `<h3>${AieyesI18n.t("历史用量")}</h3><label><input id="field-preciseCounting" type="checkbox" name="preciseCounting" ${s.preciseCounting ? 'checked' : ''}>${AieyesI18n.t("精确计数")}</label><p class="muted tiny">${AieyesI18n.t("为所有 Agent 去除可识别的 fork 和继承历史重复用量。关闭时按各会话计入；修改后在下次同步自动重算。")}</p>`;
    const diagnostics = state.settings.preciseCounting ? (state.dashboard?.sources ?? []).filter((source) => source.enabled && (source.status?.forkHistoryIssues ?? []).length).map((source) => '<details><summary>' + escapeHTML(source.name) + AieyesI18n.t(" · 继承历史待确认</summary>") + source.status.forkHistoryIssues.map((issue) => '<p class="muted tiny">' + escapeHTML(AieyesI18n.message((issue.path ? issue.path + ' · ' : '') + issue.message)) + '</p>').join('') + AieyesI18n.t("<p class=\"muted tiny\">首段不确定累计暂不计入，后续有效增量继续统计；继承历史恢复后自动补全。</p></details>")).join('') : '';
    body = `${diagnostics ? AieyesI18n.t("<div class=\"card\"><h2>精确计数详情</h2>") + diagnostics + '</div>' : ''}<div class="card"><h2>${AieyesI18n.t("历史记录 {0}", refreshIndicator('rescan'))}</h2><p class="muted">${AieyesI18n.t("重新读取已启用来源的完整日志，修正可确认的重复记录。已有历史和价格快照保留。")}</p><button type="button" id="history-rescan">${AieyesI18n.t("全量重扫")}</button><p id="history-rescan-status" role="status" aria-live="polite" hidden></p></div>`;
  }
  if (tab === 'notifications') {
    fields = `<h2>${AieyesI18n.t("会话提示音")}</h2><label><input type="checkbox" name="needsInputSound" ${s.sessionSounds?.needsInput !== false ? 'checked' : ''}>${AieyesI18n.t("等待输入提示音")}</label><button type="button" data-sound-preview="needsInput">${AieyesI18n.t("试听等待输入")}</button><label><input type="checkbox" name="completeSound" ${s.sessionSounds?.complete !== false ? 'checked' : ''}>${AieyesI18n.t("任务完成提示音")}</label><button type="button" data-sound-preview="complete">${AieyesI18n.t("试听任务完成")}</button><p class="muted tiny">${AieyesI18n.t("仅在新状态发生时播放；关闭面板后仍有效。音量跟随系统输出。")}</p><p data-sound-result role="status"></p><h2>${AieyesI18n.t("订阅额度重置提醒")}</h2><label><input type="checkbox" name="unexpectedResetNotification" ${s.quotaResetNotifications?.unexpected ?? true ? 'checked' : ''}>${AieyesI18n.t("提前重置时提醒")}</label><label><input type="checkbox" name="naturalResetNotification" ${s.quotaResetNotifications?.natural ? 'checked' : ''}>${AieyesI18n.t("到期重置时提醒")}</label><p class="muted tiny">${AieyesI18n.t("账户可单独设置提醒规则。退出应用或电脑休眠时暂停检测。")}</p><details class="advanced"><summary>${AieyesI18n.t("检测条件")}</summary><p>${AieyesI18n.t("以实时查询确认的额度重置为准，按上次预计重置时间区分提前与到期。持续满额时不提醒；关闭窗口后仍会检测。")}</p></details>`;
    body = window.AieyesNotifications?.settingsHTML() ?? '';
  }
  if (tab === 'desktop') body = window.AieyesDesktop?.settingsHTML() ?? '';
  if (tab === 'updates') body = window.AieyesUpdates?.settingsHTML() ?? AieyesI18n.t("<div class=\"card\"><p>正在读取更新信息…</p></div>");
  if (fields) body = `<form id="general-form" data-draft-key="general-form:${tab}" class="card">${fields}<p class="muted tiny">${AieyesI18n.t("修改后自动保存。")}</p></form>` + body;
  return `<div class="settings-layout"><div class="settings-subpage" data-general-section="${tab}">${body}</div></div>`;
}
function renderSettings(capture = true) {
  window.AieyesMotion?.captureNavigation();
  if (capture) captureSettingsForms();const view = rememberView();
  if (state.settingsTab === 'connection') state.generalTab = 'network';
  state.settingsTab = settingsCategory(state.settingsTab);
  const tabs = { integrations: AieyesI18n.t("Agent 接入"), hosts: AieyesI18n.t("服务器"), prices: AieyesI18n.t("价格"), wakeups: AieyesI18n.t("定时唤醒"), general: AieyesI18n.t("常规") };
  let body = '';const s = draftSettings();
  if (state.settingsTab === 'integrations') body = window.AieyesAgentSettings.integrationsHTML();else
  if (state.settingsTab === 'hosts') {
    const list = configuredHosts(s);
    body = `<div class="actions ssh-import-toolbar"><button type="button" data-ssh-import>${AieyesI18n.t("从 SSH 配置导入")}</button><button type="button" data-guide-open>${AieyesI18n.t("配置向导")}</button></div><div class="card machine-list">${list.map((item) => {const collecting = item.id === 'local' ? s.localDeviceEnabled !== false : item.enabled,monitoring = item.id === 'local' ? item.enabled : item.monitoringEnabled !== false;return `<div class="list-row machine-row"><div class="row-body machine-info"><strong title="${escapeHTML(item.name || item.target)}">${escapeHTML(item.name || item.target)}</strong><small title="${escapeHTML(item.target)}">${escapeHTML(item.id === 'local' ? AieyesI18n.t("本机") : item.target)}</small>${collecting ? `<button type="button" data-native-scan-machine="${escapeHTML(item.id)}" ${window.AieyesAccountScan?.busy ? 'disabled' : ''}>${AieyesI18n.t("一键扫描账户与凭据")}</button>` : ''}</div><label class="machine-monitor"><input type="checkbox" role="switch" data-enable="${escapeHTML(item.id)}" ${monitoring ? 'checked' : ''} aria-label="${AieyesI18n.t("资源监控 · {0}", escapeHTML(item.name || item.target))}">${AieyesI18n.t("资源监控")}</label><label class="machine-collection"><input type="checkbox" role="switch" data-device-collection="${escapeHTML(item.id)}" ${collecting ? 'checked' : ''} aria-label="${AieyesI18n.t("Agent 采集 · {0}", escapeHTML(item.name || item.target))}">${AieyesI18n.t("Agent 采集")}</label><div class="machine-actions"><button data-edit="${escapeHTML(item.id)}" aria-label="${AieyesI18n.t("编辑 {0}", escapeHTML(item.name || item.target))}">${AieyesI18n.t("编辑")}</button>${item.id === 'local' ? '<span class="machine-action-placeholder" aria-hidden="true"></span>' : `<button data-remove="${escapeHTML(item.id)}" aria-label="${AieyesI18n.t("移除 {0}", escapeHTML(item.name))}">${uiIcon('minus')}</button>`}</div></div>`;}).join('')}${savedFeedback(state.settingsTab)}</div><button id="add-item" class="primary">${AieyesI18n.t("{0} 添加主机", uiIcon('plus'))}</button>`;
  } else if (state.settingsTab === 'prices') body = AieyesI18n.t("<div class=\"between\"><p class=\"muted save-model-note\">价格与模型映射保存后生效。智能引用按 ID、名称和版本匹配，未知供应商优先官方价格；无版本可匹配 latest，未标注 batch 不引用 batch，仅供估值。</p><button id=\"reprice\" type=\"button\" title=\"保存设置并按当前价格重新计算历史成本\">保存并重算</button></div>") + gapList() + `<form id="mapping-form" class="card"><h2>${AieyesI18n.t("模型映射")}</h2><details id="existing-mappings" data-agent-detail="existing-mappings"><summary>${AieyesI18n.t("已有映射（{0}）", Object.keys(s.modelMappings).length)}</summary><div data-mapping-rows></div></details>${field('mappingModel', AieyesI18n.t("日志模型名称"))}${field('mappingId', AieyesI18n.t("OpenRouter 模型 ID"))}<button>${AieyesI18n.t("添加映射")}</button></form>` + `<div class="price-toolbar"><input id="price-search" type="search" aria-label="${AieyesI18n.t("搜索模型")}" placeholder="${AieyesI18n.t("搜索模型")}" value="${escapeHTML(state.priceSearch)}"><button id="reference-prices" type="button" title="${AieyesI18n.t("按模型 ID、名称和版本引用 OpenRouter 价格；未知供应商优先官方，仅供估值")}">${AieyesI18n.t("智能引用")}</button><button id="sync-prices" title="${AieyesI18n.t("同步 OpenRouter 模型价格表")}">${AieyesI18n.t("同步 OpenRouter")}</button><button id="add-price" title="${AieyesI18n.t("手动添加一个模型价格")}">${AieyesI18n.t("添加价格")}</button>${savedFeedback('prices')}</div><div class="card prices-list">${priceRows()}</div><span class="muted tiny">${AieyesI18n.t("USD / 百万 Token")}</span>`;else
  if (state.settingsTab === 'wakeups') body = window.AieyesWakeups?.html() ?? AieyesI18n.t("<p>正在加载…</p>");else
  body = generalSettingsHTML(state.settings);
  $('#title').textContent = state.settingsTab === 'general' ? generalPages.find(([key]) => key === state.generalTab)?.[1] ?? AieyesI18n.t("外观") : { integrations: AieyesI18n.t("接入与账户"), hosts: AieyesI18n.t("机器与采集"), prices: AieyesI18n.t("模型价格"), wakeups: AieyesI18n.t("定时唤醒") }[state.settingsTab] ?? AieyesI18n.t("设置");
  $('#content').innerHTML = `<div class="settings-save-bar" id="settings-save-bar"><div class="settings-draft-label" role="status"></div><button id="settings-retry-save" type="button" hidden>${AieyesI18n.t("重试保存")}</button></div><div id="settings-panel" class="settings-block" aria-label="${escapeHTML($('#title').textContent)}">${body}</div>`;
  syncNavigation();
  if (state.settings.configurationIssues?.length) $('#content').insertAdjacentHTML('afterbegin', state.settings.configurationIssues.map((v) => `<div class="error">${escapeHTML(v)}</div>`).join(''));
  const fontLicense = $('#font-license');
  if (fontLicense) fontLicense.onclick = async () => {
    try {const response = await fetch('fonts/LICENSE.txt');if (!response.ok) throw new Error(AieyesI18n.t("字体许可读取失败"));const license = await response.text();quotaDialog(AieyesI18n.t("HarmonyOS Sans 字体许可"), `<pre class="font-license-text">${escapeHTML(license)}</pre>`);}
    catch (error) {notify(String(error), 'error');}
  };
  bindRememberedDisclosures();bindSettingsDrafts();window.AieyesDesktop?.bindSettings();window.AieyesNotifications?.bind();updateSettingsSaveBar();
  updateChartPalettePreview();
  $('#general-form')?.addEventListener('change', updateChartPalettePreview);
  if (state.settingsTab === 'wakeups') window.AieyesWakeups?.bind();
  if (state.settingsTab === 'integrations') window.AieyesAgentSettings.bind();
  if (state.settingsTab === 'integrations' && state.settingsAgent === 'codex') window.AieyesCodexAPI?.bind();
  bindSettingsSidebar('data-general-page', (page) => {state.generalTab = page;renderSettings();});
  if ($('#settings-retry-save')) $('#settings-retry-save').onclick = scheduleGeneralSave;
  document.querySelectorAll('#content [data-settings-tab]').forEach((button) => {
    button.onclick = async () => {
      state.settingsTab = button.dataset.settingsTab;renderSettings();
      try {if (state.settingsTab === 'prices') {state.prices = await api('prices.list');if (state.page === 'settings' && state.settingsTab === 'prices') renderSettings();}await loadDashboard();}
      catch (error) {notify(String(error), 'error');}
    };
    button.onkeydown = (event) => {
      const buttons = [...document.querySelectorAll('[data-settings-tab]')],index = buttons.indexOf(button);
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : event.key === 'ArrowRight' ? (index + 1) % buttons.length : event.key === 'ArrowLeft' ? (index + buttons.length - 1) % buttons.length : null;
      if (next == null) return;event.preventDefault();buttons[next].focus();buttons[next].click();
    };
  });
  document.querySelectorAll('[data-gap-price]').forEach((b) => b.onclick = () => {const g = state.dashboard.pricingGaps[Number(b.dataset.gapPrice)];editPrice(state.prices.find((p) => p.id === (g.priceId ?? g.model)) ?? { id: g.priceId ?? g.model, name: g.model });});
  document.querySelectorAll('[data-gap-reference]').forEach((b) => b.onclick = () => {const g = state.dashboard.pricingGaps[Number(b.dataset.gapReference)];void referencePrices(g.model, modelName(g.model));});
  document.querySelectorAll('[data-gap-map]').forEach((b) => b.onclick = () => {const g = state.dashboard.pricingGaps[Number(b.dataset.gapMap)];$('#field-mappingModel').value = g.model;$('#field-mappingId').value = g.priceId ?? '';$('#field-mappingId').focus();});
  const items = state.settingsTab === 'sources' ? s.sources : configuredHosts(s);
  document.querySelectorAll('[data-edit]').forEach((b) => b.onclick = () => editItem(items.find((i) => i.id === b.dataset.edit)));
  document.querySelectorAll('[data-enable]').forEach((input) => input.onchange = async () => {
    const next = structuredClone(state.settings),id = input.dataset.enable;input.disabled = true;
    if (id === 'local') next.localMonitor = { ...next.localMonitor, enabled: input.checked };else next.hosts.find((h) => h.id === id).monitoringEnabled = input.checked;
    try {await window.AieyesAgentSettings.commit(next);} catch (error) {notify(String(error), 'error');} finally
    {const saved = state.settings;input.checked = id === 'local' ? saved.localMonitor?.enabled !== false : saved.hosts.find((h) => h.id === id)?.monitoringEnabled !== false;input.disabled = false;}
  });
  document.querySelectorAll('[data-device-collection]').forEach((input) => input.onchange = async () => {
    const next = structuredClone(state.settings),id = input.dataset.deviceCollection,enabled = input.checked;input.disabled = true;
    if (id === 'local') next.localDeviceEnabled = enabled;else next.hosts.find((h) => h.id === id).enabled = enabled;
    try {await window.AieyesAgentSettings.commit(next);renderSettings(false);} catch (error) {notify(String(error), 'error');} finally
    {input.checked = id === 'local' ? state.settings.localDeviceEnabled !== false : state.settings.hosts.find((h) => h.id === id)?.enabled === true;input.disabled = false;}
  });
  document.querySelectorAll('[data-remove]').forEach((button) => button.onclick = () => {
    const kind = state.settingsTab,id = button.dataset.remove,item = draftSettings()[kind].find((row) => row.id === id);
    const linked = kind === 'hosts' ? draftSettings().sources.filter((src) => src.hostId === id) : [];
    const impact = linked.length ? `<p>${AieyesI18n.t("保存后，以下 {0} 个数据源将暂停，并需要重新选择服务器：", linked.length)}</p><ul>${linked.map((src) => `<li>${escapeHTML(src.name)}</li>`).join('')}</ul>` : AieyesI18n.t("<p>已导入的用量历史会保留。重新连接时需要再次添加配置。</p>");
    showEditor(kind === 'hosts' ? AieyesI18n.t("移除服务器") : AieyesI18n.t("移除数据源"), `<p>${AieyesI18n.t("确认移除「{0}」？", escapeHTML(item.name || item.target))}</p>${impact}`, async () => {
      const next = structuredClone(state.settings);next[kind] = next[kind].filter((row) => row.id !== id);
      if (kind === 'hosts') {
        for (const agent of next.agents ?? []) agent.machineIds = agent.machineIds.filter((machine) => machine !== id);
      }
      if (kind === 'sources') for (const account of next.accounts) if (account.quotaSourceId === id) account.quotaSourceId = null;
      await window.AieyesAgentSettings.commit(next);renderSettings(false);notify(linked.length ? AieyesI18n.t("已移除服务器，{0} 个关联数据源将暂停", linked.length) : AieyesI18n.t("已移除配置，历史记录将保留"));
    });
    const submit = $('#editor-form button[type=submit]');$('#editor').dataset.kind = 'confirmation';submit.textContent = AieyesI18n.t("确认移除");submit.classList.add('danger');
    $('#cancel-editor').focus();
  });
  if ($('#add-item')) $('#add-item').onclick = () => editItem();

  if ($('#general-form')) $('#general-form').onsubmit = (e) => {e.preventDefault();scheduleGeneralSave();};
  if ($('#history-rescan')) {
    updateRescanStatus();
    $('#history-rescan').onclick = () => {
      if (collectionBlocked()) return;
      showEditor(AieyesI18n.t("全量重扫历史记录"), AieyesI18n.t("<p>将从头读取所有已启用本机及服务器数据源的完整日志，修复可确认的历史重复。</p><p>已有历史和价格快照会保留；日志较多时可能需要较长时间。</p>"), () => {setTimeout(() => void rescan(), 0);});
      $('#editor').dataset.kind = 'confirmation';$('#editor-form button[type=submit]').textContent = AieyesI18n.t("开始重扫");
    };
  }
  window.AieyesUpdates?.bindSettings();
  if ($('#sync-prices')) $('#sync-prices').onclick = syncPrices;
  if ($('#reference-prices')) $('#reference-prices').onclick = () => referencePrices();
  if ($('#add-price')) $('#add-price').onclick = () => editPrice();
  bindPrices();bindPriceViewport();bindExistingMappings();
  if ($('#price-search')) $('#price-search').oninput = (e) => {state.priceSearch = e.target.value;$('.prices-list').scrollTop = 0;$('.prices-list').innerHTML = priceRows();bindPrices();};
  if ($('#field-appMode')) bindProxy('app');
  if ($('#mapping-form')) $('#mapping-form').onsubmit = (e) => {e.preventDefault();const f = new FormData(e.target);if (!f.get('mappingModel')?.trim() || !f.get('mappingId')?.trim()) {notify(AieyesI18n.t("请输入日志模型名称和 OpenRouter 模型 ID"), 'error');return;}job(AieyesI18n.t("保存模型映射"), async () => {const next = structuredClone(state.settings);mappingDraft(next);await saveSettings(next);$('#mapping-form').reset();resetSettingsForm('mapping-form');renderSettings();});};
  document.querySelectorAll('[data-unmap]').forEach((b) => b.onclick = () => job(AieyesI18n.t("移除模型映射"), async () => {const next = structuredClone(state.settings);delete next.modelMappings[b.dataset.unmap];await saveSettings(next);renderSettings(false);}));
  if ($('#reprice')) $('#reprice').onclick = () => job(AieyesI18n.t("保存并重算"), async () => {const button = $('#reprice');state.repricing = true;button.disabled = true;button.textContent = AieyesI18n.t("保存并重算中…");try {await saveAllSettings({ clearForms: false, refresh: false });await api('prices.recalculate');$('#mapping-form').reset();resetSettingsForm('mapping-form');await loadDashboard();renderSettings();notify(AieyesI18n.t("已保存并按当前价格重算"));} finally {state.repricing = false;const current = $('#reprice');if (current) {current.disabled = false;current.textContent = AieyesI18n.t("保存并重算");}}});
  restoreView(view);if (state.repricing && $('#reprice')) {$('#reprice').disabled = true;$('#reprice').textContent = AieyesI18n.t("保存并重算中…");}
  window.AieyesMotion?.didRender('settings:' + state.settingsTab + ':' + (state.settingsTab === 'general' ? state.generalTab : state.settingsTab === 'integrations' ? state.settingsAgent : ''), $('#settings-panel'));
}
let editorSelectors = [],editorSaving = false,editorOpener = null,editorOpenerSelector = null,editorBaseline = '',editorExtraSnapshot = () => null;
const editorSnapshot = () => JSON.stringify([AieyesUI.formValues($('#editor-form')), editorExtraSnapshot()]);
function clearEditorSelectors() {editorSelectors.forEach((c) => c.destroy());editorSelectors = [];}
function finishEditor() {
  clearEditorSelectors();$('.shell').inert = false;$('#editor-backdrop').hidden = true;document.body.classList.remove('editor-open');
  if (editorOpener || editorOpenerSelector) {const target = editorOpener?.isConnected ? editorOpener : editorOpenerSelector ? document.querySelector(editorOpenerSelector) : null;(target ?? $('#add-item') ?? $('#add-price') ?? $('.settings-tabs .active'))?.focus({ preventScroll: true });}
  editorOpener = null;editorOpenerSelector = null;updateSettingsSaveBar();
}
function closeEditor(force = false) {
  if (editorSaving) return;
  if (!force && editorSnapshot() !== editorBaseline) {
    let guard = $('#editor-discard');if (!guard) {guard = document.createElement('div');guard.id = 'editor-discard';guard.className = 'discard-guard';guard.setAttribute('role', 'alert');guard.innerHTML = AieyesI18n.t("<p>此编辑器有未保存内容。</p><button type=\"button\" id=\"editor-continue\">继续编辑</button><button type=\"button\" id=\"editor-confirm-discard\" class=\"danger\">放弃更改</button>");$('#editor-form').append(guard);$('#editor-continue').onclick = () => {guard.remove();$('#editor-fields input')?.focus();};$('#editor-confirm-discard').onclick = () => closeEditor(true);}$('#editor-continue').focus();return;
  }
  $('#editor').close();finishEditor();
}
function showEditor(title, html, onSubmit, kind = 'source') {
  if (editorSaving) return;
  $('#editor-discard')?.remove();editorExtraSnapshot = () => null;editorBaseline = '';editorOpener = document.activeElement;editorOpenerSelector = focusSelector(editorOpener);clearEditorSelectors();
  $('#editor').dataset.kind = kind;$('#editor-title').textContent = title;$('#editor-fields').innerHTML = html;
  $('#editor-error').hidden = true;$('#editor-error').textContent = '';
  $('#editor-form button[type=submit]').textContent = AieyesI18n.t("保存");$('#editor-form button[type=submit]').classList.remove('danger');
  $('#editor-form').onsubmit = async (e) => {
    e.preventDefault();if (editorSaving) return;
    const data = new FormData(e.target),controls = [...e.target.querySelectorAll('button,input,select,textarea')].map((control) => [control, control.disabled]);
    const submit = $('#editor-form button[type=submit]'),label = submit.textContent;
    editorSaving = true;controls.forEach(([control]) => control.disabled = true);submit.textContent = AieyesI18n.t("保存中…");
    $('#editor-error').hidden = true;let saved = false;
    try {await onSubmit(data);saved = true;}
    catch (error) {AieyesUI.showError($('#editor-error'), error.message ?? String(error), AieyesI18n.t("保存失败，输入已保留"));$('#editor-error').hidden = false;} finally
    {editorSaving = false;controls.forEach(([control, disabled]) => control.disabled = disabled);submit.textContent = label;}
    if (saved) closeEditor(true);else
    {$('#editor-error').tabIndex = -1;$('#editor-error').focus({ preventScroll: true });}
  };
  queueMicrotask(() => {editorBaseline = editorSnapshot();});
  window.AieyesSingleSelect?.mountAll($('#editor'));
  // Keep native-window controls interactive; only the application page is inert.
  $('.shell').inert = true;$('#editor-backdrop').hidden = false;document.body.classList.add('editor-open');$('#editor').show();
  $('#cancel-editor').focus();
}
document.addEventListener('keydown', (event) => {
  if (!$('#editor') || !$('#editor').open || event.defaultPrevented) return;
  if (event.key === 'Escape') {event.preventDefault();closeEditor();return;}
  if (event.key !== 'Tab') return;
  const controls = [...document.querySelectorAll('#titlebar:not([hidden]) button,#editor button,#editor input,#editor select,#editor textarea,#editor summary,#editor a[href],#editor [tabindex]')].filter((element) => {
    if (element.disabled || element.tabIndex < 0 || element.closest('[hidden],[inert]')) return false;
    for (let parent = element.parentElement; parent; parent = parent.parentElement) if (parent.tagName === 'DETAILS' && !parent.open && element !== parent.querySelector(':scope > summary')) return false;
    return element.checkVisibility ? element.checkVisibility({ checkVisibilityCSS: true }) : element.getClientRects().length > 0 && getComputedStyle(element).visibility !== 'hidden';
  });
  if (!controls.length) return;
  const index = controls.indexOf(document.activeElement),next = index < 0 ? event.shiftKey ? controls.length - 1 : 0 : (index + (event.shiftKey ? -1 : 1) + controls.length) % controls.length;
  event.preventDefault();controls[next].focus();
});
function editAccount(index) {
  if (index == null) window.AieyesAgentSettings.addAccount();else
  window.AieyesAgentSettings.openDetails(state.settings.accounts[index]);
}
function proxyParts(url) {
  try {
    const u = new URL(url);
    if (['http:', 'https:', 'socks5:', 'socks5h:'].includes(u.protocol) && !u.username && !u.password && (!u.pathname || u.pathname === '/') && !u.search && !u.hash) {
      return { protocol: u.protocol.slice(0, -1), host: u.hostname.replace(/^\[|\]$/g, ''), port: u.port || (u.protocol === 'http:' ? '80' : u.protocol === 'https:' ? '443' : '1080'), url };
    }
  } catch {}
  return { protocol: url ? 'url' : 'http', host: '127.0.0.1', port: '7890', url: url ?? '' };
}
function proxyFields(prefix, proxy, inherit = false) {
  const p = proxyParts(proxy?.url ?? '');
  return select(prefix + 'Mode', AieyesI18n.t("连接方式"), [...(inherit ? [['inherit', AieyesI18n.t("跟随应用")]] : []), ['system', AieyesI18n.t("系统代理")], ['direct', AieyesI18n.t("直连")], ['custom', AieyesI18n.t("指定代理")]], proxy?.mode ?? (inherit ? 'inherit' : 'system')) + `<div id="${prefix}-proxy-address">${select(prefix + 'Protocol', AieyesI18n.t("协议"), [['http', 'HTTP'], ['https', 'HTTPS'], ['socks5', 'SOCKS5'], ['socks5h', 'SOCKS5H'], ['url', AieyesI18n.t("自定义 URL")]], p.protocol)}<div id="${prefix}-proxy-basic">${field(prefix + 'Host', 'Host', p.host)}${field(prefix + 'Port', AieyesI18n.t("端口"), p.port, '', 'number')}</div><div id="${prefix}-proxy-url">${field(prefix + 'URL', AieyesI18n.t("代理 URL"), p.url)}</div></div><div class="actions"><button type="button" id="${prefix}-proxy-test">${AieyesI18n.t("测试连接")}</button><span id="${prefix}-proxy-result" role="status"></span></div>`;
}
function bindProxy(prefix) {
  const protocol = $('#field-' + prefix + 'Protocol');let previous = protocol.value;
  let revision = 0;const clearResult = () => {revision++;$('#' + prefix + '-proxy-result').textContent = '';};
  for (const name of [prefix + 'Mode', prefix + 'Protocol', prefix + 'Host', prefix + 'Port', prefix + 'URL', ...(prefix === 'app' ? ['proxyTestUrls'] : [])]) $('#field-' + name)?.addEventListener('input', clearResult);
  const update = () => {
    const active = $('#field-' + prefix + 'Mode').value === 'custom',custom = protocol.value === 'url';
    $('#' + prefix + '-proxy-address').hidden = !active;$('#' + prefix + '-proxy-basic').hidden = custom;$('#' + prefix + '-proxy-url').hidden = !custom;
  };
  $('#field-' + prefix + 'Mode').onchange = update;
  protocol.onchange = () => {
    if (protocol.value === 'url' && previous !== 'url') {
      try {const form = protocol.closest('form'),f = new FormData(form);f.set(prefix + 'Mode', 'custom');f.set(prefix + 'Protocol', previous);$('#field-' + prefix + 'URL').value = proxyValue(f, prefix).url;} catch {}
    }
    previous = protocol.value;update();
  };update();
  $('#' + prefix + '-proxy-test').onclick = async () => {
    const button = $('#' + prefix + '-proxy-test'),result = $('#' + prefix + '-proxy-result');button.disabled = true;result.textContent = AieyesI18n.t("测试中…");
    const testedRevision = revision;
    try {const form = protocol.closest('form'),f = new FormData(form);const proxy = proxyValue(f, prefix) ?? state.settings.proxy;
      const urls = prefix === 'app' ? f.get('proxyTestUrls')?.split(',').map((s) => s.trim()).filter(Boolean) : state.settings.proxyTestUrls;
      const snapshot = await api('network.test', { proxy, ...(urls ? { urls } : {}), force: true });if (revision === testedRevision) {result.textContent = AieyesNetwork.label(snapshot);result.title = AieyesNetwork.detail(snapshot);}
    } catch (error) {if (revision === testedRevision) AieyesUI.showError(result, error.message ?? String(error), AieyesI18n.t("连接测试失败"));} finally {button.disabled = false;}
  };
}
function proxyValue(f, prefix) {
  const mode = f.get(prefix + 'Mode');if (mode === 'inherit') return null;
  let url = f.get(prefix + 'URL') ?? '';
  if (mode === 'custom' && f.get(prefix + 'Protocol') !== 'url') {
    let host = f.get(prefix + 'Host').trim(),port = Number(f.get(prefix + 'Port'));
    if (!host || /[\s/@?#]/.test(host) || !Number.isInteger(port) || port < 1 || port > 65535) throw new Error(AieyesI18n.t("请输入有效代理地址和端口"));
    host = host.replace(/^\[|\]$/g, '');if (host.includes(':')) host = `[${host}]`;
    url = `${f.get(prefix + 'Protocol')}://${host}:${port}`;
  }
  return { mode, url };
}
function editItem(existing) {
  if (state.settingsTab === 'integrations') return window.AieyesAgentSettings.advanced(existing, existing?.provider || state.settingsAgent);
  const base = structuredClone(state.settings),item = structuredClone(existing ?? { id: crypto.randomUUID(), name: '', target: '', authMode: 'ssh', username: '', passwordRef: '', port: null, identityFile: '', shell: '/bin/bash', preCommand: '', enabled: true, metrics: Object.keys(groups), devices: [], details: Object.keys(detailOptions) });
  const body = (!existing ? AieyesI18n.t("<button type=\"button\" data-ssh-import>从 SSH 配置导入</button>") : '') + field('name', AieyesI18n.t("名称"), item.name) + field('target', AieyesI18n.t("SSH 别名或地址"), item.target, AieyesI18n.t("my-server 或 user@host")) + field('port', AieyesI18n.t("端口"), item.port ?? '', AieyesI18n.t("跟随 SSH 配置"), 'number') + select('authMode', AieyesI18n.t("登录方式"), [['ssh', AieyesI18n.t("SSH 配置 / 密钥")], ['password', AieyesI18n.t("用户名和密码")]], item.authMode ?? 'ssh') + field('username', AieyesI18n.t("用户名"), item.username ?? '', AieyesI18n.t("跟随地址或 SSH 配置")) + `<div id="host-password">${field('password', AieyesI18n.t("密码（留空保留）"), '', '', 'password')}</div><div id="host-key">${field('identityFile', AieyesI18n.t("密钥路径"), item.identityFile, AieyesI18n.t("跟随 SSH 配置"))}</div>` + `<section class="host-selectors"><div id="metric-select"></div><div class="section-head"><h3>${AieyesI18n.t("设备")}</h3><button type="button" id="discover-devices" title="${AieyesI18n.t("通过 SSH 读取远端设备列表")}">${AieyesI18n.t("读取设备")}</button></div><div id="discovery-status" class="error" role="status" hidden></div><div id="device-selects" class="device-selects"></div><div id="detail-select"></div></section><details class="advanced"><summary>${AieyesI18n.t("高级设置")}</summary>${field('shell', AieyesI18n.t("远程 Shell"), item.shell) + field('devices', AieyesI18n.t("设备表达式"), item.devices.join(', '), 'network:eth0, gpu:0, filesystems:/')}</details>`;
  showEditor(item.id === 'local' ? AieyesI18n.t("本机监测") : AieyesI18n.t("SSH 主机"), body, async (f) => {
    const next = structuredClone(base),host = structuredClone(item);
    for (const k of ['name', 'target', 'identityFile', 'shell', 'authMode', 'username']) host[k] = f.get(k);
    host.port = f.get('port') ? Number(f.get('port')) : null;host.devices = f.get('devices').split(',').map((s) => s.trim()).filter(Boolean);if (!host.name) host.name = host.target;
    let created;
    try {
      if (host.id !== 'local' && host.authMode === 'password' && f.get('password')) {
        const saved = await api('hosts.credentials.save', { password: f.get('password') });
        if (!saved.passwordRef) throw new Error(AieyesI18n.t("未获得密码引用"));
        created = saved.passwordRef;host.passwordRef = created;
      }
      if (host.id === 'local') next.localMonitor = { enabled: host.enabled, metrics: host.metrics, devices: host.devices, details: host.details };else {const index = next.hosts.findIndex((h) => h.id === host.id);if (index < 0) next.hosts.push(host);else next.hosts[index] = host;}
      await window.AieyesAgentSettings.commit(next, base);
    } catch (error) {
      if (created) await api('hosts.credentials.delete', { passwordRef: created }).catch(() => {});
      throw error;
    }
    renderSettings(false);
  });
  $('#editor').dataset.kind = 'host';
  if (item.id === 'local') for (const key of ['name', 'target', 'port', 'authMode', 'username', 'password', 'identityFile', 'shell']) {const el = $('#field-' + key);if (el) el.closest('.form-row').hidden = true;}
  const selectors = $('.host-selectors'),advanced = document.createElement('details');advanced.className = 'monitor-options';advanced.open = Boolean(existing);advanced.innerHTML = AieyesI18n.t("<summary>监控指标与设备 · 按需选择</summary>");selectors.before(advanced);advanced.append(selectors);
  const connect = $('#discover-devices');advanced.before(connect);connect.textContent = AieyesI18n.t("连接并读取设备");
  bindPathSelection(false);bindHostSelectors(item);
  const auth = () => {$('#host-password').hidden = $('#field-authMode').value !== 'password';$('#host-key').hidden = !$('#host-password').hidden;};$('#field-authMode').onchange = auth;auth();
}
function bindPathSelection(source) {
  const input = source ? $('#field-path') : $('#field-identityFile');if (!input) return;
  const row = input.closest('.form-row'),controls = document.createElement('div');controls.className = 'path-actions';controls.innerHTML = AieyesI18n.t("<button type=\"button\" class=\"path-pick\">选择…</button><button type=\"button\" class=\"path-check\">检测路径</button><span class=\"path-status\" role=\"status\"></span>");row.append(controls);
  const remote = () => source && Boolean($('#field-hostId')?.value);
  controls.querySelector('.path-pick').onclick = async () => {try {const result = await window.__TAURI__.core.invoke('select_local_path', { directory: source && $('#field-provider')?.value !== 'deepseek', initial: input.value });if (result) {input.value = result;input.dispatchEvent(new Event('input', { bubbles: true }));}} catch (e) {controls.querySelector('.path-status').textContent = AieyesI18n.message(String(e));}};
  controls.querySelector('.path-check').onclick = async () => {const status = controls.querySelector('.path-status');try {status.textContent = (await window.__TAURI__.core.invoke('check_local_path', { path: input.value })) ? AieyesI18n.t("路径存在") : AieyesI18n.t("路径不存在，请检查");} catch (e) {status.textContent = AieyesI18n.message(String(e));}};
  const update = () => {controls.querySelectorAll('button').forEach((b) => b.hidden = remote());controls.querySelector('.path-status').textContent = remote() ? AieyesI18n.t("远程路径 · ") + (draftSettings().hosts.find((h) => h.id === $('#field-hostId').value)?.name ?? AieyesI18n.t("SSH 主机")) : '';};$('#field-hostId')?.addEventListener('change', update);update();
}
function bindHostSelectors(item) {
  const selectors = AieyesSelect;editorExtraSnapshot = () => [item.metrics, item.details];
  let discovered = {},deviceControls = [],discoveryRevision = 0;
  const discoveryIdentity = () => JSON.stringify(['target', 'port', 'identityFile', 'shell', 'authMode', 'username', 'password'].map((k) => $('#field-' + k)?.value));
  for (const key of ['target', 'port', 'identityFile', 'shell', 'authMode', 'username', 'password']) $('#field-' + key)?.addEventListener('input', () => {discoveryRevision++;discovered = {};redrawDevices();$('#discovery-status').hidden = false;$('#discovery-status').textContent = AieyesI18n.t("连接配置已更改，请重新读取设备");});
  const tokens = () => selectors.parse($('#field-devices').value);
  const detailGroup = { cpuTimes: 'cpu', memoryCache: 'memory', swap: 'memory', fsAvailable: 'filesystems', fsType: 'filesystems', inodes: 'filesystems', diskIops: 'disk', diskBusy: 'disk', networkTotals: 'network', networkErrors: 'network', gpuMemory: 'gpu', gpuThermals: 'gpu' };
  item.details ??= Object.keys(detailOptions);
  function redrawDevices() {
    deviceControls.forEach((c) => {c.destroy();editorSelectors.splice(editorSelectors.indexOf(c), 1);});
    deviceControls = [];$('#device-selects').replaceChildren();
    for (const [group, title] of Object.entries(groups).filter(([g]) => g !== 'memory')) {
      const root = document.createElement('div');$('#device-selects').append(root);
      const options = () => {
        const rows = (discovered[group] ?? []).filter((d) => group !== 'cpu' || d.id !== 'cpu');
        const known = new Set(rows.map((d) => d.id));
        return [...rows.map((d) => ({ id: d.id, type: d.type, label: [d.id, d.type ?? d.name].filter(Boolean).join(' · ') })),
        ...selectors.deviceIds(tokens(), group).filter((id) => !known.has(id)).map((id) => ({ id, label: id, unavailable: true }))];
      };
      const control = selectors.mount(root, { title, options, selected: () => group === 'filesystems' && !tokens().some((s) => s.startsWith('filesystems:')) ? options().filter((o) => selectors.recommendedFilesystem(o.id, o.type)).map((o) => o.id) : selectors.deviceSelected(tokens(), group, options().map((o) => o.id)), all: () => tokens().includes(group + ':__all__') || group !== 'filesystems' && !tokens().some((s) => s.startsWith(group + ':')), disabled: () => !item.metrics.includes(group),
        onChange: (selected, { all }) => {$('#field-devices').value = selectors.writeDevices(tokens(), group, selected, all).join(', ');} });
      deviceControls.push(control);editorSelectors.push(control);
    }
  }
  const metrics = selectors.mount($('#metric-select'), { title: AieyesI18n.t("采集项目"), options: () => Object.entries(groups).map(([id, label]) => ({ id, label })), selected: () => item.metrics,
    onChange: (values) => {item.metrics = values;deviceControls.forEach((c) => c.refresh());details.refresh();} });
  // Disabled categories retain their saved detail values; only enabled categories are editable.
  const details = selectors.mount($('#detail-select'), { title: AieyesI18n.t("显示细分项"), options: () => Object.entries(detailOptions).filter(([id]) => id === 'uptime' || item.metrics.includes(detailGroup[id])).map(([id, label]) => ({ id, label })), selected: () => item.details,
    onChange: (values) => {item.details = values;} });
  editorSelectors.push(metrics, details);redrawDevices();
  $('#field-devices').onchange = redrawDevices;
  $('#discover-devices').onclick = async () => {
    const button = $('#discover-devices'),status = $('#discovery-status');
    button.disabled = true;button.textContent = AieyesI18n.t("读取中…");status.hidden = true;
    const form = $('#editor-form'),f = new FormData(form),host = { ...item },identity = discoveryIdentity(),revision = ++discoveryRevision;
    for (const key of ['target', 'identityFile', 'shell', 'authMode', 'username']) host[key] = f.get(key);
    host.port = f.get('port') ? Number(f.get('port')) : null;
    let temporaryReference;
    try {
      if (host.authMode === 'password' && f.get('password')) {const saved = await api('hosts.credentials.save', { password: f.get('password') });temporaryReference = saved.passwordRef;host.passwordRef = temporaryReference;}
      const sample = await api('hosts.discover', { host });
      if (!$('#editor').open || !button.isConnected || revision !== discoveryRevision || identity !== discoveryIdentity()) return;
      for (const group of Object.keys(groups)) if (Array.isArray(sample[group])) discovered[group] = sample[group];
      redrawDevices();const options = $('.monitor-options');if (options) options.open = true;
      status.textContent = Object.keys(sample.errors ?? {}).map((k) => (groups[k] ?? k) + AieyesI18n.t("读取失败")).join(' · ');if (!status.textContent) status.textContent = AieyesI18n.t("从 ") + host.target + AieyesI18n.t(" 读取 · ") + timeLabel(Date.now());status.hidden = false;
    } catch (error) {if (button.isConnected && revision === discoveryRevision && identity === discoveryIdentity()) {AieyesUI.showError(status, String(error), AieyesI18n.t("设备读取失败"));status.hidden = false;}} finally
    {if (temporaryReference) await api('hosts.credentials.delete', { passwordRef: temporaryReference }).catch(() => {});button.disabled = false;button.textContent = AieyesI18n.t("读取设备");}
  };
}
function editPrice(existing) {const price = structuredClone(existing ?? { id: '', name: '', input: null, output: null, cacheRead: null, cacheWrite: null, fetchedAt: 0 });
  showEditor(AieyesI18n.t("模型价格 · USD / 百万 Token"), field('id', AieyesI18n.t("模型 ID"), price.id) + field('name', AieyesI18n.t("显示名称"), price.name) + [['input', AieyesI18n.t("输入")], ['output', AieyesI18n.t("输出")], ['cacheRead', AieyesI18n.t("缓存读取")], ['cacheWrite', AieyesI18n.t("缓存写入")]].map(([k, label]) => field(k, label, price[k] == null ? '' : price[k] * 1e6, AieyesI18n.t("待定价"))).join(''), async (f) => {price.id = f.get('id');price.name = f.get('name');for (const k of ['input', 'output', 'cacheRead', 'cacheWrite']) {const v = f.get(k);price[k] = v === '' ? null : Number(v) / 1e6;if (price[k] != null && (!Number.isFinite(price[k]) || price[k] < 0)) throw new Error(AieyesI18n.t("价格需要为非负数"));}await api('prices.save', { prices: [price] });state.prices = await api('prices.list');await loadDashboard();renderSettings();markSaved();});
  $('#editor').dataset.kind = 'price';
}
if (!PANEL) {
  const toggle = $('#sidebar-toggle');
  const update = () => {const saved = localStorage.getItem('sidebar.collapsed'),collapsed = saved == null ? innerWidth < (document.documentElement.dataset.platform === 'windows' ? 974 : 1000) : saved === 'true';document.body.classList.toggle('sidebar-collapsed', collapsed);toggle.setAttribute('aria-expanded', String(!collapsed));toggle.title = collapsed ? AieyesI18n.t("展开导航") : AieyesI18n.t("收起导航");toggle.setAttribute('aria-label', toggle.title);};toggle.onclick = () => {localStorage.setItem('sidebar.collapsed', String(!document.body.classList.contains('sidebar-collapsed')));if (window.AieyesMotion) AieyesMotion.sidebar(update);else update();};addEventListener('resize', update);update();
}
document.querySelectorAll('[data-page]').forEach((button) => button.onclick = () => {
  if (button.dataset.page === 'settings') {void openSettings();return;}
  void navigatePage(button.dataset.page);
});
if (!PANEL) {
  document.querySelectorAll('aside [data-settings-tab]').forEach((button) => button.onclick = () => openSettings({ category: button.dataset.settingsTab }));
  document.querySelectorAll('aside [data-general-page]').forEach((button) => button.onclick = () => openSettings({ category: 'general', section: button.dataset.generalPage }));
  const buttons = [...document.querySelectorAll('aside nav button')];
  buttons.forEach((button, index) => button.onkeydown = (event) => {
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : ['ArrowDown', 'ArrowRight'].includes(event.key) ? (index + 1) % buttons.length : ['ArrowUp', 'ArrowLeft'].includes(event.key) ? (index + buttons.length - 1) % buttons.length : null;
    if (next == null) return;event.preventDefault();buttons[next].focus();buttons[next].click();
  });
}
if ($('#editor')) {
  $('#editor').addEventListener('close', () => {if (!$('#editor').open) finishEditor();});
  $('#editor').addEventListener('cancel', (e) => {e.preventDefault();closeEditor();});
  $('#close-editor').onclick = $('#cancel-editor').onclick = () => closeEditor();
}
if (!PANEL) bindRefreshMenu('refresh', 'refresh-menu');
$('#activity').onclick = () => state.page === 'servers' ? sample() : scan();$('#activity').onkeydown = (e) => {if (e.key === 'Enter' || e.key === ' ') {e.preventDefault();state.page === 'servers' ? sample() : scan();}};
document.addEventListener('keydown', (e) => {if (!(e.ctrlKey || e.metaKey) || $('#editor')?.open || $('#quota-dialog')?.open) return;if (e.key.toLowerCase() === 's' && state.page === 'settings' && !PANEL) {e.preventDefault();if (settingsFormsDirty()) job(AieyesI18n.t("保存应用配置"), () => saveAllSettings());return;}if (e.key === ',') {e.preventDefault();void openSettings().catch((error) => notify(String(error), 'error'));}if (e.shiftKey && e.key.toLowerCase() === 'd') {e.preventDefault();if (PANEL) openDetail();else {void navigatePage('agent');}}});
// Error text remains selectable; only the explicit close control dismisses it.
new ResizeObserver(() => {if (state.page === 'analysis') drawTrend();}).observe($('#content'));
function syncSharedAppearance(appearance) {
  if (!state.settings || !appearance) return;
  const previous = state.settings.appearance ?? { theme: 'system', accent: 'indigo' };
  state.settings.appearance = { ...appearance };
  if (state.settingsDraft) {
    const draft = state.settingsDraft.appearance ?? { ...previous };
    for (const key of ['theme', 'accent', 'chartPalette', 'chartContrast', 'language']) if (draft[key] === previous[key]) draft[key] = appearance[key];
    state.settingsDraft.appearance = draft;state.settingsBaseline = configKey(state.settings);
  }
  const form = $('#general-form');
  if (form) captureSettingsForms();
  const record = settingsForms.get('general-form:appearance');
  if (record) {
    for (const key of ['theme', 'accent', 'chartPalette', 'chartContrast', 'language']) {
      const name = 'appearance' + key[0].toUpperCase() + key.slice(1);
      if (record.values[name] === record.baseline[name]) record.values[name] = record.baseline[name] = appearance[key];
    }
    if (form && settingsFormKey(form) === 'general-form:appearance') {
      AieyesUI.restoreForm(form, record.values);form.dataset.draftBaseline = JSON.stringify(record.baseline);
    }
  }
}
window.addEventListener('aieyes:appearance', (event) => {
  // Adopt a sibling window's saved preference before rendering from local state.
  syncSharedAppearance(event.detail);
  updateChartPalettePreview();
  if (['agent', 'analysis'].includes(state.page) && state.dashboard) {if (interactionOpen()) pendingRender = true;else render();}
});
async function toggleTheme() {
  if (!state.settings || state.themeSaving || state.settingsSaving || state.settingsCommitting) return;
  state.themeSaving = true;window.AieyesTheme.setBusy(true);
  try {
    const base = structuredClone(state.settings),next = structuredClone(base);
    next.appearance = { ...(base.appearance ?? { accent: 'indigo' }), theme: document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark' };
    const saved = await api('settings.patch', { base, settings: next });
    syncSharedAppearance(saved.appearance);window.AieyesTheme.apply(saved.appearance);
    if ($('#message')?.dataset.source === 'theme') notify('');
  } catch (error) {notify(AieyesI18n.t("主题保存失败：") + String(error), 'error', 'theme');} finally
  {state.themeSaving = false;window.AieyesTheme.setBusy(false);}
}
document.addEventListener('click', (event) => {if (event.target.closest('[data-theme-toggle]')) void toggleTheme();});
let externalRefresh = null,externalPending = false,externalSettings = false,externalQuotaCheck = false,externalScanCheck = false;
function refreshExternalData(settings = false, method = '') {
  externalPending = true;externalSettings ||= settings;externalQuotaCheck ||= method === 'quotas.refresh';externalScanCheck ||= method === 'sources.scan' || method === 'sources.rescan';
  if (externalRefresh) return externalRefresh;
  externalRefresh = (async () => {
    while (externalPending) {
      externalPending = false;const readSettings = externalSettings,checkQuotas = externalQuotaCheck,checkScan = externalScanCheck;externalSettings = false;externalQuotaCheck = false;externalScanCheck = false;
      if (readSettings || !state.settings) {const saved = await api('settings.get');syncSharedAppearance(saved.appearance);if (window.AieyesAgentSettings) window.AieyesAgentSettings.accept(saved);else acceptSettings(saved);window.AieyesTheme?.apply(saved.appearance);}migrateProviderState();
      await loadDashboard();
      if (readSettings && PANEL && state.page !== 'agent') render();
      const scannedSources = (state.dashboard?.sources ?? []).filter((source) => state.settings.sources.some((row) => row.id === source.id && row.provider !== 'deepseek' && AieyesUI.sourceEnabled(state.settings, row))).map((source) => ({ id: source.id, error: source.status?.error, partial: source.status?.partial, issues: source.status?.issues }));
      // A filtered snapshot is not an action result. Shared feedback owns busy,
      // success time and targeted failures; invalidation must never replace it.
      for (const [key, check, rows] of [['quotas', checkQuotas, state.dashboard?.quotas], ['scan', checkScan, scannedSources]]) {
        if (check && !state.refreshState[key]) {const error = batchError(rows, refreshLabels[key]);if (error) showFailures(error, key);}
      }
      if (checkQuotas) state.quotaError = state.refreshState.quotas?.error ? AieyesI18n.t("部分账户读取失败，请逐项重试。") : '';
      renderFailures();updateActivity();
    }
  })().catch((error) => notify(String(error), 'error')).finally(() => {externalRefresh = null;resumeSharedRefresh();});
  return externalRefresh;
}
async function listenForSharedState() {
  const events = window.__TAURI__?.event;if (!events?.listen) return;
  state.sharedEstimateOperations = new Set();
  await events.listen('operations:busy', ({ payload }) => {
    if (payload.busy) state.sharedEstimateOperations.add(payload.operationId);else
    state.sharedEstimateOperations.delete(payload.operationId);scheduleRefreshes();
  });
  await events.listen('desktop:refresh-status', ({ payload }) => {
    if (!refreshLabels[payload.key]) return;
    const previous = state.refreshState[payload.key] ?? {};
    if (payload.noAttempt) {state.refreshState[payload.key] = { ...previous, busy: false };updateActivity();return;}
    if (payload.busy) state.refreshState[payload.key] = { ...previous, busy: true };else
    {if (payload.key === 'hosts') state.hostRefreshItem = payload.itemId ?? null;state.refreshState[payload.key] = { ...previous, busy: false, success: payload.success ?? previous.success };showFailures(payload.failures?.length ? { failures: payload.failures } : null, payload.key, payload.itemId ?? null);}
    updateActivity();
  });
  await events.listen('desktop:status', ({ payload }) => {
    state.sessionSnapshot = payload;window.AieyesAgentSettings?.updateSession(payload);
    const opened = !state.panelOpen && payload.panelOpen;state.panelOpen = Boolean(payload.panelOpen);state.panelPage = payload.panelPage ?? state.panelPage;scheduleRefreshes();if (opened && state.dashboardDirty) {state.dashboardDirty = false;void refreshExternalData(true);}
  });
  const revisions = runtimeVersions,seenOperations = new Set();
  let sharedRefreshTimer,sharedSettings = false,sharedPending = false,sharedReady = false,sharedMethod = '';
  resumeSharedRefresh = () => {
    if (!sharedPending || !sharedReady || externalRefresh || state.busy || state.quotaBusy || state.estimateBusy || state.settingsSaving || state.settingsCommitting || state.refreshState.quotas?.busy) return;
    const readSettings = sharedSettings,method = sharedMethod;
    sharedPending = false;sharedReady = false;sharedSettings = false;sharedMethod = '';
    if (readSettings || lastDashboardVersion !== dashboardVersion()) void refreshExternalData(readSettings, method).finally(resumeSharedRefresh);
  };
  const queueSharedRefresh = (settings, method = '') => {
    sharedSettings ||= settings;sharedMethod = method || sharedMethod;sharedPending = true;sharedReady = false;
    clearTimeout(sharedRefreshTimer);sharedRefreshTimer = setTimeout(() => {sharedReady = true;resumeSharedRefresh();}, 120);
  };
  const rememberOperation = (id) => {if (id) {seenOperations.add(id);if (seenOperations.size > 256) seenOperations.delete(seenOperations.values().next().value);}};
  await events.listen('desktop:invalidate', ({ payload }) => {
    if (!state.revisionEvents || ownedOperations.has(payload.operationId) || seenOperations.has(payload.operationId)) return;
    rememberOperation(payload.operationId);
    if (document.hidden || PANEL && !state.panelOpen) {state.dashboardDirty = true;return;}
    queueSharedRefresh(window.AieyesContract.methods[payload.method]?.invalidates.includes('settings'), payload.method);
  });
  await events.listen('runtime:changed', ({ payload }) => {
    if (!state.revisionEvents || !payload.revisions) return;
    const changed = Object.keys(payload.revisions).filter((key) => payload.revisions[key] > (revisions[key] ?? -1));
    for (const key of changed) revisions[key] = payload.revisions[key];
    if (!changed.length) return;
    if (changed.some((key) => ['settings', 'usage', 'prices'].includes(key))) clearUsageSnapshots();
    if (ownedOperations.has(payload.operationId) || seenOperations.has(payload.operationId) || !state.settings) return;
    rememberOperation(payload.operationId);
    if (changed.includes('accounts')) void window.AieyesAgentSettings?.refreshStatuses(false);
    if (document.hidden || PANEL && !state.panelOpen) {state.dashboardDirty = true;return;}
    if (changed.some((key) => ['settings', 'usage', 'quotas', 'estimates', 'prices', 'sources'].includes(key))) queueSharedRefresh(changed.includes('settings'));
  });
  await events.listen('desktop:settings', () => {if (state.revisionEvents) return;invalidateQuotaSchedule();window.AieyesNetwork?.invalidate();if (PANEL) return refreshExternalData(true);});
  await events.listen('desktop:quotas', ({ payload }) => applyQuotaSamples(payload));
  await events.listen('desktop:hosts', ({ payload }) => {applyHostSamples(payload);});
  await events.listen('hosts:changed', ({ payload }) => {applyHostMonitor(payload);});
  await events.listen('desktop:hosts-error', ({ payload }) => {applyHostFailure(payload, state.hostRefreshItem);});
  await events.listen('desktop:data-changed', ({ payload }) => {if (state.revisionEvents) return;invalidateQuotaSchedule();
    if (PANEL && window.AieyesContract.methods[payload]?.invalidates.includes('settings')) return; // desktop:settings owns this invalidation.
    if (PANEL || !state.busy && !state.settingsSaving && !state.settingsCommitting && !editorSaving) return refreshExternalData(window.AieyesContract.methods[payload]?.invalidates.includes('settings'), payload);
  });
}
function migrateProviderState() {
  AieyesUI.migrateProviderPreferences(state.settings);
  state.provider = AieyesUI.agentProvider(state.provider);
  state.accountKey = AieyesUI.canonicalAccountKey(state.accountKey, state.settings.accountAliases);
}
async function boot() {
  try {
    if (!window.__TAURI__) throw new Error(AieyesI18n.t("通过 Aieyes 桌面应用打开"));
    await listenForSharedState();
    const info = await api('hello');state.revisionEvents = Boolean(info.runtimeInvalidations);state.hostMonitoring = Boolean(info.hostMonitoring);const version = $('#app-version');if (version) version.textContent = info.version;
    if (info.contractHash && info.contractHash !== window.AieyesContract.contractHash) throw new Error(AieyesI18n.t("应用资源与核心版本不一致，请重新安装同一版本"));
    if (PANEL && state.revisionEvents) {const shell = await window.__TAURI__.core.invoke('desktop_info');state.panelOpen = Boolean(shell.panelOpen);state.panelPage = shell.panelPage ?? state.panelPage;}
    state.settings = await api('settings.get');migrateProviderState();AieyesUI.observePanelAccounts(state.settings);void window.AieyesAgentSettings?.refreshStatuses(true, null, true);await loadDashboard();
    if (!PANEL) window.AieyesSetup?.maybeStart();
    if (PANEL) {
      // Restoring before bundled fonts settle lets scroll anchoring shift the saved position.
      await window.AieyesTheme?.loadFonts?.();
      if (document.fonts) await document.fonts.ready;
      render();updateActivity();
      await new Promise((resolve) => requestAnimationFrame(() => {restoreView(viewPreferences.position(viewKey()));resolve();}));
      return;
    }
    await scan();if (hasQuotaSources()) await quotas(undefined, true);
  } catch (error) {notify(String(error), 'error');$('#activity').textContent = AieyesI18n.t("连接已断开");$('#activity').dataset.status = 'error';}
}
window.AieyesApp = {
  openSettings,
  navigatePage,
  openAnalysis,
  flushRender,
  async openSetupInMain(kind) {if (!state.settings) await appReady;await openSetup(['hosts', 'quota'].includes(kind) ? kind : 'sources');},
  async refreshPanel() {await appReady;await refreshExternalData(true);if (state.page === 'servers' && !state.hosts.length) await sample();},
  applyHostSamples,
  openPanelAccounts
};
// Quota polling belongs to the native host, including while all webviews are hidden.
let refreshTimer,refreshStopped = false;
function invalidateQuotaSchedule() {scheduleRefreshes();}
function foregroundMetrics() {return state.page === 'servers' && !document.hidden || state.panelOpen && state.panelPage === 'servers';}
function metricsDeadline() {return state.lastMetrics + (foregroundMetrics() ? state.settings.serverForegroundRefreshSeconds ?? 2 : state.settings.serverRefreshSeconds) * 1000;}
function collectionBlocked() {return state.settingsCommitting || window.AieyesUpdates?.busy || $('#editor')?.open || state.estimateBusy || state.sharedEstimateOperations?.size || $('#quota-dialog')?.dataset.busy || state.busy || state.settingsSaving || ['scan', 'rescan', 'prices'].some((key) => state.refreshState[key]?.busy);}
function quotaCollectionBlocked() {return state.settingsCommitting || window.AieyesUpdates?.busy || state.quotaBusy || state.settingsSaving || state.refreshState.quotas?.busy;}
function scheduleRefreshes() {
  resumeSharedRefresh();
  configureHostMonitor();
  clearTimeout(refreshTimer);
  if (PANEL || refreshStopped || !state.settings || window.AieyesUpdates?.busy) return;
  const now = Date.now();
  for (const key of ['lastScan', 'lastMetrics', 'lastQuota']) if (state[key] > now) state[key] = 0;
  const deadlines = [];
  if (!state.hostMonitoring && !state.serverBusy && !state.refreshState.hosts?.busy && monitoredHosts().some((h) => h.enabled)) deadlines.push(metricsDeadline());
  if (state.hostMonitoring && hostMonitorRetryAt > now) deadlines.push(hostMonitorRetryAt);
  if (!collectionBlocked()) {
    deadlines.push(state.lastScan + (state.settings.historyRefreshSeconds ?? Math.max(10, state.settings.refreshSeconds)) * 1000);
  }
  const next = Math.min(...deadlines.filter(Number.isFinite));
  if (Number.isFinite(next)) refreshTimer = setTimeout(runScheduledRefreshes, Math.max(1, next - Date.now()));
}
function runScheduledRefreshes() {
  if (PANEL || refreshStopped || !state.settings || window.AieyesUpdates?.busy) return;
  if (!state.hostMonitoring && !state.serverBusy && !state.refreshState.hosts?.busy && monitoredHosts().some((h) => h.enabled) && Date.now() >= metricsDeadline()) void sample();
  if (!collectionBlocked() && Date.now() >= state.lastScan + (state.settings.historyRefreshSeconds ?? Math.max(10, state.settings.refreshSeconds)) * 1000) void scan();
  scheduleRefreshes();
}
document.addEventListener('visibilitychange', () => {if (!document.hidden) {invalidateQuotaSchedule();if (state.dashboardDirty) {state.dashboardDirty = false;void refreshExternalData(true);}} else scheduleRefreshes();});
window.addEventListener('focus', invalidateQuotaSchedule);
document.addEventListener('close', scheduleRefreshes, true);
window.addEventListener('beforeunload', () => {rememberPage();viewPreferences.save(dashboardScope(), state);refreshStopped = true;clearTimeout(refreshTimer);clearTimeout(dashboardTimer);});
// Age visible samples even while an RPC is slow or the panel receives no new events.
setInterval(updateHostStatuses, 1000);
function updateResetDisplays(now = Date.now() / 1000) {for (const el of document.querySelectorAll('[data-reset-at]')) el.textContent = AieyesUI.quotaReset({ resetsAt: Number(el.dataset.resetAt), windowMinutes: Number(el.dataset.windowMinutes), usedPercent: el.dataset.usedPercent?.trim() ? Number(el.dataset.usedPercent) : undefined }, now);}
setInterval(() => {updateResetDisplays();updateActivity();}, 30000);
function estimateValues(e, q) {
  const visible = AieyesUI.estimateVisibility(q, [e], e.groupId),rows = [
    ...(visible.five ? [[AieyesI18n.t("本段 5h API 等价值"), e.fiveHourValue]] : []),
    ...(visible.direct ? [[e.valuationMode === 'fiveHour' ? AieyesI18n.t("7d 推算 · 同期消耗比例") : AieyesI18n.t("7d 直接估值"), e.weeklyDirectValue ?? e.weeklyValue]] : []),
    ...(visible.ratio ? [[AieyesI18n.t("7d 推算 · 近期容量倍率"), e.weeklyRatioValue]] : [])];

  const capacity = e.capacity,old = capacity?.updatedAt && Date.now() / 1000 - capacity.updatedAt > 14 * 86400;
  return rows.map(([label, value]) => `<div class="between"><span>${label}</span><strong>${value == null ? '—' : money(value) + ' USD'}</strong></div>`).join('') + (visible.ratio ? `<p class="tiny muted">${capacity?.ratio != null ? AieyesI18n.t("容量倍率 ") + capacity.ratio.toFixed(2) + ' · ' + (old ? AieyesI18n.t("历史倍率") : capacity.weeklyPercent < 5 ? AieyesI18n.t("倍率样本较少") : AieyesI18n.t("近期倍率")) + ' · ' + capacity.samples + AieyesI18n.t(" 个样本 · ") + date(capacity.updatedAt) : AieyesI18n.t("倍率学习中")}</p>` : '');
}
function estimateCalculation(e, q) {
  if (e.kind === 'credits') return AieyesI18n.t("1000 credit 的 API 等价价值 = 样本成本 × 1000 ÷ 消耗 credit");
  const visible = AieyesUI.estimateVisibility(q, [e], e.groupId);
  return [visible.five ? AieyesI18n.t("5h 价值 = 有效成本 × 100 ÷ 5h 消耗百分点") : '', visible.direct ? AieyesI18n.t("7d 直接估值 = 样本 API 等价成本 × 100 ÷ 周额度消耗百分点") : '', visible.ratio ? AieyesI18n.t("7d 倍率估算 = 5h 价值 × 近期容量倍率") : ''].filter(Boolean).join('；') || AieyesI18n.t("按采样时段的 API 等价成本与对应额度消耗比例计算");
}
function estimateStatus(e, dashboard = state.dashboard) {
  const live = AieyesUI.automaticEstimateState(e, state.settings, dashboard);
  if (live) return (e.samplingMode === 'rolling' ? AieyesI18n.t("滚动估值") + ' · ' : AieyesI18n.t("自动估值 · ")) + live.label;
  if (e.samplingMode === 'automatic') return AieyesI18n.t("自动估值 · 已封存");
  return e.status === 'active' ? e.fiveHourValue != null || e.weeklyValue != null || e.valuePer1000 != null ? AieyesI18n.t("可输出估值 · 采样继续") : AieyesI18n.t("正在积累样本") : {pending: AieyesI18n.t("需要确认"), completed: AieyesI18n.t("已结束")}[e.status] ?? e.status;
}
function quotaDialog(title, body, layout = 'estimate') {
  let dialog = $('#quota-dialog');if (!dialog) {dialog = document.createElement('dialog');dialog.id = 'quota-dialog';dialog.setAttribute('aria-labelledby', 'quota-dialog-title');document.body.append(dialog);}
  dialog.innerHTML = `<div class="dialog-head"><h2 id="quota-dialog-title">${escapeHTML(title)}</h2><button type="button" id="quota-close" aria-label="${AieyesI18n.t("关闭")}">${uiIcon('x')}</button></div><div class="quota-dialog-body">${body}<div id="quota-tool-error" class="error" role="alert" hidden></div></div>`;
  delete dialog.dataset.estimateKey;delete dialog.dataset.records;delete dialog.dataset.autoSources;delete dialog.dataset.loadingRequest;delete dialog.dataset.valuationProvider;
  delete dialog.dataset.kind;
  dialog.dataset.layout = layout;
  $('#quota-close').onclick = () => dialog.close();if (!dialog.open) {dialog.dataset.opener = focusSelector(document.activeElement) ?? '';dialog.onclose = () => {flushRender();if (dialog.dataset.opener) {const opener = document.querySelector(dialog.dataset.opener);(opener?.getClientRects().length ? opener : $('#panel-more > summary'))?.focus({ preventScroll: true });}};dialog.showModal();}return dialog;
}
async function quotaAction(action, params) {
  if (state.estimateBusy) return false;
  const dialog = $('#quota-dialog'),controls = [...dialog.querySelectorAll('button:not(#quota-close),input,select')].map((el) => [el, el.disabled]);
  dialog.dataset.busy = 'true';state.estimateBusy = true;controls.forEach(([el]) => el.disabled = true);dialog.oncancel = null;$('#quota-tool-error').hidden = true;
  const operationId = crypto.randomUUID(),started = Date.now();
  state.estimateOperation = { key: dialog.dataset.estimateKey, kind: dialog.dataset.estimateKind, group: dialog.dataset.estimateGroup, status: 'running' };
  let background = $('#background-task');if (!background) {background = document.createElement('button');background.id = 'background-task';background.setAttribute('role', 'status');document.body.append(background);}background.hidden = false;background.onclick = () => {if (state.estimateBusy || state.estimateOperation?.status === 'error') {if (!dialog.open) dialog.showModal();} else if (state.estimateOperation?.key) {openQuotaEstimate(state.estimateOperation.key, state.estimateOperation.kind, state.estimateOperation.group);background.hidden = true;} else {openSampling();background.hidden = true;}};
  let progress = dialog.querySelector('#quota-progress');if (!progress) {progress = document.createElement('p');progress.id = 'quota-progress';progress.setAttribute('role', 'status');dialog.querySelector('.quota-dialog-body').append(progress);}
  let stage = AieyesI18n.t("等待当前刷新");const update = () => {progress.textContent = AieyesI18n.message(stage + ' · ' + Math.floor((Date.now() - started) / 1000) + AieyesI18n.t(" 秒 · 可关闭窗口，后台继续"));background.textContent = AieyesI18n.t("后台处理中 · ") + stage + AieyesI18n.t(" · 查看");};update();const timer = setInterval(update, 1000);
  let unlisten = () => {};
  try {unlisten = await window.__TAURI__.event.listen('operations:progress', ({ payload }) => {if (payload.operationId === operationId) {stage = AieyesI18n.message(payload.stage);update();}});await api(action, { ...params, operationId });stage = AieyesI18n.t("读取计算结果");update();await loadDashboard();state.estimateOperation.status = 'success';background.textContent = AieyesI18n.t("后台操作已完成 · 查看结果");return true;}
  catch (error) {state.estimateOperation.status = 'error';background.textContent = AieyesI18n.t("后台操作失败 · 查看并重试");AieyesUI.showError($('#quota-tool-error'), error.message ?? String(error), AieyesI18n.t("采样操作失败"));$('#quota-tool-error').hidden = false;await loadDashboard().catch(() => {});return false;} finally
  {clearInterval(timer);unlisten();progress.remove();controls.forEach(([el, disabled]) => el.disabled = disabled);dialog.oncancel = null;delete dialog.dataset.busy;state.estimateBusy = false;scheduleRefreshes();}
}
function estimateLiveRevision(snapshot, key) {
  const separator = key.indexOf(':'),quota = {provider: key.slice(0, separator), accountId: key.slice(separator + 1)};
  return JSON.stringify([(snapshot.automaticEstimateSummaries ?? []).filter((e) => e.accountKey === key), AieyesUI.estimateSources(state.settings, quota).map((s) => [s.id, s.name, s.autoValuation, snapshot.sources?.find((row) => row.id === s.id)?.autoValuationStatus])]);
}
async function refreshQuotaDialog() {
  const dialog = $('#quota-dialog'),key = dialog?.dataset.estimateKey;
  window.AieyesAutomatic?.refresh();
  if (dialog?.dataset.autoSources) return;
  if (!dialog?.open || !key || dialog.dataset.busy) return;
  const kind = dialog.dataset.estimateKind ?? 'weekly',group = dialog.dataset.estimateGroup ?? '';
  const records = JSON.stringify((state.dashboard[kind === 'credits' ? 'creditEstimates' : 'quotaEstimates'] ?? []).filter((e) => e.accountKey === key));
  const q = state.dashboard.quotas?.find((q) => q.provider + ':' + q.accountId === key),visibility = JSON.stringify(AieyesUI.estimateVisibility(q, JSON.parse(records), group));
  if (records === dialog.dataset.records && visibility === dialog.dataset.estimateVisibility && estimateLiveRevision(state.dashboard, key) === dialog.dataset.estimateLiveRevision) return;
  const expanded = dialog.querySelector('details')?.open,focus = document.activeElement?.id;
  await openQuotaEstimate(key, kind, group);
  if (expanded && dialog.querySelector('details')) dialog.querySelector('details').open = true;
  if (focus) document.getElementById(focus)?.focus({ preventScroll: true });
}
function hasActiveFilters() {return Boolean(state.provider || state.accountKey || state.sourceId || state.model || state.dateRange || state.days !== 1);}
function bindQuotaTools() {
  document.querySelectorAll('[data-view-analysis]').forEach((b) => b.onclick = () => openAnalysis(b.dataset.viewAnalysis, b.dataset.viewAnalysis.split(':')[0]));
  bindRememberedDisclosures();
  document.querySelectorAll('[data-account-machine-settings]').forEach((b) => b.onclick = () => openSettings({ provider: 'codex', section: 'machines' }));
  document.querySelectorAll('[data-switch-account]').forEach((b) => b.onclick = () => window.AieyesAccountTargets.open(b.dataset.switchAccount));
  renderQueryStatus();
  $('#content > .active-filters')?.remove();$('#content > .panel-filter-summary')?.remove();
  if (state.page === 'analysis' && hasActiveFilters()) {
    const filters = document.createElement('div');filters.className = 'active-filters';
    for (const [key, label] of [['provider', providers[state.provider]], ['accountKey', state.accountKey === 'none' ? AieyesI18n.t("未关联账户") : historicalAccounts().find((a) => a.provider + ':' + a.id === state.accountKey)?.name ?? state.accountKey], ['sourceId', state.settings.sources.find((s) => s.id === state.sourceId)?.name ?? state.sourceId], ['model', state.model]]) {
      if (!state[key]) continue;
      const button = document.createElement('button');button.innerHTML = escapeHTML(label) + uiIcon('x');button.setAttribute('aria-label', AieyesI18n.t("清除筛选：") + label);
      button.onclick = () => {state[key] = '';if (key === 'provider') {state.accountKey = '';state.sourceId = '';state.model = '';}if (key === 'accountKey') state.sourceId = '';changeFilters();};filters.append(button);
    }
    if (state.dateRange || state.days !== 1) {const range = document.createElement('button');range.textContent = rangeLabel();range.setAttribute('aria-label', AieyesI18n.t("清除时间范围"));range.onclick = () => {state.days = 1;state.dateRange = null;changeFilters();};filters.append(range);}
    const clear = document.createElement('button');clear.textContent = AieyesI18n.t("清除全部");clear.onclick = () => {state.provider = state.accountKey = state.sourceId = state.model = '';state.days = 1;state.dateRange = null;changeFilters();};filters.append(clear);
    if (PANEL) {const disclosure = document.createElement('details');disclosure.className = 'panel-filter-summary';disclosure.innerHTML = AieyesI18n.t("<summary>已筛选 ") + (filters.children.length - 1) + AieyesI18n.t(" 项</summary>");disclosure.append(filters);$('#content .filters')?.after(disclosure);} else $('#content .filters')?.after(filters);
  }
  document.querySelectorAll('.quota-disclosure').forEach((detail) => {
    detail.querySelector('summary').onclick = (event) => {
      event.preventDefault();if (detail.dataset.collapsible !== 'true') return;detail.open = !detail.open;
      try {localStorage.setItem('quota.expanded.v2.' + (PANEL ? 'panel.' : 'detail.') + detail.closest('[data-quota]').dataset.quota, String(detail.open));} catch (_) {}
    };
  });
  document.querySelectorAll('[data-manage-sampling]').forEach((button) => button.onclick = openSampling);
  if ($('#order-quotas')) $('#order-quotas').onclick = openQuotaOrder;
  document.querySelectorAll('[data-credit-estimate]').forEach((b) => b.onclick = () => openQuotaEstimate(b.dataset.creditEstimate, 'credits'));
  document.querySelectorAll('[data-estimate]').forEach((b) => b.onclick = () => openQuotaEstimate(b.dataset.estimate, "weekly", b.dataset.estimateGroup));
}
function openPanelAccounts() {
  if (state.estimateBusy) {notify(AieyesI18n.t("采样操作正在保存，请稍后调整面板选择。"));return;}
  const preference = AieyesUI.panelPreference(),selected = AieyesUI.panelAccounts(state.settings, state.dashboard.quotaOrder ?? [], preference),accounts = state.settings.accounts.filter((a) => !a.archived && a.quotaEnabled && AieyesUI.supports(a.provider, 'quota', state.settings));
  const rawProviders = [...new Set([...accounts.map((a) => a.provider), ...Object.keys(preference.providers)])],groups = [...new Set(rawProviders.map((p) => AieyesUI.agentProvider(p)))];
  const members = (provider) => rawProviders.filter((p) => AieyesUI.sameAgent(p, provider));
  const manual = (provider) => members(provider).some((p) => preference.providers[p]?.mode === 'custom');
  quotaDialog(AieyesI18n.t("面板显示账户"), AieyesI18n.t("<p class=\"muted\">自动显示每类前 5 个账户；新增账户默认加入。手动选择可留空、不限数量，仅影响本机面板。</p>") + groups.map((provider) => '<fieldset class="panel-account-group" data-account-provider="' + escapeHTML(provider) + '"><legend>' + providerIdentity(provider) + ' <span class="selected-count"></span><button type="button" data-panel-default="' + escapeHTML(provider) + AieyesI18n.t("\">恢复自动显示</button></legend>") + accounts.filter((a) => AieyesUI.sameAgent(a.provider, provider)).map((a) => '<label><input type="checkbox" data-panel-account="' + escapeHTML(a.provider + ':' + a.id) + '" ' + (selected[a.provider]?.includes(a.provider + ':' + a.id) ? 'checked' : '') + '><span data-user-text>' + escapeHTML(a.name) + '</span></label>').join('') + '</fieldset>').join('') + (accounts.length ? '' : AieyesI18n.t("<p>尚无可显示额度的账户</p>")) + AieyesI18n.t("<div class=\"dialog-foot\"><button type=\"button\" id=\"panel-accounts-cancel\">取消</button><button type=\"button\" class=\"primary\" id=\"panel-accounts-save\">保存面板选择</button></div>"), 'selection');
  if (PANEL) $('#quota-dialog').dataset.opener = '#panel-more > summary';
  const update = () => {for (const group of document.querySelectorAll('[data-account-provider]')) {const inputs = [...group.querySelectorAll('input')],count = inputs.filter((i) => i.checked).length;group.querySelector('.selected-count').textContent = (manual(group.dataset.accountProvider) ? AieyesI18n.t("手动 · ") : AieyesI18n.t("自动 · ")) + count + AieyesI18n.t(" 个");}};
  document.querySelectorAll('[data-panel-account]').forEach((input) => input.onchange = () => {const provider = input.closest('[data-account-provider]').dataset.accountProvider;for (const p of members(provider)) preference.providers[p] = { mode: 'custom', keys: [] };update();});update();
  document.querySelectorAll('[data-panel-default]').forEach((button) => button.onclick = () => {const provider = button.dataset.panelDefault;for (const p of members(provider)) preference.providers[p] = { mode: 'auto', keys: [] };const choices = AieyesUI.panelAccounts(state.settings, state.dashboard.quotaOrder ?? [], preference),defaults = members(provider).flatMap((p) => choices[p] ?? []);button.closest('fieldset').querySelectorAll('input').forEach((input) => input.checked = defaults.includes(input.dataset.panelAccount));update();});
  $('#panel-accounts-cancel').onclick = () => $('#quota-dialog').close();
  $('#panel-accounts-save').onclick = () => {for (const group of document.querySelectorAll('[data-account-provider]')) {for (const provider of members(group.dataset.accountProvider)) if (preference.providers[provider]?.mode === 'custom') preference.providers[provider].keys = [...group.querySelectorAll('input:checked')].map((i) => i.dataset.panelAccount).filter((key) => key.startsWith(provider + ':'));}try {localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify(preference));} catch (error) {$('#quota-tool-error').textContent = AieyesI18n.t("无法保存本机选择：") + String(error);$('#quota-tool-error').hidden = false;return;}$('#quota-dialog').close();render();};
  $('#panel-accounts-cancel').focus();
}
function openQuotaOrder() {
  if (state.estimateBusy) {notify(AieyesI18n.t("后台采样操作仍在进行，可从任务入口返回"));return;}
  let keys = [...(state.dashboard.quotaOrder ?? state.settings.accounts.map((a) => a.provider + ':' + a.id))],dragged = null;
  quotaDialog(AieyesI18n.t("调整账户顺序"), AieyesI18n.t("<p class=\"muted\">拖动账户或使用上下按钮排序，应用于所有额度面板。</p><div id=\"quota-order-list\"></div><button class=\"primary\" id=\"quota-order-save\">保存顺序</button>"), 'selection');
  function draw(focus) {
    const positions = new Map([...document.querySelectorAll('[data-order]')].map((row) => [row.dataset.order, row.getBoundingClientRect().top]));
    $('#quota-order-list').innerHTML = keys.map((key, i) => `<div class="quota-order-row" draggable="true" data-order="${escapeHTML(key)}"><span aria-hidden="true">${uiIcon('grip')}</span><span class="order-account">${providerMark(key.split(':')[0])}${escapeHTML(historicalAccounts().find((a) => a.provider + ':' + a.id === key)?.name ?? key)}</span><button type="button" data-direction="-1" aria-label="${AieyesI18n.t("上移账户")}" ${i === 0 ? 'disabled' : ''}>${uiIcon('up')}</button><button type="button" data-direction="1" aria-label="${AieyesI18n.t("下移账户")}" ${i === keys.length - 1 ? 'disabled' : ''}>${uiIcon('down')}</button></div>`).join('');
    for (const row of document.querySelectorAll('[data-order]')) {
      row.ondragstart = (e) => {document.querySelectorAll('[data-order]').forEach((el) => el.getAnimations().forEach((animation) => animation.cancel()));dragged = row.dataset.order;e.dataTransfer.effectAllowed = 'move';e.dataTransfer.setData('text/plain', dragged);};row.ondragend = () => {dragged = null;document.querySelectorAll('.drop-target').forEach((el) => el.classList.remove('drop-target'));};row.ondragover = (e) => {e.preventDefault();row.classList.add('drop-target');};row.ondragleave = () => row.classList.remove('drop-target');
      row.ondrop = (e) => {e.preventDefault();if (!dragged || dragged === row.dataset.order) return;const to = keys.indexOf(row.dataset.order);keys.splice(keys.indexOf(dragged), 1);keys.splice(to, 0, dragged);dragged = null;requestAnimationFrame(() => draw());};
      row.querySelectorAll('button').forEach((b) => b.onclick = () => {const i = keys.indexOf(row.dataset.order),j = i + Number(b.dataset.direction);[keys[i], keys[j]] = [keys[j], keys[i]];draw({ key: row.dataset.order, direction: b.dataset.direction });});
    }
    if (focus && !matchMedia('(prefers-reduced-motion: reduce)').matches) for (const row of document.querySelectorAll('[data-order]')) {const old = positions.get(row.dataset.order);if (old != null) row.animate([{ transform: `translateY(${old - row.getBoundingClientRect().top}px)` }, { transform: 'translateY(0)' }], { duration: 220, easing: 'cubic-bezier(.2,.8,.2,1)' });}
    if (focus) {const row = [...document.querySelectorAll('[data-order]')].find((r) => r.dataset.order === focus.key);(row?.querySelector(`button[data-direction="${focus.direction}"]:not(:disabled)`) ?? row?.querySelector('button:not(:disabled)'))?.focus();}
  }
  draw();$('#quota-order-save').onclick = async () => {if (await quotaAction('quotas.order.set', { keys })) {$('#quota-dialog').close();flushRender();}};
}
async function openQuotaEstimate(key, kind = "weekly", group = "", snapshot = state.dashboard) {
  if (state.estimateBusy) {const dialog = $('#quota-dialog');if (dialog && !dialog.open) dialog.showModal();return;}
  const returnProvider = $('#quota-dialog')?.dataset.valuationProvider;
  const credits = kind === 'credits';
  let q = snapshot.quotas?.find((q) => q.provider + ':' + q.accountId === key);
  if (!q) {
    const dialog = quotaDialog(AieyesI18n.t("估值与采样"), AieyesI18n.t("<p role=\"status\">正在读取账户与采样记录…</p>")),request = crypto.randomUUID();dialog.dataset.loadingRequest = request;
    try {snapshot = await api('dashboard.summary');} catch (error) {if (dialog.open && dialog.dataset.loadingRequest === request) {AieyesUI.showError($('#quota-tool-error'), String(error), AieyesI18n.t("读取失败"));$('#quota-tool-error').hidden = false;const retry = document.createElement('button');retry.textContent = AieyesI18n.t("重试读取");retry.onclick = () => openQuotaEstimate(key, kind, group);dialog.querySelector('.quota-dialog-body').append(retry);}return;}
    if (!dialog.open || dialog.dataset.loadingRequest !== request) return;
    q = snapshot.quotas?.find((q) => q.provider + ':' + q.accountId === key);
  }
  const unavailable = !q;
  if (!q) {const account = historicalAccounts().find((a) => a.provider + ':' + a.id === key);q = { provider: account?.provider ?? key.split(':')[0], accountId: account?.id ?? key.slice(key.indexOf(':') + 1), name: account?.name ?? key, windows: [] };}
  if (state.revisionEvents) {try {const rows = await api(credits ? 'creditEstimates.list' : 'quotaEstimates.list', { accountKey: key });snapshot = { ...snapshot, [credits ? 'creditEstimates' : 'quotaEstimates']: rows };} catch (error) {notify(String(error), 'error');return;}}
  const records = (snapshot[credits ? 'creditEstimates' : 'quotaEstimates'] ?? []).filter((e) => e.accountKey === key),current = records.find((e) => e.status !== 'completed');
  const windows = AieyesUI.estimateWindows(q),eligible = credits ? q.provider === 'codex' && q.credits?.balance != null && !q.credits.unlimited : windows.some((w) => !group || (w.groupId || w.groupName) === group);
  const rolling = (snapshot.automaticEstimateSummaries ?? []).filter((e) => e.accountKey === key && e.kind === 'credits' === credits && (!group || e.groupId === group));
  const history = records.filter((e) => e.status === 'completed' && (!group || !e.groupId || e.groupId === group));
  const sources = AieyesUI.estimateSources(state.settings, q);
  const shared = sources.some((s) => s.codexHomeId);
  const confirmation = '<label class="quota-confirm"><input type="checkbox" id="estimate-confirm">' + (credits ? AieyesI18n.t("我确认本段仅消耗 credits，所有设备的用量均已纳入；不混用包含额度、API 或中转。") : AieyesI18n.t("我确认采样期间只使用目标订阅，所有设备用量均已纳入所选来源；不混用 API / 中转。")) + '</label>' + (shared ? AieyesI18n.t("<p class=\"tiny muted\">共享目录仅统计本次采样时段。切换账户前请结束采样，切换后重新开始。</p>") : '');
  const result = (e) => {
    const live = AieyesUI.automaticEstimateState(e, state.settings, snapshot);
    return `<div class="estimate-result">${e.groupId ? `<strong>${escapeHTML(q.windows.find((w) => (w.groupId || w.groupName) === e.groupId)?.groupName || e.groupId)}</strong>` : q.provider === 'antigravity' ? AieyesI18n.t("<strong>账户总体 · 旧记录</strong>") : ''}<div class="between"><strong>${estimateStatus(e, snapshot)}</strong><strong>${credits ? creditValue(e) : AieyesUI.estimatePeriods(q, [e], e.groupId) || AieyesI18n.t("额度估值")}</strong></div>${credits ? '' : estimateValues(e, q)}<p class="muted">${escapeHTML(AieyesI18n.message(e.calculationNote))}</p>${live?.note ? `<p class="muted" data-estimate-runtime>${escapeHTML(live.note)}</p>` : ''}${['rolling', 'automatic'].includes(e.samplingMode) ? `<p class="tiny muted">${escapeHTML(AieyesI18n.t("样本截止：{0}", date(e.checkpointAt)))}</p>` : ''}${e.originalEstimateId ? AieyesI18n.t("<p class=\"muted\">已修正 · 原采样记录保留</p>") : e.samplingMode !== 'automatic' && e.status === 'completed' && !(credits ? [e.valuePer1000] : [e.fiveHourValue, e.weeklyValue, e.weeklyRatioValue]).some(Number.isFinite) ? `<button type="button" data-estimate-repair="${escapeHTML(e.id)}">${AieyesI18n.t("按原记录修复估值")}</button>` : ''}<details><summary>${AieyesI18n.t("计算依据")}</summary><p>${date(e.startedAt)} → ${date(e.checkpointAt)}<br>${AieyesI18n.t("{0} · 消耗 {1} · 样本成本 {2} USD", escapeHTML(AieyesI18n.message(e.windowName)), credits ? creditAmount(e.consumedCredits) + ' credit' : pct(e.consumedPercent), money(e.cost))}<br>${AieyesI18n.t("计价 Token：{0} / {1}", compact(e.pricedTokens), compact(e.totalTokens))}<br>${live ? escapeHTML(AieyesI18n.t("当前参与来源：{0}", live.sourceNames.join('、') || '—')) + '<br>' : ''}${escapeHTML(AieyesI18n.t("历史样本来源：{0}", e.sourceNames.join('、')))}</p><p class="tiny muted">${AieyesI18n.t("{0}；结束后价格与倍率固定。", estimateCalculation(credits ? { ...e, kind: 'credits' } : e, q))}</p>${(e.segments ?? []).length ? `<p class="tiny muted">${AieyesI18n.t("已保存 {0} 个有效片段", e.segments.length)}</p>` : ''}${e.reason ? `<div class="error">${escapeHTML(AieyesI18n.message(e.reason))}</div>` : ''}${(e.prices ?? []).length ? `<p class="tiny muted">${AieyesI18n.t("价格快照：{0}", e.prices.map((p) => escapeHTML(p.id) + ' · ' + date(p.fetchedAt)).join('；'))}</p>` : ''}</details></div>`;
  };
  const setup = eligible && sources.length ? `${credits ? '' : `<label>${AieyesI18n.t("主要额度池")}<select id="estimate-window">${windows.map((w) => option(w.id || w.name, AieyesI18n.message(w.name), windows.find((x) => (x.groupId || x.groupName) === group)?.id ?? '')).join('')}</select></label>`}<p>${AieyesI18n.t("纳入的用量来源")}</p>${sources.map((s) => `<label class="quota-confirm"><input type="checkbox" name="estimate-source" value="${escapeHTML(s.id)}" checked><span data-user-text>${escapeHTML(s.name)}</span></label>`).join('')}${confirmation}<p class="tiny muted">${AieyesI18n.t("开始后请新建会话。有可确认的额度消耗且价格完整时显示估值；跨采样边界的累计用量不计入。")}</p><button class="primary" id="estimate-start" disabled>${AieyesI18n.t("开始采样")}</button>` : AieyesI18n.t("<p class=\"muted\">需要可采集 Token 的关联数据源及可靠的额度池映射。请检查数据目录、账户关联和模型映射。</p>");
  quotaDialog(q.name + ' · ' + (credits ? AieyesI18n.t("credit 价值") : (AieyesUI.estimatePeriods(q, records, group) + AieyesI18n.t(" 额度价值")).trim()), `${returnProvider ? AieyesI18n.t("<button type=\"button\" id=\"valuation-back\">返回估值与采样</button>") : ''}<p class="muted">${AieyesI18n.t("按样本成本与额度消耗比例估算 API 等价价值，金额不可兑换。")}</p>${unavailable ? AieyesI18n.t("<p class=\"muted\">当前没有可用的额度快照；已有采样和历史仍可管理。</p>") : ''}${window.AieyesAutomatic?.html(sources) ?? ''}${current ? (current.groupId && group && current.groupId !== group ? AieyesI18n.t("<p class=\"muted\">此账户正在采样其他模型组，请先结束当前采样。</p>") : '') + result(current) + (current.status === 'pending' ? confirmation + AieyesI18n.t("<button id=\"estimate-restart\" disabled>确认并开始新一段</button>") : '') + `<button class="primary" id="estimate-stop">${current.status === 'pending' ? AieyesI18n.t("结束并保留有效段") : AieyesI18n.t("结束并保存本段结果")}</button>` : setup}${rolling.map(result).join('')}${history.length ? AieyesI18n.t("<h3>采样历史</h3>") + history.map(result).join('') : ''}`);
  if (returnProvider) {$('#quota-dialog').dataset.valuationProvider = returnProvider;$('#valuation-back').onclick = () => openProviderEstimates(returnProvider);}
  $('#quota-dialog').dataset.estimateGroup = group;$('#quota-dialog').dataset.estimateKind = kind;$('#quota-dialog').dataset.estimateKey = key;$('#quota-dialog').dataset.records = JSON.stringify(records);$('#quota-dialog').dataset.estimateVisibility = JSON.stringify(AieyesUI.estimateVisibility(q, records, group));
  $('#quota-dialog').dataset.estimateLiveRevision = estimateLiveRevision(snapshot, key);
  if ($('#estimate-window')) $('#estimate-window').onchange = () => {group = windows.find((w) => (w.id || w.name) === $('#estimate-window').value)?.groupId ?? '';$('#quota-dialog').dataset.estimateGroup = group;};
  const confirm = $('#estimate-confirm');
  const updateStart = () => {const b = $('#estimate-start') ?? $('#estimate-restart');const chosen = [...document.querySelectorAll('[name=estimate-source]:checked')].map((el) => el.value);const automatic = sources.some((s) => chosen.includes(s.id) && s.autoValuation?.enabled);if (b) {b.disabled = !confirm?.checked || Boolean($('#estimate-start') && (!chosen.length || automatic));if ($('#estimate-start')) b.textContent = automatic ? AieyesI18n.t("请先关闭所选来源的自动估值") : AieyesI18n.t("开始采样");}};
  if (confirm) confirm.onchange = updateStart;
  document.querySelectorAll('[name=estimate-source]').forEach((input) => input.onchange = updateStart);
  const act = async (action, params) => {if (await quotaAction((credits ? 'creditEstimates.' : 'quotaEstimates.') + action, params)) {if ($('#quota-dialog').open) openQuotaEstimate(key, kind, group);} else {const message = $('#quota-tool-error').textContent;refreshQuotaDialog();$('#quota-tool-error').textContent = AieyesI18n.message(message);$('#quota-tool-error').hidden = false;}};
  document.querySelectorAll('[data-estimate-repair]').forEach((button) => button.onclick = () => act('repair', { id: button.dataset.estimateRepair }));
  if ($('#estimate-start')) $('#estimate-start').onclick = () => act('start', { accountKey: key, windowId: credits ? 'credits' : $('#estimate-window').value, sourceIds: [...document.querySelectorAll('[name=estimate-source]:checked')].map((el) => el.value), confirmed: confirm.checked });
  if ($('#estimate-stop')) $('#estimate-stop').onclick = () => act('stop', { id: current.id });
  if ($('#estimate-restart')) $('#estimate-restart').onclick = () => act('restart', { id: current.id, confirmed: confirm.checked });
}

// Settings and authentication modules are loaded by later scripts in this document.
const appReady = new Promise((resolve) => {if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', resolve, { once: true });else resolve();}).then(boot);
window.AieyesApp.ready = appReady;

window.addEventListener('aieyes:fonts', () => requestAnimationFrame(() => drawTrend()));

// A locale update preserves the current page, filters and form drafts.
window.addEventListener('aieyes:language', () => {
  if (!state.settings) return;
  captureSettingsForms();
  render();
  window.AieyesTheme?.updateButtons();
  document.querySelectorAll('dialog[open]').forEach(AieyesI18n.refreshControls);
});
async function setLanguage(language) {
  if (!state.settings || state.languageSaving) return;
  state.languageSaving = true;
  try {
    const base = structuredClone(state.settings), next = structuredClone(base);
    next.appearance = {...(base.appearance ?? {}), language: AieyesI18n.normalize(language)};
    const saved = await api('settings.patch', {base, settings: next});
    syncSharedAppearance(saved.appearance);
    window.AieyesTheme.apply(saved.appearance);
  } catch (error) { notify(AieyesI18n.t('语言保存失败：') + AieyesI18n.message(String(error)), 'error'); }
  finally { state.languageSaving = false; }
}
document.addEventListener('click', event => {
  if (event.target.closest('[data-language-toggle]')) void setLanguage(AieyesI18n.language === 'en' ? 'zh-CN' : 'en');
});
