// Orbit popover interaction card: approvals, questions, text replies and read-only cards.
// Usage: node scripts/test-orbit-interaction-ui.cjs [chromium|webkit]
const assert = require('node:assert/strict');
const http = require('node:http');
const path = require('node:path');
const playwright = require('../apps/desktop/node_modules/playwright');
const { serveWebRoot } = require('./ui-test-helpers.cjs');

const engine = process.argv[2] === 'webkit' ? 'webkit' : 'chromium';
const LONG = 'https://example.invalid/' + 'averyveryverylongpathsegmentwithoutanybreaks'.repeat(12);

// Runs inside the page before any script. `options` picks the sessions on screen and scripts the
// core: { sessions, auto, status, payloads, respond: [results], respondDelay: ms,
// inspectDelay: {id: ms}, inspectQueue: {id: [payloads]} }.
function fixture(options) {
  const listeners = new Map();
  const base = (id, extra) => ({
    sessionId: id, stateKey: `needsInput:${id}`, promptHash: `ph-${id}`, kind: 'readonly', adapter: 'focusOnly',
    canRespond: false, canHandoff: false, needsClaudeHook: false, requestId: '', title: id,
    approval: null, questions: [], textPrompt: '', readonlyReason: '', ...extra,
  });
  const option = (label, extra = {}) => ({ label, description: `${label} description`, ...extra });
  const payloads = {
    'sess-approval': base('sess-approval', {
      kind: 'approval', adapter: 'claudeHook', canRespond: true, canHandoff: true, requestId: 'req-1',
      approval: {
        toolName: 'Bash', reason: 'Run test suite', command: 'cargo test --workspace', details: '',
        decisions: [
          { id: 'approve', label: '允许本次', style: 'primary' },
          { id: 'decline', label: '拒绝', style: 'danger', acceptsReason: true },
        ],
      },
    }),
    'sess-questions': base('sess-questions', {
      kind: 'questions', adapter: 'claudeHook', canRespond: true, canHandoff: true, requestId: 'req-2',
      questions: [{
        id: '1', key: '', question: 'Select target platforms', kind: 'string', multiSelect: true, allowCustom: true,
        options: [option('macOS'), option('Windows', { preview: 'fn main() {}' }), option('Linux')],
      }],
    }),
    'sess-single': base('sess-single', {
      kind: 'questions', adapter: 'claudeHook', canRespond: true, canHandoff: true, requestId: 'req-3',
      questions: [{
        id: '1', key: '', question: 'Pick a database?', kind: 'string', multiSelect: false, allowCustom: true,
        options: [option('Postgres'), option('SQLite')],
      }],
    }),
    'sess-form': base('sess-form', {
      kind: 'questions', adapter: 'claudeHook', canRespond: true, canHandoff: false, requestId: 'req-4',
      questions: [
        { id: '1', key: '', question: 'First?', kind: 'string', multiSelect: false, allowCustom: true, options: [option('A1'), option('B1')] },
        { id: '2', key: '', question: 'Second?', kind: 'string', multiSelect: false, allowCustom: true, options: [option('A2'), option('B2')] },
      ],
    }),
    'sess-text': base('sess-text', {
      kind: 'text', adapter: 'tmux', canRespond: true, textPrompt: 'Which branch should I rebase onto?',
    }),
    'sess-readonly': base('sess-readonly', {
      promptHash: '', needsClaudeHook: true, textPrompt: '1: Pick a database?',
      readonlyReason: '启用 Claude / Qoder 交互 Hook 后可直接在弹窗选择回答',
    }),
    'sess-locked-approval': base('sess-locked-approval', {
      kind: 'approval',
      approval: { toolName: 'Bash', reason: 'Remove build output', command: 'rm -rf target', details: '', decisions: [] },
      readonlyReason: '可在设置中开启 Claude / Qoder 工具授权弹窗，或点击跳转到原窗口确认',
    }),
    'sess-decisions': base('sess-decisions', {
      kind: 'approval', adapter: 'codex', canRespond: true, pendingCount: 3, inputKind: 'approval',
      approval: {
        toolName: 'Bash', reason: 'Run the tests', command: 'cargo test', details: '', kind: 'command',
        title: 'Bash · tests', cwd: '/work/repo',
        decisions: [
          { id: 'approve', label: '允许本次', style: 'primary', behavior: 'allow', scope: 'once' },
          { id: 'approveSession', label: '本会话内允许', style: 'secondary', behavior: 'allow', scope: 'session' },
          { id: 'approveAlways', label: '始终允许', style: 'secondary', behavior: 'allow', scope: 'always', description: 'cargo test' },
          { id: 'approveHost.2', label: '允许访问主机', style: 'secondary', behavior: 'allow', scope: 'always', description: 'example.invalid' },
          { id: 'decline', label: '拒绝', style: 'danger', behavior: 'deny', scope: 'once', acceptsReason: true },
          { id: 'declineInterrupt', label: '拒绝并中断', style: 'danger', behavior: 'deny', scope: 'once', interrupts: true },
        ],
      },
    }),
    'sess-plan': base('sess-plan', {
      kind: 'approval', adapter: 'claudeHook', canRespond: true, canHandoff: true, requestId: 'req-plan', pendingCount: 1,
      approval: {
        toolName: 'ExitPlanMode', reason: '', command: '', details: '', kind: 'plan',
        plan: { markdown: '# Migration plan\n\n1. Create `users` table\n2. Backfill\n3. Verify\n\n```\ncargo test <all>\n```\n<img src=x onerror=alert(1)>', truncated: true },
        decisions: [
          { id: 'approve', label: '批准计划', style: 'primary', behavior: 'allow' },
          { id: 'approveAcceptEdits', label: '批准并自动接受编辑', style: 'secondary', behavior: 'allow' },
          { id: 'decline', label: '继续修改计划', style: 'danger', behavior: 'deny', acceptsReason: true },
        ],
      },
    }),
    'sess-files': base('sess-files', {
      kind: 'approval', adapter: 'codex', canRespond: true, pendingCount: 1,
      approval: {
        toolName: 'apply_patch', reason: '', command: '', details: '', kind: 'fileChange',
        changes: [
          { path: 'src/main.rs', kind: 'update', diff: '--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,2 +1,2 @@\n-fn old() {}\n+fn new() {}\n fn keep() {}', truncated: false },
          { path: 'src/added.rs', kind: 'add', newText: 'line one\nline two', truncated: true },
          { path: 'src/old.rs', kind: 'move', movePath: 'src/new.rs', truncated: false },
          { path: 'src/gone.rs', kind: 'delete', truncated: false },
        ],
        paths: ['/work/repo/src'],
        decisions: [
          { id: 'approve', label: '允许本次', style: 'primary' },
          { id: 'decline', label: '拒绝', style: 'danger', acceptsReason: true },
        ],
      },
    }),
    'sess-mcp': base('sess-mcp', {
      kind: 'questions', adapter: 'claudeHook', canRespond: true, canHandoff: true, requestId: 'req-mcp', inputKind: 'elicitation',
      form: { title: 'Deploy settings', description: 'Choose how to deploy', source: 'mcp', serverName: 'github', url: '', canCancel: true, canSkip: true },
      questions: [
        { id: '1', key: 'region', header: 'Region', question: 'Which region?', kind: 'string', multiSelect: false, allowCustom: false, required: true, default: 'eu-west',
          options: [{ label: 'US East', value: 'us-east', preview: 'us-east-1' }, { label: 'EU West', value: 'eu-west' }] },
        { id: '2', key: 'replicas', header: 'Replicas', question: 'How many replicas?', kind: 'integer', multiSelect: false, allowCustom: true, required: true, default: 2,
          constraints: { minimum: 1, maximum: 5 }, options: [] },
        { id: '3', key: 'public', question: 'Expose publicly?', kind: 'boolean', multiSelect: false, allowCustom: false, required: true,
          options: [{ label: '是', value: 'true' }, { label: '否', value: 'false' }] },
        { id: '4', key: 'token', header: 'Token', question: 'Deploy token', kind: 'string', multiSelect: false, allowCustom: true, secret: true, required: true, options: [] },
        { id: '5', key: 'note', question: 'Anything else?', kind: 'string', multiSelect: false, allowCustom: true, required: false, options: [] },
        { id: '6', key: 'features', question: 'Features', kind: 'multiselect', multiSelect: true, allowCustom: false, required: true,
          options: [{ label: 'cache', value: 'cache' }, { label: 'waf', value: 'waf' }] },
      ],
    }),
    'sess-mcp-url': base('sess-mcp-url', {
      inputKind: 'elicitation', readonlyReason: '此表单包含条件或外链字段，请回到原窗口处理',
      form: { title: 'Authorize', description: '', source: 'mcp', serverName: 'github', url: 'https://example.invalid/authorize', canCancel: false, canSkip: false },
    }),
    'sess-queue': base('sess-queue', {
      kind: 'approval', adapter: 'opencode', canRespond: true, requestId: 'perm-1', promptHash: 'ph-perm-1', pendingCount: 2,
      pendingRequests: [{ requestId: 'perm-1', kind: 'approval', title: 'git status' }, { requestId: 'perm-2', kind: 'approval', title: 'rm -rf build' }],
      approval: { toolName: 'Shell', reason: '', command: 'git status', details: '', kind: 'command', decisions: [{ id: 'approve', label: '允许本次', style: 'primary' }, { id: 'decline', label: '拒绝', style: 'danger', acceptsReason: true }] },
      byRequest: {
        'perm-2': { requestId: 'perm-2', promptHash: 'ph-perm-2', approval: { toolName: 'Shell', reason: '', command: 'rm -rf build', details: '', kind: 'command', decisions: [{ id: 'approve', label: '允许本次', style: 'primary' }, { id: 'decline', label: '拒绝', style: 'danger', acceptsReason: true }] } },
      },
    }),
    'sess-running': base('sess-running', {
      kind: 'followup', adapter: 'opencode', canRespond: true, stateKey: 'working:1', promptHash: 'ph-follow-1', checkState: true,
      followUp: { available: true, adapter: 'opencode', mode: 'queue', reason: '', canInterrupt: true },
    }),
    'sess-busy': base('sess-busy', {
      kind: 'none', stateKey: 'working:b', promptHash: '',
      followUp: { available: false, adapter: '', mode: '', reason: '需等待本轮结束', canInterrupt: false },
    }),
    'sess-stoppable': base('sess-stoppable', {
      kind: 'none', adapter: 'opencode', stateKey: 'working:s', promptHash: '',
      followUp: { available: false, adapter: 'opencode', mode: '', reason: '原会话正在等待回答，请先处理当前请求', canInterrupt: true },
    }),
    'sess-generic': base('sess-generic', {
      kind: 'text', adapter: 'tmux', canRespond: true, textPrompt: 'Should I continue with the migration?', promptSource: 'lastMessage',
    }),
    'sess-focus-questions': base('sess-focus-questions', {
      kind: 'questions', inputKind: 'question', readonlyReason: '此 Agent 的选项需要在原窗口选择，点击跳转后处理',
      questions: [{ id: '1', key: '', question: 'Pick a database?', kind: 'string', multiSelect: false, allowCustom: false, options: [option('Postgres'), option('SQLite')] }],
    }),
    'sess-focus-approval': base('sess-focus-approval', {
      kind: 'approval', inputKind: 'approval', readonlyReason: '此审批需要在原窗口确认',
      approval: { toolName: 'Bash', reason: '', command: 'rm -rf probe_dir', details: '', kind: 'command', decisions: [] },
    }),
    'sess-answered': base('sess-answered', { kind: 'none', answered: true }),
  };
  for (const [id, extra] of Object.entries(options.payloads || {})) {
    payloads[id] = options.merge ? { ...payloads[id], ...extra } : extra;
  }
  const providers = { 'sess-text': 'kiro' };
  const ids = options.sessions || ['sess-approval', 'sess-questions'];
  const session = (id, index) => ({
    id, provider: providers[id] || 'claude',
    machine: options.machines?.[id] ? 'build-box' : '本机', machineId: options.machines?.[id] || 'local', sourceId: 'src-1',
    turnId: `t-${id}`, phase: options.phases?.[id] || 'needsInput', updatedAt: Date.now() / 1000, workspace: `/work/${id}`,
    pid: 1234 + index, processStart: 100 + index, nativeSession: `${id}-native`, inputStateKey: `needsInput:${id}`,
  });
  const sessions = ids.map(session);
  const agent = sessions[0].provider;
  const waiting = sessions.some((s) => s.phase === 'needsInput');
  const state = {
    calls: [],
    payloads,
    session,
    respond: [...(options.respond || [])],
    inspectDelay: { ...(options.inspectDelay || {}) },
    inspectQueue: { ...(options.inspectQueue || {}) },
    status: {
      enabled: true, autoPopup: false, claudeQuestions: true, claudeApprovals: true, allowTerminal: true,
      desktopAllowTerminal: true, feishuPaired: false, hooks: [], ...(options.status || {}),
    },
    info: {
      platform: 'linux', page: 'agent', mode: 'pet', effectiveMode: 'pet', nativePet: false, petEnabled: true,
      petScale: 1, panelOpen: true, panelPinned: false, orbitAgent: agent, orbitAuto: !!options.auto,
      orbitPreferredSession: ids[0],
      petOrbit: [{ provider: agent, displayProvider: agent, phase: waiting ? 'needsInput' : 'working', activeCount: ids.length, overflowProviders: [], quota: null }],
      sessions, summary: waiting ? '等待输入' : '进行中', phase: waiting ? 'needsInput' : 'working', activeCount: ids.length,
      pet: { key: 'attention', behavior: 'attention', label: '等待输入', row: 3, frames: 4, frameMs: 140 },
    },
    emit(next) {
      Object.assign(state.info, next);
      for (const fn of listeners.get('desktop:status') || []) fn({ payload: structuredClone(state.info) });
    },
    engine(method) {
      return state.calls.filter((c) => c.args?.method === method).map((c) => c.args.params);
    },
    invokes(name) {
      return state.calls.filter((c) => c.name === name).map((c) => c.args);
    },
  };
  window.interactionTest = state;
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  window.__TAURI__ = {
    event: {
      async listen(name, fn) {
        listeners.set(name, [...(listeners.get(name) || []), fn]);
        return () => {};
      },
    },
    window: {
      getCurrentWindow: () => ({
        async startDragging() {},
        async isMaximized() { return false; },
        async onResized() { return () => {}; },
      }),
    },
    core: {
      async invoke(name, args = {}) {
        state.calls.push({ name, args: structuredClone(args) });
        if (name === 'desktop_info') return structuredClone(state.info);
        if (name === 'desktop_panel') {
          state.info.panelOpen = !!args.open;
          return structuredClone(state.info);
        }
        if (name === 'focus_session') return { outcome: 'window', name: 'Terminal' };
        if (name !== 'engine_call') return {};
        const { method, params } = args;
        if (method === 'sessions.interaction.status') return structuredClone(state.status);
        if (method === 'sessions.interaction.configure') {
          Object.assign(state.status, params);
          return structuredClone(state.status);
        }
        if (method === 'sessions.interaction.inspect') {
          const id = params.sessionId;
          if (state.inspectDelay[id]) await wait(state.inspectDelay[id]);
          const queued = state.inspectQueue[id];
          if (queued?.length) return structuredClone(queued.shift());
          const { byRequest, checkState, ...payload } = state.payloads[id] || { kind: 'none' };
          return structuredClone({ ...payload, ...(byRequest?.[params.requestId] || {}) });
        }
        if (method === 'sessions.interaction.respond') {
          const current = state.payloads[params.sessionId];
          if (options.respondDelay) await wait(options.respondDelay);
          if (current?.checkState && params.stateKey !== current.stateKey) return { status: 'blocked', message: '会话状态已变化，请刷新后再操作' };
          return structuredClone(state.respond.shift() || { status: 'confirmed', message: '已允许本次操作' });
        }
        if (method === 'sessions.interaction.handoff') return { handedOff: true, message: '已交还给原终端确认' };
        const defaults = {
          'hello': { version: 'fixture' },
          'settings.get': { accounts: [], sources: [], hosts: [], appearance: { language: 'zh-CN' } },
          'dashboard': { summary: {}, models: [], quotas: [], sources: [], hosts: [] },
          'sessions.list': { sessions: [], unavailable: false },
        };
        return structuredClone(defaults[method] ?? []);
      },
    },
  };
}

(async () => {
  const root = path.resolve(__dirname, '../apps/desktop/web');
  const server = http.createServer(serveWebRoot(root, 'floating.html'));
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await playwright[engine].launch();
  const errors = [];
  // The real popover window is 368 logical pixels wide and starts 380 high.
  async function open(options = {}) {
    const page = await browser.newPage({ viewport: { width: 368, height: options.height || 380 } });
    page.on('pageerror', (e) => errors.push(e.stack || String(e)));
    await page.addInitScript(fixture, options);
    await page.goto(`http://127.0.0.1:${server.address().port}/floating.html`);
    await page.waitForSelector(options.card === false ? '.orbit-popover, #orbit-popover' : '.orbit-icard');
    return page;
  }
  const engineCalls = (page, method) => page.evaluate((m) => interactionTest.engine(m), method);
  const invokes = (page, name) => page.evaluate((n) => interactionTest.invokes(n), name);
  const focusedKey = (page) => page.evaluate(() => {
    const el = document.activeElement;
    return el ? `${el.tagName}:${el.dataset.ifield || el.dataset.icustom || el.dataset.iolabel || el.dataset.idecision || el.dataset.iaction || el.dataset.istep || ''}` : '';
  });
  const composingKey = (page, selector, key) => page.evaluate(([sel, k]) => {
    const target = sel ? document.querySelector(sel) : document.body;
    target.dispatchEvent(new KeyboardEvent('keydown', { key: k, isComposing: true, keyCode: 229, bubbles: true, cancelable: true }));
  }, [selector, key]);
  const noOverflow = async (page, label) => {
    const size = await page.evaluate(() => {
      const body = document.querySelector('.orbit-body');
      return { body: [body.scrollWidth, body.clientWidth], doc: [document.documentElement.scrollWidth, window.innerWidth] };
    });
    assert.ok(size.body[0] <= size.body[1], `${label}: body scrolls sideways ${size.body}`);
    assert.ok(size.doc[0] <= size.doc[1], `${label}: page scrolls sideways ${size.doc}`);
  };

  try {
    // 1. Approval: jump to the original window, approve once, and never answer the request twice.
    {
      const page = await open();
      await page.waitForSelector('.orbit-iapproval .orbit-code');
      assert.equal(await page.locator('.orbit-iapproval .orbit-code').textContent(), 'cargo test --workspace');
      assert.equal(await page.locator('.orbit-itab').count(), 2);
      await page.click('.orbit-icard-tools [data-iaction=focus]');
      await page.waitForFunction(() => interactionTest.invokes('focus_session').length === 1);
      assert.equal((await invokes(page, 'focus_session'))[0].pid, 1234);

      await page.click('[data-idecision=approve]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const [approve] = await engineCalls(page, 'sessions.interaction.respond');
      assert.equal(approve.action, 'decide');
      assert.equal(approve.decisionId, 'approve');
      assert.equal(approve.requestId, 'req-1');
      assert.deepEqual(approve.capabilities, { localTerminal: false });
      assert.equal(approve.promptHash, 'ph-sess-approval');
      assert.equal(approve.stateKey, 'needsInput:sess-approval');
      // The confirmed card stays locked, including for a click that bypasses the disabled state.
      assert.equal(await page.locator('[data-idecision=approve]').isDisabled(), true);
      assert.equal(await page.locator('[data-idecision=decline]').isDisabled(), true);
      assert.equal(await page.locator('[data-iaction=handoff]').isDisabled(), true);
      await page.evaluate(() => {
        const btn = document.querySelector('[data-idecision=approve]');
        btn.disabled = false;
        btn.click();
      });
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 1);

      // After the confirmation the card moves on to the next waiting session.
      await page.waitForSelector('.orbit-iquestions .orbit-opt');
      assert.equal(await page.locator('.orbit-itab[aria-selected=true]').getAttribute('data-itarget'), 'sess-questions');
      await page.click('[data-ipreview="1:Windows"]');
      assert.equal(await page.locator('.orbit-iquestions .orbit-code').textContent(), 'fn main() {}');

      // Multi-select: a keyboard toggle keeps focus on the option, and the answer follows option
      // order (as on macOS), not click order.
      await page.focus('.orbit-opt[data-iolabel="Linux"]');
      await page.keyboard.press('Space');
      assert.equal(await focusedKey(page), 'BUTTON:Linux');
      await page.click('.orbit-opt[data-iolabel="macOS"]');
      await page.evaluate(() => interactionTest.emit({ activeCount: 2 }));
      assert.equal(await page.locator('.orbit-opt[data-iolabel="macOS"]').getAttribute('data-selected'), 'true');
      assert.equal(await page.locator('.orbit-opt[data-iolabel="Linux"]').getAttribute('data-selected'), 'true');
      assert.equal(await page.locator('.orbit-opt[data-iolabel="Windows"]').getAttribute('data-selected'), 'false');
      await page.fill('[data-icustom="1"]', 'FreeBSD');
      await page.click('[data-iaction="submit-questions"]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 2);
      assert.deepEqual((await engineCalls(page, 'sessions.interaction.respond'))[1].answers, { '1': ['macOS', 'Linux', 'FreeBSD'] });
      await page.close();
    }

    // 2. A tab switch while the first inspect is still running shows the newly selected session.
    {
      const page = await open({ inspectDelay: { 'sess-approval': 900 } });
      await page.waitForSelector('.orbit-iloading');
      await page.click('[data-itarget="sess-questions"]');
      await page.waitForSelector('.orbit-iquestions .orbit-opt');
      await page.waitForTimeout(1200);
      await page.evaluate(() => interactionTest.emit({ activeCount: 2 }));
      assert.equal(await page.locator('.orbit-itab[aria-selected=true]').getAttribute('data-itarget'), 'sess-questions');
      assert.equal(await page.locator('.orbit-iloading').count(), 0);
      assert.equal(await page.locator('.orbit-iapproval').count(), 0);
      assert.equal(await page.locator('.orbit-iquestions').count(), 1);
      assert.equal(await page.locator('[data-iaction=refresh]').isDisabled(), false);
      await page.close();
    }

    // 3. Text reply: focus, caret and keystrokes survive a change of the waiting list; the shell is
    //    told about the unsent draft; a composing Ctrl+Enter does not send.
    {
      const page = await open({ sessions: ['sess-text'] });
      await page.waitForSelector('[data-ifield="text-reply"]');
      await page.click('[data-ifield="text-reply"]');
      await page.keyboard.type('hello wor');
      await page.waitForFunction(() => interactionTest.invokes('desktop_orbit_popover').some((a) => a.draft === true));
      await page.evaluate(() => {
        const t = interactionTest;
        t.emit({ activeCount: 2, sessions: [...t.info.sessions, { ...t.session('sess-readonly', 5), provider: 'kiro' }] });
      });
      assert.equal(await page.locator('.orbit-itab').count(), 2);
      assert.equal(await focusedKey(page), 'TEXTAREA:text-reply');
      await page.keyboard.type('ld');
      assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'hello world');
      await page.evaluate(() => interactionTest.emit({ summary: '等待输入' }));
      assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'hello world');

      await page.evaluate(() => {
        document.querySelector('[data-ifield="text-reply"]').dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, isComposing: true, keyCode: 229, bubbles: true, cancelable: true }));
      });
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      await page.keyboard.press('Control+Enter');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const [sent] = await engineCalls(page, 'sessions.interaction.respond');
      assert.equal(sent.action, 'answer');
      assert.equal(sent.text, 'hello world');
      assert.equal(await page.locator('[data-ifield="text-reply"]').isDisabled(), true);
      await page.waitForFunction(() => interactionTest.invokes('desktop_orbit_popover').filter((a) => 'draft' in a).at(-1)?.draft === false);
      await page.close();
    }

    // 4. Input methods and Enter: a composing Enter never submits, Enter sends a one-question form
    //    only, and a composing Escape does not close the panel.
    {
      const page = await open({ sessions: ['sess-form'] });
      await page.waitForSelector('.orbit-iquestions .orbit-opt');
      // Keyboard selection keeps its place (WebKit does not focus a button on click).
      await page.focus('.orbit-opt[data-iolabel="A1"]');
      await page.keyboard.press('Enter');
      assert.equal(await page.locator('.orbit-opt[data-iolabel="A1"]').getAttribute('data-selected'), 'true');
      assert.equal(await focusedKey(page), 'BUTTON:A1');
      assert.equal(await page.locator('[data-iaction="submit-questions"]').isDisabled(), true);
      await page.fill('[data-icustom="2"]', '中文');
      assert.equal(await page.locator('[data-iaction="submit-questions"]').isDisabled(), false);
      await composingKey(page, '[data-icustom="2"]', 'Enter');
      await page.press('[data-icustom="2"]', 'Enter');
      await page.waitForTimeout(100);
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      assert.equal(await page.inputValue('[data-icustom="2"]'), '中文');
      await composingKey(page, '[data-icustom="2"]', 'Escape');
      assert.equal((await invokes(page, 'desktop_panel')).length, 0);
      await page.click('[data-iaction="submit-questions"]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      assert.deepEqual((await engineCalls(page, 'sessions.interaction.respond'))[0].answers, { '1': 'A1', '2': '中文' });
      await page.keyboard.press('Escape');
      await page.waitForFunction(() => interactionTest.invokes('desktop_panel').length === 1);
      assert.equal((await invokes(page, 'desktop_panel'))[0].open, false);
      await page.close();
    }
    {
      const page = await open({ sessions: ['sess-single'] });
      await page.waitForSelector('[data-icustom="1"]');
      await page.fill('[data-icustom="1"]', 'DuckDB');
      await composingKey(page, '[data-icustom="1"]', 'Enter');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      await page.press('[data-icustom="1"]', 'Enter');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      assert.deepEqual((await engineCalls(page, 'sessions.interaction.respond'))[0].answers, { '1': 'DuckDB' });
      await page.close();
    }

    // 5. A single-choice question answers on click; "uncertain" may have been delivered, so the
    //    card locks instead of offering a second send.
    {
      const page = await open({ sessions: ['sess-single'], respond: [{ status: 'uncertain', message: '连接中断，无法确认是否送达' }] });
      await page.waitForSelector('.orbit-opt[data-iolabel="Postgres"]');
      await page.click('.orbit-opt[data-iolabel="Postgres"]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=warn]');
      const calls = await engineCalls(page, 'sessions.interaction.respond');
      assert.deepEqual(calls.map((c) => c.answers), [{ '1': 'Postgres' }]);
      assert.equal(await page.locator('.orbit-opt[data-iolabel="SQLite"]').isDisabled(), true);
      await page.evaluate(() => {
        const btn = document.querySelector('.orbit-opt[data-iolabel="SQLite"]');
        btn.disabled = false;
        btn.click();
      });
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 1);
      // An explicit refresh re-reads the request and unlocks it.
      await page.click('[data-iaction=refresh]');
      await page.waitForFunction(() => !document.querySelector('.orbit-opt[data-iolabel="SQLite"]')?.disabled && !document.querySelector('.orbit-ifeedback'));
      await page.close();
    }

    // 6. Decline with a reason: Escape backs out of the reason row, Enter confirms it.
    {
      const page = await open({ sessions: ['sess-approval'] });
      await page.waitForSelector('[data-idecision=decline]');
      await page.click('[data-idecision=decline]');
      await page.waitForSelector('[data-ifield="decline-reason"]');
      assert.equal(await focusedKey(page), 'INPUT:decline-reason');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      // The inline confirm button keeps its label on one line.
      const confirm = await page.locator('[data-iaction=confirm-decline]').boundingBox();
      const approve = await page.locator('[data-idecision=approve]').boundingBox();
      assert.ok(confirm.height <= approve.height + 2, `confirm button wraps: ${confirm.height} vs ${approve.height}`);
      await page.keyboard.press('Escape');
      assert.equal(await page.locator('[data-ifield="decline-reason"]').count(), 0);
      assert.equal((await invokes(page, 'desktop_panel')).length, 0);
      assert.equal(await focusedKey(page), 'BUTTON:decline');
      await page.click('[data-idecision=decline]');
      await page.keyboard.type('too risky');
      await composingKey(page, '[data-ifield="decline-reason"]', 'Enter');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      await page.keyboard.press('Enter');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const [decline] = await engineCalls(page, 'sessions.interaction.respond');
      assert.deepEqual([decline.action, decline.decisionId], ['decide', 'decline']);
      assert.equal(decline.reason, 'too risky');
      await page.close();
    }

    // 7. Handoff releases the request by id and brings the original window forward.
    {
      const page = await open({ sessions: ['sess-approval'] });
      await page.waitForSelector('[data-iaction=handoff]');
      await page.click('[data-iaction=handoff]');
      await page.waitForFunction(() => interactionTest.invokes('focus_session').length === 1);
      assert.deepEqual(await engineCalls(page, 'sessions.interaction.handoff'), [{ sessionId: 'sess-approval', requestId: 'req-1' }]);
      assert.equal((await invokes(page, 'focus_session'))[0].pid, 1234);
      assert.equal(await page.locator('[data-idecision=approve]').isDisabled(), true);
      assert.equal(await page.locator('[data-iaction=handoff]').isDisabled(), true);
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      await page.close();
    }

    // 8. A blocked answer re-reads the request and keeps the explanation on screen.
    {
      const changed = { adapter: 'claudeHook', canRespond: true, canHandoff: true, requestId: 'req-9', kind: 'approval', sessionId: 'sess-approval', stateKey: 'needsInput:sess-approval', promptHash: 'ph-new', questions: [], textPrompt: '', readonlyReason: '', approval: { toolName: 'Bash', reason: '', command: 'cargo build', details: '', decisions: [{ id: 'approve', label: '允许本次', style: 'primary' }] } };
      const page = await open({ sessions: ['sess-approval'], respond: [{ status: 'blocked', message: '审批请求已变化，请刷新后再操作' }] });
      await page.waitForSelector('[data-idecision=approve]');
      await page.evaluate((next) => { interactionTest.payloads['sess-approval'] = next; }, changed);
      await page.click('[data-idecision=approve]');
      await page.waitForFunction(() => document.querySelector('.orbit-iapproval .orbit-code')?.textContent === 'cargo build');
      assert.equal(await page.locator('.orbit-ifeedback[data-tone=warn]').textContent(), '审批请求已变化，请刷新后再操作');
      assert.equal((await engineCalls(page, 'sessions.interaction.inspect')).length, 2);
      assert.equal(await page.locator('[data-idecision=approve]').isDisabled(), false);
      await page.click('[data-idecision=approve]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond'))[1].promptHash, 'ph-new');
      await page.close();
    }

    // 9. A card with nothing to act on is re-read until the request shows up.
    {
      const page = await open({ sessions: ['sess-approval'], inspectQueue: { 'sess-approval': [{ kind: 'none' }] } });
      await page.waitForSelector('[data-ikind=none]');
      await page.waitForSelector('[data-idecision=approve]', { timeout: 8000 });
      assert.ok((await engineCalls(page, 'sessions.interaction.inspect')).length >= 2);
      await page.close();
    }

    // 10. Long unbroken text wraps inside the 368 px popover, and inline buttons stay on one line.
    {
      const long = LONG;
      const page = await open({
        sessions: ['sess-approval', 'sess-single'],
        payloads: {
          'sess-approval': {
            sessionId: 'sess-approval', stateKey: 'needsInput:sess-approval', promptHash: 'ph-long', kind: 'approval', adapter: 'claudeHook',
            canRespond: true, canHandoff: true, requestId: 'req-long', questions: [], textPrompt: '', readonlyReason: '',
            approval: { toolName: long.slice(0, 90), reason: long, command: long, details: long, decisions: [{ id: 'approve', label: long.slice(0, 120), style: 'primary' }, { id: 'decline', label: '拒绝', style: 'danger', acceptsReason: true }] },
          },
          'sess-single': {
            sessionId: 'sess-single', stateKey: 'needsInput:sess-single', promptHash: 'ph-long-q', kind: 'questions', adapter: 'claudeHook',
            canRespond: true, canHandoff: true, requestId: 'req-long-q', approval: null, textPrompt: '', readonlyReason: '',
            questions: [{ id: '1', key: '', question: long, kind: 'string', multiSelect: false, allowCustom: true, options: [{ label: long.slice(0, 300), description: long }, { label: 'Short', description: '' }] }],
          },
        },
        respond: [{ status: 'error', message: long }],
      });
      await page.waitForSelector('.orbit-iapproval .orbit-code');
      await noOverflow(page, 'approval');
      await page.click('[data-idecision=decline]');
      await page.waitForSelector('[data-ifield="decline-reason"]');
      await noOverflow(page, 'decline reason');
      await page.click('[data-iaction=confirm-decline]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=danger]');
      await noOverflow(page, 'long feedback');
      await page.click('[data-itarget="sess-single"]');
      await page.waitForSelector('.orbit-iquestions .orbit-opt');
      await noOverflow(page, 'questions');
      const send = await page.locator('.orbit-icustom-row [data-iaction=submit-questions]').boundingBox();
      const field = await page.locator('[data-icustom="1"]').boundingBox();
      assert.ok(send.height <= field.height + 4, `send button wraps: ${send.height} vs ${field.height}`);
      await page.close();
    }

    // 11. Read-only cards: enable the question hook, enable approvals, and jump to the window.
    {
      const page = await open({ sessions: ['sess-readonly'], status: { claudeQuestions: false, claudeApprovals: false } });
      await page.waitForSelector('[data-iaction="enable-claude-hook"]');
      assert.equal(await page.locator('[data-iaction="enable-claude-approvals"]').count(), 0);
      await page.click('.orbit-ireadonly [data-iaction=focus]');
      await page.waitForFunction(() => interactionTest.invokes('focus_session').length === 1);
      await page.click('[data-iaction="enable-claude-hook"]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.configure').length === 1);
      assert.deepEqual((await engineCalls(page, 'sessions.interaction.configure'))[0], { claudeQuestions: true });
      await page.close();
    }
    {
      const page = await open({ sessions: ['sess-locked-approval'], status: { claudeQuestions: false, claudeApprovals: false } });
      await page.waitForSelector('[data-iaction="enable-claude-approvals"]');
      assert.equal(await page.locator('[data-idecision]').count(), 0);
      await page.click('[data-iaction="enable-claude-approvals"]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      assert.deepEqual((await engineCalls(page, 'sessions.interaction.configure'))[0], { claudeQuestions: true, claudeApprovals: true });
      assert.equal((await engineCalls(page, 'sessions.interaction.inspect')).length, 2);
      // Nothing was approved by enabling the hook, and the button is gone once the setting is on.
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      assert.equal(await page.locator('[data-iaction="enable-claude-approvals"]').count(), 0);
      await page.close();
    }
    {
      // With approvals already on, a read-only approval offers no enable button.
      const page = await open({ sessions: ['sess-locked-approval'] });
      await page.waitForSelector('.orbit-iapproval');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.status').length > 0);
      await page.evaluate(() => interactionTest.emit({ summary: '等待输入' }));
      assert.equal(await page.locator('[data-iaction="enable-claude-approvals"]').count(), 0);
      await page.close();
    }

    // 12. Terminal delivery that only the native macOS app can perform is reported, not retried.
    {
      const page = await open({ sessions: ['sess-text'], respond: [{ status: 'needsLocalTerminal', terminalJob: {} }] });
      await page.waitForSelector('[data-ifield="text-reply"]');
      await page.fill('[data-ifield="text-reply"]', 'main');
      await page.click('[data-iaction=send-text]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=warn]');
      await page.waitForFunction(() => interactionTest.invokes('focus_session').length === 1);
      assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'main');
      await page.close();
    }

    // 13. The shell is told how tall the card is, and that it is gone when the session stops
    //     waiting; an auto-opened Linux popover asks for keyboard focus on the first press only.
    {
      const page = await open({ sessions: ['sess-approval'], auto: true });
      await page.waitForSelector('[data-idecision=approve]');
      await page.waitForFunction(() => interactionTest.invokes('desktop_orbit_popover').some((a) => a.height > 0));
      const heights = (await invokes(page, 'desktop_orbit_popover')).filter((a) => 'height' in a).map((a) => a.height);
      const expected = await page.evaluate(() => {
        const pop = document.querySelector('#orbit-popover');
        return pop.querySelector('.orbit-head').offsetHeight + pop.querySelector('.orbit-foot').offsetHeight + document.querySelector('#orbit-interaction').offsetHeight;
      });
      assert.ok(heights.at(-1) >= expected && heights.at(-1) <= expected + 40, `reported ${heights.at(-1)}, content ${expected}`);
      assert.equal((await invokes(page, 'desktop_orbit_popover')).some((a) => a.focus), false);
      await page.click('.orbit-icard-meta');
      await page.click('.orbit-icard-meta');
      await page.waitForFunction(() => interactionTest.invokes('desktop_orbit_popover').some((a) => a.focus === true));
      assert.equal((await invokes(page, 'desktop_orbit_popover')).filter((a) => a.focus).length, 1);
      await page.evaluate(() => {
        const t = interactionTest;
        t.emit({ phase: 'working', sessions: t.info.sessions.map((s) => ({ ...s, phase: 'working' })), petOrbit: t.info.petOrbit.map((a) => ({ ...a, phase: 'working' })) });
      });
      await page.waitForFunction(() => interactionTest.invokes('desktop_orbit_popover').filter((a) => 'height' in a).at(-1)?.height === 0);
      assert.equal(await page.locator('#orbit-interaction').isHidden(), true);
      await page.close();
    }

    // 13b. A stale auto flag says nothing about the full panel: once the auto-opened popover is
    //      gone, a panel the user opens parks the Linux keyboard focus on itself as usual.
    {
      const page = await open({ sessions: ['sess-approval'], auto: true });
      await page.waitForSelector('[data-idecision=approve]');
      await page.evaluate(() => interactionTest.emit({ panelOpen: false, orbitAgent: null }));
      await page.waitForFunction(() => document.body.dataset.view === 'ball');
      // The fixture's dashboard is too sparse for the full panel's own rendering, which is not under test.
      await page.evaluate(() => { window.render = () => {}; });
      await page.evaluate(() => interactionTest.emit({ panelOpen: true, orbitAgent: null, orbitAuto: true }));
      await page.waitForFunction(() => document.body.dataset.view === 'panel');
      await page.waitForFunction(() => document.activeElement?.id === 'panel', null, { timeout: 3000 });
      // The auto-opened popover itself still leaves the focus alone.
      await page.evaluate(() => interactionTest.emit({ panelOpen: false, orbitAgent: null }));
      await page.waitForFunction(() => document.body.dataset.view === 'ball');
      await page.evaluate(() => { document.activeElement?.blur(); interactionTest.emit({ panelOpen: true, orbitAgent: 'claude', orbitAuto: true }); });
      await page.waitForFunction(() => document.body.dataset.view === 'orbit');
      await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      assert.notEqual(await page.evaluate(() => document.activeElement?.id), 'orbit-popover');
      await page.close();
    }

    // 21. An answer given in an auto-opened popover closes it, also when the session resumes
    //     before the card's own timer runs or while the answer is still in flight. A popover the
    //     user opened, and an answer that was refused, stay.
    {
      const resume = (page) => page.evaluate(() => {
        const t = interactionTest;
        t.emit({ phase: 'working', sessions: t.info.sessions.map((s) => ({ ...s, phase: 'working' })), petOrbit: t.info.petOrbit.map((a) => ({ ...a, phase: 'working' })) });
      });
      const closes = async (page) => (await invokes(page, 'desktop_action')).filter((a) => a.action === 'close-panel').length;
      for (const scenario of [
        { name: 'resumed before the timer', options: { auto: true }, expected: 1 },
        { name: 'resumed while in flight', options: { auto: true, respondDelay: 400 }, inFlight: true, expected: 1 },
        { name: 'no status update', options: { auto: true }, quiet: true, expected: 1 },
        { name: 'opened by the user', options: { auto: false }, expected: 0 },
        { name: 'refused while in flight', options: { auto: true, respondDelay: 400, respond: [{ status: 'blocked', message: '审批请求已变化，请刷新后再操作' }] }, inFlight: true, expected: 0 },
      ]) {
        const page = await open({ sessions: ['sess-approval'], ...scenario.options });
        await page.waitForSelector('[data-idecision=approve]');
        await page.click('[data-idecision=approve]');
        if (scenario.inFlight) {
          await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 1);
          assert.equal(await page.locator('.orbit-ifeedback').count(), 0, scenario.name);
        } else {
          await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
        }
        if (!scenario.quiet) {
          await resume(page);
          assert.equal(await page.locator('#orbit-interaction').isHidden(), true, scenario.name);
        }
        await page.waitForTimeout(1300);
        // Later status updates do not ask again.
        if (!scenario.quiet) await resume(page);
        assert.equal(await closes(page), scenario.expected, scenario.name);
        assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 1, scenario.name);
        await page.close();
      }
    }

    // 22. Stopping the turn acts on the session: it still works on a card locked by an
    //     unconfirmed answer, and it does not unlock that answer.
    {
      const page = await open({
        sessions: ['sess-approval'], merge: true,
        payloads: { 'sess-approval': { followUp: { available: false, adapter: 'claudeHook', mode: '', reason: '', canInterrupt: true } } },
        respond: [{ status: 'uncertain', message: '连接中断，无法确认是否送达' }, { status: 'confirmed', message: '已中断本轮' }],
      });
      await page.waitForSelector('[data-idecision=approve]');
      await page.click('[data-idecision=approve]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=warn]');
      assert.equal(await page.locator('[data-idecision=approve]').isDisabled(), true);
      assert.equal(await page.locator('[data-iaction=interrupt]').isDisabled(), false);
      await page.click('[data-iaction=interrupt]');
      assert.equal(await page.locator('[data-iaction=interrupt]').textContent(), '确认中断');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 1);
      await page.click('[data-iaction=interrupt]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const calls = await engineCalls(page, 'sessions.interaction.respond');
      assert.deepEqual(calls.map((c) => c.action), ['decide', 'interrupt']);
      assert.equal('promptHash' in calls[1], false);
      assert.equal('requestId' in calls[1], false);
      // The request that may have been answered stays locked.
      await page.waitForSelector('[data-iaction=interrupt]:not([disabled])');
      assert.equal(await page.locator('[data-iaction=interrupt]').textContent(), '中断本轮');
      assert.equal(await page.locator('[data-idecision=approve]').isDisabled(), true);
      await page.evaluate(() => {
        const btn = document.querySelector('[data-idecision=approve]');
        btn.disabled = false;
        btn.click();
      });
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 2);
      await page.close();
    }

    // 23. A refused text keeps focus and caret in its field while the card re-reads the request.
    {
      const page = await open({ sessions: ['sess-text'], inspectDelay: { 'sess-text': 150 }, respond: [{ status: 'blocked', message: '回复过长' }] });
      await page.waitForSelector('[data-ifield="text-reply"]');
      await page.focus('[data-ifield="text-reply"]');
      await page.keyboard.type('hello');
      await page.keyboard.press('Control+Enter');
      // The reload goes through the loading card, which has no field.
      await page.waitForSelector('.orbit-iloading');
      await page.waitForSelector('.orbit-ifeedback[data-tone=warn]');
      await page.waitForSelector('[data-ifield="text-reply"]');
      assert.equal(await focusedKey(page), 'TEXTAREA:text-reply');
      await page.keyboard.type(' more');
      assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'hello more');
      await page.click('[data-iaction=send-text]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 2);
      assert.equal((await engineCalls(page, 'sessions.interaction.respond'))[1].text, 'hello more');
      await page.close();
    }

    // 14. Extended decisions come from the card's list: each is sent back by its own id, a note goes
    //     only with a decision that takes one, and descriptions are shown.
    {
      const page = await open({ sessions: ['sess-decisions'] });
      await page.waitForSelector('[data-idecision=approveSession]');
      assert.deepEqual(
        await page.locator('[data-idecision]').evaluateAll((nodes) => nodes.map((n) => n.dataset.idecision)),
        ['approve', 'approveSession', 'approveAlways', 'approveHost.2', 'decline', 'declineInterrupt'],
      );
      assert.equal(await page.locator('[data-idecision=approveAlways] small').textContent(), 'cargo test');
      assert.equal(await page.locator('.orbit-ititle').textContent(), 'Bash · tests');
      assert.match(await page.locator('.orbit-icwd').textContent(), /\/work\/repo/);
      assert.equal(await page.locator('.orbit-ipending-count').textContent(), '此会话共有 3 个待处理请求，按顺序逐个处理');
      assert.equal(await page.locator('[data-istep]').count(), 0);
      await noOverflow(page, 'decisions');
      // Keyboard: the buttons are reachable in order and Enter presses one.
      await page.focus('[data-idecision=approve]');
      // WebKit only tabs to buttons with Option held (the macOS default).
      await page.keyboard.press(engine === 'webkit' ? 'Alt+Tab' : 'Tab');
      assert.equal(await focusedKey(page), 'BUTTON:approveSession');
      await page.keyboard.press('Enter');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const [sent] = await engineCalls(page, 'sessions.interaction.respond');
      assert.equal(sent.action, 'decide');
      assert.equal(sent.decisionId, 'approveSession');
      assert.equal('reason' in sent, false);
      // More requests are waiting on this session: the card re-reads it instead of moving on.
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.inspect').length >= 2);
      await page.close();
    }
    {
      const page = await open({ sessions: ['sess-decisions'] });
      await page.waitForSelector('[data-idecision="approveHost.2"]');
      await page.click('[data-idecision="approveHost.2"]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 1);
      assert.equal((await engineCalls(page, 'sessions.interaction.respond'))[0].decisionId, 'approveHost.2');
      await page.close();
    }
    {
      // An empty note is not sent; "deny and stop" has no note and is sent at once.
      const page = await open({ sessions: ['sess-decisions'], respond: [{ status: 'blocked', message: '审批请求已变化，请刷新后再操作' }] });
      await page.waitForSelector('[data-idecision=decline]');
      await page.click('[data-idecision=decline]');
      await page.waitForSelector('[data-ifield="decline-reason"]');
      assert.equal(await page.locator('[data-idecision=decline]').getAttribute('aria-expanded'), 'true');
      await page.click('[data-iaction=confirm-decline]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 1);
      const [declined] = await engineCalls(page, 'sessions.interaction.respond');
      assert.equal(declined.decisionId, 'decline');
      assert.equal('reason' in declined, false);
      await page.waitForSelector('[data-idecision=declineInterrupt]:not([disabled])');
      await page.click('[data-idecision=declineInterrupt]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 2);
      assert.equal(await page.locator('[data-ifield="decline-reason"]').count(), 0);
      assert.equal((await engineCalls(page, 'sessions.interaction.respond'))[1].decisionId, 'declineInterrupt');
      await page.close();
    }

    // 15. Plan approval: the plan is rendered from escaped Markdown, the approve modes are listed,
    //     and "keep planning" takes feedback.
    {
      const page = await open({ sessions: ['sess-plan'] });
      await page.waitForSelector('.orbit-plan');
      assert.equal(await page.locator('.orbit-iapproval').getAttribute('data-approval'), 'plan');
      assert.equal(await page.locator('.orbit-plan [role=heading]').first().textContent(), 'Migration plan');
      assert.equal(await page.locator('.orbit-plan li').count(), 3);
      assert.equal(await page.locator('.orbit-plan pre').textContent(), 'cargo test <all>');
      assert.equal(await page.locator('.orbit-plan code').first().textContent(), 'users');
      // Markup inside the plan stays text.
      assert.equal(await page.locator('.orbit-plan img, .orbit-plan script').count(), 0);
      assert.match(await page.locator('.orbit-plan').textContent(), /<img src=x onerror=alert\(1\)>/);
      assert.equal(await page.locator('.orbit-itrunc').count(), 1);
      assert.equal(await page.locator('.orbit-plan').getAttribute('tabindex'), '0');
      await noOverflow(page, 'plan');
      assert.deepEqual(
        await page.locator('[data-idecision] span').allTextContents(),
        ['批准计划', '批准并自动接受编辑', '继续修改计划'],
      );
      await page.click('[data-idecision=decline]');
      await page.waitForSelector('[data-ifield="decline-reason"]');
      assert.equal(await page.locator('[data-ifield="decline-reason"]').getAttribute('placeholder'), '希望如何修改计划（可选）');
      assert.equal(await page.locator('[data-iaction=confirm-decline]').textContent(), '继续修改计划');
      await page.keyboard.type('Split step 2');
      await page.keyboard.press('Enter');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const [sent] = await engineCalls(page, 'sessions.interaction.respond');
      assert.deepEqual([sent.action, sent.decisionId, sent.reason], ['decide', 'decline', 'Split step 2']);
      await page.close();
    }
    {
      const page = await open({ sessions: ['sess-plan'] });
      await page.waitForSelector('[data-idecision=approveAcceptEdits]');
      await page.click('[data-idecision=approveAcceptEdits]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 1);
      assert.equal((await engineCalls(page, 'sessions.interaction.respond'))[0].decisionId, 'approveAcceptEdits');
      await page.close();
    }

    // 16. File changes: path, kind and a diff that reads without colour; further files are folded
    //     and open from the keyboard.
    {
      const page = await open({ sessions: ['sess-files'] });
      await page.waitForSelector('.orbit-change');
      assert.equal(await page.locator('.orbit-change').count(), 4);
      assert.deepEqual(await page.locator('.orbit-change-kind').allTextContents(), ['修改', '新增', '移动', '删除']);
      assert.equal(await page.locator('.orbit-change-path').nth(2).textContent(), 'src/old.rs → src/new.rs');
      assert.equal(await page.locator('.orbit-change').first().evaluate((n) => n.open), true);
      assert.equal(await page.locator('.orbit-change').nth(1).evaluate((n) => n.open), false);
      const kinds = await page.locator('.orbit-change').first().locator('.orbit-diff span').evaluateAll((nodes) => nodes.map((n) => `${n.dataset.diff}:${n.textContent}`));
      assert.deepEqual(kinds, ['ctx:--- a/src/main.rs', 'ctx:+++ b/src/main.rs', 'hunk:@@ -1,2 +1,2 @@', 'del:-fn old() {}', 'add:+fn new() {}', 'ctx: fn keep() {}']);
      // Without a diff the old and new text are shown as removed and added lines.
      await page.focus('[data-isummary="change:1"]');
      await page.keyboard.press('Enter');
      await page.waitForFunction(() => document.querySelectorAll('.orbit-change')[1].open);
      const added = await page.locator('.orbit-change').nth(1).locator('.orbit-diff span').evaluateAll((nodes) => nodes.map((n) => `${n.dataset.diff}:${n.textContent}`));
      assert.deepEqual(added, ['add:+ line one', 'add:+ line two']);
      assert.equal(await page.locator('.orbit-change').nth(1).locator('.orbit-itrunc').count(), 1);
      // The folded state survives a rebuild of the card, and so does keyboard focus.
      await page.evaluate(() => interactionTest.emit({ summary: '等待输入' }));
      await page.click('[data-idecision=decline]');
      await page.waitForSelector('[data-ifield="decline-reason"]');
      assert.equal(await page.locator('.orbit-change').nth(1).evaluate((n) => n.open), true);
      assert.deepEqual(await page.locator('.orbit-ipaths span').allTextContents(), ['/work/repo/src']);
      await noOverflow(page, 'file changes');
      await page.close();
    }

    // 17. MCP form: typed fields, a masked secret, an optional field, defaults, skip and cancel.
    {
      const page = await open({ sessions: ['sess-mcp'] });
      await page.waitForSelector('.orbit-form-head');
      assert.equal(await page.locator('[data-ibadge=form]').textContent(), '来自 MCP 服务 github');
      assert.equal(await page.locator('.orbit-form-title').textContent(), 'Deploy settings');
      assert.deepEqual(await page.locator('.orbit-qheader').allTextContents(), ['Region', 'Replicas', 'Token']);
      // Defaults are filled in: the enum by value, the number as text.
      assert.equal(await page.locator('.orbit-opt[data-iolabel="EU West"]').getAttribute('aria-checked'), 'true');
      assert.equal(await page.inputValue('[data-icustom="2"]'), '2');
      assert.equal(await page.locator('[data-icustom="2"]').getAttribute('inputmode'), 'numeric');
      // A one-choice click does not submit an MCP form.
      await page.click('.orbit-opt[data-iolabel="US East"]');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      // The secret is masked and its value never reaches the markup.
      assert.equal(await page.locator('[data-icustom="4"]').getAttribute('type'), 'password');
      assert.equal(await page.locator('[data-icustom="4"]').getAttribute('autocomplete'), 'off');
      assert.equal(await page.locator('[data-iaction=submit-questions]').isDisabled(), true);
      await page.fill('[data-icustom="4"]', 's3cr3t value');
      await page.click('[data-ipreview="1:US East"]');
      assert.equal(await page.inputValue('[data-icustom="4"]'), 's3cr3t value');
      assert.equal((await page.locator('#orbit-interaction').innerHTML()).includes('s3cr3t'), false);
      // Boolean and multi-select fields.
      await page.focus('.orbit-opt[data-iqid="3"][data-iolabel="是"]');
      await page.keyboard.press('ArrowDown');
      assert.equal(await focusedKey(page), 'BUTTON:否');
      await page.keyboard.press('Space');
      await page.click('.orbit-opt[data-iqid="6"][data-iolabel="waf"]');
      await page.click('.orbit-opt[data-iqid="6"][data-iolabel="cache"]');
      assert.equal(await page.locator('[data-iaction=submit-questions]').isDisabled(), false);
      // A number outside the range is refused before sending.
      await page.fill('[data-icustom="2"]', '9');
      assert.equal(await page.locator('[data-ierror="2"]').textContent(), '数值超出允许范围');
      assert.equal(await page.locator('[data-iaction=submit-questions]').isDisabled(), true);
      await page.fill('[data-icustom="2"]', '1.5');
      assert.equal(await page.locator('[data-ierror="2"]').textContent(), '请输入有效的整数');
      await page.fill('[data-icustom="2"]', '3');
      assert.equal(await page.locator('[data-ierror="2"]').isHidden(), true);
      // Enter in a field of a longer form does not submit it.
      await page.press('[data-icustom="2"]', 'Enter');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      await noOverflow(page, 'mcp form');
      await page.click('[data-iaction=submit-questions]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const [sent] = await engineCalls(page, 'sessions.interaction.respond');
      assert.equal(sent.action, 'answer');
      // The optional comment was left empty and is not sent.
      assert.deepEqual(sent.answers, { '1': 'US East', '2': '3', '3': '否', '4': 's3cr3t value', '6': ['cache', 'waf'] });
      await page.close();
    }
    for (const [button, action] of [['skip-form', 'skip'], ['cancel-form', 'cancel']]) {
      const page = await open({ sessions: ['sess-mcp'] });
      await page.waitForSelector(`[data-iaction=${button}]`);
      await page.click(`[data-iaction=${button}]`);
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const [sent] = await engineCalls(page, 'sessions.interaction.respond');
      assert.equal(sent.action, action);
      assert.equal('answers' in sent, false);
      assert.equal(await page.locator(`[data-iaction=${button}]`).isDisabled(), true);
      await page.close();
    }
    {
      // A form that cannot be filled here is read-only and names the link.
      const page = await open({ sessions: ['sess-mcp-url'] });
      await page.waitForSelector('.orbit-form-url');
      assert.equal(await page.locator('.orbit-form-url span').textContent(), 'https://example.invalid/authorize');
      assert.equal(await page.locator('[data-ibadge=readonly]').textContent(), '只读');
      assert.equal(await page.locator('[data-iaction=skip-form],[data-iaction=cancel-form],[data-iaction=submit-questions]').count(), 0);
      await page.click('.orbit-ireadonly [data-iaction=focus]');
      await page.waitForFunction(() => interactionTest.invokes('focus_session').length === 1);
      await page.close();
    }

    // 18. Several requests on one session: the stepper shows the picked one, the answer goes to
    //     that request, and the card then shows what is left.
    {
      const page = await open({ sessions: ['sess-queue'] });
      await page.waitForSelector('[data-istep="1"]');
      assert.equal(await page.locator('.orbit-ipending-count').textContent(), '第 1 个请求，共 2 个');
      assert.equal(await page.locator('.orbit-ipending-title').textContent(), 'git status');
      assert.equal(await page.locator('[data-istep="-1"]').isDisabled(), true);
      await page.focus('[data-istep="1"]');
      await page.keyboard.press('Enter');
      await page.waitForFunction(() => document.querySelector('.orbit-iapproval .orbit-code')?.textContent === 'rm -rf build');
      assert.equal(await page.locator('.orbit-ipending-count').textContent(), '第 2 个请求，共 2 个');
      assert.equal(await focusedKey(page), 'BUTTON:-1');
      assert.equal((await engineCalls(page, 'sessions.interaction.inspect')).at(-1).requestId, 'perm-2');
      // Only the second request is left once it has been answered.
      await page.evaluate(() => {
        const t = interactionTest;
        const first = t.payloads['sess-queue'];
        t.payloads['sess-queue'] = { ...first, pendingCount: 1, pendingRequests: [first.pendingRequests[0]] };
      });
      await page.click('[data-idecision=approve]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 1);
      const [sent] = await engineCalls(page, 'sessions.interaction.respond');
      assert.deepEqual([sent.requestId, sent.promptHash, sent.decisionId], ['perm-2', 'ph-perm-2', 'approve']);
      await page.waitForFunction(() => document.querySelector('.orbit-iapproval .orbit-code')?.textContent === 'git status' && !document.querySelector('[data-istep]'));
      assert.equal(await page.locator('[data-idecision=approve]').isDisabled(), false);
      // The result of the answered request is not shown on the next one.
      assert.equal(await page.locator('.orbit-ifeedback').count(), 0);
      assert.equal('requestId' in (await engineCalls(page, 'sessions.interaction.inspect')).at(-1), false);
      await page.close();
    }

    // 19. Follow-up and interrupt for a running session, offered only where inspect allows them.
    {
      const page = await open({
        sessions: ['sess-approval', 'sess-running', 'sess-busy', 'sess-remote-run'],
        phases: { 'sess-running': 'working', 'sess-busy': 'tool', 'sess-remote-run': 'working' },
        machines: { 'sess-remote-run': 'host-1' },
      });
      await page.waitForSelector('[data-ifollow="sess-running"]');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.inspect').some((c) => c.sessionId === 'sess-busy'));
      await page.evaluate(() => interactionTest.emit({ summary: '等待输入' }));
      assert.equal(await page.locator('[data-ifollow]').count(), 1);
      // Remote sessions are not probed.
      assert.equal((await engineCalls(page, 'sessions.interaction.inspect')).some((c) => c.sessionId === 'sess-remote-run'), false);
      assert.equal(await page.locator('[data-idecision=approve]').count(), 1);
      await page.click('[data-ifollow="sess-running"]');
      await page.waitForSelector('[data-ikind=followup] [data-ifield="text-reply"]');
      assert.equal(await focusedKey(page), 'TEXTAREA:text-reply');
      assert.equal(await page.locator('.orbit-itab').count(), 2);
      assert.equal(await page.locator('.orbit-itab[aria-selected=true]').getAttribute('data-waiting'), 'false');
      assert.equal(await page.locator('[data-ifollow="sess-running"]').getAttribute('aria-pressed'), 'true');
      assert.equal(await page.locator('.orbit-iadapter').textContent(), '会话仍在执行，指令将排队到本轮结束后执行');
      assert.equal(await page.locator('.orbit-iprompt').count(), 0);
      await page.keyboard.type('Also add tests');
      // The draft survives status updates and a change of the session's state key.
      await page.evaluate(() => {
        const t = interactionTest;
        t.payloads['sess-running'] = { ...t.payloads['sess-running'], stateKey: 'working:2', promptHash: 'ph-follow-2' };
        t.emit({ summary: '进行中' });
      });
      assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'Also add tests');
      await page.keyboard.press('Control+Enter');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 1);
      const [first] = await engineCalls(page, 'sessions.interaction.respond');
      assert.deepEqual([first.action, first.text, first.stateKey], ['followup', 'Also add tests', 'working:1']);
      // The stale state key was refused: the card re-reads, keeps the text and sends it again.
      await page.waitForSelector('.orbit-ifeedback[data-tone=warn]');
      await page.waitForSelector('[data-iaction=send-text]:not([disabled])');
      assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'Also add tests');
      await page.click('[data-iaction=send-text]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
      const second = (await engineCalls(page, 'sessions.interaction.respond'))[1];
      assert.deepEqual([second.action, second.text, second.stateKey, second.promptHash], ['followup', 'Also add tests', 'working:2', 'ph-follow-2']);
      assert.deepEqual(second.capabilities, { localTerminal: false });
      await page.waitForFunction(() => document.querySelector('[data-ifield="text-reply"]')?.value === '');
      assert.equal(await page.locator('[data-ifield="text-reply"]').isDisabled(), false);

      // Interrupt takes two presses and carries no request identity.
      await page.waitForSelector('[data-iaction=interrupt]:not([disabled])');
      await page.focus('[data-iaction=interrupt]');
      await page.keyboard.press('Enter');
      assert.equal(await page.locator('[data-iaction=interrupt]').textContent(), '确认中断');
      assert.equal(await focusedKey(page), 'BUTTON:interrupt');
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 2);
      await page.keyboard.press('Enter');
      await page.waitForFunction(() => interactionTest.engine('sessions.interaction.respond').length === 3);
      const stop = (await engineCalls(page, 'sessions.interaction.respond'))[2];
      assert.equal(stop.action, 'interrupt');
      assert.equal(stop.stateKey, 'working:2');
      assert.equal('promptHash' in stop, false);
      assert.equal('requestId' in stop, false);

      // Closing the follow-up returns to the waiting request.
      await page.waitForSelector('[data-iaction=close-follow]');
      await page.click('[data-iaction=close-follow]');
      await page.waitForSelector('[data-idecision=approve]');
      assert.equal(await page.locator('.orbit-itab').count(), 0);
      assert.equal(await page.locator('[data-ifollow="sess-running"]').getAttribute('aria-pressed'), 'false');
      await page.close();
    }
    {
      // No session is waiting: the list button opens the card by keyboard, and a session that
      // cannot take an instruction right now says why and can still be stopped.
      const page = await open({
        sessions: ['sess-stoppable'], phases: { 'sess-stoppable': 'working' }, card: false,
      });
      await page.waitForSelector('[data-ifollow="sess-stoppable"]');
      assert.equal(await page.locator('#orbit-interaction').isHidden(), true);
      await page.focus('[data-ifollow="sess-stoppable"]');
      await page.keyboard.press('Enter');
      await page.waitForSelector('[data-ikind=none]');
      assert.equal(await page.locator('[data-ikind=none]').textContent(), '原会话正在等待回答，请先处理当前请求');
      assert.equal(await page.locator('[data-ifield="text-reply"]').count(), 0);
      assert.equal(await focusedKey(page), 'BUTTON:interrupt');
      await page.waitForFunction(() => interactionTest.invokes('desktop_orbit_popover').some((a) => a.height > 0));
      await page.keyboard.press('Escape');
      await page.waitForFunction(() => interactionTest.invokes('desktop_panel').length === 1);
      await page.close();
    }

    // 20. Text cards say where their prompt comes from and send an answer; read-only cards are
    //     labelled, list what is asked and jump to the window (or say why they cannot).
    {
      const page = await open({ sessions: ['sess-generic'], respond: [{ status: 'uncertain', message: '连接中断，无法确认是否送达' }] });
      await page.waitForSelector('.orbit-iprompt');
      assert.equal(await page.locator('.orbit-iprompt-caption').textContent(), 'Agent 最后一条回复');
      assert.equal(await page.locator('.orbit-iprompt').textContent(), 'Should I continue with the migration?');
      await page.fill('[data-ifield="text-reply"]', 'yes');
      await page.click('[data-iaction=send-text]');
      await page.waitForSelector('.orbit-ifeedback[data-tone=warn]');
      const [sent] = await engineCalls(page, 'sessions.interaction.respond');
      assert.deepEqual([sent.action, sent.text, sent.promptHash], ['answer', 'yes', 'ph-sess-generic']);
      assert.deepEqual(sent.capabilities, { localTerminal: false });
      assert.deepEqual((await engineCalls(page, 'sessions.interaction.inspect'))[0], { sessionId: 'sess-generic', capabilities: { localTerminal: false } });
      // Unconfirmed: the text stays, the card locks and says to check the window first.
      assert.match(await page.locator('.orbit-ifeedback').textContent(), /连接中断，无法确认是否送达 结果未确认/);
      assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'yes');
      assert.equal(await page.locator('[data-ifield="text-reply"]').isDisabled(), true);
      await page.close();
    }
    {
      const page = await open({ sessions: ['sess-generic'], payloads: { 'sess-generic': { textPrompt: '', promptSource: 'none' } }, merge: true });
      await page.waitForSelector('[data-ifield="text-reply"]');
      assert.equal(await page.locator('.orbit-itext .orbit-ireadonly-reason').textContent(), '原会话正在等待输入，未读取到提示内容');
      await page.close();
    }
    {
      const page = await open({ sessions: ['sess-focus-questions'] });
      await page.waitForSelector('.orbit-iquestions .orbit-opt');
      assert.equal(await page.locator('[data-ibadge=readonly]').textContent(), '只读');
      assert.equal(await page.locator('.orbit-opt').count(), 2);
      assert.equal(await page.locator('.orbit-opt:not([disabled])').count(), 0);
      assert.equal(await page.locator('[data-icustom],[data-iaction=submit-questions]').count(), 0);
      assert.equal(await page.locator('.orbit-ireadonly-note .orbit-ireadonly-reason').textContent(), '此 Agent 的选项需要在原窗口选择，点击跳转后处理');
      await page.click('.orbit-iquestions .orbit-btn[data-iaction=focus]');
      await page.waitForFunction(() => interactionTest.invokes('focus_session').length === 1);
      assert.equal((await engineCalls(page, 'sessions.interaction.respond')).length, 0);
      await page.close();
    }
    {
      // A remote session cannot be brought forward: no jump button, and the card says where to go.
      const page = await open({ sessions: ['sess-focus-approval'], machines: { 'sess-focus-approval': 'host-1' } });
      await page.waitForSelector('.orbit-iapproval');
      assert.equal(await page.locator('.orbit-iapproval .orbit-code').first().textContent(), 'rm -rf probe_dir');
      assert.equal(await page.locator('[data-idecision]').count(), 0);
      assert.equal(await page.locator('[data-iaction=focus]').count(), 0);
      assert.match(await page.locator('.orbit-iapproval').textContent(), /远程会话无法从这里跳转，请在 build-box 的原终端处理/);
      await page.close();
    }
    {
      const page = await open({ sessions: ['sess-answered'] });
      await page.waitForSelector('[data-ikind=none]');
      assert.equal(await page.locator('[data-ikind=none]').textContent(), '已提交，等待原会话继续');
      await page.close();
    }

    assert.deepEqual(errors, []);
    console.log(`orbit-interaction UI tests passed (${engine})`);
  } finally {
    await browser.close();
    server.close();
  }
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
