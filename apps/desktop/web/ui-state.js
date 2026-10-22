/* Shared UI semantics. Query timestamps deliberately never imply a successful sync. */
window.AieyesUI = {
  syncFailure(status) {
    if (status?.error) return AieyesI18n.message(status.error);
    if (!status?.partial) return null;
    const issues = status.issues ?? [],visible = issues.filter((i) => !['unknownModel', 'missingHistoryBase'].includes(i.code));
    if (issues.length && !visible.length) return null;
    return visible.map((i) => (i.path ? i.path + ' · ' : '') + (i.step != null ? AieyesI18n.t("步骤 ") + i.step + ' · ' : '') + AieyesI18n.message(i.message)).join('；') || AieyesI18n.t("部分记录需要处理");
  },
  errorPresentation(message, context = AieyesI18n.t("操作未完成")) {
    const original = AieyesI18n.message(String(message ?? '')).trim();
    let text = original;
    for (const [pattern, replacement] of [
    [/(["']?cookie["']?\s*[:=]\s*["']?)[^\r\n]+/gi, AieyesI18n.t("$1[已隐藏]")],
    [/(bearer\s+)[a-z0-9._~+/=-]+/gi, AieyesI18n.t("$1[已隐藏]")],
    [/((?:["']?)(?:api[_ -]?key|access[_-]?token|refresh[_-]?token|token|authorization|password)(?:["']?)\s*[:=]\s*["']?)[^\s"';,}&]+/gi, AieyesI18n.t("$1[已隐藏]")],
    [/(https?:\/\/)[^\s/@]+:[^\s/@]+@/g, AieyesI18n.t("$1[已隐藏]@")],
    [/\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{8,}/g, AieyesI18n.t("[已隐藏]")]])
    text = text.replace(pattern, replacement);
    const first = text.split(/\r?\n/)[0],sourceFirst = original.split(/\r?\n/)[0];
    const readable = (/\p{Script=Han}/u.test(String(message)) || original !== String(message).trim()) && !['{', '['].includes(sourceFirst[0]);
    const summary = Array.from(first).length <= 72 && readable ? first : context;
    return { summary, details: text && text !== summary ? text : '' };
  },
  errorHTML(message, context) {
    const { summary, details } = this.errorPresentation(message, context),e = this.escapeHTML;
    return `<div class="error-notice"><span>${e(summary)}</span>${details ? `<details class="error-details"><summary>${AieyesI18n.t("错误详情")}</summary><pre tabindex="0">${e(details)}</pre></details>` : ''}</div>`;
  },
  showError(element, message, context) {
    if (element) element.innerHTML = message ? this.errorHTML(message, context) : '';
  },
  shortPath(path, limit = 42) {
    const text = Array.from(String(path ?? ''));
    if (text.length <= limit) return text.join('');
    const head = Math.floor((limit - 1) / 3);
    return text.slice(0, head).join('') + '…' + text.slice(-(limit - head - 1)).join('');
  },
  canonicalProvider(provider) {return provider === 'agy' ? 'antigravity' : provider;},
  agentProvider(provider) {return window.AieyesContract.agentAliases?.[provider] ?? this.canonicalProvider(provider);},
  sameAgent(left, right) {return this.agentProvider(left) === this.agentProvider(right);},
  supports(provider, feature, settings) {
    if (window.AieyesContract.providerCapabilities?.[provider]?.includes(feature)) return true;
    return ['quota', 'quotaReset', 'valuation'].includes(feature) && (settings?.sources ?? []).some((source) => source.provider === provider && source.quotaCommand?.trim());
  },
  featureProvider(provider, feature) {
    return Object.keys(window.AieyesContract.providers).find((key) => this.sameAgent(key, provider) && this.supports(key, feature));
  },
  agentSupports(provider, feature, settings) {
    return Object.keys(window.AieyesContract.providers).some((key) => this.sameAgent(key, provider) && this.supports(key, feature, settings));
  },
  agentConfigurations(rows) {
    const groups = new Map();
    for (const row of rows) {
      const provider = this.agentProvider(row.provider),group = groups.get(provider) ?? { provider, enabled: false, machineIds: [] };
      group.enabled ||= row.enabled;
      if (this.supports(row.provider, 'machines')) group.machineIds = [...new Set([...group.machineIds, ...row.machineIds])];
      groups.set(provider, group);
    }
    return [...groups.values()];
  },
  overviewAgentProviders(settings) {
    const rows = this.agentConfigurations(settings?.agents ?? []);
    return window.AieyesContract.agentProviders.filter((provider) => rows.some((a) => a.provider === provider && a.enabled) ||
    !settings?.agents && ((settings?.sources ?? []).some((s) => this.sameAgent(s.provider, provider) && s.enabled) || (settings?.accounts ?? []).some((a) => this.sameAgent(a.provider, provider) && !a.archived)));
  },
  balancedColumns(width, count, minimum = 290, gap = 10) {
    const fit = Math.max(1, Math.min(count, Math.floor((width + gap) / (minimum + gap))));
    return Math.max(1, Math.ceil(count / Math.ceil(Math.max(1, count) / fit)));
  },
  canonicalAccountKey(key, aliases = {}) {return aliases[key] ?? (key?.startsWith('agy:') ? 'antigravity:' + key.slice(4) : key);},
  canonicalPayload(value, aliases = value?.accountAliases ?? {}) {
    if (Array.isArray(value)) return value.map((row) => this.canonicalPayload(row, aliases));
    if (!value || typeof value !== 'object') return value;
    const next = Object.fromEntries(Object.entries(value).map(([key, row]) => [key, this.canonicalPayload(row, aliases)]));
    if (next.provider === 'agy') {
      const field = 'accountId' in next ? 'accountId' : 'quotaEnabled' in next ? 'id' : null;
      if (field) next[field] = this.canonicalAccountKey('agy:' + next[field], aliases).slice('antigravity:'.length);
      next.provider = 'antigravity';
    }
    if (next.accountKey) next.accountKey = this.canonicalAccountKey(next.accountKey, aliases);
    if (next.quotaOrder) next.quotaOrder = next.quotaOrder.map((key) => this.canonicalAccountKey(key, aliases));
    return next;
  },
  escapeHTML(value) {return String(value ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);},
  normalizePanelPreference(value) {
    const next = { version: 2, providers: {} };
    const object = (row) => row && typeof row === 'object' && !Array.isArray(row);
    const keys = (row) => Array.isArray(row) && row.every((key) => typeof key === 'string');
    if (!object(value)) return next;
    if ('version' in value || 'providers' in value) {
      if (value.version !== 2 || !object(value.providers)) return next;
      for (const [provider, choice] of Object.entries(value.providers)) {
        next.providers[provider] = choice?.mode === 'custom' && keys(choice.keys) ?
        { mode: 'custom', keys: [...choice.keys] } : { mode: 'auto', keys: [] };
      }
    } else {
      for (const [provider, choice] of Object.entries(value)) if (keys(choice)) next.providers[provider] = { mode: 'custom', keys: [...choice] };
    }
    return next;
  },
  decodePanelPreference(raw) {
    try {return this.normalizePanelPreference(JSON.parse(raw));}
    catch (_) {return { version: 2, providers: {} };}
  },
  panelPreference() {
    try {
      const current = localStorage.getItem('aieyes.panel.accounts.v2');
      if (current !== null) return this.decodePanelPreference(current);
      const next = this.decodePanelPreference(localStorage.getItem('aieyes.panel.accounts.v1') || '{}');
      localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify(next));
      return next;
    } catch (_) {return { version: 2, providers: {} };}
  },
  panelAccounts(settings, order = [], preference = this.panelPreference()) {
    preference = this.normalizePanelPreference(preference);
    const selected = {},eligible = (settings?.accounts ?? []).filter((a) => !a.archived && a.quotaEnabled);
    const rank = (a) => {const i = order.indexOf(a.provider + ':' + a.id);return i < 0 ? Number.MAX_SAFE_INTEGER : i;};
    const sorted = [...eligible].sort((a, b) => rank(a) - rank(b));
    for (const provider of new Set([...eligible.map((a) => a.provider), ...Object.keys(preference.providers)])) {
      const keys = sorted.filter((a) => a.provider === provider).map((a) => a.provider + ':' + a.id),choice = preference.providers[provider];
      const saved = Array.isArray(choice?.keys) ? choice.keys : [];
      const automatic = sorted.filter((a) => this.sameAgent(a.provider, provider) && preference.providers[a.provider]?.mode !== 'custom').slice(0, 5).filter((a) => a.provider === provider).map((a) => a.provider + ':' + a.id);
      const values = choice?.mode === 'custom' ? saved.filter((k) => keys.includes(k)) : automatic;
      selected[provider] = [...new Set(values)];
    }
    return selected;
  },
  observePanelAccounts(settings, order = [], previous = null) {
    try {
      const accounts = settings?.accounts ?? [],key = (a) => a.provider + ':' + a.id;
      let seen;
      try {const value = JSON.parse(localStorage.getItem('aieyes.panel.accounts.known.v1'));if (Array.isArray(value) && value.every((k) => typeof k === 'string')) seen = value;} catch (_) {}
      const known = new Set((seen ?? (previous?.accounts ?? accounts).map(key)).map((k) => this.canonicalAccountKey(k, settings?.accountAliases)));
      const added = accounts.filter((a) => !known.has(key(a)) && !a.archived && a.quotaEnabled);
      if (added.length) {
        const pref = this.panelPreference(),selected = this.panelAccounts({ accounts: accounts.filter((a) => known.has(key(a))) }, order, pref);
        for (const a of added) {
          const choice = pref.providers[a.provider],keys = choice?.mode === 'custom' ? [...choice.keys] : [...(selected[a.provider] ?? [])];
          if (!keys.includes(key(a))) keys.push(key(a));
          pref.providers[a.provider] = { mode: 'custom', keys };
        }
        localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify(pref));
      }
      const next = [...new Set([...known, ...accounts.map(key)])].sort(),raw = JSON.stringify(next);
      if (localStorage.getItem('aieyes.panel.accounts.known.v1') !== raw) localStorage.setItem('aieyes.panel.accounts.known.v1', raw);
    } catch (_) {/* Local preferences must not block accepting saved accounts. */}
  },
  canonicalEstimates(records = []) {
    const replaced = new Set(records.map((e) => e.originalEstimateId).filter(Boolean));
    return records.filter((e) => !replaced.has(e.id) && !e.rollingExcludedReason).sort((a, b) => (b.samplingMode === 'rolling') - (a.samplingMode === 'rolling') || b.startedAt - a.startedAt || (b.repairedAt || 0) - (a.repairedAt || 0));
  },
  estimateIssue(record) {
    if (record.rollingExcludedReason) return record.rollingExcludedReason;
    if (record.status === 'pending') return record.reason || AieyesI18n.t("待确认");
    return { boundary: AieyesI18n.t("用量边界待确认"), noUsage: AieyesI18n.t("暂无有效用量"), unpriced: AieyesI18n.t("模型价格不完整"), insufficientUsage: AieyesI18n.t("等待可确认的消耗"), invalidPrice: AieyesI18n.t("价格数据无效") }[record.calculationStatus] || record.calculationNote || AieyesI18n.t("尚无估值");
  },
  estimateVisibility(quota, records = [], group = '') {
    const latest = this.canonicalEstimates(records)[0],scope = group || latest?.groupId || '';
    const windows = quota?.windows?.length ? quota.windows : latest?.checkpointWindows ?? [];
    const rows = scope ? windows.filter((w) => (w.groupId || w.groupName) === scope) : windows;
    const five = windows.length ? rows.some((w) => w.windowMinutes === window.AieyesContract.windows.fiveHour) : latest?.valuationMode === 'fiveHour';
    const provider = quota?.provider ?? latest?.accountKey?.split(':')[0];
    const plan = String(quota?.plan || latest?.quotaPlan || '').trim().toLowerCase();
    return { five, direct: !(provider === 'codex' && plan === 'plus'), ratio: five };
  },
  estimatePeriods(quota, records = [], group = '') {
    const visible = this.estimateVisibility(quota, records, group);
    return [visible.five ? '5h' : '', visible.direct || visible.ratio ? '7d' : ''].filter(Boolean).join(' / ');
  },
  estimateHelp(quota, records = [], group = '') {
    const visible = this.estimateVisibility(quota, records, group);
    if (this.canonicalEstimates(records)[0]?.samplingMode === 'rolling') return AieyesI18n.t("滚动估值：汇总最近 90 天的有效片段，权重每 7 天减半，按额度消耗量加权。") + (visible.five ? AieyesI18n.t("5h 按对应额度片段计算。") : '') + (visible.direct ? AieyesI18n.t("7d 按对应周额度片段计算。") : '') + (visible.ratio ? AieyesI18n.t("7d 容量推算为滚动 5h 价值乘近期容量倍率。") : '') + AieyesI18n.t("金额为 API 等价价值，不可兑换。");
    if (visible.ratio && visible.direct) return AieyesI18n.t("7d 左值：5h 等价值 × 近期容量倍率；右值：同一采样时段的 API 等价成本 ÷ 周额度消耗比例。旧记录显示整周估值；这些金额为 API 等价价值。");
    if (visible.ratio) return AieyesI18n.t("7d：5h 等价值 × 近期容量倍率；这些金额为 API 等价价值。");
    if (visible.direct) return AieyesI18n.t("7d：同一采样时段的 API 等价成本 ÷ 周额度消耗比例。旧记录显示整周估值；这些金额为 API 等价价值。");
    return AieyesI18n.t("按采样时段的 API 等价成本与对应额度消耗比例计算。");
  },
  estimateEntries(records = [], credits = false, quota = null, group = '') {
    const canonical = this.canonicalEstimates(records),rolling = canonical.filter((e) => e.samplingMode === 'rolling');
    const all = rolling.length ? rolling : canonical;
    const visible = this.estimateVisibility(quota, records, group);
    return (credits ? [['valuePer1000', '1000 credits']] : [
    ...(visible.five ? [['fiveHourValue', '5h']] : []),
    ...(visible.direct ? [['weeklyValue', AieyesI18n.t("7d 整周")]] : []),
    ...(visible.ratio ? [['weeklyRatioValue', AieyesI18n.t("7d 容量倍率")]] : [])]).
    flatMap(([field, label]) => {
      const record = all.find((e) => e.status !== 'pending' && Number.isFinite(e[field]));
      return record ? [{ field, label: field === 'weeklyValue' && record.valuationMode === 'fiveHour' ? AieyesI18n.t("7d 同期") : label, value: record[field], record, historical: record !== all[0] || record.status === 'completed' }] : [];
    });
  },
  sourceEnabled(settings, source) {
    const agents = (settings?.agents ?? []).filter((agent) => agent.provider === source.provider);
    return source.enabled && source.machineBinding !== 'unbound' && (source.hostId != null || settings?.localDeviceEnabled !== false) && agents.length <= 1 && (
    !agents.length || agents[0].enabled && agents[0].machineIds.includes(source.hostId ?? 'local')) && (
    source.hostId == null || (settings?.hosts ?? []).some((host) => host.id === source.hostId && host.enabled));
  },
  usageSources(settings) {
    return (settings?.sources ?? []).filter((source) => source.provider !== 'deepseek' && this.sourceEnabled(settings, source));
  },
  estimateSources(settings, quota) {
    const account = (settings?.accounts ?? []).find((a) => a.provider === quota.provider && a.id === quota.accountId);
    if (!account) return [];
    const profiles = [account.quotaProfileId, ...(account.connections ?? []).map((c) => c.profileId)].filter(Boolean);
    return this.usageSources(settings).filter((s) => s.provider === account.provider && (
    s.accountId === account.id || s.codexHomeId && profiles.some((r) => r.startsWith(s.codexHomeId + ':'))));
  },
  freshness(settings, dashboard, now = Date.now() / 1000) {
    const sources = this.usageSources(settings);
    const statuses = sources.map((s) => dashboard?.sources?.find((row) => row.id === s.id)?.status);
    const stamps = statuses.map((s) => s?.updatedAt ?? 0);
    return {
      total: sources.length,
      // The oldest source is the only defensible timestamp for the entire record set.
      timestamp: stamps.length && stamps.every(Boolean) ? Math.min(...stamps) : 0,
      delayed: statuses.filter((s) => !s?.updatedAt || now - s.updatedAt > Math.max(60, (settings.historyRefreshSeconds ?? settings.refreshSeconds) * 2)).length,
      failed: statuses.filter((s) => this.syncFailure(s)).length
    };
  },
  resetDisplayTime(stamp, now = Date.now() / 1000) {
    // A zero usage percentage does not establish whether a window has started.
    // Only the source timestamp can establish its next reset.
    return Number.isFinite(stamp) && stamp > 0 && stamp > now ? stamp : null;
  },
  resetText(stamp, now = Date.now() / 1000) {
    const target = this.resetDisplayTime(stamp, now);
    if (target == null) return Number.isFinite(stamp) && stamp > 0 ? AieyesI18n.t("确认重置中") : AieyesI18n.t("重置时间未知");
    const minutes = Math.ceil((target - now) / 60),parts = [];
    if (!Number.isSafeInteger(minutes) || minutes <= 0) return AieyesI18n.t("重置时间未知");
    if (minutes >= 1440) parts.push(AieyesI18n.t("{0} 天", Math.floor(minutes / 1440)));
    if (minutes % 1440 >= 60) parts.push(Math.floor(minutes % 1440 / 60) + AieyesI18n.t(" 小时"));
    if (minutes < 1440 && minutes % 60) parts.push(minutes % 60 + AieyesI18n.t(" 分"));
    return parts.join(' ') + AieyesI18n.t("后重置");
  },
  quotaReset(quotaWindow, now = Date.now() / 1000) {
    if (!(Number.isFinite(quotaWindow.resetsAt) && quotaWindow.resetsAt > 0) && quotaWindow.usedPercent === 0) {
      if (quotaWindow.windowMinutes === window.AieyesContract.windows.fiveHour) return '5h';
      if (quotaWindow.windowMinutes === window.AieyesContract.windows.week) return '7d';
    }
    const text = this.resetText(quotaWindow.resetsAt, now);
    const stamp = this.resetDisplayTime(quotaWindow.resetsAt, now),date = new Date(stamp * 1000);
    if (stamp == null || !Number.isFinite(date.getTime())) return text;
    const pad = (n) => String(n).padStart(2, '0');
    return text + ' · ' + pad(date.getMonth() + 1) + '/' + pad(date.getDate()) + ' ' + pad(date.getHours()) + ':' + pad(date.getMinutes());
  },
  subscription(plan) {
    const name = String(plan ?? '').trim();
    return { plus: 'Plus', pro: 'Pro', free: 'Free', max: 'Max', team: 'Team', business: 'Business', enterprise: 'Enterprise', api: 'API' }[name.toLowerCase()] ?? (name ? name[0].toLocaleUpperCase() + name.slice(1) : '');
  },
  compactMoney(value) {
    if (!Number.isFinite(value)) return '—';
    for (const [size, suffix] of [[1e12, 'T'], [1e9, 'B'], [1e6, 'M'], [1e3, 'K']]) if (Math.abs(value) >= size * 0.999995) return '$' + (value / size).toFixed(2) + suffix;
    return '$' + value.toFixed(2);
  },
  estimateWindows(q) {
    const windows = q.windows ?? [];
    if (q.provider === 'antigravity') {
      const groups = [...new Set(windows.map((w) => w.groupId || w.groupName || ''))];
      return groups.filter((g) => ['gemini', 'gemini models', 'claude-gpt', 'claude and gpt models'].includes(g.toLowerCase())).flatMap((g) => {
        const rows = windows.filter((w) => (w.groupId || w.groupName) === g);
        const five = rows.filter((w) => w.windowMinutes === window.AieyesContract.windows.fiveHour),week = rows.filter((w) => w.windowMinutes === window.AieyesContract.windows.week);
        return five.length === 1 ? five : !five.length && week.length === 1 ? week : [];
      });
    }
    const five = windows.filter((w) => w.windowMinutes === window.AieyesContract.windows.fiveHour),week = windows.filter((w) => w.windowMinutes === window.AieyesContract.windows.week);
    return five.length === 1 ? five : q.provider === 'claude' && five.some((w) => w.id === 'five_hour') ? five.filter((w) => w.id === 'five_hour') : week.length === 1 ? week : week.filter((w) => q.provider === 'claude' && w.id === 'seven_day');
  },
  estimateLine(records = [], quota = null, group = '') {
    const entries = this.estimateEntries(records, false, quota, group),value = (field) => {const entry = entries.find((e) => e.field === field);return entry ? this.compactMoney(entry.value) : null;};
    const five = value('fiveHourValue'),week = [value('weeklyRatioValue'), value('weeklyValue')].filter((v) => v !== null);
    return [five !== null ? '5h ≈ ' + five : '', week.length ? '7d ≈ ' + week.join('/') : ''].filter(Boolean).join('  ') || AieyesI18n.t("额度估值");
  },
  creditEstimateLine(records = []) {
    const entry = this.estimateEntries(records, true)[0];
    return entry ? AieyesI18n.t("credits 估值（1000 credits ≈ ") + this.compactMoney(entry.value) + '）' : AieyesI18n.t("credits 估值");
  },
  migrateProviderPreferences(settings) {
    const aliases = settings?.accountAliases ?? {},key = (s) => aliases[s] ?? (s.startsWith('agy:') ? 'antigravity:' + s.slice(4) : s);
    try {
      const marker = JSON.stringify(aliases);if (localStorage.getItem('aieyes.antigravity.preferences') === marker) return;
      const pref = this.panelPreference(),old = pref.providers.agy;
      if (old) {const current = pref.providers.antigravity;pref.providers.antigravity = current ? { mode: current.mode === 'custom' || old.mode === 'custom' ? 'custom' : 'auto', keys: [...new Set([...(current.keys ?? []), ...(old.keys ?? [])].map(key))] } : { ...old, keys: (old.keys ?? []).map(key) };delete pref.providers.agy;localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify(pref));}
      for (const name of Object.keys(localStorage)) {if (name.startsWith('quota.expanded.v2.') && name.includes('.agy:')) {const i = name.indexOf('.agy:') + 1,next = name.slice(0, i) + key(name.slice(i));if (localStorage.getItem(next) == null) localStorage.setItem(next, localStorage.getItem(name));}}
      const view = JSON.parse(localStorage.getItem('aieyes.panel.view') || '{}');if (view.provider === 'agy') view.provider = 'antigravity';if (view.accountKey) view.accountKey = key(view.accountKey);localStorage.setItem('aieyes.panel.view', JSON.stringify(view));
      localStorage.setItem('aieyes.antigravity.preferences', marker);
    } catch (_) {}
  },
  nextTargetTime(time, zone, now = new Date()) {
    zone = zone?.trim();
    if (!zone) return { status: 'pending', date: null, approximate: false };
    if (typeof time !== 'string' || time.length !== 5 || !/^([01]\d|2[0-3]):[0-5]\d$/.test(time)) return { status: 'invalidTime', date: null, approximate: false };
    try {
      let parts,approximate = false;
      if (zone.includes('/') || zone === 'UTC' || zone === 'GMT') {
        const formatter = new Intl.DateTimeFormat('en-GB', { timeZone: zone, hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
        parts = (date) => Object.fromEntries(formatter.formatToParts(date).filter((p) => p.type !== 'literal').map((p) => [p.type, Number(p.value)]));
      } else {
        const offset = zone.match(/(?:^|\s)(?:UTC|GMT)?([+-])(\d{2}):?(\d{2})$/);
        if (!offset || Number(offset[3]) >= 60 || Number(offset[2]) * 60 + Number(offset[3]) > 18 * 60) return { status: 'invalidZone', date: null, approximate: false };
        const minutes = (Number(offset[2]) * 60 + Number(offset[3])) * (offset[1] === '-' ? -1 : 1);
        parts = (date) => {const local = new Date(date.getTime() + minutes * 60_000);return { hour: local.getUTCHours(), minute: local.getUTCMinutes() };};
        approximate = true;
      }
      const [hour, minute] = time.split(':').map(Number),start = Math.floor(now.getTime() / 60_000) * 60_000;
      if (!Number.isFinite(start)) return { status: 'unavailable', date: null, approximate: false };
      for (let i = 1; i <= 2880; i++) {
        const date = new Date(start + i * 60_000),actual = parts(date);
        if (actual.hour === hour && actual.minute === minute) return { status: 'ready', date, approximate };
      }
      return { status: 'unavailable', date: null, approximate: false };
    } catch (_) {return { status: 'invalidZone', date: null, approximate: false };}
  },
  targetTimeLocal(time, zone, now = new Date()) {
    const next = this.nextTargetTime(time, zone, now);
    if (next.date) return AieyesI18n.t("本地 ") + next.date.toLocaleString(undefined, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }) + (next.approximate ? AieyesI18n.t("（按当前偏移估算）") : '');
    return { pending: AieyesI18n.t("目标时区待读取"), invalidTime: AieyesI18n.t("请输入 HH:mm") }[next.status] ?? AieyesI18n.t("时区换算待确认，以部署状态为准");
  },
  formValues(form) {
    return Object.fromEntries([...form.querySelectorAll('input[name],select[name],textarea[name]')].map((el) =>
    [el.name, el.type === 'checkbox' ? el.checked : el.value]));
  },
  restoreForm(form, values) {
    for (const [name, value] of Object.entries(values ?? {})) {
      const el = form.elements.namedItem(name);
      if (!el) continue;
      if (el.type === 'checkbox') el.checked = value;else if (el.value !== String(value)) el.value = value;
    }
  }
};
