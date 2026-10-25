#!/usr/bin/env node
// Development-only host. No core process, personal data or outbound RPC is used.
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { spawn, spawnSync } = require('node:child_process');
const root = path.resolve(__dirname, '../..');
const web = path.join(root, 'apps/desktop/web');
const scenarios = ['workspace', 'empty', 'single', 'multi', 'long', 'failure', 'capacity'];
const platforms = ['windows', 'linux'];
const contract = JSON.parse(fs.readFileSync(path.join(root, 'docs/core-contracts.json')));
const version = JSON.parse(fs.readFileSync(path.join(web, '../src-tauri/tauri.conf.json'))).version;
const mime = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.json': 'application/json', '.png': 'image/png', '.svg': 'image/svg+xml', '.ttf': 'font/ttf', '.txt': 'text/plain; charset=utf-8' };
const csp = JSON.parse(fs.readFileSync(path.join(web, '../src-tauri/tauri.conf.json'))).app.security.csp + "; frame-src 'self'; frame-ancestors 'self'; object-src 'none'; base-uri 'self'";

function seed(scenario, platform) {
  if (!scenarios.includes(scenario) || !platforms.includes(platform)) throw new Error('Unknown preview scenario or platform');
  const generated = spawnSync(process.env.AIEYES_PREVIEW_PYTHON || (process.platform === 'win32' ? 'python' : 'python3'), [path.join(root, 'scripts/preview-quota-core.py'), '--fixture'], {
    env: { ...process.env, AIEYES_UI_SCENARIO: scenario }, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024,
  });
  if (generated.error || generated.status !== 0) throw new Error(`Synthetic fixture generation failed: ${generated.error?.message || generated.stderr}`);
  const data = JSON.parse(generated.stdout);
  data.platform = platform;data.scenario = scenario;data.version = version;data.stamp = Math.floor(Date.now() / 1000);
  data.settings.appearance = { language: 'zh-CN', theme: 'light', accent: 'indigo', chartPalette: 'classic', chartContrast: 'high' };
  data.settings.sources.forEach(s => {s.path = s.hostId || platform === 'linux' ? `/home/preview/.${s.provider}` : `C:\\Users\\Preview\\.${s.provider}`;});
  data.settings.agents = [...new Set(data.settings.sources.map(s => s.provider))].map(provider => ({ provider, enabled: true, machineIds: [...new Set(data.settings.sources.filter(s => s.provider === provider).map(s => s.hostId || 'local'))] }));
  if (scenario === 'empty') {data.settings.hosts = [];data.hosts = [];data.wakeups = [];}
  data.sessions = data.settings.sources.slice(0, 3).map((s, i) => ({ id: `preview-${i}`, source: s.provider, provider: s.provider, machine: s.hostId ? '模拟服务器' : '本机（模拟）', machineId: s.hostId || 'local', sourceId: s.id, phase: ['working', 'needsInput', 'complete'][i], updatedAt: data.stamp }));
  const model = data.prices[0];
  data.prices.push({ ...model, id: 'preview/fast-model', name: 'Preview Fast Model', input: model.input / 2, output: model.output / 2 });
  return data;
}

function createPreviewServer() {
  const clients = new Set();
  let timer,reloadWorkbench = false;
  const send = () => {for (const client of clients) client.write(`data: ${reloadWorkbench ? 'workbench' : 'app'}\n\n`);reloadWorkbench = false;};
  const watchers = [web, __dirname].map(dir => fs.watch(dir, { recursive: true }, () => {reloadWorkbench ||= dir === __dirname;clearTimeout(timer);timer = setTimeout(send, 150);}));
  const server = http.createServer((req, res) => {
    res.setHeader('Cache-Control', 'no-store');res.setHeader('X-Content-Type-Options', 'nosniff');res.setHeader('Content-Security-Policy', csp);
    // Reject DNS rebinding and cross-origin framing of this local development host.
    if (!/^127\.0\.0\.1(?::\d+)?$/.test(req.headers.host || '')) {res.writeHead(403).end('Local preview only');return;}
    if (req.method !== 'GET' && req.method !== 'HEAD') {res.writeHead(405).end();return;}
    try {
      const url = new URL(req.url, 'http://127.0.0.1');
      const name = decodeURIComponent(url.pathname);
      if (name === '/preview/events') {
        res.setHeader('Content-Type', 'text/event-stream');res.setHeader('Connection', 'keep-alive');res.write(': connected\n\n');clients.add(res);req.on('close', () => clients.delete(res));return;
      }
      if (name === '/preview/seed') {
        const scenario = url.searchParams.get('scenario') || 'workspace', platform = url.searchParams.get('platform') || 'windows';
        if (!scenarios.includes(scenario) || !platforms.includes(platform)) {res.writeHead(400).end('Unknown scenario or platform');return;}
        const value = { data: seed(scenario, platform), methods: contract.methods, providers: contract.providers };
        res.setHeader('Content-Type', mime['.json']);res.end(req.method === 'HEAD' ? '' : JSON.stringify(value));return;
      }
      const files = { '/': 'index.html', '/preview/workbench.js': 'workbench.js', '/preview/workbench.css': 'workbench.css', '/preview/model.js': 'model.js', '/preview/bridge.js': 'bridge.js' };
      let file;
      if (files[name]) file = path.join(__dirname, files[name]);
      else if (name.startsWith('/app/')) {
        const relative = name.slice(5);
        if (relative.split(/[\\/]/).some(part => part === '..' || part.startsWith('.')) || relative.includes('\0') || relative.includes('\\')) {res.writeHead(403).end();return;}
        file = path.join(web, relative);
        if (!fs.realpathSync(file).startsWith(fs.realpathSync(web) + path.sep)) {res.writeHead(403).end();return;}
      } else {res.writeHead(404).end();return;}
      if (!fs.statSync(file).isFile()) {res.writeHead(404).end();return;}
      let body = fs.readFileSync(file);
      if (file === path.join(web, 'index.html') || file === path.join(web, 'floating.html')) body = Buffer.from(body.toString().replace('<head>', '<head><script src="/preview/bridge.js"></script>'));
      res.setHeader('Content-Type', mime[path.extname(file)] || 'application/octet-stream');res.end(req.method === 'HEAD' ? '' : body);
    } catch (error) {
      const status = ['ENOENT', 'ENOTDIR', 'EISDIR'].includes(error.code) ? 404 : error instanceof URIError ? 400 : 500;
      res.writeHead(status).end(status === 500 ? error.message : 'Not found');
    }
  });
  server.on('close', () => {clearTimeout(timer);watchers.forEach(w => w.close());});
  return { server, async close() {for (const client of clients) client.end();server.closeIdleConnections();await new Promise(resolve => server.close(resolve));} };
}

if (require.main === module) {
  const args = process.argv.slice(2);let port = 4173,platform = 'windows',open = true;
  try {
    for (let i = 0; i < args.length; i++) {
      if (args[i] === '--port') {port = Number(args[++i]);if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error('Invalid port');}
      else if (args[i] === '--platform') {platform = args[++i];if (!platforms.includes(platform)) throw new Error('Use windows or linux');}
      else if (args[i] === '--no-open') open = false;
      else throw new Error(`Unknown option: ${args[i]}`);
    }
    // Fail before opening a browser if the fixture generator is unavailable.
    seed('workspace', platform);
    const preview = createPreviewServer();
    preview.server.on('error', error => {console.error(error.code === 'EADDRINUSE' ? `Port ${port} is busy; use --port <number>.` : error.message);preview.close();process.exitCode = 1;});
    preview.server.listen(port, '127.0.0.1', () => {
      const url = `http://127.0.0.1:${preview.server.address().port}/?platform=${platform}`;
      console.log(`Aieyes Windows / Linux preview: ${url}\nSynthetic data only. Ctrl+C to stop.`);
      if (open) {
        const command = process.platform === 'darwin' ? 'open' : process.platform === 'win32' ? 'rundll32' : 'xdg-open';
        const child = spawn(command, process.platform === 'win32' ? ['url.dll,FileProtocolHandler', url] : [url], { stdio: 'ignore' });
        child.on('error', () => console.log('Open the preview URL in your browser.'));
      }
    });
    for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => preview.close());
  } catch (error) {console.error(error.message);process.exitCode = 1;}
}
module.exports = { createPreviewServer, seed, scenarios };
