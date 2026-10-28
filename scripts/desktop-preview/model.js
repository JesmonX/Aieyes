/* Stateful synthetic backend, shared by both platform previews. No native I/O. */
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.AieyesPreviewModel = factory();
})(globalThis, () => {
  const copy = value => value === undefined ? undefined : structuredClone(value);
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  const zero = () => ({ tokens: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, reasoning: 0 }, total: 0, cost: 0, pricedTokens: 0, events: 0 });
  function add(a, b) {for (const k of Object.keys(a.tokens)) a.tokens[k] += b.tokens[k] || 0;for (const k of ['total', 'cost', 'pricedTokens', 'events']) a[k] += b[k] || 0;return a;}
  function scale(a, factor) {const result = zero();for (const k of Object.keys(result.tokens)) result.tokens[k] = Math.round((a.tokens[k] || 0) * factor);for (const k of ['total', 'pricedTokens', 'events']) result[k] = Math.round((a[k] || 0) * factor);result.cost = (a.cost || 0) * factor;return result;}
  function redact(value) {
    if (Array.isArray(value)) return value.map(redact);
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, /^(apiKey|password|secret|accessToken|refreshToken|authorization|cookie)$/i.test(k) ? '[hidden]' : redact(v)]));
    if (typeof value === 'string') return value.replace(/(https?:\/\/)[^\s/@]+:[^\s/@]+@/g, '$1[hidden]@').replace(/\bsk-(?:proj-|ant-)?[\w-]{8,}/g, '[hidden]');
    return value;
  }
  class Model {
    constructor(data, methods, providers, hooks = {}) {
      this.data = copy(data);this.methods = Object.fromEntries(methods.map(m => [m.name, m]));this.providers = providers;this.hooks = hooks;
      this.revisions = Object.fromEntries(['settings', 'usage', 'quotas', 'estimates', 'prices', 'sources', 'accounts'].map(k => [k, 1]));
      this.listeners = new Map();this.monitorVersion = 0;this.delay = 0;this.fail = 'none';
      this.desktop = { platform: data.platform, page: 'agent', panelPage: 'agent', mode: 'pet', effectiveMode: 'pet', nativePet: true, petEnabled: false, showCount: true, hidden: false, panelOpen: true, panelPinned: false, petScale: 1, ...copy(data.desktop), material: 'opaque', materials: { main: 'opaque', floating: 'opaque' }, sessions: copy(data.sessions), activeCount: data.sessions.filter(s => s.phase !== 'complete').length, summary: '预览会话 · 模拟数据' };
      this.updates = { currentVersion: data.version, phase: 'idle', automatic: true, prompt: false, message: '预览模式：更新查询和安装不会访问网络。' };
    }
    listen(name, callback) {const set = this.listeners.get(name) || new Set();set.add(callback);this.listeners.set(name, set);return () => {set.delete(callback);if (!set.size) this.listeners.delete(name);};}
    emit(name, payload) {for (const callback of [...(this.listeners.get(name) || [])]) {try {callback({ event: name, payload: copy(payload) });} catch (error) {this.hooks.log?.({ method: `event:${name}`, startedAt: Date.now(), error: String(error), duration: 0 });}}}
    persist() {this.hooks.save?.(copy(this.data));}
    boundary(name) {throw new Error(`预览模式：${name}需要真实系统或外部服务，此处不会执行。 / Preview: external operation unavailable.`);}
    statuses() {return this.data.settings.accounts.flatMap(a => this.data.settings.sources.filter(s => s.provider === a.provider && s.accountId === a.id).map(s => ({ accountKey: `${a.provider}:${a.id}`, sourceId: s.id, machineId: s.hostId || 'local', current: true, credential: true, checkedAt: this.data.stamp, error: this.data.scenario === 'failure' ? '模拟连接失败' : null })));}
    targets() {return this.statuses().map(row => {const s = this.data.settings.sources.find(s => s.id === row.sourceId),a = this.data.settings.accounts.find(a => `${a.provider}:${a.id}` === row.accountKey);return { ...row, accountName: a.name, provider: a.provider, sourceName: s.name, path: s.path, machineName: s.hostId ? this.data.settings.hosts.find(h => h.id === s.hostId)?.name || s.hostId : '本机（模拟）', bound: true, canDraft: !a.archived, canWake: !a.archived, canSwitch: false, profileId: s.codexHomeId ? `${s.id}:preview` : null };});}
    hostRows() {return this.data.settings.hosts.map((h, i) => {
      const old = this.data.hosts.find(row => row.id === h.id),sample = old?.sample || { timestamp: this.data.stamp, uptime: 90061, load: [0.42], errors: {}, cpu: [{ id: 'cpu', utilization: 20 + i * 7 }], memory: { total: 32 * 1024 ** 3, available: 24 * 1024 ** 3 }, gpu: [], network: [] };
      return { ...copy(old), id: h.id, name: h.name, sample: { ...copy(sample), timestamp: this.data.scenario === 'failure' ? sample.timestamp : Date.now() / 1000 }, lastAttemptAt: Date.now() / 1000, sampleSession: 'preview', sampleVersion: this.monitorVersion, ...(this.data.scenario === 'failure' ? { error: '模拟 SSH 连接失败' } : {}) };
    });}
    monitor() {this.monitorVersion++;return { rows: this.hostRows(), inFlight: [], pending: [], sampleSession: 'preview', version: this.monitorVersion };}
    dashboard(params = {}) {
      const d = copy(this.data.dashboard),settings = this.data.settings;
      const sources = settings.sources.filter(s => (!params.provider || s.provider === params.provider) && (!params.sourceId || s.id === params.sourceId));
      const models = [this.data.prices[0]?.id || 'test-model', 'preview/fast-model'];
      const summary = zero(),byModel = new Map(),days = [],trendDays = [],heatmap = [],dayModels = [];
      const stamp = new Date(this.data.stamp * 1000),range = params.dateRange;
      const keyOf = day => `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`;
      const shifted = offset => {const day = new Date(stamp);day.setHours(12, 0, 0, 0);day.setDate(day.getDate() + offset);return keyOf(day);};
      const start = range?.kind === 'custom' ? range.start : shifted(range?.kind === 'all' ? -364 : -(params.days || 1) + 1);
      const end = range?.kind === 'custom' ? range.end : keyOf(stamp);
      const explicit = range?.kind === 'custom' || range?.kind === 'all';
      const trendStart = explicit ? start : [start, shifted(-6)].sort()[0];
      const heatStart = explicit ? start : shifted(-364);
      const earliest = [start, trendStart, heatStart].sort()[0];
      const cursor = new Date(`${earliest}T12:00:00`);
      for (let i = 0; keyOf(cursor) <= end && i < 3660; i++, cursor.setDate(cursor.getDate() + 1)) {
        const key = keyOf(cursor),inHistory = key >= shifted(-364) && key <= keyOf(stamp),total = zero();
        const historyIndex = (Date.parse(key) - Date.parse(shifted(-364))) / 86400000;
        for (const [j, model] of models.entries()) {
          if (params.model && model !== params.model) continue;
          const value = zero();
          if (inHistory) sources.forEach(s => add(value, scale(this.data.dashboard.summary, (settings.sources.indexOf(s) + 1) * (2 + historyIndex % 7) / (7 * (j + 1)))));
          add(total, value);
          if (key >= start) {add(summary, value);byModel.set(model, add(byModel.get(model) || zero(), value));}
          if (sources.length && inHistory && key >= trendStart) dayModels.push({ day: key, model, usage: value });
        }
        if (key >= start) days.push({ ...copy(total), key });
        if (key >= trendStart) trendDays.push({ ...copy(total), key });
        if (key >= heatStart) heatmap.push({ ...copy(total), key });
      }
      const accountMatches = q => (!params.provider || q.provider === params.provider) && (!params.sourceId || sources.some(s => s.accountId === q.accountId && s.provider === q.provider));
      d.quotas = settings.accounts.filter(a => !a.archived && a.quotaEnabled).flatMap(a => {
        const q = this.data.dashboard.quotas.find(q => q.accountId === a.id && q.provider === a.provider);
        return q && accountMatches(q) ? [{ ...q, name: a.name }] : [];
      });
      const order = d.quotaOrder || [];d.quotas.sort((a, b) => {const rank = q => {const i = order.indexOf(`${q.provider}:${q.accountId}`);return i < 0 ? 10000 : i;};return rank(a) - rank(b);});d.quotaOrder = d.quotas.map(q => `${q.provider}:${q.accountId}`);
      d.quotaEstimates = (d.quotaEstimates || []).filter(e => d.quotaOrder.includes(e.accountKey));d.creditEstimates = (d.creditEstimates || []).filter(e => d.quotaOrder.includes(e.accountKey));
      return { ...d, generatedAt: this.data.stamp, summary, models: [...byModel].filter(([, v]) => v.events).map(([key, v]) => ({ ...v, key })), modelOptions: models, trendDays, heatmap, dayModels, days, sources: sources.map(s => ({ ...s, status: { updatedAt: this.data.stamp, error: this.data.scenario === 'failure' ? '模拟采集失败' : null } })) };
    }
    validateSettings(next) {
      for (const key of ['accounts', 'sources', 'hosts']) if (!Array.isArray(next[key])) throw new Error(`模拟配置缺少 ${key}`);
      if (next.refreshSeconds < 10 || next.refreshSeconds > 86400) throw new Error('Agent 刷新间隔范围为 10–86400 秒');
      if (next.serverRefreshSeconds < 1) throw new Error('服务器刷新间隔必须大于零');
      for (const key of ['accounts', 'sources', 'hosts']) if (new Set(next[key].map(row => row.id)).size !== next[key].length) throw new Error('模拟配置包含重复 ID');
    }
    engine(method, p = {}) {
      const d = this.data,s = d.settings,now = Date.now() / 1000;
      switch (method) {
        case 'hello': return { version: `${d.version}-preview`, runtimeInvalidations: true, hostMonitoring: true };
        case 'methods.manifest': return Object.values(this.methods);
        case 'runtime.revisions': return copy(this.revisions);
        case 'settings.get': return s;
        case 'settings.patch': case 'settings.save': {
          const next = copy(p.settings);this.validateSettings(next);
          if (p.base) for (const key of Object.keys(next)) if (!same(next[key], p.base[key]) && !same(s[key], p.base[key])) throw new Error('配置已变化，请刷新后重试');
          d.settings = { ...s, ...Object.fromEntries(Object.entries(next).filter(([key, value]) => !p.base || !same(value, p.base[key]))) };return d.settings;
        }
        case 'dashboard': case 'dashboard.status': return this.dashboard(p);
        case 'dashboard.summary': return this.dashboard({ dateRange: { kind: 'all' } });
        case 'sources.defaultPath': return { path: d.platform === 'windows' && !p.hostId ? `C:\\Users\\Preview\\.${p.provider}` : `/home/preview/.${p.provider}` };
        case 'sources.configure': {
          const i = s.sources.findIndex(v => v.id === p.source.id);if (i < 0) s.sources.push(copy(p.source));else s.sources[i] = copy(p.source);return s;
        }
        case 'sources.remove': s.sources = s.sources.filter(v => v.id !== p.sourceId);return s;
        case 'sources.rebind': {const source = s.sources.find(v => v.id === p.sourceId);if (!source) throw new Error('目录不存在');source.hostId = p.machineId === 'local' ? null : p.machineId;return s;}
        case 'sources.scan': case 'sources.rescan': return s.sources.filter(v => !p.sourceId || v.id === p.sourceId).map(v => ({ id: v.id, events: 14, updatedAt: now, error: d.scenario === 'failure' ? '模拟采集失败' : null }));
        case 'agents.manage': {
          const ids = s.appearance.settingsAgentIds ?? [...new Set((s.agents || []).filter(a => a.enabled || a.machineIds.length).map(a => a.provider).concat(s.sources.map(a => a.provider),s.accounts.map(a => a.provider)))];
          s.appearance.settingsAgentIds = ids.filter(id => id !== p.provider);
          if (p.visible) s.appearance.settingsAgentIds.push(p.provider);
          for (const a of s.agents || []) if (a.provider === p.provider) a.enabled = p.visible && a.machineIds.length > 0;
          return s;
        }
        case 'agents.set': {
          let agent = (s.agents ||= []).find(a => a.provider === p.provider);
          if (!agent) {agent = { provider: p.provider, enabled: false, machineIds: [] };s.agents.push(agent);}
          if (p.enabled !== undefined) agent.enabled = p.enabled;if (p.machineIds) agent.machineIds = copy(p.machineIds);
          for (const machineId of agent.machineIds) if (!s.sources.some(v => v.provider === p.provider && (v.hostId || 'local') === machineId)) s.sources.push({ id: crypto.randomUUID(), provider: p.provider, name: `${p.provider} · 模拟目录`, enabled: true, accountId: '', hostId: machineId === 'local' ? null : machineId, path: this.engine('sources.defaultPath', { provider: p.provider, hostId: machineId === 'local' ? null : machineId }).path });
          return s;
        }
        case 'accounts.status.get': case 'accounts.status.refreshBatch': return this.statuses().filter(r => (!p.accountKeys || p.accountKeys.includes(r.accountKey)) && (!p.sourceIds || p.sourceIds.includes(r.sourceId)));
        case 'accounts.status.refresh': return this.statuses().find(r => r.accountKey === p.accountKey && r.sourceId === p.sourceId) || { accountKey: p.accountKey, sourceId: p.sourceId, current: false, credential: false };
        case 'accounts.targets': return this.targets().filter(r => !p.accountKey || r.accountKey === p.accountKey);
        case 'accounts.cleanup.list': return [];
        case 'accounts.deletion.preview': return { tasks: [] };
        case 'accounts.delete': s.accounts = s.accounts.filter(a => `${a.provider}:${a.id}` !== p.accountKey);for (const source of s.sources) if (`${source.provider}:${source.accountId}` === p.accountKey) source.accountId = '';return { deleted: true, settings: s, failedTasks: [] };
        case 'accounts.connect': case 'accounts.create': case 'subscriptions.connect': {
          const id = p.accountId || crypto.randomUUID();let account = s.accounts.find(a => a.id === id && a.provider === p.provider);
          if (!account) {account = { id, provider: p.provider, name: p.name || `${p.provider} 预览账户`, quotaEnabled: true, archived: false };s.accounts.push(account);}
          if (method === 'subscriptions.connect') {
            const source = s.sources.find(v => v.provider === p.provider && v.accountId === id && v.subscription);
            if (source) source.subscription = copy(p.subscription);
            else s.sources.push({ id: crypto.randomUUID(), provider: p.provider, name: account.name, accountId: id, enabled: true, path: '', subscription: copy(p.subscription) });
          }
          else for (const source of s.sources.filter(v => p.sourceIds?.includes(v.id))) source.accountId = id;
          return { settings: s, account };
        }
        case 'accounts.scan': return { results: [{ provider: p.provider || 'codex', machineId: p.machineId || 'local', status: 'skipped', message: '预览模式：未扫描本机或远程凭据。' }], addedAccounts: 0, addedConnections: 0, settings: s };
        case 'accounts.deployments.sync': return { settings: s, failedTasks: [] };
        case 'credentials.save': return { path: `preview-credentials/${p.sourceId}.key` };
        case 'hosts.credentials.save': return { passwordRef: `preview-${crypto.randomUUID()}` };
        case 'hosts.credentials.delete': return null;
        case 'hosts.sample': return this.hostRows().filter(v => !p.hostId || v.id === p.hostId);
        case 'hosts.monitor.configure': case 'hosts.monitor.refresh': return this.monitor();
        case 'hosts.discover': return { cpu: [{ id: 'cpu' }, { id: 'cpu0' }, { id: 'cpu1' }], memory: [{ id: 'memory' }], gpu: [{ id: '0', name: 'Preview GPU' }], network: [{ id: 'eth0' }], filesystems: [{ id: '/', type: 'ext4' }, { id: '/data', type: 'ext4' }], disk: [{ id: 'nvme0n1' }], errors: {} };
        case 'hosts.sshConfig.read': return { hosts: [{ alias: 'preview-lab', hostName: 'lab.example.invalid', user: 'preview', port: 22 }, { alias: 'preview-build', hostName: 'build.example.invalid', user: 'preview', port: 22 }], warnings: ['预览模式：这些 SSH 别名为合成数据。'] };
        case 'network.status': return d.network;
        case 'network.test': return { ...d.network, mode: p.proxy?.mode || s.proxy.mode, testedAt: now };
        case 'sessions.list': return { sessions: d.sessions, sources: [], unavailable: d.scenario === 'failure' };
        case 'sessions.hooks.status': return s.sources.map(v => ({ sourceId: v.id, state: 'installed' }));
        case 'quotas.schedule': return { nextDueAt: now + 300 };
        case 'quotas.refresh': {
          for (const a of s.accounts.filter(a => !a.archived && a.quotaEnabled && (!p.accountId || a.id === p.accountId))) {
            let q = d.dashboard.quotas.find(q => q.accountId === a.id && q.provider === a.provider);
            const source = s.sources.find(v => v.accountId === a.id && v.provider === a.provider);
            if (!q) {
              q = { accountId: a.id, provider: a.provider, sourceId: source?.id || '', name: a.name, windows: [], origin: 'live' };
              if (a.provider === 'deepseek') Object.assign(q, { balances: [{ currency: 'CNY', total: '230.56', granted: '30.00', toppedUp: '200.56' }], isAvailable: true });
              else q.windows = [{ id: `${a.provider}:primary`, name: '5h', windowMinutes: 300, usedPercent: 35, resetsAt: now + 18000 }];
              if (!source) q.error = '模拟账户尚未关联目录';d.dashboard.quotas.push(q);
            }
            q.updatedAt = now;
          }
          return this.dashboard(p).quotas.filter(q => !p.accountId || q.accountId === p.accountId);
        }
        case 'quotas.order.set': d.dashboard.quotaOrder = copy(p.keys || []);return d.dashboard.quotaOrder;
        case 'prices.list': return d.prices;
        case 'prices.save': for (const price of p.prices) {if (!price.id) throw new Error('请输入模型 ID');const index = d.prices.findIndex(v => v.id === price.id);if (index < 0) d.prices.push(copy(price));else d.prices[index] = copy(price);}return { saved: p.prices.length, repriced: 0 };
        case 'prices.reference': return { settings: s, matched: 0, unmatched: [], skipped: d.prices.length, message: '预览价格（无网络查询）' };
        case 'prices.sync': case 'prices.recalculate': return { updated: d.prices.length };
        case 'wakeups.list': return d.wakeups;
        case 'wakeups.save': {const task = { ...copy(p.task), id: p.task.id || crypto.randomUUID() },row = d.wakeups.find(r => r.task.id === task.id);if (row) {row.task = task;if (row.deployment) row.deployment.changed = true;}else d.wakeups.push({ task, deployment: null });return { task };}
        case 'wakeups.delete': d.wakeups = d.wakeups.filter(r => r.task.id !== p.id);return null;
        case 'wakeups.status': return { installed: Boolean(d.wakeups.find(r => r.task.id === p.id)?.deployment), timezone: 'Asia/Shanghai', nextRunAt: now + 3600, history: [] };
        case 'wakeups.history': return [];
        case 'wakeups.probe': return { models: d.prices.map(v => ({ id: v.id, efforts: ['low', 'medium', 'high'], referenceCost: 0.001 })), recommendedModel: d.prices[0]?.id, efforts: ['low', 'medium', 'high'], timezone: 'Asia/Shanghai', message: '模拟模型能力；未执行模型调用' };
        case 'codexAuth.inspect': return { profiles: [], processes: [], storageMode: 'file', home: '/home/preview/.codex', warnings: ['预览模式：不读取登录凭据。'] };
        case 'codexAuth.profiles.list': case 'codexAuth.list': return { profiles: [], activeProfileId: null };
        case 'codexAuth.status': return { authenticated: false, profiles: [] };
        case 'codexAuth.reconcile': return { issues: [], settings: s };
        case 'codexAuth.model.list': return { models: d.prices.map(v => ({ id: v.id, name: v.name })) };
        case 'agyAuth.inspect': return { authenticated: false, workspaces: [], warnings: ['预览模式：不读取登录凭据。'] };
        case 'codexApi.list': return s.codexApis || [];
        case 'codexApi.status': return { sourceId: p.sourceId, ready: true, selected: null };
        case 'codexApi.probe': return { ready: true, message: '模拟连接成功；未发起网络请求' };
        case 'codexApi.save': {const c = { ...copy(p.connection), id: p.connection.id || crypto.randomUUID(), revision: crypto.randomUUID() };s.codexApis = [...(s.codexApis || []).filter(v => v.id !== c.id), c];return { settings: s, connection: c };}
        case 'codexApi.remove': s.codexApis = (s.codexApis || []).filter(v => v.id !== p.id);return { settings: s };
        case 'agentApi.inspect': {const source = s.sources.find(v => v.id === p.sourceId);return { provider: source?.provider, configPath: `${source?.path || '/home/preview'}/config.toml`, revision: 'preview', nativeRevision: 'preview', baseUrl: 'https://api.example.invalid/v1', model: d.prices[0]?.id, apiKey: '', noKey: true, ...(d.apiConfigs?.[p.sourceId] || {}) };}
        case 'agentApi.save': {const { apiKey, ...config } = p;d.apiConfigs ||= {};d.apiConfigs[p.sourceId] = copy(config);return { settings: s };}
        case 'quotaEstimates.list': return (d.dashboard.quotaEstimates || []).filter(v => !p.accountKey || v.accountKey === p.accountKey);
        case 'creditEstimates.list': return (d.dashboard.creditEstimates || []).filter(v => !p.accountKey || v.accountKey === p.accountKey);
        case 'quotaEstimates.get': case 'creditEstimates.get': return (d.dashboard[method.startsWith('credit') ? 'creditEstimates' : 'quotaEstimates'] || []).find(v => v.id === p.id) || null;
        case 'autoEstimates.configure': {const source = s.sources.find(v => v.id === p.sourceId);if (!source) throw new Error('目录不存在');source.autoValuation = { enabled: p.enabled };return { settings: s };}
        case 'pets.list': return { pets: [], selectedLibraryId: null, warnings: [] };
        case 'pets.current': return { pet: null, error: '预览模式：原生桌宠需在目标系统验收。' };
        case 'operations.status': return { busy: false, operations: [] };
        case 'quotaResetNotifications.take': return [];
        default:
          if (/^(?:codexAuth\.(?:login|switch|openUrl|adopt|enable|profiles\.)|agyAuth\.(?!inspect)|subscriptions\.openLogin|wakeups\.(?:deploy|remove|run|enable|disable)|sessions\.hooks\.(?:repair|remove)|accounts\.native\.remove|pets\.(?:import|select|asset)|(?:quota|credit)Estimates\.(?:start|stop|restart|repair)|autoEstimates\.(?:tick|rebuild))/.test(method)) return this.boundary(method);
          throw new Error(`未实现的预览调用 / Unhandled preview call: ${method}`);
      }
    }
    native(command, p = {}) {
      switch (command) {
        case 'desktop_info': return this.desktop;
        case 'desktop_panel': this.desktop.panelOpen = p.open;this.emit('desktop:panel', this.desktop);if (!p.open) this.hooks.window?.('panel-close');return this.desktop;
        case 'desktop_panel_pin': this.desktop.panelPinned = p.pinned;this.emit('desktop:panel', this.desktop);return this.desktop;
        case 'desktop_panel_page': this.desktop.panelPage = p.page;return null;
        case 'desktop_panel_ready': return null;
        case 'desktop_main_ready': return null;
        case 'desktop_mode': this.desktop.mode = p.mode;this.desktop.effectiveMode = p.mode;this.emit('desktop:status', this.desktop);return this.desktop;
        case 'desktop_pet_scale': this.desktop.petScale = p.scale;this.emit('desktop:status', this.desktop);return this.desktop;
        case 'desktop_materials': this.desktop.materialPreferences = {main:p.main, floating:p.floating};this.desktop.materials = {main:p.main === 'default'?'mica':p.main, floating:p.floating === 'default'?'acrylic':p.floating};this.emit('desktop:status', this.desktop);return this.desktop;
        case 'desktop_appearance': this.emit('desktop:status', this.desktop);return null;
        case 'desktop_detail': this.hooks.navigate?.(p.view);return null;
        case 'desktop_action': {
          const action = p.action;
          if (action.startsWith('settings-target:')) this.hooks.navigate?.({ target: JSON.parse(action.slice(16)) });
          else if (['settings', 'prices', 'add-host', 'add-source', 'add-quota', 'open'].includes(action)) this.hooks.navigate?.({ action });
          else if (action === 'quit' || action === 'quit-confirmed') this.hooks.window?.('close');
          else if (action === 'toggle-count') this.desktop.showCount = !this.desktop.showCount;
          else if (['hide-ball', 'show-ball'].includes(action)) this.desktop.hidden = action === 'hide-ball';
          else if (!['snap', 'reset-position'].includes(action)) return this.boundary(action);
          this.emit('desktop:status', this.desktop);return this.desktop;
        }
        case 'updates_info': return this.updates;
        case 'updates_automatic': this.updates.automatic = p.automatic;return this.updates;
        case 'updates_check': case 'updates_panel_check': return { ...this.updates, phase: 'current' };
        case 'notifications_info': return { permission: 'unknown', error: '预览模式：请在目标系统验证通知权限与显示。' };
        case 'select_local_path': return this.hooks.pickPath?.(p) || null;
        case 'check_local_path': return Boolean(p.path?.trim());
        default:
          if (/^(updates_|notifications_|session_sounds_)/.test(command)) return this.boundary(command);
          throw new Error(`未实现的预览调用 / Unhandled preview call: ${command}`);
      }
    }
    async invoke(command, args = {}) {
      const method = command === 'engine_call' ? args.method : command,p = command === 'engine_call' ? args.params || {} : args;
      const entry = { method, params: redact(copy(p)), startedAt: Date.now() };let before;
      try {
        if (this.delay) await new Promise(resolve => setTimeout(resolve, this.delay));
        if (this.hooks.active && !this.hooks.active()) throw new Error('预览已切换，请重新操作');
        const mutation = command === 'engine_call' && this.methods[method]?.invalidates.length > 0;
        if (this.fail === 'save' && mutation && !/^(sources\.(scan|rescan)|quotas\.refresh|hosts\.)/.test(method)) throw new Error('模拟保存失败：数据未写入，输入已保留');
        if (this.fail === 'query' && /^(dashboard|quotas\.refresh|hosts\.(sample|monitor\.refresh)|network\.test)/.test(method)) throw new Error('模拟查询失败：连接不可用，请重试');
        const desktopChange = ['desktop_mode', 'desktop_pet_scale', 'desktop_panel_pin', 'desktop_action'].includes(command);
        if (mutation || desktopChange) before = copy(this.data);
        const result = copy(command === 'engine_call' ? this.engine(method, p) : this.native(command, p));
        if (desktopChange) {this.data.desktop = Object.fromEntries(['mode','effectiveMode','petScale','hidden','showCount','panelPinned'].map(k => [k, this.desktop[k]]));this.persist();}
        if (mutation) {
          this.persist();
          const revisions = this.methods[method].invalidates;for (const key of revisions) this.revisions[key] = (this.revisions[key] || 0) + 1;
          this.emit('runtime:changed', { revisions: copy(this.revisions), operationId: args.operationId });
        }
        entry.result = redact(result);return result;
      } catch (error) {if (before) this.data = before;entry.error = error.message;throw error;}
      finally {entry.duration = Date.now() - entry.startedAt;this.hooks.log?.(entry);}
    }
  }
  return { Model, redact };
});
