/* A personal, bidirectional Feishu application bot. Secrets only exist in the editor;
   the core saves them in the OS credential store and returns non-secret status. */
(() => {
  let status = null, commands = [], sessions = [], pairing = null, error = '', loading = false, timer = null;
  const escape = (value) => AieyesUI.escapeHTML(String(value ?? ''));
  const t = (value) => AieyesI18n.t(value);
  const statuses = { queued: "等待提交", blocked: "待处理", automation: "等待桌面操作", dispatching: "正在提交", submitted: "已提交，待确认", confirmed: "已确认", uncertain: "结果待核对", cancelled: "已取消" };
  const connections = { disabled: "已停用", stopped: "未连接", connecting: "正在连接", connected: "已连接", reconnecting: "正在重连" };
  const visible = () => Boolean(document.querySelector('#channel-settings'));
  function statusHTML() {
    if (!status) return `<p class="muted" role="status">${t("正在读取聊天渠道…")}</p>`;
    return `<div class="setting-row"><strong>${t("连接状态")}</strong><span role="status">${t(connections[status.connection?.state] ?? "未连接")}</span></div>`
      + `<div class="setting-row"><span>${t("个人私聊")}</span><span>${status.paired ? t("已配对") : t("尚未配对")}</span></div>`
      + (status.appId ? `<div class="setting-row"><span>App ID</span><code data-user-text>${escape(status.appId)}</code></div>` : '')
      + (status.connection?.error ? `<p class="error" role="alert">${escape(t(status.connection.error))}</p>` : '')
      + (status.deliveryError || status.workerError ? `<p class="error" role="alert">${escape(t((status.deliveryError ?? status.workerError).message))}</p>` : '')
      + (status.undelivered > 0 ? `<p class="muted tiny">${escape(AieyesI18n.t("{0} 条消息未送达", status.undelivered))}</p>` : '')
      + `<div class="actions"><button type="button" data-channel-configure>${t("配置飞书")}</button><button type="button" data-channel-pair ${!status.enabled || !status.hasSecret || status.paired ? 'disabled' : ''}>${t("创建配对码")}</button>${status.paired ? `<button type="button" data-channel-unpair>${t("解除配对")}</button>` : ''}</div>`;
  }
  function pairingHTML() {
    if (!pairing || pairing.expiresAt * 1000 <= Date.now() || status?.paired) return '';
    return `<div class="channel-pairing"><p>${t("在飞书中私聊机器人，发送以下指令；配对码 10 分钟内有效。")}</p><code data-user-text>${escape(pairing.message)}</code><button type="button" data-channel-copy>${t("复制配对指令")}</button></div>`;
  }
  function commandsHTML() {
    if (!commands.length) return `<p class="muted">${t("暂无渠道指令")}</p>`;
    return commands.slice(0, 20).map((command) => `<div class="channel-command" data-channel-command="${escape(command.id)}"><div class="between"><code>${escape(command.id)}</code><span>${t(statuses[command.status] ?? "未知状态")}</span></div><p class="channel-message" data-user-text>${escape(command.body)}</p>${command.reason ? `<p class="muted tiny">${escape(t(command.reason))}</p>` : ''}<div class="actions">${command.status === 'blocked' ? `<button type="button" data-channel-retry>${t("重试")}</button>` : ''}${['blocked', 'queued', 'automation'].includes(command.status) ? `<button type="button" data-channel-cancel>${t("取消")}</button>` : ''}</div></div>`).join('');
  }
  function sessionsHTML() {
    if (!sessions.length) return `<p class="muted">${t("启用渠道后，这里显示已监测的会话。")}</p>`;
    return sessions.slice(0, 30).map((session) => `<div class="setting-row"><span class="channel-session-title" data-user-text>${escape(session.title)}</span><small class="muted">${escape(t(session.control))}</small></div>`).join('');
  }
  function body() {
    return `<section class="card" id="channel-settings" data-settings-section="channels"><h2>${t("飞书会话控制")}</h2><p class="muted">${t("在个人私聊中回复问题，或在中断、完成后继续原会话。支持本机与 SSH，按 Agent 和终端能力接入。")}</p><div data-channel-status>${statusHTML()}</div><div data-channel-pairing>${pairingHTML()}</div><div data-channel-error role="alert">${escape(error)}</div></section>`
      + `<section class="card"><h2>${t("接入步骤")}</h2><ol class="channel-steps"><li>${t("在飞书开放平台创建企业自建应用并启用机器人。")}</li><li>${t("订阅 im.message.receive_v1，选择长连接，并开通接收单聊消息和以应用身份发送消息的权限。")}</li><li>${t("发布应用，在此保存 App ID 与 App Secret，再生成配对码完成私聊绑定。")}</li></ol><p class="muted tiny">${t("仅向已配对账号发送问题与最终回复。消息和指令在本机保留 24 小时；退出 Aieyes 或电脑休眠时暂停服务。")}</p></section>`
      + `<section class="card"><h2>${t("渠道指令")}</h2><p class="muted tiny">${t("已提交或结果不确定的指令不会自动重发。会话已改变时，请发送新指令。")}</p><div data-channel-commands>${commandsHTML()}</div></section>`
      + `<section class="card"><h2>${t("可关联会话")}</h2><div data-channel-sessions>${sessionsHTML()}</div></section>`;
  }
  function repaint() {
    if (!visible()) return;
    const root = document.querySelector('#channel-settings');
    const patch = (selector, html) => {const element = document.querySelector(selector);if (element && element.innerHTML !== html && !element.contains(document.activeElement)) element.innerHTML = html;};
    patch('[data-channel-status]', statusHTML());patch('[data-channel-pairing]', pairingHTML());
    patch('[data-channel-commands]', commandsHTML());patch('[data-channel-sessions]', sessionsHTML());
    root.querySelector('[data-channel-error]').textContent = error;
    handlers();
  }
  async function refresh() {
    if (loading || !visible() || document.hidden) return;
    loading = true;
    try {
      const results = await Promise.all([api('channels.status'), api('channels.commands'), api('channels.sessions')]);
      status = results[0];commands = results[1]?.commands ?? [];sessions = results[2]?.sessions ?? [];error = '';
      if (status?.paired) pairing = null;
    } catch (e) {error = e.message ?? String(e);}
    finally {loading = false;repaint();}
  }
  function configure() {
    if (!status) return;
    const current = { ...status };
    const html = `<label class="quota-confirm"><input type="checkbox" name="channelEnabled" ${current.enabled ? 'checked' : ''}>${t("启用飞书会话控制")}</label>`
      + field('channelAppId', 'App ID', current.appId, 'cli_…')
      + field('channelAppSecret', 'App Secret', '', current.hasSecret ? t("已保存，留空保留") : t("输入应用密钥"), 'password')
      + `<label class="quota-confirm"><input type="checkbox" name="channelTerminal" ${current.allowTerminal ? 'checked' : ''}>${t("允许按终端适配输入")}</label><p class="muted tiny">${t("优先使用原生接口；终端输入需要准确定位原进程和分屏。桌面锁定、权限不足或定位不明确时保留指令。")}</p>`;
    showEditor(t("配置飞书"), html, async (form) => {
      status = await api('channels.configure', { revision: current.revision, enabled: form.get('channelEnabled') === 'on', appId: String(form.get('channelAppId') ?? '').trim(), appSecret: String(form.get('channelAppSecret') ?? ''), allowTerminal: form.get('channelTerminal') === 'on' });
      pairing = null;error = '';repaint();void refresh();
    });
    const secret = document.querySelector('#field-channelAppSecret');if (secret) secret.autocomplete = 'new-password';
  }
  async function action(button, operation) {
    button.disabled = true;
    try {await operation();error = '';}
    catch (e) {error = e.message ?? String(e);}
    finally {button.disabled = false;repaint();}
  }
  function handlers() {
    const configureButton = document.querySelector('[data-channel-configure]');if (configureButton) configureButton.onclick = configure;
    const pair = document.querySelector('[data-channel-pair]');if (pair) pair.onclick = () => action(pair, async () => {pairing = await api('channels.pair');});
    const unpair = document.querySelector('[data-channel-unpair]');if (unpair) unpair.onclick = () => action(unpair, async () => {status = await api('channels.unpair', { revision: status.revision });pairing = null;commands = [];});
    const copy = document.querySelector('[data-channel-copy]');if (copy) copy.onclick = () => action(copy, async () => {await navigator.clipboard.writeText(pairing.message);notify(t("已复制"));});
    document.querySelectorAll('[data-channel-retry],[data-channel-cancel]').forEach((button) => button.onclick = () => action(button, async () => {
      const id = button.closest('[data-channel-command]').dataset.channelCommand;
      await api(button.hasAttribute('data-channel-retry') ? 'channels.retry' : 'channels.cancel', { id });
      const next = await api('channels.commands');commands = next.commands ?? [];
    }));
  }
  function bind() {
    handlers();void refresh();
    if (!timer) timer = setInterval(() => {if (visible()) void refresh();else {clearInterval(timer);timer = null;}}, 5000);
  }
  window.AieyesSettingsPages?.register('channels', { body, bind });
})();
