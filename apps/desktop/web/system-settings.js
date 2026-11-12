/* Settings › 启动与系统 (system): launch at login and the aieyes:// navigation links. Both are
   native, per-user settings that apply at once; there is nothing to save. */
(() => {
  let info = null,loading = null,saving = false,failure = '';
  const invoke = (name, args) => window.__TAURI__.core.invoke(name, args);
  const escape = (value) => AieyesUI.escapeHTML(value);
  async function load() {
    if (!window.__TAURI__) return null;
    loading ??= invoke('system_info').then((value) => {info = value;return value;}).finally(() => {loading = null;});
    return loading;
  }
  function autostartHTML() {
    const autostart = info?.autostart;
    const error = failure || autostart?.error || '';
    return `<section class="card system-launch" data-settings-section="launch"><h2>${AieyesI18n.t("启动")}</h2><label class="quota-confirm"><input type="checkbox" role="switch" id="system-autostart" ${autostart?.enabled ? 'checked' : ''} ${!autostart?.available || saving ? 'disabled' : ''}>${AieyesI18n.t("登录时启动 Aieyes")}</label>${error ? `<div class="error" role="alert">${AieyesUI.errorHTML(error, AieyesI18n.t("登录时启动设置未保存"))}</div>` : ''}</section>`;
  }
  function linksHTML() {
    const links = info?.links ?? [];
    return `<section class="card system-links" data-settings-section="links"><h2>${AieyesI18n.t("应用链接")}</h2><p class="muted tiny">${AieyesI18n.t("其他应用、脚本和快捷指令可以用下面的链接打开面板或页面。链接只用于导航。")}</p>${links.map((link) => `<div class="system-link-row"><code>${escape(link)}</code><button type="button" data-copy-link="${escape(link)}">${AieyesI18n.t("复制")}</button></div>`).join('')}</section>`;
  }
  function paint() {
    const launch = document.querySelector('[data-settings-section=launch]'),links = document.querySelector('[data-settings-section=links]');
    if (launch) launch.outerHTML = autostartHTML();
    if (links) links.outerHTML = linksHTML();
    bind();
  }
  async function copy(button) {
    const text = button.dataset.copyLink;
    try {await navigator.clipboard.writeText(text);}
    catch (_) {
      const area = document.createElement('textarea');area.value = text;area.setAttribute('readonly', '');area.style.position = 'fixed';area.style.opacity = '0';document.body.append(area);area.select();
      const copied = document.execCommand('copy');area.remove();if (!copied) {notify(AieyesI18n.t("复制失败"), 'error');return;}
    }
    button.textContent = AieyesI18n.t("已复制");
    setTimeout(() => {if (button.isConnected) button.textContent = AieyesI18n.t("复制");}, 1500);
  }
  function bind() {
    const toggle = document.querySelector('#system-autostart');
    if (toggle) toggle.onchange = async () => {
      saving = true;failure = '';toggle.disabled = true;
      try {info = await invoke('system_autostart', { enabled: toggle.checked });}
      catch (error) {failure = String(error);}
      finally {saving = false;paint();document.querySelector('#system-autostart')?.focus({ preventScroll: true });}
    };
    document.querySelectorAll('[data-copy-link]').forEach((button) => button.onclick = () => void copy(button));
  }
  window.AieyesSettingsPages?.register('system', {
    body: () => autostartHTML() + linksHTML(),
    bind() {
      bind();
      // The first visit reads the native state; later visits repaint from it and refresh quietly.
      void load().then(() => {if (document.querySelector('[data-settings-section=launch]')) paint();}).catch((error) => {failure = String(error);paint();});
    }
  });
})();
