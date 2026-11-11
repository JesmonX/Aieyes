/* Website demo, application side. Loaded in the frame before the desktop frontend's own scripts.
   The frontend runs unmodified: this file supplies the clock, storage and IPC it expects, backed by
   the synthetic model of scripts/desktop-preview. Nothing is persisted and nothing leaves the page. */
(() => {
  const config = window.AieyesDemoConfig;
  const query = new URLSearchParams(location.search);
  const language = query.get('lang') === 'zh-CN' ? 'zh-CN' : 'en';
  const theme = query.get('theme') === 'light' ? 'light' : 'dark';
  const panel = document.documentElement.dataset.surface === 'panel';
  const zh = language === 'zh-CN';
  const text = zh ? {
    boundary: '演示环境不执行此操作：它需要真实的系统或外部服务。',
    window: '这是网页中的演示窗口，会一直保持打开。',
    updates: '演示环境不检查更新。',
    notifications: '演示环境不发送系统通知。',
    summary: '3 个活跃会话', local: '本机', completed: '已完成',
  } : {
    boundary: 'The demo does not perform this action: it needs a real system or an external service.',
    window: 'This demo window lives in the page and stays open.',
    updates: 'The demo does not check for updates.',
    notifications: 'The demo does not send system notifications.',
    summary: '3 active sessions', local: 'Local device', completed: 'Completed',
  };
  // The page that embeds the frame; absent when the demo is opened as a page of its own.
  let host = null;
  try {if (window.parent !== window) host = window.parent.AieyesDemoHost ?? null;} catch {}

  // The synthetic day starts when the page loads and then runs at normal speed, so reset times,
  // "today" and running turns stay consistent with the data while timers keep working.
  const RealDate = Date, shift = RealDate.now() - config.stamp * 1000;
  window.Date = class extends RealDate {
    constructor(...args) {if (args.length) super(...args);else super(RealDate.now() - shift);}
    static now() {return RealDate.now() - shift;}
  };

  // Interface preferences live for one page view only.
  const memory = new Map(Object.entries({
    'aieyes.appearance': JSON.stringify({ language, theme, accent: 'indigo', chartPalette: 'classic', chartContrast: 'high' }),
    'aieyes.language': language,
    'quota.expanded.v2.detail.codex:codex': 'true',
    'quota.expanded.v2.panel.codex:codex': 'true',
    'aieyes.sessions.panel': 'true',
  }));
  const methods = {
    getItem: key => memory.get(String(key)) ?? null,
    setItem: (key, value) => {memory.set(String(key), String(value));},
    removeItem: key => {memory.delete(String(key));},
    clear: () => memory.clear(),
    key: index => [...memory.keys()][index] ?? null,
  };
  Object.defineProperty(window, 'localStorage', { configurable: true, value: new Proxy({}, {
    get: (_, name) => name === 'length' ? memory.size : methods[name] ?? (typeof name === 'string' ? memory.get(name) : undefined),
    set: (_, name, value) => {memory.set(String(name), String(value));return true;},
    has: (_, name) => memory.has(name),
    deleteProperty: (_, name) => {memory.delete(name);return true;},
    ownKeys: () => [...memory.keys()],
    getOwnPropertyDescriptor: (_, name) => memory.has(name) ? { enumerable: true, configurable: true, value: memory.get(name) } : undefined,
  }) });

  const iso = day => `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, '0')}-${String(day.getDate()).padStart(2, '0')}`;
  function createModel(seed) {
    const { Model } = window.AieyesPreviewModel;
    const contract = Object.entries(window.AieyesContract.methods).map(([name, method]) => ({ name, ...method }));
    seed.settings.appearance = { ...seed.settings.appearance, language, theme };
    const usage = seed.usage, last = usage.totals.length - 1;
    const today = new Date();today.setHours(12, 0, 0, 0);
    const keyAt = index => {const day = new Date(today);day.setDate(day.getDate() - (last - index));return iso(day);};
    const indexOf = key => last - Math.round((today - new Date(key + 'T12:00:00')) / 86400000);
    const record = (total, cost) => ({
      total, cost, pricedTokens: total, events: total ? Math.max(1, Math.floor(total / 28000)) : 0,
      tokens: { ...Object.fromEntries(Object.entries(usage.split).map(([name, part]) => [name, Math.floor(total * part)])), reasoning: 0 },
    });
    class Demo extends Model {
      boundary() {throw new Error(text.boundary);}
      targets() {return super.targets().map(row => row.machineId === 'local' ? { ...row, machineName: text.local } : row);}
      // Readings drift a little on every sample, as a monitored machine's would.
      hostRows() {
        const wave = (value, span, phase) => Math.max(1, Math.min(99, Math.round(value + span * Math.sin(Date.now() / 9000 + phase))));
        return super.hostRows().map(row => {
          const sample = row.sample;if (!sample) return row;
          sample.cpu = (sample.cpu ?? []).map((cpu, i) => ({ ...cpu, utilization: wave(cpu.utilization, 7, i) }));
          sample.gpu = (sample.gpu ?? []).map((gpu, i) => ({ ...gpu, utilization: wave(gpu.utilization, 9, 2 + i), ...(gpu.powerWatts ? { powerWatts: Math.round(gpu.powerWatts + 12 * Math.sin(Date.now() / 9000 + 2 + i)) } : {}) }));
          sample.network = (sample.network ?? []).map((link, i) => ({ ...link, ...(link.rxBytesPerSecond ? { rxBytesPerSecond: Math.round(link.rxBytesPerSecond * (1 + .3 * Math.sin(Date.now() / 7000 + i))) } : {}) }));
          return row;
        });
      }
      // The published day, week and year of usage, narrowed by the filters the analysis page sends.
      dashboard(params = {}) {
        const result = super.dashboard(params), sources = this.data.settings.sources;
        const source = params.sourceId ? sources.find(s => s.id === params.sourceId) : null;
        const share = model => {
          if (params.model && params.model !== model.key) return 0;
          if (params.provider && params.provider !== model.provider) return 0;
          if (!source) return params.sourceId ? 0 : 1;
          if (source.provider !== model.provider) return 0;
          const peers = sources.filter(s => s.provider === model.provider);
          return peers.length < 2 ? 1 : peers.indexOf(source) === 0 ? .62 : .38 / (peers.length - 1);
        };
        const day = index => {
          const total = usage.totals[index] ?? 0, models = usage.models.map(model => {
            const part = share(model);
            return { key: model.key, usage: record(Math.round(total * model.ratio * part), Math.round(total * usage.rate * model.costShare * part * 100) / 100) };
          });
          const all = record(models.reduce((sum, m) => sum + m.usage.total, 0), Math.round(models.reduce((sum, m) => sum + m.usage.cost, 0) * 100) / 100);
          return { key: keyAt(index), all, models };
        };
        const range = params.dateRange, explicit = ['custom', 'all'].includes(range?.kind);
        const clamp = index => Math.max(0, Math.min(last, index));
        const end = range?.kind === 'custom' ? clamp(indexOf(range.end)) : last;
        const start = range?.kind === 'custom' ? clamp(indexOf(range.start)) : range?.kind === 'all' ? 0 : clamp(last - (params.days || 1) + 1);
        const rows = Array.from({ length: last + 1 }, (_, index) => day(index));
        const sum = (from, to) => rows.slice(from, to + 1).reduce((total, row) => {
          total.total += row.all.total;total.cost += row.all.cost;total.pricedTokens += row.all.pricedTokens;total.events += row.all.events;
          for (const name of Object.keys(total.tokens)) total.tokens[name] += row.all.tokens[name];
          return total;
        }, record(0, 0));
        const flat = (from, to) => rows.slice(from, to + 1).map(row => ({ ...row.all, key: row.key }));
        const trendStart = explicit ? start : Math.min(start, clamp(last - 6));
        const side = (from, to) => {const total = sum(from, to);return { start: keyAt(from), end: keyAt(to), total: total.total, cost: total.cost, pricedTokens: total.pricedTokens, recordCount: total.events, pricingComplete: true };};
        const length = end - start + 1;
        const models = usage.models.map(model => rows.slice(start, end + 1).reduce((total, row) => {
          const value = row.models.find(m => m.key === model.key).usage;
          total.total += value.total;total.cost += value.cost;total.pricedTokens += value.pricedTokens;total.events += value.events;
          for (const name of Object.keys(total.tokens)) total.tokens[name] += value.tokens[name];
          return total;
        }, { ...record(0, 0), key: model.key })).filter(model => model.events);
        return {
          ...result, summary: sum(start, end), models, modelOptions: usage.models.map(model => model.key),
          days: flat(start, end), trendDays: flat(trendStart, end), heatmap: flat(explicit ? start : 0, end),
          dayModels: rows.slice(trendStart, end + 1).flatMap(row => row.models.map(model => ({ day: row.key, model: model.key, usage: model.usage }))),
          comparison: range?.kind === 'all' || start - length < 0 ? null : { current: side(start, end), previous: side(start - length, start - 1), currentOpen: end === last },
        };
      }
      engine(method, params = {}) {
        const data = this.data, now = Date.now() / 1000;
        switch (method) {
          case 'hello': return { version: data.version, runtimeInvalidations: true, hostMonitoring: true };
          case 'sessions.list': return { sessions: data.sessions, sources: [], warnings: [], unavailable: false };
          case 'wakeups.status': {
            const row = data.wakeups.find(r => r.task.id === params.id), task = row?.task;
            const next = new Date();next.setHours(...(task?.times?.[0] ?? '08:30').split(':').map(Number), 0, 0);
            if (next.getTime() / 1000 <= now) next.setDate(next.getDate() + 1);
            const ran = offset => {const at = new Date(next);at.setDate(at.getDate() - offset);return { startedAt: at.getTime() / 1000, endedAt: at.getTime() / 1000 + 2, status: 'success', model: task?.model, effort: task?.effort, message: text.completed };};
            return { installed: Boolean(row?.deployment), enabled: row?.deployment?.enabled !== false, timezone: 'UTC+08:00', nextRunAt: next.getTime() / 1000, history: row?.deployment ? [ran(1), ran(2), ran(3)] : [] };
          }
          case 'pets.list': return data.petLibrary;
          case 'pets.asset': case 'pets.current': return { pet: data.petLibrary.pets[0], imageDataUrl: config.pet };
          case 'pets.select': return { pet: data.petLibrary.pets[0] };
        }
        try {return super.engine(method, params);}
        catch (error) {if (/Unhandled preview call/.test(error.message)) return this.boundary();throw error;}
      }
      native(command, params = {}) {
        switch (command) {
          case 'desktop_panel': return this.desktop;
          case 'focus_session': return { outcome: 'unsupported', name: '' };
          case 'updates_info': case 'updates_check': case 'updates_panel_check': return { ...this.updates, message: text.updates };
          case 'notifications_info': return { permission: 'unknown', error: text.notifications };
        }
        try {return super.native(command, params);}
        catch (error) {if (/Unhandled preview call/.test(error.message)) return this.boundary();throw error;}
      }
    }
    const model = new Demo(seed, contract, [], {
      // The panel's "open details" and "settings" lead to the main window.
      navigate: view => {if (host) host.open(window, view);else location.href = 'index.html' + location.search;},
      window: () => {},
      pickPath: () => null,
    });
    Object.assign(model.desktop, { summary: text.summary, panelOpen: panel, windowCorners: 'round', hourCycle: 'h23' });
    return model;
  }

  // The data can arrive while later scripts are still loading; the model needs the frontend's contract.
  const parsed = new Promise(resolve => document.addEventListener('DOMContentLoaded', resolve, { once: true }));
  const ready = fetch(config.data.replace('{language}', language)).then(response => {
    if (!response.ok) throw new Error('Demo data unavailable');
    return response.json();
  }).then(async seed => {await parsed;return createModel(seed);});
  ready.catch(error => host?.failed(window, String(error)));
  const say = message => {if (typeof notify === 'function') notify(message);};
  const native = {
    isMaximized: async () => Boolean(host?.expanded?.()),
    toggleMaximize: async () => {if (host?.toggleExpanded) await host.toggleExpanded();else say(text.window);},
    minimize: async () => say(text.window),
    close: async () => say(text.window),
    startDragging: async () => {},
    onResized: async callback => {window.addEventListener('resize', callback);return () => window.removeEventListener('resize', callback);},
  };
  window.__TAURI__ = {
    core: { invoke: async (command, args) => structuredClone(await (await ready).invoke(command, args === undefined ? undefined : structuredClone(args))) },
    event: { listen: async (name, callback) => (await ready).listen(name, callback) },
    window: { getCurrentWindow: () => native },
  };

  // `state` is the frontend's own top-level binding; scripts of this page share it.
  const view = () => {
    const tab = state.page === 'settings' ? state.settingsTab : null;
    return {
      page: state.page, tab, section: tab === 'general' ? state.generalTab : null, agent: tab === 'integrations' ? state.settingsAgent : null,
      sessions: Boolean(document.querySelector('#live-sessions')?.open), theme: document.documentElement.dataset.theme,
    };
  };
  async function go(target) {
    await window.AieyesApp.ready;
    if (panel) {if (['agent', 'servers'].includes(target.page)) await window.AieyesApp.navigatePage(target.page);}
    else if (target.page === 'settings') await window.AieyesApp.openSettings({ category: target.tab, ...(target.tab === 'integrations' ? { provider: target.agent ?? 'codex' } : {}), ...(target.tab === 'general' && target.section ? { section: target.section } : {}) });
    else if (target.page === 'setup') await window.AieyesSetup.open();
    else await window.AieyesApp.navigatePage(target.page);
    const sessions = document.querySelector('#live-sessions');
    if (sessions && typeof target.sessions === 'boolean') sessions.open = target.sessions;
    report();
  }
  let reported = '', queued = false;
  function report() {
    queued = false;
    const next = view(), key = JSON.stringify(next);
    if (key !== reported) {reported = key;host?.changed(window, next);}
  }
  window.AieyesDemo = { go, view, language, panel };
  window.addEventListener('DOMContentLoaded', async () => {
    try {
      await window.AieyesApp.ready;await window.AieyesTheme?.loadFonts();
      const start = query.get('view');
      if (start) await go(JSON.parse(start));
      new MutationObserver(() => {if (!queued) {queued = true;queueMicrotask(report);}})
        .observe(document.documentElement, { subtree: true, attributes: true, attributeFilter: ['aria-current', 'aria-selected', 'aria-pressed', 'open', 'data-theme'] });
      reported = JSON.stringify(view());
      host?.ready(window, view());
    } catch (error) {host?.failed(window, String(error));}
  }, { once: true });
})();
