/* Shared UI semantics. Query timestamps deliberately never imply a successful sync. */
window.AieyesUI = {
  panelAccounts(settings, order = []) {
    let selected = {};
    try { const value=JSON.parse(localStorage.getItem('aieyes.panel.accounts.v1')||'{}');if(value&&typeof value==='object'&&!Array.isArray(value))selected=value; } catch (_) {}
    const eligible=(settings?.accounts??[]).filter(a=>!a.archived&&a.quotaEnabled);
    const sorted=[...eligible].sort((a,b)=>{const rank=a=>{const i=order.indexOf(a.provider+':'+a.id);return i<0?Number.MAX_SAFE_INTEGER:i;};return rank(a)-rank(b);});
    for(const provider of new Set([...eligible.map(a=>a.provider),...Object.keys(selected)])) {
      const keys=sorted.filter(a=>a.provider===provider).map(a=>a.provider+':'+a.id);
      selected[provider]=[...new Set((Array.isArray(selected[provider])?selected[provider]:keys).filter(k=>keys.includes(k)))].slice(0,5);
    }
    try { localStorage.setItem('aieyes.panel.accounts.v1',JSON.stringify(selected)); } catch (_) {}
    return selected;
  },
  usageSources(settings) {
    return (settings?.sources ?? []).filter(s => s.enabled && !['agy','deepseek'].includes(s.provider) &&
      (!s.hostId || settings.hosts.some(h => h.id === s.hostId && h.enabled)));
  },
  freshness(settings, dashboard, now = Date.now() / 1000) {
    const sources = this.usageSources(settings);
    const statuses = sources.map(s => dashboard?.sources?.find(row => row.id === s.id)?.status);
    const stamps = statuses.map(s => s?.updatedAt ?? 0);
    return {
      total: sources.length,
      // The oldest source is the only defensible timestamp for the entire record set.
      timestamp: stamps.length && stamps.every(Boolean) ? Math.min(...stamps) : 0,
      delayed: statuses.filter(s => !s?.updatedAt || now - s.updatedAt > Math.max(60, settings.refreshSeconds * 2)).length,
      failed: statuses.filter(s => s?.error).length
    };
  },
  resetDisplayTime(stamp, windowMinutes, now = Date.now() / 1000) {
    if (Number.isFinite(stamp) && stamp > now) return stamp;
    return Number.isFinite(windowMinutes) && windowMinutes > 0 ? now + windowMinutes * 60 : null;
  },
  resetText(stamp, now = Date.now() / 1000, windowMinutes) {
    const target = this.resetDisplayTime(stamp, windowMinutes, now);
    if (target == null) return Number.isFinite(stamp) && stamp > 0 ? '确认重置中' : '重置时间未知';
    const minutes = Math.ceil((target - now) / 60), parts = [];
    if (!Number.isSafeInteger(minutes) || minutes <= 0) return '重置时间未知';
    if (minutes >= 1440) parts.push(Math.floor(minutes / 1440) + ' 天');
    if (minutes % 1440 >= 60) parts.push(Math.floor(minutes % 1440 / 60) + ' 小时');
    if (minutes % 60) parts.push(minutes % 60 + ' 分');
    return parts.join(' ') + '后重置';
  },
  quotaReset(window, now = Date.now() / 1000) {
    const text = this.resetText(window.resetsAt, now, window.windowMinutes);
    const stamp = this.resetDisplayTime(window.resetsAt, window.windowMinutes, now), date = new Date(stamp * 1000);
    if (stamp == null || !Number.isFinite(date.getTime())) return text;
    const pad = n => String(n).padStart(2, '0');
    return text + ' · ' + pad(date.getMonth()+1) + '/' + pad(date.getDate()) + ' ' + pad(date.getHours()) + ':' + pad(date.getMinutes());
  },
  subscription(plan) {
    const name = String(plan ?? '').trim();
    return ({plus:'Plus',pro:'Pro',free:'Free',max:'Max',team:'Team',business:'Business',enterprise:'Enterprise',api:'API'})[name.toLowerCase()] ?? name;
  },
  estimateLabel(record) {
    if (!record) return '';
    const five = record.valuationMode === 'fiveHour';
    const value = five ? record.fiveHourValue : record.weeklyValue;
    return (five ? '5h' : '7d 整周') + (value == null ? ' · 样本积累中' : ' ≈ $' + value.toFixed(2) + ' USD');
  },
  targetTimeLocal(time, zone, now = new Date()) {
    if (!zone) return '目标时区待读取';
    if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(time)) return '请输入 HH:mm';
    try {
      const formatter = new Intl.DateTimeFormat('en-CA', {timeZone:zone, year:'numeric', month:'2-digit', day:'2-digit', hour:'2-digit', minute:'2-digit', hourCycle:'h23'});
      const parts = date => Object.fromEntries(formatter.formatToParts(date).filter(p=>p.type!=='literal').map(p=>[p.type,Number(p.value)]));
      const p=parts(now),[hour,minute]=time.split(':').map(Number);
      for(let day=0;day<3;day++) {
        const target=Date.UTC(p.year,p.month-1,p.day+day,hour,minute);let candidate=target;
        for(let i=0;i<4;i++){const z=parts(new Date(candidate));candidate=target-(Date.UTC(z.year,z.month-1,z.day,z.hour,z.minute)-candidate);}
        const actual=parts(new Date(candidate));
        if(candidate>now.getTime()&&actual.hour===hour&&actual.minute===minute)return '本地 '+new Date(candidate).toLocaleString(undefined,{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'});
      }
      return '时区换算待确认，以部署状态为准';
    } catch (_) { return '目标时区 '+zone+'；以部署状态为准'; }
  },
  formValues(form) {
    return Object.fromEntries([...form.querySelectorAll('input[name],select[name],textarea[name]')].map(el =>
      [el.name, el.type === 'checkbox' ? el.checked : el.value]));
  },
  restoreForm(form, values) {
    for (const [name,value] of Object.entries(values ?? {})) {
      const el = form.elements.namedItem(name);
      if (!el) continue;
      if (el.type === 'checkbox') el.checked = value; else el.value = value;
    }
  }
};
