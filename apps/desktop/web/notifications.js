(() => {
  const invoke = (command) => window.__TAURI__.core.invoke(command);
  let messages = { granted: AieyesI18n.t("通知接口可用，请发送测试通知检查显示"), denied: AieyesI18n.t("系统通知未授权，请在系统通知设置中允许 Aieyes"), prompt: AieyesI18n.t("尚未请求系统通知权限"), unknown: AieyesI18n.t("无法读取通知权限，请使用测试通知验证") };window.addEventListener("aieyes:language", () => {messages = { granted: AieyesI18n.t("通知接口可用，请发送测试通知检查显示"), denied: AieyesI18n.t("系统通知未授权，请在系统通知设置中允许 Aieyes"), prompt: AieyesI18n.t("尚未请求系统通知权限"), unknown: AieyesI18n.t("无法读取通知权限，请使用测试通知验证") };});
  let lastError = '',lastNotice = '';
  async function refresh() {
    const target = document.querySelector('#notification-permission');if (!target) return;
    try {
      const info = await invoke('notifications_info');
      if (!target.isConnected) return;
      target.textContent = messages[info.permission] ?? messages.unknown;
      const error = document.querySelector('#notification-error');AieyesUI.showError(error, lastError || info.error || '', AieyesI18n.t("通知操作失败"));
      const notice = document.querySelector('#notification-test-result');if (notice) notice.textContent = lastNotice;
    } catch (error) {if (target.isConnected) target.textContent = AieyesI18n.t("读取通知状态失败：") + String(error);}
  }
  window.AieyesNotifications = {
    settingsHTML: () => AieyesI18n.t("<section class=\"card\"><h2>系统通知</h2><p id=\"notification-permission\" role=\"status\">正在读取通知权限…</p><button type=\"button\" id=\"notification-test\">发送测试通知</button><p id=\"notification-test-result\" role=\"status\"></p><div id=\"notification-error\" class=\"error\" role=\"status\"></div><p class=\"muted tiny\">通知显示和声音由系统通知及勿扰设置控制。</p></section>"),
    bind() {
      const volume = document.querySelector('[name=soundVolume]');
      if (volume) volume.addEventListener('input', () => {const output = document.querySelector('[data-sound-volume]');if (output) output.textContent = volume.value + '%';});
      for (const button of document.querySelectorAll('[data-sound-preview]')) button.onclick = async () => {
        button.disabled = true;
        const result = document.querySelector('[data-sound-result]');
        const kind = button.dataset.soundPreview,tone = document.querySelector('[name=' + (kind === 'needsInput' ? 'needsInputTone' : 'completeTone') + ']')?.value ?? 'default';
        try { await window.__TAURI__.core.invoke('session_sounds_preview', { kind, tone, volume: Number(volume?.value ?? 100) / 100 }); if (result) result.textContent = ''; }
        catch (error) { if (result) result.textContent = AieyesI18n.message(String(error)); }
        finally { button.disabled = false; }
      };
      const button = document.querySelector('#notification-test');if (!button) return;
      button.onclick = async () => {
        button.disabled = true;lastError = '';lastNotice = '';
        try {await invoke('notifications_test');lastNotice = AieyesI18n.t("测试通知已提交，请检查系统显示。");} catch (error) {lastError = String(error);} finally
        {button.disabled = false;await refresh();}
      };
      void refresh();
    }
  };
  window.addEventListener('focus', () => void refresh());
  window.__TAURI__?.event?.listen('desktop:notifications', () => void refresh()).catch(() => {});
})();
