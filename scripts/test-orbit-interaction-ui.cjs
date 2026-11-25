const assert = require('node:assert/strict');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require('../apps/desktop/node_modules/playwright');
const { serveWebRoot } = require('./ui-test-helpers.cjs');

function fixture() {
  const listeners = new Map();
  const state = {
    calls: [],
    status: {
      enabled: true,
      autoPopup: false,
      claudeQuestions: true,
      claudeApprovals: true,
      allowTerminal: true,
      desktopAllowTerminal: true,
      feishuPaired: false,
      hooks: [],
    },
    payloads: {
      'sess-approval': {
        sessionId: 'sess-approval',
        stateKey: 'needsInput:t1:h1',
        promptHash: 'ph-1',
        kind: 'approval',
        adapter: 'claudeHook',
        canRespond: true,
        canHandoff: true,
        needsClaudeHook: false,
        requestId: 'req-1',
        title: 's1 · Claude Code · 本机',
        approval: {
          toolName: 'Bash',
          reason: 'Run test suite',
          command: 'cargo test --workspace',
          details: '',
          decisions: [
            { id: 'approve', label: '允许本次', style: 'primary' },
            { id: 'decline', label: '拒绝', style: 'danger' },
          ],
        },
        questions: [],
        textPrompt: '',
        readonlyReason: '',
      },
      'sess-questions': {
        sessionId: 'sess-questions',
        stateKey: 'needsInput:t2:h2',
        promptHash: 'ph-2',
        kind: 'questions',
        adapter: 'claudeHook',
        canRespond: true,
        canHandoff: true,
        needsClaudeHook: false,
        requestId: 'req-2',
        title: 's2 · Claude Code · 本机',
        approval: null,
        questions: [
          {
            id: '1',
            key: '',
            question: 'Select target platforms',
            kind: 'string',
            multiSelect: true,
            allowCustom: true,
            options: [
              { label: 'macOS', description: 'SwiftUI native' },
              { label: 'Windows', description: 'Tauri desktop', preview: 'fn main() {}' },
            ],
          },
        ],
        textPrompt: '',
        readonlyReason: '',
      },
      'sess-text': {
        sessionId: 'sess-text',
        stateKey: 'needsInput:t3:h3',
        promptHash: 'ph-3',
        kind: 'text',
        adapter: 'tmux',
        canRespond: true,
        canHandoff: false,
        needsClaudeHook: false,
        requestId: '',
        title: 's3 · Kiro · 本机',
        approval: null,
        questions: [],
        textPrompt: 'Which branch should I rebase onto?',
        readonlyReason: '',
      },
      'sess-readonly': {
        sessionId: 'sess-readonly',
        stateKey: 'needsInput:t4:h4',
        promptHash: '',
        kind: 'readonly',
        adapter: 'focusOnly',
        canRespond: false,
        canHandoff: false,
        needsClaudeHook: true,
        requestId: '',
        title: 's4 · Claude Code · 本机',
        approval: null,
        questions: [],
        textPrompt: '1: Pick a database?',
        readonlyReason: '启用 Claude / Qoder 交互 Hook 后可直接在弹窗选择回答',
      },
    },
    info: {
      platform: 'linux',
      page: 'agent',
      mode: 'pet',
      effectiveMode: 'pet',
      nativePet: false,
      petEnabled: true,
      petScale: 1,
      panelOpen: true,
      panelPinned: false,
      orbitAgent: 'claude',
      orbitAuto: false,
      orbitPreferredSession: 'sess-approval',
      petOrbit: [
        {
          provider: 'claude',
          displayProvider: 'claude',
          phase: 'needsInput',
          activeCount: 2,
          overflowProviders: [],
          quota: null,
        },
      ],
      sessions: [
        {
          id: 'sess-approval',
          provider: 'claude',
          machine: '本机',
          machineId: 'local',
          sourceId: 'src-1',
          turnId: 't1',
          phase: 'needsInput',
          updatedAt: Date.now() / 1000,
          workspace: '/work/Aieyes',
          pid: 1234,
          processStart: 100,
          nativeSession: '11111111-2222-4333-8444-555555555555',
          inputStateKey: 'needsInput:t1:h1',
        },
        {
          id: 'sess-questions',
          provider: 'claude',
          machine: '本机',
          machineId: 'local',
          sourceId: 'src-1',
          turnId: 't2',
          phase: 'needsInput',
          updatedAt: Date.now() / 1000,
          workspace: '/work/Backend',
          pid: 1235,
          processStart: 101,
          nativeSession: '22222222-3333-4444-8555-666666666666',
          inputStateKey: 'needsInput:t2:h2',
        },
      ],
      summary: '等待输入',
      phase: 'needsInput',
      activeCount: 2,
      pet: { key: 'attention', behavior: 'attention', label: '等待输入', row: 3, frames: 4, frameMs: 140 },
    },
    emit(next) {
      Object.assign(state.info, next);
      for (const fn of listeners.get('desktop:status') || []) {
        fn({ payload: structuredClone(state.info) });
      }
    },
  };
  window.interactionTest = state;
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
        if (name === 'desktop_panel_ready') return {};
        if (name === 'desktop_action') return {};
        if (name === 'focus_session') return { outcome: 'window', name: 'Terminal' };
        if (name !== 'engine_call') return {};
        if (args.method === 'sessions.interaction.status') return structuredClone(state.status);
        if (args.method === 'sessions.interaction.configure') {
          Object.assign(state.status, args.params);
          return structuredClone(state.status);
        }
        if (args.method === 'sessions.interaction.inspect') {
          return structuredClone(state.payloads[args.params.sessionId] || { kind: 'none' });
        }
        if (args.method === 'sessions.interaction.respond') {
          return { status: 'confirmed', message: '已允许本次操作' };
        }
        if (args.method === 'sessions.interaction.handoff') {
          return { handedOff: true, message: '已交还给原终端确认' };
        }
        const defaults = {
          'hello': { version: 'fixture' },
          'settings.get': { accounts: [], sources: [], hosts: [], appearance: { language: 'zh-CN' } },
          'accounts.status.get': [],
          'accounts.status.refreshBatch': [],
          'dashboard': { summary: {}, models: [], quotas: [], sources: [], hosts: [] },
          'prices.list': [],
          'hosts.sample': [],
          'sessions.list': { sessions: [], unavailable: false },
          'wakeups.list': [],
          'quotas.refresh': [],
          'sources.scan': [],
        };
        return structuredClone(defaults[args.method] ?? []);
      },
    },
  };
}

(async () => {
  const root = path.resolve(__dirname, '../apps/desktop/web');
  const server = http.createServer(serveWebRoot(root, 'floating.html'));
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage({ viewport: { width: 380, height: 500 } });
    const errors = [];
    page.on('pageerror', (e) => errors.push(e.stack || String(e)));
    await page.addInitScript(fixture);
    await page.goto(`http://127.0.0.1:${server.address().port}/floating.html`);

    // 1. Approval card renders with command & allow/deny/handoff actions
    await page.waitForSelector('.orbit-iapproval .orbit-code');
    assert.equal(await page.locator('.orbit-iapproval .orbit-code').textContent(), 'cargo test --workspace');
    assert.equal(await page.locator('.orbit-itab').count(), 2);

    // Click Approve and verify RPC payload
    await page.click('[data-idecision=approve]');
    await page.waitForSelector('.orbit-ifeedback[data-tone=ok]');
    const approveCall = await page.evaluate(() =>
      interactionTest.calls.find((c) => c.args?.method === 'sessions.interaction.respond')
    );
    assert.equal(approveCall.args.params.action, 'approve');
    assert.equal(approveCall.args.params.promptHash, 'ph-1');

    // 2. Switch to multi-select questions tab, toggle preview & options, submit
    await page.click('[data-itarget="sess-questions"]');
    await page.waitForSelector('.orbit-iquestions .orbit-opt');
    await page.click('[data-ipreview="1:Windows"]');
    assert.equal(await page.locator('.orbit-iquestions .orbit-code').textContent(), 'fn main() {}');
    await page.click('.orbit-opt[data-iolabel="macOS"]');
    await page.click('.orbit-opt[data-iolabel="Windows"]');

    // Emit 1s polling status update while selections are in progress — selections must survive
    await page.evaluate(() => interactionTest.emit({ activeCount: 2 }));
    assert.equal(await page.locator('.orbit-opt[data-iolabel="macOS"]').getAttribute('data-selected'), 'true');
    assert.equal(await page.locator('.orbit-opt[data-iolabel="Windows"]').getAttribute('data-selected'), 'true');

    await page.click('[data-iaction="submit-questions"]');
    const answerCall = await page.evaluate(() =>
      interactionTest.calls.filter((c) => c.args?.method === 'sessions.interaction.respond').at(-1)
    );
    assert.deepEqual(answerCall.args.params.answers, { '1': ['macOS', 'Windows'] });

    // 3. Text reply card survives 1s polling ticks without wiping user draft
    await page.evaluate(() => {
      interactionTest.emit({
        orbitAgent: 'kiro',
        orbitPreferredSession: 'sess-text',
        petOrbit: [{ provider: 'kiro', displayProvider: 'kiro', phase: 'needsInput', activeCount: 1, overflowProviders: [], quota: null }],
        sessions: [{
          id: 'sess-text', provider: 'kiro', machine: '本机', machineId: 'local', sourceId: 's-kiro',
          turnId: 't3', phase: 'needsInput', updatedAt: Date.now() / 1000, workspace: '/work/App',
          pid: 1236, processStart: 102, nativeSession: 'kiro-1', inputStateKey: 'needsInput:t3:h3',
        }],
      });
    });
    await page.waitForSelector('[data-ifield="text-reply"]');
    await page.fill('[data-ifield="text-reply"]', 'rebase onto main');
    await page.evaluate(() => interactionTest.emit({ summary: '等待输入' }));
    assert.equal(await page.inputValue('[data-ifield="text-reply"]'), 'rebase onto main');

    // 4. Readonly card shows prompt + quick-enable Claude hook button + focus button
    await page.evaluate(() => {
      interactionTest.emit({
        orbitAgent: 'claude',
        orbitPreferredSession: 'sess-readonly',
        petOrbit: [{ provider: 'claude', displayProvider: 'claude', phase: 'needsInput', activeCount: 1, overflowProviders: [], quota: null }],
        sessions: [{
          id: 'sess-readonly', provider: 'claude', machine: '本机', machineId: 'local', sourceId: 'src-1',
          turnId: 't4', phase: 'needsInput', updatedAt: Date.now() / 1000, workspace: '/work/Docs',
          pid: 1237, processStart: 103, nativeSession: '44444444-5555-4666-8777-888888888888', inputStateKey: 'needsInput:t4:h4',
        }],
      });
    });
    await page.waitForSelector('[data-iaction="enable-claude-hook"]');
    await page.click('[data-iaction="enable-claude-hook"]');
    const cfgCall = await page.evaluate(() =>
      interactionTest.calls.find((c) => c.args?.method === 'sessions.interaction.configure')
    );
    assert.equal(cfgCall.args.params.claudeQuestions, true);

    assert.deepEqual(errors, []);
    console.log('orbit-interaction UI tests passed');
  } finally {
    await browser.close();
    server.close();
  }
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
