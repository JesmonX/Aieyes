#!/usr/bin/env python3
"""Check exact UI message coverage and placeholder integrity without build dependencies."""
import json
import re
from collections import Counter
from html.parser import HTMLParser
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HAN = re.compile(r'[\u3400-\u9fff]')
SLOT = re.compile(r'\{\d+\}')
references = {}
errors = []


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            errors.append('Duplicate catalog key: ' + key)
        result[key] = value
    return result


catalog = json.loads((ROOT / 'locales/en.json').read_text(encoding='utf-8'), object_pairs_hook=unique_object)
for source, target in catalog.items():
    if not source or not isinstance(target, str) or not target.strip():
        errors.append('Empty translation: ' + repr(source))
    elif HAN.search(target):
        errors.append('Untranslated English value: ' + repr(source))
    elif Counter(SLOT.findall(source)) != Counter(SLOT.findall(target)):
        errors.append('Changed placeholders: ' + repr(source))


def add(value, location):
    if HAN.search(value):
        references.setdefault(value, set()).add(location)


def decode(value):
    return re.sub(r'\\(u\{[0-9a-fA-F]+\}|u[0-9a-fA-F]{4}|n|r|t|0|[\\\"\x27`])',
                  lambda m: chr(int(m[1][2:-1], 16)) if m[1].startswith('u{') else
                  chr(int(m[1][1:], 16)) if m[1].startswith('u') else
                  {'n': '\n', 'r': '\r', 't': '\t', '0': '\0'}.get(m[1], m[1]), value)


# Comments and raw strings are tokens, so quotes in comments/regexes do not count as copy.
TOKEN = re.compile(r'//[^\n]*|/\*[\s\S]*?\*/|r(?P<hashes>#+)\"[\s\S]*?\"(?P=hashes)|r\"[\s\S]*?\"|\"(?:\\[\s\S]|[^\"\\])*\"|\x27(?:\\[\s\S]|[^\x27\\\n])*\x27')


def scan_js(text, location):
    # Template HTML contains quotes that are not JavaScript string delimiters.
    # Scan interpolation expressions recursively instead of matching through tags.
    def quoted(i):
        mark, start = text[i], i
        i += 1
        while i < len(text):
            if text[i] == chr(92):
                i += 2
            elif text[i] == mark:
                value = decode(text[start + 1:i])
                if value not in ('简体中文', '中文'):
                    add(value, f'{location}:{text.count(chr(10), 0, start) + 1}')
                return i + 1
            else:
                i += 1
        return i

    def template(i):
        i += 1
        while i < len(text):
            if text[i] == chr(92):
                i += 2
            elif text[i] == '`':
                return i + 1
            elif text.startswith('${', i):
                i = code(i + 2, True)
            else:
                i += 1
        return i

    def code(i, expression=False):
        depth = 1
        while i < len(text):
            if text.startswith('//', i):
                end = text.find(chr(10), i)
                i = len(text) if end < 0 else end
            elif text.startswith('/*', i):
                end = text.find('*/', i + 2)
                i = len(text) if end < 0 else end + 2
            elif text[i] == '/' and (not text[:i].rstrip() or text[:i].rstrip()[-1] in '=([{,:;!?&|' or text[:i].rstrip().endswith('return')):
                j, character_class = i + 1, False
                while j < len(text) and text[j] != chr(10):
                    if text[j] == chr(92):
                        j += 2
                        continue
                    if text[j] == '[':
                        character_class = True
                    elif text[j] == ']':
                        character_class = False
                    elif text[j] == '/' and not character_class:
                        break
                    j += 1
                i = j + 1
            elif text[i] in ('"', "'"):
                i = quoted(i)
            elif text[i] == '`':
                i = template(i)
            elif expression and text[i] == '{':
                depth += 1
                i += 1
            elif expression and text[i] == '}':
                depth -= 1
                i += 1
                if depth == 0:
                    return i
            else:
                i += 1
        return i
    code(0)


def scan_code(file):
    text = file.read_text(encoding='utf-8')
    if file.suffix == '.js':
        scan_js(text, str(file.relative_to(ROOT)))
        return
    tokens = list(TOKEN.finditer(text))
    ignored = []
    if file.suffix == '.rs':
        masked = list(text)
        for token in tokens:
            masked[token.start():token.end()] = ['\n' if c == '\n' else ' ' for c in token[0]]
        masked = ''.join(masked)
        for match in re.finditer(r'#\[cfg\((?:test|all\(test\b[^\]]*\))\)\]', masked):
            start = masked.find('{', match.end())
            if start < 0:
                continue
            depth, end = 1, start + 1
            while depth and end < len(masked):
                depth += (masked[end] == '{') - (masked[end] == '}')
                end += 1
            ignored.append((match.start(), end))
    for token in tokens:
        raw = token[0]
        if raw.startswith(('/', 'r')) or not HAN.search(raw) or any(a <= token.start() < b for a, b in ignored):
            continue
        # Rust lifetime and character literals cannot be UI messages.
        if file.suffix == '.rs' and raw.startswith("'"):
            continue
        value = decode(raw[1:-1])
        if file.suffix == '.rs':
            if value.startswith('UPDATE '):
                # Stored migration notice is SQL data; the statement itself is not copy.
                for inner in re.findall(r"'([^']*)'", value):
                    add(inner, str(file.relative_to(ROOT)))
                continue
            index = [0]
            def slot(_):
                result = '{' + str(index[0]) + '}'
                index[0] += 1
                return result
            value = re.sub(r'(?<!\{)\{[^{}]*\}(?!\})', slot, value)
        if file.suffix == '.js' and value in ('简体中文', '中文'):
            continue  # Language endonyms are intentional in both locales.
        add(value, f'{file.relative_to(ROOT)}:{text.count(chr(10), 0, token.start()) + 1}')


def swift_string(text, start):
    i = start
    while i < len(text) and text[i] == '#':
        i += 1
    hashes = text[start:i]
    if i >= len(text) or text[i] != '"':
        return None
    mark = '"""' if text.startswith('"""', i) else '"'
    i += len(mark)
    begin, chunks, arguments = i, [], []
    stop, escape = mark + hashes, '\\' + hashes
    while i < len(text):
        if text.startswith(stop, i):
            chunks.append(text[begin:i])
            return i + len(stop), ''.join(chunks), arguments, bool(hashes)
        if text.startswith(escape + '(', i):
            chunks.append(text[begin:i])
            chunks.append('{' + str(len(arguments)) + '}')
            i += len(escape) + 1
            a, depth = i, 1
            while i < len(text) and depth:
                nested = swift_string(text, i) if text[i] in '#"' else None
                if nested:
                    i = nested[0]
                    continue
                depth += (text[i] == '(') - (text[i] == ')')
                i += 1
            arguments.append(text[a:i - 1])
            begin = i
        elif text.startswith(escape, i):
            i += len(escape) + 1
        else:
            i += 1
    raise ValueError('Unclosed Swift string')


def scan_swift(text, location):
    i = 0
    while i < len(text):
        if text.startswith('//', i):
            end = text.find('\n', i)
            i = len(text) if end < 0 else end
            continue
        if text.startswith('/*', i):
            i = text.index('*/', i + 2) + 2
            continue
        parsed = swift_string(text, i) if text[i] in '#"' else None
        if parsed:
            end, value, arguments, raw = parsed
            if not raw and value not in ('简体中文', '中文'):
                add(decode(value), f'{location}:{text.count(chr(10), 0, i) + 1}')
            for argument in arguments:
                scan_swift(argument, location)
            i = end
        else:
            i += 1


class Markup(HTMLParser):
    def __init__(self, location):
        super().__init__(convert_charrefs=True)
        self.location, self.ignored = location, []

    def handle_starttag(self, tag, attrs):
        if tag in ('script', 'style'):
            self.ignored.append(tag)
        for name, value in attrs:
            if name in ('title', 'aria-label', 'aria-description', 'placeholder', 'alt') and value:
                add(value, self.location)

    def handle_endtag(self, tag):
        if self.ignored and self.ignored[-1] == tag:
            self.ignored.pop()

    def handle_data(self, data):
        if not self.ignored and data.strip():
            add(data, self.location)


for folder in ('crates/core/src', 'apps/desktop/src-tauri/src'):
    for file in (ROOT / folder).rglob('*.rs'):
        scan_code(file)
for file in (ROOT / 'apps/desktop/web').glob('*.js'):
    if file.name not in ('i18n-catalog.js', 'i18n.js'):
        scan_code(file)
for file in (ROOT / 'apps/desktop/web').glob('*.html'):
    Markup(str(file.relative_to(ROOT))).feed(file.read_text(encoding='utf-8'))
for file in (ROOT / 'apps/macos/Sources/Aieyes').glob('*.swift'):
    if file.name != 'LocalizationCatalog.swift':
        scan_swift(file.read_text(encoding='utf-8'), str(file.relative_to(ROOT)))


def visit(value, location):
    if isinstance(value, str):
        add(value, location)
    elif isinstance(value, list):
        for item in value:
            visit(item, location)
    elif isinstance(value, dict):
        for item in value.values():
            visit(item, location)


for name in ('docs/core-contracts.json', 'docs/chart-palettes.json'):
    visit(json.loads((ROOT / name).read_text(encoding='utf-8')), name)
for value, locations in references.items():
    if value not in catalog:
        errors.append(f'Missing exact message {value!r}: {", ".join(sorted(locations))}')

# Web i18n rules (docs/ui-review-and-optimization-2026-10-26.md E-04): status attributes hold codes, translations
# made once at module level must follow language changes, and fragments are not glued into sentences.
# Existing concatenations are budgeted; lower the budget as they become whole-sentence templates with {0} slots.
CONCATENATION_BUDGET = 0
concatenations = []
for file in sorted((ROOT / 'apps/desktop/web').glob('*')):
    if file.suffix not in ('.js', '.html') or file.name in ('i18n-catalog.js', 'i18n.js'):
        continue
    lines = file.read_text(encoding='utf-8').splitlines()
    wrapped = any(line.startswith(('(() => {', '(function')) for line in lines[:5])
    for number, line in enumerate(lines, 1):
        location = f'{file.relative_to(ROOT)}:{number}'
        concatenations += [location] * len(re.findall(r'\+\s*AieyesI18n\.t\("', line))
        if re.search(r'data-status="[^"$]*[㐀-鿿]', line):
            errors.append(f'{location}: data-status holds display text; use a status code and translate the label')
        if (re.match(r'(const|let|var)\s+\w+\s*=\s*AieyesI18n\.t\(', line)
                or wrapped and re.match(r'  (const|let|var)\s+\w+\s*=\s*AieyesI18n\.t\(', line)) and 'aieyes:language' not in line:
            errors.append(f'{location}: a translation kept at module level misses language changes; translate where it is rendered')
if len(concatenations) > CONCATENATION_BUDGET:
    errors.append(f'{len(concatenations)} concatenated translations exceed the budget of {CONCATENATION_BUDGET}; '
                  f'use one whole-sentence template with {{0}} slots (latest: {concatenations[-1]})')
if errors:
    raise SystemExit('\n'.join(errors))
print(f'Localization coverage: {len(references)} exact source messages; {len(catalog)} translations and placeholders verified')
