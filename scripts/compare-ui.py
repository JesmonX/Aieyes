#!/usr/bin/env python3
"""Build a local review gallery from matching synthetic native/browser captures."""
from pathlib import Path
import html
root = Path(__file__).resolve().parent.parent / '.local/windows-ui-sync'
rows = []
for scenario in ['empty', 'single', 'multi', 'long', 'failure', 'capacity']:
    for theme in ['light', 'dark']:
        for native, web in [('menubar', 'panel'), ('dashboard', 'overview'), ('servers', 'servers'), *[(f'settings-{tab}', f'settings-{"hosts" if tab == "servers" else tab}') for tab in ['sources', 'servers', 'prices', 'wakeups', 'general']]]:
            mac = f'macos/{scenario}/{native}-{theme}.png'
            desktop = f'browser/{scenario}-{theme}-{web}.png'
            if not (root / mac).exists() or not (root / desktop).exists():
                continue
            label = html.escape(f'{scenario} · {theme} · {web}')
            rows.append(f'<details><summary>{label}</summary><div class="pair"><figure><figcaption>macOS 原生基准</figcaption><img loading="lazy" src="{mac}"></figure><figure><figcaption>桌面 Web（本次运行所在系统字体）</figcaption><img loading="lazy" src="{desktop}"></figure></div></details>')
if not rows:
    raise SystemExit('Generate native and browser captures first')
root.joinpath('comparison.html').write_text('''<!doctype html><meta charset="utf-8"><title>Aieyes UI 对照</title>
<style>body{font:15px system-ui;margin:24px;background:#eceef1;color:#222}summary{padding:12px;cursor:pointer}.pair{display:grid;grid-template-columns:1fr 1fr;gap:16px}figure{margin:0}img{width:100%;height:auto}figcaption{padding:8px}details{border-top:1px solid #bbb}</style>
<h1>Aieyes · macOS 与桌面 Web 对照</h1><p>同一组模拟数据；保留 Windows 外壳。浏览器截图不能验证 Win32 窗口显隐、实际 Windows 字体或混合 DPI。</p>
''' + '\n'.join(rows), encoding='utf-8')
print(f'{len(rows)} comparison pairs: {root / "comparison.html"}')
