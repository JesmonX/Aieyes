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
  const row = (label,text,target,level = 'info') => ({label,text,target,level});
  const stamp = value => value > 0 ? new Date(value * 1000).toLocaleString() : '时间未知';
  const action = (label,target,id) => `<button type="button" ${id ? `data-guide-step="${e(id)}"` : ''} data-guide-target="${e(JSON.stringify(target))}">${e(label)}</button>`;
  const rowsHTML = rows => `<ul class="setup-checks">${rows.map(r => `<li data-guide-level="${r.level}"><div><strong>${e(r.label)}</strong><span>${e(r.text)}</span></div>${r.target ? action('查看设置',r.target) : ''}</li>`).join('')}</ul>`;
  function quotaRow(a) {
    const available = accountSources(a).filter(s => inScene(s) && AieyesUI.sourceEnabled(settings(),s));
    const q = snapshot()?.quotas?.find(q => q.provider === a.provider && q.accountId === a.id);
    let text = '已配置，限额待验证', level = 'pending';
    if (!a.quotaEnabled) { text = '尚未开启“显示并查询额度”'; level = 'warn'; }
    else if (!available.length) { text = '待配置：关联账户并启用此场景的 Agent、机器和目录'; level = 'warn'; }
    else if (q?.error) { text = '上次查询失败，请查看账户错误；旧数据不代表当前连接正常'; level = 'warn'; }
    else if (['live','command'].includes(q?.origin) && q.updatedAt > 0 && available.some(s => s.id === q.sourceId)) { text = '上次查询成功 · ' + stamp(q.updatedAt); level = 'ok'; }
    return row((providers[a.provider] ?? a.provider) + ' · ' + a.name,text,accountTarget(a),level);
  }
  function usageRows() {
    const rows = sources().filter(s => s.provider !== 'deepseek' && relevantSource(s)).map(s => {
      const status = snapshot()?.sources?.find(r => r.id === s.id)?.status;
      const enabled = AieyesUI.sourceEnabled(settings(),s);
      return row((providers[s.provider] ?? s.provider) + ' · ' + machineName(s.hostId || 'local') + ' · ' + s.name,
        !enabled ? '待配置：检查 Agent、机器选择、采集开关及目录绑定' : status?.error || status?.partial ? '采集存在异常，请检查目录和连接' : status?.updatedAt > 0 ? '上次采集成功 · ' + stamp(status.updatedAt) : '目录已配置，采集待验证',
        {provider:s.provider,section:'machines'},!enabled || status?.error || status?.partial ? 'warn' : status?.updatedAt > 0 ? 'ok' : 'pending');
    });
    if (!rows.length) rows.push(row('用量采集','需要用量统计时，启用 Agent 并选择机器和日志目录；DeepSeek 余额无需日志目录',{section:'machines'},'pending'));
    const a = accounts();
    return rows.concat(a.length ? a.map(quotaRow) : [row('账户与限额','待配置：按 Agent 添加账户并连接登录；支持的登录方式以账户页为准',{section:'accounts'},'pending')]);
  }
  function networkRows() {
    const rows = [];
    if (scene() !== 'ssh') {
      const modes = {system:'跟随系统代理',direct:'直连',custom:'指定代理'};
      rows.push(row('本机应用网络',(modes[settings().proxy?.mode] ?? '跟随系统代理') + ' · 到网络代理页测试出站连接',general('network')));
      for (const s of sources().filter(s => !s.hostId && s.proxy && s.proxy.mode !== 'inherit')) rows.push(row((providers[s.provider] ?? s.provider) + ' · ' + s.name,'此本机来源覆盖应用代理，请在目录设置中核对',{provider:s.provider,section:'machines'}));
    }
    if (scene() !== 'local') {
      if (!settings().hosts.length) rows.push(row('SSH 服务器','待配置：先添加服务器，再配置账户在目标机器上的网络',{category:'hosts'},'pending'));
      const remote = accounts().flatMap(a => [...new Set(accountSources(a).filter(s => s.hostId).map(s => s.hostId))].map(id => ({a,id})));
      if (!remote.length) rows.push(row('服务器账户网络','添加或连接账户后，在该账户的对应机器设置前置命令',{section:'accounts'},'pending'));
      for (const {a,id} of remote) {
        const device = a.deviceSettings?.find(d => d.machineId === id);
        rows.push(row(a.name + ' · ' + machineName(id),device?.preCommandResetRequired ? '前置命令需重新确认，请在账户设备设置中保存' : device?.preCommand?.trim() ? '前置命令已保存；联网效果需通过账户查询或运行结果确认' : '未设置前置命令；服务器可直连时无需填写，需要代理时在此配置',
          {...accountTarget(a),machineId:id,section:'command'},device?.preCommandResetRequired ? 'warn' : 'info'));
      }
    }
    return rows;
  }
  function refreshRows() {
    return [row('全局自动刷新',`限额 ${settings().refreshSeconds} 秒 · 记录 ${settings().historyRefreshSeconds ?? Math.max(10,settings().refreshSeconds)} 秒`,general('refresh')),
      ...(scene() !== 'local' ? [row('服务器监控间隔',`前台 ${settings().serverForegroundRefreshSeconds ?? 2} 秒 · 后台 ${settings().serverRefreshSeconds} 秒`,general('refresh'))] : []),
      ...accounts().map(a => row(a.name,a.quotaRefreshSeconds != null ? `此账户每 ${a.quotaRefreshSeconds} 秒查询一次，覆盖全局设置` : `沿用全局限额间隔：${settings().refreshSeconds} 秒`,accountTarget(a)))];
  }
  function hostRows() {
    if (!settings().hosts.length) return [row('SSH 连接','待配置：导入 SSH config 或手动添加服务器',{category:'hosts'},'pending')];
    return settings().hosts.map(h => {
      const observed = state.hosts.find(r => r.id === h.id);
      return row(h.name || h.target,`Agent 采集${h.enabled ? '已开启' : '已暂停'} · 资源监控${h.monitoringEnabled !== false ? '已开启' : '已暂停'} · ` + (observed?.error ? '上次监控连接失败' : observed?.sample ? '上次监控采样成功（不代表 Agent 联网正常）' : '连接待验证'),{category:'hosts',hostId:h.id},observed?.error ? 'warn' : 'info');
    });
  }
  function wakeupRows() {
    if (!observations.wakeups) return [row('任务状态',readErrors.includes('wakeups') ? '状态读取失败，请前往定时唤醒页查看并重试' : '任务状态待确认',{category:'wakeups'},'pending')];
    const records = observations.wakeups.filter(r => { const source = settings().sources.find(s => s.id === r.task.sourceId); return !source || inScene(source); });
    if (!records.length) return [row('定时任务','尚无此场景的任务；需要时创建并部署',{category:'wakeups'},'info')];
    return records.map(r => {
      const d = r.deployment;
      let text = '已保存草稿，尚未部署', level = 'pending';
      if (!settings().sources.some(s => s.id === r.task.sourceId)) { text = '任务引用的目录已变化，请到任务页检查'; level = 'warn'; }
      else if (d?.state === 'pending-removal') { text = '部署待移除，请在任务页完成清理'; level = 'warn'; }
      else if (d?.state === 'settings-pending') text = '部署待同步，请在任务页重试';
      else if (d?.state === 'deployment-unconfirmed') text = '部署待确认，请检查任务状态';
      else if (d?.changed) text = '草稿已修改，需要更新部署';
      else if (d?.state === 'deployed') { text = d.enabled ? '已保存启用的部署，实际执行请查看运行记录' : '部署已停用'; level = 'info'; }
      else if (d) text = '部署状态待确认，请查看任务页';
      return row(r.task.name,text,{category:'wakeups'},level);
    });
  }
  function panelRows() {
    const selected = AieyesUI.panelAccounts(settings(),snapshot()?.quotaOrder ?? []);
    const rows = accounts(true).map(a => row(a.name,a.archived ? '账户已归档，恢复后才能显示在面板' : !a.quotaEnabled ? '尚未开启“显示并查询额度”' : !snapshot()?.quotaOrder ? '面板选择待确认，请前往账户设置查看' : selected[a.provider]?.includes(accountKey(a)) ? '已选入面板' : '未选入面板，请调整账户选择',accountTarget(a),a.archived || !a.quotaEnabled ? 'warn' : 'info'));
    return rows.length ? rows : [row('面板账户','尚无此场景的账户，请先添加账户',{section:'accounts'},'pending')];
  }
  const cards = [
    {id:'usage',title:'我想查看 Agent 用量和限额',detail:'用量需要采集日志；限额需要连接账户并开启查询。',steps:['启用要使用的 Agent，选择本机或服务器，再核对日志目录及采集开关。','在该 Agent 下添加账户，使用已有登录、引导登录或填写 API Key / Cookie，并连接到对应机器。DeepSeek 余额通过本机 API Key 查询，无需日志目录。'],verify:'手动同步记录、刷新限额，分别检查采集状态、账户错误和更新时间。支持能力以对应 Agent 页面为准。',rows:usageRows,actions:[['机器与目录',{section:'machines'},'agents'],['账户与限额',{section:'accounts'},'accounts']]},
    {id:'network',title:'国外 Agent 查询失败，或唤醒连不上网络？',detail:'本机检查应用代理；服务器在账户设备前置命令中配置代理。',steps:[],verify:'本机先在网络代理页测试出站连接，再刷新具体账户限额。服务器先确认 SSH 可连接，再查询该账户；已有唤醒任务可查看运行记录。出站测试和 SSH 成功都不能代表所有 Agent 可联网。',rows:networkRows,actions:[]},
    {id:'refresh',title:'限额怎么很久才更新？',detail:'调整自动查询间隔；单次请求耗时长时，先检查网络和登录。',steps:['在“刷新与历史”调整全局限额间隔；账户的查询间隔会覆盖全局值，留空才沿用全局。','记录采集、限额查询和服务器监控有各自的间隔。缩短限额间隔不会加速正在进行的请求；查询慢或超时请检查网络、登录及目标机器执行环境。'],verify:'保存后查看账户实际采用的间隔；可以手动刷新一次，确认查询结果和更新时间。',rows:refreshRows,actions:[['调整全局刷新',general('refresh'),'refresh'],['检查网络',general('network')]]},
    {id:'hosts',title:'我想采集服务器 Agent，或查看服务器资源',detail:'添加 SSH 主机；Agent 采集和资源监控可以分别开启。',steps:['导入 SSH config 或手动添加主机，确认身份验证和 SSH 连接可用。跳板连接沿用 SSH 配置中的 ProxyJump / ProxyCommand。','查看资源时开启资源监控；采集 Agent 时还需开启 Agent 采集，并在 Agent 的机器与目录中选择服务器。远端 Agent 联网另行配置账户前置命令。'],verify:'连接并读取设备，在服务器页检查资源数据；在 Agent 总览分别检查记录采集和限额查询。',rows:hostRows,actions:[['配置 SSH 主机',{category:'hosts'},'hosts'],['选择采集目录',{section:'machines'}]]},
    {id:'wakeups',title:'我想每天自动唤醒订阅',detail:'确认账户、执行机器和网络，再保存任务并部署。',steps:['选择支持唤醒的订阅账户与执行机器，确认目标机器的 CLI 已登录且能联网；服务器代理使用该账户在此机器上的前置命令。','读取模型与执行能力，选择模型和每天时刻；按目标机器时区执行。保存草稿后仍需部署，修改草稿后需要更新部署。','退出 Aieyes 后已部署任务仍可运行，但不会唤醒休眠电脑，离线或错过时刻不补发。'],verify:'部署后检查启用状态与运行记录。需要测试时主动点击“立即运行”，这会实际调用模型并可能消耗额度。',rows:wakeupRows,actions:[['配置定时唤醒',{category:'wakeups'},'wakeups']]},
    {id:'panel',title:'配置了账户，为什么面板没显示？',detail:'检查账户归档、额度显示开关及面板账户选择。',steps:['账户必须未归档，并开启“显示并查询额度”。在账户设置中调整面板选择。','自动模式显示每种 Agent 排序前五个账户；手动模式可以选择任意数量或留空，新增账户不会自动加入手动选择。'],verify:'打开面板确认账户是否出现；面板选中不代表该账户最近一次查询成功。',rows:panelRows,actions:[['调整面板账户',{section:'accounts'},'panel']]},
    {id:'prices',title:'有用量却没有金额，或提示未计价？',detail:'先补齐模型价格和映射，需要时再了解额度估值与采样。',steps:['在模型价格中补齐有用量的模型价格；模型名称不同的情况配置映射，再同步价格或重新计价。','API 等价成本由日志用量和模型价格计算；订阅额度价值需要估值或采样，结果不是可兑换余额。'],verify:'在用量分析中检查未计价提示和金额。仅限额查询成功不能证明用量计价完整。',rows:() => [row('用量计价',!snapshot() ? '计价状态待确认，请前往模型价格和用量分析查看' : snapshot().pricingGaps?.length ? `当前统计存在 ${snapshot().pricingGaps.length} 项价格缺口` : '当前统计未报告价格缺口；请结合实际用量确认',{category:'prices'},snapshot()?.pricingGaps?.length ? 'warn' : 'info')],actions:[['模型价格与映射',{category:'prices'},'prices'],['了解估值与采样',{section:'valuation'}]]},
    {id:'notifications',title:'我想在额度重置时收到提醒',detail:'配置提醒规则、账户覆盖项，并验证系统通知。',steps:['在通知设置中选择非预期重置、自然重置提醒；账户可单独覆盖全局规则。额度一直满额时不会提醒。','允许 Aieyes 发送系统通知，并检查勿扰模式。提醒依赖实时限额查询；退出应用或电脑休眠时暂停检测。'],verify:'在通知页发送测试通知确认系统可显示，再检查账户查询是否正常。',rows:() => [row('全局重置提醒',`非预期重置${settings().quotaResetNotifications?.unexpected !== false ? '开启' : '关闭'} · 自然重置${settings().quotaResetNotifications?.natural ? '开启' : '关闭'} · 系统权限待测试`,general('notifications')),...accounts().filter(a => a.quotaResetNotifications).map(a => row(a.name,'此账户设置了单独的提醒规则，请核对覆盖项',accountTarget(a)))],actions:[['通知规则与测试',general('notifications'),'notifications'],['账户提醒覆盖',{section:'accounts'}]]},
  ];
  const more = [
    ['多账户与切换','同一 Agent 可添加多个账户并连接不同机器；在账户页区分“已登录”和“正在使用”，按需切换。',{section:'accounts'}],
    ['Codex API 连接','需要通过 API 使用 Codex 时，在 Codex 的 API 连接区域配置；与订阅账户分开管理。',{provider:'codex',section:'api'}],
    ['桌宠与会话','配置桌宠资源、大小和会话展示；要看账户限额，请先选择面板账户。',general('desktop')],
    ['外观','选择主题、强调色和图表配色。',general('appearance')],
    ['更新与关于','查看版本、检查更新及更新设置。',general('updates')],
  ];
  function networkHelp() {
    return (scene() !== 'ssh' ? '<p><strong>本机：</strong>进入网络代理，选择系统代理、直连或指定代理。Agent 的本机目录可单独覆盖应用代理，需要一起核对。</p>' : '') + (scene() !== 'local' ? '<p><strong>服务器：</strong>进入“账户 → 对应机器 → 前置命令”，按远端实际网络填写以下一种方式。只有需要代理时才配置，服务器可直连时可以留空。</p><pre class="setup-command"><code>export http_proxy=http://PROXY_HOST:PORT\nexport https_proxy=http://PROXY_HOST:PORT</code></pre><p class="muted">或加载服务器上已有的代理脚本：</p><pre class="setup-command"><code>source /path/to/proxy</code></pre><p>代理地址必须在服务器上可达；服务器中的 <code>127.0.0.1</code> 指服务器自身。SSH 的 ProxyJump / ProxyCommand 只决定 SSH 如何连接，应用代理不会自动传给远端 Agent。前置命令用于此账户在此机器上的授权、切换、额度查询及定时唤醒。</p>' : '');
  }
  function render() {
    const previous = document.querySelector('.setup-guide') ? rememberView() : null;
    $('#title').textContent = '配置向导';
    const visible = cards.filter(c => c.id !== 'hosts' || scene() !== 'local');
    $('#content').innerHTML = `<div class="setup-guide"><section class="card setup-intro"><h2>你想用 Aieyes 做什么？</h2><p class="muted">找到你的目标或问题，查看还需要哪些配置。状态来自已保存配置和上次观察结果，保存过设置不代表连接已验证。</p><div class="setup-toolbar"><label class="setup-scene">使用场景<select id="setup-scene">${[['local','仅本机'],['ssh','仅 SSH 服务器'],['mixed','本机与 SSH']].map(([v,t]) => option(v,t,scene())).join('')}</select></label><button type="button" data-guide-recheck ${loading ? 'disabled' : ''}>${loading ? '正在读取状态…' : '重新读取状态'}</button><button type="button" data-guide-finish>返回总览</button></div>${readErrors.length ? '<p class="setup-read-error" role="status">部分状态读取失败，相关项目待确认。可以重新读取，或前往对应页面查看。</p>' : ''}</section><div class="setup-goals">${visible.map(c => {
      const rows = c.rows(), warn = rows.filter(r => r.level === 'warn').length, pending = rows.filter(r => r.level === 'pending').length;
      const summary = warn ? `${warn} 项配置或异常需检查` : pending ? `${pending} 项待配置或验证` : '查看当前配置与验证方法';
      const expanded = progress.expanded?.[c.id] === true;
      return `<details class="card setup-step" data-guide-card="${c.id}" data-agent-detail="guide:${c.id}" ${expanded ? 'open' : ''}><summary><h3>${e(c.title)}</h3><p class="muted">${e(c.detail)}</p><small data-guide-level="${warn ? 'warn' : pending ? 'pending' : 'info'}">${e(summary)}</small></summary><div class="setup-answer">${c.id === 'network' ? networkHelp() : `<ol>${c.steps.map(text => `<li>${e(text)}</li>`).join('')}</ol>`}<h4>当前配置</h4>${rowsHTML(rows)}${c.id === 'usage' ? `<div class="setup-providers" aria-label="选择要接入的 Agent">${Object.keys(providers).map(provider => `<button type="button" data-guide-provider="${provider}">${providerMark(provider)}${e(providers[provider])}</button>`).join('')}</div>` : ''}<div class="actions">${c.actions.map(([label,target,id]) => action(label,target,id)).join('')}</div><p class="setup-verify"><strong>怎样确认生效：</strong>${e(c.verify)}</p></div></details>`;
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
      bar.innerHTML = '<button type="button" data-guide-open>← 返回配置向导</button><span class="muted">保存后返回，查看配置和验证状态</span>';
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
