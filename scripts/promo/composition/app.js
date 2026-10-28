// Aieyes promo composition. Every frame is a pure function of time: renderAt(t).
const params = new URLSearchParams(location.search);
const LANG = params.get('lang') || 'zh-CN', ZH = LANG === 'zh-CN';
// ?theme=light: light native renders, light stage, and accents deep enough to read on white.
const LIGHT = params.get('theme') === 'light';
if (LIGHT) document.documentElement.dataset.theme = 'light';
const RECOLOR = {"#ebc198": "#c8742a", "#8ddbdc": "#138f93", "#b9a4ff": "#7a5af5", "#eeb3cf": "#cf4f8b", "#9ad6bb": "#23905f", "#94b9ff": "#3a6fdc", "rgba(255,255,255,.14)": "rgba(0,0,0,.12)", "rgba(0,0,0,.6)": "rgba(40,44,70,.18)", "rgba(0,0,0,.55)": "rgba(40,44,70,.16)", "#d6d8e2": "#2b2e3a", "#cfd2e0": "#3b3e4c", "#9ea3b6": "#686d7e", "#7d8196": "#7b8092", "#4b4f60": "#c3c6d0", "#1e1e1e": "#ffffff", "#2a2a2a": "#ffffff", "rgba(44,44,48,.96)": "rgba(246,246,248,.97)", "#ececec": "#1d1d1f", "rgba(255,255,255,.18)": "rgba(0,0,0,.1)", "rgba(255,255,255,.12)": "rgba(0,0,0,.1)", "rgba(255,255,255,.1)": "rgba(0,0,0,.08)", "rgba(255,255,255,.05)": "rgba(255,255,255,.7)", "rgba(255,255,255,.04)": "rgba(0,0,0,.03)", "rgba(255,255,255,.85)": "rgba(30,30,40,.55)", "rgba(255,255,255,.8)": "rgba(30,30,40,.55)", "rgba(238,179,207,.9)": "rgba(207,79,139,.8)", "rgba(235,193,152,.3)": "rgba(200,116,42,.35)", "rgba(235,193,152,.07)": "rgba(200,116,42,.07)", "rgba(160,165,185,.75)": "rgba(120,126,145,.45)", "rgba(154,214,187,.22)": "rgba(35,144,95,.12)", "rgba(141,219,220,.5)": "rgba(19,143,147,.45)", "#f3d7b4": "#f0c08c", "#e8f6ef": "#1d5a40", "#e59a9a": "#b9483f", "#3a2c2c": "#f6dcdc", "#2b2224": "#fbeeee", "#8a4f4f": "#dea3a3", "#e4e6f0": "#1d1f29", "#c9eeee": "#0f6e6e", "#c7cad8": "#4a4e5c", "#7b8095": "#878b9b", "#6d7183": "#b9bdcb", "#666b7e": "#9a9eae", "#3b2f5c": "#d9d0fb", "#1d3a55": "#cfe4f8", "#141826": "#f1f3f9", "#0c0e15": "#e2e6f0", "#16181f": "#ffffff", "#1f2129": "#efeff2", "#0d1512": "#ffffff", "#c792ea": "#8e44ad", "#82aaff": "#2f6fd6", "#c3e88d": "#3d8b37", "#f78c6c": "#d0602e", "#89ddff": "#1c8ea8", "#5c6370": "#a0a6b0"};
const K = c => LIGHT ? (RECOLOR[c] ?? c) : c;
// ?font=pingfang swaps the website's brand fonts for the macOS system typeface.
if (params.get('font') === 'pingfang') document.documentElement.style.setProperty('--font', '-apple-system,"SF Pro Display","PingFang SC",sans-serif');
const SHOT = `/.local/promo/native/${LANG}${LIGHT ? '-light' : ''}/`, WEB = '/apps/website/public/';
const DURATION = 99;
const stage = document.getElementById('stage');

const T = (zh, en) => ZH ? zh : en;
const COPY = {
  lines: T(['Agent 越用越多，', '账户越开越多，', '机器越连越多。'], ['More agents.', 'More accounts.', 'More machines.']),
  questions: T(['额度还剩多少？', '该切哪个账户？', '这个套餐值不值？', '额度几点重置？', 'Agent 是不是在等我？', '服务器还好吗？'],
               ['How much quota is left?', 'Which account next?', 'Is my plan worth it?', 'When does it reset?', 'Is an agent waiting on me?', 'Are my servers OK?']),
  accounts: T(['个人工作区 · Plus', '团队工作区 · Pro', 'Claude · Max', 'Gemini 工作区'], ['Personal · Plus', 'Studio · Pro', 'Claude · Max', 'Gemini workspace']),
  devices: T(['MacBook Pro', 'GPU 训练机', '东京 VPS', '家庭 NAS'], ['MacBook Pro', 'GPU Lab', 'Tokyo VPS', 'Home NAS']),
  tagline: T(['所有 AI Agent，', '一眼看清。'], ['Your AI agents.', 'One clear view.']),
  tagSub: T('面向 AI 编程工作流的桌面工作台', 'A desktop command center for your AI coding workflow.'),
  s3: { k: T('01 · 额度聚合', '01 · QUOTAS AT A GLANCE'), q: T('额度还剩多少？该切哪个账户？', 'How much is left? Which account next?'),
        h: T('多 Agent、多账户，\n额度一屏看全。', 'Every agent,\nevery account,\none screen.'),
        c: T(['5h / 7d 剩余额度与重置时间', '剩余额度三档配色：充足 · 偏低 · 告急', '今日 Token 与 API 等价成本', '一键切换已保存的账户，接着原来的对话'],
             ['5h / 7d remaining and reset times', 'Three colors for what is left: plenty, low, critical', "Today's tokens and API value", 'Switch saved accounts, keep the conversation going']) },
  s4: { k: T('02 · 自动估值', '02 · AUTOMATIC VALUATION'), q: T('这个套餐值不值？', 'Is my plan worth it?'),
        h: T('按真实消耗，\n自动估算额度价值。', 'Real usage,\nturned into\nreal value.'),
        c: T(['每个额度窗口的 API 等价价值', '滚动估值：最近 90 天，权重每 7 天减半'], ['API value of every quota window', 'Rolling: last 90 days, weight halves every 7 days']),
        plan: T('订阅费（示例）', 'Subscription (example)'), planV: T('$20 / 月', '$20 / mo'),
        value: T('估算 API 等价价值', 'Estimated API value'), valueV: T('≈ $187 / 月', '≈ $187 / mo'),
        formula: T('7d 额度 ≈ $43.20  ×  52 周 ÷ 12 月', '7d quota ≈ $43.20  ×  52 weeks ÷ 12 months'),
        verdict: T('对比订阅价格，值不值心里有数。', 'Compare it with what you pay. The answer is clear.'),
        note: T('示例数据 · 估值为 API 等价价值，不是可兑换余额', 'Illustrative data · API-equivalent value, not a redeemable balance') },
  s5: { k: T('03 · 定时唤醒', '03 · SCHEDULED WAKEUPS'), q: T('额度几点重置？', 'When does it reset?'),
        h: T('让额度重置，\n落在你需要的时刻。', 'Put quota resets\nwhere you\nneed them.'),
        row1: T('不唤醒：首次提问才开始计时', 'No wakeup: the window starts at your first prompt'),
        row2: T('定时唤醒：06:30 · 11:30 · 16:30', 'Wakeups at 06:30 · 11:30 · 16:30'),
        work: T('工作时段', 'Workday'), used: T('额度用完，等待重置', 'Out of quota, waiting'),
        sum: T('开工前就开始计时，工作时段内多一次满额刷新。', 'The clock starts before you do: one more fresh window in your workday.'),
        c: T(['在本机或 SSH 服务器上运行', '退出 Aieyes 也照常执行', '选用低价模型与推理强度'], ['Runs on this Mac or an SSH host', 'Keeps running after you quit Aieyes', 'Pick a light model and effort']) },
  s6: { k: T('04 · 会话状态与桌宠', '04 · SESSIONS & DESKTOP PET'), q: T('Agent 是不是在等我？', 'Is an agent waiting on me?'),
        h: T('后台运行的 Agent，\n状态不再错过。', 'Background agents,\nnever out of sight.'),
        c: T(['专注手头的工作，Agent 在后台运行', '需要你时，桌宠、菜单栏和提示音第一时间提醒', '点击菜单栏图标，面板随手打开', '本机与 SSH 会话、今日用量、账户额度，都在面板里',
              '切到服务器，资源读数一目了然', '单击桌宠，同样打开面板', '右键桌宠：打开面板、设置、桌宠设置', '桌宠可导入，也能随时切换'],
             ['Stay focused while agents run in the background', 'When one needs you, the pet, menu bar and a chime tell you', 'Click the menu bar icon to open the panel',
              "Local and SSH sessions, today's usage and quotas, all in the panel", 'Switch to Servers for live resource readings', 'Or click the pet to open the same panel',
              'Right-click the pet for the panel, settings and pet settings', 'Import a pet and switch any time']),
        ctx: T(['打开面板', '设置…', '桌宠设置', '检查更新', '隐藏桌宠'], ['Open panel', 'Settings…', 'Pet settings', 'Check for updates', 'Hide pet']),
        pet: T(['进行中', '等待输入', '已完成'], ['Working', 'Needs input', 'Complete']),
        menu: T(['编辑器', '文件', '编辑', '视图', '窗口'], ['Editor', 'File', 'Edit', 'View', 'Window']), clock: T('10月21日 周三  20:01', 'Wed Oct 21  8:01 PM') },
  s7: { k: T('05 · 多服务器监控', '05 · SERVER MONITORING'), q: T('服务器还好吗？', 'Are my servers OK?'),
        h: T('本机、GPU、\nVPS、NAS，\n关键读数一眼看到。', 'Mac, GPU box,\nVPS, NAS.\nKey numbers\nat a glance.'),
        tags: T(['本机', 'GPU 型', 'VPS 型', 'NAS 型'], ['This Mac', 'GPU server', 'VPS', 'NAS']),
        c: T(['4 × H100：利用率 · 显存 · 温度 · 功耗', 'CPU · 内存 · 磁盘 · 流量', '存储卷 · 磁盘 I/O · 网络'],
             ['4 × H100: load, memory, temperature, power', 'CPU, memory, disk and traffic', 'Volumes, disk I/O and network']),
        pick: T('每台主机，按你的偏好选择监控项。', 'Choose what to watch on every host.'),
        hosts: T(['本机', 'GPU 训练机', '东京 VPS', '家庭 NAS'], ['Local', 'GPU Lab', 'Tokyo VPS', 'Home NAS']),
        metrics: T(['CPU', '内存', 'GPU', '文件系统', '磁盘 I/O', '网络'], ['CPU', 'Memory', 'GPU', 'Filesystems', 'Disk I/O', 'Network']) },
  s8: { k: T('还有更多', 'AND MORE'), h: T('每一份用量，\n都有迹可循。', 'See where\nyour usage goes.'),
        sub: T('每日趋势 · 模型分布 · 缓存命中 · 全年热力图', 'Daily trends · Model mix · Cache hits · Year heatmap'),
        h2: T('接入你已经在用的工具。', 'Bring the tools you already use.'),
        chips: T(['用量历史留在本机', '中文 / English', 'macOS · Windows · Linux'], ['Usage history stays local', 'English / 中文', 'macOS · Windows · Linux']) },
  demo: T('界面为合成演示数据', 'Synthetic demo data'),
  credits: T('界面为合成演示数据 · 桌宠原画：月薪喵 · kiffin', 'Synthetic demo data · Pet artwork: Yuexinmiao by kiffin'),
};

// ---------- math & dom helpers ----------
const clamp = (x, a = 0, b = 1) => Math.min(b, Math.max(a, x));
const E = { lin: x => x, out: x => 1 - Math.pow(1 - x, 3), in: x => x * x * x,
  inOut: x => x < .5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2,
  back: x => { const c = 1.5; return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2); } };
const ramp = (t, a, d, e = E.out) => e(clamp((t - a) / d));
const io = (t, a, d, b, d2 = d) => Math.min(ramp(t, a, d), 1 - ramp(t, b, d2, E.inOut));
const mix = (a, b, p) => a + (b - a) * p;
const loads = [];
function el(tag, cls, parent, style = {}, html) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  Object.assign(e.style, style);
  if (html !== undefined) e.innerHTML = html;
  if (parent) parent.appendChild(e);
  if (tag === 'img') loads.push(e);
  return e;
}
function img(parent, src, style = {}) { const e = el('img', '', parent, style); e.src = src; return e; }
function put(e, { x = 0, y = 0, s = 1, o, blur } = {}) {
  e.style.transform = `translate(${x}px,${y}px) scale(${s})`;
  if (o !== undefined) e.style.opacity = o;
  if (blur !== undefined) e.style.filter = blur > .05 ? `blur(${blur}px)` : 'none';
}
const esc = s => s.replace(/&/g, '&amp;').replace(/</g, '&lt;');
function text(parent, str, cls, style = {}) {
  const e = el('div', cls, parent, style);
  const units = ZH ? [...str] : str.split(/(\n|[ \t]+)/).filter(Boolean);
  e._u = units.map(u => u === '\n' ? (el('br', '', e), null) : el('span', 'char', e, {}, esc(u))).filter(Boolean);
  return e;
}
function reveal(e, p, spread = 7) {
  const n = e._u.length;
  e._u.forEach((u, i) => {
    const k = E.out(clamp((p * (n + spread) - i) / spread));
    u.style.opacity = k; u.style.transform = `translateY(${(1 - k) * 22}px)`;
  });
}
function callout(parent, label, color) {
  const c = el('div', 'callout', parent, {}, `<b style="background:${color};box-shadow:0 0 14px ${color}"></b>${esc(label)}`);
  return c;
}
// Callout at (x, y): anchor 'l' = left middle, 'r' = right middle, 'c' = centered.
function show(c, x, y, o, anchor = 'l') {
  const w = c.offsetWidth, h = c.offsetHeight;
  const dx = anchor === 'l' ? 0 : anchor === 'r' ? -w : -w / 2;
  put(c, { x: x + dx, y: y - h / 2 + (1 - o) * 14, o });
}

// A macOS-style window whose content is a native render, with a camera inside.
function win(parent, src, w, h, vw = w, vh = h) {
  const box = el('div', 'window', parent, { width: vw + 'px', height: (vh + 40) + 'px', left: 0, top: 0 });
  const bar = el('div', 'titlebar', box);
  const lights = el('div', 'lights', bar);
  ['#ff5f57', '#febc2e', '#28c840'].forEach(c => el('i', '', lights, { background: c }));
  el('span', '', bar, {}, 'Aieyes');
  const vp = el('div', 'viewport', box, { width: vw + 'px', height: vh + 'px' });
  const world = el('div', 'world', vp, { width: w + 'px', height: h + 'px' });
  const layers = [img(world, src, { width: w + 'px', height: h + 'px', position: 'absolute' })];
  const W = { box, vp, world, w, h, vw, vh, layers, p: { x: 0, y: 0, s: 1 }, c: { tx: 0, ty: 0, s: 1 }, hls: [] };
  W.place = (x, y, s, o = 1, blur = 0) => { W.p = { x, y, s }; put(box, { x, y, s, o, blur }); };
  W.cam = (cx, cy, s) => {
    const tx = vw / 2 - cx * s, ty = vh / 2 - cy * s;
    W.c = { tx, ty, s }; world.style.transform = `translate(${tx}px,${ty}px) scale(${s})`;
    W.hls.forEach(h => { h.style.borderWidth = (3 / s) + 'px'; });
  };
  W.layer = (src2) => { const l = img(world, src2, { width: w + 'px', position: 'absolute', left: 0, top: 0, opacity: 0 }); layers.push(l); return l; };
  W.hl = (x, y, w2, h2, color) => {
    const e = el('div', 'hl', world, { left: x + 'px', top: y + 'px', width: w2 + 'px', height: h2 + 'px', border: `3px solid ${color}` });
    e._color = color; W.hls.push(e); return e;
  };
  W.toStage = (px, py) => [W.p.x + W.p.s * (W.c.tx + px * W.c.s), W.p.y + W.p.s * (40 + W.c.ty + py * W.c.s)];
  W.cam(w / 2, vh / 2, 1);
  return W;
}
function glowHL(e, o) { e.style.opacity = o; e.style.boxShadow = `0 0 ${30 * o}px ${e._color}66, inset 0 0 ${20 * o}px ${e._color}22`; }
// Camera keyframes: [[t, cx, cy, s], ...], scale interpolated geometrically.
function camAt(W, t, keys) {
  let a = keys[0], b = keys[keys.length - 1];
  for (let i = 0; i < keys.length - 1; i++) if (t >= keys[i][0] && t <= keys[i + 1][0]) { a = keys[i]; b = keys[i + 1]; break; }
  if (t < keys[0][0]) b = a; if (t > b[0]) a = b;
  const p = a === b ? 0 : E.inOut(clamp((t - a[0]) / (b[0] - a[0])));
  W.cam(mix(a[1], b[1], p), mix(a[2], b[2], p), Math.exp(mix(Math.log(a[3]), Math.log(b[3]), p)));
}
// Left column copy used by the feature scenes.
function column(root, data, color) {
  const box = el('div', 'abs', root, { left: '80px', top: '170px', width: '470px' });
  const k = text(box, data.k, 'kicker', { color });
  const q = data.q ? text(box, data.q, 'question', { marginTop: '34px' }) : null;
  const h = text(box, data.h, 'head', { marginTop: '18px', fontSize: ZH ? '46px' : '50px' });
  return { box, update(t, end) {
    reveal(k, ramp(t, .2, .8, E.lin)); if (q) reveal(q, ramp(t, .5, 1, E.lin)); reveal(h, ramp(t, .9, 1.3, E.lin));
    box.style.opacity = 1 - ramp(t, end - .5, .5);
  } };
}

// ---------- background ----------
const glows = [0, 1].map(() => el('div', 'glow', stage));
const PALETTE = LIGHT ? { violet: [122, 90, 245], cyan: [19, 143, 147], blue: [58, 111, 220], pink: [207, 79, 139], green: [35, 144, 95], orange: [200, 116, 42] } : { violet: [185, 164, 255], cyan: [141, 219, 220], blue: [148, 185, 255], pink: [238, 179, 207], green: [154, 214, 187], orange: [235, 193, 152] };
const GLOW = [[0, 'blue'], [8.5, 'violet'], [14, 'violet'], [27, 'cyan'], [37, 'orange'], [48, 'pink'], [71, 'green'], [84, 'cyan'], [92, 'violet'], [99, 'violet']];
function background(t) {
  let i = 0; while (i < GLOW.length - 2 && t >= GLOW[i + 1][0]) i++;
  const [ta, ca] = GLOW[i], [tb, cb] = GLOW[i + 1];
  const p = E.inOut(clamp((t - (tb - 1.2)) / 1.2));
  const rgb = PALETTE[ca].map((v, k) => Math.round(mix(v, PALETTE[cb][k], p)));
  const fade = 1 - ramp(t, DURATION - 1, 1, E.lin);
  glows.forEach((g, k) => {
    g.style.background = `radial-gradient(circle, rgba(${rgb},${(k ? .13 : .17) * (LIGHT ? .55 : 1)}) 0%, rgba(${rgb},0) 62%)`;
    const a = t * .11 + k * 2.6;
    put(g, { x: (k ? 1100 : -300) + Math.sin(a) * 160, y: (k ? -200 : 300) + Math.cos(a * 1.3) * 120, o: fade * ramp(t, 0, 1.5) });
  });
}

// ---------- scenes ----------
const scenes = [];
function scene(start, end, build) {
  const root = el('div', 'scene', stage);
  const update = build(root);
  scenes.push({ start, end, root, update });
}
const providers = [['codex', 'Codex'], ['claude', 'Claude Code'], ['antigravity', 'Antigravity'], ['deepseek', 'DeepSeek'], ['zcode', 'ZCode'], ['qoder', 'Qoder'],
  ['grok', 'Grok'], ['cursor', 'Cursor'], ['opencode-dark', 'OpenCode'], ['kimi-dark', 'Kimi Code'], ['codebuddy', 'CodeBuddy'], ['pi', 'Pi'], ['omp-dark', 'Oh My Pi']];
const icon = id => `${WEB}providers/provider-${LIGHT ? id.replace('-dark', '') : id}.png`;
// Monochrome marks are black artwork; show them light on the dark stage, as the website does.
const MONO = new Set(['codex', 'zcode', 'qoder', 'grok', 'cursor']);
const iconStyle = id => MONO.has(id) && !LIGHT ? 'filter:brightness(0) invert(1)' : '';

// S1: the juggling act
scene(0, 8.5, root => {
  const items = [];
  providers.slice(0, 9).forEach(([id, name]) => items.push(`<img src="${icon(id)}" style="${iconStyle(id)}">${name}`));
  COPY.accounts.forEach(a => items.push(`<i style="width:10px;height:10px;border-radius:50%;background:var(--violet)"></i>${esc(a)}`));
  COPY.devices.forEach(d => items.push(`<i style="width:10px;height:10px;border-radius:3px;background:var(--green)"></i>${esc(d)}`));
  const chips = items.map((html, i) => {
    const c = el('div', 'chip', root, {}, html);
    c.querySelectorAll('img').forEach(e => loads.push(e));
    const a = i / items.length * Math.PI * 2 + .3, r = 1 + (i % 3) * .07;
    c._p = [960 + Math.cos(a) * 760 * r, 540 + Math.sin(a) * 390 * r]; c._d = .25 + (i * 7 % items.length) * .16;
    return c;
  });
  const lines = COPY.lines.map((l, i) => text(root, l, 'center head', { top: `${370 + i * 92}px`, fontSize: '72px' }));
  const qs = COPY.questions.map(q => callout(root, q, `${K('#b9a4ff')}`));
  const qpos = [[560, 380], [1360, 380], [520, 540], [1400, 540], [560, 700], [1360, 700]];
  return t => {
    const out = ramp(t, 7.4, 1, E.inOut);
    chips.forEach((c, i) => {
      const o = ramp(t, c._d, .7) * (1 - out) * (t > 4.2 ? .55 : 1);
      const [px, py] = c._p, dx = Math.sin(t * .5 + i) * 18, dy = Math.cos(t * .4 + i * 1.7) * 14;
      put(c, { x: mix(px + dx, 960, out * .6) - c.offsetWidth / 2, y: mix(py + dy, 540, out * .6) - 26, s: mix(.85, 1, ramp(t, c._d, .7, E.back)), o });
    });
    lines.forEach((l, i) => { reveal(l, ramp(t, .5 + i * 1.1, .8, E.lin)); l.style.opacity = 1 - ramp(t, 4.1, .6); l.style.transform = `translateY(${-ramp(t, 4.1, .6) * 40}px)`; });
    qs.forEach((q, i) => show(q, qpos[i][0], qpos[i][1], ramp(t, 4.5 + i * .38, .5) * (1 - out), 'c'));
  };
});

// S2: reveal
scene(8.5, 14, root => {
  const logo = img(root, WEB + 'brand.png', { position: 'absolute', width: '150px', left: '885px', top: '250px' });
  const name = el('div', 'center', root, { top: '420px', fontSize: '40px', letterSpacing: '4px', color: `${K('#cfd2e0')}` }, 'Aieyes');
  const l1 = text(root, COPY.tagline[0], 'center', { top: '500px', fontSize: '92px', fontWeight: 600 });
  const l2 = text(root, COPY.tagline[1], 'center', { top: '610px', fontSize: '92px', fontWeight: 600, color: 'var(--violet)' });
  const sub = text(root, COPY.tagSub, 'center sub', { top: '760px' });
  return t => {
    const out = ramp(t, 4.6, .9, E.inOut);
    put(logo, { s: mix(.6, 1, ramp(t, .1, .9, E.back)), y: -out * 120, o: ramp(t, .1, .6) * (1 - out), blur: (1 - ramp(t, .1, .7)) * 12 });
    put(name, { y: -out * 120, o: ramp(t, .5, .6) * (1 - out) });
    reveal(l1, ramp(t, .8, .8, E.lin)); reveal(l2, ramp(t, 1.5, .8, E.lin)); reveal(sub, ramp(t, 2.4, 1, E.lin));
    [l1, l2, sub].forEach(e => put(e, { y: -out * 120, o: 1 - out }));
  };
});

// S3: quotas at a glance + account switching
scene(14, 27, root => {
  const col = column(root, COPY.s3, 'var(--violet)');
  const W = win(root, SHOT + 'overview-grid.png', 1360, 900);
  const codex = W.layer(SHOT + 'overview-codex.png');
  const cards = [[240, 369, 532, 156], [787, 369, 531, 156], [240, 562, 532, 124], [787, 562, 531, 231]].map(r => W.hl(...r, `${K('#b9a4ff')}`));
  const today = W.hl(240, 216, 1078, 68, `${K('#8ddbdc')}`), sw = W.hl(256, 434, 142, 30, `${K('#94b9ff')}`);
  const c = COPY.s3.c.map((label, i) => callout(root, label, [`${K('#b9a4ff')}`, `${K('#b9a4ff')}`, `${K('#8ddbdc')}`, `${K('#94b9ff')}`][i]));
  const S = win(root, SHOT + 'account-switch.png', 640, 470);
  return t => {
    col.update(t, 13.4);
    const enter = ramp(t, 0, 1.2, E.out), leave = ramp(t, 12.4, .8, E.inOut);
    W.place(560, 110 + (1 - enter) * 160, .97, enter * (1 - leave));
    camAt(W, t, [[0, 680, 450, 1], [4.8, 680, 450, 1], [5.8, 780, 650, 1.45], [7.6, 780, 650, 1.45], [8.4, 680, 300, 1.25], [9.6, 680, 300, 1.25], [10.4, 340, 470, 1.9], [13, 340, 470, 1.9]]);
    cards.forEach((h, i) => glowHL(h, Math.max(io(t, 1.4 + i * .6, .3, 2.2 + i * .6, .4), i >= 2 ? io(t, 5.2, .4, 7.6, .4) : 0)));
    glowHL(today, io(t, 8.5, .3, 9.6, .3));
    codex.style.opacity = ramp(t, 9.5, .5);
    glowHL(sw, io(t, 10.4, .3, 12.6, .3));
    let [x, y] = W.toStage(780, 447); show(c[0], x + 20, y, io(t, 1.8, .4, 4.6, .3));
    [x, y] = W.toStage(780, 793); show(c[1], x, Math.min(y + 40, 1030), io(t, 6, .4, 7.6, .3), 'c');
    [x, y] = W.toStage(680, 284); show(c[2], x, y + 36, io(t, 8.5, .4, 9.6, .3), 'c');
    const pop = ramp(t, 10.8, .6, E.back);
    S.place(1130, 420 + (1 - pop) * 40, mix(.9, 1.1, pop), ramp(t, 10.8, .4) * (1 - leave));
    show(c[3], 1840, 1040, io(t, 11.1, .4, 12.8, .3), 'r');
  };
});

// S4: automatic valuation
scene(27, 37, root => {
  const col = column(root, COPY.s4, 'var(--cyan)');
  const W = win(root, SHOT + 'overview-codex.png', 1360, 1000, 1360, 900);
  const line = W.hl(256, 634, 504, 29, `${K('#8ddbdc')}`), credits = W.hl(256, 673, 504, 68, `${K('#8ddbdc')}`);
  const est = win(root, SHOT + 'estimate.png', 518, 620);
  const box = est.hl(24, 346, 453, 183, `${K('#8ddbdc')}`);
  const c = COPY.s4.c.map(l => callout(root, l, `${K('#8ddbdc')}`));
  // infographic
  const g = el('div', 'abs', root, { left: '600px', top: '250px', width: '1240px' });
  const rows = [[COPY.s4.plan, COPY.s4.planV, 20, `${K('#6d7183')}`], [COPY.s4.value, COPY.s4.valueV, 187, `linear-gradient(90deg,${K('#8ddbdc')},${K('#b9a4ff')})`]].map(([label, value, v, bg], i) => {
    const r = el('div', 'abs', g, { left: 0, top: `${i * 190}px`, width: '1240px' });
    el('div', '', r, { fontSize: '30px', color: `${K('#c7cad8')}` }, esc(label));
    const bar = el('div', 'bar', r, { top: '62px', left: 0, height: '64px', background: bg });
    const num = el('div', 'abs', r, { top: '66px', fontSize: '44px', fontWeight: 600 }, esc(value));
    return { r, bar, num, w: v / 187 * 880 };
  });
  const formula = el('div', 'abs', g, { top: '400px', left: 0, fontSize: '26px', color: `${K('#9ea3b6')}` }, esc(COPY.s4.formula));
  const big = el('div', 'abs', g, { top: '470px', left: 0, fontSize: '120px', fontWeight: 600, color: 'var(--cyan)' }, '≈ 9.4×');
  const verdict = text(g, COPY.s4.verdict, 'abs', { top: '640px', left: 0, fontSize: '36px' });
  const note = el('div', 'abs', g, { top: '705px', left: 0, font: '18px var(--ui)', color: `${K('#7b8095')}` }, esc(COPY.s4.note));
  return t => {
    col.update(t, 9.4);
    const dim = ramp(t, 6.4, .7, E.inOut), leave = ramp(t, 9.2, .7);
    W.place(560, 110 + (1 - ramp(t, 0, 1)) * 60, .97, ramp(t, 0, .8) * mix(1, .12, dim) * (1 - leave), dim * 8);
    camAt(W, t, [[0, 507, 600, 1.5], [1.2, 507, 690, 2.15], [3.2, 507, 690, 2.15], [4.2, 680, 450, 1]]);
    glowHL(line, io(t, 1.4, .3, 3.4, .3)); glowHL(credits, io(t, 2.4, .3, 3.4, .3));
    let [x, y] = W.toStage(507, 634); show(c[0], x, y - 50, io(t, 1.6, .4, 3.3, .3), 'c');
    const pop = ramp(t, 3.6, .7, E.back);
    est.place(1230, 190 + (1 - pop) * 50, 1.12, ramp(t, 3.6, .4) * (1 - dim));
    glowHL(box, io(t, 4.4, .3, 6.2, .3));
    [x, y] = est.toStage(250, 529); show(c[1], x, y + 52, io(t, 4.6, .4, 6.3, .3), 'c');
    g.style.opacity = ramp(t, 6.6, .5) * (1 - leave);
    rows.forEach((r, i) => { const p = ramp(t, 6.9 + i * .5, 1.1, E.inOut); r.bar.style.width = Math.max(4, r.w * p) + 'px'; r.num.style.left = (r.w * p + 28) + 'px'; r.r.style.opacity = ramp(t, 6.7 + i * .5, .4); });
    formula.style.opacity = ramp(t, 7.9, .5);
    put(big, { s: mix(.7, 1, ramp(t, 8.1, .6, E.back)), o: ramp(t, 8.1, .3) }); big.style.transformOrigin = '0 50%';
    reveal(verdict, ramp(t, 8.4, .7, E.lin)); note.style.opacity = ramp(t, 8.6, .5);
  };
});

// S5: scheduled wakeups
scene(37, 48, root => {
  const col = column(root, COPY.s5, 'var(--orange)');
  const tl = el('div', 'abs', root, { left: '600px', top: '210px', width: '1240px', height: '640px' });
  const X = h => (h - 6) / 16 * 1180 + 30;
  const band = el('div', 'abs', tl, { left: X(9) + 'px', width: (X(19) - X(9)) + 'px', top: '40px', height: '470px', background: `${K('rgba(235,193,152,.07)')}`, borderRadius: '14px', border: `1px dashed ${K('rgba(235,193,152,.3)')}` });
  el('div', 'abs', band, { top: '-36px', left: 0, right: 0, textAlign: 'center', fontSize: '22px', color: `${K('#ebc198')}` }, esc(COPY.s5.work));
  for (let h = 6; h <= 22; h += 2) {
    el('div', 'abs', tl, { left: X(h) - 30 + 'px', width: '60px', textAlign: 'center', top: '530px', font: '18px var(--ui)', color: `${K('#7d8196')}` }, `${String(h).padStart(2, '0')}:00`);
    el('div', 'abs', tl, { left: X(h) + 'px', top: '515px', width: '1px', height: '10px', background: `${K('#4b4f60')}` });
  }
  el('div', 'abs', tl, { left: X(6) + 'px', width: (X(22) - X(6)) + 'px', top: '515px', height: '1px', background: `${K('#4b4f60')}` });
  const row = (y, label, spans, color) => {
    el('div', 'abs', tl, { left: '30px', top: y - 50 + 'px', fontSize: '26px', color: `${K('#d6d8e2')}` }, esc(label));
    return spans.map(([a, b, kind]) => {
      const bar = el('div', 'bar', tl, { left: X(a) + 'px', top: y + 'px', height: '56px', width: 0,
        background: kind === 'wait' ? `repeating-linear-gradient(45deg,${K('#3a2c2c')} 0 10px,${K('#2b2224')} 10px 20px)` : color, border: kind === 'wait' ? `1px solid ${K('#8a4f4f')}` : 'none' });
      const lab = el('div', 'abs', tl, { left: X(a) + 12 + 'px', top: y + 14 + 'px', font: '600 19px var(--ui)', color: kind === 'wait' ? `${K('#e59a9a')}` : '#15161c', whiteSpace: 'nowrap' },
        kind === 'wait' ? esc(COPY.s5.used) : `${fmt(a)} – ${fmt(b)}`);
      return { bar, lab, w: X(b) - X(a), a };
    });
  };
  function fmt(h) { return `${String(Math.floor(h)).padStart(2, '0')}:${h % 1 ? String(Math.round(h % 1 * 60)).padStart(2, '0') : '00'}`; }
  const r1 = row(130, COPY.s5.row1, [[9.5, 11.75], [11.75, 14.5, 'wait'], [14.5, 19.5]], `${K('rgba(160,165,185,.75)')}`);
  const r2 = row(360, COPY.s5.row2, [[6.5, 11.5], [11.5, 16.5], [16.5, 21.5]], `linear-gradient(90deg,${K('#ebc198')},${K('#f3d7b4')})`);
  const pings = [6.5, 11.5, 16.5].map(h => el('div', 'abs', tl, { left: X(h) - 9 + 'px', top: '298px', width: '18px', height: '18px', borderRadius: '50%', background: `${K('#ebc198')}`, boxShadow: `0 0 24px ${K('#ebc198')}` }));
  const sum = text(tl, COPY.s5.sum, 'abs', { top: '600px', left: '30px', fontSize: '30px', width: '1200px' });
  const W = win(root, SHOT + 'wakeups.png', 1360, 900);
  const task = W.hl(245, 180, 1084, 207, `${K('#ebc198')}`);
  const c = COPY.s5.c.map(l => callout(root, l, `${K('#ebc198')}`));
  return t => {
    col.update(t, 10.4);
    const swap = ramp(t, 5.6, .8, E.inOut), leave = ramp(t, 10.3, .7);
    tl.style.opacity = ramp(t, .3, .6) * (1 - swap);
    [...r1, ...r2].forEach((s, i) => {
      const start = i < 3 ? .9 + i * .55 : 2.6 + (i - 3) * .55;
      const p = ramp(t, start, .6, E.inOut);
      s.bar.style.width = s.w * p + 'px'; s.lab.style.opacity = ramp(t, start + .35, .3);
    });
    pings.forEach((p, i) => put(p, { s: mix(.3, 1, ramp(t, 2.6 + i * .55, .4, E.back)), o: ramp(t, 2.6 + i * .55, .2) }));
    reveal(sum, ramp(t, 4.2, 1, E.lin));
    W.place(560, 110 + (1 - swap) * 140, .97, swap * (1 - leave));
    camAt(W, t, [[6, 680, 450, 1], [6.8, 680, 450, 1], [7.6, 787, 300, 1.2], [11, 787, 300, 1.2]]);
    glowHL(task, io(t, 7.4, .3, 10.4, .3));
    const [x, y] = W.toStage(787, 387);
    c.forEach((e, i) => show(e, x - 300 + i * 0, y + 70 + i * 74, io(t, 7.8 + i * .45, .4, 10.4, .3), 'l'));
  };
});

// S6: everyday use — menu bar, the popover panel, and the desktop pet
const PET_ROWS = { 1: [120, 120, 120, 120, 120, 120, 120, 220], 3: [140, 140, 140, 280], 4: [140, 140, 140, 140, 280] };
function petFrame(row, ms) {
  const d = PET_ROWS[row], cycle = d.reduce((a, b) => a + b, 0);
  let r = ms % cycle;
  for (let i = 0; i < d.length; i++) { if (r < d[i]) return i; r -= d[i]; }
  return 0;
}
// Panel geometry in points: header ends at 51, footer starts at 663 (720 tall); the tall render scrolls 780pt.
const PANEL = { header: 51, footer: 663, tall: 1500 };
function popover(parent, arrowUp) {
  const box = el('div', 'abs', parent, { width: '450px', height: '720px', left: 0, top: 0, transformOrigin: arrowUp ? '50% 0' : '50% 100%' });
  const arrow = el('div', 'abs', box, { width: '22px', height: '22px', background: arrowUp ? `${K('#2a2a2a')}` : `${K('#1e1e1e')}`, transform: 'rotate(45deg)',
    left: '214px', [arrowUp ? 'top' : 'bottom']: '-9px', boxShadow: `0 0 0 1px ${K('rgba(255,255,255,.14)')}` });
  const frame = el('div', 'abs', box, { inset: 0, borderRadius: '12px', overflow: 'hidden', boxShadow: `0 30px 90px ${K('rgba(0,0,0,.6)')},0 0 0 1px ${K('rgba(255,255,255,.14)')}` });
  img(frame, SHOT + 'panel-sessions.png', { position: 'absolute', width: '450px', height: '720px' });
  const clip = el('div', 'abs', frame, { left: 0, top: PANEL.header + 'px', width: '450px', height: (PANEL.footer - PANEL.header) + 'px', overflow: 'hidden' });
  const tall = img(clip, SHOT + 'panel-sessions-tall.png', { position: 'absolute', width: '450px', height: PANEL.tall + 'px', left: 0 });
  const servers = img(frame, SHOT + 'panel-servers.png', { position: 'absolute', width: '450px', height: '720px', opacity: 0 });
  const hl = el('div', 'hl', frame, { left: '12px', top: '160px', width: '426px', height: '56px', border: `2px solid ${K('#eeb3cf')}` }); hl._color = `${K('#eeb3cf')}`;
  return { box, arrow, tall, servers, hl,
    set(x, y, s, o, scroll = 0, serversOn = 0) {
      put(box, { x, y, s, o }); tall.style.top = -(PANEL.header + scroll) + 'px'; servers.style.opacity = serversOn;
    } };
}
// The pointer follows keyframes [t, x, y]; clicks are [t, right?].
const CURSOR = '<svg width="30" height="36" viewBox="0 0 30 36"><path d="M2 2 L2 28 L8.5 22 L13 33 L17.5 31 L13 20.5 L22 20.5 Z" fill="#111" stroke="#fff" stroke-width="2" stroke-linejoin="round"/></svg>';
function pointer(parent) {
  const c = el('div', 'abs', parent, { left: 0, top: 0, zIndex: 20 }, CURSOR);
  const ring = el('div', 'abs', parent, { left: 0, top: 0, width: '40px', height: '40px', borderRadius: '50%', border: `2px solid ${K('rgba(255,255,255,.8)')}`, zIndex: 19 });
  return (t, path, clicks) => {
    let a = path[0], b = path[path.length - 1];
    for (let i = 0; i < path.length - 1; i++) if (t >= path[i][0] && t <= path[i + 1][0]) { a = path[i]; b = path[i + 1]; break; }
    if (t < path[0][0]) b = a; if (t > b[0]) a = b;
    const p = a === b ? 0 : E.inOut(clamp((t - a[0]) / (b[0] - a[0])));
    const x = mix(a[1], b[1], p), y = mix(a[2], b[2], p);
    const press = Math.max(0, ...clicks.map(([ct]) => 1 - Math.abs(t - ct) / .12));
    put(c, { x: x - 2, y: y - 2, s: 1 - press * .12, o: io(t, path[0][0], .3, path[path.length - 1][0], .3) });
    const last = clicks.filter(([ct]) => t >= ct).pop(), q = last ? clamp((t - last[0]) / .45) : 1;
    put(ring, { x: x - 20, y: y - 20, s: .4 + q * 1.1, o: q < 1 ? (1 - q) * .9 : 0 });
    ring.style.borderColor = last && last[1] ? `${K('rgba(238,179,207,.9)')}` : `${K('rgba(255,255,255,.85)')}`;
  };
}
scene(48, 71, root => {
  const desk = el('div', 'abs', root, { inset: 0, background: `radial-gradient(1200px 800px at 75% 30%, ${K('#3b2f5c')} 0%, transparent 60%), radial-gradient(900px 700px at 15% 85%, ${K('#1d3a55')} 0%, transparent 60%), linear-gradient(160deg,${K('#141826')},${K('#0c0e15')})` });
  const mb = el('div', 'menubar', desk);
  el('b', '', mb, { fontWeight: 700 }, '●');
  COPY.s6.menu.forEach((m, i) => el('span', '', mb, { fontWeight: i ? 500 : 700 }, esc(m)));
  const right = el('div', 'right', mb);
  const itemHL = el('div', 'abs', mb, { left: '1552px', top: '4px', width: '96px', height: '26px', borderRadius: '6px', background: `${K('rgba(255,255,255,.18)')}`, opacity: 0 });
  const icons = ['waveform', 'hand-raised-fill', 'checkmark'].map(n => img(mb, SHOT + `menu-${n}.png`, { width: '180px', height: '28px', position: 'absolute', left: '1510px', top: '3px' }));
  el('span', '', right, {}, esc(COPY.s6.clock));
  const ed = el('div', 'window', desk, { left: '70px', top: '84px', width: '1160px', height: '820px', background: `${K('#16181f')}` });
  const bar = el('div', 'titlebar', ed, { background: `${K('#1f2129')}` }); const lights = el('div', 'lights', bar);
  ['#ff5f57', '#febc2e', '#28c840'].forEach(c => el('i', '', lights, { background: c })); el('span', '', bar, {}, 'workspace — main.swift');
  const palette = [`${K('#c792ea')}`, `${K('#82aaff')}`, `${K('#c3e88d')}`, `${K('#f78c6c')}`, `${K('#89ddff')}`, `${K('#5c6370')}`];
  for (let i = 0; i < 26; i++) {
    let x = 60 + (i % 5 === 0 ? 0 : 40 + (i % 3) * 40);
    for (let k = 0; k < 1 + (i * 7) % 4; k++) {
      const w = 50 + ((i * 37 + k * 53) % 180);
      el('div', 'code', ed, { left: x + 'px', top: 80 + i * 27 + 'px', width: w + 'px', background: palette[(i + k * 2) % palette.length], opacity: .55 });
      x += w + 16;
    }
  }
  const col = el('div', 'abs', root, { left: '1300px', top: '150px', width: '560px' });
  const k = text(col, COPY.s6.k, 'kicker', { color: 'var(--pink)' });
  const q = text(col, COPY.s6.q, 'question', { marginTop: '30px' });
  const h = text(col, COPY.s6.h, 'head', { marginTop: '16px', fontSize: ZH ? '52px' : '48px' });
  const pet = el('div', 'pet', root, { left: '1650px', top: '730px' });
  const sprite = el('div', 'sprite', pet, { width: '192px', height: '208px', backgroundRepeat: 'no-repeat', backgroundImage: "url('/.local/website/pet-sheet.png')", backgroundSize: '1536px 1872px' });
  loads.push(Object.assign(new Image(), { src: '/.local/website/pet-sheet.png' }));
  const label = el('div', 'label', pet);
  const rings = [0, 1, 2].map(() => el('div', 'abs', root, { left: '1746px', top: '820px', width: '10px', height: '10px', borderRadius: '50%', border: `2px solid ${K('#ebc198')}` }));
  const menuPanel = popover(root, true), petPanel = popover(root, false);
  const menu = el('div', 'abs', root, { left: 0, top: 0, width: '230px', padding: '5px', borderRadius: '9px', background: `${K('rgba(44,44,48,.96)')}`, boxShadow: `0 18px 50px ${K('rgba(0,0,0,.55)')},0 0 0 1px ${K('rgba(255,255,255,.14)')}`, font: '15px var(--ui)', color: `${K('#ececec')}`, zIndex: 15 });
  const rows = [];
  COPY.s6.ctx.forEach((label, i) => {
    if (i === 3) el('div', '', menu, { height: '1px', background: `${K('rgba(255,255,255,.14)')}`, margin: '5px 8px' });
    rows.push(el('div', '', menu, { padding: '4px 12px', borderRadius: '5px' }, esc(label)));
  });
  // The library render centers its rows; crop to them so the card is not mostly empty.
  const lib = el('div', 'abs', root, { width: '530px', height: '170px', overflow: 'hidden', borderRadius: '14px', background: `${K('#1e1e1e')}`, boxShadow: `0 30px 90px ${K('rgba(0,0,0,.6)')},0 0 0 1px ${K('rgba(255,255,255,.14)')}`, left: 0, top: 0 });
  img(lib, SHOT + 'pet-library.png', { position: 'absolute', width: '530px', height: '530px', left: 0, top: '-180px' });
  const caps = COPY.s6.c.map(l => callout(root, l, `${K('#eeb3cf')}`));
  const cursor = pointer(root);
  const PET = [1746, 830], ITEM = [1600, 18], TAB = [1375 + 255, 44 + 25];
  const path = [[6, 820, 560], [6.9, ...ITEM], [7.3, ...ITEM], [11.6, 1520, 420], [12.1, ...TAB], [12.6, ...TAB], [14.1, 1100, 640], [14.5, 1100, 640],
                [15.6, ...PET], [16.6, ...PET], [18.7, ...PET], [19.2, PET[0] - 6, PET[1] - 4], [20.4, PET[0] - 150, PET[1] - 80 + 26 * 2], [21.2, PET[0] - 150, PET[1] - 80 + 26 * 2], [22.4, 1300, 700]];
  const clicks = [[7], [12.4], [14.4], [16.4], [18.6], [19.2, true], [20.8]];
  return t => {
    root.style.opacity = 1 - ramp(t, 22.4, .6);
    desk.style.opacity = ramp(t, 0, .8);
    reveal(k, ramp(t, .4, .8, E.lin)); reveal(q, ramp(t, .7, 1, E.lin)); reveal(h, ramp(t, 1.1, 1.3, E.lin));
    col.style.opacity = 1 - ramp(t, 5.8, .5);
    // pet: working → needs input (chime) → working after you answer → complete (chime)
    const phase = t < 3.2 ? 0 : t < 14.6 ? 1 : t < 22 ? 0 : 2, row = [1, 3, 4][phase];
    const since = [t < 3.2 ? 0 : 14.6, 3.2, 22][phase];
    sprite.style.backgroundPosition = `${-petFrame(row, (t - since) * 1000) * 192}px ${-row * 208}px`;
    label.textContent = COPY.s6.pet[phase];
    put(pet, { y: -Math.abs(Math.sin(t * 3)) * (phase === 1 ? 6 : 0), o: ramp(t, .6, .6) });
    icons.forEach((e, i) => { e.style.opacity = i === phase ? 1 : 0; });
    rings.forEach((r, i) => {
      const p = clamp((t - 3.2 - i * .35) / 1.4);
      put(r, { s: 1 + p * 26, o: p > 0 && p < 1 ? (1 - p) * .8 : 0 }); r.style.borderWidth = (2 / (1 + p * 26) * 1.6) + 'px';
    });
    // menu bar popover: open, scroll to quotas, switch to Servers, click away
    const open1 = Math.min(ramp(t, 7.05, .3), 1 - ramp(t, 14.45, .25, E.lin));
    itemHL.style.opacity = open1;
    const scroll = 560 * ramp(t, 9.6, 1.6, E.inOut) * (1 - ramp(t, 12.4, .01, E.lin));
    menuPanel.set(1375, 44, mix(.96, 1, open1), open1, scroll, ramp(t, 12.45, .3));
    glowHL(menuPanel.hl, io(t, 7.6, .3, 9.4, .3));
    // pet popover: click the pet, close with a second click
    const open2 = Math.min(ramp(t, 16.45, .3), 1 - ramp(t, 18.65, .25, E.lin));
    petPanel.set(1440, -10, .9 * mix(.96, 1, open2), open2);  // bottom edge just above the pet
    // right-click menu → Pet settings → pet library
    const menuOn = Math.min(ramp(t, 19.25, .12, E.lin), 1 - ramp(t, 20.85, .15, E.lin));
    put(menu, { x: PET[0] - 236, y: PET[1] - 84, o: menuOn });
    rows.forEach((r, i) => { const hover = i === 2 && t > 20.3; r.style.background = hover ? '#2f6fe0' : 'transparent'; r.style.color = hover ? '#fff' : ''; });
    put(lib, { x: 1060, y: 520 + (1 - ramp(t, 21, .6)) * 40, o: io(t, 21, .5, 22.2, .3) });
    cursor(t, path, clicks);
    const capAt = [[1.2, 3], [3.4, 6.3], [7.3, 9.4], [9.6, 12.2], [12.6, 14.3], [16.6, 18.5], [19.3, 20.8], [21.1, 22.4]];
    caps.forEach((c, i) => show(c, 650, 990, io(t, capAt[i][0], .35, capAt[i][1], .3), 'c'));
  };
});

// S7: server monitoring
scene(71, 84, root => {
  const col = column(root, COPY.s7, 'var(--green)');
  const specs = [['server-local', 291, 600, 140], ['server-preview-host', 478, 600, 461], ['server-vps-host', 291, 1250, 140], ['server-nas-host', 324, 1250, 461]];
  const cards = specs.map(([name, h, x, y], i) => {
    const box = el('div', 'abs', root, { left: x + 'px', top: y + 'px', width: '620px', height: h + 'px', borderRadius: '14px', overflow: 'hidden',
      boxShadow: `0 30px 80px ${K('rgba(0,0,0,.55)')},0 0 0 1px ${K('rgba(255,255,255,.12)')}`, transformOrigin: '50% 50%' });
    img(box, SHOT + name + '.png', { width: '620px', height: h + 'px' });
    const tag = el('div', 'abs', root, { left: x + 620 - 14 + 'px', top: y - 16 + 'px', padding: '6px 16px', borderRadius: '999px', background: `${K('#9ad6bb')}`, color: `${K('#0d1512')}`, fontSize: '20px', fontWeight: 600, whiteSpace: 'nowrap', transform: 'translateX(-100%)' }, esc(COPY.s7.tags[i]));
    return { box, tag, h, x, y };
  });
  const c = COPY.s7.c.map(l => callout(root, l, `${K('#9ad6bb')}`));
  const picker = el('div', 'abs', root, { left: '600px', top: '160px', width: '1270px' });
  const pickText = text(picker, COPY.s7.pick, '', { fontSize: '40px', fontWeight: 600, marginBottom: '40px' });
  const sel = [[1, 1, 0, 1, 0, 1], [1, 1, 1, 1, 1, 1], [1, 1, 0, 1, 0, 1], [1, 1, 0, 1, 1, 1]];
  const pickRows = COPY.s7.hosts.map((name, r) => {
    const row = el('div', '', picker, { display: 'flex', alignItems: 'center', gap: '14px', marginTop: '26px' });
    el('div', '', row, { width: '200px', fontSize: '28px', color: `${K('#d6d8e2')}` }, esc(name));
    return COPY.s7.metrics.map((m, i) => el('div', '', row, { padding: '10px 18px', borderRadius: '12px', fontSize: '22px', border: `1px solid ${K('rgba(255,255,255,.14)')}`, whiteSpace: 'nowrap', _on: sel[r][i] }, (sel[r][i] ? '✓ ' : '') + esc(m)));
  });
  return t => {
    col.update(t, 12.4);
    const focus = t < 2.6 ? -1 : t < 5.2 ? 1 : t < 7 ? 2 : t < 8.8 ? 3 : -1;
    const pick = ramp(t, 8.9, .7, E.inOut), leave = ramp(t, 12.3, .7);
    cards.forEach((card, i) => {
      const enter = ramp(t, .3 + i * .3, .8, E.back), f = focus === i ? 1 : 0, other = focus >= 0 && focus !== i;
      card.box.style.zIndex = card.tag.style.zIndex = f ? 5 : 1;
      put(card.box, { y: (1 - enter) * 80, s: mix(.9, 1, enter) * (1 + .08 * f * ramp(t, [0, 2.6, 5.2, 7][Math.max(focus, 0)], .5)), o: clamp(enter * 1.2) * (other ? .35 : 1) * (1 - pick * .93) * (1 - leave) });
      card.tag.style.opacity = clamp(enter * 1.2) * (other ? .35 : 1) * (1 - pick) * (1 - leave);
    });
    show(c[0], 1250, 700, io(t, 2.9, .4, 5.1, .3), 'l');
    show(c[1], 1880, 455, io(t, 5.4, .4, 7, .3), 'r');
    show(c[2], 1880, 820, io(t, 7.2, .4, 8.8, .3), 'r');
    picker.style.opacity = pick * (1 - leave);
    reveal(pickText, ramp(t, 9.1, .8, E.lin));
    pickRows.forEach((row, r) => row.forEach((chip, i) => {
      const on = chip.style._on ?? sel[r][i], p = ramp(t, 9.4 + r * .25 + i * .07, .35);
      chip.style.opacity = .25 + .75 * p;
      chip.style.background = sel[r][i] && p > .5 ? `${K('rgba(154,214,187,.22)')}` : `${K('rgba(255,255,255,.04)')}`;
      chip.style.borderColor = sel[r][i] && p > .5 ? `${K('#9ad6bb')}` : `${K('rgba(255,255,255,.14)')}`;
      chip.style.color = sel[r][i] ? `${K('#e8f6ef')}` : `${K('#7d8196')}`;
    }));
  };
});

// S8: analytics and integrations
scene(84, 92, root => {
  const col = column(root, { k: COPY.s8.k, h: COPY.s8.h }, 'var(--cyan)');
  const sub = text(root, COPY.s8.sub, 'abs sub', { left: '80px', top: '470px', width: '440px' });
  const W = win(root, SHOT + 'analysis-tall.png', 1360, 1700, 1360, 900);
  const wall = el('div', 'abs', root, { left: '600px', top: '200px', width: '1260px' });
  const h2 = text(wall, COPY.s8.h2, '', { fontSize: '52px', fontWeight: 600, marginBottom: '50px' });
  const grid = el('div', '', wall, { display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: '22px' });
  const tiles = providers.map(([id, name]) => {
    const tile = el('div', '', grid, { display: 'flex', alignItems: 'center', gap: '14px', padding: '18px 20px', borderRadius: '16px', background: `${K('rgba(255,255,255,.05)')}`, border: `1px solid ${K('rgba(255,255,255,.1)')}`, fontSize: '23px', whiteSpace: 'nowrap' });
    img(tile, icon(id), { width: '36px', height: '36px', objectFit: 'contain', filter: MONO.has(id) && !LIGHT ? 'brightness(0) invert(1)' : 'none' }); el('span', '', tile, {}, name);
    return tile;
  });
  const chips = el('div', '', wall, { display: 'flex', gap: '18px', marginTop: '60px' });
  const cs = COPY.s8.chips.map(l => el('div', '', chips, { padding: '12px 24px', borderRadius: '999px', border: `1px solid ${K('rgba(141,219,220,.5)')}`, color: `${K('#c9eeee')}`, fontSize: '24px' }, esc(l)));
  return t => {
    col.update(t, 4.3); reveal(sub, ramp(t, 1.4, 1, E.lin)); sub.style.opacity = 1 - ramp(t, 3.9, .5);
    const swap = ramp(t, 4, .7, E.inOut), leave = ramp(t, 7.4, .6);
    W.place(560, 110 + (1 - ramp(t, 0, 1)) * 80, .97, ramp(t, 0, .7) * (1 - swap));
    camAt(W, t, [[0, 680, 450, 1], [.8, 680, 450, 1], [4, 680, 1240, 1]]);
    wall.style.opacity = swap * (1 - leave);
    reveal(h2, ramp(t, 4.2, .8, E.lin));
    tiles.forEach((e, i) => put(e, { y: (1 - ramp(t, 4.5 + i * .09, .5, E.back)) * 30, o: ramp(t, 4.5 + i * .09, .4) }));
    cs.forEach((e, i) => put(e, { o: ramp(t, 6 + i * .2, .4), y: (1 - ramp(t, 6 + i * .2, .4)) * 16 }));
  };
});

// S9: closing
scene(92, 99, root => {
  const logo = img(root, WEB + 'brand.png', { position: 'absolute', width: '120px', left: '900px', top: '210px' });
  const name = el('div', 'center', root, { top: '350px', fontSize: '40px', letterSpacing: '4px', color: `${K('#cfd2e0')}` }, 'Aieyes');
  const l1 = text(root, COPY.tagline[0], 'center', { top: '420px', fontSize: '84px', fontWeight: 600 });
  const l2 = text(root, COPY.tagline[1], 'center', { top: '525px', fontSize: '84px', fontWeight: 600, color: 'var(--violet)' });
  const url = el('div', 'center', root, { top: '680px', fontSize: '34px', color: `${K('#e4e6f0')}` }, 'aieyes.2h3ngy4n.space');
  const meta = el('div', 'center', root, { top: '740px', fontSize: '24px', color: `${K('#9ea3b6')}` }, 'github.com/JesmonX/Aieyes  ·  macOS · Windows · Linux');
  const cred = el('div', 'center', root, { top: '1020px', font: '16px var(--ui)', color: `${K('#666b7e')}` }, esc(COPY.credits));
  return t => {
    const fade = 1 - ramp(t, 6.1, .9, E.inOut);
    put(logo, { s: mix(.7, 1, ramp(t, .1, .8, E.back)), o: ramp(t, .1, .5) * fade });
    name.style.opacity = ramp(t, .4, .5) * fade;
    reveal(l1, ramp(t, .6, .7, E.lin)); reveal(l2, ramp(t, 1.1, .7, E.lin));
    l1.style.opacity = l2.style.opacity = fade;
    put(url, { o: ramp(t, 2, .6) * fade, y: (1 - ramp(t, 2, .6)) * 16 });
    meta.style.opacity = ramp(t, 2.4, .6) * fade; cred.style.opacity = ramp(t, 2.8, .6) * fade;
  };
});

const demo = el('div', 'demo', stage, {}, esc(COPY.demo));

window.DURATION = DURATION;
window.renderAt = t => {
  background(t);
  for (const s of scenes) {
    const on = t >= s.start && t < s.end + .5;
    s.root.style.display = on ? 'block' : 'none';
    if (!on) continue;
    s.root.style.opacity = Math.min(ramp(t, s.start, .4, E.lin), 1 - ramp(t, s.end, .5, E.lin));
    s.update(t - s.start);
  }
  demo.style.opacity = io(t, 14.5, .6, 83.4, .6) * .9;
};
window.ready = (async () => {
  await document.fonts.load('600 60px "LXGW WenKai"'); await document.fonts.load('400 60px "LXGW WenKai"'); await document.fonts.load('60px "Comic Sans MS"');
  await Promise.all(loads.map(e => e.decode ? e.decode().catch(() => { throw new Error('Image failed: ' + e.src); }) : null));
  await document.fonts.ready;
  window.renderAt(Number(params.get('t') || 0));
  return true;
})();
