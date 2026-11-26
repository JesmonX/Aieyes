/* Interactive waiting-for-input cards for the pet orbit popover and desktop settings. */
(() => {
  const esc = (v) => (window.AieyesUI ? window.AieyesUI.escapeHTML(v) : String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c])));
  const tr = (text, ...args) => (window.AieyesI18n ? window.AieyesI18n.t(text, ...args) : text);

  async function callCore(method, params = {}) {
    if (typeof window.api === 'function') return window.api(method, params);
    return window.__TAURI__.core.invoke('engine_call', { method, params });
  }

  let statusCache = {
    enabled: true,
    autoPopup: false,
    claudeQuestions: false,
    claudeApprovals: false,
    allowTerminal: false,
  };
  let statusLoaded = false;
  // The card being shown: { container, session, tabs, waiting, active, info }. Handlers read it when
  // they run, so a response always uses the latest session list and never a stale closure.
  let ctx = null;
  let activeSessionId = null;
  let renderedCardSig = null;
  let renderedTabSig = null;
  let currentPayload = null;
  // Inspect requests are tracked per request: only the newest one may apply its result.
  let loadSeq = 0;
  let loadingSessionId = null;
  let lastInspectAt = 0;
  let submitting = false;
  // Only the newest submit may clear `submitting`; an answer still in flight for a card that was
  // switched away must not re-enable the buttons of the card shown now.
  let submitToken = 0;
  // The request that was just answered or handed off. Its controls stay disabled until the core
  // reports a different request, or the user refreshes, so one prompt is never answered twice.
  let resolvedKey = null;
  let feedback = null; // { status, message }
  // The decision whose optional note is being typed (a decision with `acceptsReason`).
  let reasonFor = null;
  // Stopping a turn takes two presses: the first arms the button for a few seconds.
  let interruptArmed = false;
  let armTimer = 0;
  // An IME composition owns its text field; a rebuild would abort it.
  let composing = false;
  let renderDeferred = false;
  let pollTimer = 0;
  let advanceTimer = 0;
  // The answer that closes an auto-opened popover once nothing is left to answer: { sessionId,
  // settled }. It outlives the card, so the popover also closes when the session resumes before
  // the advance timer runs, or while the answer is still in flight.
  let autoClose = null;
  let lastInfo = null;
  let reportedDraft = null;
  // A running session whose follow-up card was opened from the session list.
  let followTarget = null;
  // The card opened by a press on a list button takes keyboard focus once it has loaded.
  let focusOnLoad = false;
  // sessionId -> requestId picked in the request stepper (adapters that list `pendingRequests`).
  const pickedRequest = new Map();
  // sessionId -> { key, at, can }: whether a running local session takes a follow-up or an interrupt.
  const followProbes = new Map();
  let probing = false;
  const drafts = new Map(); // draftKey -> see getDraft
  const CLAUDE_HOOK_PROVIDERS = ['claude', 'qoder', 'qoder-cn'];
  const POLL_STALE_MS = 2500;
  const POLL_LIVE_MS = 5000;
  const PROBE_TTL_MS = 30000;
  const PROBE_LIMIT = 8;
  // This shell cannot type a `terminalJob` into the user's terminal; only the native macOS app can.
  const CAPABILITIES = { localTerminal: false };

  const list = (value) => (Array.isArray(value) ? value : []);
  const waitingPhase = (session) => session?.phase === 'needsInput';

  /* Every field the card reads has a value, whatever the core version sent. */
  function normalize(result, sessionId) {
    const p = result && typeof result === 'object' ? result : {};
    return {
      ...p,
      sessionId: p.sessionId || sessionId,
      stateKey: p.stateKey || '',
      promptHash: p.promptHash || '',
      requestId: p.requestId || '',
      kind: p.kind || 'none',
      adapter: p.adapter || 'focusOnly',
      approval: p.approval && typeof p.approval === 'object' ? p.approval : null,
      questions: list(p.questions),
      form: p.form && typeof p.form === 'object' ? p.form : null,
      pendingCount: Number(p.pendingCount) || 0,
      pendingRequests: list(p.pendingRequests),
      followUp: { available: false, adapter: '', mode: '', reason: '', canInterrupt: false, ...(p.followUp && typeof p.followUp === 'object' ? p.followUp : {}) },
    };
  }
  /* A follow-up is typed for the session, not for one request: it survives the turn ending. */
  const followCard = (payload) => payload.kind === 'followup' || (payload.kind === 'none' && !payload.answered);
  function draftKey(payload) {
    return followCard(payload) ? `${payload.sessionId}|followup` : `${payload.sessionId}|${payload.stateKey}|${payload.promptHash}`;
  }
  function payloadKey(payload) {
    return `${payload.sessionId}|${payload.stateKey}|${payload.promptHash}|${payload.requestId || ''}`;
  }
  const samePayload = (a, b) => !!a && !!b && JSON.stringify(a) === JSON.stringify(b);
  const sameRequest = (a, b) => !!a && !!b && a.kind === b.kind && payloadKey(a) === payloadKey(b);
  const options = (q) => list(q.options);
  const numeric = (q) => (q.kind === 'integer' || q.kind === 'number') && !options(q).length;
  const customAllowed = (q) => !!q.allowCustom || !options(q).length;

  function getDraft(payload) {
    const key = draftKey(payload);
    if (!drafts.has(key)) {
      const draft = { sessionId: payload.sessionId, follow: followCard(payload), touched: false, single: {}, multi: {}, custom: {}, text: '', reason: '', previews: new Set(), open: new Map() };
      // Defaults the form declares are filled in, never secrets.
      for (const q of payload.questions) {
        if (!payload.canRespond || q.secret || q.default === undefined || q.default === null || q.multiSelect) continue;
        const wanted = String(q.default);
        const match = options(q).find((o) => (o.value !== undefined && String(o.value) === wanted) || o.label === wanted);
        if (match) draft.single[q.id] = match.label;
        else if (customAllowed(q)) draft.custom[q.id] = wanted;
      }
      drafts.set(key, draft);
    }
    return drafts.get(key);
  }
  function clearDraft(payload) {
    if (payload) drafts.delete(draftKey(payload));
  }
  function draftHasContent(draft) {
    return draft.touched && !!(draft.text.trim() || draft.reason.trim() ||
      Object.values(draft.custom).some((v) => String(v || '').trim()) ||
      Object.values(draft.single).some(Boolean) ||
      Object.values(draft.multi).some((picked) => Array.isArray(picked) && picked.length));
  }
  /* Drafts belong to one waiting request, or to the follow-up card that is open. */
  function pruneDrafts(info) {
    if (!Array.isArray(info?.sessions)) return;
    const waitingIds = new Set(info.sessions.filter(waitingPhase).map((s) => s.id));
    const known = new Set(info.sessions.map((s) => s.id));
    for (const [key, draft] of drafts) {
      const keep = draft.follow ? draft.sessionId === followTarget && known.has(draft.sessionId) : waitingIds.has(draft.sessionId);
      if (!keep) drafts.delete(key);
    }
  }
  function hasDraft() {
    if (!ctx) return false;
    const ids = new Set(ctx.tabs.map((s) => s.id));
    for (const draft of drafts.values()) {
      if (ids.has(draft.sessionId) && draftHasContent(draft)) return true;
    }
    return false;
  }
  /* The shell keeps a popover with unsent input open when the user clicks another window. */
  function reportDraft() {
    const next = hasDraft();
    if (next === reportedDraft) return;
    reportedDraft = next;
    window.__TAURI__?.core?.invoke('desktop_orbit_popover', { draft: next })?.catch?.(() => {});
  }
  const isResolved = (payload) => !!payload && resolvedKey !== null && resolvedKey === payloadKey(payload);
  const locked = () => submitting || isResolved(currentPayload);
  const isLoading = () => loadingSessionId !== null;
  const imeEvent = (event) => composing || event.isComposing || event.keyCode === 229;
  /* One plain choice answers on click. A form that can also be skipped or cancelled, a secret or
     an optional field is sent with the explicit button instead. */
  function singleDirect(payload) {
    const q = payload.questions[0];
    return !!payload.canRespond && payload.questions.length === 1 && !q.multiSelect && options(q).length > 0 &&
      q.required !== false && !q.secret && payload.form?.source !== 'mcp';
  }
  /* Claude / Qoder approvals stay read-only until the approval hook is on. The core flags it when it
     can; older cores only say so in the reason text, so fall back to the provider and the setting. */
  function needsApprovalHook(payload, session) {
    if (payload.canRespond) return false;
    if (typeof payload.needsClaudeApprovals === 'boolean') return payload.needsClaudeApprovals;
    if (typeof payload.quickEnable === 'string') return payload.quickEnable === 'approvals';
    if (payload.needsClaudeHook || !statusLoaded || statusCache.claudeApprovals) return false;
    // Only a pending tool approval; a read-only text prompt would not become answerable.
    if (payload.kind !== 'approval' && !payload.approval) return false;
    return CLAUDE_HOOK_PROVIDERS.includes(session.provider);
  }
  const needsQuestionHook = (payload) => !payload.canRespond && (!!payload.needsClaudeHook || payload.quickEnable === 'questions');

  async function refreshStatus() {
    try {
      const next = await callCore('sessions.interaction.status', {});
      if (next && typeof next === 'object') {
        statusCache = { ...statusCache, ...next };
        statusLoaded = true;
      }
    } catch {
      /* keep defaults */
    }
    return statusCache;
  }

  function formatSessionMeta(session) {
    const parts = [];
    if (session.machine) parts.push(session.machine);
    if (session.nativeSession) parts.push('#' + String(session.nativeSession).slice(-8));
    return parts.join(' · ');
  }

  function sessionTitle(session) {
    if (session.workspace) {
      const trimmed = String(session.workspace).replace(/[\\/]+$/, '');
      const base = trimmed.split(/[\\/]/).pop();
      if (base) return base;
    }
    return window.AieyesContract?.providers?.[session.provider] || session.provider;
  }

  /* The answer a question would send: a string, a list for multi-select, or undefined when empty. */
  function answerOf(q, draft) {
    const raw = String(draft.custom[q.id] || '');
    // A secret is sent exactly as typed.
    const custom = customAllowed(q) ? (q.secret ? raw : raw.trim()) : '';
    if (q.multiSelect) {
      // Option order, as on macOS: the same selection gives the same answer on every platform.
      const chosen = list(draft.multi[q.id]);
      const picked = options(q).map((o) => o.label).filter((label) => chosen.includes(label));
      if (custom) picked.push(custom);
      return picked.length ? picked : undefined;
    }
    if (custom) return custom;
    return draft.single[q.id] || undefined;
  }
  /* Typed fields are checked before sending; the core checks everything again. */
  function questionError(q, draft) {
    if (!numeric(q)) return '';
    const text = String(draft.custom[q.id] || '').trim();
    if (!text) return '';
    if (q.kind === 'integer' && !/^[+-]?\d+$/.test(text)) return tr('请输入有效的整数');
    const value = Number(text);
    if (!Number.isFinite(value)) return tr('请输入有效的数字');
    const { minimum, maximum } = q.constraints || {};
    if ((typeof minimum === 'number' && value < minimum) || (typeof maximum === 'number' && value > maximum)) return tr('数值超出允许范围');
    return '';
  }
  /* Required questions are answered, optional ones may stay empty, and something is answered. */
  function allQuestionsAnswered(payload, draft) {
    if (!payload.questions.length) return false;
    let answered = 0;
    for (const q of payload.questions) {
      if (questionError(q, draft)) return false;
      if (answerOf(q, draft) !== undefined) answered += 1;
      else if (q.required !== false) return false;
    }
    return answered > 0;
  }
  function buildAnswers(payload, draft) {
    const out = {};
    for (const q of payload.questions) {
      const answer = answerOf(q, draft);
      if (answer !== undefined) out[q.id] = answer;
    }
    return out;
  }

  function renderFeedback() {
    if (!feedback) return '';
    const tone = feedback.status === 'confirmed' || feedback.status === 'submitted'
      ? 'ok'
      : feedback.status === 'error' ? 'danger' : 'warn';
    return `<div class="orbit-ifeedback" data-tone="${tone}" role="status">${esc(feedback.message)}</div>`;
  }

  function tabsHTML() {
    const { session, tabs } = ctx;
    if (tabs.length < 2) return '';
    return `<div class="orbit-itabs" role="tablist">${tabs.map((item) => {
      const picked = item.id === session.id;
      return `<button type="button" class="orbit-itab" role="tab" aria-selected="${picked}" data-itarget="${esc(item.id)}" data-waiting="${waitingPhase(item)}"><span class="orbit-idot" aria-hidden="true"></span><span>${esc(sessionTitle(item))}</span></button>`;
    }).join('')}</div>`;
  }

  const optionMark = (multi, selected) => (multi ? (selected ? '☑' : '☐') : (selected ? '●' : '○'));

  /* Plan text is the agent's Markdown. Only headings, lists, code and emphasis are rendered, from
     escaped text, so nothing in a plan can become markup of its own. */
  function inlineMarkdown(text) {
    return esc(text).replace(/`([^`]+)`/g, '<code>$1</code>').replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  }
  function markdownHTML(markdown) {
    const out = [];
    let items = null, code = null, para = [];
    const flushPara = () => {
      if (para.length) out.push(`<p>${para.map(inlineMarkdown).join('<br>')}</p>`);
      para = [];
    };
    const flushList = () => {
      if (items) out.push(`<${items.tag}>${items.rows.map((row) => `<li>${row}</li>`).join('')}</${items.tag}>`);
      items = null;
    };
    for (const line of String(markdown).split(/\r?\n/)) {
      if (code) {
        if (/^\s*```/.test(line)) {
          out.push(`<pre class="orbit-code">${esc(code.join('\n'))}</pre>`);
          code = null;
        } else code.push(line);
        continue;
      }
      if (/^\s*```/.test(line)) {
        flushPara();
        flushList();
        code = [];
        continue;
      }
      const heading = /^(#{1,6})\s+(.*)$/.exec(line);
      const item = /^\s*(?:([-*+])|\d+[.)])\s+(.*)$/.exec(line);
      if (heading) {
        flushPara();
        flushList();
        out.push(`<div class="orbit-plan-heading" role="heading" aria-level="${Math.min(6, heading[1].length + 2)}">${inlineMarkdown(heading[2])}</div>`);
      } else if (item) {
        flushPara();
        const tag = item[1] ? 'ul' : 'ol';
        if (items && items.tag !== tag) flushList();
        items ||= { tag, rows: [] };
        items.rows.push(inlineMarkdown(item[2]));
      } else if (!line.trim()) {
        flushPara();
        flushList();
      } else {
        flushList();
        para.push(line);
      }
    }
    if (code) out.push(`<pre class="orbit-code">${esc(code.join('\n'))}</pre>`);
    flushPara();
    flushList();
    return out.join('');
  }

  const truncatedNote = () => `<small class="orbit-itrunc">${esc(tr('内容过长，已截断显示；完整内容请在原窗口查看'))}</small>`;
  const CHANGE_KINDS = { add: '新增', update: '修改', delete: '删除', move: '移动' };
  /* Each line keeps its + or - prefix, so the diff reads without colour. */
  function diffHTML(change) {
    const row = (kind, text) => `<span data-diff="${kind}">${esc(text)}</span>`;
    let rows = [];
    if (change.diff) {
      rows = String(change.diff).replace(/\n$/, '').split('\n').map((line) => row(
        /^@@/.test(line) ? 'hunk' : /^\+(?!\+\+ )/.test(line) ? 'add' : /^-(?!-- )/.test(line) ? 'del' : 'ctx', line));
    } else {
      const side = (text, kind, mark) => (text ? String(text).replace(/\n$/, '').split('\n').map((line) => row(kind, `${mark} ${line}`)) : []);
      rows = [...side(change.oldText, 'del', '-'), ...side(change.newText, 'add', '+')];
    }
    return rows.length ? `<pre class="orbit-code orbit-diff">${rows.join('')}</pre>` : '';
  }
  function changesHTML(approval, draft) {
    const changes = list(approval.changes);
    if (!changes.length) return '';
    return `<div class="orbit-changes">${changes.map((change, index) => {
      const key = `change:${index}`;
      const open = draft.open.has(key) ? draft.open.get(key) : index === 0;
      const kind = CHANGE_KINDS[change.kind] ? tr(CHANGE_KINDS[change.kind]) : tr('文件');
      const path = change.movePath ? `${change.path} → ${change.movePath}` : change.path;
      const body = diffHTML(change) || `<small class="orbit-itrunc">${esc(tr('没有可显示的差异'))}</small>`;
      return `<details class="orbit-change" data-idetails="${key}" ${open ? 'open' : ''}>
        <summary data-isummary="${key}"><span class="orbit-change-kind" data-kind="${esc(change.kind || 'unknown')}">${esc(kind)}</span><span class="orbit-change-path">${esc(path || '')}</span></summary>
        ${body}${change.truncated ? truncatedNote() : ''}
      </details>`;
    }).join('')}</div>`;
  }

  function readonlyNote(payload, session, canFocus) {
    const remote = !canFocus && session.machineId && session.machineId !== 'local'
      ? `<div class="orbit-ireadonly-reason">${esc(tr('远程会话无法从这里跳转，请在 {0} 的原终端处理', session.machine || session.machineId))}</div>`
      : '';
    return `<div class="orbit-ireadonly-note"><span class="orbit-ibadge" data-ibadge="readonly">${esc(tr('只读'))}</span><span class="orbit-ireadonly-reason">${esc(payload.readonlyReason || tr('此请求需要在原窗口处理'))}</span></div>${remote}`;
  }

  /* Several requests can wait on one session. Adapters that list them let the user step through;
     the others show the oldest first, as their own window does. */
  function pendingHTML(payload) {
    const requests = payload.pendingRequests;
    if (requests.length > 1) {
      const index = Math.max(0, requests.findIndex((item) => item.requestId === payload.requestId));
      const step = (dir, label, mark) => `<button type="button" class="orbit-icon-btn" data-istep="${dir}" title="${esc(label)}" aria-label="${esc(label)}" ${submitting || index + dir < 0 || index + dir >= requests.length ? 'disabled' : ''}>${mark}</button>`;
      return `<div class="orbit-ipending" role="group" aria-label="${esc(tr('待处理请求'))}">${step(-1, tr('上一个请求'), '‹')}<span class="orbit-ipending-count" aria-live="polite">${esc(tr('第 {0} 个请求，共 {1} 个', index + 1, requests.length))}</span>${step(1, tr('下一个请求'), '›')}<span class="orbit-ipending-title">${esc(requests[index]?.title || '')}</span></div>`;
    }
    if (payload.pendingCount > 1) {
      return `<div class="orbit-ipending"><span class="orbit-ipending-count">${esc(tr('此会话共有 {0} 个待处理请求，按顺序逐个处理', payload.pendingCount))}</span></div>`;
    }
    return '';
  }

  function formHTML(payload) {
    const form = payload.form;
    if (!form) return '';
    const source = form.source === 'mcp' ? (form.serverName ? tr('来自 MCP 服务 {0}', form.serverName) : tr('MCP 表单')) : '';
    const parts = [
      source ? `<span class="orbit-ibadge" data-ibadge="form">${esc(source)}</span>` : '',
      form.title ? `<strong class="orbit-form-title">${esc(form.title)}</strong>` : '',
      form.description ? `<div class="orbit-form-desc">${esc(form.description)}</div>` : '',
      form.url ? `<div class="orbit-form-url"><small>${esc(tr('需要在原窗口打开的链接'))}</small><span>${esc(form.url)}</span></div>` : '',
    ].filter(Boolean);
    return parts.length ? `<div class="orbit-form-head">${parts.join('')}</div>` : '';
  }

  function promptHTML(payload) {
    if (!payload.textPrompt) return '';
    const caption = payload.promptSource === 'lastMessage' ? `<small class="orbit-iprompt-caption">${esc(tr('Agent 最后一条回复'))}</small>` : '';
    return `${caption}<div class="orbit-iprompt" tabindex="0" role="group" aria-label="${esc(tr('提示内容'))}">${esc(payload.textPrompt)}</div>`;
  }

  function interruptButton(payload) {
    if (!payload.followUp.canInterrupt) return '';
    return `<button type="button" class="orbit-btn danger" data-iaction="interrupt" data-armed="${interruptArmed}" title="${esc(tr('中断会撤销本轮未完成的操作和待处理请求'))}" ${submitting ? 'disabled' : ''}>${esc(interruptArmed ? tr('确认中断') : tr('中断本轮'))}</button>`;
  }

  function approvalHTML(p, session, parts) {
    const a = p.approval;
    const draft = getDraft(p);
    const off = locked();
    const decisions = p.canRespond ? list(a.decisions) : [];
    const noted = decisions.find((d) => d.id === reasonFor && d.acceptsReason);
    const cwd = a.cwd && a.cwd !== a.details ? `<div class="orbit-icwd" title="${esc(a.cwd)}">${esc(tr('工作目录：{0}', a.cwd))}</div>` : '';
    const plan = a.plan?.markdown
      ? `<div class="orbit-plan" tabindex="0" role="group" aria-label="${esc(tr('计划内容'))}">${markdownHTML(a.plan.markdown)}</div>${a.plan.truncated ? truncatedNote() : ''}`
      : '';
    const paths = list(a.paths).length
      ? `<div class="orbit-ipaths"><small>${esc(tr('涉及路径'))}</small>${list(a.paths).map((path) => `<span>${esc(path)}</span>`).join('')}</div>`
      : '';
    const reasonRow = noted && !isResolved(p)
      ? `<div class="orbit-idecline-row"><input type="text" class="orbit-input" data-ifield="decline-reason" aria-label="${esc(tr('给 Agent 的说明（可选）'))}" placeholder="${esc(a.kind === 'plan' ? tr('希望如何修改计划（可选）') : noted.behavior === 'deny' ? tr('拒绝原因（可选）') : tr('给 Agent 的说明（可选）'))}" value="${esc(draft.reason)}"><button type="button" class="orbit-btn ${noted.style === 'danger' ? 'danger' : 'primary'}" data-iaction="confirm-decline" ${off ? 'disabled' : ''}>${esc(a.kind !== 'plan' && noted.behavior === 'deny' ? tr('确认拒绝') : noted.label)}</button></div>`
      : '';
    const actions = p.canRespond
      ? `<div class="orbit-iactions">${decisions.map((d) => {
          const cls = d.style === 'primary' ? 'primary' : d.style === 'danger' ? 'danger' : '';
          return `<button type="button" class="orbit-btn orbit-decision ${cls}" data-idecision="${esc(d.id)}" ${d.acceptsReason ? `aria-expanded="${noted?.id === d.id}"` : ''} ${off ? 'disabled' : ''}><span>${esc(d.label || d.id)}</span>${d.description ? `<small>${esc(d.description)}</small>` : ''}</button>`;
        }).join('')}<span class="orbit-ispacer"></span>${interruptButton(p)}${p.canHandoff ? `<button type="button" class="orbit-btn ghost" data-iaction="handoff" title="${esc(tr('释放弹窗拦截并在原终端显示确认框'))}" ${off ? 'disabled' : ''}>${esc(tr('回原终端'))}</button>` : ''}</div>`
      : `${readonlyNote(p, session, parts.canFocus)}<div class="orbit-iactions">${parts.approvalHook}<span class="orbit-ispacer"></span>${interruptButton(p)}${parts.focus}</div>`;
    return `<div class="orbit-iapproval" data-ikind="approval" data-approval="${esc(a.kind || 'generic')}">
      <div class="orbit-itool-row"><span class="orbit-itool-tag">${esc(a.toolName || 'Permission')}</span>${a.title ? `<strong class="orbit-ititle">${esc(a.title)}</strong>` : ''}<span class="orbit-ireason" title="${esc(a.reason || '')}">${esc(a.reason || '')}</span></div>
      ${a.command ? `<pre class="orbit-code">${esc(a.command)}</pre>` : ''}${cwd}${plan}${changesHTML(a, draft)}${paths}
      ${a.details ? `<pre class="orbit-code orbit-code-muted">${esc(a.details)}</pre>` : ''}${reasonRow}
      ${actions}
    </div>`;
  }

  function questionBlock(p, q, draft, direct) {
    const off = locked();
    const opts = options(q).map((o) => {
      const selected = q.multiSelect ? list(draft.multi[q.id]).includes(o.label) : draft.single[q.id] === o.label;
      const pKey = `${q.id}:${o.label}`;
      const expanded = draft.previews.has(pKey);
      const previewHtml = o.preview
        ? `<button type="button" class="orbit-preview-toggle" data-ipreview="${esc(pKey)}" aria-expanded="${expanded}">${esc(expanded ? tr('收起预览') : tr('查看预览'))}</button>${expanded ? `<pre class="orbit-code">${esc(o.preview)}</pre>` : ''}`
        : '';
      return `<div class="orbit-opt-wrap">
        <button type="button" class="orbit-opt" role="${q.multiSelect ? 'checkbox' : 'radio'}" aria-checked="${selected}" data-selected="${selected}" data-iqid="${esc(q.id)}" data-iolabel="${esc(o.label)}" data-idirect="${direct}" ${off || !p.canRespond ? 'disabled' : ''}>
          <span class="orbit-opt-mark" aria-hidden="true">${optionMark(q.multiSelect, selected)}</span>
          <span class="orbit-opt-text"><strong>${esc(o.label)}</strong>${o.description ? `<small>${esc(o.description)}</small>` : ''}</span>
        </button>
        ${previewHtml}
      </div>`;
    }).join('');
    const hasOptions = options(q).length > 0;
    const limits = q.constraints || {};
    const range = numeric(q) && typeof limits.minimum === 'number' && typeof limits.maximum === 'number'
      ? `<small class="orbit-qhelp">${esc(tr('允许范围：{0} – {1}', limits.minimum, limits.maximum))}</small>`
      : '';
    const placeholder = q.placeholder || (hasOptions ? tr('其他自定义回答…') : q.secret ? tr('输入内容已隐藏') : tr('输入回答…'));
    const error = questionError(q, draft);
    // A secret is masked, kept out of autofill and never written back into the markup.
    const customRow = p.canRespond && customAllowed(q)
      ? `<div class="orbit-icustom-row">
          <input type="${q.secret ? 'password' : 'text'}" class="orbit-input" data-icustom="${esc(q.id)}" aria-label="${esc(q.header || q.question || '')}" ${numeric(q) ? `inputmode="${q.kind === 'integer' ? 'numeric' : 'decimal'}"` : ''} ${q.secret ? 'autocomplete="off" spellcheck="false" data-isecret="true"' : ''} ${Number.isInteger(limits.maxLength) && limits.maxLength > 0 ? `maxlength="${limits.maxLength}"` : ''} placeholder="${esc(placeholder)}" value="${q.secret ? '' : esc(draft.custom[q.id] || '')}" ${isResolved(p) ? 'disabled' : ''}>
          ${direct ? `<button type="button" class="orbit-btn primary" data-iaction="submit-questions" ${off || !String(draft.custom[q.id] || '').trim() ? 'disabled' : ''}>${esc(tr('发送'))}</button>` : ''}
        </div>${range}<small class="orbit-qerror" role="alert" data-ierror="${esc(q.id)}" ${error ? '' : 'hidden'}>${esc(error)}</small>`
      : '';
    const hints = [q.multiSelect ? tr('（可多选）') : '', q.required === false ? tr('（可选）') : ''].filter(Boolean)
      .map((text) => `<span class="orbit-qhint">${esc(text)}</span>`).join('');
    return `<div class="orbit-qblock" role="group" aria-label="${esc(q.header || q.question || '')}">
      ${q.header && q.header !== q.question ? `<span class="orbit-qheader">${esc(q.header)}</span>` : ''}
      <div class="orbit-qtitle">${esc(q.question)}${hints}</div>
      ${q.description ? `<div class="orbit-qdesc">${esc(q.description)}</div>` : ''}
      ${hasOptions ? `<div class="orbit-opts" role="${q.multiSelect ? 'group' : 'radiogroup'}">${opts}</div>` : ''}
      ${customRow}
    </div>`;
  }

  function questionsHTML(p, session, parts) {
    const draft = getDraft(p);
    const off = locked();
    const direct = singleDirect(p);
    const blocks = p.questions.map((q) => questionBlock(p, q, draft, direct)).join('');
    const formActions = p.canRespond && p.form
      ? `${p.form.canSkip ? `<button type="button" class="orbit-btn" data-iaction="skip-form" title="${esc(tr('告诉 Agent 不提供这些信息'))}" ${off ? 'disabled' : ''}>${esc(tr('拒绝填写'))}</button>` : ''}${p.form.canCancel ? `<button type="button" class="orbit-btn ghost" data-iaction="cancel-form" title="${esc(tr('关闭这个表单，不提交任何内容'))}" ${off ? 'disabled' : ''}>${esc(tr('取消表单'))}</button>` : ''}`
      : '';
    const interrupt = interruptButton(p);
    let footer = '';
    if (!p.canRespond) {
      footer = `${readonlyNote(p, session, parts.canFocus)}<div class="orbit-iactions">${parts.questionHook}<span class="orbit-ispacer"></span>${interrupt}${parts.focus}</div>`;
    } else if (!direct || p.canHandoff || formActions || interrupt) {
      footer = `<div class="orbit-iactions">
        ${!direct ? `<button type="button" class="orbit-btn primary" data-iaction="submit-questions" ${off || !allQuestionsAnswered(p, draft) ? 'disabled' : ''}>${esc(submitting ? tr('提交中…') : tr('提交回答'))}</button>` : ''}
        ${formActions}<span class="orbit-ispacer"></span>${interrupt}
        ${p.canHandoff ? `<button type="button" class="orbit-btn ghost" data-iaction="handoff" ${off ? 'disabled' : ''}>${esc(tr('回原终端'))}</button>` : ''}
      </div>`;
    }
    return `<div class="orbit-iquestions" data-ikind="questions">${formHTML(p)}${blocks}${footer}</div>`;
  }

  function textHTML(p) {
    const draft = getDraft(p);
    const off = locked();
    const follow = p.kind === 'followup';
    const via = p.adapter === 'tmux' ? tr('通过 tmux 发送') : tr('通过原终端发送');
    const note = !follow
      ? via
      : p.followUp.mode === 'queue' ? tr('会话仍在执行，指令将排队到本轮结束后执行')
        : p.followUp.mode === 'newTurn' ? tr('将在原会话开启下一轮') : via;
    const missing = !p.textPrompt && !follow ? `<div class="orbit-ireadonly-reason">${esc(tr('原会话正在等待输入，未读取到提示内容'))}</div>` : '';
    const label = follow ? tr('后续指令') : tr('回复内容');
    return `<div class="orbit-itext" data-ikind="${follow ? 'followup' : 'text'}">
      ${promptHTML(p)}${missing}
      <textarea class="orbit-textarea" rows="3" data-ifield="text-reply" aria-label="${esc(label)}" placeholder="${esc(follow ? tr('输入后续指令…') : tr('输入回复内容…'))}" ${isResolved(p) ? 'disabled' : ''}>${esc(draft.text)}</textarea>
      <div class="orbit-iactions">
        <span class="orbit-iadapter">${esc(note)}</span>
        <span class="orbit-ispacer"></span>${interruptButton(p)}
        <button type="button" class="orbit-btn primary" data-iaction="send-text" title="${esc(tr('按 Ctrl+Enter 发送'))}" ${off || !draft.text.trim() ? 'disabled' : ''}>${esc(submitting ? tr('发送中…') : tr('发送'))}</button>
      </div>
    </div>`;
  }

  function cardHTML() {
    const { session } = ctx;
    const canFocus = !!window.AieyesSessions?.canFocus?.(session);
    const follow = !waitingPhase(session);
    const tool = (action, label, mark, disabled = false) => `<button type="button" class="orbit-icon-btn" data-iaction="${action}" title="${esc(label)}" aria-label="${esc(label)}" ${disabled ? 'disabled' : ''}>${mark}</button>`;
    const header = `<div class="orbit-icard-head">
      <div class="orbit-icard-meta">
        <strong>${esc(sessionTitle(session))}</strong>
        <small>${esc(formatSessionMeta(session))}</small>
      </div>
      <div class="orbit-icard-tools">
        ${tool('refresh', tr('刷新待处理内容'), '↻', (isLoading() && !currentPayload) || submitting)}
        ${canFocus ? tool('focus', window.AieyesSessions.focusHelp(session), '↗') : ''}
        ${follow ? tool('close-follow', tr('收起后续指令'), '✕') : ''}
      </div>
    </div>`;
    const parts = {
      canFocus,
      focus: canFocus ? `<button type="button" class="orbit-btn primary" data-iaction="focus">${esc(tr('跳转到原窗口'))}</button>` : '',
      approvalHook: '',
      questionHook: '',
    };

    let body = '';
    if (!currentPayload) {
      body = isLoading() ? `<div class="orbit-iloading">${esc(tr('正在读取待处理内容…'))}</div>` : '';
    } else {
      const p = currentPayload;
      if (needsApprovalHook(p, session)) parts.approvalHook = `<button type="button" class="orbit-btn" data-iaction="enable-claude-approvals" ${submitting ? 'disabled' : ''}>${esc(tr('启用弹窗审批'))}</button>`;
      if (needsQuestionHook(p)) parts.questionHook = `<button type="button" class="orbit-btn" data-iaction="enable-claude-hook" ${submitting ? 'disabled' : ''}>${esc(tr('启用弹窗回答'))}</button>`;
      if (p.kind === 'approval' && p.approval) {
        body = approvalHTML(p, session, parts);
      } else if (p.kind === 'questions' && p.questions.length) {
        body = questionsHTML(p, session, parts);
      } else if ((p.kind === 'text' || p.kind === 'followup') && p.canRespond) {
        body = textHTML(p);
      } else if (p.kind === 'none') {
        const interrupt = interruptButton(p);
        const reason = p.answered || !follow
          ? (p.readonlyReason || tr(p.answered ? '已提交，等待原会话继续' : '当前没有需要处理的请求'))
          : (p.followUp.reason || p.readonlyReason || tr('此会话现在不能接收后续指令'));
        body = `<div class="orbit-ireadonly" data-ifollowing="${follow}">${follow ? promptHTML(p) : ''}<div class="orbit-ireadonly-reason" data-ikind="none">${esc(reason)}</div>${interrupt || (follow && parts.focus) ? `<div class="orbit-iactions"><span class="orbit-ispacer"></span>${interrupt}${follow ? parts.focus : ''}</div>` : ''}</div>`;
      } else {
        body = `<div class="orbit-ireadonly" data-ikind="readonly">${formHTML(p)}${promptHTML(p)}${readonlyNote(p, session, canFocus)}<div class="orbit-iactions">${parts.questionHook}${parts.approvalHook}<span class="orbit-ispacer"></span>${interruptButton(p)}${parts.focus}</div></div>`;
      }
      body = pendingHTML(p) + body;
    }
    return `<div class="orbit-icard" role="group" aria-label="${esc(sessionTitle(session))}">${header}${body}${renderFeedback()}</div>`;
  }

  /* Focus, caret and inner scroll positions survive a rebuild of the card. */
  const FOCUS_KEYS = ['ifield', 'icustom', 'iolabel', 'iqid', 'ipreview', 'iaction', 'idecision', 'itarget', 'istep', 'isummary'];
  const SCROLLERS = '.orbit-code,.orbit-iprompt,.orbit-plan';
  let parkedFocus = null; // focus owed to a control that was disabled while its request was in flight
  function captureView(container) {
    const el = document.activeElement;
    const view = { focus: !el || el === document.body ? parkedFocus : null, scroll: [...container.querySelectorAll(SCROLLERS)].map((node) => node.scrollTop) };
    if (el && el !== document.body && container.contains(el)) {
      const keys = FOCUS_KEYS.filter((key) => el.dataset[key] !== undefined);
      if (keys.length) {
        view.focus = { tag: el.tagName, keys: keys.map((key) => [key, el.dataset[key]]), index: [...container.querySelectorAll(el.tagName)].filter((node) => keys.every((key) => node.dataset[key] === el.dataset[key])).indexOf(el) };
        if (typeof el.selectionStart === 'number') {
          view.focus.selection = [el.selectionStart, el.selectionEnd, el.selectionDirection || 'none'];
        }
      }
    }
    return view;
  }
  function restoreView(container, view) {
    const blocks = container.querySelectorAll(SCROLLERS);
    if (blocks.length === view.scroll.length) blocks.forEach((node, index) => { node.scrollTop = view.scroll[index]; });
    if (!view.focus) {
      parkedFocus = null;
      return;
    }
    const matches = [...container.querySelectorAll(view.focus.tag)].filter((node) => view.focus.keys.every(([key, value]) => node.dataset[key] === value));
    let el = matches[view.focus.index] || matches[0];
    const stepping = view.focus.keys.some(([key]) => key === 'istep');
    const busy = submitting || !!loadingSessionId;
    // The end of the request list disables the step button just used: focus moves to the other one.
    if (stepping && !busy && (!el || el.disabled)) el = container.querySelector('[data-istep]:not([disabled])') || el;
    // A control that is disabled, or gone while the card reloads, gets its focus and caret back
    // from the render that follows.
    parkedFocus = el?.disabled || (!el && busy) ? view.focus : null;
    if (!el || el.disabled) return;
    el.focus({ preventScroll: true });
    if (view.focus.selection && typeof el.setSelectionRange === 'function') {
      try { el.setSelectionRange(...view.focus.selection); } catch { /* not a text control */ }
    }
  }
  function notifyLayout() {
    window.dispatchEvent(new CustomEvent('aieyes:orbit-layout'));
  }

  function renderCard() {
    if (!ctx) return;
    if (composing) {
      renderDeferred = true;
      return;
    }
    renderDeferred = false;
    const { container } = ctx;
    const view = captureView(container);
    container.innerHTML = `${tabsHTML()}${cardHTML()}`;
    // Secrets are set as properties: the typed value never appears in the markup.
    if (currentPayload) {
      const draft = getDraft(currentPayload);
      for (const input of container.querySelectorAll('[data-isecret]')) input.value = draft.custom[input.dataset.icustom] || '';
    }
    bindCardEvents(container);
    restoreView(container, view);
    reportDraft();
    notifyLayout();
  }

  /* A change in the session list touches only the tab strip, so a field being typed in is left alone. */
  function renderTabs() {
    if (!ctx) return;
    const { container } = ctx;
    const view = captureView(container);
    const old = container.querySelector(':scope > .orbit-itabs');
    const html = tabsHTML();
    if (!html) {
      old?.remove();
    } else {
      const holder = document.createElement('template');
      holder.innerHTML = html;
      const next = holder.content.firstElementChild;
      if (old) old.replaceWith(next);
      else container.prepend(next);
      bindTabs(next);
    }
    restoreView(container, view);
    notifyLayout();
  }

  function disarmInterrupt() {
    clearTimeout(armTimer);
    armTimer = 0;
    interruptArmed = false;
  }
  function resetCardState() {
    currentPayload = null;
    feedback = null;
    reasonFor = null;
    disarmInterrupt();
  }
  function rerun() {
    if (ctx) update(ctx.info, null, ctx.active);
  }

  function switchTo(sessionId) {
    if (!ctx) return;
    activeSessionId = sessionId;
    renderedCardSig = null;
    resetCardState();
    rerun();
  }

  function bindTabs(root) {
    for (const tab of root.querySelectorAll('[data-itarget]')) {
      tab.addEventListener('click', () => {
        if (tab.dataset.itarget !== activeSessionId) switchTo(tab.dataset.itarget);
      });
    }
  }

  function bindContainer(container) {
    if (container.dataset.ibound) return;
    container.dataset.ibound = 'true';
    container.addEventListener('compositionstart', () => { composing = true; });
    const settle = () => {
      if (!composing) return;
      composing = false;
      // WebKit sends the confirming keydown after compositionend; let it pass before rebuilding.
      if (renderDeferred) setTimeout(() => { if (renderDeferred && !composing) renderCard(); }, 0);
    };
    container.addEventListener('compositionend', settle);
    container.addEventListener('focusout', settle);
  }

  function beginSubmit() {
    submitting = true;
    submitToken += 1;
    return submitToken;
  }
  function endSubmit(token) {
    if (token === submitToken) submitting = false;
  }

  function focusSession() {
    if (ctx) void window.AieyesSessions?.focusSession?.(ctx.session.id);
  }

  async function enableClaudeHook(patch, doneMessage) {
    if (!ctx || submitting) return;
    const sessionId = ctx.session.id;
    const token = beginSubmit();
    renderCard();
    let ok = false;
    try {
      statusCache = { ...statusCache, ...(await callCore('sessions.interaction.configure', patch)) };
      statusLoaded = true;
      ok = true;
    } catch (error) {
      if (activeSessionId === sessionId) feedback = { status: 'error', message: String(error) };
    } finally {
      endSubmit(token);
    }
    if (!ctx || activeSessionId !== sessionId) return;
    if (!ok) {
      renderCard();
      return;
    }
    await loadPayload({ force: true });
    // A request raised before the hook was on stays read-only; say that the switch itself worked.
    if (doneMessage && ctx && activeSessionId === sessionId && currentPayload && !currentPayload.canRespond && !feedback) {
      feedback = { status: 'confirmed', message: doneMessage };
      renderCard();
    }
  }

  async function handoff() {
    const payload = currentPayload;
    if (!payload || !ctx || locked()) return;
    const sessionId = ctx.session.id;
    const token = beginSubmit();
    renderCard();
    try {
      const res = await callCore('sessions.interaction.handoff', {
        sessionId: payload.sessionId,
        requestId: payload.requestId,
      });
      if (activeSessionId !== sessionId || currentPayload !== payload) return;
      feedback = { status: res?.handedOff ? 'confirmed' : 'blocked', message: res?.message || '' };
      if (res?.handedOff) {
        resolvedKey = payloadKey(payload);
        reasonFor = null;
        focusSession();
        afterResolved(sessionId, payload);
      }
    } catch (error) {
      if (activeSessionId === sessionId) feedback = { status: 'error', message: String(error) };
    } finally {
      endSubmit(token);
      renderCard();
    }
  }

  function submitAnswers() {
    if (!currentPayload || locked()) return;
    const draft = getDraft(currentPayload);
    if (!allQuestionsAnswered(currentPayload, draft)) return;
    void submitResponse({ action: 'answer', answers: buildAnswers(currentPayload, draft) });
  }

  /* A decision is sent by the id the card listed; a note goes along only where the decision takes one. */
  function submitDecision(id) {
    const payload = currentPayload;
    if (!payload || locked()) return;
    const decision = list(payload.approval?.decisions).find((d) => d.id === id);
    if (!decision) return;
    const reason = decision.acceptsReason ? getDraft(payload).reason.trim() : '';
    void submitResponse({ action: 'decide', decisionId: decision.id, ...(reason ? { reason } : {}) });
  }

  function interrupt() {
    if (!currentPayload?.followUp.canInterrupt || submitting) return;
    if (!interruptArmed) {
      interruptArmed = true;
      clearTimeout(armTimer);
      armTimer = setTimeout(() => {
        if (!interruptArmed) return;
        disarmInterrupt();
        renderCard();
      }, 4000);
      renderCard();
      return;
    }
    disarmInterrupt();
    void submitResponse({ action: 'interrupt' });
  }

  function refreshSubmitState(container) {
    if (!currentPayload) return;
    const draft = getDraft(currentPayload);
    draft.touched = true;
    const submitBtn = container.querySelector('[data-iaction=submit-questions]');
    if (submitBtn) {
      const ready = singleDirect(currentPayload)
        ? !!String(draft.custom[currentPayload.questions[0].id] || '').trim()
        : allQuestionsAnswered(currentPayload, draft);
      submitBtn.disabled = locked() || !ready;
    }
    for (const node of container.querySelectorAll('[data-ierror]')) {
      const q = currentPayload.questions.find((item) => item.id === node.dataset.ierror);
      const error = q ? questionError(q, draft) : '';
      if (node.textContent === error) continue;
      node.textContent = error;
      node.hidden = !error;
      notifyLayout();
    }
    reportDraft();
  }
  function markOption(node, multi, selected) {
    node.dataset.selected = String(selected);
    node.setAttribute('aria-checked', String(selected));
    node.querySelector('.orbit-opt-mark').textContent = optionMark(multi, selected);
  }

  function bindCardEvents(container) {
    bindTabs(container);
    const on = (selector, handler) => container.querySelector(selector)?.addEventListener('click', handler);
    on('[data-iaction=refresh]', () => {
      // An explicit refresh re-reads the request and unlocks the card; the core still verifies
      // the state key and prompt hash of whatever is sent next.
      resolvedKey = null;
      feedback = null;
      void loadPayload({ force: true });
    });
    for (const btn of container.querySelectorAll('[data-iaction=focus]')) {
      btn.addEventListener('click', focusSession);
    }
    on('[data-iaction=close-follow]', () => closeFollowUp());
    on('[data-iaction=enable-claude-hook]', () => {
      void enableClaudeHook({ claudeQuestions: true });
    });
    on('[data-iaction=enable-claude-approvals]', () => {
      void enableClaudeHook({ claudeQuestions: true, claudeApprovals: true }, tr('已启用弹窗审批，之后的工具授权会显示在这里'));
    });
    on('[data-iaction=handoff]', () => { void handoff(); });
    on('[data-iaction=interrupt]', interrupt);
    on('[data-iaction=skip-form]', () => { if (currentPayload?.form?.canSkip) void submitResponse({ action: 'skip' }); });
    on('[data-iaction=cancel-form]', () => { if (currentPayload?.form?.canCancel) void submitResponse({ action: 'cancel' }); });
    for (const btn of container.querySelectorAll('[data-istep]')) {
      btn.addEventListener('click', () => {
        const payload = currentPayload;
        if (!payload || submitting) return;
        const requests = payload.pendingRequests;
        const next = requests[Math.max(0, requests.findIndex((item) => item.requestId === payload.requestId)) + Number(btn.dataset.istep)];
        if (!next) return;
        pickedRequest.set(payload.sessionId, next.requestId);
        feedback = null;
        reasonFor = null;
        void loadPayload({ force: true });
      });
    }
    for (const node of container.querySelectorAll('[data-idetails]')) {
      node.addEventListener('toggle', () => {
        if (currentPayload) getDraft(currentPayload).open.set(node.dataset.idetails, node.open);
        notifyLayout();
      });
    }

    for (const btn of container.querySelectorAll('[data-idecision]')) {
      btn.addEventListener('click', () => {
        if (!currentPayload || locked()) return;
        const id = btn.dataset.idecision;
        const decision = list(currentPayload.approval?.decisions).find((d) => d.id === id);
        if (!decision) return;
        // The first press on a decision that takes a note opens the note; the second sends it.
        if (decision.acceptsReason && reasonFor !== id) {
          reasonFor = id;
          renderCard();
          container.querySelector('[data-ifield=decline-reason]')?.focus();
          return;
        }
        submitDecision(id);
      });
    }
    on('[data-iaction=confirm-decline]', () => { if (reasonFor) submitDecision(reasonFor); });
    const reasonInput = container.querySelector('[data-ifield=decline-reason]');
    if (reasonInput) {
      reasonInput.addEventListener('input', (e) => {
        if (!currentPayload) return;
        const draft = getDraft(currentPayload);
        draft.reason = e.target.value;
        draft.touched = true;
        reportDraft();
      });
      reasonInput.addEventListener('keydown', (e) => {
        if (imeEvent(e)) return;
        if (e.key === 'Enter') {
          e.preventDefault();
          if (reasonFor) submitDecision(reasonFor);
        } else if (e.key === 'Escape') {
          // Escape backs out of the note row; a second Escape closes the popover.
          e.preventDefault();
          const id = reasonFor;
          reasonFor = null;
          renderCard();
          [...container.querySelectorAll('[data-idecision]')].find((node) => node.dataset.idecision === id)?.focus({ preventScroll: true });
        }
      });
    }

    for (const opt of container.querySelectorAll('.orbit-opt')) {
      opt.addEventListener('click', () => {
        if (!currentPayload || locked()) return;
        const qid = opt.dataset.iqid;
        const label = opt.dataset.iolabel;
        const q = currentPayload.questions.find((item) => item.id === qid);
        if (!q) return;
        const draft = getDraft(currentPayload);
        const siblings = [...container.querySelectorAll('.orbit-opt')].filter((other) => other.dataset.iqid === qid);
        // Selections are patched in place so the pressed option keeps keyboard focus.
        if (q.multiSelect) {
          const picked = [...list(draft.multi[qid])];
          const idx = picked.indexOf(label);
          if (idx >= 0) picked.splice(idx, 1);
          else picked.push(label);
          draft.multi[qid] = picked;
          markOption(opt, true, idx < 0);
          refreshSubmitState(container);
          return;
        }
        // An optional single choice can be cleared by pressing it again.
        const clear = q.required === false && draft.single[qid] === label;
        if (clear) delete draft.single[qid];
        else draft.single[qid] = label;
        draft.custom[qid] = '';
        if (opt.dataset.idirect === 'true') {
          void submitResponse({ action: 'answer', answers: buildAnswers(currentPayload, draft) });
          return;
        }
        for (const other of siblings) markOption(other, false, !clear && other === opt);
        for (const input of container.querySelectorAll('[data-icustom]')) {
          if (input.dataset.icustom === qid) input.value = '';
        }
        refreshSubmitState(container);
      });
      // Arrow keys move between the choices of one question, as in a native radio group.
      opt.addEventListener('keydown', (e) => {
        const dir = e.key === 'ArrowDown' || e.key === 'ArrowRight' ? 1 : e.key === 'ArrowUp' || e.key === 'ArrowLeft' ? -1 : 0;
        if (!dir) return;
        const siblings = [...container.querySelectorAll('.orbit-opt')].filter((other) => other.dataset.iqid === opt.dataset.iqid && !other.disabled);
        const next = siblings[(siblings.indexOf(opt) + dir + siblings.length) % siblings.length];
        if (!next) return;
        e.preventDefault();
        next.focus({ preventScroll: false });
      });
    }
    for (const toggle of container.querySelectorAll('[data-ipreview]')) {
      toggle.addEventListener('click', () => {
        if (!currentPayload) return;
        const previews = getDraft(currentPayload).previews;
        const key = toggle.dataset.ipreview;
        if (previews.has(key)) previews.delete(key);
        else previews.add(key);
        renderCard();
      });
    }
    for (const input of container.querySelectorAll('[data-icustom]')) {
      input.addEventListener('input', (e) => {
        if (!currentPayload) return;
        const qid = input.dataset.icustom;
        const draft = getDraft(currentPayload);
        draft.custom[qid] = e.target.value;
        const q = currentPayload.questions.find((item) => item.id === qid);
        if (q && !q.multiSelect && e.target.value.trim() && draft.single[qid]) {
          delete draft.single[qid];
          for (const other of container.querySelectorAll('.orbit-opt')) {
            if (other.dataset.iqid === qid) markOption(other, false, false);
          }
        }
        refreshSubmitState(container);
      });
      input.addEventListener('keydown', (e) => {
        if (e.key !== 'Enter' || imeEvent(e) || !currentPayload) return;
        // Enter sends a one-question form only; longer forms need the explicit submit button.
        if (currentPayload.questions.length !== 1) return;
        e.preventDefault();
        submitAnswers();
      });
    }
    on('[data-iaction=submit-questions]', submitAnswers);

    const textarea = container.querySelector('[data-ifield=text-reply]');
    const sendText = () => {
      if (!currentPayload || locked()) return;
      const body = (getDraft(currentPayload).text || '').trim();
      if (body) void submitResponse({ action: currentPayload.kind === 'followup' ? 'followup' : 'answer', text: body });
    };
    if (textarea) {
      textarea.addEventListener('input', () => {
        if (!currentPayload) return;
        const draft = getDraft(currentPayload);
        draft.text = textarea.value;
        draft.touched = true;
        const btn = container.querySelector('[data-iaction=send-text]');
        if (btn) btn.disabled = locked() || !textarea.value.trim();
        reportDraft();
      });
      textarea.addEventListener('keydown', (e) => {
        if (imeEvent(e)) return;
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
          e.preventDefault();
          sendText();
        }
      });
    }
    on('[data-iaction=send-text]', sendText);
  }

  async function submitResponse(extra) {
    const payload = currentPayload;
    if (!payload || !ctx) return;
    // A follow-up and an interrupt act on the session; everything else answers one request.
    const sessionAction = extra.action === 'followup' || extra.action === 'interrupt';
    // The lock of an answered request holds its own answers back, not a session-level action: a
    // turn can still be stopped after an unconfirmed answer. Callers gate the follow-up text.
    if (sessionAction ? submitting : locked()) return;
    const sessionId = ctx.session.id;
    const token = beginSubmit();
    const closing = sessionAction ? null : { sessionId, settled: false };
    if (closing) autoClose = closing;
    renderCard();
    let reinspect = false;
    try {
      const res = await callCore('sessions.interaction.respond', {
        sessionId: payload.sessionId,
        stateKey: payload.stateKey,
        // Stopping a turn is not an answer: it carries no request identity.
        ...(extra.action === 'interrupt' ? {} : { promptHash: payload.promptHash, ...(payload.requestId ? { requestId: payload.requestId } : {}) }),
        capabilities: CAPABILITIES,
        ...extra,
      });
      // The card moved on while the core was answering; its feedback belongs to the old request.
      if (activeSessionId !== sessionId || currentPayload !== payload) {
        // The session resumed and took its card away before the result arrived.
        if (closing && autoClose === closing) {
          autoClose = null;
          if (!ctx && (res?.status === 'confirmed' || res?.status === 'submitted')) closeAutoPopover(lastInfo);
        }
        return;
      }
      const status = res?.status || 'blocked';
      if (status === 'needsLocalTerminal') {
        // Only the native macOS app can type into a local terminal; nothing was sent.
        feedback = { status: 'blocked', message: tr('此平台无法自动向原终端输入，请跳转到原窗口处理') };
        focusSession();
      } else {
        // An unconfirmed delivery must be checked in the original window before any retry.
        feedback = { status, message: [res?.message, status === 'uncertain' ? tr('结果未确认，请先核对原窗口；确认未送达后可刷新重试') : ''].filter(Boolean).join(' ') };
      }
      if (status === 'confirmed' || status === 'submitted') {
        reasonFor = null;
        if (extra.action !== 'interrupt') clearDraft(payload);
        if (sessionAction) {
          // The session moves on by itself; show what it accepts next.
          reinspect = true;
        } else {
          resolvedKey = payloadKey(payload);
          afterResolved(sessionId, payload);
        }
      } else if (status === 'uncertain') {
        // It may have been delivered: sending again could answer twice, so lock without clearing.
        reasonFor = null;
        resolvedKey = payloadKey(payload);
      } else if (status !== 'needsLocalTerminal') {
        reinspect = true;
      }
    } catch (error) {
      if (activeSessionId === sessionId) feedback = { status: 'error', message: String(error) };
    } finally {
      endSubmit(token);
      // Only a settled answer closes the popover; `afterResolved` has replaced the marker then.
      if (closing && autoClose === closing) autoClose = null;
      // A blocked answer means the request changed underneath the card: show the current one.
      if (reinspect && ctx && activeSessionId === sessionId) void loadPayload({ force: true, keepFeedback: true });
      else renderCard();
    }
  }

  /* After a request is settled: the next request of the same session, else the next session,
     else an auto-opened popover closes. */
  function closeAutoPopover(info) {
    if (info?.orbitAuto) window.__TAURI__?.core?.invoke('desktop_action', { action: 'close-panel' })?.catch?.(() => {});
  }
  function afterResolved(sessionId, payload) {
    clearTimeout(advanceTimer);
    autoClose = { sessionId, settled: true };
    const more = payload.pendingCount > 1 || payload.pendingRequests.length > 1;
    advanceTimer = setTimeout(() => {
      advanceTimer = 0;
      if (!ctx || activeSessionId !== sessionId) return;
      if (more) {
        pickedRequest.delete(sessionId);
        void loadPayload({ force: true, keepFeedback: 'same' });
        return;
      }
      const next = ctx.waiting.find((s) => s.id !== sessionId);
      if (next) {
        switchTo(next.id);
      } else {
        autoClose = null;
        closeAutoPopover(ctx.info);
      }
    }, 700);
  }

  /* `silent` re-reads behind the visible card and only redraws when the request changed. */
  async function loadPayload({ force = false, keepFeedback = false, silent = false } = {}) {
    if (!ctx) return;
    const session = ctx.session;
    if (loadingSessionId === session.id && !force) return;
    const seq = ++loadSeq;
    loadingSessionId = session.id;
    lastInspectAt = Date.now();
    const previous = currentPayload;
    if (!silent) {
      if (force) currentPayload = null;
      disarmInterrupt();
      renderCard();
    }
    let result = null;
    let failure = null;
    try {
      const picked = pickedRequest.get(session.id);
      result = normalize(await callCore('sessions.interaction.inspect', {
        sessionId: session.id,
        capabilities: CAPABILITIES,
        ...(picked ? { requestId: picked } : {}),
      }), session.id);
    } catch (error) {
      failure = error;
    }
    // A newer request owns the loading state and the card.
    if (seq !== loadSeq) return;
    loadingSessionId = null;
    lastInspectAt = Date.now();
    if (!ctx || activeSessionId !== session.id) return;
    if (failure) {
      if (silent) return;
      feedback = { status: 'error', message: String(failure) };
    } else if (silent && (submitting || samePayload(currentPayload, result))) {
      return;
    } else {
      const same = sameRequest(previous, result);
      if (!same) {
        reasonFor = null;
        disarmInterrupt();
      }
      // The core shows the oldest request when the picked one is gone.
      if (pickedRequest.has(session.id) && pickedRequest.get(session.id) !== result.requestId) pickedRequest.delete(session.id);
      currentPayload = result;
      // Another request the user has not answered keeps an auto-opened popover open.
      if (autoClose?.settled && result.canRespond && !isResolved(result)) autoClose = null;
      // `keepFeedback: 'same'` keeps a result on screen only while its own request is still shown.
      if ((keepFeedback === 'same' ? !same : !keepFeedback) && currentPayload.kind !== 'none') feedback = null;
      if (!waitingPhase(session)) noteFollowUp(session, result);
    }
    renderCard();
    if (focusOnLoad && !silent && ctx) {
      focusOnLoad = false;
      const box = ctx.container;
      (box.querySelector('[data-ifield=text-reply]') || box.querySelector('[data-iaction=interrupt]') || box.querySelector('[data-iaction=refresh]'))?.focus({ preventScroll: true });
    }
  }

  /* The session snapshot does not change when a hook request arrives late, expires, is handed back
     or is answered elsewhere, so a card that cannot act (or one that can expire) is re-read on a
     timer. A follow-up card is re-read too: what a running session accepts changes as it runs. */
  function pollTick() {
    if (!ctx || ctx.container.hidden) {
      stopPoll();
      return;
    }
    const payload = currentPayload;
    if (!payload || document.hidden || submitting || composing || isLoading() || interruptArmed) return;
    const stale = (payload.kind === 'none' && waitingPhase(ctx.session)) || isResolved(payload);
    const idle = !payload.canRespond;
    const expiring = !!payload.canRespond && (payload.adapter === 'claudeHook' || payload.kind === 'followup');
    if (!stale && !idle && !expiring) return;
    const remote = !!ctx.session.machineId && ctx.session.machineId !== 'local';
    const every = (stale ? POLL_STALE_MS : POLL_LIVE_MS) * (remote ? 3 : 1);
    if (Date.now() - lastInspectAt < every) return;
    // Text being typed is not pulled away; the core rejects it if the request has changed.
    const focused = document.activeElement;
    if (focused && ctx.container.contains(focused) && typeof focused.value === 'string' && focused.matches('input,textarea') && focused.value.trim()) return;
    void loadPayload({ force: true, silent: true });
  }
  function startPoll() {
    if (!pollTimer) pollTimer = setInterval(pollTick, 500);
  }
  function stopPoll() {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = 0;
  }

  /* Running sessions: the list offers a follow-up button only where the core says the session
     takes an instruction or can be stopped. Remote sessions are not probed; each probe is one
     inspect, cached per turn, and probes run one after another. */
  const probeKey = (session) => `${session.turnId || ''}|${session.phase === 'needsInput'}`;
  const followCapable = (payload) => !!payload.followUp.available || !!payload.followUp.canInterrupt;
  function noteFollowUp(session, payload) {
    const before = followProbes.get(session.id)?.can;
    followProbes.set(session.id, { key: probeKey(session), at: Date.now(), can: followCapable(payload) });
    if (before !== followCapable(payload)) window.dispatchEvent(new CustomEvent('aieyes:orbit-followup'));
  }
  async function probeFollowUps(active) {
    if (probing || !statusLoaded || !statusCache.enabled) return;
    const due = active.filter((s) => !waitingPhase(s) && s.machineId === 'local').slice(0, PROBE_LIMIT).find((s) => {
      const cached = followProbes.get(s.id);
      return !cached || cached.key !== probeKey(s) || Date.now() - cached.at > PROBE_TTL_MS;
    });
    if (!due) return;
    probing = true;
    try {
      noteFollowUp(due, normalize(await callCore('sessions.interaction.inspect', { sessionId: due.id, capabilities: CAPABILITIES }), due.id));
    } catch {
      followProbes.set(due.id, { key: probeKey(due), at: Date.now(), can: false });
    } finally {
      probing = false;
    }
    // One at a time: the next session without a fresh answer is read once this one is back.
    if (ctx || document.querySelector('#orbit-interaction')) void probeFollowUps(active);
  }
  /* Markup of the follow-up button for a running session row, or '' when it has none. */
  function followUpHTML(session) {
    if (!statusCache.enabled || waitingPhase(session) || !followProbes.get(session.id)?.can) return '';
    const label = tr('发送后续指令或中断本轮');
    return `<button type="button" class="orbit-btn orbit-follow-btn" data-ifollow="${esc(session.id)}" aria-pressed="${followTarget === session.id}" title="${esc(label)}" aria-label="${esc(label)}">${esc(tr('指令'))}</button>`;
  }
  function openFollowUp(sessionId) {
    if (followTarget === sessionId && activeSessionId === sessionId) {
      closeFollowUp();
      return;
    }
    followTarget = sessionId;
    activeSessionId = sessionId;
    renderedCardSig = null;
    resetCardState();
    focusOnLoad = true;
    window.dispatchEvent(new CustomEvent('aieyes:orbit-followup'));
  }
  function closeFollowUp() {
    const sessionId = followTarget;
    followTarget = null;
    activeSessionId = null;
    renderedCardSig = null;
    resetCardState();
    window.dispatchEvent(new CustomEvent('aieyes:orbit-followup'));
    // Focus returns to the list button the card was opened from.
    [...document.querySelectorAll('[data-ifollow]')].find((node) => node.dataset.ifollow === sessionId)?.focus({ preventScroll: true });
  }
  // The session list is rebuilt on every status update, which can replace the button between press
  // and release: a pointer press is taken on release, and `click` only serves the keyboard.
  const followButton = (event) => (event.target instanceof Element ? event.target.closest('[data-ifollow]') : null);
  document.addEventListener('pointerup', (event) => {
    const btn = event.button === 0 ? followButton(event) : null;
    if (btn) openFollowUp(btn.dataset.ifollow);
  });
  document.addEventListener('click', (event) => {
    const btn = event.detail === 0 ? followButton(event) : null;
    if (btn) openFollowUp(btn.dataset.ifollow);
  });

  function clearCard() {
    activeSessionId = null;
    renderedCardSig = null;
    renderedTabSig = null;
    resetCardState();
    submitting = false;
    submitToken += 1;
    parkedFocus = null;
    composing = false;
    renderDeferred = false;
    loadSeq += 1;
    loadingSessionId = null;
    clearTimeout(advanceTimer);
    advanceTimer = 0;
    stopPoll();
    ctx = null;
  }

  function update(info, _entry, activeSessions) {
    const container = document.querySelector('#orbit-interaction');
    if (!container) return false;
    // Follow-up buttons need the switch state; a failed read is not retried in a loop.
    if (!statusLoaded) void refreshStatus().then(() => { if (statusLoaded) window.dispatchEvent(new CustomEvent('aieyes:orbit-followup')); });
    const active = activeSessions || [];
    const waiting = active.filter(waitingPhase);
    lastInfo = info;
    // The follow-up card stays while its session is listed, also once the turn has ended.
    const followed = followTarget ? list(info?.sessions).find((s) => s.id === followTarget) : null;
    if (followTarget && (!followed || waitingPhase(followed))) followTarget = null;
    pruneDrafts(info);
    void probeFollowUps(active);
    const tabs = followTarget ? [...waiting, followed] : waiting;
    if (!statusCache.enabled || !tabs.length) {
      const shown = !container.hidden;
      container.hidden = true;
      container.innerHTML = '';
      clearCard();
      // The answered session resumed before the advance timer ran: nothing is left to show.
      if (autoClose?.settled) {
        autoClose = null;
        if (!waiting.length) closeAutoPopover(info);
      }
      if (reportedDraft !== false) {
        reportedDraft = null;
        reportDraft();
      }
      if (shown) notifyLayout();
      return false;
    }
    bindContainer(container);
    container.hidden = false;
    let target = tabs.find((s) => s.id === activeSessionId);
    if (!target && info?.orbitPreferredSession) {
      target = waiting.find((s) => s.id === info.orbitPreferredSession);
    }
    if (!target) target = tabs[0];
    if (activeSessionId !== target.id) {
      clearTimeout(advanceTimer);
      advanceTimer = 0;
    }
    activeSessionId = target.id;
    if (autoClose && autoClose.sessionId !== target.id) autoClose = null;
    ctx = { container, session: target, tabs, waiting, active, info };
    const cardSig = waitingPhase(target) ? `${target.id}|${target.inputStateKey || target.turnId || ''}` : `${target.id}|followup`;
    const tabSig = tabs.map((s) => `${s.id}:${sessionTitle(s)}:${waitingPhase(s)}`).join(',');
    startPoll();
    if (cardSig !== renderedCardSig || container.children.length === 0) {
      renderedCardSig = cardSig;
      renderedTabSig = tabSig;
      resetCardState();
      submitting = false;
      submitToken += 1;
      parkedFocus = null;
      void loadPayload({ force: true });
    } else if (tabSig !== renderedTabSig) {
      renderedTabSig = tabSig;
      renderTabs();
    }
    reportDraft();
    return true;
  }

  function reset() {
    clearCard();
    autoClose = null;
    lastInfo = null;
    followTarget = null;
    focusOnLoad = false;
    pickedRequest.clear();
    // The shell forgets the draft hold when the panel closes; report again on the next open.
    reportedDraft = null;
  }

  function settingsHTML() {
    return `<div class="card">
      <h2>${esc(tr('等待输入快捷交互'))}</h2>
      <label class="quota-confirm"><input type="checkbox" role="switch" id="interaction-enabled">${esc(tr('启用桌宠等待输入交互弹窗'))}</label>
      <p class="muted tiny">${esc(tr('点击等待输入的 Agent 徽章可直接审批、选择选项或回复，不依赖飞书渠道。'))}</p>
      <div id="interaction-options">
        <label class="quota-confirm"><input type="checkbox" role="switch" id="interaction-auto-popup">${esc(tr('等待输入时自动浮出弹窗'))}</label>
        <p class="muted tiny">${esc(tr('不抢占当前应用键盘焦点；全屏隐藏桌宠时不弹出，同一请求关闭后不重复弹出。'))}</p>
        <label class="quota-confirm"><input type="checkbox" role="switch" id="interaction-claude-questions">${esc(tr('在弹窗中回答 Claude / Qoder 提问选项'))}</label>
        <label class="quota-confirm"><input type="checkbox" role="switch" id="interaction-claude-approvals">${esc(tr('在弹窗中审批 Claude / Qoder 工具权限'))}</label>
        <p class="muted tiny">${esc(tr('开启后会拦截 Claude / Qoder CLI 工具授权并在弹窗显示「允许本次 / 拒绝 / 回原终端」；应用退出或超时将自动交还终端。'))}</p>
        <label class="quota-confirm"><input type="checkbox" role="switch" id="interaction-allow-terminal">${esc(tr('允许通过终端适配发送文本'))}</label>
      </div>
      <p class="error tiny" id="interaction-error" hidden></p>
    </div>`;
  }

  function bindSettings() {
    const enabledEl = document.querySelector('#interaction-enabled');
    if (!enabledEl) return;
    const autoEl = document.querySelector('#interaction-auto-popup');
    const claudeQEl = document.querySelector('#interaction-claude-questions');
    const claudeAEl = document.querySelector('#interaction-claude-approvals');
    const termEl = document.querySelector('#interaction-allow-terminal');
    const optsWrap = document.querySelector('#interaction-options');
    const errEl = document.querySelector('#interaction-error');

    const apply = (st) => {
      enabledEl.checked = !!st.enabled;
      if (autoEl) autoEl.checked = !!st.autoPopup;
      if (claudeQEl) claudeQEl.checked = !!(st.claudeQuestions || st.claudeApprovals);
      if (claudeAEl) claudeAEl.checked = !!st.claudeApprovals;
      if (termEl) termEl.checked = !!(st.desktopAllowTerminal ?? st.allowTerminal);
      if (optsWrap) optsWrap.hidden = !st.enabled;
    };
    const save = async (patch) => {
      const controls = [enabledEl, autoEl, claudeQEl, claudeAEl, termEl].filter(Boolean);
      controls.forEach((el) => { el.disabled = true; });
      if (errEl) { errEl.hidden = true; errEl.textContent = ''; }
      try {
        const next = await callCore('sessions.interaction.configure', patch);
        statusCache = { ...statusCache, ...next };
        apply(statusCache);
      } catch (error) {
        apply(statusCache);
        if (errEl) { errEl.hidden = false; errEl.textContent = String(error); }
      } finally {
        controls.forEach((el) => { el.disabled = false; });
      }
    };

    apply(statusCache);
    void refreshStatus().then(apply);
    enabledEl.onchange = () => save({ enabled: enabledEl.checked });
    if (autoEl) autoEl.onchange = () => save({ autoPopup: autoEl.checked });
    if (claudeQEl) claudeQEl.onchange = () => save({ claudeQuestions: claudeQEl.checked, claudeApprovals: claudeQEl.checked ? !!claudeAEl?.checked : false });
    if (claudeAEl) claudeAEl.onchange = () => save({ claudeApprovals: claudeAEl.checked });
    if (termEl) termEl.onchange = () => save({ allowTerminal: termEl.checked });
  }

  window.AieyesOrbitInteraction = {
    update,
    reset,
    refreshStatus,
    followUpHTML,
    settingsHTML,
    bindSettings,
  };
})();
