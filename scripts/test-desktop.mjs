import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { readFileSync } from 'node:fs';

const localeRuntime = 'if(typeof window!=="undefined"&&!window.addEventListener)window.addEventListener=()=>{};\n' + readFileSync(new URL('../apps/desktop/web/i18n.js', import.meta.url), 'utf8');
const read = name => localeRuntime + '\n' + readFileSync(new URL(`../apps/desktop/web/${name}`, import.meta.url), 'utf8');
const flush = () => new Promise(resolve => setImmediate(resolve));
test('all usage surfaces discard legacy account filters but preserve analysis dimensions', () => {
  const context={structuredClone};vm.runInNewContext(read('view-state.js'),context);
  for(const scope of ['overview','panel','analysis']) {
    const value=context.AieyesViewState.normalize({provider:'codex',accountKey:'codex:old',sourceId:'source',model:'model',days:30},scope);
    assert.equal(value.accountKey,'');assert.equal(value.provider,'codex');
    assert.equal(value.sourceId,scope==='analysis'?'source':'');
  }
});
test('unverified retained estimates cannot become live value fallbacks', () => {
  const window={};vm.runInNewContext(read('ui-state.js'),{window});
  const old={id:'old',accountKey:'codex:a',status:'completed',startedAt:100,fiveHourValue:999,rollingExcludedReason:'缺少账户证据'};
  assert.equal(window.AieyesUI.canonicalEstimates([old]).length,0);
  assert.equal(window.AieyesUI.estimateIssue(old),'缺少账户证据');
});
test('error presentation keeps known actions and safely exposes unknown diagnostics', () => {
  const window = {};
  vm.runInNewContext(read('ui-state.js'), {window});
  const ui = window.AieyesUI;
  assert.equal(ui.errorPresentation('请先连接订阅账户与运行机器').summary, '请先连接订阅账户与运行机器');
  const raw = 'HTTP 500 <script>alert(1)</script>\nAuthorization: Bearer private-token\nCookie: session=private-cookie; other=secret\n{"api_key":"private-key","refresh_token":"private-refresh"}\nhttps://user:private-password@example.invalid/?access_token=private-query&retry=1\nsk-proj-privatekey1234';
  const jsonCookie = ui.errorPresentation('{"cookie":"session=private-json-cookie; other=private-json-secondary"}');
  assert.equal(jsonCookie.summary, '操作未完成');
  assert(!jsonCookie.details.includes('private-json-cookie') && !jsonCookie.details.includes('private-json-secondary'));
  const result = ui.errorPresentation(raw, '额度查询失败');
  assert.equal(result.summary, '额度查询失败');
  for (const secret of ['private-token','private-cookie','secret','private-key','private-refresh','private-password','private-query','privatekey1234']) assert(!result.details.includes(secret), secret);
  assert.match(result.details, /HTTP 500/);
  assert.match(result.details, /retry=1/);
  const html = ui.errorHTML(raw, '额度查询失败');
  assert(!html.includes('<script>'));
  assert.match(html, /&lt;script&gt;/);
  assert.match(html, /<details class="error-details">/);
  assert.equal(ui.errorPresentation('查询失败，请重试\nHTTP 503').summary, '查询失败，请重试');
});

test('wakeup previews follow shared timezone and DST fixtures', () => {
  const window = {};
  vm.runInNewContext(read('ui-state.js'), {window});
  const fixture = JSON.parse(readFileSync(new URL('fixtures/wakeup-times.json', import.meta.url), 'utf8'));
  for (const row of fixture) {
    const result = window.AieyesUI.nextTargetTime(row.time,row.zone,new Date(row.now));
    assert.equal(result.date?.getTime()??null,row.expected?Date.parse(row.expected):null,row.name);
    assert.equal(result.status,row.status,row.name);
    assert.equal(result.approximate,row.approximate,row.name);
    if(result.approximate)assert.match(window.AieyesUI.targetTimeLocal(row.time,row.zone,new Date(row.now)),/按当前偏移估算/);
  }
});
test('provider icons are bundled as PNG assets', () => {
  for (const provider of ['codex','claude','antigravity','deepseek']) {
    const png = readFileSync(new URL(`../apps/desktop/web/provider-${provider}.png`, import.meta.url));
    assert.equal(png.subarray(0,8).toString('hex'), '89504e470d0a1a0a');
  }
});
test('source availability and freshness follow the shared core rules', () => {
  const window = {};
  vm.runInNewContext(read('ui-state.js'), {window});
  const fixture = JSON.parse(readFileSync(new URL('fixtures/source-availability.json', import.meta.url), 'utf8'));
  for (const row of fixture.cases) {
    const settings = {sources:fixture.sources, agents:row.agents, hosts:row.hosts, refreshSeconds:300};
    assert.deepEqual(fixture.sources.filter(source => window.AieyesUI.sourceEnabled(settings, source)).map(source => source.id), row.expectedEnabled, row.name);
    assert.deepEqual([...window.AieyesUI.usageSources(settings)].map(source => source.id), row.expectedUsage, row.name);
    const dashboard = {sources:fixture.sources.map(source => ({...source, status:row.expectedUsage.includes(source.id)
      ? {updatedAt:900} : {updatedAt:1, error:'old failure', partial:true}}))};
    const actual = window.AieyesUI.freshness(settings, dashboard, 1000);
    assert.deepEqual({...actual}, {total:row.expectedUsage.length, timestamp:row.expectedUsage.length ? 900 : 0, delayed:0, failed:0}, row.name);
  }
});

test('legacy providers normalize at the payload boundary with collision aliases', () => {
  const window = {};
  vm.runInNewContext(read('ui-state.js'), {window});
  const payload = {accountAliases:{'agy:a':'antigravity:legacy-a'},accounts:[{provider:'agy',id:'a',quotaEnabled:true}],sources:[{provider:'agy',accountId:'a'}],quotas:[{provider:'agy',accountId:'a'}],quotaOrder:['agy:a']};
  const next = window.AieyesUI.canonicalPayload(payload);
  assert.equal(next.accounts[0].provider,'antigravity');
  assert.equal(next.accounts[0].id,'legacy-a');
  assert.equal(next.sources[0].accountId,'legacy-a');
  assert.equal(next.quotas[0].accountId,'legacy-a');
  assert.deepEqual([...next.quotaOrder],['antigravity:legacy-a']);
  assert.equal(payload.accounts[0].provider,'agy');
});
test('native and Web share panel preference and valuation window fixtures', () => {
  const window = {};
  vm.runInNewContext(read('contracts.js'), {window});
  vm.runInNewContext(read('ui-state.js'), {window});
  const fixture = JSON.parse(readFileSync(new URL('fixtures/ui-semantics.json', import.meta.url), 'utf8'));
  for (const row of fixture.panels) {
    const preference = row.rawPreference === undefined ? row.preference : window.AieyesUI.decodePanelPreference(row.rawPreference);
    const actual = window.AieyesUI.panelAccounts({accounts:row.accounts},row.order,preference);
    assert.deepEqual(JSON.parse(JSON.stringify(actual)),row.expected,row.name);
  }
  for (const row of fixture.windows) {
    const actual = window.AieyesUI.estimateWindows(row).map(w=>w.id);
    assert.deepEqual([...actual],row.expected,row.name);
  }
});
test('valuation fields follow the current account plan and quota windows', () => {
  const window = {};
  vm.runInNewContext(read('contracts.js'), {window});
  vm.runInNewContext(read('ui-state.js'), {window});
  const fixture = JSON.parse(readFileSync(new URL('fixtures/estimate-presentation.json', import.meta.url), 'utf8'));
  for (const row of fixture.cases) {
    const records = (row.records ?? [{}]).map(record => ({...fixture.record, ...record}));
    assert.equal(window.AieyesUI.estimateLine(records,row.quota,row.group),row.summary,row.name);
    assert.deepEqual([...window.AieyesUI.estimateEntries(records,false,row.quota,row.group)].map(e=>e.field),row.fields,row.name);
    const visible=window.AieyesUI.estimateVisibility(row.quota,records,row.group);
    const help=window.AieyesUI.estimateHelp(row.quota,records,row.group);
    if(!visible.five)assert.doesNotMatch(help,/5h|容量倍率/,row.name);
    if(!visible.direct)assert.doesNotMatch(help,/同期|周额度消耗比例|整周/,row.name);
  }
});
test('panel account choices cap each Agent and preserve explicit empty choices', () => {
  const data=new Map(),window={};
  vm.runInNewContext(read('contracts.js'), {window});
  vm.runInNewContext(read('ui-state.js'),{window,localStorage:{getItem:key=>data.get(key)??null,setItem:(key,value)=>data.set(key,value)}});
  const accounts=[...Array.from({length:20},(_,i)=>({provider:'codex',id:'c'+i,quotaEnabled:true})),...Array.from({length:6},(_,i)=>({provider:'claude',id:'a'+i,quotaEnabled:true}))];
  const order=accounts.map(a=>a.provider+':'+a.id).reverse(),settings={accounts};
  const defaults=window.AieyesUI.panelAccounts(settings,order);
  assert.deepEqual([...defaults.codex],['codex:c19','codex:c18','codex:c17','codex:c16','codex:c15']);
  assert.equal(defaults.claude.length,5);
  data.delete('aieyes.panel.accounts.v2');
  data.set('aieyes.panel.accounts.v1',JSON.stringify({codex:[],claude:['claude:a0','claude:a0','claude:missing','claude:a1']}));
  assert.equal(window.AieyesUI.panelAccounts(settings,order).codex.length,0);
  assert.deepEqual([...window.AieyesUI.panelAccounts(settings,order).claude],['claude:a0','claude:a1']);
  accounts.at(-6).archived=true;
  assert.deepEqual([...window.AieyesUI.panelAccounts(settings,order).claude],['claude:a1']);
  accounts.filter(a=>a.provider==='claude').forEach(a=>a.archived=true);
  assert.deepEqual([...window.AieyesUI.panelAccounts(settings,order).claude],[]);
  data.set('aieyes.panel.accounts.v2', '{"version":2,"providers":null}');
  assert.deepEqual([...window.AieyesUI.panelAccounts(settings,order).codex], [...defaults.codex]);
  data.set('aieyes.panel.accounts.v2', '{');
  assert.deepEqual([...window.AieyesUI.panelAccounts(settings,order).codex], [...defaults.codex]);
});
test('new accounts join every Agent panel without restoring hidden old accounts', () => {
  const data=new Map(),window={};
  vm.runInNewContext(read('contracts.js'),{window});
  vm.runInNewContext(read('ui-state.js'),{window,localStorage:{getItem:key=>data.get(key)??null,setItem:(key,value)=>data.set(key,value)}});
  const ui=window.AieyesUI;
  for(const provider of [...window.AieyesContract.agentProviders,'deepseek-harness','future-agent']){
    data.delete('aieyes.panel.accounts.known.v1');
    const account=id=>({provider,id,quotaEnabled:true}),old={accounts:[account('visible'),account('hidden')]},next={accounts:[...old.accounts,account('new')]};
    data.set('aieyes.panel.accounts.v2',JSON.stringify({version:2,providers:{[provider]:{mode:'custom',keys:[provider+':visible']}}}));
    ui.observePanelAccounts(old);
    ui.observePanelAccounts(next);
    assert.deepEqual([...ui.panelAccounts(next)[provider]],[provider+':visible',provider+':new'],provider);
    data.set('aieyes.panel.accounts.v2',JSON.stringify({version:2,providers:{[provider]:{mode:'custom',keys:[]}}}));
    ui.observePanelAccounts(next);
    assert.deepEqual([...ui.panelAccounts(next)[provider]],[],provider+' repeated load preserves explicit hiding');
  }
  data.clear();
  const five={accounts:Array.from({length:5},(_,i)=>({provider:'codex',id:'a'+i,quotaEnabled:true}))};
  ui.observePanelAccounts(five);
  const next={accounts:[...five.accounts,{provider:'codex',id:'sixth',quotaEnabled:true}]};
  ui.observePanelAccounts(next);
  assert.equal(ui.panelAccounts(next).codex.length,6,'new accounts remain visible beyond five defaults');
  data.clear();data.set('aieyes.panel.accounts.v2',JSON.stringify({version:2,providers:{codex:{mode:'custom',keys:[]}}}));
  ui.observePanelAccounts(next);
  assert.deepEqual([...ui.panelAccounts(next).codex],[],'first upgrade does not reveal previously hidden accounts');
});
test('wakeup source choices consume backend targets and contract capabilities', async () => {
  const source = (id, provider, extra = {}) => ({id, name:id, provider, enabled:true, accountId:'account', ...extra});
  for (const row of [
    {providers:['codex'], sources:[source('removed','antigravity')], targets:[{sourceId:'removed',provider:'antigravity'}], expected:[]},
    {providers:['future-provider'], sources:[source('added','future-provider')], targets:[{sourceId:'added',provider:'future-provider'}], expected:['added']},
    {providers:['codex','claude','antigravity'], sources:[
      source('local','codex'), source('remote','claude',{hostId:'online'}),
      source('profile','codex',{accountId:'',codexHomeId:'home'}),
      source('disabled','codex',{enabled:false}), source('unlinked','codex',{accountId:''}),
      source('offline','antigravity',{hostId:'offline'}), source('missing-host','codex',{hostId:'missing'}),
      source('legacy','agy'),
    ], targets:[{sourceId:'local',provider:'codex'},{sourceId:'remote',provider:'claude'},{sourceId:'profile',provider:'codex'}], expected:['local','remote','profile']},
    {providers:['codex'], sources:[source('paused-agent','codex')], targets:[], expected:[]},
  ]) {
    const elements = new Map(), notices = [], window = {};
    const editorShown = new Error('editor rendered');
    let body = '';
    const sandbox = {
      window, structuredClone,
      state:{page:'other',settings:{sources:row.sources,hosts:[{id:'online',enabled:true},{id:'offline',enabled:false}]}},
      $: selector => {if(!elements.has(selector))elements.set(selector,{});return elements.get(selector);},
      document:{querySelectorAll:()=>[]}, api:async(method)=>method==='accounts.targets'?structuredClone(row.targets):[], notify:message=>notices.push(message),
      field:()=>'', escapeHTML:String, option:(id,label)=>`<option value="${id}">${label}</option>`,
      showEditor:(_title,html)=>{body=html;throw editorShown;},
    };
    vm.createContext(sandbox);
    vm.runInContext(read('contracts.js'),sandbox);
    window.AieyesContract.wakeupProviders = row.providers;
    vm.runInContext(read('wakeups.js'),sandbox);
    window.AieyesWakeups.bind();
    if (row.expected.length) {
      await assert.rejects(elements.get('#wake-add').onclick(), error=>error===editorShown);
      const choices = body.match(/<select name="sourceId" id="wake-source">(.*?)<\/select>/s)[1];
      assert.deepEqual([...choices.matchAll(/<option value="([^"]+)"/g)].map(match=>match[1]),row.expected);
      assert.equal(notices.length,0);
    } else {
      await elements.get('#wake-add').onclick();
      assert.equal(body,'');
      assert.equal(notices.length,1);
    }
    await flush();
  }
});
function harness(file, initial = {}) {
  const elements = new Map(), events = new Map(), calls = [], notices = [], runs = [], timers = new Map();
  let dragCount = 0, maximized = false, timerId = 0, now = 1000;
  const windowCalls = [];
  function element(id) {
    if (!elements.has(id)) elements.set(id, {
      textContent:'', innerHTML:'', value:'', hidden:false, remove(){}, after(){}, dataset:{}, listeners:new Map(),
      addEventListener(name, fn) { this.listeners.set(name, [...(this.listeners.get(name) || []), fn]); },
      setAttribute(name, value) { this[name] = value; },
      focus() { document.activeElement = this; },
      contains(other) { return other === this || other?.parent === this; },
      querySelector: selector => element(selector),
      querySelectorAll() { return this.children || []; },
      async dispatch(name, fields = {}) { const event={defaultPrevented:false,stopped:false,preventDefault(){this.defaultPrevented=true;},stopPropagation(){},stopImmediatePropagation(){this.stopped=true;},...fields}; for(const fn of this.listeners.get(name)||[]){await fn(event);if(event.stopped)break;} },
    });
    return elements.get(id);
  }
  const document = {body:element('body'),documentElement:element('html'),activeElement:null,querySelector:element,listeners:new Map(),
    addEventListener(name, fn) { this.listeners.set(name, [...(this.listeners.get(name) || []), fn]); },
    async dispatch(name, fields = {}) { const event={defaultPrevented:false,stopped:false,preventDefault(){this.defaultPrevented=true;},stopPropagation(){},stopImmediatePropagation(){this.stopped=true;},...fields}; for(const fn of this.listeners.get(name)||[]){await fn(event);if(event.stopped)break;} },
  };
  const info = {mode:'auto',effectiveMode:'floating',platform:'windows',summary:'思考中 · 2 个会话',phase:'thinking',activeCount:2,sessions:[],page:'agent',panelOpen:false,panelPinned:false,hidden:false,...initial};
  const native = {
    async startDragging(){dragCount++;},
    async isMaximized(){return maximized;},
    async toggleMaximize(){maximized=!maximized;windowCalls.push('maximize');},
    async minimize(){windowCalls.push('minimize');},
    async close(){windowCalls.push('close');},
    async onResized(fn){events.set('resize',fn);return ()=>{};},
  };
  const window = {__TAURI__:{
    core:{async invoke(command, args) {
      calls.push({command,args});
      if (command === 'desktop_panel') { info.panelOpen = args.open; return {...info}; }
      if (command === 'desktop_panel_pin') { info.panelPinned = args.pinned; return {...info}; }
      return info;
    }},
    event:{async listen(name, fn) { events.set(name, fn); return () => {}; }},
    window:{getCurrentWindow:()=>native},
  }};
  const state = {page:'agent',settingsTab:'accounts',settings:{accounts:[],sources:[],hosts:[]},hosts:[],dashboard:null,provider:'',accountKey:'',sourceId:'',model:'',days:1,cost:false,busy:false};
  const sandbox = {
    window, document, state, console, PANEL:file==='floating.js',
    Date: class extends Date { static now() { return now; } },
    notify: e => notices.push(e),
    render(){}, drawTrend(){}, flushRender(){}, updateActivity(){},
    loadDashboard: async () => {},
    scan: async () => { runs.push('scan'); },
    quotas: async () => { runs.push('quotas'); },
    syncPrices: async () => { runs.push('prices'); },
    sample: async () => { runs.push('hosts'); },
    api: async () => ({ prices: [] }),
    setTimeout: (fn, ms = 0) => { const id = ++timerId; timers.set(id, {fn, ms}); return id; },
    clearTimeout: id => { timers.delete(id); },
    requestAnimationFrame: fn => { fn(); return 0; },
    Element: class {},
  };
  vm.createContext(sandbox);
  element('#panel-menu').hidden = true;
  vm.runInContext(read('icons.js'), sandbox, {filename:'icons.js'});
  vm.runInContext(read('contracts.js'), sandbox, {filename:'contracts.js'});
  vm.runInContext(read('ui-state.js'), sandbox, {filename:'ui-state.js'});
  vm.runInContext(read('sessions.js'), sandbox, {filename:'sessions.js'});
  vm.runInContext(read('refresh.js'), sandbox, {filename:'refresh.js'});
  vm.runInContext(read(file), sandbox, {filename:file});
  return {window,document,element,events,calls,notices,runs,state,info,windowCalls,native,timers,
    get dragCount(){return dragCount;},
    async runTimers(ms) { now += ms ?? 0; for (const [id, timer] of [...timers.entries()]) { if (ms === undefined || timer.ms === ms) { timers.delete(id); await timer.fn(); } } },
  };
}

test('floating ball receives live status and opens the panel on click', async () => {
  const h = harness('floating.js'); await flush();
  assert.equal(h.element('#phase').textContent, '思考中');
  assert.equal(h.element('#count').textContent, '2');
  assert.equal(h.document.body.dataset.active, 'true');
  await h.element('#ball').dispatch('click',{detail:1}); await flush();
  assert.equal(h.calls.at(-1).command,'desktop_panel');
  assert.equal(h.calls.at(-1).args.open,true);
  assert.equal(h.document.body.dataset.view,'panel');
  h.events.get('desktop:status')({payload:{...h.info,panelOpen:true,phase:'complete',activeCount:0}});
  assert.equal(h.element('#count').hidden,true);
  assert.equal(h.element('#phase').textContent,'已完成');
  assert.equal(h.document.body.dataset.active,'false');
});
test('dragging snaps the ball without opening the panel; keyboard activation still works', async () => {
  const h = harness('floating.js'); await flush();
  const ball = h.element('#ball');
  await ball.dispatch('pointerdown',{button:0,clientX:20,clientY:20});
  await ball.dispatch('pointermove',{buttons:1,clientX:22,clientY:20});
  assert.equal(h.dragCount,0);
  await ball.dispatch('pointermove',{buttons:1,clientX:34,clientY:20}); await flush();
  assert.equal(h.dragCount,1);
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').at(-1).args.action,'snap');
  await ball.dispatch('click',{detail:1}); await flush();
  assert.equal(h.calls.filter(c=>c.command==='desktop_panel').length,0);
  await ball.dispatch('click',{detail:0}); await flush();
  assert.equal(h.calls.at(-1).command,'desktop_panel');
  await h.document.dispatch('contextmenu',{target:{}});
  assert.equal(h.calls.at(-1).args.action,'menu');
});
test('hover does not open; click, pin and Escape operate the panel', async () => {
  const h=harness('floating.js');await flush();
  const ball=h.element('#ball');
  await ball.dispatch('pointerenter');await h.runTimers(350);await flush();
  assert.equal(h.info.panelOpen,false);
  await ball.dispatch('click',{detail:1});await flush();
  assert.equal(h.info.panelOpen,true);
  await h.element('#panel-pin').dispatch('click');await flush();
  assert.equal(h.info.panelPinned,true);
  await h.document.dispatch('keydown',{key:'Escape'});await flush();
  assert.equal(h.info.panelOpen,false);
  await ball.dispatch('click',{detail:1});await flush();
  assert.equal(h.info.panelOpen,true);
});
test('drag capture cancellation never leaves the pet stuck', async () => {
  const h=harness('floating.js');await flush();
  const ball=h.element('#ball');
  await ball.dispatch('pointerdown',{button:0,clientX:20,clientY:20});
  await h.document.dispatch('pointercancel');
  await ball.dispatch('click',{detail:0});await flush();
  assert.equal(h.info.panelOpen,true);
});
test('the refresh menu runs scans, quotas, prices and host sampling', async () => {
  const h = harness('floating.js'); await flush();
  await h.element('#ball').dispatch('click',{detail:1}); await flush();
  h.element('#panel-menu').hidden = true;
  await h.element('#panel-refresh').dispatch('click');
  assert.equal(h.element('#panel-menu').hidden,false);
  assert.equal(h.element('#panel-refresh')['aria-expanded'],'true');
  await h.element('#panel-menu').dispatch('click',{target:{closest:()=>({dataset:{refresh:'quotas'}})}});
  assert.deepEqual(h.runs,['quotas']);
  assert.equal(h.element('#panel-menu').hidden,true);
});
test('Escape closes the refresh menu before the panel and restores keyboard focus', async () => {
  const h = harness('floating.js'); await flush();
  await h.element('#ball').dispatch('click',{detail:0}); await flush();
  assert.equal(h.document.activeElement,null,'Opening the floating panel does not take keyboard focus');
  await h.element('#panel-refresh').dispatch('click');
  await h.document.dispatch('keydown',{key:'Escape'}); await flush();
  assert.equal(h.element('#panel-menu').hidden,true);
  assert.equal(h.info.panelOpen,true);
  assert.equal(h.document.activeElement,h.element('#panel-refresh'));
  h.element('#panel-refresh').parent = h.element('#panel');
  await h.document.dispatch('keydown',{key:'Escape'}); await flush();
  assert.equal(h.info.panelOpen,false);
  assert.equal(h.document.activeElement,h.element('#ball'));
});
test('native Windows panel close never focuses the hidden Web pet', async () => {
  const h=harness('floating.js',{nativePet:true,panelOpen:true});
  await flush();await flush();
  const input=h.element('#panel-input');input.parent=h.element('#panel');input.focus();
  await h.document.dispatch('keydown',{key:'Escape'});await flush();
  assert.equal(h.info.panelOpen,false);
  assert.notEqual(h.document.activeElement,h.element('#ball'));
});
test('Linux native panel takes keyboard focus and closes explicitly even when pinned', async () => {
  const h=harness('floating.js',{platform:'linux',nativePet:true,panelOpen:true,panelPinned:true});
  await flush();await flush();
  assert.equal(h.document.activeElement,h.element('#panel-refresh'));
  await h.element('#panel-close').dispatch('click');await flush();
  assert.equal(h.info.panelOpen,false);
  assert.notEqual(h.document.activeElement,h.element('#ball'));
  h.events.get('desktop:panel')({payload:{...h.info,panelOpen:true}});
  await h.document.dispatch('keydown',{key:'Escape'});await flush();
  assert.equal(h.info.panelOpen,false);
});
test('refresh menu arrow keys move between its actions', async () => {
  const h = harness('floating.js'); await flush();
  const items=['scan','quotas','prices','hosts'].map(id=>h.element(id));
  h.element('#panel-menu').children=items;
  await h.element('#panel-refresh').dispatch('keydown',{key:'ArrowDown'});
  assert.equal(h.document.activeElement,items[0]);
  await h.element('#panel-menu').dispatch('keydown',{key:'ArrowUp'});
  assert.equal(h.document.activeElement,items[3]);
  await h.element('#panel-menu').dispatch('keydown',{key:'Home'});
  assert.equal(h.document.activeElement,items[0]);
});
test('opening the panel refreshes its settings as well as its dashboard', async () => {
  const h = harness('floating.js'); await flush();
  let refreshes=0;
  h.window.AieyesApp={refreshPanel:async()=>{refreshes++;}};
  await h.element('#ball').dispatch('click',{detail:0}); await flush();
  assert.equal(refreshes,1);
});
test('panel footer opens details, settings and quit through desktop actions', async () => {
  const h = harness('floating.js'); await flush();
  await h.element('#ball').dispatch('click',{detail:1}); await flush();
  await h.element('#panel-detail').dispatch('click'); await flush();
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').at(-1).args.action,'open');
  await h.element('#panel-settings').dispatch('click'); await flush();
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').at(-1).args.action,'settings');
  await h.element('#panel-quit').dispatch('click'); await flush();
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').at(-1).args.action,'quit');
});
test('unavailable logs are visible and never look like confirmed idle status', async () => {
  const h = harness('floating.js',{unavailable:true,phase:null,activeCount:0}); await flush();
  assert.equal(h.element('#phase').textContent,'状态不全');
  assert.equal(h.element('#count').hidden,true);
});
test('main window escapes source names and can navigate from the native menu', async () => {
  const h = harness('desktop.js',{sessions:[{id:'12345678abcdefgh',source:'<img src=x onerror=evil()>',phase:'tool',updatedAt:1}]}); await flush();
  assert.ok(h.element('#session-list').innerHTML.includes('&lt;img'));
  assert.ok(!h.element('#session-list').innerHTML.includes('<img'));
  const destinations=[];
  h.window.AieyesApp={ready:Promise.resolve(),openSettings:async target=>destinations.push(JSON.parse(JSON.stringify(target)))};
  h.events.get('desktop:navigate')({payload:'settings'}); await flush();
  h.events.get('desktop:navigate')({payload:'prices'}); await flush();
  h.events.get('desktop:navigate')({payload:'settings-target:{"category":"hosts","hostId":"host"}'}); await flush();
  h.events.get('desktop:navigate')({payload:'edit-account:claude:work'}); await flush();
  assert.deepEqual(destinations,[{},{category:'prices'},{category:'hosts',hostId:'host'},{accountKey:'claude:work'}]);
  h.window.AieyesDesktop.bindSettings();
  const mode = h.element('#desktop-mode');
  mode.value='tray';
  h.window.__TAURI__.core.invoke = async () => { throw new Error('磁盘不可写'); };
  await mode.onchange();
  assert.equal(mode.value,'auto');
  assert.equal(mode.disabled,false);
  assert.equal(h.notices.length,1);
});
test('settings hide and restore the floating ball with the live label', async () => {
  const h = harness('desktop.js'); await flush();
  h.window.AieyesDesktop.bindSettings();
  const hide = h.element('#hide-ball');
  assert.equal(hide.textContent,'暂时隐藏桌宠');
  await hide.onclick();
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').at(-1).args.action,'hide-ball');
  h.events.get('desktop:status')({payload:{...h.info,hidden:true}});
  assert.equal(hide.textContent,'恢复显示桌宠');
  await hide.onclick();
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').at(-1).args.action,'show-ball');
});
test('native setup routes open the requested main-window editor', async () => {
  const h = harness('desktop.js'); await flush();
  const editors=[];
  h.window.AieyesApp={openSetupInMain:async kind=>editors.push(kind)};
  h.events.get('desktop:navigate')({payload:'add-host'}); await flush();
  h.events.get('desktop:navigate')({payload:'add-source'}); await flush();
  assert.deepEqual(editors,['hosts','sources']);
});
test('Windows titlebar minimizes, restores, and requests close rather than quit', async () => {
  const h = harness('desktop.js',{material:'mica'}); await flush();
  assert.equal(h.element('#titlebar').hidden,false);
  assert.equal(h.document.documentElement.dataset.material,'mica');
  await h.element('#window-minimize').onclick();
  await h.element('#window-maximize').onclick();
  assert.equal(h.element('#window-maximize')['aria-label'],'还原');
  await h.element('#window-maximize').onclick();
  assert.equal(h.element('#window-maximize')['aria-label'],'最大化');
  await h.element('#window-close').onclick();
  assert.deepEqual(h.windowCalls,['minimize','maximize','maximize','close']);
  assert.equal(h.calls.some(c=>c.args?.action==='quit'),false);
  h.native.close = async () => {throw new Error('关闭失败');};
  await h.element('#window-close').onclick();
  assert.match(h.notices.at(-1),/关闭失败/);
});


test('each surface uses its own material and follows accessibility fallback status', async () => {
  for(const [file,surface] of [['desktop.js','main'],['floating.js','floating']]) {
    const h=harness(file,{material:'mica',materials:{main:'acrylic',floating:'blur'}});await flush();
    assert.equal(h.document.documentElement.dataset.platform,'windows');
    assert.equal(h.document.documentElement.dataset.material,surface==='main'?'acrylic':'blur');
    h.events.get('desktop:status')({payload:{...h.info,materials:{main:'opaque',floating:'opaque'}}});
    assert.equal(h.document.documentElement.dataset.material,'opaque');
    h.events.get('desktop:status')({payload:{...h.info,materials:{main:'acrylic',floating:'opaque'}}});
    assert.equal(h.document.documentElement.dataset.material,surface==='main'?'acrylic':'opaque');
  }
  const legacy=harness('floating.js',{material:'mica'});await flush();
  assert.equal(legacy.document.documentElement.dataset.material,'opaque','legacy main material never enables panel transparency');
});


test('overview hides disabled Agents while retaining historical configuration', () => {
  const window = {AieyesContract:{agentProviders:['codex','zcode'],agentAliases:{},providerCapabilities:{}}};
  vm.runInNewContext(readFileSync(new URL('../apps/desktop/web/ui-state.js', import.meta.url),'utf8'),{window});
  const settings={agents:[{provider:'codex',enabled:true,machineIds:['local']},{provider:'zcode',enabled:false,machineIds:['local']}],sources:[{provider:'zcode',enabled:false}],accounts:[{provider:'zcode',id:'old'}]};
  assert.deepEqual([...window.AieyesUI.overviewAgentProviders(settings)],['codex']);
  assert.equal(settings.accounts.length,1);
  for(const n of [1,2,3,4,6,8])assert(window.AieyesUI.balancedColumns(900,n)<=n);
  assert.equal(window.AieyesUI.balancedColumns(900,4),2);
  assert.equal(window.AieyesUI.balancedColumns(420,4),1);
});


test('rolling values never fill missing periods with unrelated manual estimates', () => {
  const window = {};
  vm.runInNewContext(read('contracts.js'),{window});
  vm.runInNewContext(read('ui-state.js'),{window});
  const manual={id:'manual',status:'completed',startedAt:10,fiveHourValue:999,weeklyValue:100,weeklyRatioValue:500};
  const rolling={id:'rolling',samplingMode:'rolling',status:'active',startedAt:1,weeklyValue:25};
  const quota={provider:'codex',plan:'pro',windows:[{windowMinutes:300},{windowMinutes:10080}]};
  const entries=[...window.AieyesUI.estimateEntries([manual,rolling],false,quota)];
  assert.equal(entries.length,1);assert.equal(entries[0].field,'weeklyValue');assert.equal(entries[0].value,25);
});

test('estimate labels omit unavailable amounts for every Agent and preserve zero', () => {
  const window={};
  vm.runInNewContext(read('contracts.js'),{window});
  vm.runInNewContext(read('ui-state.js'),{window});
  const ui=window.AieyesUI;
  for(const provider of Object.keys(window.AieyesContract.providers)) {
    const quota={provider,plan:'pro',windows:[{windowMinutes:300},{windowMinutes:10080}]};
    assert.equal(ui.estimateLine([],quota),'额度估值',provider);
    assert(!ui.estimateLine([{id:'pending',status:'pending',fiveHourValue:10,weeklyValue:20}],quota).includes('—'),provider);
  }
  assert.equal(ui.creditEstimateLine([]),'credits 估值');
  assert.equal(ui.creditEstimateLine([{id:'pending',status:'pending',valuePer1000:123}]),'credits 估值');
  assert.equal(ui.creditEstimateLine([{id:'invalid',status:'active',valuePer1000:NaN}]),'credits 估值');
  assert.equal(ui.creditEstimateLine([{id:'zero',status:'active',valuePer1000:0}]),'credits 估值（1000 credits ≈ $0.00）');
  const q={provider:'codex',plan:'pro',windows:[{windowMinutes:300},{windowMinutes:10080}]};
  assert.equal(ui.estimateLine([{id:'zero',status:'active',valuationMode:'fiveHour',fiveHourValue:0}],q),'5h ≈ $0.00');
});
