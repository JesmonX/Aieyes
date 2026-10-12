/* Shared UI semantics. Query timestamps deliberately never imply a successful sync. */
window.AieyesUI = {
  canonicalProvider(provider) { return provider === 'agy' ? 'antigravity' : provider; },
  canonicalAccountKey(key, aliases = {}) { return aliases[key] ?? (key?.startsWith('agy:') ? 'antigravity:' + key.slice(4) : key); },
  canonicalPayload(value, aliases = value?.accountAliases ?? {}) {
    if (Array.isArray(value)) return value.map(row => this.canonicalPayload(row, aliases));
    if (!value || typeof value !== 'object') return value;
    const next = Object.fromEntries(Object.entries(value).map(([key,row]) => [key,this.canonicalPayload(row,aliases)]));
    if (next.provider === 'agy') {
      const field = 'accountId' in next ? 'accountId' : 'quotaEnabled' in next ? 'id' : null;
      if (field) next[field] = this.canonicalAccountKey('agy:' + next[field], aliases).slice('antigravity:'.length);
      next.provider = 'antigravity';
    }
    if (next.accountKey) next.accountKey = this.canonicalAccountKey(next.accountKey, aliases);
    if (next.quotaOrder) next.quotaOrder = next.quotaOrder.map(key => this.canonicalAccountKey(key, aliases));
    return next;
  },
  escapeHTML(value) { return String(value ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c])); },
  normalizePanelPreference(value) {
    const next = {version:2, providers:{}};
    const object = row => row && typeof row === 'object' && !Array.isArray(row);
    const keys = row => Array.isArray(row) && row.every(key => typeof key === 'string');
    if (!object(value)) return next;
    if ('version' in value || 'providers' in value) {
      if (value.version !== 2 || !object(value.providers)) return next;
      for (const [provider,choice] of Object.entries(value.providers)) {
        next.providers[provider] = choice?.mode === 'custom' && keys(choice.keys)
          ? {mode:'custom', keys:[...choice.keys]} : {mode:'auto', keys:[]};
      }
    } else {
      for (const [provider,choice] of Object.entries(value)) if (keys(choice)) next.providers[provider] = {mode:'custom', keys:[...choice]};
    }
    return next;
  },
  decodePanelPreference(raw) {
    try { return this.normalizePanelPreference(JSON.parse(raw)); }
    catch (_) { return {version:2, providers:{}}; }
  },
  panelPreference() {
    try {
      const current = localStorage.getItem('aieyes.panel.accounts.v2');
      if (current !== null) return this.decodePanelPreference(current);
      const next = this.decodePanelPreference(localStorage.getItem('aieyes.panel.accounts.v1') || '{}');
      localStorage.setItem('aieyes.panel.accounts.v2', JSON.stringify(next));
      return next;
    } catch (_) { return {version:2,providers:{}}; }
  },
  panelAccounts(settings, order = [], preference = this.panelPreference()) {
    preference = this.normalizePanelPreference(preference);
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
  sourceEnabled(settings, source) {
    const agents = (settings?.agents ?? []).filter(agent => agent.provider === source.provider);
    return source.enabled && agents.length <= 1 &&
      (!agents.length || (agents[0].enabled && agents[0].machineIds.includes(source.hostId ?? 'local'))) &&
      (source.hostId == null || (settings?.hosts ?? []).some(host => host.id === source.hostId && host.enabled));
  },
  usageSources(settings) {
    return (settings?.sources ?? []).filter(source => source.provider !== 'deepseek' && this.sourceEnabled(settings, source));
  },
  estimateSources(settings, quota) {
    const account=(settings?.accounts??[]).find(a=>a.provider===quota.provider&&a.id===quota.accountId);
    if(!account)return [];
    const profiles=[account.quotaProfileId,...(account.connections??[]).map(c=>c.profileId)].filter(Boolean);
    return this.usageSources(settings).filter(s=>s.provider===account.provider &&
      (s.accountId===account.id || s.codexHomeId&&profiles.some(r=>r.startsWith(s.codexHomeId+':'))));
  },
  freshness(settings, dashboard, now = Date.now() / 1000) {
    const sources = this.usageSources(settings);
    const statuses = sources.map(s => dashboard?.sources?.find(row => row.id === s.id)?.status);
    const stamps = statuses.map(s => s?.updatedAt ?? 0);
    return {
      total: sources.length,
      // The oldest source is the only defensible timestamp for the entire record set.
      timestamp: stamps.length && stamps.every(Boolean) ? Math.min(...stamps) : 0,
      delayed: statuses.filter(s => !s?.updatedAt || now - s.updatedAt > Math.max(60, (settings.historyRefreshSeconds ?? settings.refreshSeconds) * 2)).length,
      failed: statuses.filter(s => s?.error || s?.partial).length
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
  quotaReset(quotaWindow, now = Date.now() / 1000) {
    if (!(Number.isFinite(quotaWindow.resetsAt) && quotaWindow.resetsAt > 0) && quotaWindow.usedPercent === 0) {
      if (quotaWindow.windowMinutes===window.AieyesContract.windows.fiveHour) return '5h';
      if (quotaWindow.windowMinutes===window.AieyesContract.windows.week) return '7d';
    }
    const text = this.resetText(quotaWindow.resetsAt, now);
    const stamp = this.resetDisplayTime(quotaWindow.resetsAt, now), date = new Date(stamp * 1000);
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
    if(q.provider==='antigravity') {
      const groups=[...new Set(windows.map(w=>w.groupId||w.groupName||''))];
      return groups.filter(g=>['gemini','gemini models','claude-gpt','claude and gpt models'].includes(g.toLowerCase())).flatMap(g=>{
        const rows=windows.filter(w=>(w.groupId||w.groupName)===g);
        const five=rows.filter(w=>w.windowMinutes===window.AieyesContract.windows.fiveHour),week=rows.filter(w=>w.windowMinutes===window.AieyesContract.windows.week);
        return five.length===1?five:!five.length&&week.length===1?week:[];
      });
    }
    const five=windows.filter(w=>w.windowMinutes===window.AieyesContract.windows.fiveHour),week=windows.filter(w=>w.windowMinutes===window.AieyesContract.windows.week);
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
  nextTargetTime(time, zone, now = new Date()) {
    zone = zone?.trim();
    if (!zone) return {status:'pending',date:null,approximate:false};
    if (typeof time!=='string'||time.length!==5||!/^([01]\d|2[0-3]):[0-5]\d$/.test(time)) return {status:'invalidTime',date:null,approximate:false};
    try {
      let parts, approximate=false;
      if(zone.includes('/')||zone==='UTC'||zone==='GMT') {
        const formatter = new Intl.DateTimeFormat('en-GB', {timeZone:zone,hour:'2-digit',minute:'2-digit',hourCycle:'h23'});
        parts = date => Object.fromEntries(formatter.formatToParts(date).filter(p=>p.type!=='literal').map(p=>[p.type,Number(p.value)]));
      } else {
        const offset=zone.match(/(?:^|\s)(?:UTC|GMT)?([+-])(\d{2}):?(\d{2})$/);
        if(!offset||Number(offset[3])>=60||Number(offset[2])*60+Number(offset[3])>18*60)return {status:'invalidZone',date:null,approximate:false};
        const minutes=(Number(offset[2])*60+Number(offset[3]))*(offset[1]==='-'?-1:1);
        parts = date => {const local=new Date(date.getTime()+minutes*60_000);return {hour:local.getUTCHours(),minute:local.getUTCMinutes()};};
        approximate=true;
      }
      const [hour,minute]=time.split(':').map(Number), start=Math.floor(now.getTime()/60_000)*60_000;
      if(!Number.isFinite(start))return {status:'unavailable',date:null,approximate:false};
      for(let i=1;i<=2880;i++) {
        const date=new Date(start+i*60_000), actual=parts(date);
        if(actual.hour===hour&&actual.minute===minute)return {status:'ready',date,approximate};
      }
      return {status:'unavailable',date:null,approximate:false};
    } catch (_) { return {status:'invalidZone',date:null,approximate:false}; }
  },
  targetTimeLocal(time, zone, now = new Date()) {
    const next=this.nextTargetTime(time,zone,now);
    if(next.date)return '本地 '+next.date.toLocaleString(undefined,{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'})+(next.approximate?'（按当前偏移估算）':'');
    return ({pending:'目标时区待读取',invalidTime:'请输入 HH:mm'})[next.status]??'时区换算待确认，以部署状态为准';
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
