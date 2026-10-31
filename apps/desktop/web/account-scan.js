/* Native credentials stay in their original configuration, including over SSH.
   Core detects them at launch and when their files change; there is no scan UI. */
(() => {
  let busy = false;
  const e = (value) => escapeHTML(value ?? '');
  const same = (a, b) => AieyesUI.sameAgent(a, b);
  const machine = (id) => id === 'local' ? AieyesI18n.t("本机") : state.settings.hosts.find((h) => h.id === id)?.name || id;
  // Legacy "pending"/"unavailable" rows read as identified/past accounts; detection is automatic now.
  function status(r) {return r.status === 'stale' ? AieyesI18n.t("打开 {0} 一次即可刷新", providers[r.provider] ?? r.provider) : ['remembered', 'unavailable'].includes(r.status) ? AieyesI18n.t("历史账号") : r.status === 'expired' ? AieyesI18n.t("已过期") : AieyesI18n.t("已识别账户");}
  function list(provider, kind) {
    const rows = (state.settings.nativeConnections ?? []).filter((r) => same(r.provider, provider) && r.kind === kind);
    if (!rows.length) return '';
    return `<div class="native-connections"><div class="between"><h3>${kind === 'api' ? AieyesI18n.t("原生 API 连接") : AieyesI18n.t("已发现的原生登录凭据")}（${rows.length}）</h3><button type="button" class="danger" data-native-remove-all="${e(provider)}" data-native-kind="${e(kind)}" ${busy ? 'disabled' : ''}>${AieyesI18n.t("取消全部引用")}</button></div><details data-agent-detail="native-connections-${e(provider)}-${e(kind)}"><summary>${AieyesI18n.t("查看{0}", kind === 'api' ? AieyesI18n.t("连接") : AieyesI18n.t("凭据"))}</summary>${rows.map((r) => `<div class="list-row"><div class="row-body"><strong>${e(r.email || (r.kind === 'api' ? r.baseUrl || AieyesI18n.t("原生 API") : r.identityKey ? r.identityKey.startsWith('qoder:') ? 'Qoder · ' + r.identityKey.slice(6, 18) : AieyesI18n.t("{0} 账户", providers[r.provider] ?? r.provider) : AieyesI18n.t("账户身份待确认")))}</strong><small>${e(machine(r.machineId))} · ${e(status(r))}${r.accountKey ? AieyesI18n.t(" · 已关联账户") : ''}</small>${r.models?.length ? `<small>${e(r.models.join('、'))}</small>` : ''}<small class="source-path" title="${e(r.configPath)}">${e(r.configPath)}</small><small>${e(AieyesI18n.message(r.note))}</small></div><button type="button" data-native-remove="${e(r.id)}" ${busy ? 'disabled' : ''}>${AieyesI18n.t("取消引用")}</button></div>`).join('')}<p class="muted tiny">${AieyesI18n.t("凭据保留在原机器的 Agent 配置中；取消引用不会删除原文件。")}</p></details></div>`;
  }
  document.addEventListener('click', async (event) => {
    const remove = event.target.closest('[data-native-remove]'),removeAll = event.target.closest('[data-native-remove-all]');
    const button = remove || removeAll;
    if (button && !button.disabled && !busy) {
      if (!(await window.AieyesAgentSettings.flush())) return;
      if (busy) return;
      const connectionIds = remove ? [remove.dataset.nativeRemove] : (state.settings.nativeConnections ?? []).filter((r) => same(r.provider, removeAll.dataset.nativeRemoveAll) && r.kind === removeAll.dataset.nativeKind).map((r) => r.id);
      if (!connectionIds.length) return;
      const unlink = async () => {
        busy = true;button.disabled = true;
        try {const r = await api('accounts.native.remove', { connectionIds });window.AieyesAgentSettings.accept(r.settings);notify(AieyesI18n.t("已取消 {0} 个引用", r.removed));}
        catch (error) {notify(String(error), 'error');} finally
        {busy = false;renderSettings(false);}
      };
      // Unlinking everything at once is confirmed first; it only forgets references, never files.
      if (removeAll) confirmDestructive(AieyesI18n.t("取消全部引用？"), `<p>${AieyesI18n.t("将取消 {0} 个引用。凭据文件不会被删除；之后在 Agent 中重新登录会自动识别。", connectionIds.length)}</p>`, unlink, AieyesI18n.t("取消全部引用"));
      else await unlink();
    }
  });
  window.AieyesAccountScan = { list, status, get busy() {return busy;} };
})();
