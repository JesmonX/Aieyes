/* Shared account/machine projection; credential handling stays in the core. */
(() => {
  const e = (v) => AieyesUI.escapeHTML(v ?? '');
  let pendingSources = new Set();
  let dialog,accountKey = '',rows = [],errors = [],checking = false,embedded = false,generation = 0;
  const eligible = (key) => {
    const a = state.settings?.accounts.find((a) => a.provider + ':' + a.id === key && !a.archived);
    return a?.provider === 'codex' && state.settings.sources.some((s) => s.provider === 'codex' && AieyesUI.sourceEnabled(state.settings, s));
  };
  async function refresh(key, changed) {
    let targets = await api('accounts.targets', key ? { accountKey: key } : {});
    const checkingSources = new Set(targets.filter((t) => t.provider === 'codex' && !t.nativeConnectionId).map((t) => t.sourceId));
    changed?.(targets, [], checkingSources);
    const sources = [...new Set(targets.filter((t) => t.provider === 'codex' && !t.nativeConnectionId).map((t) => t.sourceId))];
    const issues = [];
    const machineName = (sourceId) => targets.find((t) => t.sourceId === sourceId)?.machineName || sourceId;
    await Promise.all(sources.map(async (sourceId) => {
      try {
        const result = await api('codexAuth.reconcile', { sourceId, accountId: key?.replace(/^codex:/, '') || '' });
        window.AieyesAgentSettings.accept(await api('settings.get'));
        issues.push(...result.issues.map((i) => machineName(i.sourceId || sourceId) + '：' + i.message));
      } catch (error) {issues.push(machineName(sourceId) + '：' + String(error.message ?? error));}
      targets = await api('accounts.targets', key ? { accountKey: key } : {});
      checkingSources.delete(sourceId);
      changed?.(targets, [...issues], new Set(checkingSources));
    }));
    await window.AieyesAgentSettings.refreshStatuses(false);
    return targets;
  }
  function paint() {
    if (!dialog?.open) return;
    const title = state.settings.accounts.find((a) => a.provider + ':' + a.id === accountKey)?.name ?? AieyesI18n.t("账户");
    dialog.innerHTML = `<div class="dialog-head"><h2>${AieyesI18n.t("切换账户 · {0}", e(title))}</h2></div><div class="account-target-body">${embedded ? window.AieyesCodexAuth.html() : `${checking ? `<p role="status">${AieyesI18n.t("正在检查各机器的登录…")}</p>` : ''}${errors.map((v) => `<div class="error">${e(v)}</div>`).join('')}${rows.map((t, i) => `<div class="list-row"><div class="row-body"><strong>${e(t.machineName)}</strong><small class="source-path" title="${e(t.path)}">${e(AieyesUI.shortPath(t.path))}</small><small>${e(AieyesI18n.message(t.reason || (t.current === true ? AieyesI18n.t("正在使用") : t.credential === true ? AieyesI18n.t("已登录，尚未选用") : t.credential === false ? AieyesI18n.t("未登录") : AieyesI18n.t("登录状态待确认"))))}</small>${t.draftReason ? `<small class="error">${e(t.draftReason)}</small>` : ''}</div>${t.nativeConnectionId ? `<button data-native-scan-agent="${e(t.provider)}">${AieyesI18n.t("重新扫描")}</button>` : t.current === true && t.bound !== false && t.canDraft !== false ? '' : `<button data-account-target="${i}" ${pendingSources.has(t.sourceId) && !t.canSwitch ? 'disabled' : ''}>${t.bound === false ? AieyesI18n.t("完成关联") : t.canSwitch ? AieyesI18n.t("切换") : t.credential === false ? AieyesI18n.t("登录") : AieyesI18n.t("检查 / 登录")}</button>`}</div>`).join('')}`}</div><div class="dialog-foot">${embedded ? `<button id="target-back">${AieyesI18n.t("返回机器列表")}</button>` : `<button id="target-refresh">${AieyesI18n.t("重新检查")}</button>`}<button id="target-close">${AieyesI18n.t("完成")}</button></div>`;
    dialog.querySelector('#target-close').disabled = embedded && window.AieyesCodexAuth.busy;
    dialog.querySelector('#target-close').onclick = () => {generation++;dialog.close();embedded = false;};
    if (embedded) {
      dialog.querySelector('#target-back').disabled = window.AieyesCodexAuth.busy;
      dialog.querySelector('#target-back').onclick = () => {embedded = false;reload();};
      window.AieyesCodexAuth.bind();
    } else {
      dialog.querySelector('#target-refresh').disabled = checking;
      dialog.querySelector('#target-refresh').onclick = reload;
      dialog.querySelectorAll('[data-account-target]').forEach((button) => button.onclick = () => {
        const t = rows[Number(button.dataset.accountTarget)];
        window.AieyesCodexAuth.open(accountKey.replace(/^codex:/, ''), t.sourceId, t.canSwitch ? 'switch' : 'login');
        embedded = true;paint();
      });
    }
  }
  async function reload() {
    const revision = ++generation;checking = true;errors = [];paint();
    try {await refresh(accountKey, (next, issues, pending) => {if (revision === generation) {rows = next;errors = issues;pendingSources = pending;paint();}});}
    catch (error) {if (revision === generation) errors = [String(error.message ?? error)];} finally
    {if (revision === generation) {checking = false;paint();}}
  }
  function open(key) {
    if (!eligible(key)) return;
    accountKey = key;embedded = false;rows = [];
    if (!dialog) {dialog = document.createElement('dialog');dialog.id = 'account-targets';dialog.className = 'editor-dialog';document.body.append(dialog);dialog.oncancel = (event) => {if (embedded && window.AieyesCodexAuth.busy) event.preventDefault();else {generation++;embedded = false;}};}
    dialog.showModal();reload();
  }
  window.AieyesAccountTargets = { eligible, open, refresh, paintAuth() {if (dialog?.open && embedded) {paint();return true;}return false;} };
})();
