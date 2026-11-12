/* Settings › Webhook: receivers (at most 8) and the events sent to them. Every change is saved
   at once; delivery runs in the native shell. A receiver's address can hold a token, so the
   list shows its host only. */
(() => {
  const MAX = 8;
  const FORMATS = ['json', 'text', 'bark', 'slack', 'wechat', 'feishu'];
  const EVENTS = [['quotaReset', "订阅额度重置"], ['sessionNeedsInput', "会话等待输入"], ['sessionComplete', "会话完成"], ['sessionFailed', "会话执行失败"], ['serverAlert', "服务器告警"]];
  const tests = new Map();
  const escape = (value) => AieyesUI.escapeHTML(value);
  const defaults = () => ({ enabled: false, targets: [], quotaReset: true, sessionNeedsInput: true, sessionComplete: true, sessionFailed: true, serverAlert: true });
  // A copy: changes are made on it and compared with the stored settings when saving.
  const preferences = () => structuredClone({ ...defaults(), ...(state.settings?.webhooks ?? {}) });
  const formatTitle = (format) => ({ text: AieyesI18n.t("纯文本"), bark: 'Bark', slack: 'Slack', wechat: AieyesI18n.t("企业微信"), feishu: AieyesI18n.t("飞书") })[format] ?? 'JSON';
  const formatNote = (format) => ({ text: AieyesI18n.t("以 POST 发送纯文本，标题与内容各占一行。"), bark: AieyesI18n.t("填写 Bark 推送地址，以设备密钥结尾。"), slack: AieyesI18n.t("填写 Slack Incoming Webhook 地址。"), wechat: AieyesI18n.t("填写企业微信群机器人的 Webhook 地址。"), feishu: AieyesI18n.t("填写飞书群机器人的 Webhook 地址。") })[format] ?? AieyesI18n.t("以 POST 发送 JSON，含 event、title、text、at 与 data 字段。");
  function host(url) {try {return new URL(String(url).trim()).hostname;} catch (_) {return '';}}
  function valid(url) {try {const parsed = new URL(String(url).trim());return ['http:', 'https:'].includes(parsed.protocol) && Boolean(parsed.hostname);} catch (_) {return false;}}
  const toggle = (attribute, checked, label, disabled = false) => `<label class="quota-confirm"><input type="checkbox" role="switch" ${attribute} ${checked ? 'checked' : ''} ${disabled ? 'disabled' : ''}>${label}</label>`;
  function testHTML(id) {
    const test = tests.get(id);
    if (test === 'delivered') return `<p class="webhook-test ok" role="status">${uiIcon('checked')}${AieyesI18n.t("测试消息已送达")}</p>`;
    if (test?.error) return `<div class="error webhook-test" role="alert"><strong>${AieyesI18n.t("测试消息未送达")}</strong>${AieyesUI.errorHTML(test.error, AieyesI18n.t("测试消息未送达"))}</div>`;
    return '';
  }
  function rowHTML(target) {
    const name = target.name?.trim() || host(target.url),sending = tests.get(target.id) === 'sending';
    return `<div class="webhook-row" data-webhook-target="${escape(target.id)}"><label class="webhook-target"><input type="checkbox" role="switch" data-webhook-enable ${target.enabled ? 'checked' : ''} aria-label="${escape(name)}"><span><strong data-user-text>${escape(name)}</strong><small class="muted">${escape(formatTitle(target.format))} · ${escape(host(target.url))}</small></span></label><div class="webhook-actions"><button type="button" data-webhook-test ${sending ? 'disabled aria-busy="true"' : ''}>${AieyesI18n.t("发送测试")}</button><button type="button" class="icon-button" data-webhook-edit title="${AieyesI18n.t("编辑")}" aria-label="${AieyesI18n.t("编辑")}">${uiIcon('settings')}</button><button type="button" class="icon-button" data-webhook-remove title="${AieyesI18n.t("移除")}" aria-label="${AieyesI18n.t("移除")}">${uiIcon('x')}</button></div>${testHTML(target.id)}</div>`;
  }
  function body() {
    const p = preferences(),full = p.targets.length >= MAX;
    return `<section class="card webhook-settings" data-settings-section="webhook"><h2>Webhook</h2>${toggle('id="webhooks-enabled"', p.enabled, AieyesI18n.t("发送 Webhook 通知"))}<p class="muted tiny">${AieyesI18n.t("事件发生时向接收地址发送一条消息：会话消息包含 Agent、机器与项目文件夹名，额度消息包含账户与额度窗口，服务器消息包含主机与使用率。")}</p></section>`
      + `<section class="card webhook-targets" data-settings-section="targets"><h2>${AieyesI18n.t("接收地址")}</h2>${p.targets.length ? `<div class="webhook-list">${p.targets.map(rowHTML).join('')}</div>` : `<p class="muted">${AieyesI18n.t("尚未添加接收地址")}</p>`}<div class="actions"><button type="button" id="webhook-add" ${full ? 'disabled' : ''}>${AieyesI18n.t("添加地址")}</button>${full ? `<span class="muted tiny">${AieyesI18n.t("最多 8 个接收地址")}</span>` : ''}</div></section>`
      + `<section class="card webhook-events" data-settings-section="events"><h2>${AieyesI18n.t("通知事件")}</h2>${EVENTS.map(([key, label]) => toggle(`data-webhook-event="${key}"`, p[key], AieyesI18n.t(label))).join('')}<p class="muted tiny">${AieyesI18n.t("额度重置发送每一次已确认的重置；服务器告警使用“通知”中的阈值与持续时间。")}</p></section>`;
  }
  // Saves one change on top of the stored settings; the page repaints from the saved value.
  async function save(change) {
    const next = structuredClone(state.settings);next.webhooks = preferences();change(next.webhooks);
    try {await window.AieyesAgentSettings.commit(next);}
    catch (error) {notify(error.message ?? String(error), 'error');}
    finally {if (state.page === 'settings') renderSettings(false);}
  }
  function edit(target) {
    const current = target ?? { id: crypto.randomUUID(), name: '', url: '', format: 'json', enabled: true };
    const html = field('webhookName', AieyesI18n.t("名称"), current.name, AieyesI18n.t("可选"))
      + select('webhookFormat', AieyesI18n.t("格式"), FORMATS.map((format) => [format, formatTitle(format)]), current.format)
      + `<p class="muted tiny" id="webhook-format-note">${escape(formatNote(current.format))}</p>`
      + field('webhookUrl', AieyesI18n.t("地址"), current.url, 'https://', 'url');
    showEditor(target ? AieyesI18n.t("编辑接收地址") : AieyesI18n.t("添加接收地址"), html, async (form) => {
      const url = String(form.get('webhookUrl') ?? '').trim();
      if (!valid(url)) throw new Error(AieyesI18n.t("请输入以 http:// 或 https:// 开头的完整地址"));
      const saved = { ...current, name: String(form.get('webhookName') ?? '').trim(), url, format: FORMATS.includes(form.get('webhookFormat')) ? form.get('webhookFormat') : 'json' };
      const next = structuredClone(state.settings);next.webhooks = preferences();
      const index = next.webhooks.targets.findIndex((t) => t.id === saved.id);
      if (index >= 0) next.webhooks.targets[index] = saved;else next.webhooks.targets.push(saved);
      tests.delete(saved.id);
      await window.AieyesAgentSettings.commit(next);
      if (state.page === 'settings') renderSettings(false);
    });
    const format = document.querySelector('#field-webhookFormat');
    if (format) format.addEventListener('change', () => {document.querySelector('#webhook-format-note').textContent = formatNote(format.value);});
  }
  function remove(target) {
    confirmDestructive(AieyesI18n.t("移除此接收地址？"), `<p>${AieyesI18n.t("移除后不再向该地址发送消息。")}</p>`, async () => {
      tests.delete(target.id);
      const next = structuredClone(state.settings);next.webhooks = preferences();next.webhooks.targets = next.webhooks.targets.filter((t) => t.id !== target.id);
      await window.AieyesAgentSettings.commit(next);
      if (state.page === 'settings') renderSettings(false);
    }, AieyesI18n.t("移除"));
  }
  function repaintRow(id) {
    const row = document.querySelector(`[data-webhook-target="${CSS.escape(id)}"]`),target = preferences().targets.find((t) => t.id === id);
    if (!row || !target) return;
    const focused = row.contains(document.activeElement) ? [...row.querySelectorAll('button,input')].indexOf(document.activeElement) : -1;
    row.outerHTML = rowHTML(target);bind();
    if (focused >= 0) document.querySelector(`[data-webhook-target="${CSS.escape(id)}"]`)?.querySelectorAll('button,input')[focused]?.focus({ preventScroll: true });
  }
  async function test(target) {
    tests.set(target.id, 'sending');repaintRow(target.id);
    try {await api('webhooks.test', { target });tests.set(target.id, 'delivered');}
    catch (error) {tests.set(target.id, { error: error.message ?? String(error) });}
    repaintRow(target.id);
  }
  function bind() {
    const p = preferences(),target = (el) => p.targets.find((t) => t.id === el.closest('[data-webhook-target]')?.dataset.webhookTarget);
    const enabled = document.querySelector('#webhooks-enabled');
    if (enabled) enabled.onchange = () => void save((w) => {w.enabled = enabled.checked;});
    document.querySelectorAll('[data-webhook-event]').forEach((input) => input.onchange = () => void save((w) => {w[input.dataset.webhookEvent] = input.checked;}));
    document.querySelectorAll('[data-webhook-enable]').forEach((input) => input.onchange = () => {const t = target(input);if (t) void save((w) => {w.targets.find((x) => x.id === t.id).enabled = input.checked;});});
    document.querySelectorAll('[data-webhook-test]').forEach((button) => button.onclick = () => {const t = target(button);if (t) void test(t);});
    document.querySelectorAll('[data-webhook-edit]').forEach((button) => button.onclick = () => {const t = target(button);if (t) edit(t);});
    document.querySelectorAll('[data-webhook-remove]').forEach((button) => button.onclick = () => {const t = target(button);if (t) remove(t);});
    const add = document.querySelector('#webhook-add');
    if (add) add.onclick = () => edit();
  }
  window.AieyesSettingsPages?.register('webhooks', { body, bind });
})();
