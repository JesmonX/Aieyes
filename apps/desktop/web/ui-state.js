/* Shared UI semantics. Query timestamps deliberately never imply a successful sync. */
window.AieyesUI = {
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
  resetText(stamp, now = Date.now() / 1000) {
    if (!stamp) return '重置时间未知';
    if (stamp <= now) return '确认重置中';
    const minutes = Math.ceil((stamp - now) / 60), hours = Math.floor(minutes / 60);
    return (hours ? hours + ' 小时 ' : '') + minutes % 60 + ' 分后重置';
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
