#!/usr/bin/env python3
"""Check actual SwiftUI accessibility geometry captured by website/render.py.

Short labels are checked for extra lines; explanatory text may wrap. Scrolled
content is clipped to its nearest scroll area before testing visible controls.
PNG review complements these checks for glyph truncation and material rendering.
"""
import argparse
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTROLS = {'AXButton', 'AXPopUpButton', 'AXMenuButton', 'AXCheckBox', 'AXRadioButton', 'AXTextField', 'AXComboBox'}


def rect(row):
    return row['x'], row['y'], row['x'] + row['width'], row['y'] + row['height']


def intersection(a, b):
    return max(a[0], b[0]), max(a[1], b[1]), min(a[2], b[2]), min(a[3], b[3])


def visible_rect(row, elements, width, height):
    if not row.get('frameAvailable', True):
        return (0, 0, 0, 0)
    clip = (0, 0, width, height)
    for parent in elements.values():
        if parent['role'] == 'AXScrollArea' and parent.get('frameAvailable', True) and row['path'].startswith(parent['path'] + '.'):
            clip = intersection(clip, rect(parent))
    return intersection(clip, rect(row))


def visible(row, elements, width, height):
    area = visible_rect(row, elements, width, height)
    return area[2] - area[0] > 2 and area[3] - area[1] > 2


def is_control(row, elements):
    # AppKit exposes scrollbar thumbs/arrows as buttons. Their size reflects
    # the total scrollable content, including explanatory text allowed to wrap.
    return row['role'] in CONTROLS and not any(
        parent['role'] == 'AXScrollBar' and row['path'].startswith(parent['path'] + '.')
        for parent in elements.values()
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=ROOT / '.local/website/native')
    args = parser.parse_args()
    report = dict(pairs=[], failures=[])
    for theme in ('light', 'dark'):
        names = [f'{page}-{width}-{theme}' for page in ('overview', 'analytics', 'servers') for width in (760, 1120, 1440)]
        names += [f'panel-{width}-{theme}' for width in (360, 450)]
        for name in names:
            pair = dict(name=name, controls=0, shortLabels=0)
            snapshots = {}
            sizes = {}
            for language in ('en', 'zh-CN'):
                data = json.loads((args.root / language / 'audit' / (name + '.json')).read_text())
                assert len(data['elements']) > 20, f'{language}/{name}: SwiftUI accessibility tree was not captured'
                elements = {r['path']: r for r in data['elements']}
                snapshots[language] = elements
                sizes[language] = (data['width'], data['height'])
                controls = [r for r in elements.values() if is_control(r, elements) and visible(r, elements, *sizes[language])]
                for row in controls:
                    # Horizontal scrolling of Agent tabs is intentional. Test
                    # all remaining controls against the host's actual width.
                    horizontal_scroll = any(p['role'] == 'AXScrollArea' and p['height'] < 100 and row['path'].startswith(p['path'] + '.') for p in elements.values())
                    if not horizontal_scroll and (row['x'] < -2 or row['x'] + row['width'] > data['width'] + 2):
                        report['failures'].append(dict(name=name, language=language, issue='control outside host', row=row))
                for i, left in enumerate(controls):
                    for right in controls[i+1:]:
                        if left['path'].startswith(right['path'] + '.') or right['path'].startswith(left['path'] + '.'):
                            continue
                        overlap = intersection(visible_rect(left, elements, *sizes[language]), visible_rect(right, elements, *sizes[language]))
                        if overlap[2]-overlap[0] > 3 and overlap[3]-overlap[1] > 3:
                            report['failures'].append(dict(name=name, language=language, issue='overlapping controls', left=left, right=right))
            for path, en in snapshots['en'].items():
                zh = snapshots['zh-CN'].get(path)
                if not zh or en['role'] != zh['role'] or not en.get('frameAvailable', True) or not zh.get('frameAvailable', True):
                    continue
                if not visible(en, snapshots['en'], *sizes['en']) or not visible(zh, snapshots['zh-CN'], *sizes['zh-CN']):
                    continue
                if is_control(en, snapshots['en']) and is_control(zh, snapshots['zh-CN']):
                    pair['controls'] += 1
                    if abs(en['height'] - zh['height']) > 4:
                        report['failures'].append(dict(name=name, issue='control height changed with language', en=en, zh=zh))
                elif en['role'] == 'AXStaticText' and en['value'].strip() and zh['value'].strip() and max(len(en['value']), len(zh['value'])) <= 64:
                    pair['shortLabels'] += 1
                    short, tall = sorted((en['height'], zh['height']))
                    if short > 0 and tall >= 24 and tall / short > 1.45:
                        report['failures'].append(dict(name=name, issue='extra short-label line', en=en, zh=zh))
            report['pairs'].append(pair)
            if pair['controls'] < 10 or pair['shortLabels'] < 5:
                report['failures'].append(dict(name=name, issue='too few comparable visible elements', compared=pair))
    target = args.root / 'layout-report.json'
    target.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    assert not report['failures'], json.dumps(report['failures'][:8], ensure_ascii=False, indent=2)
    print(f"Native bilingual layout passed: {len(report['pairs'])} language pairs; host bounds, control overlap and short-label line parity. Report: {target}")


if __name__ == '__main__':
    main()
