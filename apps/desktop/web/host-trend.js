/* Server minute history (hosts.history) drawn as small trend lines on the server cards.
   Each line covers a fixed 60-minute window that ends at the host's latest minute, on a fixed
   0–100 scale like the bar below it. Readings further apart than the sampling allows are not
   joined (a pause shows as a break, never as a drop), a minute without the metric is skipped
   (never drawn as 0), and the newest reading carries a dot while it is current. */
(() => {
  // app.js declares `state` with const: a global binding, not a property of globalThis.
  const pageState = () => typeof state === 'undefined' ? undefined : state;
  const WINDOW = 60,history = new Map();
  let revision = 0,loading = null,loadedAt = 0;
  const finite = (value) => typeof value === 'number' && Number.isFinite(value);
  const clamp = (value) => Math.min(100, Math.max(0, value));
  // Sampling slower than once a minute leaves minutes without a reading; those still join.
  function gap(settings = pageState()?.settings) {
    const foreground = Number(settings?.serverForegroundRefreshSeconds) || 2,background = Number(settings?.serverRefreshSeconds) || 10;
    return Math.max(150, 2.5 * Math.max(foreground, background));
  }
  function series(minutes = [], metric = 'cpu', gapSeconds = gap()) {
    const times = minutes.map((m) => m?.t).filter(finite);
    if (!times.length) return { points: [], start: 0, end: 0, gap: gapSeconds, peak: null };
    const end = Math.max(...times),start = end - (WINDOW - 1) * 60,points = [];let peak = null;
    for (const minute of minutes) {
      if (!finite(minute?.t) || minute.t < start) continue;
      const value = minute[metric];if (!finite(value)) continue;
      const high = metric === 'cpu' && finite(minute.cpuMax) ? minute.cpuMax : value;
      points.push({ t: minute.t, value: clamp(value) });peak = Math.max(peak ?? 0, clamp(high));
    }
    points.sort((a, b) => a.t - b.t);
    return { points, start, end, gap: gapSeconds, peak };
  }
  const hasLine = (s) => s.points.some((point, i) => i > 0 && point.t - s.points[i - 1].t <= s.gap);
  const latest = (s) => {const point = s.points.at(-1);return point && s.end - point.t <= s.gap ? point : null;};
  function summary(s) {
    if (!s.points.length || s.peak == null) return '';
    const mean = s.points.reduce((sum, p) => sum + p.value, 0) / s.points.length,span = Math.floor((s.points.at(-1).t - s.points[0].t) / 60) + 1;
    return AieyesI18n.t("{0} 分钟走势 · 均值 {1}% · 峰值 {2}%", span, mean.toFixed(0), s.peak.toFixed(0));
  }
  // viewBox units: 100 wide (stretched to the slot), 20 high (drawn at 20 px). Strokes do not scale,
  // so the line stays 1.5 px and the dots, zero-length round-capped segments, stay round.
  function svg(s, className = '') {
    const W = 100,H = 20,inset = 2.5,span = s.end - s.start;
    if (!(span > 0)) return '';
    const x = (p) => (inset + (p.t - s.start) / span * (W - inset * 2)).toFixed(2),y = (p) => (H - inset - p.value / 100 * (H - inset * 2)).toFixed(2);
    const runs = [];let run = [];
    for (const point of s.points) {if (run.length && point.t - run.at(-1).t > s.gap) {runs.push(run);run = [];}run.push(point);}
    if (run.length) runs.push(run);
    const line = runs.map((r) => `M${x(r[0])} ${y(r[0])}L${x(r[0])} ${y(r[0])}` + r.slice(1).map((p) => `L${x(p)} ${y(p)}`).join('')).join('');
    const area = runs.filter((r) => r.length > 1).map((r) => `M${x(r[0])} ${H - inset}` + r.map((p) => `L${x(p)} ${y(p)}`).join('') + `L${x(r.at(-1))} ${H - inset}Z`).join('');
    const dot = latest(s),label = AieyesUI.escapeHTML(summary(s));
    return `<svg class="host-trend${className ? ' ' + className : ''}" viewBox="0 0 ${W} ${H}" preserveAspectRatio="none" role="img" aria-label="${label}"><title>${label}</title>${area ? `<path class="host-trend-area" d="${area}"/>` : ''}<path class="host-trend-line" d="${line}"/>${dot ? `<path class="host-trend-dot" d="M${x(dot)} ${y(dot)}L${x(dot)} ${y(dot)}"/>` : ''}</svg>`;
  }
  // The trend for one tile or GPU header, or '' when there is no line to draw yet.
  function tile(hostId, metric, className = '') {
    const s = series(history.get(hostId) ?? [], metric);
    return hasLine(s) ? svg(s, className) : '';
  }
  // Fetches the minute history at most every 20 s (a minute changes once a minute); a change
  // repaints the server cards.
  async function refresh(force = false) {
    if (loading || !force && Date.now() - loadedAt < 20000 || typeof api !== 'function') return loading;
    loading = api('hosts.history', {}).then((value) => {
      loadedAt = Date.now();let changed = false;
      const seen = new Set();
      for (const row of value?.hosts ?? []) {
        if (!row?.id) continue;seen.add(row.id);
        const minutes = Array.isArray(row.minutes) ? row.minutes : [];
        if (JSON.stringify(history.get(row.id)) !== JSON.stringify(minutes)) {history.set(row.id, minutes);changed = true;}
      }
      for (const id of [...history.keys()]) if (!seen.has(id)) {history.delete(id);changed = true;}
      if (changed) {revision++;if (pageState()?.page === 'servers') globalThis.renderServers?.();}
    }).catch(() => {}).finally(() => {loading = null;});
    return loading;
  }
  globalThis.AieyesHostTrend = { gap, series, hasLine, latest, summary, svg, tile, refresh, history, get revision() {return revision;} };
})();
