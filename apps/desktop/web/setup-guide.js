/* Goal-based help. Saved configuration and observed results are separate facts. */
(() => {
  if (PANEL) return;
  const key = 'aieyes.setup.v1',e = escapeHTML;
  const read = () => {
    try {
      const value = JSON.parse(localStorage.getItem(key) || '{}');
      if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
      return { seen: value.seen === true, scene: ['local', 'ssh', 'mixed'].includes(value.scene) ? value.scene : undefined,
        expanded: value.expanded && typeof value.expanded === 'object' && !Array.isArray(value.expanded) ? value.expanded : {} };
    } catch (_) {return {};}
  };
  let progress = read(),active = false,revision = 0,loading = false;
  let observations = {},readErrors = [];
  const save = () => {try {localStorage.setItem(key, JSON.stringify(progress));} catch (_) {}};
  const settings = () => state.settings;
  const scene = () => progress.scene ?? (settings().hosts.length ? settings().localDeviceEnabled === false ? 'ssh' : 'mixed' : 'local');
  const inScene = (source) => scene() === 'mixed' || (scene() === 'local' ? !source.hostId : Boolean(source.hostId));
  const machineName = (id) => id === 'local' ? AieyesI18n.t("本机") : settings().hosts.find((h) => h.id === id)?.name || settings().hosts.find((h) => h.id === id)?.target || AieyesI18n.t("已移除设备");
  const accountKey = (a) => a.provider + ':' + a.id;
  const linked = (a, s) => window.AieyesAgentSettings.linked(a, s);
  const sources = () => settings().sources.filter(inScene);
  const relevantSource = (source) => settings().agents?.find((a) => a.provider === source.provider)?.enabled ||
  settings().accounts.some((a) => !a.archived && a.provider === source.provider) ||
  source.id !== 'local-' + source.provider;
  const accountSources = (a) => settings().sources.filter((s) => linked(a, s));
  const accounts = (archived = false) => settings().accounts.filter((a) => (archived || !a.archived) && (!accountSources(a).length || accountSources(a).some(inScene)));
  const snapshot = () => readErrors.includes('dashboard') ? null : observations.dashboard ?? state.dashboard;
  const accountTarget = (a) => ({ accountKey: accountKey(a) });
  const general = (section) => ({ category: 'general', section });
  let statusLabels = { configured: AieyesI18n.t("已配置"), pending: AieyesI18n.t("待验证"), missing: AieyesI18n.t("待配置"), paused: AieyesI18n.t("已暂停"), queryFailed: AieyesI18n.t("查询失败"), querySucceeded: AieyesI18n.t("查询成功"), collectionFailed: AieyesI18n.t("采集异常"), collected: AieyesI18n.t("采集成功"), unconfigured: AieyesI18n.t("未配置"), reconfirm: AieyesI18n.t("待确认"), custom: AieyesI18n.t("单独设置"), inherited: AieyesI18n.t("沿用全局"), archived: AieyesI18n.t("已归档"), selected: AieyesI18n.t("已选择"), excluded: AieyesI18n.t("未选择"), noTasks: AieyesI18n.t("无任务"), draft: AieyesI18n.t("未部署"), pendingRemoval: AieyesI18n.t("待移除"), pendingSync: AieyesI18n.t("待同步"), modified: AieyesI18n.t("待部署"), deployed: AieyesI18n.t("已部署"), stopped: AieyesI18n.t("已停用"), missingSource: AieyesI18n.t("目录变化"), priceMissing: AieyesI18n.t("缺少价格"), noPriceGaps: AieyesI18n.t("未报缺价"), sampled: AieyesI18n.t("采样成功") };window.addEventListener("aieyes:language", () => {statusLabels = { configured: AieyesI18n.t("已配置"), pending: AieyesI18n.t("待验证"), missing: AieyesI18n.t("待配置"), paused: AieyesI18n.t("已暂停"), queryFailed: AieyesI18n.t("查询失败"), querySucceeded: AieyesI18n.t("查询成功"), collectionFailed: AieyesI18n.t("采集异常"), collected: AieyesI18n.t("采集成功"), unconfigured: AieyesI18n.t("未配置"), reconfirm: AieyesI18n.t("待确认"), custom: AieyesI18n.t("单独设置"), inherited: AieyesI18n.t("沿用全局"), archived: AieyesI18n.t("已归档"), selected: AieyesI18n.t("已选择"), excluded: AieyesI18n.t("未选择"), noTasks: AieyesI18n.t("无任务"), draft: AieyesI18n.t("未部署"), pendingRemoval: AieyesI18n.t("待移除"), pendingSync: AieyesI18n.t("待同步"), modified: AieyesI18n.t("待部署"), deployed: AieyesI18n.t("已部署"), stopped: AieyesI18n.t("已停用"), missingSource: AieyesI18n.t("目录变化"), priceMissing: AieyesI18n.t("缺少价格"), noPriceGaps: AieyesI18n.t("未报缺价"), sampled: AieyesI18n.t("采样成功") };});
  const row = (label, text, target, level = 'info', status = 'configured', details = '', updatedAt = 0) => ({ label, text, target, level, status, details, updatedAt });
  const stamp = (value) => new Date(value * 1000).toLocaleString(AieyesI18n.locale, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', ...AieyesI18n.clock });
  const action = (label, target, id) => `<button type="button" ${id ? `data-guide-step="${e(id)}"` : ''} data-guide-target="${e(JSON.stringify(target))}">${e(label)}</button>`;
  function destinationLabel(target = {}) {
    if (target.accountKey) return AieyesI18n.t("管理账户");
    const category = settingsCategory(target.category ?? 'integrations');
    if (category === 'hosts') return AieyesI18n.t("配置机器");
    if (category === 'prices') return AieyesI18n.t("设置价格");
    if (category === 'wakeups') return AieyesI18n.t("管理任务");
    return { accounts: AieyesI18n.t("管理账户"), machines: AieyesI18n.t("选择目录"), api: AieyesI18n.t("配置 API"), valuation: AieyesI18n.t("管理采样"), appearance: AieyesI18n.t("设置外观"), desktop: AieyesI18n.t("设置桌宠"), network: AieyesI18n.t("设置网络"), refresh: AieyesI18n.t("调整刷新"), notifications: AieyesI18n.t("设置通知"), updates: AieyesI18n.t("检查更新") }[target.section] ?? AieyesI18n.t("接入 Agent");
  }
  const rowsHTML = (rows) => `<ul class="setup-checks">${rows.map((r) => `<li data-guide-level="${r.level}" data-guide-state="${r.status}"><div><strong>${e(r.label)}</strong><div class="setup-check-status"><span class="setup-status-badge">${e(statusLabels[r.status])}</span><span>${e(r.text)}</span></div>${r.updatedAt > 0 ? `<time datetime="${new Date(r.updatedAt * 1000).toISOString()}">${AieyesI18n.t("上次数据：{0}", e(stamp(r.updatedAt)))}</time>` : ''}${r.details ? `<details class="setup-row-details"><summary>${AieyesI18n.t("状态说明")}</summary><p>${e(r.details)}</p></details>` : ''}</div>${r.target ? action(destinationLabel(r.target), r.target) : ''}</li>`).join('')}</ul>`;
  function quotaRow(a) {
    const available = accountSources(a).filter((s) => inScene(s) && AieyesUI.sourceEnabled(settings(), s));
    const q = snapshot()?.quotas?.find((q) => q.provider === a.provider && q.accountId === a.id);
    const result = row((providers[a.provider] ?? a.provider) + ' · ' + a.name, AieyesI18n.t("刷新账户额度以验证连接"), accountTarget(a), 'pending', 'pending');
    if (!a.quotaEnabled) Object.assign(result, { text: AieyesI18n.t("开启“显示并查询额度”"), level: 'warn', status: 'paused' });else
    if (!available.length) Object.assign(result, { text: AieyesI18n.t("关联账户并启用对应机器的采集"), level: 'warn', status: 'missing' });else
    if (q?.error) Object.assign(result, { text: AieyesI18n.t("查看账户错误后重试"), level: 'warn', status: 'queryFailed', details: AieyesI18n.t("仍显示上次数据时，请核对更新时间。"), updatedAt: q.updatedAt });else
    if (['live', 'command'].includes(q?.origin) && q.updatedAt > 0 && available.some((s) => s.id === q.sourceId)) Object.assign(result, { text: AieyesI18n.t("上次实时查询已完成"), level: 'ok', status: 'querySucceeded', updatedAt: q.updatedAt });
    return result;
  }
  function usageRows() {
    const rows = sources().filter((s) => s.provider !== 'deepseek' && relevantSource(s)).map((s) => {
      const status = snapshot()?.sources?.find((r) => r.id === s.id)?.status;
      const enabled = AieyesUI.sourceEnabled(settings(), s);
      const result = row((providers[s.provider] ?? s.provider) + ' · ' + machineName(s.hostId || 'local') + ' · ' + s.name, AieyesI18n.t("同步记录以验证日志采集"), { provider: s.provider, section: 'machines' }, 'pending', 'pending');
      if (!enabled) Object.assign(result, { text: AieyesI18n.t("检查机器、采集开关和目录绑定"), level: 'warn', status: 'missing' });else
      if (AieyesUI.syncFailure(status)) Object.assign(result, { text: AieyesI18n.t("检查日志目录和机器连接"), level: 'warn', status: 'collectionFailed' });else
      if (status?.updatedAt > 0) Object.assign(result, { text: AieyesI18n.t("上次日志采集已完成"), level: 'ok', status: 'collected', updatedAt: status.updatedAt });
      return result;
    });
    if (!rows.length) rows.push(row(AieyesI18n.t("用量采集"), AieyesI18n.t("选择 Agent、机器和日志目录"), { section: 'machines' }, 'pending', 'missing'));
    const a = accounts();
    return rows.concat(a.length ? a.map(quotaRow) : [row(AieyesI18n.t("账户与额度"), AieyesI18n.t("添加账户并完成认证"), { section: 'accounts' }, 'pending', 'missing')]);
  }
  function networkRows() {
    const rows = [];
    if (scene() !== 'ssh') {
      const modes = { system: AieyesI18n.t("跟随系统代理"), direct: AieyesI18n.t("直连"), custom: AieyesI18n.t("指定代理") };
      rows.push(row(AieyesI18n.t("本机应用网络"), AieyesI18n.t("{0} · 可测试出站连接", modes[settings().proxy?.mode] ?? AieyesI18n.t("跟随系统代理")), general('network')));
      for (const s of sources().filter((s) => !s.hostId && s.proxy && s.proxy.mode !== 'inherit')) rows.push(row((providers[s.provider] ?? s.provider) + ' · ' + s.name, AieyesI18n.t("此目录使用独立代理设置"), { provider: s.provider, section: 'machines' }, 'info', 'custom'));
    }
    if (scene() !== 'local') {
      if (!settings().hosts.length) rows.push(row(AieyesI18n.t("SSH 服务器"), AieyesI18n.t("添加服务器后配置账户网络"), { category: 'hosts' }, 'pending', 'missing'));
      const remote = accounts().flatMap((a) => [...new Set(accountSources(a).filter((s) => s.hostId).map((s) => s.hostId))].map((id) => ({ a, id })));
      if (!remote.length) rows.push(row(AieyesI18n.t("服务器账户网络"), AieyesI18n.t("连接账户后配置对应机器"), { section: 'accounts' }, 'pending', 'missing'));
      for (const { a, id } of remote) {
        const device = a.deviceSettings?.find((d) => d.machineId === id),needsConfirm = device?.preCommandResetRequired,configured = Boolean(device?.preCommand?.trim());
        rows.push(row(a.name + ' · ' + machineName(id), needsConfirm ? AieyesI18n.t("重新确认并保存前置命令") : configured ? AieyesI18n.t("刷新账户额度以检查连接") : AieyesI18n.t("需要代理时设置前置命令"),
        { ...accountTarget(a), machineId: id, section: 'command' }, needsConfirm ? 'warn' : 'info', needsConfirm ? 'reconfirm' : configured ? 'configured' : 'unconfigured', AieyesI18n.t("服务器可直连时无需填写；命令内容仅在账户设置中查看。")));
      }
    }
    return rows;
  }
  function refreshRows() {
    return [row(AieyesI18n.t("全局自动刷新"), AieyesI18n.t("额度 {0} 秒 · 记录 {1} 秒", settings().refreshSeconds, settings().historyRefreshSeconds ?? Math.max(10, settings().refreshSeconds)), general('refresh')),
    ...(scene() !== 'local' ? [row(AieyesI18n.t("服务器监控"), AieyesI18n.t("前台 {0} 秒 · 后台 {1} 秒", settings().serverForegroundRefreshSeconds ?? 2, settings().serverRefreshSeconds), general('refresh'))] : []),
    ...accounts().map((a) => row(a.name, AieyesI18n.t("每 {0} 秒查询", a.quotaRefreshSeconds ?? settings().refreshSeconds), accountTarget(a), 'info', a.quotaRefreshSeconds == null ? 'inherited' : 'custom'))];
  }
  function hostRows() {
    if (!settings().hosts.length) return [row(AieyesI18n.t("SSH 连接"), AieyesI18n.t("导入 SSH 配置或添加服务器"), { category: 'hosts' }, 'pending', 'missing')];
    return settings().hosts.map((h) => {
      const observed = state.hosts.find((r) => r.id === h.id);
      return row(h.name || h.target, AieyesI18n.t("Agent 采集{0} · 资源监控{1}", h.enabled ? AieyesI18n.t("已开启") : AieyesI18n.t("已暂停"), h.monitoringEnabled !== false ? AieyesI18n.t("已开启") : AieyesI18n.t("已暂停")), { category: 'hosts', hostId: h.id }, observed?.error ? 'warn' : 'info', observed?.error ? 'collectionFailed' : observed?.sample ? 'sampled' : 'pending', AieyesI18n.t("此状态来自资源监控；账户查询结果在账户页查看。"));
    });
  }
  function wakeupRows() {
    if (!observations.wakeups) return [row(AieyesI18n.t("任务状态"), readErrors.includes('wakeups') ? AieyesI18n.t("读取失败，请前往任务页重试") : AieyesI18n.t("前往任务页读取状态"), { category: 'wakeups' }, 'pending', 'reconfirm')];
    const records = observations.wakeups.filter((r) => {const source = settings().sources.find((s) => s.id === r.task.sourceId);return !source || inScene(source);});
    if (!records.length) return [row(AieyesI18n.t("定时任务"), AieyesI18n.t("按需创建并部署任务"), { category: 'wakeups' }, 'info', 'noTasks')];
    return records.map((r) => {
      const d = r.deployment,result = row(r.task.name, AieyesI18n.t("部署后才会自动运行"), { category: 'wakeups' }, 'pending', 'draft');
      if (!settings().sources.some((s) => s.id === r.task.sourceId)) Object.assign(result, { text: AieyesI18n.t("检查任务引用的日志目录"), level: 'warn', status: 'missingSource' });else
      if (d?.state === 'pending-removal') Object.assign(result, { text: AieyesI18n.t("完成远程部署清理"), level: 'warn', status: 'pendingRemoval' });else
      if (d?.state === 'settings-pending') Object.assign(result, { text: AieyesI18n.t("重试同步账户配置"), status: 'pendingSync' });else
      if (d?.state === 'deployment-unconfirmed') Object.assign(result, { text: AieyesI18n.t("检查目标机器的部署状态"), status: 'reconfirm' });else
      if (d?.changed) Object.assign(result, { text: AieyesI18n.t("更新部署以应用草稿修改"), status: 'modified' });else
      if (d?.state === 'deployed') Object.assign(result, { text: d.enabled ? AieyesI18n.t("实际执行结果见运行记录") : AieyesI18n.t("启用后恢复定时运行"), level: 'info', status: d.enabled ? 'deployed' : 'stopped' });else
      if (d) Object.assign(result, { text: AieyesI18n.t("前往任务页检查部署状态"), status: 'reconfirm' });
      return result;
    });
  }
  function panelRows() {
    const selected = AieyesUI.panelAccounts(settings(), snapshot()?.quotaOrder ?? []);
    const rows = accounts(true).map((a) => {
      const status = a.archived ? 'archived' : !a.quotaEnabled ? 'paused' : !snapshot()?.quotaOrder ? 'reconfirm' : selected[a.provider]?.includes(accountKey(a)) ? 'selected' : 'excluded';
      return row(a.name, a.archived ? AieyesI18n.t("恢复账户后可加入面板") : !a.quotaEnabled ? AieyesI18n.t("开启“显示并查询额度”") : status === 'selected' ? AieyesI18n.t("此账户将在面板显示") : AieyesI18n.t("在账户设置中调整面板选择"), accountTarget(a), a.archived || !a.quotaEnabled ? 'warn' : 'info', status);
    });
    return rows.length ? rows : [row(AieyesI18n.t("面板账户"), AieyesI18n.t("添加账户后选择面板显示项"), { section: 'accounts' }, 'pending', 'missing')];
  }
  function priceRows() {
    const count = snapshot()?.pricingGaps?.length ?? 0;
    return [row(AieyesI18n.t("用量计价"), !snapshot() ? AieyesI18n.t("查看模型价格和用量分析") : count ? AieyesI18n.t("补齐 {0} 项模型价格", count) : AieyesI18n.t("当前统计未发现缺价模型"), { category: 'prices' }, count ? 'warn' : 'info', !snapshot() ? 'reconfirm' : count ? 'priceMissing' : 'noPriceGaps')];
  }
  function notificationRows() {
    return [row(AieyesI18n.t("全局重置提醒"), AieyesI18n.t("提前重置{0} · 到期重置{1}", settings().quotaResetNotifications?.unexpected !== false ? AieyesI18n.t("开启") : AieyesI18n.t("关闭"), settings().quotaResetNotifications?.natural ? AieyesI18n.t("开启") : AieyesI18n.t("关闭")), general('notifications'), 'info', 'configured', AieyesI18n.t("发送测试通知，检查系统是否允许显示。")),
    ...accounts().filter((a) => a.quotaResetNotifications).map((a) => row(a.name, AieyesI18n.t("查看此账户的提醒规则"), accountTarget(a), 'info', 'custom'))];
  }
  const cards = [
  { "id": "usage", "title": AieyesI18n.t("查看用量与额度"), "detail": AieyesI18n.t("采集日志统计用量，连接账户查询额度。"), "steps": [AieyesI18n.t("启用 Agent，选择机器和日志目录。"), AieyesI18n.t("开启日志采集并添加账户。"), AieyesI18n.t("登录或填写 API Key / Cookie。")], "help": "", "actions": [[AieyesI18n.t("机器与目录"), { "section": "machines" }, "agents"], [AieyesI18n.t("账户与额度"), { "section": "accounts" }, "accounts"]], rows: usageRows },
  { "id": "network", "title": AieyesI18n.t("排查网络"), "detail": AieyesI18n.t("分别检查本机应用网络和服务器账户连接。"), "steps": [], "help": "", "actions": [], rows: networkRows },
  { "id": "refresh", "title": AieyesI18n.t("调整刷新频率"), "detail": AieyesI18n.t("分别设置额度查询、日志同步和资源监控频率。"), "steps": [AieyesI18n.t("设置全局刷新间隔。"), AieyesI18n.t("按需为账户设置独立查询间隔。")], "help": AieyesI18n.t("账户间隔优先于全局设置。调整间隔不会加速当前请求；查询慢时请检查网络和登录状态。"), "actions": [[AieyesI18n.t("全局刷新设置"), { "category": "general", "section": "refresh" }, "refresh"], [AieyesI18n.t("网络设置"), { "category": "general", "section": "network" }]], rows: refreshRows },
  { "id": "hosts", "title": AieyesI18n.t("连接服务器"), "detail": AieyesI18n.t("通过 SSH 采集日志或监控服务器资源。"), "steps": [AieyesI18n.t("导入 SSH 配置或添加服务器。"), AieyesI18n.t("按需开启资源监控和 Agent 采集。"), AieyesI18n.t("在 Agent 中选择服务器及日志目录。")], "help": AieyesI18n.t("SSH 跳板连接沿用 ProxyJump / ProxyCommand。账户代理在对应机器的前置命令中设置。"), "actions": [[AieyesI18n.t("添加 SSH 服务器"), { "category": "hosts" }, "hosts"], [AieyesI18n.t("选择采集目录"), { "section": "machines" }]], rows: hostRows },
  { "id": "wakeups", "title": AieyesI18n.t("设置定时唤醒"), "detail": AieyesI18n.t("按设定时间调用订阅模型。可用于帮助规划 5h 重置时间。"), "steps": [AieyesI18n.t("选择订阅账户与运行机器。"), AieyesI18n.t("读取可用模型，设置每日时刻。"), AieyesI18n.t("保存草稿，再部署任务。")], "help": AieyesI18n.t("修改草稿后需更新部署。执行时间、启用状态和运行记录见任务页。"), "actions": [[AieyesI18n.t("定时唤醒设置"), { "category": "wakeups" }, "wakeups"]], rows: wakeupRows },
  { "id": "panel", "title": AieyesI18n.t("选择面板账户"), "detail": AieyesI18n.t("选择要在本机面板展示的账户。"), "steps": [AieyesI18n.t("开启账户的额度显示与查询。"), AieyesI18n.t("选择面板账户或使用自动模式。")], "help": AieyesI18n.t("自动模式显示每类前 5 个账户。手动模式可留空；新增账户默认加入面板，可随时取消。"), "actions": [[AieyesI18n.t("调整面板账户"), { "section": "accounts" }, "panel"]], rows: panelRows },
  { "id": "prices", "title": AieyesI18n.t("补齐模型价格"), "detail": AieyesI18n.t("通过日志用量和模型价格计算 API 等价成本。"), "steps": [AieyesI18n.t("补齐模型价格和名称映射。"), AieyesI18n.t("同步价格后重新计价。")], "help": AieyesI18n.t("额度估值见“估值与采样”，与实际余额分开显示。"), "actions": [[AieyesI18n.t("模型价格与映射"), { "category": "prices" }, "prices"], [AieyesI18n.t("估值与采样"), { "section": "valuation" }]], rows: priceRows },
  { "id": "notifications", "title": AieyesI18n.t("设置额度提醒"), "detail": AieyesI18n.t("订阅额度提前或到期重置时提醒。"), "steps": [AieyesI18n.t("选择全局或单账户提醒规则。"), AieyesI18n.t("发送测试通知并检查系统设置。")], "help": AieyesI18n.t("提醒依赖实时额度查询。退出应用或电脑休眠时暂停检测。"), "actions": [[AieyesI18n.t("通知设置"), { "category": "general", "section": "notifications" }, "notifications"], [AieyesI18n.t("账户提醒设置"), { "section": "accounts" }]], rows: notificationRows }];

  const more = [
  [AieyesI18n.t("多账户与切换"), AieyesI18n.t("添加多个账户，管理各机器的登录与当前使用账户。"), { section: 'accounts' }],
  [AieyesI18n.t("Codex API 连接"), AieyesI18n.t("在 Codex 中添加 API 连接，按需切换登录方式。"), { provider: 'codex', section: 'api' }],
  [AieyesI18n.t("桌宠与会话"), AieyesI18n.t("选择桌宠、调整大小和会话展示。"), general('desktop')],
  [AieyesI18n.t("外观"), AieyesI18n.t("选择主题、强调色和图表配色。"), general('appearance')],
  [AieyesI18n.t("更新与关于"), AieyesI18n.t("查看版本、检查更新及更新设置。"), general('updates')]];

  function networkHelp() {
    return (scene() !== 'ssh' ? `<p><strong>${AieyesI18n.t("本机：")}</strong>${AieyesI18n.t("检查应用代理及目录的独立代理。{0}", action(AieyesI18n.t("网络设置"), general('network')))}</p>` : '') + (scene() !== 'local' ? `<p><strong>${AieyesI18n.t("服务器：")}</strong>${AieyesI18n.t("在下方选择账户与机器，按需配置前置命令。")}</p><details class="setup-help" data-guide-help="network"><summary>${AieyesI18n.t("服务器代理配置示例")}</summary><p>${AieyesI18n.t("设置服务器可访问的代理地址：")}</p><pre class="setup-command"><code>export http_proxy=http://PROXY_HOST:PORT
export https_proxy=http://PROXY_HOST:PORT</code></pre><p>${AieyesI18n.t("或加载已有代理脚本：")}</p><pre class="setup-command"><code>source /path/to/proxy</code></pre><p>${AieyesI18n.t("127.0.0.1 指服务器自身。应用代理不会自动传给远端；ProxyJump / ProxyCommand 仅用于 SSH 连接。前置命令用于账户授权、切换、额度查询和定时唤醒。")}</p></details>` : '');
  }
  function render() {
    const previous = document.querySelector('.setup-guide') ? rememberView() : null;
    $('#title').textContent = AieyesI18n.t("配置向导");
    const visible = cards.filter((c) => c.id !== 'hosts' || scene() !== 'local');
    $('#content').innerHTML = `<div class="setup-guide"><section class="card setup-intro"><h2>${AieyesI18n.t("你想用 Aieyes 做什么？")}</h2><div class="setup-toolbar"><label class="setup-scene">${AieyesI18n.t("使用场景")}<select id="setup-scene">${[['local', AieyesI18n.t("仅本机")], ['ssh', AieyesI18n.t("仅 SSH 服务器")], ['mixed', AieyesI18n.t("本机与 SSH")]].map(([v, t]) => option(v, t, scene())).join('')}</select></label><button type="button" data-guide-recheck ${loading ? 'disabled' : ''}>${loading ? AieyesI18n.t("正在读取状态…") : AieyesI18n.t("重新读取状态")}</button><button type="button" data-guide-finish>${AieyesI18n.t("返回总览")}</button></div>${readErrors.length ? `<div class="setup-read-error" role="status">${AieyesI18n.t("部分状态读取失败，请重试或打开对应设置。")}</div>` : ''}</section><div class="setup-goals">${visible.map((c) => {
      const rows = c.rows(),warn = rows.filter((r) => r.level === 'warn').length,pending = rows.filter((r) => r.level === 'pending').length;
      const summary = warn ? AieyesI18n.t("{0} 项需检查", warn) : pending ? AieyesI18n.t("{0} 项待配置或验证", pending) : AieyesI18n.t("查看当前状态");
      const expanded = progress.expanded?.[c.id] !== false;
      return `<details class="card setup-step" data-guide-card="${c.id}" data-agent-detail="guide:${c.id}" ${expanded ? 'open' : ''}><summary><h3>${e(c.title)}</h3><small data-guide-level="${warn ? 'warn' : pending ? 'pending' : 'info'}">${e(summary)}</small></summary><div class="setup-answer">${rowsHTML(rows)}${c.id === 'usage' ? `<div class="setup-providers" aria-label="${AieyesI18n.t("选择要接入的 Agent")}">${window.AieyesContract.agentProviders.map((provider) => `<button type="button" data-guide-provider="${provider}">${providerMark(provider)}${e(providers[provider])}</button>`).join('')}</div>` : ''}<div class="actions">${c.actions.map(([label, target, id]) => action(label, target, id)).join('')}</div>${c.id === 'network' ? networkHelp() : `<details class="setup-help" data-guide-help="${c.id}"><summary>${AieyesI18n.t("配置步骤")}</summary><ol>${c.steps.map((text) => `<li>${e(text)}</li>`).join('')}</ol>${c.help ? `<p>${e(c.help)}</p>` : ''}</details>`}</div></details>`;
    }).join('')}</div><details class="card setup-more" data-guide-card="more" data-agent-detail="guide:more" ${progress.expanded?.more === true ? 'open' : ''}><summary><h3>${AieyesI18n.t("更多功能")}</h3></summary>${more.map(([title, detail, target]) => `<div class="setup-more-item"><strong>${e(title)}</strong>${action(destinationLabel(target), target)}</div>`).join('')}</details></div>`;
    window.AieyesSingleSelect?.mountAll($('#content'));
    if (previous) restoreView(previous);
  }
  async function loadObservations() {
    const current = ++revision;
    loading = true;readErrors = [];if (state.page === 'setup') render();
    const names = ['dashboard', 'wakeups'];
    const results = await Promise.allSettled([api('dashboard.summary'), api('wakeups.list')]);
    if (current !== revision) return;
    observations = {};readErrors = [];
    results.forEach((result, i) => {
      if (result.status === 'fulfilled' && (i === 0 ? Array.isArray(result.value?.quotas) : Array.isArray(result.value))) observations[names[i]] = result.value;else
      readErrors.push(names[i]);
    });
    loading = false;if (state.page === 'setup') render();
  }
  async function open() {progress.seen = true;save();active = true;observations = {};readErrors = [];rememberPage();captureSettingsForms();state.page = 'setup';window.render();await loadObservations();}
  function returnBar() {
    document.querySelector('#setup-return')?.remove();
    if (active && state.page === 'settings') {
      const bar = document.createElement('div');bar.id = 'setup-return';bar.className = 'setup-return';
      bar.innerHTML = `<button type="button" data-guide-open>${AieyesI18n.t("配置向导")}</button>`;
      $('#content').before(bar);
    }
  }
  document.addEventListener('toggle', (event) => {
    if (!event.target.matches?.('[data-guide-card]') || !event.target.isConnected) return;
    progress.expanded ??= {};progress.expanded[event.target.dataset.guideCard] = event.target.open;save();
  }, true);
  document.addEventListener('click', async (event) => {
    if (event.target.closest('[data-guide-open]')) {event.target.closest('.workspace-help')?.removeAttribute('open');await open();return;}
    if (event.target.closest('[data-guide-recheck]')) {await loadObservations();return;}
    if (event.target.closest('[data-guide-finish]')) {active = false;progress.seen = true;save();await navigatePage('agent');return;}
    const provider = event.target.closest('[data-guide-provider]'),button = event.target.closest('[data-guide-target]');
    if (provider || button) {
      rememberPage();
      await openSettings(provider ? { provider: provider.dataset.guideProvider, section: 'accounts' } : JSON.parse(button.dataset.guideTarget));
      returnBar();
    }
  });
  document.addEventListener('change', (event) => {if (event.target.id === 'setup-scene') {progress.scene = event.target.value;save();render();}});
  function maybeStart() {
    if (!state.settings || progress.seen) return;
    const s = state.settings,defaultsOnly = !s.accounts.length && !s.hosts.length && s.sources.every((source) => source.id === 'local-' + source.provider) && (!s.proxy || s.proxy.mode === 'system');
    progress.seen = true;save();if (defaultsOnly) void open();
  }
  window.AieyesSetup = { open, render, returnBar, maybeStart };
  window.AieyesApp.ready.then(maybeStart);
})();
