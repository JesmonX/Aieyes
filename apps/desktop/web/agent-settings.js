/* Agent setup and account connections. Credentials never enter rendered markup. */
(() => {
  const e = (value) => escapeHTML(value ?? '');
  let directorySelection = null,saveQueue = Promise.resolve(),pendingSaves = 0;
  const accountDrafts = new Map(),accountErrors = new Map();
  let showCodex = false,detailKey = '',statusRows = [],statusLoading = false,statusLoaded = false,statusFullLoaded = false,statusError = '',cleanupRows = [],syncMessage = '',syncRunning = false,refreshTimer;
  let statusRevision = 0,statusRefreshAgain = false;
  const sourceStatusRequests = new Map(),sourceStatusTasks = new Map();
  const antigravityRefreshTasks = new Map();
  const accountKey = (a) => a.provider + ':' + a.id;
  const machineId = (s) => s.hostId || 'local';
  const deviceName = (id) => id === 'local' ? AieyesI18n.t("本机") : state.settings.hosts.find((h) => h.id === id)?.name || state.settings.hosts.find((h) => h.id === id)?.target || AieyesI18n.t("已移除设备");
  function machines(provider, paused = false) {
    const config = rawConfigurations().find((a) => a.provider === provider);
    const active = (config?.enabled ? config.machineIds : []).filter((id) => id === 'local' ? state.settings.localDeviceEnabled !== false : state.settings.hosts.some((h) => h.id === id && h.enabled));
    return [...new Set(paused ? [...active, ...state.settings.sources.filter((s) => s.provider === provider).map(machineId)] : active)].sort((a, b) => a === 'local' ? -1 : b === 'local' ? 1 : a.localeCompare(b));
  }
  const statuses = (a, machine) => statusRows.filter((r) => r.accountKey === accountKey(a) && r.machineId === machine);
  const loggedIn = (row) => row.current === true || row.credential === true;
  const identityName = (identity) => identity.email?.trim() || (identity.key?.startsWith('qoder:') ? 'Qoder · ' + identity.key.slice(6, 18) : identity.key) || AieyesI18n.t("身份已确认");
  const identityText = (row, machine = false) => row?.identity ? (machine || row.current == null ? AieyesI18n.t("当前登录 ") : row.current === false ? AieyesI18n.t("身份不一致 · 当前登录 ") : AieyesI18n.t("正在使用 ")) + identityName(row.identity) + ' · ' + (row.identity.subscription || AieyesI18n.t("订阅未知")) + (row.identity.stale ? AieyesI18n.t(" · 订阅待更新") : '') + (row.error ? AieyesI18n.t(" · 上次检查") : '') : '';
  const statusText = (rows) => {
    const identified = rows.find((r) => r.current === true && r.identity) || rows.find((r) => r.identity);if (identified) return identityText(identified);
    if (rows.some((r) => r.authenticated === true)) return AieyesI18n.t("登录有效 · 账户身份待确认");
    const login = rows.some(loggedIn) ? AieyesI18n.t("已登录") : rows.length && rows.every((r) => r.current === false && r.credential === false) ? AieyesI18n.t("未登录") : AieyesI18n.t("登录待确认");
    const current = rows.some((r) => r.current === true) ? AieyesI18n.t("正在使用") : rows.length && rows.every((r) => r.current === false) ? AieyesI18n.t("未使用") : AieyesI18n.t("使用状态待确认");
    return login + ' · ' + current + (rows.some((r) => r.error) ? AieyesI18n.t(" · 待确认") : '');
  };
  function deviceCounts(a) {
    const ids = machines(a.provider),rows = (id) => statuses(a, id);
    return { total: ids.length, logged: ids.filter((id) => rows(id).some(loggedIn)).length, using: ids.filter((id) => rows(id).some((r) => r.current === true)).length, pending: ids.some((id) => !rows(id).length || rows(id).some((r) => r.current == null || r.credential == null || r.error)) };
  }
  function counts(a) {const c = deviceCounts(a);return AieyesI18n.t("已登录 {0}/{1} · 使用中 {2}/{3}", c.logged, c.total, c.using, c.total) + (c.pending ? AieyesI18n.t(" · 有设备待确认") : '');}
  function countsHTML(key) {const a = state.settings.accounts.find((a) => accountKey(a) === key);if (!a) return '';const c = deviceCounts(a);return `<span class="device-count-line" title="${c.pending ? AieyesI18n.t("部分机器状态待确认；离线时显示上次检查结果") : AieyesI18n.t("已登录：已保存凭据；使用中：机器当前选用的账户")}">${AieyesI18n.t("已登录 {0}/{1} · 使用中 {2}/{3}", c.logged, c.total, c.using, c.total)}</span>`;}
  let repaintPending = false;
  const updateText = (el, text) => {if (el.textContent !== text) el.textContent = text;};
  // Compare with the markup last written, not innerHTML: serialization rarely equals the template, so a
  // direct comparison rebuilt these buttons on every repaint. First sight compares normalized markup.
  const written = new WeakMap(),scratch = document.createElement('template');
  const updateHTML = (el, html) => {
    if (written.get(el) === html) return;
    scratch.innerHTML = html;
    if (el.innerHTML !== scratch.innerHTML) el.innerHTML = html;
    written.set(el, html);
  };
  const repaint = () => {
    if (repaintPending) return;repaintPending = true;
    requestAnimationFrame(() => {
      repaintPending = false;
      document.querySelectorAll('[data-device-counts]').forEach((el) => updateHTML(el, countsHTML(el.dataset.deviceCounts)));
      document.querySelectorAll('[data-account-summary]').forEach((el) => {const a = state.settings.accounts.find((a) => accountKey(a) === el.dataset.accountSummary);if (a) updateText(el, counts(a));});
      document.querySelectorAll('[data-account-identity]').forEach((el) => {const row = statusRows.find((r) => r.accountKey === el.dataset.accountIdentity && r.current === true && r.identity);updateText(el, row ? identityText(row) : '');el.hidden = !row;});
      document.querySelectorAll('[data-machine-current]').forEach((el) => {updateText(el, machineState(el.dataset.provider, el.dataset.machineCurrent).text);el.title = el.textContent;});
      document.querySelectorAll('[data-device-summary]').forEach((el) => {const a = state.settings.accounts.find((a) => accountKey(a) === el.dataset.account);if (a) updateText(el, statusText(statuses(a, el.dataset.deviceSummary)));});
      document.querySelectorAll('[data-source-status]').forEach((el) => {const row = statusRows.find((r) => r.sourceId === el.dataset.sourceStatus && r.accountKey === el.dataset.account);updateHTML(el, sourceStatusHTML(row));});
      document.querySelectorAll('[data-source-actions]').forEach((el) => {const source = state.settings.sources.find((s) => s.id === el.dataset.sourceActions),account = state.settings.accounts.find((a) => accountKey(a) === el.dataset.account);if (source && account) {const row = statusRows.find((r) => r.sourceId === source.id && r.accountKey === el.dataset.account),html = sourceActionsHTML(account, source, row);updateHTML(el, html);}});
      const refresh = document.querySelector('#account-status-refresh');if (refresh) {refresh.disabled = statusLoading;refresh.textContent = statusLoading ? AieyesI18n.t("检查中…") : AieyesI18n.t("刷新设备状态");}
      document.querySelectorAll('[data-machine-account-refresh]').forEach((button) => {button.disabled = statusLoading || !rawConfigurations().find((a) => a.provider === button.dataset.machineAccountRefresh)?.enabled;button.textContent = statusLoading ? AieyesI18n.t("检查中…") : AieyesI18n.t("刷新账户状态");});
      bindStatusActions();
      document.querySelectorAll('[data-account-sync]').forEach((el) => updateHTML(el, syncContent()));
      document.querySelectorAll('[data-account-retry-status]').forEach((el) => el.onclick = () => refreshStatuses(true));
      document.querySelectorAll('[data-deployment-sync]').forEach((el) => el.onclick = syncDeployments);
      document.querySelectorAll('[data-account-save-errors]').forEach((el) => updateHTML(el, [...accountErrors].filter(([key]) => key.startsWith(el.dataset.accountSaveErrors + ':')).map(([key, message]) => `<div class="error">${AieyesUI.errorHTML(message, AieyesI18n.t("保存失败，输入已保留"))} <button data-account-retry="${e(key)}">${AieyesI18n.t("重试保存")}</button></div>`).join('')));
      document.querySelectorAll('[data-account-retry]').forEach((b) => b.onclick = () => {const key = b.dataset.accountRetry,a = state.settings.accounts.find((a) => key.startsWith(accountKey(a) + ':'));if (a) void queueAccountField(a, key.slice(accountKey(a).length + 1), accountDrafts.get(key));});
      paintHooks();
    });
  };
  function ensureStatuses(provider = null) {
    if (statusLoading || statusError || (provider ? statusLoaded : statusFullLoaded)) return;
    return refreshStatuses(false, provider);
  }
  async function refreshStatuses(refresh = true, provider = null, dueOnly = false) {
    if (provider && !state.settings.accounts.some((a) => a.provider === provider)) {statusLoaded = true;return;}
    if (statusLoading) return;
    const previousError = statusError;statusLoading = true;statusError = '';
    const revision = statusRevision;
    try {
      const loaded = await api('accounts.status.get');if (revision === statusRevision) statusRows = Array.isArray(loaded) ? loaded : [];
      cleanupRows = await api('accounts.cleanup.list');if (!Array.isArray(cleanupRows)) cleanupRows = [];
      statusLoaded = true;repaint();
      if (refresh) {
        try {
          if (!state.revisionEvents) throw new Error('Unexpected call: accounts.status.refreshBatch');
          const keys = state.settings.accounts.filter((a) => !a.archived && (!provider || a.provider === provider)).map(accountKey);
          const rows = await api('accounts.status.refreshBatch', { accountKeys: keys, dueOnly });
          if (revision === statusRevision) for (const row of rows) {statusRows = statusRows.filter((r) => !(r.accountKey === row.accountKey && r.sourceId === row.sourceId));statusRows.push(row);}
          repaint();
        } catch (error) {
          if (!/Unexpected call|未知.*方法|不存在.*方法/i.test(String(error))) throw error;
          for (const a of state.settings.accounts.filter((a) => !a.archived && (!provider || a.provider === provider))) for (const s of state.settings.sources.filter((s) => s.provider === a.provider && AieyesUI.sourceEnabled(state.settings, s) && machines(a.provider).includes(machineId(s))).sort((a, b) => Number(Boolean(a.hostId)) - Number(Boolean(b.hostId)))) {
            if (revision !== statusRevision) return;
            try {const row = await api('accounts.status.refresh', { accountKey: accountKey(a), sourceId: s.id });if (revision === statusRevision) {statusRows = statusRows.filter((r) => !(r.accountKey === row.accountKey && r.sourceId === row.sourceId));statusRows.push(row);repaint();}} catch {/* Settings changed while this device was being checked. */}
          }
        }
      }
      if (refresh && !provider && revision === statusRevision) statusFullLoaded = true;
      if (previousError && document.querySelector('#message')?.dataset.source === 'account-status') notify('');
    } catch (error) {statusError = AieyesI18n.t("设备状态读取失败：") + String(error);notify(statusError, 'error', 'account-status');} finally
    {await syncIdentitySettings();statusLoading = false;repaint();if (statusRefreshAgain) {statusRefreshAgain = false;void refreshStatuses(true, null, true);}}
  }
  async function syncIdentitySettings(reportFailure = false) {
    if (!state.settings.accounts.length || pendingSaves || state.settingsSaving) return;
    const before = configKey(state.settings),saved = await api('settings.get').catch((error) => {if (reportFailure) notify(AieyesI18n.t("登录已确认，账户设置刷新失败：") + String(error), 'error');return null;});
    if (!saved) return;
    if (before === configKey(state.settings) && !pendingSaves && !state.settingsSaving) {
      acceptSettings(saved);
    }
  }
  async function refreshAntigravitySource(sourceId, status = null) {
    const source = state.settings.sources.find((s) => s.id === sourceId && s.provider === 'antigravity');
    if (!source) throw new Error(AieyesI18n.t("账户连接已变化，请重新检查"));
    const account = state.settings.accounts.find((a) => a.provider === 'antigravity' && a.id === source.accountId);
    if (!account) throw new Error(AieyesI18n.t("账户连接已变化，请重新检查"));
    const revision = ++statusRevision;
    const row = status ?? (await api('accounts.status.refresh', { accountKey: accountKey(account), sourceId }));
    if (!status && row.error) throw new Error(AieyesI18n.t("设备状态更新失败：") + row.error);
    if (revision !== statusRevision || row.sourceId !== sourceId || row.accountKey !== accountKey(account) || row.machineId !== machineId(source) || !state.settings.sources.some((s) => s.id === sourceId && s.accountId === account.id && s.enabled)) throw new Error(AieyesI18n.t("账户连接已变化，请重新检查"));
    statusRows = statusRows.filter((r) => !(r.accountKey === row.accountKey && r.sourceId === row.sourceId));statusRows.push(row);repaint();
    if (!antigravityRefreshTasks.has(sourceId)) {
      const work = (async () => {await syncIdentitySettings(true);repaint();await quotas(account.id);await loadDashboard();})().
      catch((error) => notify(AieyesI18n.t("登录已确认，后续刷新失败：") + String(error), 'error')).
      finally(() => antigravityRefreshTasks.delete(sourceId));
      antigravityRefreshTasks.set(sourceId, work);
    }
  }
  async function refreshCodexSource(sourceId) {
    sourceStatusRequests.set(sourceId, ++statusRevision);
    if (sourceStatusTasks.has(sourceId)) return sourceStatusTasks.get(sourceId);
    const work = (async () => {
      while (true) {
        const requested = sourceStatusRequests.get(sourceId),rows = [];
        let succeeded = true;
        if (!state.settings.sources.some((s) => s.id === sourceId && s.provider === 'codex' && s.enabled)) return false;
        if (state.revisionEvents) {
          try {rows.push(...(await api('accounts.status.refreshBatch', { sourceIds: [sourceId], accountKeys: state.settings.accounts.filter((a) => a.provider === 'codex').map(accountKey) })));if (rows.some((r) => r.error)) succeeded = false;} catch {succeeded = false;}
        } else for (const a of state.settings.accounts.filter((a) => a.provider === 'codex')) {
          try {const row = await api('accounts.status.refresh', { accountKey: accountKey(a), sourceId });rows.push(row);if (row.error) succeeded = false;} catch {succeeded = false;}
        }
        if (requested !== sourceStatusRequests.get(sourceId)) continue;
        for (const row of rows) {
          if (!state.settings.accounts.some((a) => accountKey(a) === row.accountKey)) continue;
          statusRows = statusRows.filter((r) => !(r.accountKey === row.accountKey && r.sourceId === row.sourceId));statusRows.push(row);
        }
        repaint();return succeeded;
      }
    })().finally(() => sourceStatusTasks.delete(sourceId));
    sourceStatusTasks.set(sourceId, work);return work;
  }
  async function syncDeployments() {if (syncRunning) return;syncRunning = true;try {const r = await api('accounts.deployments.sync');syncMessage = r.failedTasks?.length ? AieyesI18n.t("配置已保存，部分唤醒部署待同步，可重试。") : '';} catch (error) {syncMessage = AieyesI18n.t("配置已保存，唤醒部署待同步：") + String(error);} finally {syncRunning = false;repaint();}}
  function backgroundRefresh() {clearTimeout(refreshTimer);refreshTimer = setTimeout(() => {void loadDashboard().catch((error) => notify(AieyesI18n.t("配置已保存，刷新失败：") + String(error), 'error'));}, 300);void syncDeployments();}

  const rawConfigurations = () => {
    const order = Object.keys(providers).filter((p) => p !== 'custom');
    const rows = state.settings.agents ?? Object.keys(providers).map((provider) => ({ provider, enabled: state.settings.sources.some((s) => s.provider === provider && s.enabled), machineIds: [...new Set(state.settings.sources.filter((s) => s.provider === provider && s.enabled).map(machineId))] }));
    const rank = (p) => p === 'custom' ? order.length + 1 : order.includes(p) ? order.indexOf(p) : order.length;
    return [...rows].sort((a, b) => rank(a.provider) - rank(b.provider));
  };
  const configurations = () => AieyesUI.agentConfigurations(rawConfigurations());
  const machineName = (s) => s.hostId ? state.settings.hosts.find((h) => h.id === s.hostId)?.name || s.hostId : AieyesI18n.t("本机");
  const profiles = (a) => [...new Set([a.quotaProfileId, ...(a.connections ?? []).map((c) => c.profileId)].filter(Boolean))];
  const linked = (a, s) => s.provider === a.provider && (s.accountId === a.id || s.codexHomeId && profiles(a).some((r) => r.startsWith(s.codexHomeId + ':')));
  function deviceConfig(settings) {
    const pick = (row, keys) => Object.fromEntries(keys.map((key) => [key, row[key]]));
    return configKey({ accounts: settings.accounts.map((a) => pick(a, ['provider', 'id', 'identityKey', 'quotaProfileId', 'connections', 'deviceSettings'])), sources: settings.sources.map((s) => pick(s, ['id', 'provider', 'accountId', 'path', 'hostId', 'enabled', 'machineBinding', 'codexHomeId', 'codexBinary', 'agyBinary', 'quotaPreCommand', 'subscription', 'proxy'])), hosts: settings.hosts.map((h) => pick(h, ['id', 'target', 'port', 'username', 'identityFile', 'authMode', 'passwordRef', 'shell', 'preCommand', 'enabled'])), agents: settings.agents, localDeviceEnabled: settings.localDeviceEnabled, nativeConnections: (settings.nativeConnections ?? []).map((r) => pick(r, ['id', 'provider', 'machineId', 'machineFingerprint', 'sourceId', 'sourcePath', 'root', 'configPath', 'selector', 'kind', 'accountKey', 'identityKey', 'status', 'hasCredential'])), proxy: settings.sources.some((s) => s.provider !== 'codex') ? settings.proxy : null });
  }
  function accept(saved, submittedForms = null) {
    // A shared refresh can deliver settings before boot assigns the first copy; there is nothing to compare yet.
    if (!state.settings) {acceptSettings(saved, submittedForms);window.AieyesTheme?.apply(saved.appearance);return;}
    const quotaKey = (value) => configKey([value.accounts, value.sources, value.hosts, value.proxy, value.refreshSeconds, value.agents, value.localDeviceEnabled]);
    if (quotaKey(state.settings) !== quotaKey(saved)) state.lastQuota = 0;
    const changed = deviceConfig(state.settings) !== deviceConfig(saved);
    if (!changed) {acceptSettings(saved, submittedForms);window.AieyesTheme?.apply(saved.appearance);return;}
    statusRevision++;if (statusLoading) statusRefreshAgain = true;for (const id of sourceStatusRequests.keys()) sourceStatusRequests.set(id, statusRevision);statusLoaded = false;statusFullLoaded = false;statusError = '';acceptSettings(saved, submittedForms);state.lastQuota = 0;
    if (state.revisionEvents) void refreshStatuses(true, null, true);
    window.AieyesTheme?.apply(saved.appearance);
  }
  async function commit(next, base = state.settings) {
    if (state.settingsSaving) throw new Error(AieyesI18n.t("正在保存，请稍后重试"));
    state.settingsSaving = true;
    try {accept(await api('settings.patch', { base, settings: next }));markSaved();} finally
    {state.settingsSaving = false;scheduleRefreshes();}
    backgroundRefresh();
  }
  async function run(action) {
    if (state.settingsSaving) return;
    try {await action();} catch (error) {notify(error.message ?? String(error), 'error');} finally
    {if (state.page === 'settings') renderSettings(false);}
  }
  const machineSources = (provider, machine) => state.settings.sources.filter((s) => s.provider === provider && machineId(s) === machine);
  const directorySources = (provider, machine) => machineSources(provider, machine).filter((s) => !s.subscription && s.path?.trim());
  const hookSources = directorySources;
  function automaticMachineHTML(provider, machine) {
    if (!AieyesUI.supports(provider, 'valuation', state.settings)) return '';
    return `<span class="tiny muted" data-auto-machine-status="${e(machine)}" data-auto-provider="${e(provider)}">${AieyesI18n.t("估值 {0}", e(window.AieyesAutomatic.summary(machineSources(provider, machine))))}</span>`;
  }
  let sessionSnapshot = { sources: [] };const hookOverrides = new Map();
  let hookLabels = { installed: AieyesI18n.t("已安装"), missing: AieyesI18n.t("等待监测安装"), disabled: AieyesI18n.t("已停用"), error: AieyesI18n.t("安装失败") };window.addEventListener("aieyes:language", () => {hookLabels = { installed: AieyesI18n.t("已安装"), missing: AieyesI18n.t("等待监测安装"), disabled: AieyesI18n.t("已停用"), error: AieyesI18n.t("安装失败") };});
  function hookState(source) {const override = hookOverrides.get(source.id);if (override && override.until > Date.now()) return override.state;return sessionSnapshot.sources?.find((row) => row.sourceId === source.id)?.hooks ?? 'missing';}
  function hookLabel(provider, machine) {
    const sources = hookSources(provider, machine);if (!sources.length) return AieyesI18n.t("会话监测未启用");
    const active = sources.filter((s) => AieyesUI.sourceEnabled(state.settings, s));if (!active.length) return AieyesI18n.t("采集已暂停");
    if (['codex', 'kimi', 'deepseek-harness', 'grok', 'qoder', 'opencode', 'pi', 'omp'].includes(provider)) return active.every((s) => sessionSnapshot.sources?.some((row) => row.sourceId === s.id && row.state === 'healthy')) ? AieyesI18n.t("日志接入") : AieyesI18n.t("日志待确认");
    const values = active.map(hookState);return values.includes('error') ? AieyesI18n.t("接入失败") : values.includes('disabled') ? AieyesI18n.t("实时监测已停用") : values.every((v) => v === 'installed') ? AieyesI18n.t("会话监测已启用") : AieyesI18n.t("等待监测安装");
  }
  function hooksHTML(provider, machine) {return ['codex', 'claude', 'antigravity', 'cursor', 'kimi', 'codebuddy', 'deepseek-harness', 'grok', 'zcode', 'qoder', 'opencode', 'pi', 'omp'].includes(provider) ? `<button type="button" class="tiny" data-machine-hooks="${e(provider)}" data-machine="${e(machine)}" ${hookSources(provider, machine).length ? '' : 'disabled'}>${e(hookLabel(provider, machine))}</button>` : '';}
  function paintHooks() {document.querySelectorAll('[data-machine-hooks]').forEach((el) => updateText(el, hookLabel(el.dataset.machineHooks, el.dataset.machine)));}
  function updateSession(value) {sessionSnapshot = value ?? { sources: [] };paintHooks();}
  function openHooks(provider, machine) {
    const sources = hookSources(provider, machine).filter((s, i, a) => s.machineBinding !== 'unbound' && a.findIndex((v) => v.path === s.path) === i),dialog = document.createElement('dialog');
    dialog.id = 'session-hooks';dialog.setAttribute('aria-label', AieyesI18n.t("会话监测"));
    dialog.innerHTML = `<div class="between"><h2>${AieyesI18n.t("会话监测 · {0}", e(deviceName(machine)))}</h2><button data-close>${AieyesI18n.t("完成")}</button></div><p class="muted">${['codex', 'kimi', 'deepseek-harness', 'grok', 'qoder', 'opencode', 'pi', 'omp'].includes(provider) ? AieyesI18n.t("从 {0} 原生记录读取会话状态。", e(providers[provider])) : provider === 'zcode' ? AieyesI18n.t("实时状态由 ZCode 原生监测提供；停用后仅显示已结算的状态。SSH 监测需要远端 Python 3。") : provider === 'cursor' ? AieyesI18n.t("从用户或项目的 .cursor/hooks.json 接收状态；停用后不再接收新状态。原生会话库的 Token 暂未接入，SSH 监测需要远端 Python 3。") : machine === 'local' ? AieyesI18n.t("采集时自动监测会话；停用实时监测后仍读取日志。") : AieyesI18n.t("通过 SSH 监测会话，需要远端 Python 3。停用实时监测后仍读取日志。")}</p>` + sources.map((source) => `<section class="sampling-row" data-hook-row="${e(source.id)}"><code>${e(source.path)}</code>${['codex', 'kimi', 'deepseek-harness', 'grok', 'qoder', 'opencode', 'pi', 'omp'].includes(provider) ? '' : `<div class="between"><label><input type="checkbox" role="switch" data-hook-toggle ${hookState(source) === 'disabled' ? '' : 'checked'}>${AieyesI18n.t("实时监测")}</label><span data-hook-label>${e(hookLabels[hookState(source)] ?? AieyesI18n.t("等待监测安装"))}</span><button data-hook-repair>${AieyesI18n.t("修复监测")}</button></div>`}<div class="error" hidden></div></section>`).join('');
    document.body.append(dialog);dialog.showModal();dialog.querySelector('[data-close]').onclick = () => dialog.close();dialog.onclose = () => window.AieyesMotion ? AieyesMotion.removeAfterExit(dialog) : dialog.remove();
    let busy = false;dialog.oncancel = (event) => {if (busy) event.preventDefault();};
    async function act(row, action) {
      if (busy) return;busy = true;const source = sources.find((s) => s.id === row.dataset.hookRow),input = row.querySelector('[data-hook-toggle]');input.checked = hookState(source) !== 'disabled';dialog.querySelectorAll('button,input').forEach((el) => el.disabled = true);
      try {const rows = await api('sessions.hooks.' + action, { sourceId: source.id });const result = rows.find((r) => r.sourceId === source.id);if (result) {hookOverrides.set(source.id, { state: result.state, until: Date.now() + 35000 });row.querySelector('[data-hook-label]').textContent = hookLabels[result.state] ?? result.state;if (result.error) throw new Error(result.error);}row.querySelector('.error').hidden = true;}
      catch (error) {AieyesUI.showError(row.querySelector('.error'), String(error), AieyesI18n.t("会话监测设置失败"));row.querySelector('.error').hidden = false;} finally
      {busy = false;input.checked = hookState(source) !== 'disabled';dialog.querySelectorAll('button,input').forEach((el) => el.disabled = false);paintHooks();}
    }
    dialog.querySelectorAll('[data-hook-row]').forEach((row) => {const toggle = row.querySelector('[data-hook-toggle]');if (toggle) toggle.onchange = () => act(row, toggle.checked ? 'repair' : 'remove');const repair = row.querySelector('[data-hook-repair]');if (repair) repair.onclick = () => act(row, 'repair');});
  }
  function machineState(provider, id) {
    const sources = machineSources(provider, id).filter((s) => AieyesUI.sourceEnabled(state.settings, s)),rows = statusRows.filter((r) => r.machineId === id && r.accountKey.startsWith(provider + ':') && sources.some((s) => s.id === r.sourceId));
    const names = state.settings.accounts.filter((a) => a.provider === provider && rows.some((r) => r.accountKey === accountKey(a) && r.current === true)).map((a) => a.name);
    const identities = [...new Map(rows.filter((r) => r.identity).sort((a, b) => Number(Boolean(b.error)) - Number(Boolean(a.error)) || (a.checkedAt ?? 0) - (b.checkedAt ?? 0)).map((r) => [r.identity.key, r])).values()];
    return { sources, rows, text: identities.length ? provider === 'qoder' ? identities.map((r) => identityText(r, true)).join('；') : identityText(identities[0]) : rows.some((r) => r.authenticated === true) ? AieyesI18n.t("登录有效 · 账户身份待确认") : names.length ? AieyesI18n.t("正在使用 ") + names.join('、') : rows.some(loggedIn) ? AieyesI18n.t("已登录，尚未选用") : !rows.length || rows.some((r) => r.current == null || r.credential == null || r.error) ? AieyesI18n.t("登录状态待确认") : AieyesI18n.t("未登录") };
  }
  async function setDeviceEnabled(id, enabled) {const next = structuredClone(state.settings);if (id === 'local') next.localDeviceEnabled = enabled;else next.hosts.find((h) => h.id === id).enabled = enabled;await commit(next);renderSettings(false);}
  function navigate(target = {}) {
    const account = state.settings.accounts.find((a) => accountKey(a) === target.accountKey);
    state.settingsAgent = AieyesUI.agentProvider(account?.provider ?? target.provider ?? state.settingsAgent);
    if (!configurations().some((a) => a.provider === state.settingsAgent)) state.settingsAgent = 'codex';
    detailKey = account ? accountKey(account) : '';showCodex = false;directorySelection = null;
    try {localStorage.setItem('aieyes.settings.agent', state.settingsAgent);} catch (_) {}
  }
  function entryIds() {
    return state.settings.appearance?.settingsAgentIds ?? configurations().filter(a => a.enabled || a.machineIds.length || state.settings.sources.some(s => AieyesUI.sameAgent(s.provider,a.provider)) || state.settings.accounts.some(row => AieyesUI.sameAgent(row.provider,a.provider))).map(a => a.provider);
  }
  function selectorHTML(agent) {
    const entries = entryIds(), rows = entries.map(id => configurations().find(a => a.provider === id)).filter(Boolean);
    const button = a => `<button type="button" data-settings-agent="${e(a.provider)}" tabindex="${a.provider === (rows.find(row => row.provider === agent?.provider) ?? rows[0])?.provider ? 0 : -1}" ${a.provider === agent?.provider ? 'class="active" aria-current="page"' : ''}>${providerMark(a.provider)}<span>${e(providers[a.provider] ?? a.provider)}</span></button>`;
    return `<nav class="settings-agent-selector" aria-label="${AieyesI18n.t("选择 Agent")}"><div class="agent-common agent-selector">${rows.map(button).join('')}</div><details class="agent-more"><summary aria-haspopup="dialog"><span class="agent-wide-label">${AieyesI18n.t("更多 Agent")}</span><span class="agent-narrow-label">${e(providers[agent?.provider] || AieyesI18n.t("选择 Agent"))}</span>${uiIcon('chevron')}</summary><div class="agent-picker-menu" role="dialog" aria-label="${AieyesI18n.t("选择 Agent")}"><input type="search" data-agent-search aria-label="${AieyesI18n.t("搜索 Agent")}" placeholder="${AieyesI18n.t("搜索 Agent")}"><div class="agent-picker-results">${[true,false].map(joined => `<section data-agent-picker-group><h4>${AieyesI18n.t(joined ? "已加入" : "未加入")}</h4>${(joined ? rows : configurations().filter(a=>!entries.includes(a.provider))).map(a => button(a).replace('data-settings-agent=', 'data-agent-choice=').replace(/ tabindex="-?\d+"/, '').replace('</span></button>', `</span><small>${AieyesI18n.t(joined ? "已加入" : "添加")}</small></button>`)).join('')}</section>`).join('')}<p data-agent-no-results hidden>${AieyesI18n.t("没有匹配项")}</p></div></div></details><button type="button" data-manage-agents>${AieyesI18n.t("管理 Agent")}</button></nav>`;
  }
  function manageEntries() {
    const dialog = quotaDialog(AieyesI18n.t("管理 Agent"), `<p class="muted">${AieyesI18n.t("移出会暂停采集，保留目录、账户和历史。定时唤醒由任务开关独立控制。")}</p><input type="search" id="agent-management-search" aria-label="${AieyesI18n.t("搜索 Agent")}" placeholder="${AieyesI18n.t("搜索 Agent")}"><div id="agent-management-list"></div>`);
    const search = dialog.querySelector('#agent-management-search'), list = dialog.querySelector('#agent-management-list');
    const draw = () => {
      list.innerHTML = [...entryIds().map(id=>configurations().find(a=>a.provider===id)).filter(Boolean),...configurations().filter(a=>!entryIds().includes(a.provider))].filter(a => ((providers[a.provider] || a.provider) + ' ' + a.provider).toLowerCase().includes(search.value.trim().toLowerCase())).map((a,index,all) => `${index === 0 || entryIds().includes(a.provider) !== entryIds().includes(all[index-1].provider) ? `<h4>${AieyesI18n.t(entryIds().includes(a.provider) ? "已加入" : "未加入")}</h4>` : ''}<div class="list-row agent-order-row" data-order-agent="${e(a.provider)}" draggable="${entryIds().includes(a.provider) && !search.value}">${entryIds().includes(a.provider) && !search.value ? `<span class="agent-drag-handle" aria-hidden="true">${uiIcon('grip')}</span>` : ''}<span class="row-body">${e(providers[a.provider] || a.provider)}</span>${entryIds().includes(a.provider) && !search.value ? `<button data-agent-move="${e(a.provider)}" data-direction="-1" aria-label="${AieyesI18n.t("上移 Agent")}" ${entryIds().indexOf(a.provider) === 0 ? 'disabled' : ''}>${uiIcon('up')}</button><button data-agent-move="${e(a.provider)}" data-direction="1" aria-label="${AieyesI18n.t("下移 Agent")}" ${entryIds().indexOf(a.provider) === entryIds().length-1 ? 'disabled' : ''}>${uiIcon('down')}</button>` : ''}<button type="button" data-agent-manage="${e(a.provider)}" data-visible="${!entryIds().includes(a.provider)}">${AieyesI18n.t(entryIds().includes(a.provider) ? "移出并暂停采集" : "添加")}</button></div>`).join('');
      const saveOrder = async (order, focus) => { if(state.settingsSaving)return; state.settingsSaving=true; try {accept(await api('agents.manage',{order})); markSaved();} catch(error){notify(String(error.message||error),'error');} finally {state.settingsSaving=false;renderSettings(false);draw();list.querySelector(`[data-agent-move="${CSS.escape(focus)}"]`)?.focus();} };
      list.querySelectorAll('[data-agent-move]').forEach(button=>button.onclick=()=>{const order=[...entryIds()],index=order.indexOf(button.dataset.agentMove),target=index+Number(button.dataset.direction); if(target<0||target>=order.length)return; [order[index],order[target]]=[order[target],order[index]];void saveOrder(order,button.dataset.agentMove);});
      let dragged='';list.querySelectorAll('[draggable=true]').forEach(row=>{row.ondragstart=event=>{dragged=row.dataset.orderAgent;event.dataTransfer.setData('text/plain',dragged);};row.ondragover=event=>event.preventDefault();row.ondrop=event=>{event.preventDefault();const order=[...entryIds()],from=order.indexOf(dragged),to=order.indexOf(row.dataset.orderAgent);if(from<0||to<0)return;order.splice(from,1);order.splice(to,0,dragged);void saveOrder(order,dragged);};});
      list.querySelectorAll('[data-agent-manage]').forEach(button => button.onclick = async () => {
        if (state.settingsSaving) return;
        state.settingsSaving = true;list.querySelectorAll('button').forEach(el => el.disabled = true);
        try {
          const provider = button.dataset.agentManage, visible = button.dataset.visible === 'true';
          accept(await api('agents.manage', {provider, visible}));markSaved();
          if (visible) navigate({provider});else if (state.settingsAgent === provider) state.settingsAgent = entryIds()[0] || '';
          renderSettings(false);draw();list.querySelector(`[data-agent-manage="${CSS.escape(provider)}"]`)?.focus();backgroundRefresh();
        } catch (error) {notify(AieyesI18n.message(error.message || String(error)), 'error');draw();}
        finally {state.settingsSaving = false;renderSettings(false);}
      });
    };
    search.oninput = draw;draw();search.focus();
  }
  document.addEventListener('pointerdown', event => {document.querySelectorAll('.agent-more[open]').forEach(menu => {if (!menu.contains(event.target)) menu.open = false;});});
  function bindSelector() {
    document.querySelectorAll('[data-manage-agents]').forEach(button => button.onclick = manageEntries);
    const more = document.querySelector('.agent-more');if (!more) return;
    const search = more.querySelector('[data-agent-search]');
    more.addEventListener('toggle', () => {if (more.open) search.focus();});
    more.querySelectorAll('[data-agent-choice]').forEach(button => button.onclick = () => run(async () => { const provider=button.dataset.agentChoice; if(!entryIds().includes(provider)) accept(await api('agents.manage',{provider,visible:true})); navigate({provider});renderSettings();}));
    search.oninput = () => { more.querySelectorAll('[data-agent-choice]').forEach(button => button.hidden = !(button.textContent + ' ' + button.dataset.agentChoice).toLowerCase().includes(search.value.trim().toLowerCase())); more.querySelectorAll('[data-agent-picker-group]').forEach(group=>group.hidden=![...group.querySelectorAll('button')].some(b=>!b.hidden)); more.querySelector('[data-agent-no-results]').hidden=[...more.querySelectorAll('[data-agent-choice]')].some(b=>!b.hidden); };
    more.addEventListener('keydown', event => {if (event.key === 'Escape') {event.preventDefault();event.stopPropagation();more.open = false;more.querySelector('summary').focus();}});
    more.addEventListener('focusout', () => setTimeout(() => {if (!more.contains(document.activeElement)) more.open = false;},0));
  }
  function integrationsHTML() {
    state.settingsAgent = AieyesUI.agentProvider(state.settingsAgent);
    const agent = configurations().find((a) => a.provider === state.settingsAgent) ?? configurations()[0];
    if (!agent || (!entryIds().length && !detailKey)) return selectorHTML(null) + `<div class="empty">${AieyesI18n.t("添加要接入的 Agent")}</div>`;
    state.settingsAgent = agent.provider;
    let body;
    if (directorySelection) body = agentsHTML();else
    if (showCodex || state.settings.accounts.some((a) => accountKey(a) === detailKey)) body = accountsHTML();else
    {
      const section = (key, title, detail, content, opened = false) => `<details class="${document.documentElement.dataset.platform === 'windows' ? '' : 'card '}settings-disclosure" id="agent-section-${key}" data-agent-detail="settings:${agent.provider}:${key}" data-persist-disclosure="settings:${agent.provider}:${key}" ${disclosureOpen('settings:' + agent.provider + ':' + key, opened) ? 'open' : ''}><summary><strong>${title}</strong><small>${e(detail)}</small></summary><div class="settings-section-body">${content}</div></details>`;
      body = `<section class="card agent-integration-header" data-provider="${e(agent.provider)}"><div class="between"><h2>${providerMark(agent.provider)} ${e(providers[agent.provider])}</h2><label><input type="checkbox" role="switch" data-agent-enable="${e(agent.provider)}" ${state.settingsSaving ? 'disabled' : ''} ${agent.enabled ? 'checked' : ''}>${AieyesI18n.t("启用")}</label></div></section>`;
      if (['pi', 'omp'].includes(agent.provider)) body += helpHTML(AieyesI18n.t("额度"), AieyesI18n.t("额度由当前模型服务商提供；本地 Token 和预估费用不代表可用余额。"));
      const grouped = document.documentElement.dataset.platform === 'windows';
      if(grouped) body += '<section class="card agent-settings-group">';
      body += section('accounts', AieyesI18n.t("账户"), AieyesI18n.t("{0} 个账户", state.settings.accounts.filter(a => AieyesUI.sameAgent(a.provider, agent.provider) && !a.archived).length), accountsHTML(false), true);
      const apiProvider = AieyesUI.featureProvider(agent.provider, 'api');
      if (apiProvider) {
        const nativeApiSources = state.settings.sources.filter((s) => s.provider === apiProvider && !s.hostId && !s.subscription && s.machineBinding !== 'unbound');
        const protocol = apiProvider === 'claude' ? 'Anthropic Messages' : 'Chat Completions';
        body += section('api', AieyesI18n.t("API 连接"), apiProvider === 'codex' ? AieyesI18n.t("{0} 个连接", (state.settings.codexApis ?? []).length) : AieyesI18n.t("自定义模型"), apiProvider === 'codex' ? window.AieyesCodexAPI?.html() ?? '' : helpHTML(AieyesI18n.t("API 连接"), AieyesI18n.t("验证 {0} 端点后，保存到本机 {1} 配置并设为默认模型。已有配置会备份，Key 保存在原生配置文件中。", protocol, providers[apiProvider])) + nativeApiSources.map((s) => `<button type="button" data-native-api="${e(s.id)}">${AieyesI18n.t("{0} · 配置 API", e(s.name))}</button>`).join('') + (!nativeApiSources.length ? `<p class="muted">${AieyesI18n.t("先在机器与目录中添加本机数据目录。")}</p>` : ''));
      }
      const nativeApis = window.AieyesAccountScan?.list(agent.provider, 'api') ?? '';
      if (nativeApis) body += section('native-api', AieyesI18n.t("原生 API 连接"), '', nativeApis);
      if (AieyesUI.agentSupports(agent.provider, 'machines', state.settings)) body += section('machines', AieyesI18n.t("机器与目录"), AieyesI18n.t("{0} 台机器", agent.machineIds.length), agentsHTML(grouped));
      if (AieyesUI.agentSupports(agent.provider, 'valuation', state.settings)) body += section('valuation', AieyesI18n.t("估值与采样"), '', `<button type="button" data-agent-estimates="${e(agent.provider)}">${AieyesI18n.t("管理估值与采样")}</button>`);
      if (grouped) body += '</section>';
    }
    return `<div class="settings-layout agent-settings-layout">${selectorHTML(agent)}<div class="settings-subpage" data-agent-section="${e(agent.provider)}">${body}</div></div>`;
  }
  function directoryLinks(provider, machine) {
    if (machine === 'local' || !state.settings.hosts.some((h) => h.id === machine)) return '';
    const sources = machineSources(provider, machine),accounts = state.settings.accounts.filter((a) => !a.archived && a.provider === provider && sources.some((s) => linked(a, s)));
    const buttons = accounts.map((a) => `<button type="button" data-directory-account="${e(accountKey(a))}" data-machine="${e(machine)}">${e(accounts.length === 1 ? AieyesI18n.t("账户前置命令") : a.name)}</button>`).join('');
    return `<div class="actions directory-shortcuts"><button type="button" data-directory-shell="${e(machine)}">${AieyesI18n.t("远程 Shell 设置")}</button>${accounts.length > 1 ? `<details><summary>${AieyesI18n.t("账户前置命令")}</summary><div class="actions">${buttons}</div></details>` : buttons || `<button type="button" data-directory-add-account="${e(provider)}">${AieyesI18n.t("添加账户")}</button>`}</div>`;
  }
  function agentsHTML(embedded = false) {
    if (directorySelection) {const { provider, machine } = directorySelection,unbound = machine !== 'local' && !state.settings.hosts.some((h) => h.id === machine);return `<button id="directories-back">${AieyesI18n.t("返回 Agent 接入")}</button><section class="card"><h2>${e(providers[provider])} · ${e(deviceName(machine))}</h2>${directorySources(provider, machine).map((s) => `<div class="list-row"><code class="row-body">${e(s.path)}</code>${s.machineBinding === 'unbound' ? `<span class="muted">${AieyesI18n.t("机器未绑定")}</span><button data-source-rebind="${e(s.id)}">${AieyesI18n.t("重新绑定机器")}</button>` : `<button data-source-advanced="${e(s.id)}">${AieyesI18n.t("编辑目录")}</button>`}<button class="danger" data-source-remove="${e(s.id)}">${AieyesI18n.t("移除目录配置")}</button></div>`).join('')}${unbound ? '' : `<button data-source-add="${e(provider)}">${AieyesI18n.t("添加目录")}</button>`}<p class="muted">${AieyesI18n.t("配置采集位置，原文件保留。")}</p>${directoryLinks(provider, machine)}</section>`;}
    return rawConfigurations().filter((a) => a.provider === AieyesUI.featureProvider(state.settingsAgent, 'machines')).map((raw) => {
      const a = { ...raw, enabled: configurations().find((group) => AieyesUI.sameAgent(group.provider, raw.provider))?.enabled ?? raw.enabled };
      const unbound = [...new Set(state.settings.sources.filter((s) => s.provider === a.provider && s.machineBinding === 'unbound').map((s) => s.hostId))].filter((id) => !state.settings.hosts.some((h) => h.id === id)).map((id) => ({ id, name: state.settings.sources.find((s) => s.hostId === id)?.lastMachineName ?? id, unbound: true, enabled: false }));
      const hosts = [{ id: 'local', name: AieyesI18n.t("本机"), enabled: state.settings.localDeviceEnabled !== false }, ...state.settings.hosts, ...unbound];
      return `<section class="card agent-setup" id="agent-machines">${embedded ? '' : `<h2>${AieyesI18n.t("机器与目录")}</h2>`}<fieldset class="agent-machines"><legend>${AieyesI18n.t("所在机器 · 可多选")}</legend>${hosts.map((h) => {
        const sources = directorySources(a.provider, h.id),current = machineState(a.provider, h.id);
        const managementSource = current.sources.find(s => current.rows.some(r => r.sourceId === s.id && (r.current === true || r.credential === true))) ?? current.sources[0];
        const actions = (managementSource ? [managementSource] : []).map((source) => (a.provider === 'antigravity' ? `<button data-agy-login="${e(source.id)}" title="${e(source.path)}">${AieyesI18n.t("登录")}</button>` : '') + (a.provider === 'codex' ? `<button data-device-manage="${e(source.id)}" data-intent="login" title="${e(source.path)}">${AieyesI18n.t("登录")}</button><button data-device-manage="${e(source.id)}" data-intent="switch" title="${e(source.path)}">${AieyesI18n.t("切换账户")}</button>` : '')).join('');
        return `<div class="agent-machine list-row"><label><input type="checkbox" data-agent-machine="${e(a.provider)}" ${state.settingsSaving || !a.enabled || h.unbound ? 'disabled' : ''} value="${e(h.id)}" ${!h.unbound && a.machineIds.includes(h.id) ? 'checked' : ''}>${e(h.name || h.target)}${h.unbound ? AieyesI18n.t(" · 机器未绑定") : h.enabled ? '' : AieyesI18n.t(" · 采集已暂停")}</label><span class="row-body tiny muted machine-current" data-machine-current="${e(h.id)}" data-provider="${e(a.provider)}" title="${e(current.text)}">${h.unbound ? '' : e(current.text)}</span><button data-machine-directories="${e(a.provider)}" data-machine="${e(h.id)}">${AieyesI18n.t("目录 {0}", sources.length)}</button>${h.unbound ? '' : hooksHTML(a.provider, h.id)}${automaticMachineHTML(a.provider, h.id)}${h.unbound ? '' : `<details class="machine-menu"><summary aria-label="${AieyesI18n.t("{0}操作", e(h.name || h.target))}">⋯</summary><div class="machine-menu-body">${actions || `<p class="tiny muted">${AieyesI18n.t("请启用此机器的采集并配置日志目录。")}</p>`}<button data-device-enabled="${e(h.id)}" data-enabled="${!h.enabled}">${h.enabled ? AieyesI18n.t("暂停 Agent 采集") : AieyesI18n.t("恢复 Agent 采集")}</button></div></details>`}</div>`;
      }).join('')}</fieldset>${!hosts.some((h) => !h.unbound && h.enabled) ? `<p class="muted">${AieyesI18n.t("暂无可采集机器，请在服务器设置开启 Agent 采集。")}</p>` : a.machineIds.some((id) => hosts.some((h) => !h.unbound && h.id === id)) ? '' : `<p class="muted">${AieyesI18n.t("未选择机器，暂不采集。")}</p>`}<button data-agent-host>${AieyesI18n.t("添加 SSH 主机")}</button>${a.provider === 'qoder' ? `<button data-machine-account-refresh="qoder" ${statusLoading || !a.enabled ? 'disabled' : ''}>${statusLoading ? AieyesI18n.t("检查中…") : AieyesI18n.t("刷新账户状态")}</button>` : ''}</section>`;
    }).join('');
  }
  function accountsHTML(includeAPI = true) {
    if (showCodex) return `<button type="button" id="accounts-back" class="back-link">${uiIcon('back')}${AieyesI18n.t("返回账户")}</button>` + window.AieyesCodexAuth.html();
    const detail = state.settings.accounts.find((a) => accountKey(a) === detailKey);
    if (detail) return detailsHTML(detail);
    const row = (a) => `<div class="list-row"><div class="row-body"><strong>${e(a.name)}</strong><small data-account-summary="${e(accountKey(a))}">${e(counts(a))}</small><small data-account-identity="${e(accountKey(a))}" class="account-identity">${e(identityText(statusRows.find((r) => r.accountKey === accountKey(a) && r.current === true && r.identity)))}</small></div><button data-account-edit="${e(accountKey(a))}">${AieyesI18n.t("设置")}</button></div>`;
    const groups = configurations().filter((config) => config.provider === state.settingsAgent);
    return syncHTML() + groups.map((config) => `<section class="card account-agent-group" id="agent-accounts"><div class="between">${window.AieyesAccountScan?.button(config.provider) ?? ''}<button data-account-add="${e(config.provider)}" class="tinted" ${config.enabled ? '' : 'disabled'}>${AieyesI18n.t("添加账户")}</button></div>${window.AieyesAccountScan?.hint(config.provider) ?? ''}${state.settings.accounts.filter((a) => AieyesUI.sameAgent(a.provider, config.provider) && !a.archived).map(row).join('') || `<p class="muted">${AieyesI18n.t("尚未添加账户")}</p>`}${window.AieyesAccountScan?.list(config.provider, 'login') ?? ''}</section>`).join('') + (
    includeAPI && state.settingsAgent === 'codex' ? window.AieyesCodexAPI?.html() ?? '' : '') + (
    state.settings.accounts.some((a) => a.archived && AieyesUI.sameAgent(a.provider, state.settingsAgent)) ? `<details class="card" data-agent-detail="archived-accounts"><summary>${AieyesI18n.t("已归档账户")}</summary>${state.settings.accounts.filter((a) => a.archived && AieyesUI.sameAgent(a.provider, state.settingsAgent)).map((a) => `<div class="list-row"><span class="row-body">${e(providers[a.provider])} · ${e(a.name)}</span><button data-account-restore="${e(accountKey(a))}">${AieyesI18n.t("恢复")}</button><button class="danger" data-account-delete="${e(accountKey(a))}">${AieyesI18n.t("删除账户")}</button></div>`).join('')}</details>` : '') + (
    cleanupRows.length ? `<details class="card" data-agent-detail="abandoned-tasks"><summary>${AieyesI18n.t("已解除管理的远端任务（可能仍在运行）")}</summary>${cleanupRows.map((r) => `<h3>${e(r.name)}</h3>${r.tasks.map((t) => `<p>${e(t.name)} · ${e(t.machine)} · <code>${e(t.root)}</code></p>`).join('')}`).join('')}</details>` : '');
  }
  function syncContent() {return (statusError ? `<div class="error" id="account-status-error" role="alert">${e(statusError)} <button data-account-retry-status>${AieyesI18n.t("重试读取")}</button></div>` : '') + (syncMessage ? `<div class="error">${e(syncMessage)} <button data-deployment-sync>${AieyesI18n.t("重试同步")}</button></div>` : '');}
  function syncHTML() {return `<div data-account-sync>${syncContent()}</div>`;}
  function legacyCommand(s) {return s.quotaPreCommand || state.settings.hosts.find((h) => h.id === s.hostId)?.preCommand || '';}
  function deviceCommand(a, id) {const configured = a.deviceSettings?.find((d) => d.machineId === id);if (configured) return configured.preCommand;const values = [...new Set(state.settings.sources.filter((s) => s.provider === a.provider && machineId(s) === id).map(legacyCommand))];return values.length === 1 ? values[0] : '';}
  const draftValue = (a, field, fallback) => accountDrafts.get(accountKey(a) + ':' + field) ?? fallback;
  function queueAccountField(a, field, value) {
    const key = accountKey(a),draftKey = key + ':' + field;
    accountDrafts.set(draftKey, value);pendingSaves++;
    saveQueue = saveQueue.catch(() => {}).then(async () => {
      try {
        if (field === 'name' && !String(value).trim()) throw new Error(AieyesI18n.t("请输入账户名称"));
        if (field === 'quotaRefreshSeconds' && value !== '' && (!/^\d+$/.test(String(value)) || Number(value) < 5 || Number(value) > 86400)) throw new Error(AieyesI18n.t("查询间隔范围为 5–86400 秒"));
        const next = structuredClone(state.settings),row = next.accounts.find((a) => accountKey(a) === key);if (!row) throw new Error(AieyesI18n.t("账户已移除"));
        if (field.startsWith('command:')) {const machine = field.slice(8);row.deviceSettings = (row.deviceSettings ?? []).filter((d) => d.machineId !== machine);row.deviceSettings.push({ machineId: machine, preCommand: value });} else
        if (field.startsWith('quotaResetNotifications.')) {const kind = field.split('.')[1];row.quotaResetNotifications = { ...row.quotaResetNotifications, [kind]: value === 'inherit' ? null : value === 'on' };} else
        row[field] = field === 'quotaRefreshSeconds' ? value === '' ? null : Number(value) : field === 'name' ? value.trim() : value;
        await commit(next);accountErrors.delete(draftKey);
        if (accountDrafts.get(draftKey) === value) accountDrafts.delete(draftKey);
      } catch (error) {accountErrors.set(draftKey, error.message ?? String(error));} finally
      {pendingSaves--;repaint();}
    });return saveQueue;
  }
  function resetPreferencesHTML(a) {
    if (!AieyesUI.supports(a.provider, 'quotaReset', state.settings)) return '';
    if (state.settings.sources.some((s) => linked(a, s) && s.subscription?.mode === 'api') && a.provider === 'grok') return '';
    return [['unexpected', AieyesI18n.t("提前重置提醒"), true], ['natural', AieyesI18n.t("到期重置提醒"), false]].map(([kind, label, fallback]) => {
      const current = a.quotaResetNotifications?.[kind],selected = draftValue(a, 'quotaResetNotifications.' + kind, current == null ? 'inherit' : current ? 'on' : 'off');
      const inherited = state.settings.quotaResetNotifications?.[kind] ?? fallback;
      return `<label>${label}<select data-account-reset="${kind}" data-account="${e(accountKey(a))}">${option('inherit', AieyesI18n.t("沿用全局 · ") + (inherited ? AieyesI18n.t("开启") : AieyesI18n.t("关闭")), selected)}${option('on', AieyesI18n.t("开启"), selected)}${option('off', AieyesI18n.t("关闭"), selected)}</select></label>`;
    }).join('');
  }
  function sourceStatusHTML(row) {
    return (row?.checkedAt ? `<small>${AieyesI18n.t("检查于 {0}", e(new Date(row.checkedAt * 1000).toLocaleString(AieyesI18n.locale)))}</small>` : '') + (row?.error || row?.note ? `<small>${e(AieyesI18n.message(row.error || row.note))}</small>` : '');
  }
  function sourceActionsHTML(a, s, row) {
    const active = AieyesUI.sourceEnabled(state.settings, s);
    if (a.provider === 'antigravity' && s.accountId === a.id) return `<button data-agy-login="${e(s.id)}" ${active ? '' : 'disabled'}>${AieyesI18n.t("登录 / 检查")}</button>`;
    if (a.provider !== 'codex') return '';
    if (row?.current === true) return `<span class="muted">${AieyesI18n.t("正在使用")}</span>`;
    return `<button data-account-login="${e(accountKey(a))}" data-source="${e(s.id)}" data-intent="${row?.credential ? 'switch' : 'login'}" ${active ? '' : 'disabled'}>${row?.credential ? AieyesI18n.t("切换账户") : AieyesI18n.t("登录")}</button>`;
  }
  function bindStatusActions() {
    document.querySelectorAll('[data-account-retry-status]').forEach((button) => button.onclick = () => refreshStatuses(true));
    document.querySelectorAll('[data-account-login]').forEach((b) => b.onclick = async () => {if (!(await flush())) return;const a = state.settings.accounts.find((a) => accountKey(a) === b.dataset.accountLogin);if (!a) return;showCodex = true;window.AieyesCodexAuth.open(a.id, b.dataset.source, b.dataset.intent);renderSettings(false);});
  }
  function detailsHTML(a) {
    const selected = AieyesUI.panelAccounts(state.settings, state.dashboard?.quotaOrder ?? []),isSelected = selected[a.provider]?.includes(accountKey(a));
    const hasQuota = AieyesUI.supports(a.provider, 'quota', state.settings);
    const priority = `<label>${AieyesI18n.t("优先查询位置")}<select data-account-priority="${e(accountKey(a))}">${option('', AieyesI18n.t("自动 · 优先本机"), draftValue(a, 'quotaSourceId', a.quotaSourceId) || '')}${state.settings.sources.filter((s) => linked(a, s)).map((s) => option(s.id, deviceName(machineId(s)) + ' · ' + s.name + (s.enabled ? '' : AieyesI18n.t(" · 已暂停")), draftValue(a, 'quotaSourceId', a.quotaSourceId) || '')).join('')}</select></label>`;
    const interval = `<label>${AieyesI18n.t("查询间隔（秒）")}<input id="account-interval" data-account-field="quotaRefreshSeconds" data-account="${e(accountKey(a))}" type="number" min="5" max="86400" value="${e(draftValue(a, 'quotaRefreshSeconds', a.quotaRefreshSeconds ?? ''))}" placeholder="${AieyesI18n.t("沿用全局 · {0} 秒", state.settings.refreshSeconds)}"></label>`;
    const quota = state.dashboard?.quotas?.find((q) => q.provider === a.provider && q.accountId === a.id);
    const content = `<div class="between"><button type="button" id="account-detail-back" class="back-link">${uiIcon('back')}${AieyesI18n.t("返回账户")}</button><button id="account-status-refresh" ${statusLoading ? 'disabled' : ''}>${statusLoading ? AieyesI18n.t("检查中…") : AieyesI18n.t("刷新设备状态")}</button></div>
      <section class="card account-config"><h2>${providerMark(a.provider)} ${e(a.name)}</h2><p data-account-summary="${e(accountKey(a))}">${e(counts(a))}</p>
      <label>${AieyesI18n.t("账户名称")}<input id="account-name" data-account-field="name" data-account="${e(accountKey(a))}" value="${e(draftValue(a, 'name', a.name))}"></label>
      ${hasQuota && quota?.error ? `<div class="error">${AieyesUI.errorHTML(quota.error, AieyesI18n.t("额度查询失败"))}</div>` : ''}
      <div class="actions">${hasQuota ? `<label><input type="checkbox" role="switch" data-account-quota="${e(accountKey(a))}" ${draftValue(a, 'quotaEnabled', a.quotaEnabled) ? 'checked' : ''}>${AieyesI18n.t("显示并查询额度")}</label><label><input type="checkbox" role="switch" data-account-panel="${e(accountKey(a))}" ${isSelected ? 'checked' : ''} ${!a.quotaEnabled ? 'disabled' : ''}>${AieyesI18n.t("在面板显示")}</label><button data-account-panel-manage>${AieyesI18n.t("调整面板选择")}</button>` : ''}${a.provider !== 'codex' ? `<button data-account-connect="${e(accountKey(a))}">${a.provider === 'deepseek' ? AieyesI18n.t("更新 API Key") : window.AieyesContract.subscriptions[a.provider] ? AieyesI18n.t("更新额度凭据") : AieyesI18n.t("连接机器")}</button>` : ''}</div>
      ${hasQuota ? `<div class="between"><span class="tiny muted" role="status">${AieyesI18n.t("已选 {0} 个账户", selected[a.provider]?.length ?? 0)}</span>${helpHTML(AieyesI18n.t("面板账户"), AieyesI18n.t("自动模式显示每类前 5 个账户。手动模式可留空；新增账户默认加入面板，可随时取消。"))}</div><details class="settings-details" data-agent-detail="query:${e(accountKey(a))}" data-persist-disclosure="query:${e(accountKey(a))}" ${disclosureOpen('query:' + accountKey(a), false) ? 'open' : ''}><summary>${AieyesI18n.t("查询与提醒")}</summary>${priority}${interval}${resetPreferencesHTML(a)}</details>` : ''}
      <label><input type="checkbox" role="switch" data-account-archive="${e(accountKey(a))}" ${draftValue(a, 'archived', a.archived) ? 'checked' : ''}>${AieyesI18n.t("归档账户")}</label><p class="muted tiny">${pendingSaves ? AieyesI18n.t("正在保存…") : ''}</p>
      <div data-account-save-errors="${e(accountKey(a))}">${[...accountErrors].filter(([key]) => key.startsWith(accountKey(a) + ':')).map(([key, message]) => `<div class="error">${AieyesUI.errorHTML(message, AieyesI18n.t("保存失败，输入已保留"))} <button data-account-retry="${e(key)}">${AieyesI18n.t("重试保存")}</button></div>`).join('')}</div>${syncHTML()}</section>` +
    machines(a.provider, true).map((id) => {
      const sources = state.settings.sources.filter((s) => s.provider === a.provider && machineId(s) === id),active = machines(a.provider).includes(id),rows = statuses(a, id);
      const resetRequired = a.deviceSettings?.some((d) => d.machineId === id && d.preCommandResetRequired);
      return `<section class="card"><h3>${e(deviceName(id))}${active ? '' : AieyesI18n.t(" · 已暂停")}</h3><p class="muted" data-device-summary="${e(id)}" data-account="${e(accountKey(a))}">${e(AieyesI18n.message(statusText(rows)))}</p>${sources.map((s) => {const r = rows.find((r) => r.sourceId === s.id);return `<div class="list-row"><div class="row-body">${e(s.name)}<small>${e(s.path || (s.subscription?.mode === 'cli' ? AieyesI18n.t("CLI 额度连接") : s.subscription ? AieyesI18n.t("订阅额度连接") : ''))}</small><span data-source-status="${e(s.id)}" data-account="${e(accountKey(a))}">${sourceStatusHTML(r)}</span></div><span data-source-actions="${e(s.id)}" data-account="${e(accountKey(a))}">${sourceActionsHTML(a, s, r)}</span></div>`;}).join('')}${id !== 'local' ? `<details data-agent-detail="command:${e(accountKey(a))}:${e(id)}"><summary>${AieyesI18n.t("前置命令")}</summary>${helpHTML(AieyesI18n.t("前置命令"), AieyesI18n.t("用于此账户的授权、切换、额度查询和定时唤醒；历史同步、监控不执行。清空后不执行。"))}${resetRequired ? `<div class="error">${AieyesI18n.t("此机器的旧目录前置命令不一致，已清空，请重新设置。留空保存表示不执行前置命令。")}</div><button type="button" data-command-confirm="` + e(id) + '" data-account="' + e(accountKey(a)) + `">${AieyesI18n.t("确认当前命令")}</button>` : ''}<label class="command-field"><textarea aria-label="${AieyesI18n.t("前置命令")}" id="account-command-${e(id)}" data-command-field="${e(id)}" data-account="${e(accountKey(a))}">${e(draftValue(a, 'command:' + id, deviceCommand(a, id)))}</textarea></label></details>` : ''}</section>`;
    }).join('');
    return document.documentElement.dataset.platform === 'windows' ? `<section class="card account-settings-group">${content}</section>` : content;
  }
  async function deleteAccount(a) {
    let preview;try {preview = await api('accounts.deletion.preview', { accountKey: accountKey(a) });} catch (error) {notify(String(error), 'error');return;}
    const tasks = preview.tasks ?? [];
    confirmDestructive(AieyesI18n.t("删除归档账户「") + a.name + '」', `<p>${AieyesI18n.t("删除账户配置和关联任务；历史记录及机器凭证保留，不退出 Agent。")}</p><ul>` + tasks.map((t) => `<li>${e(t.name)} · ${e(t.machine)}</li>`).join('') + '</ul>' + (tasks.length ? `<label><input type="checkbox" name="force">${AieyesI18n.t("强制删除（设备已弃用或无法连接）")}</label><div class="error">${AieyesI18n.t("强制删除无法保证远端任务停止。将解除本地管理，不再自动重试；未清理位置保留供查看。")}</div>` : '') + '<div id="account-delete-failures"></div>', async (f) => {
      const result = await api('accounts.delete', { accountKey: accountKey(a), force: f.has('force') });
      if (!result.deleted) {$('#account-delete-failures').innerHTML = (result.failedTasks ?? []).map((t) => `<div class="error">${e(t.name)} · ${e(AieyesI18n.message(t.error))}</div>`).join('');throw new Error(AieyesI18n.t("部分任务清理失败，账户仍保留；可重试或勾选强制删除。"));}
      accept(result.settings);cleanupRows = await api('accounts.cleanup.list');if (!Array.isArray(cleanupRows)) cleanupRows = [];repaint();backgroundRefresh();
    }, AieyesI18n.t("确认删除"));
  }
  async function advanced(existing, provider) {
    const base = structuredClone(state.settings),item = structuredClone(existing ?? { id: crypto.randomUUID(), provider, name: providers[provider], accountId: '', path: window.AieyesContract.defaultPaths[provider] || '', hostId: directorySelection?.machine === 'local' ? null : directorySelection?.machine ?? null, enabled: true, codexBinary: window.AieyesContract.binaries.codex, agyBinary: window.AieyesContract.binaries.antigravity, quotaCommand: '', quotaPreCommand: '', proxy: null });
    let defaultError = '';
    if (!existing) {
      try {item.path = (await api('sources.defaultPath', { provider: item.provider, hostId: item.hostId })).path;}
      catch (error) {item.path = '';defaultError = error.message ?? String(error);}
    }
    showEditor(AieyesI18n.t("{0} · 目录设置", providers[item.provider]), select('hostId', AieyesI18n.t("机器"), [['', AieyesI18n.t("本机")], ...base.hosts.map((h) => [h.id, h.name || h.target])], item.hostId || '') + (item.subscription ? `<input type="hidden" name="path" value=""><p class="muted">${AieyesI18n.t("此来源只查询订阅；历史请在机器与采集中添加独立目录。")}</p>` : field('path', item.provider === 'deepseek' ? AieyesI18n.t("API Key 文件") : AieyesI18n.t("数据目录"), item.path)) + (item.codexHomeId ? `<p class="muted">${AieyesI18n.t("更换目录会解除旧账户连接，保留原文件和凭据。之后请读取新目录的登录。")}</p>` : '') + (item.provider === 'codex' ? field('codexBinary', AieyesI18n.t("Codex 程序"), item.codexBinary) : '') + (item.provider === 'antigravity' ? field('agyBinary', AieyesI18n.t("agy 程序"), item.agyBinary) : '') + (['custom', 'kimi', 'deepseek-harness', 'codebuddy'].includes(item.provider) || window.AieyesContract.subscriptions[item.provider] ? textarea('quotaCommand', AieyesI18n.t("额度查询命令"), item.quotaCommand || '') : '') + (item.hostId ? `<p class="muted tiny">${AieyesI18n.t("远程账户的前置命令在账户设置中配置，同机目录共享。")}</p>` : proxyFields('source', item.proxy, true)), async (f) => {
      if (defaultError && !item.subscription && !f.get('path').trim()) throw new Error(AieyesI18n.t("请选择实际的原生数据目录后保存"));
      item.path = f.get('path').trim();if (!['antigravity', 'deepseek'].includes(item.provider) && !window.AieyesContract.subscriptions[item.provider] && !item.path) throw new Error(AieyesI18n.t("请输入数据目录"));
      for (const key of ['codexBinary', 'agyBinary', 'quotaCommand']) if (f.has(key)) item[key] = f.get(key);
      item.proxy = item.hostId ? null : proxyValue(f, 'source');
      accept(await api('sources.configure', { source: item, ...(existing ? { baseSource: existing } : {}) }));markSaved();backgroundRefresh();renderSettings(false);
    });
    if (defaultError) {$('#editor-error').textContent = AieyesI18n.message(defaultError);$('#editor-error').hidden = false;}
    $('#field-hostId').disabled = true;if (!item.hostId) bindProxy('source');bindPathSelection(true);
  }
  function removeDirectory(source) {confirmDestructive(AieyesI18n.t("移除目录配置"), `<p>${AieyesI18n.t("仅解除此目录的采集和账户连接；文件、登录凭证和历史记录保留。")}</p><code>` + e(source.path) + '</code>', async () => {accept(await api('sources.remove', { sourceId: source.id, baseSource: source }));markSaved();backgroundRefresh();renderSettings(false);}, AieyesI18n.t("移除配置"));}
  function editAccount(account) {state.settingsAgent = AieyesUI.agentProvider(account.provider);detailKey = accountKey(account);showCodex = false;renderSettings(false);}
  function subscriptionAccount(account, provider) {
    const spec = window.AieyesContract.subscriptions[provider];
    const existing = state.settings.sources.find((s) => s.provider === provider && s.accountId === account?.id && s.subscription);
    const config = existing?.subscription || {};
    showEditor(AieyesI18n.t("{0} · 连接订阅", providers[provider]),
    '<p class="muted">' + e(spec.hint) + `</p><button type="button" id="subscription-login">${AieyesI18n.t("打开官方登录 / 用量页")}</button>` +
    field('name', AieyesI18n.t("账户名称"), account?.name || AieyesI18n.t("{0} 账户", providers[provider])) +
    select('mode', AieyesI18n.t("服务 / 地区"), Object.entries(spec.modes), config.mode || Object.keys(spec.modes)[0]) +
    field('workspaceId', AieyesI18n.t("Workspace / Team ID（仅 Console 或 xAI API）"), config.workspaceId || '') + (
    provider === 'qoder' ? field('nativeConfigDir', AieyesI18n.t("CLI 配置目录"), config.nativeConfigDir || '', AieyesI18n.t("留空跟随 CLI 默认目录")) : '') +
    field('secret', AieyesI18n.t("API Key / 登录令牌 / Cookie"), '', '', 'password') + (
    ['grok', 'opencode'].includes(provider) ? `<label><input type="checkbox" name="useLocalLogin">${AieyesI18n.t("读取本机已登录凭据（仅 Grok 订阅 / OpenCode Go）")}</label>` : '') +
    '<p class="muted tiny" id="subscription-storage-note"></p>', async (f) => {
      const result = await api('subscriptions.connect', { provider, accountId: account?.id || '', name: f.get('name'), secret: f.get('secret'), useLocalLogin: f.has('useLocalLogin'), subscription: { mode: f.get('mode'), workspaceId: (f.get('workspaceId') || '').trim(), nativeConfigDir: (f.get('nativeConfigDir') || '').trim() } });
      accept(result.settings);markSaved();backgroundRefresh();renderSettings(false);
    });
    $('#editor').dataset.kind = 'account';
    const modeChanged = () => {
      const mode = $('#field-mode').value,native = provider === 'qoder' && mode === 'cli';
      const localLogin = provider === 'grok' && mode === 'subscription' || provider === 'opencode' && mode === 'go';
      const localToggle = $('#editor-form [name=useLocalLogin]');
      if (localToggle) {localToggle.disabled = !localLogin;localToggle.closest('label').hidden = !localLogin;}
      const visibility = { secret: !native && !(localLogin && localToggle?.checked), workspaceId: provider === 'grok' && mode === 'api' || provider === 'opencode' && mode === 'console', nativeConfigDir: native, mode: Object.keys(spec.modes).length > 1 };
      for (const [id, visible] of Object.entries(visibility)) {
        const input = $('#field-' + id);if (!input) continue;
        input.closest('.form-row').hidden = !visible;
        if (id !== 'mode') input.disabled = !visible;
      }
      $('#subscription-login').hidden = native;
      $('#subscription-storage-note').textContent = native ? AieyesI18n.t("读取 CLI 当前登录并绑定账户身份，无需填写密钥。CLI 切换账号后请切回原账户或另建账户；查询失败不创建账户。") : AieyesI18n.t("验证成功后保存到系统凭据库。不会切换客户端账户；登录失败不创建账户。");
    };
    $('#field-mode').onchange = modeChanged;
    const localToggle = $('#editor-form [name=useLocalLogin]');if (localToggle) localToggle.onchange = modeChanged;
    modeChanged();
    $('#subscription-login').onclick = () => run(() => api('subscriptions.openLogin', { provider, mode: $('#field-mode').value }));
  }
  function addAccount(account, selectedProvider) {
    const enabled = configurations().filter((a) => a.enabled),initial = account?.provider || selectedProvider || state.settingsAgent || enabled[0]?.provider;
    if (!initial) return;
    const legacyBalance = account && initial === 'deepseek' && state.settings.sources.some((s) => linked(account, s) && !s.subscription);
    if (window.AieyesContract.subscriptions[initial] && !legacyBalance) {subscriptionAccount(account, initial);return;}
    if (initial === 'codex') {detailKey = account ? accountKey(account) : '';showCodex = true;window.AieyesCodexAuth.open(account?.id || '');renderSettings(false);return;}
    let secret = null,secretSourceId = crypto.randomUUID();
    showEditor(providers[initial] + ' · ' + (account ? initial === 'deepseek' ? AieyesI18n.t("更新 API Key") : AieyesI18n.t("连接机器") : AieyesI18n.t("添加账户")), `<input id="field-provider" type="hidden" name="provider" value="${e(initial)}">` + '<div id="account-connection-fields"></div>', async (f) => {
      const provider = f.get('provider') || account?.provider,accountId = account?.id || '';
      const name = account?.name || AieyesI18n.t("{0} 账户", providers[provider]),base = structuredClone(state.settings);
      if (provider === 'deepseek') {
        const key = f.get('apiKey')?.trim();if (!key) throw new Error(AieyesI18n.t("请输入 API Key"));
        const next = structuredClone(base),id = accountId || crypto.randomUUID();
        if (!accountId) next.accounts.push({ id, provider, name, quotaEnabled: true, archived: false, connections: [] });
        let source = next.sources.find((s) => s.provider === provider && s.accountId === id);
        if (!source) {source = { id: secretSourceId, provider, name: name || 'DeepSeek', accountId: id, hostId: null, enabled: true, path: '', quotaCommand: '', quotaPreCommand: '', codexBinary: window.AieyesContract.binaries.codex, agyBinary: window.AieyesContract.binaries.antigravity, proxy: null };next.sources.push(source);}
        if (!secret || secret.key !== key) {const result = await api('credentials.save', { sourceId: secretSourceId + '-' + crypto.randomUUID(), apiKey: key });secret = { key, path: result.path };}
        source.path = secret.path;await commit(next, base);
      } else {const result = await api('accounts.connect', { provider, accountId, name, sourceIds: f.getAll('sourceIds') });accept(result.settings);markSaved();backgroundRefresh();}
      renderSettings(false);
    });
    $('#editor').dataset.kind = 'account';
    const draw = () => {
      const provider = $('#field-provider').value;
      $('#account-connection-fields').innerHTML = '<p class="muted">' + (provider === 'deepseek' ? AieyesI18n.t("添加账户的 API Key，之后可修改显示名称。") : AieyesI18n.t("读取所选机器上已有账户的登录。")) + '</p>' + (provider === 'deepseek' ? field('apiKey', 'API Key', '', '', 'password') : `<fieldset id="account-source-options"><legend>${AieyesI18n.t("已有登录的机器 · 可多选")}</legend></fieldset>`);
      const drawSources = () => {
        const id = account?.id || '';
        if ($('#account-source-options')) $('#account-source-options').innerHTML = state.settings.sources.filter((s) => AieyesUI.sourceEnabled(state.settings, s) && s.provider === provider && (!s.accountId || s.accountId === id)).map((s) => `<label><input type="checkbox" name="sourceIds" value="${e(s.id)}">${e(s.name)} · ${e(machineName(s))}<small class="source-path" title="${e(s.path)}">${e(AieyesUI.shortPath(s.path))}</small></label>${provider === 'antigravity' ? `<button type="button" data-agy-login="${e(s.id)}">${AieyesI18n.t("登录 / 检查")}</button>` : ''}`).join('') || `<p>${AieyesI18n.t("请在“接入与账户”选择机器和采集目录。每个目录连接一个当前登录账户。")}</p>`;
      };drawSources();
      $('#editor-form button[type=submit]').textContent = account ? provider === 'deepseek' ? AieyesI18n.t("更新 API Key") : AieyesI18n.t("保存连接") : AieyesI18n.t("添加账户");
    };draw();
  }
  async function nativeApiEditor(sourceId) {
    try {
      const provider = state.settings.sources.find((s) => s.id === sourceId)?.provider,name = providers[provider] ?? 'Agent';
      let loaded,initialError = '';
      try {loaded = await api('agentApi.inspect', { sourceId });}
      catch (error) {initialError = error.message ?? String(error);loaded = await api('agentApi.inspect', { sourceId, locationOnly: true });}
      const capacity = ['kimi', 'deepseek-harness', 'zcode', 'grok', 'qoder', 'pi', 'omp'].includes(provider),output = ['deepseek-harness', 'zcode', 'grok', 'qoder', 'pi', 'omp'].includes(provider),reasoning = ['claude', 'zcode', 'codebuddy', 'pi', 'omp'].includes(provider);
      showEditor(AieyesI18n.t("{0} · API 连接", name), (provider === 'qoder' ? `<p class="muted">${AieyesI18n.t("此配置目录需先完成 qodercli login；保存通过原生 CLI 的自定义模型管理接口。")}</p>` : '') + field('configPath', AieyesI18n.t("{0} 配置文件", name), loaded.configPath) + `<button type="button" id="native-api-read">${AieyesI18n.t("读取此配置")}</button><p id="native-api-read-status" role="status"></p>` + field('baseUrl', provider === 'claude' ? AieyesI18n.t("API 根地址（不含 /v1）") : AieyesI18n.t("Base URL（含 /v1）"), loaded.baseUrl) + field('model', AieyesI18n.t("模型 ID"), loaded.model) + (provider === 'claude' ? select('authMode', AieyesI18n.t("认证方式"), [['api-key', 'API Key · X-Api-Key'], ['auth-token', 'Bearer Token']], loaded.authMode || 'api-key') : '') + `<details class="settings-details"><summary>${AieyesI18n.t("模型选项")}</summary>` + (capacity ? field('contextWindow', AieyesI18n.t("模型上下文窗口（Token）"), loaded.contextWindow ?? '', AieyesI18n.t("例如 261000"), 'number') : '') + (output ? field('maxOutputTokens', AieyesI18n.t("最大输出 Token"), loaded.maxOutputTokens ?? '', AieyesI18n.t("例如 2048"), 'number') : '') + (reasoning ? select('effort', AieyesI18n.t("推理强度"), [['codebuddy', 'pi', 'omp'].includes(provider) ? ['disabled', AieyesI18n.t("不发送推理强度")] : ['', provider === 'zcode' ? AieyesI18n.t("服务端默认") : AieyesI18n.t("客户端默认")], ...(provider === 'codebuddy' ? [['minimal', 'minimal']] : []), ['low', 'low'], ['medium', 'medium'], ['high', 'high'], ['xhigh', 'xhigh'], ['max', 'max']], loaded.effort || (['codebuddy', 'pi', 'omp'].includes(provider) ? 'low' : '')) : '') + (provider === 'codebuddy' ? `<p class="muted">${AieyesI18n.t("新连接默认使用 low；模型不支持推理参数时，选择“不发送推理强度”。")}</p>` : '') + '</details>' + field('apiKey', provider === 'claude' ? AieyesI18n.t("API Key / Token（留空保留）") : AieyesI18n.t("API Key（留空保留已保存 Key）"), '', '', 'password') + `<label><input type="checkbox" name="noKey" ${loaded.noKey ? 'checked' : ''}>${AieyesI18n.t("服务无需 Key")}</label>${helpHTML(AieyesI18n.t("API 连接"), AieyesI18n.t("验证成功后保存并设为 {0} 默认模型。配置路径应与客户端实际使用的文件一致；客户端启动参数或环境变量可能覆盖文件设置。", name))}`, async (f) => {
        if (f.get('configPath') !== loaded.configPath) throw new Error(AieyesI18n.t("配置路径已改变，请先读取此配置"));
        if (!loaded.revision) throw new Error(AieyesI18n.t("请先读取有效配置后保存"));
        await api('agentApi.save', { sourceId, configPath: loaded.configPath, revision: loaded.revision, nativeRevision: loaded.nativeRevision, baseUrl: f.get('baseUrl'), model: f.get('model'), apiKey: f.get('apiKey'), noKey: f.get('noKey') === 'on', ...(capacity ? { contextWindow: Number(f.get('contextWindow')) } : {}), ...(output ? { maxOutputTokens: Number(f.get('maxOutputTokens')) } : {}), ...(reasoning ? { effort: f.get('effort') } : {}), ...(provider === 'claude' ? { authMode: f.get('authMode') } : {}) });
        notify(AieyesI18n.t("API 已验证并保存，可重新打开 {0} 使用。", name));
      });
      $('#native-api-read-status').textContent = initialError;
      $('#editor-form button[type=submit]').textContent = AieyesI18n.t("验证并设为默认");
      $('#native-api-read').onclick = async () => {
        const button = $('#native-api-read');button.disabled = true;
        try {
          loaded = await api('agentApi.inspect', { sourceId, configPath: $('#field-configPath').value });
          for (const key of ['configPath', 'baseUrl', 'model']) $('#field-' + key).value = loaded[key] ?? '';
          if ($('#field-contextWindow')) $('#field-contextWindow').value = loaded.contextWindow ?? '';
          if ($('#field-maxOutputTokens')) $('#field-maxOutputTokens').value = loaded.maxOutputTokens ?? '';
          if ($('#field-effort')) $('#field-effort').value = loaded.effort || (['codebuddy', 'pi', 'omp'].includes(provider) ? 'low' : '');
          if ($('#field-authMode')) $('#field-authMode').value = loaded.authMode ?? 'api-key';
          $('#field-apiKey').value = '';$('#editor-form [name=noKey]').checked = loaded.noKey;
          $('#native-api-read-status').textContent = loaded.hasApiKey ? AieyesI18n.t("已读取，Key 已保存") : AieyesI18n.t("已读取");
        } catch (error) {$('#native-api-read-status').textContent = AieyesI18n.message(error.message ?? String(error));} finally
        {button.disabled = false;}
      };
    } catch (error) {run(() => Promise.reject(error));}
  }
  function bind() {
    bindSelector();
    document.querySelectorAll('[data-native-api]').forEach((button) => button.onclick = () => nativeApiEditor(button.dataset.nativeApi));
    bindMachineMenus();
    bindStatusActions();
    paintHooks();
    document.querySelectorAll('[data-machine-hooks]').forEach((button) => button.onclick = () => openHooks(button.dataset.machineHooks, button.dataset.machine));
    document.querySelectorAll('[data-device-enabled]').forEach((button) => button.onclick = () => run(() => setDeviceEnabled(button.dataset.deviceEnabled, button.dataset.enabled === 'true')));
    document.querySelectorAll('[data-source-rebind]').forEach((button) => button.onclick = () => {
      const source = state.settings.sources.find((s) => s.id === button.dataset.sourceRebind);
      showEditor(AieyesI18n.t("重新绑定机器"), `<p>${AieyesI18n.t("保留原目录、账户关联和历史；重新绑定后从目标机器采集。")}</p><code>${e(source.path)}</code>` + select('machineId', AieyesI18n.t("目标机器"), [['', AieyesI18n.t("请选择机器")], ['local', AieyesI18n.t("本机")], ...state.settings.hosts.map((h) => [h.id, h.name || h.target])], ''), async (form) => {const machineId = form.get('machineId');if (!machineId) throw new Error(AieyesI18n.t("请选择目标机器"));accept(await api('sources.rebind', { sourceId: source.id, machineId, baseSource: source }));directorySelection = { provider: source.provider, machine: machineId };markSaved();backgroundRefresh();renderSettings(false);});
    });
    bindSettingsSidebar('data-settings-agent', (provider) => {navigate({ provider });renderSettings();});
    document.querySelectorAll('[data-agent-estimates]').forEach((button) => button.onclick = () => openProviderEstimates(button.dataset.agentEstimates));
    document.querySelectorAll('[data-agent-enable],[data-agent-machine]').forEach((input) => input.onchange = () => run(async () => {
      const provider = input.dataset.agentEnable || input.dataset.agentMachine,a = (input.dataset.agentEnable ? configurations() : rawConfigurations()).find((a) => a.provider === provider);
      const params = input.dataset.agentEnable ? { provider, group: true, enabled: input.checked, ...(input.checked && !a.machineIds.length ? { machineIds: ['local'] } : {}) } : { provider, enabled: true, machineIds: input.checked ? [...new Set([...a.machineIds, input.value])] : a.machineIds.filter((id) => id !== input.value) };
      state.settingsSaving = true;document.querySelectorAll('[data-agent-enable],[data-agent-machine]').forEach((i) => i.disabled = true);
      try {accept(await api('agents.set', params));markSaved();backgroundRefresh();} finally {state.settingsSaving = false;scheduleRefreshes();}
    }));
    document.querySelectorAll('[data-directory-shell]').forEach((b) => b.onclick = () => openSettings({ category: 'hosts', hostId: b.dataset.directoryShell, section: 'shell' }));
    document.querySelectorAll('[data-directory-account]').forEach((b) => b.onclick = () => openSettings({ accountKey: b.dataset.directoryAccount, machineId: b.dataset.machine, section: 'command' }));
    document.querySelectorAll('[data-directory-add-account]').forEach((b) => b.onclick = () => openSettings({ provider: b.dataset.directoryAddAccount, section: 'accounts' }));
    if ($('#directories-back')) $('#directories-back').onclick = () => {directorySelection = null;renderSettings(false);};
    document.querySelectorAll('[data-machine-directories]').forEach((b) => b.onclick = () => {directorySelection = { provider: b.dataset.machineDirectories, machine: b.dataset.machine };renderSettings(false);});
    document.querySelectorAll('[data-machine-account-refresh]').forEach((b) => b.onclick = () => refreshStatuses(true, b.dataset.machineAccountRefresh));
    document.querySelectorAll('[data-machine-estimates]').forEach((b) => b.onclick = () => window.AieyesAutomatic.open(machineSources(b.dataset.machineEstimates, b.dataset.machine).map((s) => s.id)));
    document.querySelectorAll('[data-source-remove]').forEach((b) => b.onclick = () => removeDirectory(state.settings.sources.find((s) => s.id === b.dataset.sourceRemove)));
    document.querySelectorAll('[data-source-advanced]').forEach((b) => b.onclick = () => advanced(state.settings.sources.find((s) => s.id === b.dataset.sourceAdvanced)));
    document.querySelectorAll('[data-source-add]').forEach((b) => b.onclick = () => advanced(null, b.dataset.sourceAdd));
    document.querySelectorAll('[data-agent-host]').forEach((b) => b.onclick = () => {state.settingsTab = 'hosts';renderSettings(false);editItem();});
    if ($('#accounts-back')) $('#accounts-back').onclick = () => {showCodex = false;statusLoaded = false;statusFullLoaded = false;statusError = '';renderSettings(false);};
    if ($('#account-detail-back')) $('#account-detail-back').onclick = () => {detailKey = '';renderSettings(false);};
    if ($('#account-status-refresh')) $('#account-status-refresh').onclick = () => refreshStatuses(true);
    document.querySelectorAll('[data-deployment-sync]').forEach((b) => b.onclick = syncDeployments);
    if (state.settingsTab === 'integrations' && !showCodex) void ensureStatuses();
    document.querySelectorAll('[data-account-add]').forEach((b) => b.onclick = () => addAccount(null, b.dataset.accountAdd));
    const account = (key) => state.settings.accounts.find((a) => a.provider + ':' + a.id === key);
    document.querySelectorAll('[data-account-edit]').forEach((b) => b.onclick = () => {detailKey = b.dataset.accountEdit;renderSettings(false);});
    document.querySelectorAll('[data-account-options]').forEach((b) => b.onclick = () => editAccount(account(b.dataset.accountOptions)));
    document.querySelectorAll('[data-account-delete]').forEach((b) => b.onclick = () => deleteAccount(account(b.dataset.accountDelete)));
    document.querySelectorAll('[data-account-field],[data-command-field]').forEach((input) => {
      const a = account(input.dataset.account),field = input.dataset.accountField || 'command:' + input.dataset.commandField,key = accountKey(a) + ':' + field;
      input.oninput = () => accountDrafts.set(key, input.value);
      input.onchange = () => queueAccountField(a, field, input.value);
    });
    document.querySelectorAll('[data-command-confirm]').forEach((b) => b.onclick = () => queueAccountField(account(b.dataset.account), 'command:' + b.dataset.commandConfirm, document.getElementById('account-command-' + b.dataset.commandConfirm).value));
    document.querySelectorAll('[data-account-retry]').forEach((b) => b.onclick = () => {const key = b.dataset.accountRetry,a = account(detailKey),field = key.slice(detailKey.length + 1);void queueAccountField(a, field, accountDrafts.get(key));});
    document.querySelectorAll('[data-account-archive]').forEach((b) => b.onchange = () => queueAccountField(account(b.dataset.accountArchive), 'archived', b.checked));
    document.querySelectorAll('[data-account-reset]').forEach((b) => b.onchange = () => queueAccountField(account(b.dataset.account), 'quotaResetNotifications.' + b.dataset.accountReset, b.value));
    document.querySelectorAll('[data-account-priority]').forEach((b) => b.onchange = () => queueAccountField(account(b.dataset.accountPriority), 'quotaSourceId', b.value || null));
    document.querySelectorAll('[data-account-quota]').forEach((b) => b.onchange = () => queueAccountField(account(b.dataset.accountQuota), 'quotaEnabled', b.checked));
    document.querySelectorAll('[data-account-login]').forEach((b) => b.onclick = async () => {if (!(await flush())) return;showCodex = true;window.AieyesCodexAuth.open(account(b.dataset.accountLogin).id, b.dataset.source, b.dataset.intent);renderSettings(false);});
    document.querySelectorAll('[data-device-manage]').forEach((b) => b.onclick = () => {showCodex = true;window.AieyesCodexAuth.open('', b.dataset.deviceManage, b.dataset.intent);renderSettings(false);});
    document.querySelectorAll('[data-account-connect]').forEach((b) => b.onclick = () => {const a = account(b.dataset.accountConnect);if (a.provider === 'codex') {showCodex = true;window.AieyesCodexAuth.open(a.id);renderSettings(false);} else addAccount(a);});
    document.querySelectorAll('[data-account-restore]').forEach((b) => b.onclick = () => run(async () => {const next = structuredClone(state.settings),a = next.accounts.find((a) => a.provider + ':' + a.id === b.dataset.accountRestore);a.archived = false;await commit(next);}));
    document.querySelectorAll('[data-account-panel]').forEach((i) => i.onchange = () => {const a = account(i.dataset.accountPanel),pref = AieyesUI.panelPreference(),choice = pref.providers[a.provider],keys = (choice?.mode === 'custom' ? choice.keys : AieyesUI.panelAccounts(state.settings, state.dashboard?.quotaOrder ?? [], pref)[a.provider] ?? []).filter((k) => k !== i.dataset.accountPanel);if (i.checked) keys.push(i.dataset.accountPanel);pref.providers[a.provider] = { mode: 'custom', keys };try {localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify(pref));renderSettings(false);} catch (error) {i.checked = !i.checked;notify(AieyesI18n.t("无法保存面板选择：") + error, 'error');}});
    document.querySelectorAll('[data-account-panel-manage]').forEach((b) => b.onclick = openPanelAccounts);
    if (showCodex) window.AieyesCodexAuth.bind();
  }
  async function flush() {
    document.activeElement?.blur();await saveQueue;
    for (const [key, value] of [...accountDrafts]) {const a = state.settings.accounts.find((a) => key.startsWith(accountKey(a) + ':'));if (a) await queueAccountField(a, key.slice(accountKey(a).length + 1), value);}
    if (accountErrors.size) {notify(AieyesI18n.t("账户设置尚未保存，请修正输入或重试。"), 'error');return false;}return true;
  }
  function closeMachineMenus(except = null, restore = false) {
    document.querySelectorAll('.machine-menu[open]').forEach((menu) => {
      if (menu === except) return;menu.open = false;menu.querySelector('summary')?.setAttribute('aria-expanded', 'false');
      if (restore) menu.querySelector('summary')?.focus({ preventScroll: true });
    });
  }
  function bindMachineMenus() {
    document.querySelectorAll('.machine-menu').forEach((menu) => {
      const trigger = menu.querySelector('summary'),body = menu.querySelector('.machine-menu-body');
      const position = () => {
        const rect = trigger.getBoundingClientRect(),above = rect.top - 12,below = innerHeight - rect.bottom - 12;
        const upward = below < Math.min(body.scrollHeight, 240) && above > below;
        body.style.top = upward ? 'auto' : '100%';body.style.bottom = upward ? '100%' : 'auto';
        body.style.maxHeight = Math.max(80, Math.min(420, upward ? above : below)) + 'px';
      };
      trigger.setAttribute('aria-haspopup', 'menu');trigger.setAttribute('aria-expanded', String(menu.open));body.setAttribute('role', 'menu');
      body.querySelectorAll('button').forEach((button) => {button.setAttribute('role', 'menuitem');button.tabIndex = -1;});
      trigger.onclick = (event) => {event.preventDefault();const open = !menu.open;closeMachineMenus();menu.open = open;trigger.setAttribute('aria-expanded', String(open));if (open) position();};
      menu.onkeydown = (event) => {
        if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault();closeMachineMenus(menu);menu.open = true;trigger.setAttribute('aria-expanded', 'true');position();
        const buttons = [...body.querySelectorAll('button:not(:disabled)')],index = buttons.indexOf(document.activeElement);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : index < 0 ? event.key === 'ArrowUp' ? buttons.length - 1 : 0 : (index + (event.key === 'ArrowUp' ? -1 : 1) + buttons.length) % buttons.length;
        buttons[next]?.focus({ preventScroll: true });
      };
    });
  }
  document.addEventListener('pointerdown', (event) => {closeMachineMenus(event.target.closest?.('.machine-menu'));}, true);
  document.addEventListener('click', (event) => {if (event.target.closest?.('.machine-menu-body button')) closeMachineMenus();}, true);
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && document.querySelector('.machine-menu[open]')) {event.preventDefault();event.stopImmediatePropagation();closeMachineMenus(null, true);} else
    if (event.key === 'Tab') closeMachineMenus();
  }, true);
  window.AieyesAgentSettings = { flush, navigate, integrationsHTML, agentsHTML, accountsHTML, bind, commit, accept, advanced, editAccount, addAccount, profiles, linked, deviceCounts, countsHTML, ensureStatuses, refreshStatuses, refreshCodexSource, refreshAntigravitySource, syncIdentitySettings, syncDeployments, updateSession, get statusLoaded() {return statusLoaded;}, get statusBusy() {return statusLoading;}, get antigravityRefreshing() {return antigravityRefreshTasks.size > 0;}, openDetails(a) {return openSettings({ provider: a.provider, accountKey: accountKey(a) });} };
  void window.AieyesApp.ready.then(() => ensureStatuses('codex'));
})();

// A separate modal keeps the connection form intact during the CLI's authorization flow.
window.AieyesAgyAuth = { busy: false };
document.addEventListener('click', (event) => {const button = event.target.closest('[data-agy-login]');if (button) openAntigravityLogin(button.dataset.agyLogin);});
async function openAntigravityLogin(sourceId) {
  if (document.querySelector('#agy-login')) return;
  const dialog = document.createElement('dialog');dialog.id = 'agy-login';dialog.setAttribute('aria-label', AieyesI18n.t("Antigravity 登录"));
  dialog.innerHTML = `<h2>${AieyesI18n.t("Antigravity 登录")}</h2><p>${AieyesI18n.t("读取机器现有登录，或通过浏览器授权。凭据保存在登录机器。")}</p><p id="agy-login-status" role="status"></p><p id="agy-login-identity"></p><p id="agy-login-metadata" class="muted"></p><div id="agy-auth-url" hidden><button type="button" id="agy-open-url">${AieyesI18n.t("打开授权网页")}</button><button type="button" id="agy-copy-url">${AieyesI18n.t("复制授权网址")}</button><label>${AieyesI18n.t("浏览器返回的授权码")}<input id="agy-code" type="password" autocomplete="off"></label><button type="button" id="agy-submit-code">${AieyesI18n.t("提交授权码")}</button></div><div id="agy-login-error" class="error"></div><div class="actions"><button id="agy-trust" hidden>${AieyesI18n.t("确认临时目录")}</button><button id="agy-verify">${AieyesI18n.t("验证登录")}</button><button id="agy-retry" hidden>${AieyesI18n.t("重新登录 / 检查")}</button><button id="agy-close">${AieyesI18n.t("完成")}</button></div>`;
  document.body.append(dialog);dialog.showModal();
  let session = null,sending = false,closed = false,refreshedSession = null;
  const active = () => !session || ['checking', 'starting', 'authorizing', 'verifying'].includes(session.phase);
  function draw() {AieyesAgyAuth.busy = active() || sending;dialog.querySelector('#agy-login-status').textContent = AieyesI18n.message(session?.message || AieyesI18n.t("正在检查现有登录…"));dialog.querySelector('#agy-login-identity').textContent = session?.identity ? [session.identity.email, session.identity.subscription || AieyesI18n.t("订阅未知")].join(' · ') : '';dialog.querySelector('#agy-login-metadata').textContent = AieyesI18n.message(session?.metadataError);dialog.querySelector('#agy-auth-url').hidden = !session?.authUrl;dialog.querySelector('#agy-trust').hidden = session?.workspaceConfirmationRequired !== true;dialog.querySelector('#agy-trust').disabled = sending;dialog.querySelector('#agy-submit-code').disabled = sending;dialog.querySelector('#agy-retry').disabled = sending;dialog.querySelector('#agy-retry').hidden = active();dialog.querySelector('#agy-verify').disabled = sending || !session || ['checking', 'verifying'].includes(session.phase);dialog.querySelector('#agy-close').disabled = sending;dialog.querySelector('#agy-close').textContent = active() ? AieyesI18n.t("取消授权") : AieyesI18n.t("完成");}
  async function call(method, extra = {}) {if (sending) return;sending = true;if (['start', 'verify'].includes(method)) refreshedSession = null;draw();try {session = await api('agyAuth.' + method, { sourceId, id: session?.id, ...extra });dialog.querySelector('#agy-login-error').textContent = '';draw();if (session?.authenticated && refreshedSession !== session.id) {await window.AieyesAgentSettings.refreshAntigravitySource(sourceId, session.accountStatus);refreshedSession = session.id;}} catch (error) {dialog.querySelector('#agy-login-error').textContent = AieyesI18n.message(String(error));} finally {sending = false;draw();}}
  async function close() {if (sending) return;if (active() && session) await call('cancel');closed = true;AieyesAgyAuth.busy = false;dialog.close();if (window.AieyesMotion) AieyesMotion.removeAfterExit(dialog);else dialog.remove();}
  dialog.oncancel = (e) => {e.preventDefault();close();};dialog.querySelector('#agy-close').onclick = close;
  dialog.querySelector('#agy-verify').onclick = () => call('verify');dialog.querySelector('#agy-retry').onclick = () => call('start');dialog.querySelector('#agy-trust').onclick = () => call('confirmWorkspace');
  dialog.querySelector('#agy-submit-code').onclick = () => {const input = dialog.querySelector('#agy-code'),code = input.value;input.value = '';call('submitCode', { code });};
  dialog.querySelector('#agy-open-url').onclick = () => call('openUrl');
  dialog.querySelector('#agy-copy-url').onclick = async () => {try {await navigator.clipboard.writeText(session.authUrl);} catch (error) {dialog.querySelector('#agy-login-error').textContent = AieyesI18n.t("复制失败：") + String(error);}};
  draw();await call('start');
  while (!closed) {await new Promise((resolve) => setTimeout(resolve, 1000));if (!closed && !sending && session && (active() || session.authenticated && refreshedSession !== session.id)) await call('status');}
}
