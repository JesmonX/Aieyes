/* A single live application surface backed by a persistent, isolated mock. */
(() => {
  const $ = selector => document.querySelector(selector),frame = $('#app'),params = new URLSearchParams(location.search);
  const validScenario = v => [...$('#scenario').options].some(o => o.value === v);
  const validState = (saved, seed) => saved && saved.version === seed.version && saved.platform === seed.platform && saved.scenario === seed.scenario && Number.isFinite(saved.stamp)
    && ['sources','accounts','hosts'].every(k => Array.isArray(saved.settings?.[k]) && saved.settings[k].every(row => row && typeof row.id === 'string'))
    && ['light','dark','system'].includes(saved.settings?.appearance?.theme) && ['zh-CN','en'].includes(saved.settings?.appearance?.language)
    && saved.dashboard?.summary?.tokens && Array.isArray(saved.dashboard.quotas) && Array.isArray(saved.prices) && Array.isArray(saved.hosts)
    && Array.isArray(saved.wakeups) && Array.isArray(saved.sessions) && Array.isArray(saved.network?.sites);
  let platform = params.get('platform') === 'linux' ? 'linux' : 'windows',scenario = validScenario(params.get('scenario')) ? params.get('scenario') : 'workspace';
  let surface = params.get('surface') === 'panel' ? 'panel' : 'main',model,generation = 0,request = 0,pendingNavigation = null,maximized = false,restoreSize = null,pathResolve;
  const entries = [];let logTask,storageWarning = '';
  const scope = () => `${platform}.${scenario}`,stateKey = () => `aieyes.preview.v1.state.${scope()}`;
  function note(text, error = false) {$('#status').textContent = text;$('#status').dataset.error = String(error);}
  function log(entry) {
    if (entry.result !== undefined) {const result = JSON.stringify(entry.result);if (result.length > 14000) entry.result = result.slice(0, 14000) + '\n…（结果已截断）';}
    entries.unshift(entry);entries.length = Math.min(200, entries.length);
    if (!logTask) logTask = requestAnimationFrame(renderLogs);
  }
  function renderLogs() {
    logTask = null;$('#log-count').textContent = entries.length;const errors = entries.filter(e => e.error).length;$('#error-count').textContent = errors ? `${errors} 条异常` : '';
    const list = $('#log-list');if (!$('#logs').open) return;
    const expanded = new Set([...list.querySelectorAll('details[open]')].map(el => Number(el.dataset.startedAt)));
    list.replaceChildren(...entries.map(entry => {
      const row = document.createElement('details');row.className = 'log-entry';row.dataset.error = String(Boolean(entry.error));row.dataset.startedAt = entry.startedAt;row.open = expanded.has(entry.startedAt);
      const summary = document.createElement('summary'),time = document.createElement('time'),duration = document.createElement('span');
      time.textContent = new Date(entry.startedAt).toLocaleTimeString();duration.className = 'duration';duration.textContent = `${entry.duration || 0} ms`;
      summary.append(time,document.createTextNode(`${entry.error ? '✕ ' : '✓ '}${entry.method}`),duration);
      const pre = document.createElement('pre');const value = JSON.stringify(entry, null, 2);pre.textContent = value.slice(0, 14000) + (value.length > 14000 ? '\n…（结果已截断）' : '');row.append(summary,pre);return row;
    }));
  }
  function controls() {
    document.querySelectorAll('[data-platform]').forEach(el => el.setAttribute('aria-pressed', String(el.dataset.platform === platform)));
    document.querySelectorAll('[data-surface]').forEach(el => el.setAttribute('aria-pressed', String(el.dataset.surface === surface)));
    $('#scenario').value = scenario;$('#size').disabled = surface === 'panel';
    if (model) {$('#theme').value = model.data.settings.appearance.theme;$('#language').value = model.data.settings.appearance.language;}
    $('#boundary').textContent = platform === 'linux' ? 'Linux：预览真实应用页面；外层标题栏为示意。GTK 桌宠、系统托盘、Wayland / X11 定位与窗口材质需 Linux 环境验收。' : 'Windows：预览真实应用页面与自绘标题栏。系统桌宠、托盘、DWM 材质与真实 DPI 需 Windows 环境验收。';
    const url = new URL(location.href);url.searchParams.set('platform', platform);url.searchParams.set('scenario', scenario);url.searchParams.set('surface', surface);history.replaceState(null, '', url);
  }
  function sizePreset() {
    const size = surface === 'panel' ? [450,720] : $('#size').value === 'minimum' ? [640,440] : $('#size').value === 'wide' ? [1440,900] : platform === 'linux' ? [1120,800] : [1294,840];
    $('#width').value = size[0];$('#height').value = size[1];fit();
  }
  function fit() {
    const width = Math.max(320, Math.min(3840, Number($('#width').value) || 1294)),height = Math.max(240, Math.min(2160, Number($('#height').value) || 840));
    frame.width = width;frame.height = height;
    const title = platform === 'linux' && surface === 'main';$('#linux-title').hidden = !title;
    const outerWidth = width + 2,outerHeight = height + 2 + (title ? 32 : 0),stage = $('#stage');
    const zoom = $('#zoom').value === 'fit' ? Math.max(.15, Math.min(1, (stage.clientWidth - 52) / outerWidth, (stage.clientHeight - 52) / outerHeight)) : Number($('#zoom').value);
    $('#window').style.width = outerWidth + 'px';$('#window').style.transform = `scale(${zoom})`;$('#scaled').style.width = outerWidth * zoom + 'px';$('#scaled').style.height = outerHeight * zoom + 'px';
    $('#viewport-label').textContent = `${platform === 'linux' ? 'Linux' : 'Windows'} · ${width} × ${height} CSS px · 画布 ${Math.round(zoom * 100)}%（非系统 DPI）`;
  }
  function mount() {
    if (!model) return;
    generation++;maximized = false;restoreSize = null;
    if (pathResolve) {pathResolve(null);pathResolve = null;$('#path-dialog').close();}
    $('#closed').hidden = true;$('#scaled').hidden = false;
    model.desktop.panelOpen = surface === 'panel';controls();
    frame.dataset.ready = 'false';frame.src = `/app/${surface === 'panel' ? 'floating.html' : 'index.html'}?generation=${generation}`;
    note('正在载入真实应用页面…');fit();
  }
  async function load(reset = false) {
    const token = ++request;generation++;frame.dataset.ready = 'false';frame.src = 'about:blank';model = null;storageWarning = '';note('正在生成模拟场景…');
    if (pathResolve) {pathResolve(null);pathResolve = null;$('#path-dialog').close();}
    const key = stateKey();controls();
    try {
      const response = await fetch(`/preview/seed?platform=${platform}&scenario=${scenario}`);if (!response.ok) throw new Error(await response.text());
      const seed = await response.json();if (token !== request) return;
      let data = seed.data,warning = '';
      if (reset) {
        localStorage.removeItem(key);const prefix = `aieyes.preview.v1.ui.${scope()}.`;Object.keys(localStorage).filter(k => k.startsWith(prefix)).forEach(k => localStorage.removeItem(k));
      } else {
        try {
          const saved = JSON.parse(localStorage.getItem(key) || 'null');
          if (validState(saved, data)) data = saved;
          else if (saved) warning = '旧版本预览存储已忽略；当前使用初始场景。';
        } catch {warning = '预览存储无法读取；当前使用初始场景。';}
      }
      const instance = new AieyesPreviewModel.Model(data, seed.methods, seed.providers, {
        active: () => model === instance,
        save: value => {try {localStorage.setItem(key, JSON.stringify(value));} catch {throw new Error('预览存储写入失败，请清理浏览器空间后重试');}},
        log: entry => {log(entry);if (entry.error && /未实现的预览调用/.test(entry.error)) note(entry.error, true);controls();},
        navigate: view => {pendingNavigation = view;frame.dataset.ready = 'false';note('正在打开主窗口…');setTimeout(() => {if (model !== instance) return;surface = 'main';$('#size').value = 'default';sizePreset();mount();},0);},
        window: action => window.PreviewHost.windowAction(generation, action),
        pickPath: p => new Promise(resolve => {pathResolve = resolve;$('#mock-path').value = p.initial && p.initial !== '~' ? p.initial : platform === 'windows' ? 'C:\\Users\\Preview\\.codex' : '/home/preview/.codex';$('#path-dialog').showModal();$('#mock-path').focus();}),
      });
      model = instance;model.delay = Number($('#delay').value);model.fail = $('#failure').value;
      try {model.persist();} catch (error) {warning = error.message;}
      pendingNavigation = null;storageWarning = warning;sizePreset();mount();if (warning) note(warning, true);
    } catch (error) {if (token === request) note(error.message, true);}
  }
  window.PreviewHost = {
    get model() {return model;},get maximized() {return maximized;},
    attach(id) {if (!model || id !== generation) return null;return { id, key: scope(), platform, appearance: structuredClone(model.data.settings.appearance) };},
    async invoke(id, command, args) {if (id !== generation || !model) throw new Error('预览已重载');return model.invoke(command, args);},
    listen(id, name, fn) {if (id !== generation || !model) return () => {};return model.listen(name, event => {if (id === generation) fn(event);});},
    report(id, error) {if (id === generation) {log({ method: 'page:error', startedAt: Date.now(), error, duration: 0 });note('页面异常，请展开调用与错误查看详情。', true);}},
    async ready(id) {
      if (id !== generation) return;
      const view = pendingNavigation;pendingNavigation = null;
      try {
        const app = frame.contentWindow.AieyesApp;
        if (view?.target) await app.openSettings(view.target);
        else if (view?.action) {
          const action = view.action;if (['settings','prices'].includes(action)) await app.openSettings(action === 'prices' ? { category: 'prices' } : {});
          else if (action !== 'open') await app.openSetupInMain(action === 'add-host' ? 'hosts' : action === 'add-quota' ? 'quota' : 'sources');
        } else if (view) await app.navigatePage(view.page || 'agent', view);
        if (id === generation) {frame.dataset.ready = 'true';note('预览已就绪 · 所有数据均为模拟数据，保存仅影响当前平台和场景。' + (storageWarning ? ' ' + storageWarning : ''), Boolean(storageWarning));}
      } catch (error) {this.report(id, String(error));}
    },
    windowAction(id, action) {
      if (id !== generation) return;
      log({ method: `window.${action}`, startedAt: Date.now(), duration: 0 });
      if (action === 'maximize') {
        if (!maximized) {restoreSize = [$('#width').value,$('#height').value];$('#width').value = 1440;$('#height').value = 900;}
        else {$('#width').value = restoreSize[0];$('#height').value = restoreSize[1];}
        maximized = !maximized;fit();frame.contentWindow.dispatchEvent(new Event('resize'));
      } else {$('#scaled').hidden = true;$('#closed').hidden = false;note('窗口已在工作台中收起，点击“恢复预览”继续。');}
    },note,
  };
  document.querySelectorAll('[data-platform]').forEach(b => b.onclick = () => {if (platform !== b.dataset.platform) {platform = b.dataset.platform;$('#size').value = 'default';load();}});
  document.querySelectorAll('[data-surface]').forEach(b => b.onclick = () => {surface = b.dataset.surface;$('#size').value = 'default';sizePreset();mount();});
  $('#scenario').onchange = e => {scenario = e.target.value;load();};$('#size').onchange = () => {if ($('#size').value !== 'custom') sizePreset();};
  for (const id of ['width','height']) $('#' + id).onchange = () => {$('#size').value = 'custom';fit();};
  $('#zoom').onchange = fit;new ResizeObserver(fit).observe($('#stage'));
  for (const id of ['theme','language']) $('#' + id).onchange = async e => {
    if (!model) return;
    try {const next = structuredClone(model.data.settings);next.appearance[id] = e.target.value;await model.invoke('engine_call', { method: 'settings.patch', params: { base: structuredClone(model.data.settings), settings: next } });frame.contentWindow.AieyesTheme?.apply(next.appearance);} catch (error) {note(error.message, true);controls();}
  };
  $('#delay').onchange = () => {if (model) model.delay = Number($('#delay').value);};$('#failure').onchange = () => {if (model) model.fail = $('#failure').value;};
  $('#reload').onclick = mount;$('#restore').onclick = () => {$('#closed').hidden = true;$('#scaled').hidden = false;if (surface === 'panel') {model.desktop.panelOpen = true;model.emit('desktop:panel', model.desktop);}fit();};
  $('#reset').onclick = () => $('#reset-dialog').showModal();$('#cancel-reset').onclick = () => $('#reset-dialog').close();$('#confirm-reset').onclick = () => {$('#reset-dialog').close();load(true);};
  function resolvePath(value) {const resolve = pathResolve;pathResolve = null;$('#path-dialog').close();resolve?.(value);}
  $('#path-form').onsubmit = e => {e.preventDefault();resolvePath($('#mock-path').value.trim() || null);};$('#cancel-path').onclick = () => resolvePath(null);$('#path-dialog').oncancel = e => {e.preventDefault();resolvePath(null);};
  $('#logs').ontoggle = renderLogs;$('#clear-logs').onclick = () => {entries.length = 0;renderLogs();};$('#copy-logs').onclick = async () => {try {await navigator.clipboard.writeText(JSON.stringify(entries, null, 2));note('调试日志已复制。');} catch {note('无法访问剪贴板，请展开日志手动复制。', true);}};
  const changes = new EventSource('/preview/events');let reloadTimer;changes.onmessage = event => {clearTimeout(reloadTimer);reloadTimer = setTimeout(() => {if (event.data === 'workbench') location.reload();else {note('源码已更新，正在重载…');mount();}},150);};
  changes.onerror = () => note('预览服务连接中断；请检查终端，恢复后会自动重连。', true);
  changes.onopen = () => {if (model) note('预览服务已连接。');};
  load();
})();
