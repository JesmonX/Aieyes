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
  let activeSessionId = null;
  let renderedSignature = null;
  let currentPayload = null;
  let loading = false;
  let submitting = false;
  let feedback = null; // { status, message }
  let showDeclineReason = false;
  let expandedPreviews = new Set();
  const drafts = new Map(); // draftKey -> { single: {}, multi: {}, custom: {}, text: '', declineReason: '' }

  function draftKey(payload) {
    return `${payload.sessionId}|${payload.stateKey}|${payload.promptHash}`;
  }
  function getDraft(payload) {
    const key = draftKey(payload);
    if (!drafts.has(key)) {
      drafts.set(key, { single: {}, multi: {}, custom: {}, text: '', declineReason: '' });
    }
    return drafts.get(key);
  }
  function clearDraft(payload) {
    if (payload) drafts.delete(draftKey(payload));
  }

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

  function allQuestionsAnswered(payload, draft) {
    if (!Array.isArray(payload.questions) || !payload.questions.length) return false;
    for (const q of payload.questions) {
      const custom = (draft.custom[q.id] || '').trim();
      if (q.multiSelect) {
        const picked = draft.multi[q.id] || [];
        if (!picked.length && !custom) return false;
      } else {
        const picked = draft.single[q.id] || '';
        if (!picked && !custom) return false;
      }
    }
    return true;
  }

  function buildAnswers(payload, draft) {
    const out = {};
    for (const q of payload.questions || []) {
      const custom = (draft.custom[q.id] || '').trim();
      if (q.multiSelect) {
        const picked = Array.isArray(draft.multi[q.id]) ? [...draft.multi[q.id]] : [];
        if (custom) picked.push(custom);
        out[q.id] = picked;
      } else if (custom) {
        out[q.id] = custom;
      } else if (draft.single[q.id]) {
        out[q.id] = draft.single[q.id];
      }
    }
    return out;
  }

  function renderFeedback() {
    if (!feedback) return '';
    const tone = feedback.status === 'confirmed' || feedback.status === 'submitted'
      ? 'ok'
      : feedback.status === 'blocked' || feedback.status === 'uncertain'
        ? 'warn'
        : 'danger';
    return `<div class="orbit-ifeedback" data-tone="${tone}">${esc(feedback.message)}</div>`;
  }

  function renderCard(container, session, waitingSessions, info) {
    const tabs = waitingSessions.length > 1
      ? `<div class="orbit-itabs" role="tablist">${waitingSessions.map((item) => {
          const picked = item.id === session.id;
          return `<button type="button" class="orbit-itab" role="tab" aria-selected="${picked}" data-itarget="${esc(item.id)}"><span class="orbit-idot" aria-hidden="true"></span><span>${esc(sessionTitle(item))}</span></button>`;
        }).join('')}</div>`
      : '';

    const canFocus = window.AieyesSessions?.canFocus?.(session);
    const header = `<div class="orbit-icard-head">
      <div class="orbit-icard-meta">
        <strong>${esc(sessionTitle(session))}</strong>
        <small>${esc(formatSessionMeta(session))}</small>
      </div>
      <div class="orbit-icard-tools">
        <button type="button" class="orbit-icon-btn" data-iaction="refresh" title="${esc(tr('刷新待处理内容'))}" ${loading || submitting ? 'disabled' : ''}>↻</button>
        ${canFocus ? `<button type="button" class="orbit-icon-btn" data-iaction="focus" title="${esc(window.AieyesSessions.focusHelp(session))}">↗</button>` : ''}
      </div>
    </div>`;

    let body = '';
    if (loading && !currentPayload) {
      body = `<div class="orbit-iloading">${esc(tr('正在读取待处理内容…'))}</div>`;
    } else if (currentPayload) {
      const p = currentPayload;
      const draft = getDraft(p);
      if (p.kind === 'approval' && p.approval) {
        const a = p.approval;
        const cmdBlock = a.command ? `<pre class="orbit-code">${esc(a.command)}</pre>` : '';
        const detailBlock = a.details ? `<pre class="orbit-code orbit-code-muted">${esc(a.details)}</pre>` : '';
        const reasonRow = p.canRespond && showDeclineReason && p.adapter === 'claudeHook'
          ? `<div class="orbit-idecline-row"><input type="text" class="orbit-input" data-ifield="decline-reason" placeholder="${esc(tr('拒绝原因（可选）'))}" value="${esc(draft.declineReason)}"><button type="button" class="orbit-btn danger" data-iaction="confirm-decline" ${submitting ? 'disabled' : ''}>${esc(tr('确认拒绝'))}</button></div>`
          : '';
        const actionsHtml = p.canRespond
          ? `<div class="orbit-iactions">${(a.decisions || []).map((d) => {
              const cls = d.style === 'primary' ? 'primary' : d.style === 'danger' ? 'danger' : '';
              return `<button type="button" class="orbit-btn ${cls}" data-idecision="${esc(d.id)}" ${submitting ? 'disabled' : ''}>${esc(d.label)}</button>`;
            }).join('')}<span class="orbit-ispacer"></span>${p.canHandoff ? `<button type="button" class="orbit-btn ghost" data-iaction="handoff" title="${esc(tr('释放弹窗拦截并在原终端显示确认框'))}" ${submitting ? 'disabled' : ''}>${esc(tr('回原终端'))}</button>` : ''}</div>`
          : `${p.readonlyReason ? `<div class="orbit-ireadonly-reason">${esc(p.readonlyReason)}</div>` : ''}<div class="orbit-iactions"><span class="orbit-ispacer"></span>${canFocus ? `<button type="button" class="orbit-btn primary" data-iaction="focus">${esc(tr('跳转到原窗口'))}</button>` : ''}</div>`;
        body = `<div class="orbit-iapproval">
          <div class="orbit-itool-row"><span class="orbit-itool-tag">${esc(a.toolName || 'Permission')}</span><span class="orbit-ireason">${esc(a.reason || '')}</span></div>
          ${cmdBlock}${detailBlock}${reasonRow}
          ${actionsHtml}
        </div>`;
      } else if (p.kind === 'questions' && Array.isArray(p.questions)) {
        const isSingleDirect = p.canRespond && p.questions.length === 1 && !p.questions[0].multiSelect && (p.questions[0].options || []).length > 0;
        const blocks = p.questions.map((q) => {
          const opts = (q.options || []).map((o) => {
            const selected = q.multiSelect
              ? (draft.multi[q.id] || []).includes(o.label)
              : draft.single[q.id] === o.label;
            const pKey = `${q.id}:${o.label}`;
            const expanded = expandedPreviews.has(pKey);
            const previewHtml = o.preview
              ? `<button type="button" class="orbit-preview-toggle" data-ipreview="${esc(pKey)}">${esc(expanded ? tr('收起预览') : tr('查看预览'))}</button>${expanded ? `<pre class="orbit-code">${esc(o.preview)}</pre>` : ''}`
              : '';
            return `<div class="orbit-opt-wrap">
              <button type="button" class="orbit-opt" data-selected="${selected}" data-iqid="${esc(q.id)}" data-iolabel="${esc(o.label)}" data-idirect="${isSingleDirect}" ${submitting || !p.canRespond ? 'disabled' : ''}>
                <span class="orbit-opt-mark" aria-hidden="true">${q.multiSelect ? (selected ? '☑' : '☐') : (selected ? '●' : '○')}</span>
                <span class="orbit-opt-text"><strong>${esc(o.label)}</strong>${o.description ? `<small>${esc(o.description)}</small>` : ''}</span>
              </button>
              ${previewHtml}
            </div>`;
          }).join('');
          const customRow = p.canRespond && (q.allowCustom || !(q.options || []).length)
            ? `<div class="orbit-icustom-row">
                <input type="text" class="orbit-input" data-icustom="${esc(q.id)}" placeholder="${esc((q.options || []).length ? tr('其他自定义回答…') : tr('输入回答…'))}" value="${esc(draft.custom[q.id] || '')}">
                ${isSingleDirect ? `<button type="button" class="orbit-btn primary" data-iaction="submit-questions" ${submitting || !(draft.custom[q.id] || '').trim() ? 'disabled' : ''}>${esc(tr('发送'))}</button>` : ''}
              </div>`
            : '';
          return `<div class="orbit-qblock">
            <div class="orbit-qtitle">${esc(q.question)}${q.multiSelect ? `<span class="orbit-qhint">${esc(tr('（可多选）'))}</span>` : ''}</div>
            <div class="orbit-opts">${opts}</div>
            ${customRow}
          </div>`;
        }).join('');
        const footerRow = !p.canRespond
          ? `${p.readonlyReason ? `<div class="orbit-ireadonly-reason">${esc(p.readonlyReason)}</div>` : ''}<div class="orbit-iactions">${p.needsClaudeHook ? `<button type="button" class="orbit-btn" data-iaction="enable-claude-hook" ${submitting ? 'disabled' : ''}>${esc(tr('启用弹窗回答'))}</button>` : ''}<span class="orbit-ispacer"></span>${canFocus ? `<button type="button" class="orbit-btn primary" data-iaction="focus">${esc(tr('跳转到原窗口'))}</button>` : ''}</div>`
          : (!isSingleDirect || p.canHandoff
            ? `<div class="orbit-iactions">
                ${!isSingleDirect ? `<button type="button" class="orbit-btn primary" data-iaction="submit-questions" ${submitting || !allQuestionsAnswered(p, draft) ? 'disabled' : ''}>${esc(submitting ? tr('提交中…') : tr('提交回答'))}</button>` : ''}
                <span class="orbit-ispacer"></span>
                ${p.canHandoff ? `<button type="button" class="orbit-btn ghost" data-iaction="handoff" ${submitting ? 'disabled' : ''}>${esc(tr('回原终端'))}</button>` : ''}
              </div>`
            : '');
        body = `<div class="orbit-iquestions">${blocks}${footerRow}</div>`;
      } else if (p.kind === 'text') {
        const promptHtml = p.textPrompt ? `<div class="orbit-iprompt">${esc(p.textPrompt)}</div>` : '';
        body = `<div class="orbit-itext">
          ${promptHtml}
          <textarea class="orbit-textarea" rows="3" data-ifield="text-reply" placeholder="${esc(tr('输入回复内容…'))}">${esc(draft.text)}</textarea>
          <div class="orbit-iactions">
            <span class="orbit-iadapter">${esc(p.adapter === 'tmux' ? tr('通过 tmux 发送') : tr('通过原终端发送'))}</span>
            <span class="orbit-ispacer"></span>
            <button type="button" class="orbit-btn primary" data-iaction="send-text" ${submitting || !draft.text.trim() ? 'disabled' : ''}>${esc(submitting ? tr('发送中…') : tr('发送'))}</button>
          </div>
        </div>`;
      } else if (p.kind === 'readonly') {
        const promptHtml = p.textPrompt ? `<div class="orbit-iprompt">${esc(p.textPrompt)}</div>` : '';
        const reasonHtml = p.readonlyReason ? `<div class="orbit-ireadonly-reason">${esc(p.readonlyReason)}</div>` : '';
        const hookBtn = p.needsClaudeHook
          ? `<button type="button" class="orbit-btn" data-iaction="enable-claude-hook" ${submitting ? 'disabled' : ''}>${esc(tr('启用弹窗回答'))}</button>`
          : '';
        const focusBtn = canFocus
          ? `<button type="button" class="orbit-btn primary" data-iaction="focus">${esc(tr('跳转到原窗口'))}</button>`
          : '';
        body = `<div class="orbit-ireadonly">${promptHtml}${reasonHtml}<div class="orbit-iactions">${hookBtn}<span class="orbit-ispacer"></span>${focusBtn}</div></div>`;
      } else {
        body = `<div class="orbit-ireadonly-reason">${esc(tr('当前没有需要处理的请求'))}</div>`;
      }
    }

    container.innerHTML = `${tabs}<div class="orbit-icard">${header}${body}${renderFeedback()}</div>`;
    bindCardEvents(container, session, waitingSessions, info);
  }

  function bindCardEvents(container, session, waitingSessions, info) {
    for (const tab of container.querySelectorAll('[data-itarget]')) {
      tab.addEventListener('click', () => {
        activeSessionId = tab.dataset.itarget;
        renderedSignature = null;
        currentPayload = null;
        feedback = null;
        showDeclineReason = false;
        update(info, null, waitingSessions);
      });
    }
    container.querySelector('[data-iaction=refresh]')?.addEventListener('click', () => {
      void loadPayload(container, session, waitingSessions, info, true);
    });
    for (const btn of container.querySelectorAll('[data-iaction=focus]')) {
      btn.addEventListener('click', () => {
        window.AieyesSessions?.focus?.(session);
      });
    }
    container.querySelector('[data-iaction=enable-claude-hook]')?.addEventListener('click', async () => {
      submitting = true;
      renderCard(container, session, waitingSessions, info);
      try {
        statusCache = { ...statusCache, ...(await callCore('sessions.interaction.configure', { claudeQuestions: true })) };
        await loadPayload(container, session, waitingSessions, info, true);
      } catch (error) {
        feedback = { status: 'error', message: String(error) };
      } finally {
        submitting = false;
        renderCard(container, session, waitingSessions, info);
      }
    });
    container.querySelector('[data-iaction=handoff]')?.addEventListener('click', async () => {
      if (!currentPayload || submitting) return;
      submitting = true;
      renderCard(container, session, waitingSessions, info);
      try {
        const res = await callCore('sessions.interaction.handoff', {
          sessionId: currentPayload.sessionId,
          requestId: currentPayload.requestId,
        });
        feedback = { status: res.handedOff ? 'confirmed' : 'blocked', message: res.message || '' };
        if (res.handedOff) {
          window.AieyesSessions?.focus?.(session);
          scheduleAdvance(container, session, waitingSessions, info);
        }
      } catch (error) {
        feedback = { status: 'error', message: String(error) };
      } finally {
        submitting = false;
        renderCard(container, session, waitingSessions, info);
      }
    });

    for (const btn of container.querySelectorAll('[data-idecision]')) {
      btn.addEventListener('click', async () => {
        if (!currentPayload || submitting) return;
        const decision = btn.dataset.idecision;
        if (decision === 'decline' && currentPayload.adapter === 'claudeHook' && !showDeclineReason) {
          showDeclineReason = true;
          renderCard(container, session, waitingSessions, info);
          container.querySelector('[data-ifield=decline-reason]')?.focus();
          return;
        }
        const draft = getDraft(currentPayload);
        await submitResponse(container, session, waitingSessions, info, {
          action: decision,
          reason: decision === 'decline' ? draft.declineReason : undefined,
        });
      });
    }
    container.querySelector('[data-iaction=confirm-decline]')?.addEventListener('click', async () => {
      if (!currentPayload || submitting) return;
      const draft = getDraft(currentPayload);
      await submitResponse(container, session, waitingSessions, info, {
        action: 'decline',
        reason: draft.declineReason,
      });
    });
    container.querySelector('[data-ifield=decline-reason]')?.addEventListener('input', (e) => {
      if (currentPayload) getDraft(currentPayload).declineReason = e.target.value;
    });

    for (const opt of container.querySelectorAll('.orbit-opt')) {
      opt.addEventListener('click', async () => {
        if (!currentPayload || submitting) return;
        const qid = opt.dataset.iqid;
        const label = opt.dataset.iolabel;
        const direct = opt.dataset.idirect === 'true';
        const q = (currentPayload.questions || []).find((item) => item.id === qid);
        if (!q) return;
        const draft = getDraft(currentPayload);
        if (q.multiSelect) {
          const list = Array.isArray(draft.multi[qid]) ? [...draft.multi[qid]] : [];
          const idx = list.indexOf(label);
          if (idx >= 0) list.splice(idx, 1);
          else list.push(label);
          draft.multi[qid] = list;
          renderCard(container, session, waitingSessions, info);
          return;
        }
        draft.single[qid] = label;
        draft.custom[qid] = '';
        if (direct) {
          await submitResponse(container, session, waitingSessions, info, {
            action: 'answer',
            answers: buildAnswers(currentPayload, draft),
          });
        } else {
          renderCard(container, session, waitingSessions, info);
        }
      });
    }
    for (const toggle of container.querySelectorAll('[data-ipreview]')) {
      toggle.addEventListener('click', () => {
        const key = toggle.dataset.ipreview;
        if (expandedPreviews.has(key)) expandedPreviews.delete(key);
        else expandedPreviews.add(key);
        renderCard(container, session, waitingSessions, info);
      });
    }
    for (const input of container.querySelectorAll('[data-icustom]')) {
      input.addEventListener('input', (e) => {
        if (!currentPayload) return;
        const qid = input.dataset.icustom;
        const draft = getDraft(currentPayload);
        draft.custom[qid] = e.target.value;
        const q = (currentPayload.questions || []).find((item) => item.id === qid);
        if (q && !q.multiSelect && e.target.value.trim()) {
          delete draft.single[qid];
        }
        const submitBtn = container.querySelector('[data-iaction=submit-questions]');
        if (submitBtn) submitBtn.disabled = submitting || !allQuestionsAnswered(currentPayload, draft);
      });
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' && currentPayload && allQuestionsAnswered(currentPayload, getDraft(currentPayload))) {
          e.preventDefault();
          void submitResponse(container, session, waitingSessions, info, {
            action: 'answer',
            answers: buildAnswers(currentPayload, getDraft(currentPayload)),
          });
        }
      });
    }
    container.querySelector('[data-iaction=submit-questions]')?.addEventListener('click', async () => {
      if (!currentPayload || submitting) return;
      const draft = getDraft(currentPayload);
      if (!allQuestionsAnswered(currentPayload, draft)) return;
      await submitResponse(container, session, waitingSessions, info, {
        action: 'answer',
        answers: buildAnswers(currentPayload, draft),
      });
    });

    const textarea = container.querySelector('[data-ifield=text-reply]');
    if (textarea) {
      textarea.addEventListener('input', () => {
        if (!currentPayload) return;
        getDraft(currentPayload).text = textarea.value;
        const btn = container.querySelector('[data-iaction=send-text]');
        if (btn) btn.disabled = submitting || !textarea.value.trim();
      });
      textarea.addEventListener('keydown', (e) => {
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
          e.preventDefault();
          const body = textarea.value.trim();
          if (body && !submitting) {
            void submitResponse(container, session, waitingSessions, info, { action: 'text', text: body });
          }
        }
      });
    }
    container.querySelector('[data-iaction=send-text]')?.addEventListener('click', async () => {
      if (!currentPayload || submitting) return;
      const body = (getDraft(currentPayload).text || '').trim();
      if (!body) return;
      await submitResponse(container, session, waitingSessions, info, { action: 'text', text: body });
    });
  }

  async function submitResponse(container, session, waitingSessions, info, extra) {
    if (!currentPayload || submitting) return;
    submitting = true;
    renderCard(container, session, waitingSessions, info);
    try {
      const res = await callCore('sessions.interaction.respond', {
        sessionId: currentPayload.sessionId,
        stateKey: currentPayload.stateKey,
        promptHash: currentPayload.promptHash,
        ...extra,
      });
      feedback = { status: res.status || 'blocked', message: res.message || '' };
      if (res.status === 'confirmed' || res.status === 'submitted') {
        clearDraft(currentPayload);
        showDeclineReason = false;
        scheduleAdvance(container, session, waitingSessions, info);
      }
    } catch (error) {
      feedback = { status: 'error', message: String(error) };
    } finally {
      submitting = false;
      renderCard(container, session, waitingSessions, info);
    }
  }

  function scheduleAdvance(container, session, waitingSessions, info) {
    setTimeout(() => {
      const next = waitingSessions.find((s) => s.id !== session.id);
      if (next) {
        activeSessionId = next.id;
        renderedSignature = null;
        currentPayload = null;
        feedback = null;
        update(info, null, waitingSessions);
      } else if (info?.orbitAuto) {
        window.__TAURI__?.core?.invoke('desktop_action', { action: 'close-panel' }).catch(() => {});
      }
    }, 700);
  }

  async function loadPayload(container, session, waitingSessions, info, force = false) {
    if (loading && !force) return;
    loading = true;
    if (force) currentPayload = null;
    renderCard(container, session, waitingSessions, info);
    try {
      const result = await callCore('sessions.interaction.inspect', { sessionId: session.id });
      if (activeSessionId === session.id) {
        currentPayload = result;
        if (result?.kind && result.kind !== 'none') feedback = null;
      }
    } catch (error) {
      if (activeSessionId === session.id) {
        feedback = { status: 'error', message: String(error) };
      }
    } finally {
      loading = false;
      if (activeSessionId === session.id) {
        renderCard(container, session, waitingSessions, info);
      }
    }
  }

  function update(info, _entry, activeSessions) {
    const container = document.querySelector('#orbit-interaction');
    if (!container) return false;
    if (!statusLoaded) void refreshStatus();
    const waiting = (activeSessions || []).filter((s) => s.phase === 'needsInput');
    if (!statusCache.enabled || !waiting.length) {
      container.hidden = true;
      container.innerHTML = '';
      activeSessionId = null;
      renderedSignature = null;
      currentPayload = null;
      feedback = null;
      return false;
    }
    container.hidden = false;
    let target = waiting.find((s) => s.id === activeSessionId);
    if (!target && info?.orbitPreferredSession) {
      target = waiting.find((s) => s.id === info.orbitPreferredSession);
    }
    if (!target) target = waiting[0];
    activeSessionId = target.id;
    const sig = `${target.id}|${target.inputStateKey || target.turnId || ''}|${waiting.map((s) => s.id).join(',')}`;
    if (sig === renderedSignature && container.children.length > 0) {
      return true;
    }
    const sessionChanged = !renderedSignature || !renderedSignature.startsWith(`${target.id}|${target.inputStateKey || target.turnId || ''}|`);
    renderedSignature = sig;
    if (sessionChanged) {
      currentPayload = null;
      feedback = null;
      showDeclineReason = false;
      void loadPayload(container, target, waiting, info, false);
    } else {
      renderCard(container, target, waiting, info);
    }
    return true;
  }

  function reset() {
    activeSessionId = null;
    renderedSignature = null;
    currentPayload = null;
    feedback = null;
    showDeclineReason = false;
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
    settingsHTML,
    bindSettings,
  };
})();
