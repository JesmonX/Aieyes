/* One application measurement shared by both webviews; drafts stay local. */
(() => {
  let snapshot = null,busy = false,attempted = 0,revision = 0;
  const label = (result) => !result ? AieyesI18n.t("连接未测试") : result.status === 'unstable' ? AieyesI18n.t("连接不稳定") : result.status === 'failed' ? AieyesI18n.t("连接失败") : (result.mode === 'direct' ? AieyesI18n.t("直连 ") : result.mode === 'system' ? AieyesI18n.t("系统 ") : AieyesI18n.t("代理 ")) + result.averageMs + ' ms';
  const detail = result => result ? AieyesI18n.t("应用出站连接测试（不代表 SSH 或所有账户可用）\n{0}\n测试于 {1}", result.sites.map(site => site.url + ' · ' + (site.latencyMs != null ? site.latencyMs + ' ms' : AieyesI18n.message(site.error))).join('\n'), new Date(result.testedAt * 1000).toLocaleString(AieyesI18n.locale, AieyesI18n.clock)) : AieyesI18n.t("测试当前应用连接方式");
  function accept(next) {snapshot = Array.isArray(next?.sites) ? next : null;draw();}
  function draw() {
    const button = document.querySelector('#panel-latency');if (button) {button.textContent = busy ? AieyesI18n.t("出站测试中…") : AieyesI18n.t("测试出站连接");button.title = detail(snapshot);button.disabled = busy;}
    const activity = document.querySelector('#activity');if (activity && PANEL) {const base = activity.textContent.split(AieyesI18n.t(" · 出站 · "))[0],text = AieyesI18n.t("{0} · 出站 · {1}", base, busy ? AieyesI18n.t("测试中…") : label(snapshot));activity.textContent = text;activity.title = text + '\n' + detail(snapshot);activity.setAttribute('aria-label', AieyesI18n.t(state.page === 'servers' ? "{0}；点击刷新服务器" : "{0}；点击刷新记录", text));}
  }
  async function refresh(force = false) {
    if (busy || !force && Date.now() - attempted < 300000) return;
    busy = true;attempted = Date.now();draw();
    const testedRevision = revision;
    try {const measured = await api('network.test', { force });if (revision === testedRevision) accept(measured);} catch (error) {if (revision === testedRevision && typeof notify === 'function') notify(String(error), 'error');} finally
    {busy = false;draw();}
  }
  window.AieyesNetwork = { label, detail, refresh, draw, invalidate() {revision++;snapshot = null;attempted = 0;draw();} };
  Promise.resolve(appReady).then(async () => {
    await window.__TAURI__.event.listen('desktop:network', ({ payload }) => {attempted = Date.now();accept(payload);});
    try {accept(await api('network.status'));} catch {accept(null);}
    const button = document.querySelector('#panel-latency');if (button) button.onclick = () => refresh(true);
    if (PANEL && document.body.dataset.view === 'panel') refresh();
    // Only the main window owns the interval; opening a panel requests the same cached result.
    if (!PANEL) setInterval(() => {if (state.panelOpen) refresh();}, 1000);
  }).catch((error) => {if (typeof notify === 'function') notify(String(error), 'error');});
})();
