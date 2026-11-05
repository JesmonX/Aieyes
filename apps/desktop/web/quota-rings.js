/* Quota rings: the compact account styles (dial and two-column tiles) beside the cards.
   One ring per account, or per model group for grouped quotas. The top half is the short
   window (5h), the bottom half the long one (7d); colored arcs are the remaining share and
   both grow from the left, mirrored. The rules match crates/core/src/quota_ring.rs (tray)
   and QuotaRingCore.swift (macOS). Each window remembers its own style. */
(() => {
  const FIVE = 300, WEEK = 10080, STYLES = ['card', 'dial', 'tile'];
  const KEYS = { panel: 'aieyes.quota.style.panel', detail: 'aieyes.quota.style.detail' };
  let hooks = {}, open = '';
  const t = (...args) => globalThis.AieyesI18n ? globalThis.AieyesI18n.t(...args) : args[0];
  const esc = (value) => String(value ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
  const surface = () => (hooks.panel ?? Boolean(globalThis.document?.querySelector('#panel'))) ? 'panel' : 'detail';

  function style(which = surface()) {
    try {const value = localStorage.getItem(KEYS[which]);if (STYLES.includes(value)) return value;} catch (_) {}
    return 'card';
  }
  function setStyle(which, value) {
    if (!STYLES.includes(value) || !KEYS[which]) return;
    try {localStorage.setItem(KEYS[which], value);} catch (_) {}
  }

  // ---- Shared rules (quota_ring.rs) ----
  function groupsWindows(q) {
    if (q.provider === 'codex') return false;
    if (q.provider === 'antigravity') return true;
    const counts = new Map();
    for (const w of q.windows ?? []) if (w.groupName) counts.set(w.groupName, (counts.get(w.groupName) ?? 0) + 1);
    return [...counts.values()].some((n) => n > 1);
  }
  const groupLabel = (w) => w.groupName || String(w.name ?? '').split(' · ').slice(0, -1).join(' · ');
  // Every group gets its own tile; ungrouped quotas are one set.
  function windowSets(q) {
    const windows = q.windows ?? [];
    if (!groupsWindows(q) || !windows.length) return [{ label: '', windows }];
    const sets = new Map();
    for (const w of windows) {const label = groupLabel(w);if (!sets.has(label)) sets.set(label, []);sets.get(label).push(w);}
    return [...sets].map(([label, rows]) => ({ label, windows: rows }));
  }
  const timed = (w) => Number.isFinite(w.windowMinutes) && w.windowMinutes > 0;
  // A 5h + 7d pair wins; otherwise two windows of different known lengths, shorter on top;
  // otherwise the first window alone.
  function pick(windows) {
    const five = windows.find((w) => w.windowMinutes === FIVE),week = windows.find((w) => w.windowMinutes === WEEK);
    if (five && week) return [five, week];
    const sorted = windows.filter(timed).sort((a, b) => a.windowMinutes - b.windowMinutes);
    const top = sorted[0],bottom = top && sorted.find((w) => w.windowMinutes !== top.windowMinutes);
    if (top && bottom) return [top, bottom];
    return [windows[0] ?? null, null];
  }
  function arc(w) {
    const used = Number.isFinite(w.usedPercent) ? Math.min(100, Math.max(0, w.usedPercent)) : 100;
    const resetsAt = Number.isFinite(w.resetsAt) && w.resetsAt > 0 ? w.resetsAt : null;
    return { remaining: 100 - used, resetsAt, windowMinutes: Number.isFinite(w.windowMinutes) ? w.windowMinutes : null, unused: used === 0 && resetsAt == null, window: w };
  }
  function currency(code) {
    const value = String(code ?? '').trim().toUpperCase();
    return value === 'CNY' || value === 'RMB' ? '¥' : value === 'USD' ? '$' : value ? value + ' ' : '';
  }
  function meterAmount(unit, value) {
    const text = Number.isInteger(value) ? value.toFixed(0) : value.toFixed(2);
    return unit === 'usd' ? '$' + text : unit === 'cny' ? '¥' + text : text;
  }
  const isQoderCredits = q => ['qoder','qoder-cn'].includes(q.provider);
  const creditNumber = n => Number.isFinite(n) ? (Number.isInteger(n) ? String(n) : n.toFixed(2)) : '—';
  function qoderCreditSum(q) {
    const plan = (q.meters ?? []).find(m => m.id === 'userQuota'), addon = (q.meters ?? []).find(m => m.id === 'addOnQuota');
    return creditNumber(plan?.remaining) + (addon?.remaining === 0 ? '' : '+' + creditNumber(addon?.remaining));
  }
  function ring(q, windows = windowSets(q)[0].windows) {
    if (isQoderCredits(q)) {
      const meter = (q.meters ?? []).find(m => m.id === 'userQuota');
      if (!meter || meter.usedKnown === false || !(meter.total > 0) || !Number.isFinite(meter.used)) return {kind:'empty'};
      const addon = (q.meters ?? []).find(m => m.id === 'addOnQuota');
      // Credit buckets have fixed halves: addon above plan, even when no addon capacity exists.
      const creditArc = meter => ({remaining:meter?.usedKnown !== false && meter?.total > 0 && Number.isFinite(meter.used) ? 100 - Math.min(1,Math.max(0,meter.used/meter.total))*100 : 0, resetsAt:null,windowMinutes:null,unused:false,meter});
      return {kind:'duo', top:creditArc(addon), bottom:creditArc(meter)};
    }
    const [top, bottom] = pick(windows);
    if (top && bottom) return { kind: 'duo', top: arc(top), bottom: arc(bottom) };
    if (top) return { kind: 'single', top: arc(top) };
    const meters = q.meters ?? [];
    const meter = meters.find((m) => m.usedKnown !== false && m.total > 0 && Number.isFinite(m.used));
    if (meter) {
      const fraction = Math.min(1, Math.max(0, meter.used / meter.total));
      return { kind: 'single', top: { remaining: 100 - fraction * 100, resetsAt: meter.resetsAt > 0 ? meter.resetsAt : null, windowMinutes: null, unused: false, meter } };
    }
    const low = q.isAvailable === false;
    const balance = q.balances?.[0];
    if (balance) return { kind: 'coin', amount: currency(balance.currency) + String(balance.total ?? '').trim(), low };
    if (q.extraUsage) return { kind: 'coin', amount: currency(q.extraUsage.currency) + (q.extraUsage.balance ?? q.extraUsage.total), low };
    const remaining = meters.find((m) => m.remaining != null);
    if (remaining) return { kind: 'coin', amount: meterAmount(remaining.unit, Number(remaining.remaining)), low };
    if (q.credits?.balance != null) return { kind: 'coin', amount: String(q.credits.balance), low };
    return { kind: 'empty' };
  }
  // `47m`, `2h13m`, `14h`, `1d5h`, `3d`: unit letters only, so every language reads it.
  function compactMinutes(minutes) {
    minutes = Math.max(1, minutes);
    if (minutes < 60) return minutes + 'm';
    if (minutes < 600) {const hours = Math.floor(minutes / 60),rest = minutes % 60;return rest ? `${hours}h${rest}m` : `${hours}h`;}
    if (minutes < 1440) return Math.round(minutes / 60) + 'h';
    if (minutes < 4320) {
      const days = Math.floor(minutes / 1440),hours = Math.round(minutes % 1440 / 60);
      return hours === 0 ? days + 'd' : hours === 24 ? days + 1 + 'd' : `${days}d${hours}h`;
    }
    return Math.round(minutes / 1440) + 'd';
  }
  // One unit only, so a warning glyph still fits inside the dial.
  function shortMinutes(minutes) {
    minutes = Math.max(1, minutes);
    return minutes < 60 ? minutes + 'm' : minutes < 1440 ? Math.round(minutes / 60) + 'h' : Math.round(minutes / 1440) + 'd';
  }
  // An unused 5h/7d window shows its length, a passed reset `…` (the next reading confirms it), an unknown one `—`.
  function resetText(value, now = Date.now() / 1000, short = false) {
    if (value.unused && value.windowMinutes === FIVE) return '5h';
    if (value.unused && value.windowMinutes === WEEK) return '7d';
    const stamp = value.resetsAt;
    if (stamp == null) return '—';
    if (stamp <= Math.floor(now)) return '…';
    const minutes = Math.floor((stamp - Math.floor(now) + 59) / 60);
    return short ? shortMinutes(minutes) : compactMinutes(minutes);
  }
  // Warn at 30% remaining or less, danger under 10% (the cards' thresholds).
  const level = (remaining) => remaining < 10 ? 'danger' : remaining <= 30 ? 'warn' : 'ok';
  const percent = (value) => Math.round(value) + '%';

  // ---- Tile models ----
  function models(q) {
    const key = q.provider + ':' + q.accountId,grouped = groupsWindows(q);
    return windowSets(q).map((set, index) => {
      let value = ring(q, set.windows);
      if (value.kind === 'empty' && q.error) value = { kind: 'error' };
      const picked = value.kind === 'duo' ? [value.top.window, value.bottom.window] : value.top?.window ? [value.top.window] : [];
      return { id: key + '#' + index, index, key, q, group: grouped ? set.label : '', name: grouped && set.label ? set.label : q.name, ring: value, hidden: set.windows.filter((w) => !picked.includes(w)) };
    });
  }
  function slot(value, group = '') {
    if (value.meter) return globalThis.AieyesI18n?.message(value.meter.label) ?? value.meter.label ?? '';
    if (value.windowMinutes === FIVE) return '5h';
    if (value.windowMinutes === WEEK) return '7d';
    let name = String(value.window?.name ?? '');
    if (group && name.startsWith(group + ' · ')) name = name.slice(group.length + 3);
    return (globalThis.AieyesI18n?.message(name) ?? name) || (value.windowMinutes ? compactMinutes(value.windowMinutes) : '');
  }
  function resetFull(value, now) {
    if (value.window) return globalThis.AieyesUI?.quotaReset(value.window, now) ?? '';
    return value.resetsAt ? globalThis.AieyesUI?.resetText(value.resetsAt, now) ?? '' : '';
  }
  function creditShort(q) {
    if (q.credits?.unlimited) return t("无限");
    const n = Number(q.credits?.balance);
    if (q.credits?.balance == null || !Number.isFinite(n)) return '';
    const abs = Math.abs(n);
    if (abs < 999.5) return String(Math.round(n));
    if (abs < 999950) return +(n / 1e3).toFixed(abs < 9950 ? 1 : 0) + 'K';
    return +(n / 1e6).toFixed(1) + 'M';
  }
  function balanceAmount(q) {
    const balance = q.balances?.[0];
    if (balance) return currency(balance.currency) + String(balance.total ?? '').trim();
    return q.extraUsage?.balance != null ? currency(q.extraUsage.currency) + q.extraUsage.balance : '';
  }
  // Up to two short facts under or beside the ring; `text` is the spoken form.
  function extras(m, tileStyle) {
    const q = m.q,value = m.ring,out = [],first = m.index === 0;
    if (q.error && !(tileStyle === 'tile' && value.kind === 'error')) out.push({ level: 'danger', html: esc(t("额度更新失败")), text: t("额度更新失败") });
    if (value.kind === 'coin' && value.low && tileStyle === 'dial') out.push({ level: 'danger', html: esc(t("余额不足")), text: t("余额不足") });
    if (first && q.bankReset?.availableCount > 0) out.push({ html: GLYPH.reset + esc(q.bankReset.availableCount), text: 'Bank Reset ' + t("{0} 次可用", q.bankReset.availableCount) });
    if (first && value.kind !== 'coin') {
      const credits = creditShort(q);if (credits) out.push({ html: GLYPH.credit + esc(credits), text: t("余额") + ' ' + credits + ' credits' });
      const amount = balanceAmount(q);if (amount) out.push({ html: esc(amount), text: t("可用余额") + ' ' + amount });
    }
    if (m.hidden.length) {
      const lowest = Math.min(...m.hidden.map((w) => arc(w).remaining)),tone = lowest <= 30 ? level(lowest) : '';
      out.push({ level: tone, html: (tone === 'danger' ? GLYPH.warn : '') + '+' + m.hidden.length, text: m.hidden.map((w) => {const a = arc(w);return slot(a, m.group) + ' ' + t("剩余 {0}", percent(a.remaining));}).join(separators().comma) });
    }
    if (q.staleSince) {const stamp = q.snapshotAt > 0 ? q.snapshotAt : q.updatedAt,age = stamp > 0 ? hooks.age?.(stamp) : '';if (age) out.push({ html: esc(t("上次 {0}", age)), text: t("上次 {0}", age) });}
    return out;
  }
  const separators = () => globalThis.AieyesI18n?.language === 'en' ? { comma: ', ', semi: '; ' } : { comma: '，', semi: '；' };
  function label(m, now) {
    if (isQoderCredits(m.q)) return m.q.name + ': ' + ['addOnQuota','userQuota'].map(id => {
      const meter = (m.q.meters ?? []).find(m => m.id === id);
      return (id === 'userQuota' ? 'Plan credits ' : 'Addon credits ') + creditNumber(meter?.remaining) + '/' + creditNumber(meter?.total);
    }).join('; ');
    const { comma, semi } = separators(),value = m.ring,name = m.group ? m.q.name + ' · ' + m.group : m.q.name;
    const half = (a) => [[slot(a, m.group), t("剩余 {0}", percent(a.remaining))].filter(Boolean).join(' '), resetFull(a, now)].filter(Boolean).join(comma);
    const parts = value.kind === 'duo' ? [half(value.top), half(value.bottom)] : value.kind === 'single' ? [half(value.top)] : value.kind === 'coin' ? [t("可用余额") + ' ' + value.amount] : [];
    for (const extra of extras(m, 'dial')) parts.push(extra.text);
    return parts.length ? name + comma + parts.join(semi) : name;
  }

  // ---- Drawing ----
  const GLYPH = {
    warn: '<svg class="ring-glyph" viewBox="0 0 12 12" aria-hidden="true"><path d="M6 1.7 10.9 10H1.1Z" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round"/><path d="M6 5v2.3" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/><circle cx="6" cy="8.7" r=".65" fill="currentColor"/></svg>',
    reset: '<svg class="ring-glyph" viewBox="0 0 12 12" aria-hidden="true"><path d="M2.4 1.7v2.6h2.6M2.7 4.3A4 4 0 1 1 2 6.6" fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"/></svg>',
    credit: '<svg class="ring-glyph" viewBox="0 0 12 12" aria-hidden="true"><circle cx="6" cy="6" r="4.6" fill="none" stroke="currentColor" stroke-width="1.2"/><path d="M7.5 4.7a2 2 0 1 0 0 2.6" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round"/></svg>',
    card: '<svg viewBox="0 0 14 14" aria-hidden="true"><rect x="1.5" y="2" width="11" height="4.2" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.2"/><rect x="1.5" y="7.8" width="11" height="4.2" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>',
    dial: '<svg viewBox="0 0 14 14" aria-hidden="true"><path d="M2.1 6A5 5 0 0 1 11.9 6M2.1 8A5 5 0 0 0 11.9 8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>',
    tile: '<svg viewBox="0 0 14 14" aria-hidden="true"><rect x="1.2" y="2.5" width="5.1" height="9" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.2"/><rect x="7.7" y="2.5" width="5.1" height="9" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.2"/></svg>',
  };
  const rad = (d) => d * Math.PI / 180,deg = (r) => r * 180 / Math.PI;
  function arcPath(c, r, a0, a1) {
    if (Math.abs(a1 - a0) < .2) a1 = a0 + (a1 >= a0 ? .2 : -.2);
    const point = (a) => [(c + r * Math.cos(rad(a))).toFixed(2), (c + r * Math.sin(rad(a))).toFixed(2)];
    const [x0, y0] = point(a0),[x1, y1] = point(a1);
    return `M${x0} ${y0}A${r.toFixed(2)} ${r.toFixed(2)} 0 ${Math.abs(a1 - a0) > 180 ? 1 : 0} ${a1 > a0 ? 1 : 0} ${x1} ${y1}`;
  }
  // SVG angles run clockwise from 3 o'clock. Both halves start at 9 o'clock (180°): the top
  // grows clockwise over 12, the bottom counter-clockwise under 6, leaving a gap at each end.
  function ringSVG(value, size, width) {
    const c = size / 2,r = (size - width) / 2,sw = `stroke-width="${width}"`,gap = size >= 64 ? 3 : size >= 36 ? 2.5 : 2;
    let body;
    if (value.kind === 'coin') body = `<circle class="ring-coin" cx="${c}" cy="${c}" r="${c}"/><circle class="ring-rim" cx="${c}" cy="${c}" r="${(c - width * .8).toFixed(2)}" stroke-width="${Math.max(1, width * .26).toFixed(2)}"/>`;
    else if (value.kind === 'duo') {
      const g = deg(width / 2 / r) + deg(gap / 2 / r),span = 180 - 2 * g,top = value.top.remaining / 100,bottom = value.bottom.remaining / 100;
      body = `<path class="ring-track" ${sw} d="${arcPath(c, r, 180 + g, 360 - g)}"/><path class="ring-track" ${sw} d="${arcPath(c, r, 180 - g, g)}"/>`;
      if (top > 0) body += `<path class="ring-fill" data-level="${level(value.top.remaining)}" ${sw} d="${arcPath(c, r, 180 + g, 180 + g + span * top)}"/>`;
      if (bottom > 0) body += `<path class="ring-fill" data-level="${level(value.bottom.remaining)}" ${sw} d="${arcPath(c, r, 180 - g, 180 - g - span * bottom)}"/>`;
    } else if (value.kind === 'single') {
      const share = value.top.remaining / 100,tone = level(value.top.remaining);
      body = `<circle class="ring-track" ${sw} cx="${c}" cy="${c}" r="${r}"/>`;
      if (share >= .999) body += `<circle class="ring-fill" data-level="${tone}" ${sw} cx="${c}" cy="${c}" r="${r}"/>`;
      else if (share > 0) body += `<path class="ring-fill" data-level="${tone}" ${sw} d="${arcPath(c, r, 270, 270 + 360 * share)}"/>`;
    } else body = `<circle class="ring-dots" cx="${c}" cy="${c}" r="${r}" stroke-width="${(width * .72).toFixed(2)}" stroke-dasharray="0.01 ${(width * 1.45).toFixed(2)}"/>`;
    return `<svg class="ring-art" viewBox="0 0 ${size} ${size}" width="${size}" height="${size}" aria-hidden="true" focusable="false">${body}</svg>`;
  }
  const geometry = (tileStyle) => {const panel = surface() === 'panel';return tileStyle === 'dial' ? { size: panel ? 72 : 84, width: panel ? 6 : 6.5 } : { size: panel ? 48 : 56, width: panel ? 4.5 : 5 };};
  // The provider mark is decorative inside a tile; the tile's own label names the account.
  const mark = (provider) => (hooks.mark?.(provider) ?? '').replace(/ (?:title|aria-label)="[^"]*"/g, '').replace('<span class="provider-mark', '<span aria-hidden="true" class="provider-mark');
  function timeHTML(value, now, short = false) {
    return `<span class="ring-time" data-ring-reset="${value.resetsAt ?? ''}" data-window-minutes="${value.windowMinutes ?? ''}" data-unused="${value.unused}"${short ? ' data-short' : ''}>${esc(resetText(value, now, short))}</span>`;
  }
  function ringHTML(m, tileStyle, center) {
    const { size, width } = geometry(tileStyle),value = m.ring;
    return `<span class="ring" data-kind="${value.kind}"${value.low ? ' data-low' : ''}${m.q.staleSince ? ' data-stale' : ''} style="--ring-size:${size}px">${ringSVG(value, size, width)}<span class="ring-center">${center}</span></span>`;
  }
  function dialCenter(m, now) {
    if (isQoderCredits(m.q)) return `<span class="ring-value qoder-credit-sum">${esc(qoderCreditSum(m.q))}</span>`;
    const value = m.ring;
    if (value.kind === 'duo') {
      const line = (a) => {const tone = level(a.remaining);return `<span class="ring-value" data-level="${tone}"><span class="ring-swap-time">${tone === 'danger' ? GLYPH.warn : ''}${timeHTML(a, now, tone === 'danger')}</span><span class="ring-swap-pct">${percent(a.remaining)}</span></span>`;};
      return line(value.top) + '<i class="ring-divider" aria-hidden="true"></i>' + line(value.bottom);
    }
    if (value.kind === 'single') return `<span class="ring-value" data-level="${level(value.top.remaining)}">${percent(value.top.remaining)}</span>${resetText(value.top, now) === '—' ? '' : `<span class="ring-value ring-sub">${timeHTML(value.top, now)}</span>`}`;
    if (value.kind === 'coin') return `<span class="ring-amount"${value.low ? ' data-level="danger"' : ''}>${esc(value.amount)}</span>`;
    if (value.kind === 'error') return '<span class="ring-bang" aria-hidden="true">!</span>';
    return '';
  }
  function tileCenter(m) {
    const value = m.ring,number = (a) => `<span class="ring-num" data-level="${level(a.remaining)}">${Math.round(a.remaining)}</span>`;
    if (value.kind === 'duo') return number(value.top) + number(value.bottom);
    if (value.kind === 'single') return number(value.top);
    if (value.kind === 'coin') return mark(m.q.provider);
    if (value.kind === 'error') return '<span class="ring-bang" aria-hidden="true">!</span>';
    return '';
  }
  function tileTimes(m, now) {
    if (isQoderCredits(m.q)) return ['addOnQuota','userQuota'].map(id => {
      const meter = (m.q.meters ?? []).find(m => m.id === id), name = id === 'userQuota' ? 'Plan credits' : 'Addon credits';
      return `<span class="tile-time qoder-credit-ratio" title="${name}" aria-label="${name} ${esc(creditNumber(meter?.remaining))}/${esc(creditNumber(meter?.total))}">${esc(creditNumber(meter?.remaining))}/${esc(creditNumber(meter?.total))}</span>`;
    }).join('');
    const value = m.ring,line = (a) => {const tone = level(a.remaining);return `<span class="tile-time" data-level="${tone}">${tone === 'danger' ? GLYPH.warn : ''}${timeHTML(a, now)}</span>`;};
    if (value.kind === 'duo') return line(value.top) + line(value.bottom);
    if (value.kind === 'single') return line(value.top);
    if (value.kind === 'coin') return `<span class="tile-time"${value.low ? ' data-level="danger"' : ''}>${value.low ? GLYPH.warn : ''}<span class="tile-amount">${esc(value.amount)}</span></span>${value.low ? `<span class="tile-sub" data-level="danger">${esc(t("余额不足"))}</span>` : ''}`;
    if (value.kind === 'error') return `<span class="tile-time" data-level="danger">${esc(t("额度更新失败"))}</span>`;
    return '<span class="tile-time">—</span>';
  }
  const extrasHTML = (list) => list.map((e) => `<span${e.level ? ` data-level="${e.level}"` : ''}>${e.html}</span>`).join('');
  function shortName(name) {
    return ({'gemini':'Geminis','gemini models':'Geminis','claude-gpt':'C&Gs','claude and gpt models':'C&Gs','claudes & gpts':'C&Gs','gemini-4-argon':'Argon','gemini 4 argon':'Argon'})[String(name).toLowerCase()] || name;
  }
  function tileHTML(m, tileStyle, now) {
    const text = label(m, now),list = extras(m, tileStyle).slice(0, 2);
    const attributes = `type="button" class="quota-tile" data-style="${tileStyle}" data-provider="${esc(m.q.provider)}" data-quota-account="${esc(m.key)}" data-quota-tile="${esc(m.id)}" aria-expanded="${open === m.id}" aria-label="${esc(text)}" title="${esc(text)}"`;
    if (tileStyle === 'dial') return `<button ${attributes}>${ringHTML(m, 'dial', dialCenter(m, now))}<span class="tile-name">${mark(m.q.provider)}<span>${esc(shortName(m.name))}</span></span>${list.length ? `<span class="tile-extras">${extrasHTML(list)}</span>` : ''}</button>`;
    const plan = !m.group && m.q.plan ? globalThis.AieyesUI?.subscription(m.q.plan) ?? m.q.plan : '';
    return `<button ${attributes}><span class="tile-head">${mark(m.q.provider)}<span class="tile-head-name">${esc(shortName(m.name))}</span>${plan ? `<span class="tile-plan">${esc(plan)}</span>` : ''}</span><span class="tile-body">${ringHTML(m, 'tile', tileCenter(m))}<span class="tile-times">${tileTimes(m, now)}</span><span class="tile-extras">${extrasHTML(list)}</span></span></button>`;
  }
  function placeholderHTML(account, tileStyle) {
    const { size, width } = geometry(tileStyle),key = account.provider + ':' + account.id,status = hooks.placeholder?.() ?? t("尚未读取额度");
    const art = `<span class="ring" data-kind="empty" style="--ring-size:${size}px">${ringSVG({ kind: 'empty' }, size, width)}<span class="ring-center"></span></span>`;
    const attributes = `class="quota-tile" data-style="${tileStyle}" data-placeholder tabindex="0" data-provider="${esc(account.provider)}" data-quota-account="${esc(key)}" data-quota-placeholder="${esc(key)}" title="${esc(account.name + separators().comma + status)}"`;
    if (tileStyle === 'dial') return `<div ${attributes}>${art}<span class="tile-name">${mark(account.provider)}<span>${esc(account.name)}</span></span><span class="tile-extras"><span role="status">${esc(status)}</span></span></div>`;
    return `<div ${attributes}><span class="tile-head">${mark(account.provider)}<span class="tile-head-name">${esc(account.name)}</span></span><span class="tile-body">${art}<span class="tile-times"><span class="tile-sub" role="status">${esc(status)}</span></span><span class="tile-extras"></span></span></div>`;
  }
  function accountHTML(account, quota, tileStyle, now, shown) {
    if (!quota) return placeholderHTML(account, tileStyle);
    return models(quota).map((m) => {shown?.add(m.id);return tileHTML(m, tileStyle, now) + (open === m.id ? hooks.card?.(quota, m) ?? '' : '');}).join('');
  }
  // `entries`: [{ account, quota }] in display order; quota is undefined until the first reading.
  function gridHTML(entries, tileStyle = style()) {
    const now = Date.now() / 1000,shown = new Set();
    const body = entries.map(({ account, quota }) => accountHTML(account, quota, tileStyle, now, shown)).join('');
    if (!shown.has(open)) open = '';
    return `<div class="quota-rings" data-style="${tileStyle}">${body}</div>`;
  }
  function switchHTML(current = style()) {
    const names = { card: t("卡片样式"), dial: t("表盘样式"), tile: t("双列样式") };
    return `<div class="segmented quota-style-switch" role="group" aria-label="${esc(t("额度显示样式"))}">${STYLES.map((s) => `<button type="button" data-quota-style="${s}" aria-pressed="${s === current}" tabindex="${s === current ? 0 : -1}" title="${esc(names[s])}" aria-label="${esc(names[s])}">${GLYPH[s]}</button>`).join('')}</div>`;
  }

  // ---- Live updates ----
  const grid = () => globalThis.document?.querySelector('#content .quota-rings');
  // Replaces one account's tiles (and its open card) in place. False when no ring grid is shown.
  function replace(quota) {
    const container = grid();if (!container) return false;
    const key = quota.provider + ':' + quota.accountId,selector = CSS.escape(key);
    const nodes = [...container.querySelectorAll(`[data-quota-account="${selector}"],[data-ring-detail][data-quota="${selector}"]`)];
    if (!nodes.length) return true;
    const wasOpen = open.startsWith(key + '#'),shown = new Set(),template = document.createElement('template');
    template.innerHTML = accountHTML(null, quota, container.dataset.style, Date.now() / 1000, shown);
    if (wasOpen && !shown.has(open)) open = '';
    nodes[0].before(template.content);nodes.forEach((node) => node.remove());
    return true;
  }
  function refreshTimes(now = Date.now() / 1000) {
    const container = grid();if (!container) return;
    for (const el of container.querySelectorAll('[data-ring-reset]')) {
      const value = { resetsAt: el.dataset.ringReset ? Number(el.dataset.ringReset) : null, windowMinutes: el.dataset.windowMinutes ? Number(el.dataset.windowMinutes) : null, unused: el.dataset.unused === 'true' };
      const text = resetText(value, now, el.hasAttribute('data-short'));if (el.textContent !== text) el.textContent = text;
    }
    for (const tile of container.querySelectorAll('button[data-quota-tile]')) {
      const quota = hooks.quota?.(tile.dataset.quotaAccount),m = quota && models(quota).find((row) => row.id === tile.dataset.quotaTile);
      if (!m) continue;
      const text = label(m, now);if (tile.getAttribute('aria-label') !== text) {tile.setAttribute('aria-label', text);tile.title = text;}
    }
  }
  function toggle(tile) {
    const container = tile.closest('.quota-rings'),id = tile.dataset.quotaTile;
    container.querySelector('[data-ring-detail]')?.remove();
    container.querySelectorAll('[data-quota-tile][aria-expanded=true]').forEach((b) => b.setAttribute('aria-expanded', 'false'));
    if (open === id) {open = '';return;}
    const quota = hooks.quota?.(tile.dataset.quotaAccount);
    if (!quota || !hooks.card) {open = '';return;}
    open = id;tile.setAttribute('aria-expanded', 'true');
    tile.insertAdjacentHTML('afterend', hooks.card(quota, models(quota).find(m => m.id === id)));
    hooks.bind?.();
    const card = tile.nextElementSibling;if (card) globalThis.AieyesMotion?.enter(card, 'disclosure');
  }
  if (globalThis.document) {
    document.addEventListener('click', (event) => {
      if (!(event.target instanceof Element)) return;
      const choice = event.target.closest('[data-quota-style]');
      if (choice) {const value = choice.dataset.quotaStyle;if (value !== style()) {setStyle(surface(), value);open = '';hooks.render?.();}return;}
      const tile = event.target.closest('.quota-rings > button[data-quota-tile]');
      if (tile) {toggle(tile);return;}
      // The open card's own title collapses it back into its tile.
      const summary = event.target.closest('.quota-rings > [data-ring-detail] .quota-disclosure > summary');
      if (summary) {const owner = grid()?.querySelector(`[data-quota-tile="${CSS.escape(open)}"]`);if (owner) {toggle(owner);owner.focus({ preventScroll: true });}}
    });
  }

  globalThis.AieyesQuotaRings = {
    configure(next) {hooks = { ...hooks, ...next };},
    style, setStyle, keys: KEYS, styles: STYLES,
    active() {return Boolean(grid());},
    switchHTML, gridHTML, replace, refreshTimes,
    // Exposed for tests: the same rules as quota_ring.rs.
    qoderCreditSum, ringSVG, shortName, pick, ring, models, compactMinutes, resetText, level, groupsWindows,
  };
})();
