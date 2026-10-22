#!/usr/bin/env python3
"""Deterministic product-preview RPC. Never reads user data or contacts providers."""
import copy
import base64
import json
import os
from pathlib import Path
import subprocess
import sys
from datetime import date, timedelta

ROOT = Path(__file__).resolve().parents[2]
STAMP = 1792584060
env = dict(os.environ, AIEYES_UI_SCENARIO="workspace", AIEYES_UI_STAMP=str(STAMP))
data = json.loads(subprocess.check_output([sys.executable, str(ROOT / "scripts/preview-quota-core.py"), "--fixture"], env=env))
settings, dashboard = data["settings"], data["dashboard"]
for key in ["accounts", "sources"]:
    settings[key] = [r for r in settings[key] if r["provider"] != "deepseek"]
dashboard["quotas"] = [q for q in dashboard["quotas"] if q["provider"] != "deepseek"]
names = {"codex": "Personal workspace", "codex-team": "Studio workspace", "claude": "Claude workspace", "antigravity": "Gemini workspace", "deepseek": "DeepSeek API"}
for account in settings["accounts"]:
    account["name"] = names[account["id"]]
for source in settings["sources"]:
    source.update(name=names[source["accountId"]], path="/demo/agents/" + source["provider"])
    if source["provider"] == "antigravity":
        source["hostId"] = "preview-host"
for quota in dashboard["quotas"]:
    quota["name"] = names[quota["accountId"]]
    for window in quota["windows"]:
        window["resetsAt"] = STAMP + (7200 if window["windowMinutes"] == 300 else 259200)
settings.update(appearance=dict(language="en", theme="dark", accent="indigo", chartPalette="classic", chartContrast="high"), localDeviceName="Mac Studio", panelQuotaAccounts=["codex:codex", "claude:claude", "antigravity:antigravity"], panelQuotaSelectionMode="manual")
providers = json.loads((ROOT / "docs/core-contracts.json").read_text())["providers"]
settings["agents"] = [dict(provider=p["id"], enabled=p["id"] in {"codex", "claude", "antigravity", "opencode", "kimi", "pi"}, machineIds=["local", "preview-host"] if p["id"] in {"codex", "claude", "antigravity"} else ["local"]) for p in providers]
settings["codexApis"] = [dict(id="demo-api", sourceId="codex-source", name="Studio Responses API", baseUrl="https://api.example.invalid/v1", model="gpt-5.1-codex", effort="medium", revision="preview", prices=[])]
host = settings["hosts"][0]
host.update(name="GPU Lab", target="gpu-lab.example.invalid", metrics=["cpu", "memory", "gpu", "filesystems", "disk", "network"], details=["gpuMemory", "gpuThermals", "fsAvailable", "diskIops", "networkTotals"])
GiB = 1024 ** 3
sample = data["hosts"][0]["sample"]
sample.update(timestamp=STAMP, uptime=527040, load=[1.4, 1.2, 0.9], cpu=[dict(id="cpu", utilization=42)], memory=dict(total=128*GiB, available=77*GiB, cached=12*GiB, buffers=GiB, swapTotal=0, swapFree=0), gpu=[dict(id="0", name="NVIDIA A100 80GB", utilization=76, memoryUsedMiB=56320, memoryTotalMiB=81920, temperature=64, powerWatts=236), dict(id="1", name="NVIDIA RTX 4090", utilization=34, memoryUsedMiB=8192, memoryTotalMiB=24576, temperature=52, powerWatts=147)], filesystems=[dict(id="/", used=82*GiB, total=256*GiB, available=174*GiB, type="ext4"), dict(id="/datasets", used=1280*GiB, total=2048*GiB, available=768*GiB, type="ext4")], disk=[dict(id="nvme0n1", readBytesPerSecond=82*1024**2, writeBytesPerSecond=24*1024**2, readsPerSecond=340, writesPerSecond=120, busyPercent=28)], network=[dict(id="eth0", rxBytes=48*GiB, txBytes=12*GiB, rxBytesPerSecond=8*1024**2, txBytesPerSecond=2*1024**2)])

def usage(total, cost, key=""):
    return dict(key=key, total=total, cost=cost, pricedTokens=total, events=max(1, total//28000), tokens=dict(input=int(total*.42), output=int(total*.18), cacheRead=int(total*.36), cacheWrite=int(total*.04), reasoning=0))

models = [("gpt-5.1-codex", .46, 17.82), ("claude-sonnet-4.5", .34, 24.63), ("gemini-2.5-pro", .20, 6.31)]
dashboard["summary"] = usage(2840000, 48.76)
dashboard["models"] = [usage(int(2840000*ratio), cost, key) for key, ratio, cost in models]
dashboard["modelOptions"] = [m[0] for m in models]
today = date(2026, 10, 21)
dashboard["heatmap"] = [usage(0 if i%11 == 0 else 120000+(i*71239)%2800000, round(2+(i%23)*1.7, 2), (today-timedelta(days=364-i)).isoformat()) for i in range(365)]
dashboard["trendDays"] = [usage(total, round(total*.00001717, 2), (today-timedelta(days=6-i)).isoformat()) for i,total in enumerate([1240000, 1850000, 1560000, 2310000, 1980000, 2570000, 2840000])]
dashboard["dayModels"] = [dict(day=day["key"], model=key, usage=usage(int(day["total"]*ratio), round(day["cost"]*ratio, 2))) for day in dashboard["trendDays"] for key,ratio,_ in models]
dashboard["sources"] = [dict(source, status=dict(updatedAt=STAMP)) for source in settings["sources"]]
for record in dashboard["quotaEstimates"] + dashboard["creditEstimates"]:
    record.update(sourceNames=["Personal workspace"], calculationNote="Rolling estimate · 12 observed quota windows", totalTokens=2840000, pricedTokens=2840000, cost=48.76)
for record in dashboard["quotaEstimates"]:
    if record["accountKey"] == "codex:codex":
        record.update(fiveHourValue=7.1, weeklyValue=43.2, weeklyDirectValue=43.2, weeklyRatioValue=43.2,
                      cost=.852, totalTokens=50000, pricedTokens=50000)
        record["capacity"].update(ratio=43.2/7.1, weeklyPercent=.852/43.2*100)
for record in dashboard["creditEstimates"]:
    if record["accountKey"] == "codex:codex":
        record.update(valuePer1000=40, cost=.4, totalTokens=25000, pricedTokens=25000,
                      weeklyDirectValue=None, weeklyRatioValue=None)
for row in data["wakeups"]:
    row["task"].update(name="Morning workspace", times=["08:30", "13:30"], model="gpt-5.1-codex", effort="medium")
    row["deployment"].update(task=row["task"], target="Local device", timezone="UTC+08:00")
data["sessions"] = dict(sessions=[dict(id="preview-"+provider, sourceId=provider+"-source", source=label, provider=provider, machine="GPU Lab" if provider == "antigravity" else "Local device", machineId="preview-host" if provider == "antigravity" else "local", phase=phase, updatedAt=STAMP) for provider,label,phase in [("codex", "Personal workspace", "working"), ("claude", "Claude workspace", "needsInput"), ("antigravity", "Gemini workspace", "tool")]], sources=[], warnings=[], unavailable=False)
data["network"] = dict(testedAt=STAMP, mode="system", averageMs=32, status="ok", sites=[dict(url="https://api.github.com", latencyMs=32)])
data["prices"] = [dict(id=key, name=key, input=.000003, output=.000015, cacheRead=.0000003, cacheWrite=.000003, fetchedAt=STAMP) for key,_,_ in models]
sprite = ROOT / ".local/website/pet-sheet.png"
data["petLibrary"] = {"pets": [], "warnings": []}
if sprite.exists():
    dimensions = json.loads((sprite.parent / "pet-info.json").read_text())
    pet = dict(id="yuexinmiao1", displayName="Yuexinmiao", contentHash="preview-original-kiffin", spritePath=str(sprite), spriteVersionNumber=1, rows=9, cellWidth=dimensions["width"]//8, cellHeight=dimensions["height"]//9, libraryId="preview-yuexinmiao", creator="kiffin", source="codex-pet.org", importedAt="2026-10-21T12:00:00Z")
    data["petLibrary"] = dict(pets=[pet], selectedLibraryId=pet["libraryId"], warnings=[])
    data["petAsset"] = dict(pet=pet, imageDataUrl="data:image/png;base64,"+base64.b64encode(sprite.read_bytes()).decode())
if "--fixture" in sys.argv:
    print(json.dumps(data)); sys.exit(0)
rpc = {"hello": {"version": "preview"}, "settings.get": settings, "dashboard": dashboard, "dashboard.summary": dashboard, "dashboard.status": dashboard, "prices.list": data["prices"], "hosts.sample": data["hosts"], "wakeups.list": data["wakeups"], "quotas.refresh": dashboard["quotas"], "sessions.list": data["sessions"], "sources.scan": [], "accounts.status.get": data["accountStatuses"], "accounts.cleanup.list": [], "accounts.targets": [], "network.status": data["network"], "network.test": data["network"], "pets.list": data["petLibrary"], "pets.current": data.get("petAsset", {}), "settings.patch": settings, "accounts.deployments.sync": {"failedTasks": []}, "codexApi.status": {"configRevision": "preview", "installed": True, "ready": True}}
rpc["wakeups.status"] = dict(installed=True, enabled=True, timezone="UTC+08:00", nextRunAt=1792629000, history=[dict(startedAt=1792542600, endedAt=1792542602, status="success", model="gpt-5.1-codex", effort="medium", message="Completed")])
for line in sys.stdin:
    request = json.loads(line)
    print(json.dumps(dict(jsonrpc="2.0", id=request["id"], result=copy.deepcopy(rpc.get(request["method"], {})))), flush=True)
