import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { readFileSync } from 'node:fs';

const read = name => readFileSync(new URL(`../apps/desktop/web/${name}`, import.meta.url), 'utf8');
const flush = () => new Promise(resolve => setImmediate(resolve));
test('panel account choices cap each Agent and preserve explicit empty choices', () => {
  const data=new Map(),window={};
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
    window, document, state, console,
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
test('drag capture cancellation never leaves the capsule stuck', async () => {
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
test('native Windows panel close never focuses the hidden Web capsule', async () => {
  const h=harness('floating.js',{nativeCapsule:true,panelOpen:true});
  await flush();await flush();
  const input=h.element('#panel-input');input.parent=h.element('#panel');input.focus();
  await h.document.dispatch('keydown',{key:'Escape'});await flush();
  assert.equal(h.info.panelOpen,false);
  assert.notEqual(h.document.activeElement,h.element('#ball'));
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
  h.events.get('desktop:navigate')({payload:'settings'});
  assert.equal(h.state.page,'settings');
  assert.equal(h.state.settingsTab,'general');
  h.events.get('desktop:navigate')({payload:'prices'}); await flush();
  assert.equal(h.state.settingsTab,'prices');
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
  assert.equal(hide.textContent,'暂时隐藏悬浮球');
  await hide.onclick();
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').at(-1).args.action,'hide-ball');
  h.events.get('desktop:status')({payload:{...h.info,hidden:true}});
  assert.equal(hide.textContent,'恢复显示悬浮球');
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
