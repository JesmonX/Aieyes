/* Shared UI semantics. Query timestamps deliberately never imply a successful sync. */
window.AieyesUI = {
  panelPreference() {
    try {
      const current = JSON.parse(localStorage.getItem('aieyes.panel.accounts.v2') || 'null');
      if (current?.version === 2 && current.providers) return current;
      const old = JSON.parse(localStorage.getItem('aieyes.panel.accounts.v1') || '{}');
      const next = {version:2, providers:{}};
      for (const [provider,keys] of Object.entries(old || {})) if (Array.isArray(keys)) next.providers[provider] = {mode:'custom',keys};
      localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify(next));
      return next;
    } catch (_) { return {version:2,providers:{}}; }
  },
  panelAccounts(settings, order = [], preference = this.panelPreference()) {
    const selected = {}, eligible=(settings?.accounts??[]).filter(a=>!a.archived&&a.quotaEnabled);
    const rank=a=>{const i=order.indexOf(a.provider+':'+a.id);return i<0?Number.MAX_SAFE_INTEGER:i;};
    const sorted=[...eligible].sort((a,b)=>rank(a)-rank(b));
    for(const provider of new Set([...eligible.map(a=>a.provider),...Object.keys(preference.providers)])) {
      const keys=sorted.filter(a=>a.provider===provider).map(a=>a.provider+':'+a.id), choice=preference.providers[provider];
      const saved=Array.isArray(choice?.keys)?choice.keys:[];
      let values=choice?.mode==='custom'?saved.filter(k=>keys.includes(k)):keys;
      if(choice?.mode==='custom'&&saved.length&&!values.length)values=keys;
      selected[provider]=[...new Set(values)].slice(0,5);
    }
    return selected;
  },
  canonicalEstimates(records = []) {
    const replaced = new Set(records.map(e=>e.originalEstimateId).filter(Boolean));
    return records.filter(e=>!replaced.has(e.id)).sort((a,b)=>b.startedAt-a.startedAt || (b.repairedAt||0)-(a.repairedAt||0));
  },
  estimateIssue(record) {
    if (record.status==='pending') return record.reason || '待确认';
    return ({boundary:'用量边界待确认',noUsage:'暂无有效用量',unpriced:'模型价格不完整',insufficientUsage:'尚未达到采样阈值',invalidPrice:'价格数据无效'})[record.calculationStatus] || record.calculationNote || '尚无估值';
  },
  estimateEntries(records = [], credits = false) {
    const all=this.canonicalEstimates(records);
    return (credits ? [['valuePer1000','1000 credits']] : [['fiveHourValue','5h'],['weeklyValue','7d 整周'],['weeklyRatioValue','7d 容量倍率']]).flatMap(([field,label])=>{
      const record=all.find(e=>e.status!=='pending'&&Number.isFinite(e[field]));
      return record?[{field,label:field==='weeklyValue'&&record.valuationMode==='fiveHour'?'7d 同期':label,value:record[field],record,historical:record!==all[0]||record.status==='completed'}]:[];
    });
  },
  usageSources(settings) {
    return (settings?.sources ?? []).filter(s => s.enabled && s.provider!=='deepseek' &&
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
  resetDisplayTime(stamp, now = Date.now() / 1000) {
    // A zero usage percentage does not establish whether a window has started.
    // Only the source timestamp can establish its next reset.
    return Number.isFinite(stamp) && stamp > 0 && stamp > now ? stamp : null;
  },
  resetText(stamp, now = Date.now() / 1000) {
    const target = this.resetDisplayTime(stamp, now);
    if (target == null) return Number.isFinite(stamp) && stamp > 0 ? '确认重置中' : '重置时间未知';
    const minutes = Math.ceil((target - now) / 60), parts = [];
    if (!Number.isSafeInteger(minutes) || minutes <= 0) return '重置时间未知';
    if (minutes >= 1440) parts.push(Math.floor(minutes / 1440) + ' 天');
    if (minutes % 1440 >= 60) parts.push(Math.floor(minutes % 1440 / 60) + ' 小时');
    if (minutes < 1440 && minutes % 60) parts.push(minutes % 60 + ' 分');
    return parts.join(' ') + '后重置';
  },
  quotaReset(window, now = Date.now() / 1000) {
    if (!(Number.isFinite(window.resetsAt) && window.resetsAt > 0) && window.usedPercent === 0) {
      if (window.windowMinutes === 300) return '5h';
      if (window.windowMinutes === 10080) return '7d';
    }
    const text = this.resetText(window.resetsAt, now);
    const stamp = this.resetDisplayTime(window.resetsAt, now), date = new Date(stamp * 1000);
    if (stamp == null || !Number.isFinite(date.getTime())) return text;
    const pad = n => String(n).padStart(2, '0');
    return text + ' · ' + pad(date.getMonth()+1) + '/' + pad(date.getDate()) + ' ' + pad(date.getHours()) + ':' + pad(date.getMinutes());
  },
  subscription(plan) {
    const name = String(plan ?? '').trim();
    return ({plus:'Plus',pro:'Pro',free:'Free',max:'Max',team:'Team',business:'Business',enterprise:'Enterprise',api:'API'})[name.toLowerCase()] ?? (name ? name[0].toLocaleUpperCase()+name.slice(1) : '');
  },
  compactMoney(value) {
    if (!Number.isFinite(value)) return '—';
    for (const [size,suffix] of [[1e12,'T'],[1e9,'B'],[1e6,'M'],[1e3,'K']]) if (Math.abs(value)>=size*0.999995) return '$'+(value/size).toFixed(2)+suffix;
    return '$'+value.toFixed(2);
  },
  estimateWindows(q) {
    const windows=q.windows??[];
    if(['antigravity','agy'].includes(q.provider)) {
      const groups=[...new Set(windows.map(w=>w.groupId||w.groupName||''))];
      return groups.filter(g=>['gemini','gemini models','claude-gpt','claude and gpt models'].includes(g.toLowerCase())).flatMap(g=>{
        const rows=windows.filter(w=>(w.groupId||w.groupName)===g);
        const five=rows.filter(w=>w.windowMinutes===300),week=rows.filter(w=>w.windowMinutes===10080);
        return five.length===1?five:!five.length&&week.length===1?week:[];
      });
    }
    const five=windows.filter(w=>w.windowMinutes===300),week=windows.filter(w=>w.windowMinutes===10080);
    return five.length===1?five:q.provider==='claude'&&five.some(w=>w.id==='five_hour')?five.filter(w=>w.id==='five_hour'):week.length===1?week:week.filter(w=>q.provider==='claude'&&w.id==='seven_day');
  },
  estimateLine(records=[]) {
    const entries=this.estimateEntries(records), value=field=>entries.find(e=>e.field===field)?.value;
    return '5h ≈ '+this.compactMoney(value('fiveHourValue'))+'  7d ≈ '+this.compactMoney(value('weeklyRatioValue'))+'/'+this.compactMoney(value('weeklyValue'));
  },
  migrateProviderPreferences(settings) {
    const aliases=settings?.accountAliases??{}, key=s=>aliases[s]??(s.startsWith('agy:')?'antigravity:'+s.slice(4):s);
    try {
      const marker=JSON.stringify(aliases); if(localStorage.getItem('aieyes.antigravity.preferences')===marker)return;
      const pref=this.panelPreference(), old=pref.providers.agy;
      if(old){const current=pref.providers.antigravity;pref.providers.antigravity=current?{mode:current.mode==='custom'||old.mode==='custom'?'custom':'auto',keys:[...new Set([...(current.keys??[]),...(old.keys??[])].map(key))].slice(0,5)}:{...old,keys:(old.keys??[]).map(key)};delete pref.providers.agy;localStorage.setItem('aieyes.panel.accounts.v2',JSON.stringify(pref));}
      for(const name of Object.keys(localStorage)){if(name.startsWith('quota.expanded.v2.')&&name.includes('.agy:')){const i=name.indexOf('.agy:')+1,next=name.slice(0,i)+key(name.slice(i));if(localStorage.getItem(next)==null)localStorage.setItem(next,localStorage.getItem(name));}}
      const view=JSON.parse(localStorage.getItem('aieyes.panel.view')||'{}');if(view.provider==='agy')view.provider='antigravity';if(view.accountKey)view.accountKey=key(view.accountKey);localStorage.setItem('aieyes.panel.view',JSON.stringify(view));
      localStorage.setItem('aieyes.antigravity.preferences',marker);
    } catch(_) {}
  },
  estimateLabel(record) {
    if (!record) return '';
    const five = record.valuationMode === 'fiveHour';
    const value = five ? record.fiveHourValue : record.weeklyValue;
    return (five ? '5h' : '7d 整周') + (value == null ? ' · ' + this.estimateIssue(record) : ' ≈ $' + value.toFixed(2) + ' USD');
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
