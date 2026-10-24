#!/usr/bin/env python3
"""Read-only, anonymous evidence report. Never opens auth files or conversation logs."""
import argparse
import collections
import json
import pathlib
import sqlite3
import time


def session_evidence(db, session_id, sources, accounts):
    known_sources = {s['id'] for s in sources}
    identities = {a.get('identityKey'): ('account-' + str(i + 1), 'codex:' + a['id']) for i, a in enumerate(accounts) if a.get('identityKey')}
    counts = collections.Counter()
    costs = collections.defaultdict(float)
    gaps = {}
    provisional = collections.Counter()
    rows = db.execute("SELECT payload,cost FROM events WHERE provider='codex' AND json_extract(payload,'$.sessionId')=? ORDER BY stamp", (session_id,)).fetchall()
    signature_index = collections.defaultdict(list)
    if rows:
        first = min(json.loads(r[0]).get('intervalStart', json.loads(r[0])['timestamp']) for r in rows)
        last = max(json.loads(r[0])['timestamp'] for r in rows)
        seen_signatures = set()
        for identity, (label, account_key) in identities.items():
            for (raw,) in db.execute('SELECT payload FROM quota_history WHERE account_key=? AND stamp BETWEEN ? AND ?', (account_key,first-86400,last+86400)):
                q = json.loads(raw)
                if q.get('error') or q.get('origin') == 'log': continue
                windows = tuple(sorted((w['windowMinutes'],w.get('resetsAt')) for w in q.get('windows', []) if w.get('windowMinutes') is not None))
                key = (label,q.get('plan'),windows)
                if key not in seen_signatures:
                    seen_signatures.add(key);signature_index[q.get('plan')].append((label,dict(windows)))
    def cycle_candidates(e):
        candidates = None
        for (raw,) in db.execute('SELECT payload FROM codex_request_limits WHERE request_id=? AND source_id IN (SELECT source_id FROM event_sources WHERE event_id=?)', (e.get('importIdentity') or e['id'],e['id'])):
            request = json.loads(raw)
            windows = {w['windowMinutes']:w.get('resetsAt') for w in request.get('windows', []) if w.get('windowMinutes') is not None}
            matches = {label for label, known in signature_index[request.get('plan')] if windows and all(value is not None and known.get(period) is not None and abs(known[period]-value)<=10 for period,value in windows.items())}
            candidates = matches if candidates is None else candidates & matches
        return sorted(candidates or [])
    def attribution(e):
        start, end = e.get('intervalStart'), e['timestamp']
        if start is None: return None, 'missingRequestStart'
        accepted = None
        members = [r[0] for r in db.execute('SELECT source_id FROM event_sources WHERE event_id=?', (e['id'],)) if r[0] in known_sources]
        for sid in members:
            creator = db.execute('SELECT identity_key,created_at,conflict FROM codex_session_creators WHERE source_id=? AND session_id=?', (sid,session_id)).fetchone()
            if creator and creator[2]: return None, 'creatorConflict'
            before = db.execute('SELECT stamp,identity_key FROM codex_account_timeline WHERE source_id=? AND stamp<=? ORDER BY stamp DESC,id DESC LIMIT 1', (sid,start)).fetchone()
            after = db.execute('SELECT stamp,identity_key FROM codex_account_timeline WHERE source_id=? AND stamp>=? ORDER BY stamp,id LIMIT 1', (sid,end)).fetchone()
            if not before or not after: return None, 'timelineUncovered'
            if not before[1] or before[1] != after[1]: return None, 'identityEndpointsDiffer'
            if after[0]-before[0] > 300:
                gap = gaps.setdefault((before[0],after[0]), dict(leftObservation=before[0],rightObservation=after[0],events=0,firstRequestStart=start,lastRequestEnd=end))
                gap['events'] += 1;gap['firstRequestStart'] = min(gap['firstRequestStart'],start);gap['lastRequestEnd'] = max(gap['lastRequestEnd'],end)
                return None, 'identityObservationGap'
            conflict = db.execute('SELECT EXISTS(SELECT 1 FROM codex_account_timeline WHERE source_id=? AND ((stamp>? AND stamp<=? AND identity_key<>?) OR (uncertain_from IS NOT NULL AND uncertain_from<? AND stamp>?)))', (sid,start,end,before[1],end,start)).fetchone()[0]
            if conflict: return None, 'switchOrUncertainInterval'
            if creator and before[0] <= creator[1] <= end and creator[0] != before[1]: return None, 'newCreatorMismatch'
            if accepted and accepted != before[1]: return None, 'duplicateSourceConflict'
            accepted = before[1]
        return accepted, 'timelineCovered' if accepted else 'timelineUncovered'
    for raw, cost in rows:
        event = json.loads(raw)
        identity, reason = attribution(event)
        if identity in identities:
            label, key = identities[identity]
            fingerprints = [json.loads(r[0]) for r in db.execute('SELECT payload FROM codex_request_limits WHERE request_id=? AND source_id IN (SELECT source_id FROM event_sources WHERE event_id=?)', (event.get('importIdentity') or event['id'],event['id']))]
            snapshots = [json.loads(r[0]) for r in db.execute('SELECT payload FROM quota_history WHERE account_key=? AND stamp BETWEEN ? AND ?', (key,event['intervalStart']-120,event['timestamp']+120))]
            compatible = all(any(not q.get('error') and q.get('plan') == request.get('plan') and any(w.get('windowMinutes') is not None for w in request.get('windows', [])) and all(any(w.get('windowMinutes') == v.get('windowMinutes') and w.get('resetsAt') is not None and v.get('resetsAt') is not None and abs(w['resetsAt']-v['resetsAt']) <= 10 for v in q.get('windows', [])) for w in request.get('windows', []) if w.get('windowMinutes') is not None) for q in snapshots) for request in fingerprints)
            reason = label + ':' + ('fingerprintMatched' if compatible and fingerprints else 'fingerprintMissing' if not fingerprints else 'fingerprintMismatch')
        if identity is None:
            candidates = cycle_candidates(event)
            provisional[candidates[0] if len(candidates)==1 else 'ambiguous' if candidates else 'unmatched'] += 1
        counts[reason] += 1
        costs[reason] += cost
    return dict(eventCount=len(rows), classifications=[dict(reason=reason, events=count, apiEquivalentCost=costs[reason]) for reason,count in sorted(counts.items())], observationGaps=list(gaps.values()), provisionalFingerprintCandidates=dict(provisional), provisionalCandidatesAreValuationEligible=False)


def diagnose(database, days=30, session_id=None):
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
        if session_id:
            report['session'] = session_evidence(db, session_id, sources, accounts)
        for estimate, (raw,) in zip(report['estimates'], db.execute("SELECT payload FROM estimates WHERE account_key LIKE 'codex:%' AND COALESCE(json_extract(payload,'$.checkpointAt'),0)>=? ORDER BY account_key,json_extract(payload,'$.startedAt')", (since,))):
            pools = {}
            for segment in json.loads(raw).get('automaticSegments', []):
                if not segment.get('valid', True): continue
                pool_name = segment.get('pool', 'primary')
                pool = pools.setdefault(pool_name if pool_name in ('primary','weekly') else 'unknown', dict(segments=0,cost=0,consumed=0))
                pool['segments'] += 1; pool['cost'] += segment.get('cost', 0); pool['consumed'] += segment.get('consumed', 0)
            for pool in pools.values():
                pool['fullQuotaValue'] = 100*pool['cost']/pool['consumed'] if pool['consumed'] > 0 else None
            estimate['pools'] = pools
        return report
    finally:
        db.rollback()
        db.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('database', type=pathlib.Path, help='Path to aieyes.sqlite')
    parser.add_argument('--days', type=int, default=30, choices=range(1, 366), metavar='1..365')
    parser.add_argument('--session-id', help='Inspect normalized session usage metadata; never reads conversation contents')
    args = parser.parse_args()
    print(json.dumps(diagnose(args.database, args.days, args.session_id), ensure_ascii=False, indent=2, allow_nan=False))
