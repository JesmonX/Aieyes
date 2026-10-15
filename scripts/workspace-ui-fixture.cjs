// Synthetic RPCs shared by workspace navigation regressions.
function fixture({platform, failStatus = false}) {
  window.setInterval = () => 0;
  const stamp = Math.floor(Date.now() / 1000);
  const accounts = Array.from({length: 6}, (_, i) => ({id: 'a' + i, name: 'Codex 账号 ' + i, provider: 'codex', quotaEnabled: true}));
  accounts.push({id: 'claude', name: 'Claude 账号', provider: 'claude', quotaEnabled: true}, {id: 'deepseek', name: 'DeepSeek 账号', provider: 'deepseek', quotaEnabled: true});
  const sources = accounts.map(a => ({id: 's-' + a.id, provider: a.provider, accountId: a.id, name: a.name + '来源', path: '/fixture/' + a.id, enabled: true, hostId: null}));
  const settings = {version: 6, accounts, sources, agents: ['codex','claude','antigravity','deepseek','custom'].map(provider => ({provider, enabled: sources.some(s => s.provider === provider), machineIds: ['local']})), hosts: [{id: 'host', name: '测试服务器', target: 'fixture.invalid', enabled: true, metrics: ['cpu'], details: [], devices: [], shell: '/bin/bash', identityFile: ''}], modelMappings: {}, refreshSeconds: 300, historyRefreshSeconds: 400, serverForegroundRefreshSeconds: 3, serverRefreshSeconds: 12, historyShowDeletedAccounts: true, proxy: {mode: 'custom', url: 'http://proxy.invalid:8080'}, proxyTestUrls: ['https://fixture.invalid'], appearance: {theme: 'system', accent: 'indigo', chartPalette: 'classic', chartContrast: 'high'}, codexApis: [{id: 'api', sourceId: 's-a0', name: '已有 API', model: 'fixture', baseUrl: 'https://fixture.invalid/v1'}]};
  const usage = {total: 70000, cost: 1, pricedTokens: 70000, events: 1, tokens: {input: 30000, output: 10000, cacheRead: 28000, cacheWrite: 2000}};
  const record = {id: 'running', accountKey: 'codex:a0', kind: 'quota', status: 'active', samplingMode: 'manual', valuationMode: 'fiveHour', fiveHourValue: 20, weeklyValue: 100, weeklyDirectValue: 100, weeklyRatioValue: 110, startedAt: stamp - 100, checkpointAt: stamp, sourceIds: ['s-a0'], sourceNames: ['Codex 账号 0来源'], windowName: '5h', windowId: 'five', calculationNote: '测试样本', reason: '', consumedPercent: 10, totalTokens: 70000, pricedTokens: 70000, cost: 1, prices: []};
  const quotas = accounts.map(a => ({provider: a.provider, accountId: a.id, name: a.name, sourceId: 's-' + a.id, updatedAt: stamp, origin: 'live', windows: a.provider === 'deepseek' ? [] : [{id: 'five', name: '5h', windowMinutes: 300, usedPercent: 20, resetsAt: stamp + 1000}, {id: 'week', name: '7d', windowMinutes: 10080, usedPercent: 10, resetsAt: stamp + 100000}], credits: a.provider === 'codex' ? {balance: '100', unlimited: false} : null}));
  const nav = window.navigationFixture = {settings, quotas, records: [record, {...record, id: 'history', status: 'completed', checkpointAt: stamp - 20}], calls: [], listeners: {}, failSave: false, holdSave: false, failStatus};
  const copy = v => structuredClone(v);
  nav.emit = (name, payload) => Promise.all((nav.listeners[name] || []).map(fn => fn({payload})));
  function merge(base, next, current) {
    const out = copy(current);
    for (const key of Object.keys(next)) if (JSON.stringify(base[key]) !== JSON.stringify(next[key])) out[key] = copy(next[key]);
    return out;
  }
  function dashboard(params = {}) {
    const day = new Date().toISOString().slice(0, 10);
    return {generatedAt: Date.now() / 1000, summary: usage, quotas: quotas.filter(q => (!params.provider || q.provider === params.provider) && (!params.accountId || q.accountId === params.accountId)), quotaOrder: accounts.map(a => a.provider + ':' + a.id), quotaEstimates: copy(nav.records), creditEstimates: [], models: [{...usage, key: 'fixture'}], modelOptions: ['fixture'], trendDays: [{...usage, key: day}], heatmap: [{...usage, key: day}], dayModels: [{day, model: 'fixture', usage}], pricingGaps: [], sources: sources.map(s => ({...s, status: {updatedAt: stamp}}))};
  }
  window.__TAURI__ = {
    event: {async listen(name, fn) {(nav.listeners[name] ||= []).push(fn); return () => {}; }},
    window: {getCurrentWindow: () => ({async isMaximized() {return false;}, async onResized() {return () => {};}})},
    core: {async invoke(command, args) {
      nav.calls.push({command, args: copy(args)});
      if (command === 'desktop_info' || command === 'desktop_panel') return {platform, page: 'agent', mode: 'floating', effectiveMode: 'floating', sessions: [], panelOpen: true, materials: {main: 'opaque', floating: 'opaque'}};
      if (command !== 'engine_call') return {};
      const {method, params} = args;
      if (method === 'hello') return {version: 'fixture'};
      if (method === 'settings.get') return copy(nav.settings);
      if (method === 'dashboard' || method === 'dashboard.summary') return dashboard(method === 'dashboard.summary' ? {} : params);
      if (method === 'settings.patch') {
        if (nav.holdSave) {nav.holdSave = false; await new Promise(resolve => nav.releaseSave = resolve);}
        if (nav.failSave) throw Error('模拟保存失败');
        nav.settings = merge(params.base, params.settings, nav.settings); return copy(nav.settings);
      }
      if (method === 'credentials.save') return {path: '/fixture/updated-key'};
      if (method === 'quotas.refresh') return copy(quotas);
      if (method === 'quotaEstimates.stop') {nav.records.find(r => r.id === params.id).status = 'completed'; return {};}
      if (method === 'hosts.sample') return [{id: 'host', sample: {timestamp: Date.now()/1000, cpu: [{id: 'cpu', utilization: 12}], errors: {}}}];
      if (method === 'accounts.status.refresh') return {accountKey: params.accountKey, sourceId: params.sourceId, machineId: 'local', current: params.accountKey.endsWith(':a0'), credential: true, checkedAt: stamp};
      if (method === 'accounts.status.get' && nav.failStatus) throw Error('模拟设备状态读取失败');
      if (['accounts.status.get','accounts.cleanup.list','sources.scan','prices.list','wakeups.list'].includes(method)) return [];
      if (method === 'accounts.deployments.sync') return {failedTasks: []};
      if (method === 'codexAuth.inspect') return {profiles: [], processes: [], storageMode: 'file'};
      if (method === 'sessions.list') return {sessions: [], warnings: []};
      if (method === 'quotas.schedule') return {nextDueAt: stamp + 86400};
      return {};
    }}
  };
}

module.exports={fixture};
