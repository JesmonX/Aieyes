#!/usr/bin/env node
import {execFileSync} from 'node:child_process';
import fs from 'node:fs';
import zlib from 'node:zlib';

const args = process.argv.slice(2);
const flag = name => args.includes(name);
function option(name, fallback) {
  const i = args.findIndex(arg => arg === name || arg.startsWith(name + '='));
  if (i === -1) return fallback;
  return args[i].includes('=') ? args[i].slice(args[i].indexOf('=') + 1) : args[i + 1];
}

const days = Number(option('--days', '30'));
if (!Number.isFinite(days) || days <= 0) {
  console.error('--days must be a positive number');
  process.exit(2);
}
const localFile = option('--file', '');
const host = option('--host', 'proxyus');
const details = flag('--details');
const asJson = flag('--json');

const sshOptions = ['-o', 'ClearAllForwardings=yes', '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=yes', '-o', 'UpdateHostKeys=no', '-o', 'ConnectTimeout=15'];

function readLogs() {
  if (localFile) {
    const body = fs.readFileSync(localFile);
    return localFile.endsWith('.gz') ? zlib.gunzipSync(body).toString('utf8') : body.toString('utf8');
  }
  const remote = `sudo -n bash -c 'for f in /var/log/caddy/aieyes-access.log*; do case "$f" in *.gz) zcat "$f";; *) cat "$f";; esac; done' | grep -F '"uri":"/go/' || true`;
  return execFileSync('ssh', [...sshOptions, host, remote], {encoding: 'utf8', maxBuffer: 64 * 1024 * 1024});
}

const labels = {
  '/go/github': 'GitHub',
  '/go/download/macos': 'Download · macOS',
  '/go/download/windows': 'Download · Windows',
  '/go/download/linux': 'Download · Linux',
  '/go/download/latest': 'Latest release',
  '/go/download/macos/arm64/dmg': 'Download · macOS Apple Silicon DMG',
  '/go/download/macos/x64/dmg': 'Download · macOS Intel DMG',
  '/go/download/windows/x64/exe': 'Download · Windows x64 EXE',
  '/go/download/linux/x64/appimage': 'Download · Linux x64 AppImage',
  '/go/download/linux/x64/deb': 'Download · Linux x64 DEB',
  '/go/download/linux/x64/rpm': 'Download · Linux x64 RPM'
};
const botMarkers = ['bot', 'crawler', 'spider', 'headless', 'python', 'curl', 'wget', 'scan', 'leakix', 'l9scan', 'go-http-client', 'okhttp', 'facebookexternalhit', 'ahrefs', 'semrush', 'deploy-check'];
const isBot = ua => {
  const lower = ua.toLowerCase();
  return botMarkers.some(marker => lower.includes(marker));
};

let text;
try {
  text = readLogs();
} catch (error) {
  console.error(`Unable to read access logs from ${localFile || host}: ${error.message}`);
  process.exit(1);
}

const cutoff = Math.floor(Date.now() / 1000) - days * 86400;
const stats = new Map();
let logStart = Infinity, logEnd = 0, total = 0;

for (const line of text.split('\n')) {
  if (!line.startsWith('{')) continue;
  let entry;
  try {
    entry = JSON.parse(line);
  } catch {
    continue;
  }
  const request = entry.request;
  if (!request || typeof request.uri !== 'string') continue;
  const path = request.uri.split('?')[0];
  if (!path.startsWith('/go/')) continue;
  const ts = Number(entry.ts || 0);
  if (!ts) continue;
  logStart = Math.min(logStart, ts);
  logEnd = Math.max(logEnd, ts);
  if (ts < cutoff) continue;
  const ua = ((request.headers || {})['User-Agent'] || [''])[0];
  const ip = request.remote_ip || '';
  const item = stats.get(path) || {path, label: labels[path] || path, clicks: 0, humanClicks: 0, ips: new Map(), last: 0};
  item.clicks++;
  item.last = Math.max(item.last, ts);
  const current = item.ips.get(ip) || {clicks: 0, human: false, last: 0};
  current.clicks++;
  current.human ||= !isBot(ua);
  current.last = Math.max(current.last, ts);
  item.ips.set(ip, current);
  if (!isBot(ua)) item.humanClicks++;
  stats.set(path, item);
  total++;
}

const fmt = ts => new Date((ts + 8 * 3600) * 1000).toISOString().replace('T', ' ').slice(0, 16) + ' (Beijing)';
const rows = [...stats.values()]
  .map(item => {
    const ips = [...item.ips.values()];
    return {
      path: item.path,
      label: item.label,
      clicks: item.clicks,
      uniqueIps: ips.length,
      humanClicks: item.humanClicks,
      humanIps: ips.filter(ip => ip.human).length,
      last: fmt(item.last)
    };
  })
  .sort((a, b) => b.clicks - a.clicks);

if (asJson) {
  console.log(JSON.stringify({windowDays: days, logStart: logStart === Infinity ? null : fmt(logStart), logEnd: logEnd ? fmt(logEnd) : null, totalClicks: total, rows}, null, 2));
  process.exit(0);
}

if (!total) {
  console.log(`No /go/ clicks in the last ${days} day(s).`);
  if (logEnd) console.log(`Log coverage: ${fmt(logStart)} → ${fmt(logEnd)}`);
  process.exit(0);
}

console.log(`Outbound button clicks — last ${days} day(s)`);
console.log(`Log coverage: ${fmt(logStart)} → ${fmt(logEnd)}`);
console.log('');
const header = ['Button', 'Clicks', 'Unique IPs', 'Human clicks', 'Human IPs', 'Last click'];
const table = [header, ...rows.map(row => [row.label, row.clicks, row.uniqueIps, row.humanClicks, row.humanIps, row.last])];
const widths = header.map((_, column) => Math.max(...table.map(row => String(row[column]).length)));
for (const [index, row] of table.entries()) {
  console.log(row.map((cell, column) => String(cell).padEnd(widths[column])).join('  ').trimEnd());
  if (index === 0) console.log(widths.map(width => '-'.repeat(width)).join('  '));
}
console.log('');
console.log(`Total clicks: ${total}. Unique IPs and human counts are approximate; NAT and shared networks can merge visitors.`);

if (details) {
  console.log('\nPer-IP detail:');
  for (const row of rows) {
    const item = stats.get(row.path);
    console.log(`  ${row.label}`);
    for (const [ip, value] of [...item.ips.entries()].sort((a, b) => b[1].clicks - a[1].clicks)) {
      console.log(`    ${ip.padEnd(40)} clicks=${String(value.clicks).padStart(4)} ${value.human ? 'browser' : 'bot/automation'} last=${fmt(value.last)}`);
    }
  }
}
