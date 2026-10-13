/* Only public connection metadata enters settings. Keys are sent once to the target. */
window.AieyesCodexAPI = (() => {
  const e = value => escapeHTML(value ?? '');
  const sources = () => (state.settings?.sources ?? []).filter(s => s.provider === 'codex');
  const connections = () => state.settings?.codexApis ?? [];
  const statuses = new Map(), pending = new Set();
  let busy = false, notice = '', error = '';
  const paint = () => { if (state.page === 'settings' && state.settingsTab === 'integrations' && state.settingsAgent === 'codex') renderSettings(false); };
  const location = source => source ? (source.hostId ? state.settings.hosts.find(h => h.id === source.hostId)?.name || source.hostId : '本机') + ' · ' + source.name : '来源已移除';
  async function status(sourceId) {
    const value = await api('codexApi.status', {sourceId});
    statuses.set(sourceId, value);
    return value;
  }
  function html() {
    if (!sources().length && !connections().length) return '';
    return '<section class="card codex-api-settings"><div class="between"><h2>Codex · API 连接</h2><button data-codex-api="add" ' + (busy || !sources().length ? 'disabled' : '') + '>添加 API</button></div>' +
      '<p class="muted">支持官方 OpenAI 与兼容 Responses API 的接口。API 会话可与订阅同时运行，并共用对话历史。</p>' +
      (error ? '<p class="error" role="alert">' + e(error) + '</p>' : '') +
      (notice ? '<p role="status">' + e(notice) + '</p>' : '') +
      connections().map(c => {
        const source = sources().find(s => s.id === c.sourceId), info = statuses.get(c.sourceId);
        const active = info?.defaultProvider?.startsWith('aieyes_api_' + c.id + '_');
        return '<div class="api-connection"><div class="list-row"><div class="row-body"><strong>' + e(c.name) +
          (active ? ' · 默认 API' : '') + '</strong><small>' + e(location(source)) + ' · ' + e(c.model) +
          '</small><small>' + e(c.baseUrl) + '</small></div><button data-codex-api="edit" data-api-id="' + e(c.id) + '" ' + (busy ? 'disabled' : '') + '>设置</button></div>' +
          '<div class="actions">' + [['probe','测试连接'],['command','启动 / 续聊命令'],['default','设为默认'],['remove','移除']].map(([action,label]) =>
            '<button data-codex-api="' + action + '" data-api-id="' + e(c.id) + '" ' + (busy || !source || action === 'default' && active ? 'disabled' : '') + '>' + label + '</button>').join('') + '</div></div>';
      }).join('') +
      (!connections().length ? '<p class="muted">尚未添加 API 连接。密钥保存在所选设备。</p>' : '') +
      [...new Set(connections().map(c => c.sourceId))].map(id => {
        const info = statuses.get(id);
        return '<div class="list-row"><span class="row-body tiny muted">' + e(location(sources().find(s => s.id === id))) + ' · ' +
          (info ? (info.ready ? '计量服务可用' : '计量服务未就绪') : '状态待检查') + '</span><button data-codex-api="refresh" data-api-source="' + e(id) + '" ' + (busy ? 'disabled' : '') + '>检查服务</button>' +
          '<button data-codex-api="subscription" data-api-source="' + e(id) + '" ' + (busy || !info?.defaultProvider?.startsWith('aieyes_api_') ? 'disabled' : '') + '>切回订阅</button></div>';
      }).join('') + '</section>';
  }
  function bind() {
    for (const id of new Set(connections().map(c => c.sourceId))) {
      if (statuses.has(id) || pending.has(id)) continue;
      pending.add(id);
      void status(id).catch(err => { statuses.set(id, {ready:false}); error = String(err.message ?? err); }).finally(() => { pending.delete(id); paint(); });
    }
  }
  function edit(existing) {
    const c = structuredClone(existing ?? {id:'',sourceId:sources()[0]?.id,name:'OpenAI API',baseUrl:'https://api.openai.com/v1',model:'',effort:'',proxy:{mode:'system',url:''},prices:[],revision:''});
    let configRevision = null, generation = 0, priceIndex = 0;
    const priceRow = p => {
      const i = priceIndex++;
      return '<fieldset class="api-price-row" data-api-price-row><legend>模型价格 · USD / 百万 Token</legend>' +
        field('api-price-id-' + i, '模型 ID', p.id || c.model) +
        ['input','output','cacheRead','cacheWrite'].map((key,j) => field('api-price-' + key + '-' + i, ['输入','输出','缓存读取','缓存写入'][j], p[key] == null ? '' : p[key] * 1000000, '未定价', 'number')).join('') +
        '<button type="button" data-api-remove-price>移除此价格</button></fieldset>';
    };
    showEditor(existing ? '设置 Codex API' : '添加 Codex API',
      select('apiSource','保存到设备',sources().map(s => [s.id,location(s)]),c.sourceId) +
      field('apiName','连接名称',c.name) + field('apiBaseUrl','Base URL',c.baseUrl,'https://api.openai.com/v1') +
      field('apiKey','API Key','','留空保留已保存的密钥','password') +
      field('apiModel','模型 ID',c.model,'填写接口支持的模型 ID') +
      select('apiEffort','推理强度',[['','沿用默认'],...['none','minimal','low','medium','high','xhigh','max','ultra'].map(v => [v,v])],c.effort) +
      select('apiProxyMode','上游连接方式',[['system','目标设备系统代理'],['direct','直连'],['custom','指定代理']],c.proxy?.mode || 'system') +
      field('apiProxyUrl','代理地址',c.proxy?.url || '','http://127.0.0.1:7890') +
      '<p class="tiny muted">远程设备支持 HTTP(S) 代理。价格只用于此 API 的成本估算；留空会显示未计价用量。</p>' +
      '<div id="api-prices">' + (c.prices.length ? c.prices : [{id:c.model}]).map(priceRow).join('') + '</div><button type="button" id="api-add-price">添加模型价格</button>' +
      '<p id="api-config-status" class="tiny muted" role="status">正在读取设备配置…</p><button type="button" id="api-config-reload">重新读取设备状态</button>',
      async f => {
        if (!configRevision) throw new Error('请先读取设备状态');
        const number = input => {
          if (!input.value.trim()) return null;
          const value = Number(input.value);
          if (!Number.isFinite(value) || value < 0) throw new Error('价格必须为非负数');
          return value / 1000000;
        };
        const prices = [...document.querySelectorAll('[data-api-price-row]')].map(row => {
          const inputs = [...row.querySelectorAll('input')], id = inputs[0].value.trim();
          if(inputs.every(input=>!input.value.trim())) return null;
          if (!id) throw new Error('请填写价格对应的模型 ID，或移除该行');
          return {id,name:id,input:number(inputs[1]),output:number(inputs[2]),cacheRead:number(inputs[3]),cacheWrite:number(inputs[4]),fetchedAt:Math.floor(Date.now()/1000)};
        }).filter(Boolean);
        const connection = {...c,sourceId:existing ? c.sourceId : f.get('apiSource'),name:f.get('apiName').trim(),baseUrl:f.get('apiBaseUrl').trim(),model:f.get('apiModel').trim(),effort:f.get('apiEffort'),proxy:{mode:f.get('apiProxyMode'),url:f.get('apiProxyUrl').trim()},prices};
        const saved = await api('codexApi.save',{connection,apiKey:f.get('apiKey'),configRevision});
        document.getElementById('field-apiKey').value = '';
        window.AieyesAgentSettings.accept(saved.settings);
        statuses.delete(connection.sourceId); error = ''; notice = 'API 已保存并部署。可以复制启动命令，或将它设为后续会话的默认方式。';
        markSaved(); paint(); void loadDashboard();
      });
    document.getElementById('field-apiSource').disabled = Boolean(existing);
    document.getElementById('field-apiKey').autocomplete = 'new-password';
    const bindPrices = () => {
      document.querySelectorAll('[data-api-remove-price]').forEach(button => button.onclick = () => button.closest('[data-api-price-row]').remove());
      document.querySelectorAll('[data-api-price-row] input[type=number]').forEach(input => { input.min = '0'; input.step = 'any'; });
    };
    bindPrices();
    document.getElementById('api-add-price').onclick = () => { document.getElementById('api-prices').insertAdjacentHTML('beforeend',priceRow({id:document.getElementById('field-apiModel').value})); bindPrices(); };
    const readStatus = async () => {
      const epoch = ++generation, id = document.getElementById('field-apiSource').value;
      configRevision = null;
      try {
        const result = await status(id);
        if (generation !== epoch || !document.getElementById('api-config-status')) return;
        configRevision = result.configRevision;
        document.getElementById('api-config-status').textContent = '设备配置已读取；保存后计量服务可在退出 Aieyes 后继续运行。';
      } catch (err) { if (generation === epoch && document.getElementById('api-config-status')) document.getElementById('api-config-status').textContent = String(err.message ?? err); }
    };
    document.getElementById('field-apiSource').onchange = readStatus;
    document.getElementById('api-config-reload').onclick = readStatus;
    void readStatus();
  }
  async function perform(action, c, sourceId) {
    if (busy) return;
    busy = true; error = ''; notice = ''; paint();
    try {
      const params = {sourceId,id:c?.id,revision:c?.revision};
      if (action === 'refresh') { await status(sourceId); notice = '已检查计量服务。'; }
      if (action === 'probe') { const result = await api('codexApi.probe',params); notice = result.message + (result.httpStatus ? ' · HTTP ' + result.httpStatus : ''); }
      if (action === 'command') {
        const result = await api('codexApi.command',params);
        const dialog = quotaDialog('启动 API 会话', '<p>在所选设备的终端执行。续聊前先结束当前轮次，再执行续聊命令并选择原对话。</p><pre class="api-launch-command">' + e(result.launchCommand) + '</pre><button id="api-copy-command">复制启动命令</button><pre class="api-launch-command">'+e(result.resumeCommand)+'</pre><button id="api-copy-resume">复制续聊命令</button>');
        dialog.querySelector('#api-copy-command').onclick = async event => { await navigator.clipboard.writeText(result.launchCommand); event.target.textContent = '已复制'; };
        dialog.querySelector('#api-copy-resume').onclick = async event => { await navigator.clipboard.writeText(result.resumeCommand); event.target.textContent = '已复制'; };
      }
      if (action === 'default' || action === 'subscription') {
        const current = await status(sourceId);
        await api('codexApi.default',{...params,id:action === 'subscription' ? 'subscription' : c.id,configRevision:current.configRevision});
        await status(sourceId); notice = '默认方式已更新，将用于后续启动或恢复的会话。已有会话继续使用原方式。';
      }
    } catch (err) { error = String(err.message ?? err); }
    finally { busy = false; paint(); }
  }
  document.addEventListener('click',event => {
    const button = event.target.closest('[data-codex-api]');
    if (!button || busy) return;
    const action = button.dataset.codexApi, c = connections().find(v => v.id === button.dataset.apiId);
    if (action === 'add' || action === 'edit') return edit(c);
    if (action === 'remove' && c) {
      showEditor('移除 API 连接', '<p>移除「' + e(c.name) + '」的连接与设备密钥。正在使用此 API 的会话会失去连接，历史用量与费用记录保留。</p>', async () => {
        const saved = await api('codexApi.remove',{sourceId:c.sourceId,id:c.id,revision:c.revision});
        window.AieyesAgentSettings.accept(saved.settings); statuses.delete(c.sourceId); markSaved(); paint();
      }); return;
    }
    void perform(action,c,button.dataset.apiSource || c?.sourceId);
  });
  function summaryHTML(d, folded=false) {
    const b = d.billingSummary;
    if (!b || !(b.apiTokens || b.subscriptionTokens || b.unknownTokens || b.apiUnmeteredTokens || b.incompleteRequests)) return '';
    const cost = (value,tokens,priced) => tokens && !priced ? '—' : money(value || 0) + ' USD';
    const coverage = (tokens,priced) => compact(priced || 0) + ' / ' + compact(tokens || 0) + ' Token 已计价';
    return '<section class="card billing-breakdown">'+(folded?'<details data-agent-detail="api-billing"><summary>Codex · API 估算成本 '+cost(b.apiCost,b.apiTokens,b.apiPricedTokens)+'</summary>':'<h3>Codex 计费分项</h3>')+'<div class="billing-grid">' +
      '<div><span>API 估算成本</span><strong>' + cost(b.apiCost,b.apiTokens,b.apiPricedTokens) + '</strong><small>' + coverage(b.apiTokens,b.apiPricedTokens) + '</small></div>' +
      '<div><span>订阅 API 等价估值</span><strong>' + cost(b.subscriptionEquivalentCost,b.subscriptionTokens,b.subscriptionPricedTokens) + '</strong><small>' + coverage(b.subscriptionTokens,b.subscriptionPricedTokens) + '</small></div>' +
      '<div><span>未归属用量</span><strong>' + compact(b.unknownTokens || 0) + ' Token</strong><small>不参与混用订阅估值</small></div></div>' +
      (b.incompleteRequests || b.apiUnmeteredTokens ? '<p class="tiny warning">有 ' + (b.incompleteRequests || 0) + ' 个不完整请求、' + compact(b.apiUnmeteredTokens || 0) + ' 个未计量 API Token；费用可能不完整。</p>' : '') + (folded?'</details>':'')+'</section>';
  }
  return {html,bind,summaryHTML};
})();
