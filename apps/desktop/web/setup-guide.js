/* Goal-based help. Saved configuration and observed results are separate facts. */
(() => {
  if (PANEL) return;
  const key = 'aieyes.setup.v1', e = escapeHTML;
  const read = () => {
    try {
      const value = JSON.parse(localStorage.getItem(key) || '{}');
      if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
      return {seen:value.seen === true, scene:['local','ssh','mixed'].includes(value.scene) ? value.scene : undefined,
        expanded:value.expanded && typeof value.expanded === 'object' && !Array.isArray(value.expanded) ? value.expanded : {}};
    } catch (_) { return {}; }
  };
  let progress = read(), active = false, revision = 0, loading = false;
  let observations = {}, readErrors = [];
  const save = () => { try { localStorage.setItem(key, JSON.stringify(progress)); } catch (_) {} };
  const settings = () => state.settings;
  const scene = () => progress.scene ?? (settings().hosts.length ? settings().localDeviceEnabled === false ? 'ssh' : 'mixed' : 'local');
  const inScene = source => scene() === 'mixed' || (scene() === 'local' ? !source.hostId : Boolean(source.hostId));
  const machineName = id => id === 'local' ? '本机' : settings().hosts.find(h => h.id === id)?.name || settings().hosts.find(h => h.id === id)?.target || '已移除设备';
  const accountKey = a => a.provider + ':' + a.id;
  const linked = (a,s) => window.AieyesAgentSettings.linked(a,s);
  const sources = () => settings().sources.filter(inScene);
  const relevantSource = source => settings().agents?.find(a => a.provider === source.provider)?.enabled
    || settings().accounts.some(a => !a.archived && a.provider === source.provider)
    || source.id !== 'local-' + source.provider;
  const accountSources = a => settings().sources.filter(s => linked(a,s));
  const accounts = (archived = false) => settings().accounts.filter(a => (archived || !a.archived) && (!accountSources(a).length || accountSources(a).some(inScene)));
  const snapshot = () => readErrors.includes('dashboard') ? null : observations.dashboard ?? state.dashboard;
  const accountTarget = a => ({accountKey:accountKey(a)});
  const general = section => ({category:'general',section});
  const statusLabels = {configured:'已配置',pending:'待验证',missing:'待配置',paused:'已暂停',queryFailed:'查询失败',querySucceeded:'查询成功',collectionFailed:'采集异常',collected:'采集成功',unconfigured:'未配置',reconfirm:'待确认',custom:'单独设置',inherited:'沿用全局',archived:'已归档',selected:'已选择',excluded:'未选择',noTasks:'无任务',draft:'未部署',pendingRemoval:'待移除',pendingSync:'待同步',modified:'待部署',deployed:'已部署',stopped:'已停用',missingSource:'目录变化',priceMissing:'缺少价格',noPriceGaps:'未报缺价',sampled:'采样成功'};
  const row = (label,text,target,level = 'info',status = 'configured',details = '',updatedAt = 0) => ({label,text,target,level,status,details,updatedAt});
  const stamp = value => new Date(value * 1000).toLocaleString();
  const action = (label,target,id) => `<button type="button" ${id ? `data-guide-step="${e(id)}"` : ''} data-guide-target="${e(JSON.stringify(target))}">${e(label)}</button>`;
  const rowsHTML = rows => `<ul class="setup-checks">${rows.map(r => `<li data-guide-level="${r.level}" data-guide-state="${r.status}"><div><strong>${e(r.label)}</strong><div class="setup-check-status"><span class="setup-status-badge">${e(statusLabels[r.status])}</span><span>${e(r.text)}</span></div>${r.updatedAt > 0 ? `<time datetime="${new Date(r.updatedAt*1000).toISOString()}">上次数据：${e(stamp(r.updatedAt))}</time>` : ''}${r.details ? `<details class="setup-row-details"><summary>状态说明</summary><p>${e(r.details)}</p></details>` : ''}</div>${r.target ? action('查看设置',r.target) : ''}</li>`).join('')}</ul>`;
  function quotaRow(a) {
    const available = accountSources(a).filter(s => inScene(s) && AieyesUI.sourceEnabled(settings(),s));
    const q = snapshot()?.quotas?.find(q => q.provider === a.provider && q.accountId === a.id);
    const result = row((providers[a.provider] ?? a.provider) + ' · ' + a.name,'刷新账户额度以验证连接',accountTarget(a),'pending','pending');
    if (!a.quotaEnabled) Object.assign(result,{text:'开启“显示并查询额度”',level:'warn',status:'paused'});
    else if (!available.length) Object.assign(result,{text:'关联账户并启用对应机器的采集',level:'warn',status:'missing'});
    else if (q?.error) Object.assign(result,{text:'查看账户错误后重试',level:'warn',status:'queryFailed',details:'仍显示上次数据时，请核对更新时间。',updatedAt:q.updatedAt});
    else if (['live','command'].includes(q?.origin) && q.updatedAt > 0 && available.some(s => s.id === q.sourceId)) Object.assign(result,{text:'上次实时查询已完成',level:'ok',status:'querySucceeded',updatedAt:q.updatedAt});
    return result;
  }
  function usageRows() {
    const rows = sources().filter(s => s.provider !== 'deepseek' && relevantSource(s)).map(s => {
      const status = snapshot()?.sources?.find(r => r.id === s.id)?.status;
      const enabled = AieyesUI.sourceEnabled(settings(),s);
      const result = row((providers[s.provider] ?? s.provider) + ' · ' + machineName(s.hostId || 'local') + ' · ' + s.name,'同步记录以验证日志采集',{provider:s.provider,section:'machines'},'pending','pending');
      if (!enabled) Object.assign(result,{text:'检查机器、采集开关和目录绑定',level:'warn',status:'missing'});
      else if (AieyesUI.syncFailure(status)) Object.assign(result,{text:'检查日志目录和机器连接',level:'warn',status:'collectionFailed'});
      else if (status?.updatedAt > 0) Object.assign(result,{text:'上次日志采集已完成',level:'ok',status:'collected',updatedAt:status.updatedAt});
      return result;
    });
    if (!rows.length) rows.push(row('用量采集','选择 Agent、机器和日志目录',{section:'machines'},'pending','missing'));
    const a = accounts();
    return rows.concat(a.length ? a.map(quotaRow) : [row('账户与额度','添加账户并完成认证',{section:'accounts'},'pending','missing')]);
  }
  function networkRows() {
    const rows = [];
    if (scene() !== 'ssh') {
      const modes = {system:'跟随系统代理',direct:'直连',custom:'指定代理'};
      rows.push(row('本机应用网络',(modes[settings().proxy?.mode] ?? '跟随系统代理') + ' · 可测试出站连接',general('network')));
      for (const s of sources().filter(s => !s.hostId && s.proxy && s.proxy.mode !== 'inherit')) rows.push(row((providers[s.provider] ?? s.provider) + ' · ' + s.name,'此目录使用独立代理设置',{provider:s.provider,section:'machines'},'info','custom'));
    }
    if (scene() !== 'local') {
      if (!settings().hosts.length) rows.push(row('SSH 服务器','添加服务器后配置账户网络',{category:'hosts'},'pending','missing'));
      const remote = accounts().flatMap(a => [...new Set(accountSources(a).filter(s => s.hostId).map(s => s.hostId))].map(id => ({a,id})));
      if (!remote.length) rows.push(row('服务器账户网络','连接账户后配置对应机器',{section:'accounts'},'pending','missing'));
      for (const {a,id} of remote) {
        const device = a.deviceSettings?.find(d => d.machineId === id),needsConfirm = device?.preCommandResetRequired,configured = Boolean(device?.preCommand?.trim());
        rows.push(row(a.name + ' · ' + machineName(id),needsConfirm ? '重新确认并保存前置命令' : configured ? '刷新账户额度以检查连接' : '需要代理时设置前置命令',
          {...accountTarget(a),machineId:id,section:'command'},needsConfirm ? 'warn' : 'info',needsConfirm ? 'reconfirm' : configured ? 'configured' : 'unconfigured','服务器可直连时无需填写；命令内容仅在账户设置中查看。'));
      }
    }
    return rows;
  }
  function refreshRows() {
    return [row('全局自动刷新',`额度 ${settings().refreshSeconds} 秒 · 记录 ${settings().historyRefreshSeconds ?? Math.max(10,settings().refreshSeconds)} 秒`,general('refresh')),
      ...(scene() !== 'local' ? [row('服务器监控',`前台 ${settings().serverForegroundRefreshSeconds ?? 2} 秒 · 后台 ${settings().serverRefreshSeconds} 秒`,general('refresh'))] : []),
      ...accounts().map(a => row(a.name,`每 ${a.quotaRefreshSeconds ?? settings().refreshSeconds} 秒查询`,accountTarget(a),'info',a.quotaRefreshSeconds == null ? 'inherited' : 'custom'))];
  }
  function hostRows() {
    if (!settings().hosts.length) return [row('SSH 连接','导入 SSH 配置或添加服务器',{category:'hosts'},'pending','missing')];
    return settings().hosts.map(h => {
      const observed = state.hosts.find(r => r.id === h.id);
      return row(h.name || h.target,`Agent 采集${h.enabled ? '已开启' : '已暂停'} · 资源监控${h.monitoringEnabled !== false ? '已开启' : '已暂停'}`,{category:'hosts',hostId:h.id},observed?.error ? 'warn' : 'info',observed?.error ? 'collectionFailed' : observed?.sample ? 'sampled' : 'pending','此状态来自资源监控；账户查询结果在账户页查看。');
    });
  }
  function wakeupRows() {
    if (!observations.wakeups) return [row('任务状态',readErrors.includes('wakeups') ? '读取失败，请前往任务页重试' : '前往任务页读取状态',{category:'wakeups'},'pending','reconfirm')];
    const records = observations.wakeups.filter(r => { const source = settings().sources.find(s => s.id === r.task.sourceId); return !source || inScene(source); });
    if (!records.length) return [row('定时任务','按需创建并部署任务',{category:'wakeups'},'info','noTasks')];
    return records.map(r => {
      const d = r.deployment,result = row(r.task.name,'部署后才会自动运行',{category:'wakeups'},'pending','draft');
      if (!settings().sources.some(s => s.id === r.task.sourceId)) Object.assign(result,{text:'检查任务引用的日志目录',level:'warn',status:'missingSource'});
      else if (d?.state === 'pending-removal') Object.assign(result,{text:'完成远程部署清理',level:'warn',status:'pendingRemoval'});
      else if (d?.state === 'settings-pending') Object.assign(result,{text:'重试同步账户配置',status:'pendingSync'});
      else if (d?.state === 'deployment-unconfirmed') Object.assign(result,{text:'检查目标机器的部署状态',status:'reconfirm'});
      else if (d?.changed) Object.assign(result,{text:'更新部署以应用草稿修改',status:'modified'});
      else if (d?.state === 'deployed') Object.assign(result,{text:d.enabled ? '实际执行结果见运行记录' : '启用后恢复定时运行',level:'info',status:d.enabled ? 'deployed' : 'stopped'});
      else if (d) Object.assign(result,{text:'前往任务页检查部署状态',status:'reconfirm'});
      return result;
    });
  }
  function panelRows() {
    const selected = AieyesUI.panelAccounts(settings(),snapshot()?.quotaOrder ?? []);
    const rows = accounts(true).map(a => {
      const status = a.archived ? 'archived' : !a.quotaEnabled ? 'paused' : !snapshot()?.quotaOrder ? 'reconfirm' : selected[a.provider]?.includes(accountKey(a)) ? 'selected' : 'excluded';
      return row(a.name,a.archived ? '恢复账户后可加入面板' : !a.quotaEnabled ? '开启“显示并查询额度”' : status === 'selected' ? '此账户将在面板显示' : '在账户设置中调整面板选择',accountTarget(a),a.archived || !a.quotaEnabled ? 'warn' : 'info',status);
    });
    return rows.length ? rows : [row('面板账户','添加账户后选择面板显示项',{section:'accounts'},'pending','missing')];
  }
  function priceRows() {
    const count = snapshot()?.pricingGaps?.length ?? 0;
    return [row('用量计价',!snapshot() ? '查看模型价格和用量分析' : count ? `补齐 ${count} 项模型价格` : '当前统计未发现缺价模型',{category:'prices'},count ? 'warn' : 'info',!snapshot() ? 'reconfirm' : count ? 'priceMissing' : 'noPriceGaps')];
  }
  function notificationRows() {
    return [row('全局重置提醒',`提前重置${settings().quotaResetNotifications?.unexpected !== false ? '开启' : '关闭'} · 到期重置${settings().quotaResetNotifications?.natural ? '开启' : '关闭'}`,general('notifications'),'info','configured','发送测试通知，检查系统是否允许显示。'),
      ...accounts().filter(a => a.quotaResetNotifications).map(a => row(a.name,'查看此账户的提醒规则',accountTarget(a),'info','custom'))];
  }
  const cards = [
    {"id":"usage","title":"查看用量与额度","detail":"采集日志统计用量，连接账户查询额度。","steps":["启用 Agent，选择机器和日志目录。","开启日志采集并添加账户。","登录或填写 API Key / Cookie。"],"help":"","actions":[["机器与目录",{"section":"machines"},"agents"],["账户与额度",{"section":"accounts"},"accounts"]],rows:usageRows},
    {"id":"network","title":"排查网络","detail":"分别检查本机应用网络和服务器账户连接。","steps":[],"help":"","actions":[],rows:networkRows},
    {"id":"refresh","title":"调整刷新频率","detail":"分别设置额度查询、日志同步和资源监控频率。","steps":["设置全局刷新间隔。","按需为账户设置独立查询间隔。"],"help":"账户间隔优先于全局设置。调整间隔不会加速当前请求；查询慢时请检查网络和登录状态。","actions":[["全局刷新设置",{"category":"general","section":"refresh"},"refresh"],["网络设置",{"category":"general","section":"network"}]],rows:refreshRows},
    {"id":"hosts","title":"连接服务器","detail":"通过 SSH 采集日志或监控服务器资源。","steps":["导入 SSH 配置或添加服务器。","按需开启资源监控和 Agent 采集。","在 Agent 中选择服务器及日志目录。"],"help":"SSH 跳板连接沿用 ProxyJump / ProxyCommand。账户代理在对应机器的前置命令中设置。","actions":[["添加 SSH 服务器",{"category":"hosts"},"hosts"],["选择采集目录",{"section":"machines"}]],rows:hostRows},
    {"id":"wakeups","title":"设置定时唤醒","detail":"按设定时间调用订阅模型。可用于帮助规划 5h 重置时间。","steps":["选择订阅账户与运行机器。","读取可用模型，设置每日时刻。","保存草稿，再部署任务。"],"help":"修改草稿后需更新部署。执行时间、启用状态和运行记录见任务页。","actions":[["定时唤醒设置",{"category":"wakeups"},"wakeups"]],rows:wakeupRows},
    {"id":"panel","title":"选择面板账户","detail":"选择要在本机面板展示的账户。","steps":["开启账户的额度显示与查询。","选择面板账户或使用自动模式。"],"help":"自动模式显示每类前 5 个账户。手动模式可留空；新增账户默认加入面板，可随时取消。","actions":[["调整面板账户",{"section":"accounts"},"panel"]],rows:panelRows},
    {"id":"prices","title":"补齐模型价格","detail":"通过日志用量和模型价格计算 API 等价成本。","steps":["补齐模型价格和名称映射。","同步价格后重新计价。"],"help":"额度估值见“估值与采样”，与实际余额分开显示。","actions":[["模型价格与映射",{"category":"prices"},"prices"],["估值与采样",{"section":"valuation"}]],rows:priceRows},
    {"id":"notifications","title":"设置额度提醒","detail":"订阅额度提前或到期重置时提醒。","steps":["选择全局或单账户提醒规则。","发送测试通知并检查系统设置。"],"help":"提醒依赖实时额度查询。退出应用或电脑休眠时暂停检测。","actions":[["通知设置",{"category":"general","section":"notifications"},"notifications"],["账户提醒设置",{"section":"accounts"}]],rows:notificationRows},
  ];
  const more = [
    ['多账户与切换','添加多个账户，管理各机器的登录与当前使用账户。',{section:'accounts'}],
    ['Codex API 连接','在 Codex 中添加 API 连接，按需切换登录方式。',{provider:'codex',section:'api'}],
    ['桌宠与会话','选择桌宠、调整大小和会话展示。',general('desktop')],
    ['外观','选择主题、强调色和图表配色。',general('appearance')],
    ['更新与关于','查看版本、检查更新及更新设置。',general('updates')],
  ];
  function networkHelp() {
    return (scene() !== 'ssh' ? `<p><strong>本机：</strong>检查应用代理及目录的独立代理。${action('网络设置',general('network'))}</p>` : '') + (scene() !== 'local' ? '<p><strong>服务器：</strong>在下方选择账户与机器，按需配置前置命令。</p><details class="setup-help" data-guide-help="network"><summary>服务器代理配置示例</summary><p>设置服务器可访问的代理地址：</p><pre class="setup-command"><code>export http_proxy=http://PROXY_HOST:PORT\nexport https_proxy=http://PROXY_HOST:PORT</code></pre><p>或加载已有代理脚本：</p><pre class="setup-command"><code>source /path/to/proxy</code></pre><p>127.0.0.1 指服务器自身。应用代理不会自动传给远端；ProxyJump / ProxyCommand 仅用于 SSH 连接。前置命令用于账户授权、切换、额度查询和定时唤醒。</p></details>' : '');
  }
  function render() {
    const previous = document.querySelector('.setup-guide') ? rememberView() : null;
    $('#title').textContent = '配置向导';
    const visible = cards.filter(c => c.id !== 'hosts' || scene() !== 'local');
    $('#content').innerHTML = `<div class="setup-guide"><section class="card setup-intro"><h2>你想用 Aieyes 做什么？</h2><p class="muted">选择目标完成配置，下方显示最近读取的状态。</p><div class="setup-toolbar"><label class="setup-scene">使用场景<select id="setup-scene">${[['local','仅本机'],['ssh','仅 SSH 服务器'],['mixed','本机与 SSH']].map(([v,t]) => option(v,t,scene())).join('')}</select></label><button type="button" data-guide-recheck ${loading ? 'disabled' : ''}>${loading ? '正在读取状态…' : '重新读取状态'}</button><button type="button" data-guide-finish>返回总览</button></div>${readErrors.length ? '<div class="setup-read-error" role="status">部分状态读取失败，请重试或打开对应设置。</div>' : ''}</section><div class="setup-goals">${visible.map(c => {
      const rows = c.rows(), warn = rows.filter(r => r.level === 'warn').length, pending = rows.filter(r => r.level === 'pending').length;
      const summary = warn ? `${warn} 项需检查` : pending ? `${pending} 项待配置或验证` : '查看当前状态';
      const expanded = progress.expanded?.[c.id] === true;
      return `<details class="card setup-step" data-guide-card="${c.id}" data-agent-detail="guide:${c.id}" ${expanded ? 'open' : ''}><summary><h3>${e(c.title)}</h3><p class="muted">${e(c.detail)}</p><small data-guide-level="${warn ? 'warn' : pending ? 'pending' : 'info'}">${e(summary)}</small></summary><div class="setup-answer">${c.id === 'network' ? networkHelp() : `<ol>${c.steps.map(text => `<li>${e(text)}</li>`).join('')}</ol>`}<h4>当前状态</h4>${rowsHTML(rows)}${c.id === 'usage' ? `<div class="setup-providers" aria-label="选择要接入的 Agent">${window.AieyesContract.agentProviders.map(provider => `<button type="button" data-guide-provider="${provider}">${providerMark(provider)}${e(providers[provider])}</button>`).join('')}</div>` : ''}<div class="actions">${c.actions.map(([label,target,id]) => action(label,target,id)).join('')}</div>${c.help ? `<details class="setup-help" data-guide-help="${c.id}"><summary>更多说明</summary><p>${e(c.help)}</p></details>` : ''}</div></details>`;
    }).join('')}</div><details class="card setup-more" data-guide-card="more" data-agent-detail="guide:more" ${progress.expanded?.more === true ? 'open' : ''}><summary><h3>更多功能</h3><p class="muted">多账户与切换、API 连接、桌宠与会话、外观和更新</p></summary>${more.map(([title,detail,target]) => `<div class="setup-more-item"><strong>${e(title)}</strong><p class="muted">${e(detail)}</p>${action('了解与设置',target)}</div>`).join('')}</details></div>`;
    window.AieyesSingleSelect?.mountAll($('#content'));
    if (previous) restoreView(previous);
  }
  async function loadObservations() {
    const current = ++revision;
    loading = true; readErrors = []; if (state.page === 'setup') render();
    const names = ['dashboard','wakeups'];
    const results = await Promise.allSettled([api('dashboard.summary'),api('wakeups.list')]);
    if (current !== revision) return;
    observations = {}; readErrors = [];
    results.forEach((result,i) => {
      if (result.status === 'fulfilled' && (i === 0 ? Array.isArray(result.value?.quotas) : Array.isArray(result.value))) observations[names[i]] = result.value;
      else readErrors.push(names[i]);
    });
    loading = false; if (state.page === 'setup') render();
  }
  async function open() { progress.seen = true; save(); active = true; observations = {}; readErrors = []; rememberPage(); captureSettingsForms(); state.page = 'setup'; window.render(); await loadObservations(); }
  function returnBar() {
    document.querySelector('#setup-return')?.remove();
    if (active && state.page === 'settings') {
      const bar = document.createElement('div'); bar.id = 'setup-return'; bar.className = 'setup-return';
      bar.innerHTML = '<button type="button" data-guide-open>配置向导</button>';
      $('#content').before(bar);
    }
  }
  document.addEventListener('toggle',event => {
    if (!event.target.matches?.('[data-guide-card]') || !event.target.isConnected) return;
    progress.expanded ??= {}; progress.expanded[event.target.dataset.guideCard] = event.target.open; save();
  },true);
  document.addEventListener('click',async event => {
    if (event.target.closest('[data-guide-open]')) { await open(); return; }
    if (event.target.closest('[data-guide-recheck]')) { await loadObservations(); return; }
    if (event.target.closest('[data-guide-finish]')) { active = false; progress.seen = true; save(); await navigatePage('agent'); return; }
    const provider = event.target.closest('[data-guide-provider]'), button = event.target.closest('[data-guide-target]');
    if (provider || button) {
      rememberPage();
      await openSettings(provider ? {provider:provider.dataset.guideProvider,section:'accounts'} : JSON.parse(button.dataset.guideTarget));
      returnBar();
    }
  });
  document.addEventListener('change',event => { if (event.target.id === 'setup-scene') { progress.scene = event.target.value; save(); render(); } });
  function maybeStart() {
    if (!state.settings || progress.seen) return;
    const s = state.settings, defaultsOnly = !s.accounts.length && !s.hosts.length && s.sources.every(source => source.id === 'local-' + source.provider) && (!s.proxy || s.proxy.mode === 'system');
    progress.seen = true; save(); if (defaultsOnly) void open();
  }
  window.AieyesSetup = {open,render,returnBar,maybeStart};
  window.AieyesApp.ready.then(maybeStart);
})();
