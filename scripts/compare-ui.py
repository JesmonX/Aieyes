#!/usr/bin/env python3
"""Build a local review gallery from matching synthetic native/browser captures."""
from pathlib import Path
import argparse
import html
import os

root = Path(__file__).resolve().parent.parent / '.local/windows-ui-sync'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--native-root', type=Path, default=root / 'macos')
parser.add_argument('--web-root', type=Path, default=root / 'browser')
parser.add_argument('--before-web-root', type=Path)
parser.add_argument('--output', type=Path, default=root / 'comparison.html')
args = parser.parse_args()
args.output.parent.mkdir(parents=True, exist_ok=True)


def figure(path, label):
    relative = html.escape(os.path.relpath(path, args.output.parent), quote=True)
    return f'<figure><figcaption>{html.escape(label)}</figcaption><a href="{relative}"><img loading="lazy" src="{relative}" alt="{html.escape(label, quote=True)}"></a></figure>'


rows, surfaces = [], set()
for scenario in ['empty', 'single', 'multi', 'long', 'failure', 'capacity']:
    for theme in ['light', 'dark']:
        for native, web in [('menubar', 'panel'), ('dashboard', 'overview'), ('servers', 'servers'), *[(f'settings-{tab}', f'settings-{"hosts" if tab == "servers" else tab}') for tab in ['sources', 'accounts', 'servers', 'prices', 'wakeups', 'general']]]:
            mac = args.native_root / scenario / f'{native}-{theme}.png'
            desktop = args.web_root / f'{scenario}-{theme}-{web}.png'
            if not mac.exists() or not desktop.exists():
                continue
            surfaces.add(web)
            label = html.escape(f'{scenario} · {theme} · {web}')
            figures = [figure(mac, 'macOS 原生基准')]
            if args.before_web_root:
                before = args.before_web_root / desktop.name
                if before.exists():
                    figures.append(figure(before, 'Windows 调整前'))
            figures.append(figure(desktop, 'Windows 调整后 · Chromium / 本地 HarmonyOS 字体'))
            rows.append(f'<details data-scenario="{scenario}" data-theme="{theme}" data-surface="{web}"><summary>{label}</summary><div class="pair">'+''.join(figures)+'</div></details>')
if not rows:
    raise SystemExit('Generate native and browser captures first')


def select(name, label, values):
    options = ''.join(f'<option>{html.escape(value)}</option>' for value in values)
    return f'<label>{label} <select data-filter="{name}"><option value="">全部</option>{options}</select></label>'


args.output.write_text('''<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Aieyes UI 对照</title>
<style>body{font:15px system-ui;margin:24px;background:#eceef1;color:#222}summary{padding:12px;cursor:pointer}.pair{display:flex;gap:16px;align-items:flex-start}figure{flex:1;min-width:0;margin:0;overflow:auto}img{max-width:100%;height:auto}body.actual img{max-width:none}figcaption{padding:8px;font-weight:600}details{border-top:1px solid #bbb}nav{display:flex;flex-wrap:wrap;gap:16px;padding:16px 0;position:sticky;top:0;background:#eceef1;z-index:1}button,select{font:inherit}[hidden]{display:none}a{color:inherit}@media(max-width:850px){.pair{flex-direction:column}figure{width:100%}}</style>
<h1>Aieyes · macOS 与 Windows 内容区对照</h1><p>同一组模拟数据，保留 Windows 侧栏。点击图片可查看原图。Chromium 验证布局及已加载的本地字体；实际 WebView2、WebKitGTK、DWM 和跨屏缩放仍需实机检查。</p><nav>
''' + select('scenario', '数据', ['empty', 'single', 'multi', 'long', 'failure', 'capacity'])
    + select('theme', '主题', ['light', 'dark']) + select('surface', '页面', sorted(surfaces))
    + '<label><input id="actual" type="checkbox">按原始尺寸显示</label><button id="expand">展开当前结果</button></nav>'
    + '\n'.join(rows) + '''
<script>
const filters=[...document.querySelectorAll('[data-filter]')];
filters.forEach(control=>control.onchange=()=>document.querySelectorAll('details').forEach(row=>row.hidden=filters.some(filter=>filter.value&&row.dataset[filter.dataset.filter]!==filter.value)));
document.querySelector('#actual').onchange=event=>document.body.classList.toggle('actual',event.target.checked);
document.querySelector('#expand').onclick=()=>{const rows=[...document.querySelectorAll('details:not([hidden])')],open=rows.some(row=>!row.open);rows.forEach(row=>row.open=open);};
</script></html>
''', encoding='utf-8')
print(f'{len(rows)} comparison groups: {args.output}')
