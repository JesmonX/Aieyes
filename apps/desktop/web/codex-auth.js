/* Credentials remain in the selected home; this view only receives profile metadata. */
(() => {
  const e = (value) => escapeHTML(value ?? '');
  let intent = 'login',needsRefresh = false,targetAccountId = '',sourceId = '',info = null,busy = false,error = '',notice = '',prepared = null,operation = null,timer;
  let switchReview = null,switchError = '',switchedReview = null,prepareOnOpen = false;
  const savedSources = () => (state.settings?.sources ?? []).filter((s) => s.provider === 'codex' && AieyesUI.sourceEnabled(state.settings, s));
  const selected = () => savedSources().find((s) => s.id === sourceId);
  const targetAccount = () => state.settings.accounts.find((a) => a.provider === 'codex' && a.id === targetAccountId);
  const paint = () => {if (window.AieyesAccountTargets?.paintAuth()) return;if (state.page === 'settings' && state.settingsTab === 'integrations') renderSettings(false);};
  async function call(method, params = {}) {return api('codexAuth.' + method, { sourceId, accountId: targetAccountId, ...params });}
  async function settingsChanged() {
    window.AieyesAgentSettings.accept(await api('settings.get'));
    void loadDashboard();
  }
  async function refresh() {
    const requestedSource = sourceId,requestedAccount = targetAccountId;
    const loaded = await call('inspect');
    if (sourceId !== requestedSource || targetAccountId !== requestedAccount) {needsRefresh = true;return;}
    info = loaded;
  }
  async function prepareSwitch(profileId) {
    const profile = info?.profiles.find((p) => p.id === profileId);
    if (!profile) return;
    const review = { sourceId, accountId: targetAccountId, profileId, name: profile.name, currentName: info.selected?.kind === 'api' ? info.selected.name || AieyesI18n.t("当前 API") : info.profiles.find((p) => p.current)?.name || AieyesI18n.t("当前登录"), sourceName: selected()?.name || '', path: selected()?.path || '' };
    switchReview = review;switchedReview = null;prepared = null;switchError = '';paint();
    try {
      const result = await call('switch.prepare', { profileId });
      if (switchReview === review && sourceId === review.sourceId) prepared = result;
    } catch (err) {if (switchReview === review) switchError = err.message ?? String(err);}
  }
  async function refreshAfterSwitch(review) {
    switchedReview = review;
    try {
      if (review.settingsPending) {await settingsChanged();review.settingsPending = false;}
      await refresh();
      const current = info?.profiles.find((p) => p.id === review.profileId)?.current === true;
      const updated = await window.AieyesAgentSettings.refreshCodexSource(review.sourceId);
      if (current) {
        notice = AieyesI18n.t("账户已切换，请按需重新打开 Codex 或 ChatGPT 并恢复会话");
        if (!updated) error = AieyesI18n.t("账户已切换，设备状态刷新失败，请重试读取。");else
        if (review.warning) error = review.warning;
      } else {
        notice = '';error = AieyesI18n.t("当前登录与目标账户不一致，请重新检查。");
      }
    } catch (err) {
      notice = AieyesI18n.t("切换已提交，当前账户待确认。");error = AieyesI18n.t("状态读取失败：") + (err.message ?? String(err));
    }
  }
  async function commitSwitch() {
    const review = switchReview,checked = prepared;
    if (!review || !checked || review.sourceId !== sourceId || review.accountId !== targetAccountId) return;
    try {
      const result = await call('switch.commit', { operationId: checked.operationId, closeProcesses: (checked.processes ?? []).length > 0 });
      if (result.status !== 'succeeded') throw new Error(result.error || AieyesI18n.t("切换尚未完成，请重新检查"));
      prepared = null;switchReview = null;
      review.settingsPending = Boolean(result.accountId);review.warning = result.statusWarning;
      await refreshAfterSwitch(review);
    } catch (err) {prepared = null;switchError = err.message ?? String(err);}
  }
  function switchDialog() {
    if (!switchReview) return '';
    const processes = prepared?.processes ?? [],blocked = processes.some((p) => !p.canClose);
    return `<dialog id="codex-switch" aria-labelledby="codex-switch-title" aria-describedby="codex-switch-summary">
      <div class="dialog-head"><h2 id="codex-switch-title">${AieyesI18n.t("确认切换账户")}</h2></div>
      <div class="codex-switch-body">
        <p id="codex-switch-summary"><strong>${e(switchReview.currentName)} → ${e(switchReview.name)}</strong></p>
        <p class="muted">${e(switchReview.sourceName)} · ${e(switchReview.path)}</p>
        <div class="warning">${AieyesI18n.t("确认后将关闭下面列出的受影响进程，包含使用当前默认 API 的实例；已确认使用独立 API 配置的实例及其辅助进程会保留。请先保存工作。")}</div>
        ${switchError ? `<div class="error" role="alert">${e(switchError)}</div>` : ''}
        ${busy ? `<p role="status">${prepared ? AieyesI18n.t("正在切换…") : AieyesI18n.t("正在检查…")}</p>` : ''}
        ${prepared?.preservedProcesses?.length ? `<p class="muted">${AieyesI18n.t("将保留 {0} 个使用独立 API 凭据的进程（含辅助进程）。", prepared.preservedProcesses.length)}</p>` : ''}
        ${prepared ? `<div ${blocked ? 'class="error" role="alert"' : ''}>${blocked ? AieyesI18n.t("暂时无法切换，请先处理以下进程，再重新检查。") : processes.length ? AieyesI18n.t("确认后将关闭以下进程及应用。") : AieyesI18n.t("没有需要关闭的 Codex 或 ChatGPT 进程。")}</div>
        <ul class="codex-switch-processes">${processes.map((p) => `<li>${e(p.name)} · PID ${p.pid}${p.parentPid ? `<small>${AieyesI18n.t("父进程 PID {0}", p.parentPid)}</small>` : ''}${p.canClose ? '' : `<small class="error">${e(p.blockingReason || AieyesI18n.t("无法确认归属，请手动关闭后重新检查"))}</small>`}</li>`).join('')}</ul>` : ''}
        <p class="muted">${AieyesI18n.t("切换后请按需手动重新打开 Codex 或 ChatGPT 并恢复会话。")}</p>
      </div>
      <div class="dialog-foot"><button data-auth="cancelSwitch" ${busy ? 'disabled' : ''} autofocus>${AieyesI18n.t("取消")}</button><button data-auth="retrySwitch" ${busy ? 'disabled' : ''}>${AieyesI18n.t("重新检查")}</button><button class="primary" data-auth="commit" ${busy || !prepared || blocked ? 'disabled' : ''}>${processes.length ? AieyesI18n.t("关闭并切换") : AieyesI18n.t("确认切换")}</button></div>
    </dialog>`;
  }

  async function run(body) {
    if (busy) return;
    busy = true;error = '';notice = '';paint();
    try {await body();} catch (err) {error = err.message ?? String(err);} finally
    {busy = false;paint();}
  }
  function remember() {
    if (operation) localStorage.setItem('aieyes.codexLogin', JSON.stringify({ sourceId, operationId: operation.operationId, accountId: targetAccountId }));else
    localStorage.removeItem('aieyes.codexLogin');
  }
  async function poll() {
    clearTimeout(timer);
    if (!operation) return;
    try {
      operation = await call('login.status', { operationId: operation.operationId, accountId: targetAccountId });
      if (['succeeded', 'failed', 'cancelled'].includes(operation.status)) {
        if (operation.status === 'succeeded') {notice = operation.accountArchived ? AieyesI18n.t("登录已保存，该账户已归档，可在账户页恢复。") : operation.quotaEnabled === false ? AieyesI18n.t("登录成功，该账户的额度查询已暂停。") : AieyesI18n.t("登录成功，已启用此账户的额度查询。");await settingsChanged();await refresh();} else
        error = operation.error || AieyesI18n.t("登录已结束");
        operation = null;remember();
      }
    } catch (err) {error = err.message ?? String(err);}
    paint();if (operation) timer = setTimeout(poll, 1800);
  }
  function visibleProfiles() {
    const a = state.settings.accounts.find((a) => a.provider === 'codex' && a.id === targetAccountId);
    if (!a) return info?.profiles ?? [];
    return (info?.profiles ?? []).filter((p) => {const ref = selected()?.codexHomeId + ':' + p.id;return a.identityKey ? a.identityKey === p.identity.key : window.AieyesAgentSettings.profiles(a).includes(ref) || !state.settings.accounts.some((other) => window.AieyesAgentSettings.profiles(other).includes(ref));});
  }
  function html() {
    const sources = savedSources();
    if (!sourceId || !sources.some((s) => s.id === sourceId)) {sourceId = sources[0]?.id ?? '';info = null;}
    const source = selected(),account = targetAccount();
    return `<div class="card"><h2>${intent === 'switch' ? AieyesI18n.t("切换账户") : AieyesI18n.t("Codex 登录")}</h2><p class="muted">${AieyesI18n.t("{0}切换会改变机器当前使用的账户。", intent === 'switch' ? AieyesI18n.t("选择目标账户后，检查并确认切换。") : account ? AieyesI18n.t("在所选机器登录「") + e(account.name) + AieyesI18n.t("」，请使用同一个账户。") : AieyesI18n.t("登录成功后自动添加账户，默认以邮箱命名；已有账户会自动识别。"))}</p>${sources.length ? `<label>${AieyesI18n.t("运行位置")}<select id="auth-source" ${busy || operation ? 'disabled' : ''}>${sources.map((s) => `<option value="${e(s.id)}" ${s.id === sourceId ? 'selected' : ''}>${e(s.hostId ? state.settings.hosts.find((h) => h.id === s.hostId)?.name ?? s.hostId : AieyesI18n.t("本机"))} · ${e(s.path)}</option>`).join('')}</select></label><p class="tiny muted source-path" title="${e(source.path)}">${e(AieyesUI.shortPath(source.path))}</p>` : AieyesI18n.t("<p>没有可用的 Codex 目录。请启用 Agent、选择机器并恢复采集。</p><button type=\"button\" data-auth-settings>打开机器设置</button>")}${error ? `<div class="error" role="alert">${e(AieyesI18n.message(error))} <button data-auth="refresh" ${busy ? 'disabled' : ''}>${AieyesI18n.t("重试读取")}</button></div>` : ''}${notice ? `<p role="status">${e(AieyesI18n.message(notice))}</p>` : ''}${busy ? AieyesI18n.t("<p role=\"status\">正在处理…</p>") : ''}
    <fieldset ${busy || !source || operation ? 'disabled' : ''} style="border:0;padding:0"><div class="actions">${source && intent === 'login' ? `<button data-auth="adopt">${AieyesI18n.t("读取当前登录账户")}</button><button data-auth="login">${account ? AieyesI18n.t("登录此账户") : AieyesI18n.t("登录新账户")}</button>` : ''}</div>
    ${info ? `<p class="tiny muted">${AieyesI18n.t("{0} · 认证存储 {1} · 检测到 {2} 个相关进程", e(info.version || AieyesI18n.t("CLI 版本未知")), e(info.storageMode), info.processes?.length ?? 0)}</p>` : ''}
    ${info?.cliAvailable === false ? `<div class="error cli-diagnostic" role="status"><p>${e(info.cliError || AieyesI18n.t("Codex CLI 暂不可用"))}</p><p class="tiny muted">${AieyesI18n.t("检查的程序：{0}。相关进程数量不代表 CLI 可调用。", e(info.resolvedBinary || 'codex'))}</p><button type="button" data-auth-settings>${AieyesI18n.t("机器与 CLI 设置")}</button></div>` : ''}
    ${info?.selected?.kind === 'api' ? `<p class="muted">${AieyesI18n.t("当前使用 {0}。{1}", e(info.selected.name || 'API'), info.storedIdentity ? AieyesI18n.t("本机账户凭据仍已保存，可切换回账户。") : '')}</p>` : ''}
    ${visibleProfiles().map((p) => `<div class="list-row"><div class="row-body"><strong>${e(p.name)}</strong>${p.current ? AieyesI18n.t(" · 当前日常账户") : ''}<small>${e(p.identity.email)} · ${e(p.identity.plan)}</small></div>${intent !== 'login' || state.settings.accounts.some((a) => window.AieyesAgentSettings.profiles(a).includes(source.codexHomeId + ':' + p.id)) ? '' : `<button data-profile="${e(p.id)}" data-auth="bind">${AieyesI18n.t("添加此账户")}</button>`}${intent === 'login' ? `<button data-profile="${e(p.id)}" data-auth="reauth">${AieyesI18n.t("重新登录")}</button>` : ''}${p.current ? '' : intent === 'switch' || !!targetAccountId ? `<button data-profile="${e(p.id)}" data-auth="switch">${AieyesI18n.t("切换到此账户")}</button>` : `<button data-profile="${e(p.id)}" data-auth="remove">${AieyesI18n.t("移除")}</button>`}</div>`).join('')}
    </fieldset>${switchDialog()}
    ${operation ? `<div class="card" role="status"><h3>${AieyesI18n.t("等待授权")}</h3>${operation.userCode ? `<p>${AieyesI18n.t("设备码：")}<strong>${e(operation.userCode)}</strong> <button data-auth="copyCode">${AieyesI18n.t("复制设备码")}</button></p>` : ''}${operation.authUrl ? AieyesI18n.t("<button data-auth=\"open\">打开登录页面</button><button data-auth=\"copyUrl\">复制登录链接</button>") : AieyesI18n.t("<p>正在准备登录…</p>")}<button data-auth="cancelLogin">${AieyesI18n.t("取消登录")}</button><p class="tiny muted">${AieyesI18n.t("请在浏览器完成授权。登录最多等待 10 分钟。")}</p></div>` : ''}</div>`;
  }
  async function ensureManaged() {if (!selected()?.codexHomeId) {await call('enable');await settingsChanged();}}
  function login(profileId) {
    const p = info?.profiles.find((p) => p.id === profileId);
    showEditor(p ? AieyesI18n.t("重新登录") : AieyesI18n.t("添加 Codex 账户"), `<p>${AieyesI18n.t("默认以邮箱命名，可在账户设置中修改。")}</p><p>${AieyesI18n.t("将在{0}保存登录。{1}", selected().hostId ? AieyesI18n.t("服务器") : AieyesI18n.t("本机"), p ? AieyesI18n.t("请使用原账户。") : '')}</p>`, async () => {
      await ensureManaged();
      prepared = null;operation = await call('login.start', { profileId: profileId ?? null, deviceCode: !!selected().hostId });
      remember();poll();
    }, 'confirmation');
  }
  function bind() {
    if (needsRefresh && !busy && !operation && sourceId) {needsRefresh = false;queueMicrotask(() => run(async () => {
        await refresh();
        if (prepareOnOpen) {
          prepareOnOpen = false;
          const account = targetAccount(),profile = account && visibleProfiles().find((p) => !p.current && window.AieyesAgentSettings.profiles(account).includes(selected()?.codexHomeId + ':' + p.id));
          if (profile) await prepareSwitch(profile.id);
        }
      }));}
    const dialog = document.getElementById('codex-switch');
    if (dialog) {
      if (!dialog.open) dialog.showModal();
      dialog.oncancel = (event) => {event.preventDefault();if (!busy) {switchReview = null;prepared = null;paint();}};
    }
    document.querySelector('[data-auth-settings]')?.addEventListener('click', () => openSettings({ provider: 'codex', section: 'machines' }));
    const selector = document.getElementById('auth-source');
    if (selector) selector.onchange = () => {sourceId = selector.value;info = null;prepared = null;switchReview = null;switchedReview = null;run(refresh);};
    document.querySelectorAll('[data-auth]').forEach((button) => button.onclick = () => {
      const action = button.dataset.auth,profileId = button.dataset.profile;
      if (action === 'login' || action === 'reauth') return login(profileId);
      if (action === 'bind') return run(async () => {await call('profiles.bind', { profileId, accountId: targetAccountId });await settingsChanged();await refresh();});
      if (action === 'remove') return showEditor(AieyesI18n.t("移除账户档案"), AieyesI18n.t("<p>删除此位置保存的登录凭据，历史记录保留。</p>"), async () => {await call('profiles.remove', { profileId });await settingsChanged();await refresh();paint();}, 'confirmation');
      run(async () => {
        if (action === 'refresh') {if (switchedReview) await refreshAfterSwitch(switchedReview);else await refresh();}
        if (action === 'adopt') {await ensureManaged();await call('adopt', { accountId: targetAccountId });await settingsChanged();await refresh();}
        if (action === 'switch') await prepareSwitch(profileId);
        if (action === 'retrySwitch' && switchReview) await prepareSwitch(switchReview.profileId);
        if (action === 'cancelSwitch') {prepared = null;switchReview = null;switchError = '';}
        if (action === 'commit') await commitSwitch();
        if (action === 'cancelLogin') await call('login.cancel', { operationId: operation.operationId, accountId: targetAccountId });
        if (action === 'copyUrl' || action === 'copyCode') {await navigator.clipboard.writeText(action === 'copyUrl' ? operation.authUrl : operation.userCode);notice = AieyesI18n.t("已复制");}
        if (action === 'open') await api('codexAuth.openUrl', { url: operation.authUrl });
      });
    });
    if (!operation) {try {const saved = JSON.parse(localStorage.getItem('aieyes.codexLogin') || 'null');if (saved && savedSources().some((s) => s.id === saved.sourceId)) {sourceId = saved.sourceId;targetAccountId = saved.accountId || '';operation = { operationId: saved.operationId };poll();}} catch {localStorage.removeItem('aieyes.codexLogin');}}
  }
  window.AieyesCodexAuth = { get busy() {return busy || Boolean(operation) || Boolean(switchReview);}, html, bind, open(accountId = '', selectedSource = '', action = 'login') {if (!operation) {intent = action;needsRefresh = true;targetAccountId = accountId;if (selectedSource) sourceId = selectedSource;info = null;prepared = null;switchReview = null;switchedReview = null;prepareOnOpen = action === 'switch' && !!accountId;error = '';notice = '';}} };
})();
