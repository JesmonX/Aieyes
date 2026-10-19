#!/usr/bin/env python3
"""Read real client directories into an isolated Aieyes DB and verify replay.

Does not start clients, query subscriptions, or modify their configuration.
Usage: python3 scripts/verify-native-agent-history.py kimi=/path/to/.kimi-code
"""
import argparse
import json
import pathlib
import sqlite3
import subprocess
import tempfile

PROJECT = pathlib.Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sources", nargs="+", metavar="PROVIDER=PATH")
    parser.add_argument("--core", type=pathlib.Path, default=PROJECT / "target/debug/aieyes-core")
    args = parser.parse_args()
    sources = []
    for entry in args.sources:
        provider, path = entry.split("=", 1)
        sources.append(dict(id="verify-" + provider, provider=provider, name=provider,
                            path=str(pathlib.Path(path).expanduser().resolve()), enabled=True))
    with tempfile.TemporaryDirectory(prefix="aieyes-native-verify-") as root:
        def rpc(method, params=None):
            result = subprocess.run([str(args.core), "--data-dir", root, "--call", method,
                                     json.dumps(params or {}, ensure_ascii=False)],
                                    capture_output=True, text=True, timeout=180, check=True,
                                    stdin=subprocess.DEVNULL)
            return json.loads(result.stdout)
        settings = rpc("settings.get")
        settings.update(sources=sources, accounts=[], hosts=[])
        for agent in settings["agents"]:
            agent.update(enabled=any(s["provider"] == agent["provider"] for s in sources), machineIds=["local"])
        rpc("settings.save", settings)
        first = rpc("sources.scan")
        if len(first) != len(sources):
            raise RuntimeError("Not all requested sources were scanned")
        for item in first:
            detail=item.get("result",item)
            if item.get("error") or detail.get("error") or item.get("partial") or detail.get("partial"):
                raise RuntimeError(f"Initial scan was incomplete: {item}")
        repeated = rpc("sources.scan")
        replayed = rpc("sources.rescan")
        # Verify the durable records directly, independent of dashboard pricing.
        database = next(pathlib.Path(root).glob("*.sqlite"))
        with sqlite3.connect(database) as db:
            events = [json.loads(row[0]) for row in db.execute("SELECT payload FROM events")]
        report = {}
        for source in sources:
            provider = source["provider"]
            rows = [e for e in events if e["provider"] == provider]
            if not rows:
                raise RuntimeError(f"{provider}: no readable completed usage; scan results: {first}")
            report[provider] = dict(events=len(rows), sessions=len({e["sessionId"] for e in rows}),
                                    models=sorted({e["model"] for e in rows}),
                                    tokens={key:sum(e["tokens"][key] for e in rows)
                                            for key in ["input", "output", "cacheRead", "cacheWrite", "reasoning"]})
            summary = rpc("dashboard.summary", {"provider": provider, "dateRange": {"kind": "all"}})["summary"]
            if provider in summary.get("nativeCredits", {}):
                # Use the dashboard's settled ledger: parent team mirrors and
                # parallel tool outputs must not be summed as independent charges.
                report[provider]["nativeCredits"] = summary["nativeCredits"][provider]
        for label, result in [("repeat", repeated), ("replay", replayed)]:
            records = result if isinstance(result, list) else result.get("sources", [])
            for item in records:
                detail = item.get("result", item)
                if item.get("error") or detail.get("error") or detail.get("newEvents", 0):
                    raise RuntimeError(f"{label} was not idempotent: {item}")
        print(json.dumps(dict(report=report, first=first, repeated=repeated, replayed=replayed), indent=2))


if __name__ == "__main__":
    main()
