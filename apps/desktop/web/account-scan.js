/* Native credentials stay in their original configuration, including over SSH. */
(() => {
  let busy = false;
  const e = (value) => escapeHTML(value ?? '');
  const same = (a, b) => AieyesUI.sameAgent(a, b);
  const machine = (id) => id === 'local' ? AieyesI18n.t("本机") : state.settings.hosts.find((h) => h.id === id)?.name || id;
  function targets(provider) {return (state.settings.agents ?? []).filter((a) => same(a.provider, provider) && !['deepseek', 'custom'].includes(a.provider)).flatMap((a) => a.machineIds ?? []).filter((id) => id === 'local' ? state.settings.localDeviceEnabled !== false : state.settings.hosts.some((h) => h.id === id && h.enabled));}
  function button(provider) {if (provider === 'custom') return '';const enabled = (state.settings.agents ?? []).some((a) => same(a.provider, provider) && a.enabled),available = enabled && targets(provider).length > 0;return `<button type="button" data-native-scan-agent="${e(provider)}" ${busy || !available ? 'disabled' : ''} title="${available ? AieyesI18n.t("读取已选择且开启 Agent 采集的机器") : AieyesI18n.t("先启用 Agent、选择机器并开启 Agent 采集")}">${AieyesI18n.t("扫描已启用机器")}</button>`;}
  function hint(provider) {return provider !== 'custom' && !targets(provider).length ? AieyesI18n.t("<p class=\"muted tiny\">先启用 Agent、选择机器并开启 Agent 采集。</p>") : '';}
  function list(provider, kind) {
    const rows = (state.settings.nativeConnections ?? []).filter((r) => same(r.provider, provider) && r.kind === kind);
    if (!rows.length) return '';
    return `<div class="native-connections"><div class="between"><h3>${kind === 'api' ? AieyesI18n.t("原生 API 连接") : AieyesI18n.t("已发现的原生登录凭据")}（${rows.length}）</h3><button type="button" data-native-remove-all="${e(provider)}" data-native-kind="${e(kind)}" ${busy ? 'disabled' : ''}>${AieyesI18n.t("取消全部引用")}</button></div><details data-agent-detail="native-connections-${e(provider)}-${e(kind)}"><summary>${AieyesI18n.t("查看{0}", kind === 'api' ? AieyesI18n.t("连接") : AieyesI18n.t("凭据"))}</summary>${rows.map((r) => `<div class="list-row"><div class="row-body"><strong>${e(r.email || (r.kind === 'api' ? r.baseUrl || AieyesI18n.t("原生 API") : r.identityKey ? r.identityKey.startsWith('qoder:') ? 'Qoder · ' + r.identityKey.slice(6, 18) : (providers[r.provider] ?? r.provider) + AieyesI18n.t(" 账户") : AieyesI18n.t("账户身份待确认")))}</strong><small>${e(machine(r.machineId))} · ${e({ pending: AieyesI18n.t("待验证"), expired: AieyesI18n.t("已过期"), unavailable: AieyesI18n.t("需重新扫描"), observed: AieyesI18n.t("已识别账户") }[r.status] || AieyesI18n.t("待验证"))}${r.accountKey ? AieyesI18n.t(" · 已关联账户") : ''}</small>${r.models?.length ? `<small>${e(r.models.join('、'))}</small>` : ''}<small class="source-path" title="${e(r.configPath)}">${e(r.configPath)}</small><small>${e(AieyesI18n.message(r.note))}</small></div><button type="button" data-native-remove="${e(r.id)}" ${busy ? 'disabled' : ''}>${AieyesI18n.t("取消引用")}</button></div>`).join('')}<p class="muted tiny">${AieyesI18n.t("凭据保留在原机器的 Agent 配置中；取消引用不会删除原文件。")}</p></details></div>`;
  }
  async function scan(params) {
    if (busy) return;
    if (!(await window.AieyesAgentSettings.flush())) return;
    busy = true;
    const dialog = document.createElement('dialog');dialog.className = 'native-scan-dialog';dialog.setAttribute('aria-label', AieyesI18n.t("扫描账户与凭据"));
    dialog.innerHTML = AieyesI18n.t("<h2>扫描账户与凭据</h2><p role=\"status\" data-scan-progress>正在读取原生配置…</p><div data-scan-results></div><div class=\"actions\"><button type=\"button\" data-scan-retry hidden>重新扫描</button><button type=\"button\" data-scan-close>关闭，后台继续</button></div>");
    document.body.append(dialog);dialog.showModal();
    let unlisten,closed = false;
    const close = () => {closed = true;dialog.close();dialog.remove();};
    dialog.querySelector('[data-scan-close]').onclick = close;dialog.oncancel = (event) => {event.preventDefault();close();};
    const operationId = crypto.randomUUID();
    try {
      unlisten = await window.__TAURI__?.event?.listen('operations:progress', ({ payload }) => {if (payload.operationId === operationId && payload.stage && !closed) dialog.querySelector('[data-scan-progress]').textContent = AieyesI18n.message(payload.stage);});
      const result = await api('accounts.scan', { ...params, operationId });
      window.AieyesAgentSettings.accept(result.settings);state.lastQuota = 0;
      const failed = result.results.filter((r) => r.status === 'failed').length;
      const summary = AieyesI18n.t("新增 {0} 个账户、{1} 个连接{2}", result.addedAccounts, result.addedConnections, failed ? '；' + failed + AieyesI18n.t(" 项读取失败") : '');
      if (!closed) {dialog.querySelector('[data-scan-progress]').textContent = summary;dialog.querySelector('[data-scan-results]').innerHTML = result.results.map((r) => `<div class="list-row"><div class="row-body"><strong>${e(providers[r.provider] ?? r.provider)} · ${e(machine(r.machineId))}</strong><small>${e(AieyesI18n.message(r.message))}</small>${r.configPath ? `<small class="source-path">${e(r.configPath)}</small>` : ''}</div><span>${e({ added: AieyesI18n.t("已添加"), existing: AieyesI18n.t("已存在"), failed: AieyesI18n.t("失败"), empty: AieyesI18n.t("未发现"), skipped: AieyesI18n.t("已跳过") }[r.status] ?? r.status)}</span></div>`).join('');} else
      notify(summary, failed ? 'error' : 'success');
    } catch (error) {
      // Partial imports are committed independently. Read them back even if an
      // update/disconnection interrupted the last item.
      try {window.AieyesAgentSettings.accept(await api('settings.get'));} catch {}
      if (!closed) dialog.querySelector('[data-scan-progress]').textContent = AieyesI18n.message(String(error));else notify(String(error), 'error');
    } finally {
      busy = false;if (unlisten) unlisten();
      if (!closed) {dialog.querySelector('[data-scan-close]').textContent = AieyesI18n.t("完成");dialog.querySelector('[data-scan-retry]').hidden = false;dialog.querySelector('[data-scan-retry]').onclick = () => {close();void scan(params);};}
      if (state.page === 'settings') renderSettings(false);
      void loadDashboard().catch(() => {});
    }
  }
  document.addEventListener('click', async (event) => {
    const agent = event.target.closest('[data-native-scan-agent]'),host = event.target.closest('[data-native-scan-machine]'),remove = event.target.closest('[data-native-remove]'),removeAll = event.target.closest('[data-native-remove-all]');
    if (agent && !agent.disabled) void scan({ scope: 'agent', provider: agent.dataset.nativeScanAgent });
    if (host && !host.disabled) void scan({ scope: 'machine', machineId: host.dataset.nativeScanMachine });
    const button = remove || removeAll;
    if (button && !button.disabled && !busy) {
      if (!(await window.AieyesAgentSettings.flush())) return;
      if (busy) return;
      const connectionIds = remove ? [remove.dataset.nativeRemove] : (state.settings.nativeConnections ?? []).filter((r) => same(r.provider, removeAll.dataset.nativeRemoveAll) && r.kind === removeAll.dataset.nativeKind).map((r) => r.id);
      if (!connectionIds.length) return;
      busy = true;button.disabled = true;
      try {const r = await api('accounts.native.remove', { connectionIds });window.AieyesAgentSettings.accept(r.settings);notify(AieyesI18n.t("已取消 {0} 个引用", r.removed));}
      catch (error) {notify(String(error), 'error');} finally
      {busy = false;renderSettings(false);}
    }
  });
  window.AieyesAccountScan = { button, hint, list, scan, get busy() {return busy;} };
})();
