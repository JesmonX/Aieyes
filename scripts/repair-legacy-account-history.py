"""Repair proven duplicate hashes after a legacy source/account migration.

Read-only by default. --apply backs up SQLite before one atomic repair. Source
aliases must be explicitly supplied; identical usage alone never proves identity.
"""
import argparse
import collections
import datetime
import hashlib
import json
import os
from pathlib import Path
import sqlite3


def event_id(provider, namespace, identity):
    return hashlib.sha256(f"{provider}:{namespace}:{identity}".encode()).hexdigest()


def plan(db, aliases, copied_sources=()):
    settings = json.loads(db.execute("SELECT value FROM kv WHERE key='settings'").fetchone()[0])
    sources = {s['id']: s for s in settings.get('sources', [])}
    for old, new in aliases.items():
        if old in sources or new not in sources or sources[new]['provider'] != 'codex':
            raise ValueError('Aliases must map a removed Codex source to a current Codex source')
    for old, new in copied_sources:
        if old not in aliases or new not in sources or sources[new]['provider'] != 'codex':
            raise ValueError('Copied sources must link an aliased legacy source to a current Codex source')
    deleted = {json.loads(raw)['id'] for (raw,) in db.execute(
        "SELECT value FROM kv WHERE key LIKE 'deleted-account:codex:%'")}
    rows = {}
    for row in db.execute("SELECT id,payload,cost,priced_tokens,price FROM events WHERE provider='codex'"):
        rows[row[0]] = (json.loads(row[1]), *row[2:])
    memberships = collections.defaultdict(set)
    for event, source in db.execute("SELECT event_id,source_id FROM event_sources"):
        memberships[event].add(source)
    legacy_sources = collections.defaultdict(set)
    for source, account in db.execute("SELECT source_id,account_id FROM source_identities WHERE provider='codex'"):
        if account in deleted:
            legacy_sources[account].add(source)
    parent = {key: key for key in rows}

    def find(key):
        while parent[key] != key:
            parent[key] = parent[parent[key]]
            key = parent[key]
        return key

    source_ids = {d[0]['sourceId'] for d in rows.values()} | set(aliases.values())
    namespaces = deleted | {f'source:{s}' for s in source_ids}
    for key, (event, *_) in rows.items():
        identity = event.get('importIdentity')
        if not identity:
            continue
        for namespace in namespaces:
            candidate = event_id('codex', namespace, identity)
            if candidate == key or candidate not in rows:
                continue
            old = rows[candidate][0]
            # Both namespaces must be deleted or shared history. Active accounts
            # remain isolated even if their session IDs have been copied.
            if any(e['accountId'] and e['accountId'] not in deleted for e in [event, old]):
                continue
            left = {aliases.get(s, s) for s in memberships[key] | legacy_sources[event['accountId']]}
            right = {aliases.get(s, s) for s in memberships[candidate] | legacy_sources[old['accountId']]}
            for old_source, copied_source in copied_sources:
                for observed in [left, right]:
                    if aliases[old_source] in observed:
                        observed.add(copied_source)
            if not left.intersection(right):
                continue
            if not event.get('sessionId') or any(event.get(field) != old.get(field) for field in
                ['sessionId', 'timestamp', 'model', 'tokens', 'attribution']):
                continue
            if old.get('importIdentity') not in (None, identity):
                continue
            parent[find(candidate)] = find(key)
    groups = collections.defaultdict(list)
    for key in rows:
        groups[find(key)].append(key)
    result = []
    for keys in groups.values():
        if len(keys) < 2:
            continue
        # Prefer an existing shared-history hash in a current source, so a new
        # import will naturally address the surviving record.
        keep = min(keys, key=lambda k: (bool(rows[k][0]['accountId']),
            rows[k][0]['sourceId'] not in sources, not bool(rows[k][0].get('importIdentity')), k))
        result.append((keep, [key for key in keys if key != keep]))
    return result, rows, memberships


def repair(db, aliases, groups, rows, memberships):
    for keep, duplicates in groups:
        event = rows[keep][0].copy()
        event['importIdentity'] = next(rows[k][0]['importIdentity'] for k in [keep] + duplicates
                                       if rows[k][0].get('importIdentity'))
        # Keep the earliest persisted frozen valuation, including an unpriced
        # record. Repairing identity must never recalculate historical prices.
        valuation = db.execute('SELECT cost,priced_tokens,price FROM events WHERE id IN (' +
                               ','.join('?' for _ in [keep] + duplicates) + ') ORDER BY rowid LIMIT 1',
                               [keep] + duplicates).fetchone()
        for key in [keep] + duplicates:
            for source in memberships[key]:
                for observed in {source, aliases.get(source, source)}:
                    db.execute('INSERT OR IGNORE INTO event_sources VALUES(?,?)', (keep, observed))
                    db.execute('INSERT OR IGNORE INTO source_identities VALUES(?,?,?)',
                               (observed, 'codex', event['accountId']))
        # Favor stronger known billing and interval evidence over missing data.
        for key in duplicates:
            old = rows[key][0]
            if old.get('billing', {}).get('category') == 'api':
                event['billing'] = old['billing']
            if old.get('intervalEvidence') == 'task-request-v2' and event.get('intervalEvidence') != 'task-request-v2':
                event['intervalStart'] = old.get('intervalStart')
                event['intervalEvidence'] = old['intervalEvidence']
            db.execute('DELETE FROM event_sources WHERE event_id=?', (key,))
            db.execute('DELETE FROM events WHERE id=?', (key,))
        db.execute('UPDATE events SET payload=?,cost=?,priced_tokens=?,price=? WHERE id=?',
                   (json.dumps(event, separators=(',', ':')), *valuation, keep))
    for old, new in aliases.items():
        db.execute('INSERT OR IGNORE INTO source_identities SELECT ?,provider,account_id FROM source_identities WHERE source_id=?', (new, old))
    settings = json.loads(db.execute("SELECT value FROM kv WHERE key='settings'").fetchone()[0])
    settings['historyShowDeletedAccounts'] = False
    db.execute("UPDATE kv SET value=? WHERE key='settings'", (json.dumps(settings),))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', required=True, type=Path)
    parser.add_argument('--source-alias', action='append', default=[], metavar='REMOVED=CURRENT')
    parser.add_argument('--copied-source', action='append', default=[], metavar='REMOVED=COPY',
                        help='Explicitly verified copied Codex sessions; affects identity checks only')
    parser.add_argument('--apply', action='store_true')
    args = parser.parse_args()
    aliases = dict(value.split('=', 1) for value in args.source_alias)
    copied_sources = [value.split('=', 1) for value in args.copied_source]
    db = sqlite3.connect(f'file:{args.database.resolve()}?mode={"rw" if args.apply else "ro"}', uri=True, timeout=30)
    db.execute('PRAGMA foreign_keys=ON')
    backup = None
    if args.apply:
        backup = args.database.with_name(args.database.name + '.' + datetime.datetime.now().strftime('%Y%m%d-%H%M%S-%f') + '.bak')
        fd = os.open(backup, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(fd)
        destination = sqlite3.connect(backup)
        try:
            db.backup(destination)
            destination.execute('PRAGMA journal_mode=DELETE')
        finally:
            destination.close()
        db.execute('BEGIN IMMEDIATE')
    try:
        groups, rows, memberships = plan(db, aliases, copied_sources)
        extra = sum(len(duplicates) for _, duplicates in groups)
        if args.apply:
            repair(db, aliases, groups, rows, memberships)
            if db.execute('PRAGMA foreign_key_check').fetchall():
                raise ValueError('Foreign key validation failed')
            db.commit()
        print(json.dumps({'applied': args.apply, 'duplicateGroups': len(groups), 'removedRows': extra,
                          'backup': str(backup) if backup else None}, ensure_ascii=False))
    finally:
        db.close()


if __name__ == '__main__':
    main()
