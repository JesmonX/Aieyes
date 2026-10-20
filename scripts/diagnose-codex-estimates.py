#!/usr/bin/env python3
"""Read-only, anonymous evidence report. Never opens auth files or conversation logs."""
import argparse
import collections
import json
import pathlib
import sqlite3
import time


def diagnose(database, days=30):
    path = pathlib.Path(database).expanduser().resolve()
    db = sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)
    db.execute('PRAGMA query_only=ON')
    db.execute('BEGIN')
    try:
        tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        stored = db.execute("SELECT value FROM kv WHERE key='settings'").fetchone()
        settings = json.loads(stored[0]) if stored else {}
        sources = [s for s in settings.get('sources', []) if s.get('provider') == 'codex']
        accounts = [a for a in settings.get('accounts', []) if a.get('provider') == 'codex']
        since = int(time.time()) - days * 86400
        labels = {a['id']: 'account-' + str(i + 1) for i, a in enumerate(accounts)}
        report = dict(formatVersion=1, days=days, accountCount=len(accounts), sources=[], estimates=[])
        for i, source in enumerate(sources):
            sid = source['id']
            status = db.execute("SELECT value FROM kv WHERE key=?", ('automatic-status:' + sid,)).fetchone()
            status = json.loads(status[0]) if status else {}
            row = dict(source='source-' + str(i + 1), transport='ssh' if source.get('hostId') else 'local', shared=bool(source.get('codexHomeId')), automaticEnabled=bool((source.get('autoValuation') or {}).get('enabled')), state=status.get('state'), boundProfiles=sum(c.get('sourceId') == sid and bool(c.get('profileId')) for a in accounts for c in a.get('connections', [])))
            if 'codex_session_creators' in tables:
                creators, conflicts = db.execute('SELECT COUNT(*),COALESCE(SUM(conflict),0) FROM codex_session_creators WHERE source_id=?', (sid,)).fetchone()
                row.update(creatorSessions=creators, creatorConflicts=conflicts)
            if 'codex_account_timeline' in tables:
                observed, switches, gaps = db.execute("SELECT COUNT(*),COALESCE(SUM(kind='switch'),0),COALESCE(SUM(uncertain_from IS NOT NULL),0) FROM codex_account_timeline WHERE source_id=? AND stamp>=?", (sid, since)).fetchone()
                row.update(identityObservations=observed, managedSwitches=switches, uncertainIntervals=gaps)
            report['sources'].append(row)
        for (raw,) in db.execute("SELECT payload FROM estimates WHERE account_key LIKE 'codex:%' AND COALESCE(json_extract(payload,'$.checkpointAt'),0)>=? ORDER BY account_key,json_extract(payload,'$.startedAt')", (since,)):
            e = json.loads(raw)
            # Only an explicit allow-list leaves the database. Names, raw IDs,
            # paths, free-form notes, prices and arbitrary payload keys stay local.
            account = labels.get(e.get('accountKey', '').partition(':')[2], 'deleted-account')
            versions = collections.Counter(str(s.get('attributionVersion', 0)) for s in e.get('automaticSegments', []))
            report['estimates'].append(dict(account=account, mode=e.get('samplingMode', 'manual'), status=e.get('status'), calculationStatus=e.get('calculationStatus'), rollingExcluded=bool(e.get('rollingExcludedReason')), startedAt=e.get('startedAt'), checkpointAt=e.get('checkpointAt'), cost=e.get('cost'), consumedPercent=e.get('consumedPercent'), consumedCredits=e.get('consumedCredits'), attributionVersions=dict(versions), validSegments=sum(bool(s.get('valid', True)) for s in e.get('automaticSegments', []))))
        return report
    finally:
        db.rollback()
        db.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('database', type=pathlib.Path, help='Path to aieyes.sqlite')
    parser.add_argument('--days', type=int, default=30, choices=range(1, 366), metavar='1..365')
    args = parser.parse_args()
    print(json.dumps(diagnose(args.database, args.days), ensure_ascii=False, indent=2, allow_nan=False))
