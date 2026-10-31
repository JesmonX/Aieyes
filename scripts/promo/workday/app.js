// Aieyes workday promo: one day of Codex work on a light macOS desktop, in Chinese.
// Every frame is a pure function of time: renderAt(t). The app UI comes from
// scripts/promo/render_native.py --story workday; window.CUES scores the soundtrack.
const SHOT = '/.local/promo/native/zh-CN-light-workday/', WEB = '/apps/website/public/';
const PET_SHEET = '/.local/website/pet-sheet.png';
const DURATION = 109;
const stage = document.getElementById('stage');

// ---------- math & dom helpers (as in ../composition/app.js) ----------
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
  e._u = [...str].map(u => u === '\n' ? (el('br', '', e), null) : el('span', 'char', e, {}, esc(u))).filter(Boolean);
  return e;
}
function reveal(e, p, spread = 6) {
  const n = e._u.length;
  e._u.forEach((u, i) => {
    const k = E.out(clamp((p * (n + spread) - i) / spread));
    u.style.opacity = k; u.style.transform = `translateY(${(1 - k) * 18}px)`;
  });
}
// The value of the last [time, value] key at or before t.
const step = (t, keys) => keys.reduce((v, [k, x]) => t >= k ? x : v, keys[0][1]);
// Eased interpolation between [time, value] keys.
function keyed(t, keys) {
  if (t <= keys[0][0]) return keys[0][1];
  for (let i = 0; i < keys.length - 1; i++) {
    const [a, x] = keys[i], [b, y] = keys[i + 1];
    if (t < b) return mix(x, y, E.inOut(clamp((t - a) / (b - a))));
  }
  return keys[keys.length - 1][1];
}
// Highlight box: [element, color]; glow(o) fades it.
function hl(parent, [x, y, w, h], color) {
  const e = el('div', 'hl', parent, { left: x + 'px', top: y + 'px', width: w + 'px', height: h + 'px', border: `3px solid ${color}` });
  e._color = color; return e;
}
function glow(e, o) { e.style.opacity = o; e.style.boxShadow = `0 0 ${26 * o}px ${e._color}55, inset 0 0 ${16 * o}px ${e._color}1c`; }

const C = { violet: '#7a5af5', cyan: '#138f93', blue: '#3a6fdc', pink: '#cf4f8b', green: '#23905f', orange: '#c8742a', red: '#c43c3c' };

// ---------- the day ----------
const SCENES = [
  { at: 0, time: '10:40', title: '额度告急', sub: 'Codex 跑着重构，5h 额度快见底了' },
  { at: 27, time: '11:20', title: '走开一会儿', sub: '去看会儿文档，Agent 在后台继续跑' },
  { at: 41, time: '14:05', title: '额度提前重置', sub: '正专心写代码……' },
  { at: 66, time: '20:10', title: '准备训练', sub: '登录服务器之前，先看一眼' },
  { at: 81, time: '22:30', title: '安排这一周', sub: '后面几天都很忙' },
];
const CUTS = [27, 41, 66, 81], OUTRO = 102;
// Menu bar clock in minutes since midnight; it jumps while the scene cuts are veiled.
const CLOCK = [[0, 640], [26.8, 646], [26.81, 680], [40.8, 682], [40.81, 845], [50.9, 846], [52.5, 980], [65.9, 983], [65.91, 1210], [80.9, 1213], [80.91, 1350], [100, 1354]];
const hhmm = m => `${String(Math.floor(m / 60)).padStart(2, '0')}:${String(m % 60).padStart(2, '0')}`;
const clockAt = t => Math.floor(keyed(t, CLOCK));
// Session phase of the one Codex session; drives the pet and the menu bar icon.
const PHASES = [[0, 'working'], [1.2, 'tool'], [3.4, 'thinking'], [4.0, 'working'], [17.1, 'idle'], [21.85, 'working'], [23.6, 'tool'], [24.3, 'working'],
  [24.8, 'tool'], [25.4, 'working'], [25.8, 'tool'], [27.8, 'working'], [29.6, 'thinking'], [31.0, 'needsInput'], [35.95, 'tool'], [37.9, 'tool'], [38.4, 'working'],
  [38.8, 'complete'], [40.8, 'idle'], [43.0, 'working'], [43.8, 'thinking'], [44.6, 'tool'], [45.4, 'working'], [48.2, 'tool'], [48.8, 'working'],
  [51.2, 'complete'], [51.8, 'working'], [52.2, 'tool'], [52.9, 'working'], [55.6, 'tool'], [56.4, 'working'], [59.0, 'tool'], [60.2, 'working'],
  [63.2, 'complete'], [65.9, 'idle']];
const PET = { idle: [0, '空闲'], working: [1, '进行中'], thinking: [8, '思考中'], tool: [7, '执行工具'], needsInput: [3, '等待输入'], complete: [4, '已完成'] };
const PET_MS = [[280, 110, 110, 140, 140, 320], [120, 120, 120, 120, 120, 120, 120, 220], null, [140, 140, 140, 280], [140, 140, 140, 140, 280], null, null,
  [120, 120, 120, 120, 120, 220], [150, 150, 150, 150, 150, 280]];
const MENU_ICON = { idle: 'circle-dotted', working: 'waveform', thinking: 'sparkles', tool: 'gearshape-2', needsInput: 'hand-raised-fill', complete: 'checkmark' };
// Frontmost app: the menu bar name and which window is active.
const FRONT = [[0, '终端'], [13.25, 'Aieyes'], [18.9, '终端'], [28.35, '浏览器'], [35.0, '终端'], [56.3, 'Aieyes'], [64.2, '终端'], [84.7, 'Aieyes']];

// ---------- desktop ----------
const desk = el('div', '', stage); desk.id = 'desk';
el('div', 'wall', desk);
const menubar = el('div', 'menubar', desk);
el('span', '', menubar, { fontSize: '18px' }, '');
const appName = el('span', 'app', menubar);
const menus = el('span', '', menubar, { display: 'flex', gap: '24px' });
const MENUS = { 终端: ['Shell', '编辑', '显示', '窗口', '帮助'], 浏览器: ['文件', '编辑', '显示', '历史记录', '书签', '窗口', '帮助'], Aieyes: ['编辑', '窗口', '帮助'] };
const ICON_X = 1590;  // center of the Aieyes status item
const statusHL = el('div', 'abs', menubar, { left: ICON_X - 18 + 'px', top: '4px', width: '36px', height: '26px', borderRadius: '7px', background: 'rgba(0,0,0,.1)', opacity: 0 });
const statusIcons = Object.fromEntries(Object.entries(MENU_ICON).map(([phase, name]) =>
  [phase, img(menubar, `${SHOT}morning/menu-${name}.png`, { position: 'absolute', left: ICON_X - 90 + 'px', top: '3px', width: '180px', height: '28px', opacity: 0 })]));
el('div', 'abs', menubar, { left: 0, top: 0 }, `<svg style="left:1634px" width="26" height="18" viewBox="0 0 26 18"><rect x="1" y="3" width="20" height="12" rx="3.5" fill="none" stroke="#1d1d1f" stroke-opacity=".55" stroke-width="1.3"/><rect x="3" y="5" width="13" height="8" rx="1.8" fill="#1d1d1f"/><rect x="22.5" y="7" width="2" height="4" rx="1" fill="#1d1d1f" fill-opacity=".5"/></svg>`
  + `<svg style="left:1676px" width="20" height="18" viewBox="0 0 20 18"><path d="M10 15.5l2.4-2.9a3.6 3.6 0 0 0-4.8 0z M4.7 9.6a7.6 7.6 0 0 1 10.6 0l-1.5 1.8a5.3 5.3 0 0 0-7.6 0z M1.6 6.1a12.2 12.2 0 0 1 16.8 0l-1.5 1.8a9.8 9.8 0 0 0-13.8 0z" fill="#1d1d1f"/></svg>`
  + `<svg style="left:1712px" width="20" height="18" viewBox="0 0 20 18"><rect x="1.5" y="2.5" width="17" height="5.5" rx="2.75" fill="none" stroke="#1d1d1f" stroke-width="1.3"/><circle cx="15.7" cy="5.25" r="1.7" fill="#1d1d1f"/><rect x="1.5" y="10" width="17" height="5.5" rx="2.75" fill="none" stroke="#1d1d1f" stroke-width="1.3"/><circle cx="4.3" cy="12.75" r="1.7" fill="#1d1d1f"/></svg>`);
const clock = el('div', 'clock', menubar);

function windowFrame(parent, x, y, w, h, title) {
  const box = el('div', 'window', parent, { left: x + 'px', top: y + 'px', width: w + 'px', height: h + 36 + 'px' });
  const bar = el('div', 'titlebar', box), lights = el('div', 'lights', bar);
  ['#ff5f57', '#febc2e', '#28c840'].forEach(c => el('i', '', lights, { background: c }));
  const label = el('span', '', bar, {}, esc(title));
  const content = el('div', 'content', box, { width: w + 'px', height: h + 'px' });
  return { box, content, label, x, y, w, h };
}

// ---------- terminal: Codex CLI ----------
const TERM = { x: 64, y: 60, w: 1150, h: 770 };
const term = windowFrame(desk, TERM.x, TERM.y, TERM.w, TERM.h - 36, '');
const termBody = el('div', 'term abs', term.content, { inset: 0, padding: 0 });
const hist = el('div', 'abs', termBody, { left: '22px', right: '22px', top: '14px' });
const composer = el('div', 'composer', termBody);
const statusLine = el('div', 'line', composer, { marginBottom: '8px', paddingLeft: '2px' });
const approvalBox = el('div', 'approval', composer);
const composerBox = el('div', 'box', composer);
const hint = el('div', 'hint', composer);
const span = (cls, s) => `<span class="${cls}">${s}</span>`;
const PROMPT = `${span('acc', '~/dev/aieyes')} ${span('vio', '❯')} `;
function banner() {
  const rows = ['>_ OpenAI Codex (v0.58.0)', '', 'model:     gpt-5.1-codex high   /model to change', 'directory: ~/dev/aieyes'];
  const width = 52;
  return [span('dim', '╭' + '─'.repeat(width + 2) + '╮'),
    ...rows.map(r => span('dim', '│ ') + esc(r.padEnd(width)).replace('>_ OpenAI Codex', span('b', '&gt;_ OpenAI Codex')).replace('(v0.58.0)', span('dim', '(v0.58.0)')).replace('/model to change', span('dim', '/model to change')) + span('dim', ' │')),
    span('dim', '╰' + '─'.repeat(width + 2) + '╯')];
}
const user = s => `<span class="user">${span('dim', '›')} ${esc(s)}</span>`;
const did = (verb, rest) => `• ${span('b', verb)} ${rest}`;
const sub = s => `  ${span('dim', '└')} ${s}`;
const diff = (path, add, del) => `${esc(path)} ${span('ok', '(+' + add)}${del ? ' ' + span('red', '-' + del + ')') : span('ok', ')')}`;
const LOG = [];
const log = (t, ...lines) => lines.forEach(l => LOG.push([t, l]));
log(-1, ...banner(), '', user('给额度卡片加上重置倒计时：一天内显示「x 小时 y 分后」，再久显示日期'), '', '• 先读一下额度卡片和时间格式的实现。', '',
  did('Explored', ''), sub('Read Views.swift, Format.swift, Models.swift'), '', '• 计划：在 Format 里加 resetCountdown()，额度卡片底部改用它，再补上测试。', '',
  did('Edited', diff('apps/macos/Sources/Aieyes/Format.swift', 24, 3)));
log(1.2, '', did('Edited', diff('apps/macos/Sources/Aieyes/Views.swift', 11, 5)));
log(2.4, '', did('Ran', 'swift build')); log(3.4, sub('Build complete! (38.4s)'));
log(4.0, '', '• 构建通过，接下来补 FormatTests。');
log(4.6, '', span('warn', "⚠︎ Heads up, you've used over 90% of your 5h limit. Run /status for a breakdown."));
log(17.1, '');  // Aieyes closed codex for the account switch; the shell is back.
log(20.65, PROMPT + 'codex resume --last');
log(20.8, ...banner(), '', user('给额度卡片加上重置倒计时：一天内显示「x 小时 y 分后」，再久显示日期'), '', '• 构建通过，接下来补 FormatTests。');
log(21.85, '', user('继续'));
log(22.6, '', '• 好的，接着补 FormatTests。');
log(23.6, '', did('Explored', ''), sub('Read Tests/AieyesTests/FormatTests.swift'));
log(24.8, '', did('Edited', diff('apps/macos/Tests/AieyesTests/FormatTests.swift', 42)));
log(25.8, '', did('Ran', 'swift test --filter FormatTests'));
log(27.8, sub('Executed 12 tests, with 0 failures (0.42s)'));
log(28.8, '', '• Windows 版也要显示倒计时，用 date-fns 格式化，先装上依赖。');
log(35.95, '', span('ok', '✓') + ' You approved codex to run ' + span('b', 'npm install date-fns --prefix apps/desktop') + ' this time');
log(36.6, '', did('Ran', 'npm install date-fns --prefix apps/desktop')); log(37.2, sub('added 1 package in 1.8s'));
log(37.9, '', did('Edited', diff('apps/desktop/web/app.js', 19, 4)));
log(38.6, '', '• 完成：两端的额度卡片都会显示重置倒计时，测试全部通过。');
log(43.0, '', user('顺手把倒计时也加进桌宠的悬浮提示里'));
log(43.8, '', '• 先看看桌宠悬浮提示是怎么拼出来的。');
log(44.6, '', did('Explored', '')); log(45.0, sub('Search tooltip in apps/macos/Sources/Aieyes'));
log(48.2, '', did('Edited', diff('apps/macos/Sources/Aieyes/PetWindow.swift', 9, 2)));
log(51.0, '', did('Ran', 'swift build'), sub('Build complete! (36.9s)'));
log(51.2, '', '• 完成：桌宠悬浮提示也会显示下次重置时间。');
log(51.8, '', user('再跑一遍完整的 UI 测试'));
log(52.2, '', did('Ran', 'npm run test:ui --prefix apps/desktop'));
log(55.6, sub('214 passed (3m 41s)'));
log(56.4, '', '• UI 测试通过，再检查一下 macOS 原生测试。');
log(59.0, '', did('Ran', 'scripts/test-native.sh'));
log(60.2, sub('All native checks passed'));
log(63.2, '', '• 完成：UI 与原生测试全部通过。');
log(65.9, PROMPT + 'git commit -am "额度卡片与桌宠显示重置倒计时"', '[main 3f2a9c1] 额度卡片与桌宠显示重置倒计时', ' 5 files changed, 111 insertions(+), 15 deletions(-)');
const CLEARS = [65.9];
// codex: the TUI with its composer; shell: a zsh prompt; busy: a command is starting.
const MODE = [[-1, 'codex'], [17.1, 'shell'], [20.65, 'busy'], [20.8, 'codex'], [65.9, 'shell']];
// Typed input: into the shell prompt or the Codex composer, sent with Enter or erased.
const TYPES = [
  { start: 19.4, end: 20.5, text: 'codex resume --last', sent: 20.65 },
  { start: 21.25, end: 21.7, text: '继续', sent: 21.85, ime: true },
  { start: 41.4, end: 42.9, text: '顺手把倒计时也加进桌宠的悬浮提示里', sent: 43.0, ime: true },
  { start: 51.45, end: 51.7, text: '再跑一遍完整的 UI 测试', sent: 51.8, ime: true },
  { start: 66.8, end: 68.0, text: 'ssh ubuntu@gpu-lab', erase: [76.8, 77.25] },
];
const APPROVAL = [31.0, 35.95], KEY_ONE = 35.9;
// Turn start for the Working timer; the last one began before the time skip.
const TURNS = [[-1, -372], [21.85, 21.85], [43.0, 43.0], [51.8, 52.6 - 900]];
function typed(t) {
  for (const k of TYPES) {
    if (t < k.start || (k.sent && t >= k.sent)) continue;
    let n = Math.floor(clamp((t - k.start) / (k.end - k.start)) * k.text.length + .001);
    if (k.erase) n = Math.round(n * (1 - clamp((t - k.erase[0]) / (k.erase[1] - k.erase[0]))));
    return { text: [...k.text].slice(0, n).join(''), into: k.ime ? 'composer' : 'shell' };
  }
  return null;
}
let termKey = '';
function terminal(t) {
  const mode = step(t, MODE), phase = step(t, PHASES), input = typed(t);
  const cleared = CLEARS.filter(c => t >= c).pop() ?? -Infinity;
  const lines = LOG.filter(([k]) => k <= t && k >= cleared).map(([, l]) => l);
  const blink = Math.floor(t * 2) % 2 === 0;
  if (mode === 'shell') lines.push(PROMPT + esc(input && input.into === 'shell' ? input.text : '') + (blink ? '<span class="cursor"></span>' : ''));
  const codex = mode === 'codex', asking = codex && t >= APPROVAL[0] && t < APPROVAL[1];
  const active = codex && ['working', 'thinking', 'tool'].includes(phase);
  const turn = step(t, TURNS), secs = Math.max(0, Math.floor(t - turn));
  const status = active ? `${span('vio', Math.floor(t * 2.5) % 2 ? '•' : '◦')} ${span('b', 'Working')} ${span('dim', `(${secs >= 60 ? Math.floor(secs / 60) + 'm ' : ''}${String(secs % 60).padStart(secs >= 60 ? 2 : 1, '0')}s • esc to interrupt)`)}` : '';
  const draft = input && input.into === 'composer' ? input.text : '';
  const choice = t >= KEY_ONE ? 1 : 0;
  const key = [lines.length, lines[lines.length - 1], mode, status, draft, asking, blink && !draft, choice].join('|');
  if (key === termKey) return;
  termKey = key;
  composer.style.display = codex ? 'block' : 'none';
  statusLine.innerHTML = status || '&nbsp;';
  approvalBox.style.display = asking ? 'block' : 'none';
  composerBox.style.display = asking ? 'none' : 'block';
  approvalBox.innerHTML = [span('b', 'Would you like to run the following command?'), '', `${span('dim', '$')} npm install date-fns --prefix apps/desktop`, '',
    `${span('acc', '›')} ${span(choice ? 'acc b' : 'acc', '1. Yes, proceed')}`, '  2. Yes, and don\'t ask again for this command', `  3. No, and tell Codex what to do differently ${span('dim', 'esc')}`]
    .map(l => `<div class="line">${l || '&nbsp;'}</div>`).join('');
  composerBox.innerHTML = `${span('dim', '›')} ` + (draft ? esc(draft) + '<span class="cursor" style="width:2px"></span>' : span('dim', 'Ask Codex to do anything'));
  hint.innerHTML = `⏎ send   ⇧⏎ newline   ⌃T transcript   ⌃C quit<span style="float:right">${Math.round(87 - t * .4)}% context left</span>`;
  const reserved = codex ? composer.offsetHeight + 30 : 26;
  const rows = Math.floor((TERM.h - 36 - 14 - reserved) / 27);
  hist.innerHTML = lines.slice(-rows).map(l => `<div class="line">${l || ' '}</div>`).join('');
}

// ---------- browser ----------
const BROWSER = { x: 430, y: 104, w: 1080, h: 764 };
const browser = el('div', 'window', desk, { left: BROWSER.x + 'px', top: BROWSER.y + 'px', width: BROWSER.w + 'px', height: BROWSER.h + 'px' });
const tabs = el('div', 'tabs', browser), blights = el('div', 'lights', tabs, { top: '16px' });
['#ff5f57', '#febc2e', '#28c840'].forEach(c => el('i', '', blights, { background: c }));
el('div', 'tab', tabs, {}, '异步取消：drop 一个 Future 时发生了什么');
el('div', 'urlbar', browser, {}, '<div>blog.example.dev/rust/async-cancellation</div>');
const pageClip = el('div', 'abs', browser, { left: 0, right: 0, top: '86px', bottom: 0, overflow: 'hidden' });
const article = el('div', 'article', pageClip, { top: 0 }, `
  <h1>异步取消：drop 一个 Future 时发生了什么</h1>
  <div class="meta">Rust 笔记 · 阅读约 12 分钟</div>
  <p>在 async Rust 里，取消一个任务最常见的方式就是 drop 掉它的 Future。看起来干净利落，但取消只会发生在 <code>.await</code> 处：两个 await 之间的代码要么完整执行，要么根本没开始。</p>
  <h2>一、取消发生在哪里</h2>
  <p>当 <code>select!</code> 的另一个分支先完成时，没被选中的 Future 会在下一次轮询之前被直接 drop。如果它正持有锁，或者写到一半的缓冲区，状态就会停在中间。</p>
  <pre>tokio::select! {
    frame = read_frame(&amp;mut conn) =&gt; handle(frame?).await,
    _ = shutdown.recv() =&gt; return Ok(()),
}</pre>
  <h2>二、让取消变得安全</h2>
  <p>把需要原子完成的步骤放进单独 spawn 的任务，或者用 CancellationToken 显式通知，让清理逻辑有机会运行。对外部资源，优先选择可重入、可恢复的写法。</p>
  <p>另一个常见做法是把状态机显式化：每一步都先写日志、再提交结果，这样即使在任意 await 点被 drop，重启后也能从最近一步继续。</p>`);
loads.push(...article.querySelectorAll('img'));

// ---------- desktop pet ----------
const PET_AT = { x: 1690, y: 806 };
const pet = el('div', 'pet', desk, { left: PET_AT.x + 'px', top: PET_AT.y + 'px' });
const sprite = el('div', 'sprite', pet, { backgroundImage: `url('${PET_SHEET}')`, backgroundSize: '1344px 1638px' });
loads.push(Object.assign(new Image(), { src: PET_SHEET }));
const petLabel = el('div', 'label', pet);
const rings = [0, 1, 2].map(() => el('div', 'abs', desk, { left: PET_AT.x + 84 - 120 + 'px', top: PET_AT.y + 75 - 120 + 'px', width: '240px', height: '240px', borderRadius: '50%', border: `3px solid ${C.orange}`, opacity: 0 }));
function petFrame(row, ms) {
  const d = PET_MS[row], cycle = d.reduce((a, b) => a + b, 0);
  let r = ms % cycle;
  for (let i = 0; i < d.length; i++) { if (r < d[i]) return i; r -= d[i]; }
  return 0;
}
function petAt(t) {
  const phase = step(t, PHASES), [row, label] = PET[phase];
  // The animation restarts only when the sprite row changes.
  let since = 0;
  for (const [k, p] of PHASES) if (k <= t && PET[p][0] !== PET[step(k - .001, PHASES)][0]) since = k;
  sprite.style.backgroundPosition = `${-petFrame(row, (t - since) * 1000) * 168}px ${-row * 182}px`;
  petLabel.textContent = label;
  Object.entries(statusIcons).forEach(([p, e]) => { e.style.opacity = p === phase ? 1 : 0; });
  rings.forEach((r, i) => {
    const p = clamp((t - 31.0 - i * .32) / 1.3);
    put(r, { s: mix(.08, 1, E.out(p)), o: p > 0 && p < 1 ? (1 - p) * .7 : 0 });
  });
}

// ---------- the Aieyes menu bar panel ----------
const PANEL = { header: 51, footer: 663 };
const POP = { x: ICON_X - 225, y: 44 };
const pop = el('div', 'pop', desk, { left: POP.x + 'px', top: POP.y + 'px', opacity: 0, zIndex: 40 });
el('div', 'arrow', pop);
const popFrame = el('div', 'frame', pop);
// One panel page: the static render plus, for scrolling, the tall render inside the content area.
function panelPage(base, tall, tallHeight) {
  const layer = el('div', 'abs', popFrame, { inset: 0, opacity: 0 });
  img(layer, SHOT + base, { top: 0, height: '720px' });
  const page = { layer, scroller: null };
  if (tall) {
    const clip = el('div', 'abs', layer, { left: 0, top: PANEL.header + 'px', width: '450px', height: PANEL.footer - PANEL.header + 'px', overflow: 'hidden', background: '#f6f6f8' });
    page.scroller = el('div', 'abs', clip, { left: 0, top: -PANEL.header + 'px', width: '450px', height: tallHeight + 'px' });
    img(page.scroller, SHOT + tall, { height: tallHeight + 'px' });
  }
  return page;
}
const P = {
  morning: panelPage('morning/panel.png', 'morning/panel-tall.png', 1600),
  afternoon: panelPage('afternoon/panel.png', 'afternoon/panel-tall.png', 1600),
  evening: panelPage('evening/panel.png'), servers: panelPage('evening/servers.png'), gpu: panelPage('evening/servers-gpu.png'),
  night: panelPage('night/panel.png'),
};
const PH = {
  personal: hl(P.morning.scroller, [14, 454, 422, 60], C.red), team: hl(P.morning.scroller, [14, 858, 422, 60], C.blue),
  switch: hl(P.morning.scroller, [22, 826, 150, 36], C.violet), value: hl(P.afternoon.scroller, [14, 734, 422, 54], C.cyan),
  tab: hl(P.evening.layer, [221, 10, 66, 32], C.green), lab: hl(P.servers.layer, [112, 64, 104, 32], C.green),
  gpus: [292, 369, 447, 523].map(y => hl(P.gpu.layer, [22, y - 2, 406, 70], C.red)), detail: hl(P.night.layer, [339, 687, 30, 30], C.orange),
};
// Open spans of the panel: [open, close, page by time, scroll keys].
const POPS = [
  { open: 7.15, close: 13.0, pages: [[0, 'morning']], scroll: [[10.2, 0], [11.4, 379]] },
  { open: 53.45, close: 56.2, pages: [[0, 'afternoon']], scroll: [[54.3, 0], [55.3, 300]] },
  { open: 69.35, close: 75.8, pages: [[0, 'evening'], [70.5, 'servers'], [71.5, 'gpu']] },
  { open: 83.55, close: 84.6, pages: [[0, 'night']] },
];
function panel(t) {
  const shown = POPS.find(p => t >= p.open - .05 && t < p.close + .4);
  const o = shown ? Math.min(ramp(t, shown.open, .28), 1 - ramp(t, shown.close, .22, E.lin)) : 0;
  put(pop, { s: mix(.96, 1, o), o });
  pop.style.display = o > .001 ? 'block' : 'none';
  statusHL.style.opacity = o;
  if (!shown) return;
  const page = step(t, shown.pages), prev = shown.pages.filter(([k]) => k <= t).slice(-2)[0][1];
  const since = shown.pages.filter(([k]) => k <= t).pop()[0];
  Object.entries(P).forEach(([key, p]) => {
    p.layer.style.opacity = key === page ? ramp(t, since, .25, E.lin) : key === prev && page !== prev ? 1 : 0;
    p.layer.style.zIndex = key === page ? 2 : 1;
    if (p.scroller && shown.scroll) p.scroller.style.transform = `translateY(${-keyed(t, shown.scroll)}px)`;
  });
  glow(PH.personal, io(t, 8.0, .3, 10.2, .3)); glow(PH.team, io(t, 11.5, .3, 13.0, .2)); glow(PH.switch, io(t, 12.2, .3, 13.0, .2));
  glow(PH.value, io(t, 55.3, .3, 56.2, .2)); glow(PH.tab, io(t, 70.0, .25, 70.6, .2)); glow(PH.lab, io(t, 71.0, .25, 71.6, .2));
  PH.gpus.forEach((h, i) => glow(h, io(t, 72.7 + i * .3, .3, 75.4, .3))); glow(PH.detail, io(t, 84.0, .25, 84.6, .15));
}

// ---------- Aieyes windows ----------
// Switch account: the targets window, its confirmation sheet, then the switched state.
const targets = windowFrame(desk, 640, 250, 640, 470, '切换账户');
targets.box.style.zIndex = 30;
const tBefore = img(targets.content, SHOT + 'morning/targets.png', { width: '640px', height: '470px' });
const tAfter = img(targets.content, SHOT + 'switched/targets.png', { width: '640px', height: '470px', opacity: 0 });
const tRow = hl(targets.content, [18, 58, 604, 88], C.violet), tUsing = hl(targets.content, [18, 58, 604, 88], C.green);
const tDim = el('div', 'abs', targets.content, { inset: 0, background: 'rgba(30,32,40,.07)', opacity: 0 });
const sheet = el('div', 'abs', desk, { left: '695px', top: '286px', width: '530px', height: '400px', borderRadius: '0 0 12px 12px', overflow: 'hidden', background: '#fff', boxShadow: '0 22px 60px rgba(40,44,70,.24),0 0 0 1px rgba(0,0,0,.09)', zIndex: 31, opacity: 0 });
img(sheet, SHOT + 'morning/confirm.png', { width: '530px', height: '400px', position: 'absolute' });
const sArrow = hl(sheet, [16, 60, 208, 32], C.violet);
// Valuation: the estimate window scrolls its content area like the app does.
const EST = { header: 128, footer: 576 };
const estimate = windowFrame(desk, 695, 150, 530, 600, '团队工作区 · 额度估值');
estimate.box.style.zIndex = 30;
img(estimate.content, SHOT + 'afternoon/estimate.png', { width: '530px', height: '600px' });
const estClip = el('div', 'abs', estimate.content, { left: 0, top: EST.header + 'px', width: '530px', height: EST.footer - EST.header + 'px', overflow: 'hidden', background: '#fff' });
const estScroller = el('div', 'abs', estClip, { left: 0, top: -EST.header + 'px', width: '530px', height: '2520px' });
img(estScroller, SHOT + 'afternoon/estimate-tall.png', { width: '530px', height: '2520px' });
const EH = { now: hl(estScroller, [26, 398, 480, 32], C.cyan), h1: hl(estScroller, [26, 1043, 480, 32], C.blue), h2: hl(estScroller, [26, 1293, 480, 32], C.blue), h3: hl(estScroller, [26, 1542, 480, 32], C.blue) };
const EST_SCROLL = [[58.6, 0], [59.6, 835], [61.0, 835], [61.6, 1110]];
// Wakeups: the main window at 86%, its wakeup page and the editor sheet.
const MAIN = { x: 375, y: 56, s: .86 };
const main = windowFrame(desk, MAIN.x, MAIN.y, 1360, 900, 'Aieyes');
main.box.style.zIndex = 30; main.box.style.transform = `scale(${MAIN.s})`;
const mainPages = Object.fromEntries(['overview', 'wakeups', 'wakeups-draft', 'wakeups-deployed'].map(n => [n, img(main.content, `${SHOT}night/${n}.png`, { width: '1360px', height: '900px', opacity: 0 })]));
const MH = { nav: hl(main.content, [8, 337, 200, 34], C.orange), create: hl(main.content, [253, 77, 80, 29], C.orange), deploy: hl(main.content, [308, 416, 53, 29], C.orange),
  card: hl(main.content, [244, 340, 1088, 122], C.orange), status: hl(main.content, [248, 474, 246, 28], C.green) };
const mainDim = el('div', 'abs', main.content, { inset: 0, background: 'rgba(30,32,40,.07)', opacity: 0 });
const editor = el('div', 'abs', main.content, { left: '405px', top: 0, width: '550px', height: '660px', borderRadius: '0 0 12px 12px', overflow: 'hidden', background: '#fff', boxShadow: '0 22px 60px rgba(40,44,70,.24),0 0 0 1px rgba(0,0,0,.09)', opacity: 0 });
img(editor, SHOT + 'night/wake-editor.png', { width: '550px', height: '660px', position: 'absolute' });
const edTimes = hl(editor, [20, 286, 512, 134], C.orange);
// A window that pops in at `open` and closes at `close`.
function pops(box, t, open, close, from = .94) {
  const o = Math.min(ramp(t, open, .3), 1 - ramp(t, close, .25, E.lin));
  box.style.display = o > .001 ? 'block' : 'none';
  return o;
}
function windows(t) {
  // switch account
  let o = pops(targets.box, t, 13.25, 18.7);
  put(targets.box, { s: mix(.95, 1, ramp(t, 13.25, .35, E.back)), o }); targets.box.style.transformOrigin = '50% 30%';
  tAfter.style.opacity = ramp(t, 17.45, .3, E.lin);
  glow(tRow, io(t, 13.9, .3, 14.8, .2)); glow(tUsing, io(t, 17.6, .3, 18.6, .2));
  const sheetOn = Math.min(ramp(t, 14.8, .3), 1 - ramp(t, 17.05, .28, E.inOut));
  sheet.style.display = sheetOn > .001 ? 'block' : 'none';
  put(sheet, { y: (1 - sheetOn) * -36, o: sheetOn }); tDim.style.opacity = sheetOn;
  glow(sArrow, io(t, 15.3, .3, 16.6, .3));
  // valuation
  o = pops(estimate.box, t, 56.3, 64.0);
  put(estimate.box, { s: mix(.95, 1, ramp(t, 56.3, .35, E.back)), o }); estimate.box.style.transformOrigin = '50% 30%';
  estScroller.style.transform = `translateY(${-keyed(t, EST_SCROLL)}px)`;
  glow(EH.now, io(t, 57.1, .3, 58.6, .2)); glow(EH.h1, io(t, 59.7, .3, 61.0, .2)); glow(EH.h2, io(t, 59.9, .3, 62.4, .3)); glow(EH.h3, io(t, 61.7, .3, 63.6, .3));
  // wakeups
  o = pops(main.box, t, 84.7, OUTRO + .4);
  put(main.box, { s: MAIN.s * mix(.96, 1, ramp(t, 84.7, .35, E.out)), o }); main.box.style.transformOrigin = '0 0';
  const page = t < 85.9 ? 'overview' : t < 90.5 ? 'wakeups' : t < 91.5 ? 'wakeups-draft' : 'wakeups-deployed';
  Object.entries(mainPages).forEach(([n, e]) => { e.style.opacity = n === page ? 1 : 0; });
  glow(MH.nav, io(t, 85.4, .25, 86.0, .2)); glow(MH.create, io(t, 86.5, .25, 87.1, .2)); glow(MH.card, io(t, 90.7, .3, 91.6, .2));
  glow(MH.deploy, io(t, 91.0, .25, 91.6, .2)); glow(MH.status, io(t, 91.8, .3, 93.8, .3));
  const ed = Math.min(ramp(t, 87.1, .32), 1 - ramp(t, 90.45, .28, E.inOut));
  editor.style.display = ed > .001 ? 'block' : 'none';
  put(editor, { y: (1 - ed) * -40, o: ed }); mainDim.style.opacity = ed;
  glow(edTimes, io(t, 88.0, .3, 90.0, .3));
}

// ---------- notifications ----------
function banner2(parent, title, lines) {
  const n = el('div', 'note', parent, { opacity: 0, zIndex: 45 });
  img(n, WEB + 'brand.png');
  el('span', 'when', n, {}, '现在');
  n.innerHTML += `<b>${esc(title)}</b>${lines.map(esc).join('<br>')}`;
  n.querySelectorAll('img').forEach(e => loads.push(e));
  return n;
}
const notePersonal = banner2(desk, '订阅额度提前重置', ['Codex · 个人工作区', '7d：剩余 100%（原预计 10/24 20:01）']);
const noteTeam = banner2(desk, '订阅额度提前重置', ['Codex · 团队工作区', '5h：剩余 100%（原预计 10/21 15:45）；7d：剩余 100%（原预计 10/26 20:01）']);
function notifications(t) {
  const out = ramp(t, 50.2, .4, E.in);
  const a = ramp(t, 44.0, .45), b = ramp(t, 44.25, .45);
  put(notePersonal, { x: 1500 + (1 - b) * 440 + out * 440, y: 58, s: .95, o: b * .85 });
  put(noteTeam, { x: 1500 + (1 - a) * 440 + out * 440, y: 46, o: a });
  notePersonal.style.transformOrigin = '50% 0';
}

// ---------- pointer ----------
const CURSOR = '<svg width="30" height="36" viewBox="0 0 30 36"><path d="M2 2 L2 28 L8.5 22 L13 33 L17.5 31 L13 20.5 L22 20.5 Z" fill="#111" stroke="#fff" stroke-width="2" stroke-linejoin="round"/></svg>';
const arrow = el('div', 'cursor-arrow', desk, {}, CURSOR);
const ring = el('div', 'abs', desk, { left: 0, top: 0, width: '40px', height: '40px', borderRadius: '50%', border: '2px solid rgba(30,30,40,.55)', zIndex: 59 });
// Pointer paths: keys [t, x, y] in desktop coordinates and click times.
const POINTER = [
  { keys: [[6.0, 760, 640], [7.0, ICON_X, 17], [7.6, ICON_X, 17], [8.3, 1838, 330], [11.5, 1838, 330], [12.6, 1461, 509], [13.3, 1461, 509], [14.4, 1224, 389],
      [15.0, 1224, 389], [15.6, 1320, 560], [16.2, 1320, 560], [16.85, 1152, 651], [17.4, 1152, 651], [18.2, 1180, 700], [19.3, 760, 760]], clicks: [7.1, 12.9, 14.7, 17.0] },
  { keys: [[27.6, 900, 520], [28.25, 1380, 420], [29.4, 1290, 500], [30.8, 1340, 560], [34.3, 1340, 560], [34.9, 250, 600], [36.2, 250, 600]], clicks: [28.35, 35.0] },
  { keys: [[52.6, 1100, 600], [53.3, ICON_X, 17], [53.9, 1838, 300], [55.2, 1838, 300], [56.0, 1515, 496], [56.5, 1515, 496], [57.2, 1000, 520], [63.6, 1000, 520]], clicks: [53.4, 56.1] },
  { keys: [[68.6, 900, 600], [69.2, ICON_X, 17], [69.9, 1700, 110], [70.4, 1618, 70], [70.9, 1618, 70], [71.4, 1528, 124], [71.9, 1528, 124], [72.6, 1856, 300], [75.6, 1856, 300]], clicks: [69.3, 70.5, 71.5] },
  { keys: [[82.8, 1000, 600], [83.4, ICON_X, 17], [84.5, 1719, 746], [84.9, 1719, 746], [85.8, 444, 391], [86.2, 444, 391], [86.9, 627, 165], [87.4, 627, 165], [88.0, 850, 440],
      [89.7, 850, 440], [90.3, 1143, 606], [90.8, 1143, 606], [91.4, 663, 457], [91.9, 663, 457], [92.6, 860, 520], [93.8, 860, 520]], clicks: [83.5, 84.6, 85.9, 87.0, 90.4, 91.5] },
];
const CLICKS = POINTER.flatMap(p => p.clicks);
function pointer(t) {
  const path = POINTER.find(p => t >= p.keys[0][0] - .3 && t <= p.keys[p.keys.length - 1][0] + .3);
  if (!path) { arrow.style.opacity = 0; ring.style.opacity = 0; return; }
  const keys = path.keys, x = keyed(t, keys.map(([k, x]) => [k, x])), y = keyed(t, keys.map(([k, , y]) => [k, y]));
  const press = Math.max(0, ...path.clicks.map(c => 1 - Math.abs(t - c) / .12));
  put(arrow, { x: x - 2, y: y - 2, s: 1 - press * .12, o: io(t, keys[0][0] - .3, .3, keys[keys.length - 1][0], .3) });
  const last = path.clicks.filter(c => t >= c).pop(), q = last === undefined ? 1 : clamp((t - last) / .45);
  put(ring, { x: x - 20, y: y - 20, s: .4 + q * 1.1, o: q < 1 ? (1 - q) * .9 : 0 });
}

// ---------- camera ----------
// Keys [t, x, y, scale]: the desktop point to center and the zoom.
const CAM = [[0, 960, 540, 1], [4.2, 960, 540, 1], [5.0, 640, 660, 1.22], [6.0, 640, 660, 1.22], [6.8, 960, 540, 1], [7.2, 960, 540, 1], [8.0, 1560, 386, 1.4],
  [12.9, 1560, 386, 1.4], [13.7, 960, 440, 1.28], [18.7, 960, 440, 1.28], [19.5, 960, 540, 1],
  [31.4, 960, 540, 1], [32.2, 1720, 880, 1.6], [33.6, 1720, 880, 1.6], [34.3, 960, 540, 1], [35.1, 960, 540, 1], [35.7, 620, 700, 1.32], [37.4, 620, 700, 1.32], [38.2, 960, 540, 1],
  [44.2, 960, 540, 1], [45.0, 1700, 170, 1.55], [50.2, 1700, 170, 1.55], [51.0, 960, 540, 1], [53.4, 960, 540, 1], [54.2, 1560, 386, 1.4], [56.2, 1560, 386, 1.4],
  [56.9, 960, 480, 1.35], [64.0, 960, 480, 1.35], [64.8, 960, 540, 1],
  [69.3, 960, 540, 1], [70.1, 1560, 386, 1.4], [71.6, 1560, 386, 1.4], [72.4, 1590, 470, 1.75], [75.8, 1590, 470, 1.75], [76.6, 960, 540, 1],
  [87.1, 960, 540, 1], [87.9, 960, 372, 1.45], [90.4, 960, 372, 1.45], [91.1, 760, 440, 1.4], [93.8, 760, 440, 1.4], [94.4, 960, 540, 1], [DURATION, 960, 540, 1]];
function camera(t) {
  let a = CAM[0], b = CAM[CAM.length - 1];
  for (let i = 0; i < CAM.length - 1; i++) if (t >= CAM[i][0] && t <= CAM[i + 1][0]) { a = CAM[i]; b = CAM[i + 1]; break; }
  const p = a === b ? 0 : E.inOut(clamp((t - a[0]) / (b[0] - a[0])));
  const s = Math.exp(mix(Math.log(a[3]), Math.log(b[3]), p)), cx = mix(a[1], b[1], p), cy = mix(a[2], b[2], p);
  const x = clamp(960 - cx * s, 1920 - 1920 * s, 0), y = clamp(540 - cy * s, 1080 - 1080 * s, 0);
  desk.style.transform = `translate(${x}px,${y}px) scale(${s})`;
}

// ---------- overlays ----------
const veil = el('div', 'abs', stage, { inset: 0, background: '#f6f7fb', opacity: 0, zIndex: 65 });
const chips = SCENES.map(s => {
  const big = el('div', 'chip', stage, { opacity: 0 });
  el('div', 'time', big, {}, s.time);
  const title = text(big, s.title, 'title'), subline = text(big, s.sub, 'sub');
  const pill = el('div', 'pill', stage, { opacity: 0 }, `<em></em>${esc(s.title)}`);
  return { ...s, big, title, subline, pill, now: pill.firstChild };
});
function sceneChips(t) {
  chips.forEach((c, i) => {
    const end = i < CUTS.length ? CUTS[i] : OUTRO, start = c.at + (i ? .3 : .5);
    const big = io(t, start, .5, start + 3.0, .5);
    put(c.big, { y: (1 - ramp(t, start, .6)) * 24, o: big });
    reveal(c.title, ramp(t, start + .25, .6, E.lin)); reveal(c.subline, ramp(t, start + .6, .9, E.lin));
    put(c.pill, { o: io(t, start + 3.3, .4, end - .9, .4), y: (1 - ramp(t, start + 3.3, .5)) * 10 });
    c.now.textContent = hhmm(clockAt(t));
  });
}
function callout(label, color) {
  return el('div', 'callout', stage, {}, `<b style="background:${color};box-shadow:0 0 12px ${color}"></b>${esc(label)}`);
}
const CALLOUTS = [
  [8.2, 10.6, '菜单栏面板：所有账户的额度一眼看全', C.violet, 560], [15.4, 18.0, '已登录的账户，一键切换', C.violet],
  [21.0, 24.0, '换号后 codex resume，原来的对话接着跑', C.violet],
  [31.6, 34.4, 'Agent 等你确认：桌宠变换动作，提示音提醒', C.pink], [36.0, 38.6, '回来批准一下，Agent 继续干活', C.pink],
  [45.0, 47.6, '额度提前重置，Aieyes 自动通知', C.blue], [57.2, 60.6, '自动估值：每段额度值多少，一直有记录', C.cyan],
  [72.4, 75.0, '切到服务器：GPU 利用率、显存一眼看到', C.green, 560], [77.4, 80.0, '不用 SSH 登录，机器忙不忙心里有数', C.green],
  [88.2, 90.6, '按这周的节奏，定好唤醒时刻', C.orange], [92.0, 94.4, '部署到本机，退出 Aieyes 也照常执行', C.orange],
].map(([a, b, label, color, x = 960]) => ({ a, b, x, e: callout(label, color) }));
function callouts(t) {
  CALLOUTS.forEach(({ a, b, x, e }) => { const o = io(t, a, .4, b, .35); put(e, { x: x - e.offsetWidth / 2, y: 1000 - e.offsetHeight / 2 + (1 - o) * 14, o }); });
}
// The user's thoughts: [from, to, text, x, y], centered at (x, y) on screen.
const THOUGHTS = [
  [5.0, 7.0, '5h 额度快见底了……看看另一个号', 1430, 300], [11.6, 13.4, '团队号还满着，就它了！', 600, 470],
  [32.0, 33.8, '嗯？它在等我。', 1150, 560],
  [46.0, 48.4, '又提前重置了？', 700, 300], [47.8, 50.6, '老这么重置……Tibo 老贼不会偷偷砍我额度吧？', 700, 450],
  [62.4, 65.0, '嗯嗯，还好，和以前差不多。', 1600, 790],
  [68.2, 70.2, '等等，先看看机器空不空。', 880, 560], [74.4, 76.6, '全满……算了，下次吧。', 520, 520], [77.2, 79.4, '还好没登上去白忙活。', 760, 420],
  [81.8, 84.2, '这周后面都排满了，额度得跟上节奏。', 880, 560],
].map(([a, b, words, x, y]) => {
  const e = el('div', 'thought', stage);
  const line = text(e, words, '');
  [[30, 72, 16], [14, 94, 10]].forEach(([x, y, r]) => el('i', '', e, { left: x + 'px', top: y + 'px', width: r + 'px', height: r + 'px' }));
  return { a, b, e, line, x, y };
});
function thoughts(t) {
  THOUGHTS.forEach(({ a, b, e, line, x, y }) => {
    const o = io(t, a, .35, b, .35);
    put(e, { x: x - e.offsetWidth / 2, y: y - e.offsetHeight / 2 + (1 - o) * 12, s: mix(.94, 1, ramp(t, a, .4, E.back)), o });
    reveal(line, ramp(t, a + .1, Math.min(1.2, (b - a) * .45), E.lin), 4);
  });
}
// Valuation check: the four latest 5h values side by side.
const compare = el('div', 'card', stage, { left: '1352px', top: '232px', width: '520px', height: '440px', opacity: 0, zIndex: 75 });
el('div', 'abs', compare, { left: '34px', top: '30px', font: '600 26px var(--font)' }, '团队工作区 · 5h 额度价值');
el('div', 'abs', compare, { left: '34px', top: '70px', font: '18px var(--font)', color: 'var(--muted)' }, '最近 4 段，API 等价价值');
const VALUES = [['10/17', 35.8], ['10/19', 34.7], ['10/20', 36.1], ['今天', 35.2]];
const columns = VALUES.map(([day, v], i) => {
  const x = 46 + i * 116, h = v / 36.1 * 200;
  const bar = el('div', 'bar', compare, { left: x + 'px', width: '74px', top: 340 - h + 'px', height: h + 'px', transformOrigin: '50% 100%',
    background: i === 3 ? `linear-gradient(180deg,${C.cyan},#1bb3b8)` : 'linear-gradient(180deg,#c9cfe0,#dfe3ee)' });
  const num = el('div', 'abs', compare, { left: x - 20 + 'px', width: '114px', textAlign: 'center', top: 340 - h - 36 + 'px', font: '600 21px var(--font)', color: i === 3 ? C.cyan : '#4a4e5c' }, '$' + v.toFixed(2));
  el('div', 'abs', compare, { left: x - 20 + 'px', width: '114px', textAlign: 'center', top: '354px', font: '18px var(--font)', color: 'var(--muted)' }, day);
  return { bar, num };
});
el('div', 'abs', compare, { left: '34px', top: '396px', font: '16px var(--ui)', color: 'var(--faint)' }, '数据来自采样历史 · 演示数据');
function compareCard(t) {
  const o = io(t, 61.8, .45, 64.6, .4);
  put(compare, { o, y: (1 - ramp(t, 61.8, .5)) * 20 });
  columns.forEach(({ bar, num }, i) => { const p = ramp(t, 62.0 + i * .12, .55, E.back); bar.style.transform = `scaleY(${p})`; num.style.opacity = ramp(t, 62.3 + i * .12, .3); });
}
// The week: windows without and with wakeups at 07:00 · 12:00 · 17:00.
const week = el('div', 'card', stage, { left: '200px', top: '150px', width: '1520px', height: '740px', opacity: 0, zIndex: 75 });
const weekTitle = text(week, '这周每天 07:00 · 12:00 · 17:00 自动唤醒', '', { position: 'absolute', left: '60px', top: '44px', font: '600 38px var(--font)' });
el('div', 'abs', week, { left: '60px', top: '104px', font: '22px var(--font)', color: 'var(--muted)' }, '以 5h 窗口为例：开工前先发起一次请求，让重置更贴近这周的作息');
const X = h => 300 + (h - 6) / 18 * 1160;
const band = el('div', 'abs', week, { left: X(9) + 'px', width: X(22) - X(9) + 'px', top: '196px', height: '400px', borderRadius: '14px', background: 'rgba(200,116,42,.06)', border: '1px dashed rgba(200,116,42,.35)' });
el('div', 'abs', band, { top: '-32px', left: 0, right: 0, textAlign: 'center', font: '20px var(--font)', color: C.orange }, '这周的工作时段 09:00 – 22:00');
for (let h = 6; h <= 24; h++) {
  if (h % 2 === 0) el('div', 'abs', week, { left: X(h) - 30 + 'px', width: '60px', textAlign: 'center', top: '622px', font: '17px var(--ui)', color: '#7b8092' }, `${String(h).padStart(2, '0')}:00`);
  el('div', 'abs', week, { left: X(h) + 'px', top: '610px', width: '1px', height: h % 2 ? '6px' : '10px', background: '#c3c6d0' });
}
el('div', 'abs', week, { left: X(6) + 'px', width: X(24) - X(6) + 'px', top: '610px', height: '1px', background: '#c3c6d0' });
// One row: 5h windows [start, end, label], then the wait after the quota runs out at 11:40, labeled beside it.
function weekRow(y, title, note, windows, [a, b, waitText], color) {
  el('div', 'abs', week, { left: '60px', top: y + 4 + 'px', font: '600 24px var(--font)', color: '#2b2e3a' }, title);
  el('div', 'abs', week, { left: '60px', top: y + 42 + 'px', width: '224px', font: '17px/1.45 var(--font)', color: 'var(--muted)' }, note);
  const bars = windows.map(([from, to, label]) => {
    const bar = el('div', 'bar', week, { left: X(from) + 4 + 'px', top: y + 'px', height: '64px', width: 0, overflow: 'hidden', background: color });
    el('div', 'abs', bar, { left: '14px', top: '20px', font: '600 18px var(--ui)', color: '#2b2e3a', whiteSpace: 'nowrap' }, label);
    return { bar, w: X(to) - X(from) - 8 };
  });
  const hatch = el('div', 'bar', week, { left: X(a) + 'px', top: y + 74 + 'px', height: '30px', width: 0, borderRadius: '8px',
    background: 'repeating-linear-gradient(45deg,#f6dcdc 0 10px,#fbeeee 10px 20px)', border: '1px solid #dea3a3' });
  const wait = el('div', 'abs', week, { left: X(b) + 12 + 'px', top: y + 78 + 'px', font: '600 17px var(--ui)', color: '#b9483f', whiteSpace: 'nowrap', opacity: 0 }, waitText);
  return { bars: [...bars, { bar: hatch, w: X(b) - X(a) }], wait };
}
const plain = weekRow(236, '不设唤醒', '9:00 第一次提问才开始计时', [[9, 14, '09:00 – 14:00'], [14, 19, '14:00 – 19:00'], [19, 24, '19:00 – 24:00']],
  [11 + 40 / 60, 14, '11:40 用完，干等 2 小时 20 分'], 'linear-gradient(90deg,#d3d8e6,#e3e6ef)');
const timed = weekRow(436, '定时唤醒', '每天 07:00 · 12:00 · 17:00', [[7, 12, '07:00 – 12:00'], [12, 17, '12:00 – 17:00'], [17, 22, '17:00 – 22:00']],
  [11 + 40 / 60, 12, '12:00 就重置，只等 20 分钟'], 'linear-gradient(90deg,#f0c08c,#f6dcb8)');
const pings = [7, 12, 17].map(h => el('div', 'abs', week, { left: X(h) - 10 + 'px', top: '404px', width: '20px', height: '20px', borderRadius: '50%', background: C.orange, boxShadow: `0 0 20px ${C.orange}` }));
const weekSum = text(week, '开工前就开始计时：额度用完时，下一次重置已经不远。', '', { position: 'absolute', left: '60px', top: '672px', font: '500 26px var(--font)' });
function weekCard(t) {
  const o = io(t, 94.4, .5, OUTRO - .4, .4);
  put(week, { o, y: (1 - ramp(t, 94.4, .6)) * 24 });
  reveal(weekTitle, ramp(t, 94.6, .8, E.lin));
  [plain, timed].forEach((row, r) => {
    const start = 94.9 + r * .6;
    row.bars.forEach((s, i) => { const p = ramp(t, start + i * .25, .5, E.inOut); s.bar.style.width = s.w * p + 'px'; s.bar.style.opacity = p > .01 ? 1 : 0; });
    row.wait.style.opacity = ramp(t, start + 1.05, .3);
  });
  pings.forEach((p, i) => put(p, { s: mix(.3, 1, ramp(t, 95.5 + i * .25, .35, E.back)), o: ramp(t, 95.5 + i * .25, .2) }));
  reveal(weekSum, ramp(t, 96.8, .9, E.lin));
}
// Closing card.
const outro = el('div', 'abs', stage, { inset: 0, opacity: 0, zIndex: 95, background: 'radial-gradient(1200px 700px at 50% 0%,#e9e3fd 0%,transparent 60%),radial-gradient(900px 600px at 50% 110%,#dceff6 0%,transparent 60%),#f6f7fb' });
const logo = img(outro, WEB + 'brand.png', { position: 'absolute', width: '120px', left: '900px', top: '200px' });
const brandName = el('div', 'abs', outro, { left: 0, right: 0, top: '340px', textAlign: 'center', font: '500 40px var(--font)', letterSpacing: '4px', color: '#3b3e4c' }, 'Aieyes');
const tag1 = text(outro, '所有 AI Agent，', '', { position: 'absolute', left: 0, right: 0, top: '410px', textAlign: 'center', font: '600 84px var(--font)' });
const tag2 = text(outro, '一眼看清。', '', { position: 'absolute', left: 0, right: 0, top: '515px', textAlign: 'center', font: '600 84px var(--font)', color: C.violet });
const recap = el('div', 'abs', outro, { left: 0, right: 0, top: '660px', textAlign: 'center', font: '26px var(--font)', color: '#4a4e5c' }, '额度 · 换号 · 会话提醒 · 自动估值 · 服务器 · 定时唤醒');
const url = el('div', 'abs', outro, { left: 0, right: 0, top: '732px', textAlign: 'center', font: '500 32px var(--font)', color: '#1d1f29' }, 'aieyes.2h3ngy4n.space');
const meta = el('div', 'abs', outro, { left: 0, right: 0, top: '788px', textAlign: 'center', font: '22px var(--font)', color: '#686d7e' }, 'github.com/JesmonX/Aieyes  ·  macOS · Windows · Linux');
el('div', 'abs', outro, { left: 0, right: 0, top: '1022px', textAlign: 'center', font: '16px var(--ui)', color: '#9a9eae' }, '界面为合成演示数据 · 桌宠原画：月薪喵 · kiffin');
function closing(t) {
  outro.style.opacity = ramp(t, OUTRO - .5, .7, E.lin);
  put(logo, { s: mix(.7, 1, ramp(t, OUTRO, .8, E.back)), o: ramp(t, OUTRO, .5) });
  brandName.style.opacity = ramp(t, OUTRO + .3, .5);
  reveal(tag1, ramp(t, OUTRO + .5, .7, E.lin)); reveal(tag2, ramp(t, OUTRO + 1.0, .7, E.lin));
  put(recap, { o: ramp(t, OUTRO + 1.8, .6), y: (1 - ramp(t, OUTRO + 1.8, .6)) * 14 });
  put(url, { o: ramp(t, OUTRO + 2.3, .6), y: (1 - ramp(t, OUTRO + 2.3, .6)) * 14 });
  meta.style.opacity = ramp(t, OUTRO + 2.7, .6);
}
const demo = el('div', 'demo', stage, {}, '界面为合成演示数据');

// ---------- frame ----------
let frontKey = '';
function desktop(t) {
  const front = step(t, FRONT);
  if (front !== frontKey) {
    frontKey = front;
    appName.textContent = front;
    menus.innerHTML = MENUS[front].map(m => `<span>${m}</span>`).join('');
  }
  clock.textContent = `10月21日 周三  ${hhmm(clockAt(t))}`;
  const browserFront = front === '浏览器';
  term.box.style.zIndex = browserFront ? 10 : 12; browser.style.zIndex = browserFront ? 12 : 10;
  term.box.classList.toggle('inactive', front !== '终端');
  browser.classList.toggle('inactive', !browserFront);
  term.label.textContent = step(t, MODE) === 'shell' ? 'aieyes — zsh — 132×40' : 'aieyes — codex — 132×40';
  article.style.transform = `translateY(${-keyed(t, [[28.6, 0], [31.0, 430]])}px)`;
}
window.DURATION = DURATION;
window.renderAt = t => {
  camera(t); desktop(t); terminal(t); petAt(t); panel(t); windows(t); notifications(t); pointer(t);
  const cut = Math.max(0, ...CUTS.map(c => io(t, c - .8, .5, c + .1, .5)));
  desk.style.filter = cut > .02 ? `blur(${cut * 7}px)` : 'none';
  veil.style.opacity = Math.max(cut * .62, ramp(t, OUTRO - .9, .8, E.lin)) * (1 - ramp(t, DURATION - 1, 1));
  // Fade the entire stage so the closing card never reveals the desktop underneath.
  stage.style.opacity = ramp(t, 0, .8, E.lin) * (1 - ramp(t, DURATION - 1, 1, E.inOut));
  sceneChips(t); callouts(t); thoughts(t); compareCard(t); weekCard(t); closing(t);
  demo.style.opacity = io(t, 1.0, .6, OUTRO - .9, .5) * .9;
};
// Sound cues for scripts/promo/soundtrack.py: typing, clicks, banners and the app's own session sounds.
const KEYS = [...TYPES.flatMap(k => {
  const n = [...k.text].length * (k.ime ? 2 : 1), times = Array.from({ length: n }, (_, i) => k.start + i * (k.end - k.start) / n);
  if (k.sent) times.push(k.sent);
  if (k.erase) times.push(k.erase[0], k.erase[0] + .12);
  return times;
}), KEY_ONE];
window.CUES = { duration: DURATION, arpeggio: [1.0, DURATION - 2], bright: [.5, OUTRO - .4], rhythm: [.4, DURATION - 1.5], sparse: [[0, OUTRO]], cuts: [...CUTS, OUTRO],
  sounds: [[31.0, 'needs-input.wav', .7], [38.8, 'complete.wav', .55], [63.2, 'complete.wav', .55]],
  clicks: CLICKS, keys: KEYS.map(k => Math.round(k * 1000) / 1000), pops: [44.0] };
window.ready = (async () => {
  await document.fonts.ready;
  await Promise.all(loads.map(e => e.decode ? e.decode().catch(() => { throw new Error('Image failed: ' + e.src); }) : null));
  window.renderAt(Number(new URLSearchParams(location.search).get('t') || 0));
  return true;
})();
