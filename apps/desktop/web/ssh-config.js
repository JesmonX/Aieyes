/* Import aliases, never flatten SSH options or read private key contents. */
(() => {
  let dialog,loading = false;
  async function open() {
    if (loading) return;
    loading = true;
    try {
      const result = await api('hosts.sshConfig.read');
      dialog?.remove();dialog = document.createElement('dialog');
      dialog.id = 'ssh-config-import';dialog.setAttribute('aria-labelledby', 'ssh-import-title');
      const existing = new Set(state.settings.hosts.map((h) => h.target.toLowerCase()));
      const rows = result.hosts ?? [];
      dialog.innerHTML = `<div class="dialog-head"><h2 id="ssh-import-title">${AieyesI18n.t("从 SSH 配置导入")}</h2><button type="button" data-import-close aria-label="${AieyesI18n.t("关闭")}">${uiIcon('x')}</button></div><div class="ssh-import-body"><p class="muted source-path">${escapeHTML(result.path)}</p>${helpHTML(AieyesI18n.t("SSH 配置"), AieyesI18n.t("沿用 SSH 别名、密钥和跳板配置。导入后可测试连接。"))}<input type="search" id="ssh-import-search" placeholder="${AieyesI18n.t("搜索别名或地址")}" aria-label="${AieyesI18n.t("搜索 SSH 服务器")}"><div class="actions"><button type="button" data-import-all>${AieyesI18n.t("全选当前结果")}</button><button type="button" data-import-none>${AieyesI18n.t("清空选择")}</button></div><div class="ssh-import-list">${rows.map((host, i) => `<label class="ssh-import-row" data-search="${escapeHTML((host.alias + ' ' + (host.hostName ?? '')).toLowerCase())}"><input type="checkbox" data-import-index="${i}" ${existing.has(host.alias.toLowerCase()) ? 'disabled' : ''}><span><strong>${escapeHTML(host.alias)}</strong><small>${escapeHTML([host.user, host.hostName ?? AieyesI18n.t("跟随 SSH 配置"), host.port].filter(Boolean).join(' · '))}${existing.has(host.alias.toLowerCase()) ? AieyesI18n.t(" · 已添加") : ''}</small></span></label>`).join('') || `<p class="empty">${AieyesI18n.t("未找到 SSH 别名，可手动添加服务器。")}</p>`}</div>${(result.warnings ?? []).map((w) => `<div class="error">${escapeHTML(w)}</div>`).join('')}<div id="ssh-import-error" class="error" role="alert"></div></div><div class="dialog-foot"><span id="ssh-import-count" role="status"></span><button type="button" data-import-close>${AieyesI18n.t("取消")}</button><button type="button" class="primary" id="ssh-import-save">${AieyesI18n.t("导入所选服务器")}</button></div>`;
      document.body.append(dialog);
      const inputs = () => [...dialog.querySelectorAll('[data-import-index]')];
      const update = () => {const count = inputs().filter((i) => i.checked).length;dialog.querySelector('#ssh-import-count').textContent = AieyesI18n.t("已选 {0} 台", count);dialog.querySelector('#ssh-import-save').disabled = !count;};
      dialog.querySelectorAll('[data-import-close]').forEach((b) => b.onclick = () => dialog.close());
      dialog.querySelector('#ssh-import-search').oninput = (event) => {for (const row of dialog.querySelectorAll('[data-search]')) row.hidden = !row.dataset.search.includes(event.target.value.trim().toLowerCase());};
      dialog.querySelector('[data-import-all]').onclick = () => {inputs().filter((i) => !i.disabled && !i.closest('[data-search]').hidden).forEach((i) => i.checked = true);update();};
      dialog.querySelector('[data-import-none]').onclick = () => {inputs().forEach((i) => i.checked = false);update();};
      inputs().forEach((i) => i.onchange = update);update();
      dialog.querySelector('#ssh-import-save').onclick = async () => {
        const selected = inputs().filter((i) => i.checked).map((i) => rows[Number(i.dataset.importIndex)]);
        const base = structuredClone(state.settings),next = structuredClone(base),known = new Set(base.hosts.map((h) => h.target.toLowerCase()));
        let added = 0;
        for (const host of selected) if (!known.has(host.alias.toLowerCase())) {
          known.add(host.alias.toLowerCase());added++;
          next.hosts.push({ id: crypto.randomUUID(), name: host.alias, target: host.alias, authMode: 'ssh', username: '', passwordRef: '', port: null, identityFile: '', shell: '/bin/bash', preCommand: '', enabled: true, monitoringEnabled: true, metrics: Object.keys(groups), devices: [], details: Object.keys(detailOptions) });
        }
        dialog.querySelectorAll('button,input').forEach((el) => el.disabled = true);
        dialog.oncancel = (event) => event.preventDefault();
        try {
          if (added) await window.AieyesAgentSettings.commit(next, base);
          dialog.close();if ($('#editor')?.open) await closeEditor();
          if (state.page === 'settings') renderSettings(false);
          notify(AieyesI18n.t("已导入 {0} 台服务器{1}", added, selected.length > added ? AieyesI18n.t("，重复别名已跳过") : ''));
        } catch (error) {AieyesUI.showError(dialog.querySelector('#ssh-import-error'), String(error.message ?? error), AieyesI18n.t("SSH 导入失败"));} finally
        {dialog.oncancel = null;dialog.querySelectorAll('button,input').forEach((el) => el.disabled = false);inputs().forEach((i) => i.disabled = existing.has(rows[Number(i.dataset.importIndex)].alias.toLowerCase()));update();}
      };
      dialog.showModal();dialog.querySelector('#ssh-import-search').focus();
    } catch (error) {notify(AieyesI18n.t("读取 SSH 配置失败：") + String(error.message ?? error), 'error');} finally
    {loading = false;}
  }
  document.addEventListener('click', (event) => {if (event.target.closest('[data-ssh-import]')) void open();});
  window.AieyesSSHConfig = { open };
})();
