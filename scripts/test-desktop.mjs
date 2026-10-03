import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import { readFileSync } from 'node:fs';

const read = name => readFileSync(new URL(`../apps/desktop/web/${name}`, import.meta.url), 'utf8');
const flush = () => new Promise(resolve => setImmediate(resolve));
function harness(file, initial = {}) {
  const elements = new Map(), events = new Map(), calls = [], notices = [];
  let dragCount = 0;
  function element(id) {
    if (!elements.has(id)) elements.set(id, {
      textContent:'', innerHTML:'', value:'', hidden:false, dataset:{}, listeners:new Map(),
      addEventListener(name, fn) { this.listeners.set(name, fn); },
      setAttribute(name, value) { this[name] = value; },
      async dispatch(name, fields = {}) { await this.listeners.get(name)?.({preventDefault(){},...fields}); },
    });
    return elements.get(id);
  }
  const info = {mode:'auto',effectiveMode:'floating',platform:'windows',summary:'思考中 · 2 个会话',phase:'thinking',activeCount:2,sessions:[],page:'agent',...initial};
  const document = {body:element('body'),activeElement:null,querySelector:element};
  const window = {__TAURI__:{
    core:{async invoke(command, args) { calls.push({command,args}); return info; }},
    event:{async listen(name, fn) { events.set(name, fn); return () => {}; }},
    window:{getCurrentWindow:()=>({async startDragging(){dragCount++;}})},
  }};
  const state = {page:'agent',settingsTab:'accounts'};
  vm.runInNewContext(read(file), {window,document,state,notify:e=>notices.push(e),render(){},console}, {filename:file});
  return {window,document,element,events,calls,notices,state,info,get dragCount(){return dragCount;}};
}

test('floating ball receives live status and opens the details window on click', async () => {
  const h = harness('floating.js'); await flush();
  assert.equal(h.element('#phase').textContent, '思考中');
  assert.equal(h.element('#count').textContent, '2');
  assert.equal(h.document.body.dataset.active, 'true');
  await h.element('#ball').dispatch('click',{detail:1});
  assert.equal(h.calls.at(-1).args.action,'open');
  h.events.get('desktop:status')({payload:{...h.info,phase:'complete',activeCount:0}});
  assert.equal(h.element('#count').hidden,true);
  assert.equal(h.element('#phase').textContent,'已完成');
  assert.equal(h.document.body.dataset.active,'false');
});
test('dragging does not open details; keyboard activation still works after a drag', async () => {
  const h = harness('floating.js'); await flush();
  const ball = h.element('#ball');
  await ball.dispatch('pointerdown',{button:0,clientX:20,clientY:20});
  await ball.dispatch('pointermove',{buttons:1,clientX:22,clientY:20});
  assert.equal(h.dragCount,0);
  await ball.dispatch('pointermove',{buttons:1,clientX:34,clientY:20});
  assert.equal(h.dragCount,1);
  await ball.dispatch('click',{detail:1});
  assert.equal(h.calls.filter(c=>c.command==='desktop_action').length,0);
  await ball.dispatch('click',{detail:0});
  assert.equal(h.calls.at(-1).args.action,'open');
  await ball.dispatch('contextmenu');
  assert.equal(h.calls.at(-1).args.action,'menu');
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
  h.window.AieyesDesktop.bindSettings();
  const mode = h.element('#desktop-mode');
  mode.value='tray';
  h.window.__TAURI__.core.invoke = async () => { throw new Error('磁盘不可写'); };
  await mode.onchange();
  assert.equal(mode.value,'auto');
  assert.equal(mode.disabled,false);
  assert.equal(h.notices.length,1);
});
