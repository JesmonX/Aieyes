// Render the promo composition frame by frame and pipe it into the AVFoundation encoder.
//   node scripts/promo/capture.cjs --lang zh-CN --out video.mp4 [--fps 30]
//   node scripts/promo/capture.cjs --lang en --stills 3,15.5,40   (review PNGs in .local/promo/stills)
const http = require('http'), fs = require('fs'), path = require('path'), { spawn } = require('child_process');
const ROOT = path.resolve(__dirname, '../..');
const { chromium } = require(path.join(ROOT, 'apps/desktop/node_modules/playwright'));
const arg = (name, fallback) => { const i = process.argv.indexOf('--' + name); return i > 0 ? process.argv[i + 1] : fallback; };
const lang = arg('lang', 'zh-CN'), font = arg('font', 'brand'), theme = arg('theme', 'dark'), fps = Number(arg('fps', 30)), stills = arg('stills'), out = arg('out');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.png': 'image/png', '.webp': 'image/webp', '.ttf': 'font/ttf' };

const server = http.createServer((req, res) => {
  let url = decodeURIComponent(req.url.split('?')[0]);
  if (url === '/' ) url = '/scripts/promo/composition/index.html';
  if (url === '/app.js') url = '/scripts/promo/composition/app.js';
  const file = path.join(ROOT, url);
  if (!file.startsWith(ROOT) || !fs.existsSync(file)) { res.writeHead(404); return res.end(); }
  res.writeHead(200, { 'content-type': types[path.extname(file)] || 'application/octet-stream' });
  fs.createReadStream(file).pipe(res);
});

(async () => {
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const browser = await chromium.launch({ args: ['--font-render-hinting=none', '--force-color-profile=srgb'] });
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  page.on('pageerror', e => { console.error('page error:', e.message); process.exitCode = 1; });
  await page.goto(`http://127.0.0.1:${server.address().port}/?lang=${lang}&font=${font}&theme=${theme}`);
  await page.evaluate(() => window.ready);
  const duration = await page.evaluate(() => window.DURATION);
  if (stills) {
    const dir = path.join(ROOT, '.local/promo/stills'); fs.mkdirSync(dir, { recursive: true });
    for (const t of stills.split(',').map(Number)) {
      await page.evaluate(t => window.renderAt(t), t);
      await page.screenshot({ path: path.join(dir, `${lang}-${font}-${theme}-${String(t).padStart(5, '0')}.png`) });
    }
    console.log('stills in', dir);
  } else {
    const encoder = spawn(path.join(ROOT, '.local/promo/bin/video'), ['encode', '--output', out, '--fps', String(fps)], { stdio: ['pipe', 'inherit', 'inherit'] });
    const done = new Promise((resolve, reject) => encoder.on('close', code => code ? reject(new Error('encoder ' + code)) : resolve()));
    const total = Math.round(duration * fps), started = Date.now();
    const cdp = await page.context().newCDPSession(page);
    for (let i = 0; i < total; i++) {
      await page.evaluate(t => window.renderAt(t), i / fps);
      const { data } = await cdp.send('Page.captureScreenshot', { format: 'jpeg', quality: 95 });
      const frame = Buffer.from(data, 'base64'), head = Buffer.alloc(4); head.writeUInt32BE(frame.length);
      if (!encoder.stdin.write(Buffer.concat([head, frame]))) await new Promise(r => encoder.stdin.once('drain', r));
      if (i % 150 === 0) console.log(`${lang} frame ${i}/${total} · ${((Date.now() - started) / 1000).toFixed(0)}s`);
    }
    encoder.stdin.end(); await done;
  }
  await browser.close(); server.close();
})().catch(e => { console.error(e); process.exit(1); });
