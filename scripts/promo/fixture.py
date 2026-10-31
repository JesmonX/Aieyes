#!/usr/bin/env python3
"""Deterministic promo-video RPC. Extends the website fixture; never reads user data or contacts providers.

Run with --fixture to print the data set. Without it, the script answers the
engine's line-based JSON-RPC on stdin, like scripts/website/fixture.py.
"""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
BASE = 1792584060  # 2026-10-21 20:01 UTC+08:00, shared with the website renders
# The workday video renders one day at several moments: AIEYES_PROMO_STATE names the
# moment and AIEYES_PROMO_STAMP its clock. Every website timestamp moves with the clock.
STAMP = int(os.environ.get("AIEYES_PROMO_STAMP", BASE))
STATE = os.environ.get("AIEYES_PROMO_STATE", "")
DAY = BASE - 72060  # 2026-10-21 00:00 UTC+08:00
def at(day, hour, minute=0): return DAY + day * 86400 + hour * 3600 + minute * 60
LANGUAGE = os.environ.get("AIEYES_LANGUAGE", "en")
if LANGUAGE not in ("en", "zh-CN"): raise ValueError("Unsupported preview language")
ZH = LANGUAGE == "zh-CN"
def T(en, zh): return zh if ZH else en
GiB, TiB, MiB = 1024 ** 3, 1024 ** 4, 1024 ** 2


def shift(value, key=""):
    if isinstance(value, dict): return {k: shift(v, k) for k, v in value.items()}
    if isinstance(value, list): return [shift(v, key) for v in value]
    if (key.endswith("At") or key == "timestamp") and type(value) in (int, float) and abs(value - BASE) < 400 * 86400:
        return value + STAMP - BASE
    return value


data = shift(json.loads(subprocess.check_output([sys.executable, str(ROOT / "scripts/website/fixture.py"), "--fixture"], env=dict(os.environ, AIEYES_LANGUAGE=LANGUAGE))))
settings, dashboard = data["settings"], data["dashboard"]
settings["appearance"]["theme"] = os.environ.get("AIEYES_PROMO_THEME", "dark")
names = {"codex": T("Personal workspace", "个人工作区"), "codex-team": T("Studio workspace", "团队工作区"),
         "claude": T("Claude workspace", "Claude 工作区"), "antigravity": T("Gemini workspace", "Gemini 工作区")}

# Ten overview bars in the app's three remaining-quota colors, 7 : 2 : 1 —
# blue above 30%, orange at 10–30% (Claude 5h, Gemini 7d), red below 10% (Gemini 5h).
windows = {"codex": [(35, 7200), (42, 259200)], "codex-team": [(12, 15120), (27, 432000)], "claude": [(81, 2820), (38, 345600)]}
for quota in dashboard["quotas"]:
    for window, (used, reset) in zip(quota["windows"], windows.get(quota["accountId"], [])):
        window.update(usedPercent=used, resetsAt=STAMP + reset)
    for window in quota["windows"]:
        used = {"Gemini Models:300": 93, "Gemini Models:10080": 78, "Claude and GPT Models:10080": 46}.get(window["id"])
        if used is not None:
            window["usedPercent"] = used
    if quota["accountId"] == "codex-team":
        quota["credits"] = dict(hasCredits=True, unlimited=False, balance="2480.5")
for record in dashboard["quotaEstimates"] + dashboard["creditEstimates"]:
    record["sourceNames"] = [names["codex"]]
    record["calculationNote"] = T("Rolling estimate · 12 observed quota windows", "滚动估值 · 已观测 12 个额度区间")

# Automatic valuation is on for both Codex sources, with a rolling summary and sample history.
rolling = T("Rolling · last 90 days · weight halves every 7 days · 12 valid quota segments", "滚动估值 · 最近 90 天 · 权重每 7 天减半 · 12 个有效额度片段")
for source in settings["sources"]:
    if source["provider"] == "codex":
        source["autoValuation"] = dict(enabled=True, consentVersion=1, mode="automatic")
for source in dashboard["sources"]:
    if source["provider"] == "codex":
        source["autoValuationStatus"] = dict(state="sampling", note=T("Sampling · 12 quota segments in the last 90 days", "采样中 · 最近 90 天 12 个额度片段"), accountKey="codex:" + source["accountId"], updatedAt=STAMP)
quota_base = next(r for r in dashboard["quotaEstimates"] if r["accountKey"] == "codex:codex")
credit_base = next(r for r in dashboard["creditEstimates"] if r["accountKey"] == "codex:codex")
dashboard["automaticEstimateSummaries"] = [
    dict(quota_base, id="rolling-quota", kind="quota", samplingMode="rolling", status="active", sampleCount=12, lookbackDays=90, halfLifeDays=7, calculationNote=rolling),
    dict(credit_base, id="rolling-credits", kind="credits", samplingMode="rolling", status="active", sampleCount=7, lookbackDays=90, halfLifeDays=7, calculationNote=rolling)]
history = [dict(quota_base, id=f"auto-{index}", samplingMode="automatic", status="completed", startedAt=STAMP - day * 86400 - 18000, checkpointAt=STAMP - day * 86400,
                endedAt=STAMP - day * 86400, fiveHourValue=value, calculationNote=rolling)
           for index, (day, value) in enumerate([(1, 7.32), (2, 6.95), (4, 7.18)])]
data["quotaHistory"] = dashboard["quotaEstimates"] + history
data["creditHistory"] = dashboard["creditEstimates"]

# Local Mac plus three typical remote machines, each with its own metric choice.
local_metrics = ["cpu", "memory", "filesystems", "network"]
settings["localMonitor"] = dict(enabled=True, metrics=local_metrics, devices=[], details=["uptime", "memoryCache", "fsAvailable", "networkTotals"])
gpu = settings["hosts"][0]
gpu.update(name=T("GPU Lab", "GPU 训练机"), target="ubuntu@10.0.8.21", metrics=["cpu", "memory", "gpu", "filesystems", "disk", "network"],
           details=["gpuMemory", "gpuThermals", "fsAvailable", "diskIops", "networkTotals"])
vps = dict(gpu, id="vps-host", name=T("Tokyo VPS", "东京 VPS"), target="root@203.0.113.42", metrics=["cpu", "memory", "filesystems", "network"],
           details=["uptime", "swap", "fsAvailable", "networkTotals", "networkErrors"])
nas = dict(gpu, id="nas-host", name=T("Home NAS", "家庭 NAS"), target="admin@192.168.1.20", metrics=["cpu", "memory", "filesystems", "disk", "network"],
           details=["fsAvailable", "fsType", "diskBusy", "diskIops", "networkTotals"])
settings["hosts"] = [gpu, vps, nas]

def sample(**values):
    return dict(dict(timestamp=STAMP, errors={}, load=[]), **values)
gpus = [(87, 68608, 71, 612), (84, 66560, 69, 598), (76, 57344, 66, 541), (23, 20480, 48, 182)]
data["hosts"] = [
    dict(id="local", name=T("Local", "本机"), sample=sample(uptime=691200, load=[3.18, 2.94, 2.61], cpu=[dict(id="cpu", utilization=23)],
         memory=dict(total=64 * GiB, available=27 * GiB, used=37 * GiB, swapTotal=0, swapFree=0),
         filesystems=[dict(id="/", device="Macintosh HD", type="apfs", total=2000 * GiB, available=1180 * GiB, used=820 * GiB)],
         network=[dict(id="en0", rxBytes=412 * GiB, txBytes=96 * GiB, rxBytesPerSecond=3.4 * MiB, txBytesPerSecond=0.7 * MiB)])),
    dict(id=gpu["id"], name=gpu["name"], sample=sample(uptime=1296000, load=[18.4, 17.9, 16.2], cpu=[dict(id="cpu", utilization=64)],
         memory=dict(total=512 * GiB, available=188 * GiB, cached=96 * GiB, buffers=2 * GiB, swapTotal=0, swapFree=0),
         gpu=[dict(id=str(i), name="NVIDIA H100 80GB", utilization=u, memoryUsedMiB=m, memoryTotalMiB=81920, temperature=t, powerWatts=p) for i, (u, m, t, p) in enumerate(gpus)],
         filesystems=[dict(id="/", used=412 * GiB, total=1024 * GiB, available=612 * GiB, type="ext4"),
                      dict(id="/data", used=int(11.6 * TiB), total=int(15.4 * TiB), available=int(3.8 * TiB), type="xfs")],
         disk=[dict(id="nvme0n1", readBytesPerSecond=1450 * MiB, writeBytesPerSecond=380 * MiB, readsPerSecond=11800, writesPerSecond=3100, busyPercent=71)],
         network=[dict(id="ens1f0", rxBytes=38 * TiB, txBytes=6 * TiB, rxBytesPerSecond=96 * MiB, txBytesPerSecond=12 * MiB)])),
    dict(id=vps["id"], name=vps["name"], sample=sample(uptime=8467200, load=[0.32, 0.28, 0.25], cpu=[dict(id="cpu", utilization=18)],
         memory=dict(total=4 * GiB, available=int(1.7 * GiB), cached=int(0.9 * GiB), buffers=int(0.1 * GiB), swapTotal=2 * GiB, swapFree=int(1.8 * GiB)),
         filesystems=[dict(id="/", used=31 * GiB, total=80 * GiB, available=49 * GiB, type="ext4")],
         network=[dict(id="eth0", rxBytes=int(1.8 * TiB), txBytes=int(4.6 * TiB), rxBytesPerSecond=1.4 * MiB, txBytesPerSecond=3.6 * MiB, rxErrors=0, txErrors=0, rxDrops=0, txDrops=0)])),
    dict(id=nas["id"], name=nas["name"], sample=sample(uptime=3456000, load=[1.1, 0.9, 0.8], cpu=[dict(id="cpu", utilization=9)],
         memory=dict(total=16 * GiB, available=int(9.6 * GiB), cached=int(4.8 * GiB), buffers=int(0.3 * GiB), swapTotal=4 * GiB, swapFree=4 * GiB),
         filesystems=[dict(id="/volume1", used=int(13.1 * TiB), total=int(21.8 * TiB), available=int(8.7 * TiB), type="btrfs"),
                      dict(id="/volume2", used=int(5.2 * TiB), total=int(10.9 * TiB), available=int(5.7 * TiB), type="btrfs")],
         disk=[dict(id="sd" + c, readBytesPerSecond=r * MiB, writeBytesPerSecond=w * MiB, readsPerSecond=r * 9, writesPerSecond=w * 7, busyPercent=b)
               for c, r, w, b in [("a", 118, 22, 46), ("b", 116, 21, 44), ("c", 37, 64, 31), ("d", 35, 63, 29)]],
         network=[dict(id="eth0", rxBytes=96 * TiB // 10, txBytes=31 * TiB // 10, rxBytesPerSecond=42 * MiB, txBytesPerSecond=6 * MiB)])),
]

# Sessions across this Mac and remote machines, one in every attention state.
local, remote = T("Local", "本机"), gpu["name"]
data["sessions"] = dict(sessions=[dict(id="promo-" + key, sourceId=source, source=names[account], provider=provider, machine=machine,
                                       machineId=machine_id, phase=phase, updatedAt=STAMP - age)
                                  for key, source, account, provider, machine, machine_id, phase, age in [
                                      ("codex", "codex-source", "codex", "codex", local, "local", "working", 4),
                                      ("claude", "claude-source", "claude", "claude", local, "local", "needsInput", 22),
                                      ("agy", "antigravity-source", "antigravity", "antigravity", remote, gpu["id"], "tool", 9),
                                      ("team", "codex-team-source", "codex-team", "codex", local, "local", "complete", 140)]],
                        sources=[], warnings=[], unavailable=False)

# Two wakeup tasks: chained 5h windows on this Mac, and a lunch-time Claude wakeup.
def wake(id, name, source, times, model, effort, target):
    task = dict(id=id, name=name, sourceId=source, times=times, model=model, effort=effort, prompt="Hi. Reply only OK. Do not use any tools.", binary="")
    return dict(task=task, deployment=dict(task=task, deployedAt=STAMP - 86400 * 3, enabled=True, state="deployed", timezone="UTC+08:00", target=target, changed=False))
data["wakeups"] = [wake("wake-morning", T("Before work", "开工前唤醒"), "codex-source", ["06:30", "11:30", "16:30"], "gpt-5.1-codex-mini", "low", local),
                   wake("wake-claude", T("Claude lunch break", "Claude 午间唤醒"), "claude-source", ["12:10"], "claude-haiku-4-5", "", local)]
def runs(times, model, effort):
    day = 86400
    return [dict(startedAt=start, endedAt=start + 3, status="success", model=model, effort=effort)
            for start in sorted((1792535400 + offset - back for offset in times for back in (0, day)), reverse=True) if start < STAMP]
data["wakeupStatuses"] = {
    "wake-morning": dict(installed=True, enabled=True, timezone="UTC+08:00", nextRunAt=1792621800, history=runs([0, 18000, 36000], "gpt-5.1-codex-mini", "low")),
    "wake-claude": dict(installed=True, enabled=True, timezone="UTC+08:00", nextRunAt=1792642200, history=runs([20400], "claude-haiku-4-5", "")),
}

# The Personal account is signed in on this Mac but the GPU Lab uses it now.
data["accountTargets"] = [
    dict(accountKey="codex:codex", accountName=names["codex"], provider="codex", sourceId="codex-source", sourceName=names["codex"], path="~/.codex",
         machineId="local", machineName=local, current=False, credential=True, checkedAt=STAMP, canWake=True, canSwitch=True, bound=True),
    dict(accountKey="codex:codex", accountName=names["codex"], provider="codex", sourceId="codex-gpu-source", sourceName=names["codex"], path="/home/ubuntu/.codex",
         machineId=gpu["id"], machineName=remote, current=True, credential=True, checkedAt=STAMP, canWake=True, canSwitch=True, bound=True),
]


def workday():
    """One Codex user's day: the personal account runs low and the studio account takes over at 10:45,
    both accounts reset early at 14:05, training waits for a busy GPU box, and the week gets wakeups."""
    personal, team = names["codex"], names["codex-team"]
    switched = STATE != "morning"
    for source in settings["sources"] + dashboard["sources"]:
        if source["provider"] == "codex":
            source.update(path="~/.codex", name=T("This Mac", "本机"))
    # The day's usage grows with the clock.
    k = {"morning": .38, "switched": .4, "afternoon": .79}.get(STATE, 1)
    summary = dashboard["summary"]
    for key in ("total", "pricedTokens", "events"):
        summary[key] = int(summary[key] * k)
    summary["cost"] = round(summary["cost"] * k, 2)
    summary["tokens"] = {key: int(value * k) for key, value in summary["tokens"].items()}
    # A single Codex session on this Mac, until the evening.
    phase = {"morning": "working", "switched": "working", "afternoon": "working"}.get(STATE)
    data["sessions"]["sessions"] = [dict(id="promo-codex", sourceId="codex-team-source" if switched else "codex-source", source=team if switched else personal,
                                         provider="codex", machine=local, machineId="local", phase=phase, updatedAt=STAMP - 3)] if phase else []
    # Remaining quota: (used %, reset time) for 5h and 7d.
    if STATE in ("morning", "switched"):
        plan = {"codex": [(93, at(0, 11, 52)), (69, at(3, 20, 1))],
                "codex-team": [(0, STAMP + 18000), (27, at(5, 20, 1))] if STATE == "morning" else [(1, at(0, 15, 45)), (28, at(5, 20, 1))]}
    else:  # OpenAI reset both accounts at 14:05; the studio account has been busy since.
        five = (46, at(0, 19, 5)) if STATE == "afternoon" else (18, at(0, 23, 50))
        plan = {"codex": [(0, STAMP + 18000), (0, STAMP + 7 * 86400)], "codex-team": [five, (9 if STATE == "afternoon" else 14, at(7, 14, 5))]}
    for quota in dashboard["quotas"]:
        for window, (used, reset) in zip(quota["windows"], plan.get(quota["accountId"], [])):
            window.update(usedPercent=used, resetsAt=reset)
    data["accountStatuses"] = [dict(accountKey="codex:" + account, sourceId="codex-source", machineId="local", current=(account == "codex-team") == switched,
                                    credential=True, checkedAt=STAMP) for account in ("codex", "codex-team")] + \
                              [row for row in data["accountStatuses"] if not row["accountKey"].startswith("codex:")]
    # Switching targets: the studio account can take over ~/.codex on this Mac. Later its wakeups run there too.
    target = dict(provider="codex", sourceId="codex-source", sourceName=T("This Mac", "本机"), path="~/.codex", machineId="local", machineName=local,
                  credential=True, checkedAt=STAMP, canWake=True, canSwitch=True, bound=True)
    data["accountTargets"] = [dict(target, accountKey="codex:codex-team", accountName=team, current=switched, sourceId="codex-team-source" if STATE == "night" else "codex-source"),
                              dict(target, accountKey="codex:codex", accountName=personal, current=not switched)]
    # The switch dialog reads both sign-in profiles of ~/.codex and the one process it closes.
    if STATE == "morning":
        next(s for s in settings["sources"] if s["id"] == "codex-source")["codexHomeId"] = "local-home"
        for account in settings["accounts"]:
            if account["provider"] == "codex":
                account.update(identityKey="identity-" + account["id"], quotaProfileId="local-home:profile-" + account["id"])
    process = dict(pid=917, name="codex", canClose=True)
    data["codexInspection"] = dict(profiles=[dict(id="profile-" + account, name=name, current=(account == "codex-team") == switched,
                                                  identity=dict(key="identity-" + account, workspace=name, email=email, plan=plan_name))
                                             for account, name, email, plan_name in [("codex", personal, "me@example.com", "plus"), ("codex-team", team, "me@studio.example", "pro")]],
                                   processes=[process], version="codex-cli 0.58.0", storageMode="file", cliAvailable=True)
    data["codexSwitch"] = dict(operationId="promo-switch", status="prepared", processes=[process], preservedProcesses=[])
    # Automatic valuation of the studio account: the window after the early reset next to earlier ones.
    base = next(r for r in dashboard["quotaEstimates"] if r["accountKey"] == "codex:codex")
    note = T("Rolling · last 90 days · weight halves every 7 days · 14 valid quota segments", "滚动估值 · 最近 90 天 · 权重每 7 天减半 · 14 个有效额度片段")
    def record(id, value, start, end, mode, status):
        return dict(base, id=id, accountKey="codex:codex-team", sourceIds=["codex-team-source"], sourceNames=[team], samplingMode=mode, status=status,
                    startedAt=start, checkpointAt=end, endedAt=end if status == "completed" else None, fiveHourValue=value, weeklyValue=round(value * 5.9, 2),
                    weeklyDirectValue=round(value * 5.9, 2), weeklyRatioValue=round(value * 6.1, 2), cost=round(value * .31, 3),
                    capacity=dict(base["capacity"], ratio=6.1, samples=14, updatedAt=end), calculationNote=note if mode == "rolling" else T("Automatic segment", "自动采样片段"),
                    sampleCount=14, lookbackDays=90, halfLifeDays=7)
    if STATE in ("afternoon", "evening", "night"):
        segment = record("team-auto-now", 35.2, at(0, 14, 5), STAMP - 60, "automatic", "active")
        dashboard["automaticEstimateSummaries"].append(record("team-rolling", 35.4, at(-89, 9), STAMP - 60, "rolling", "active"))
        dashboard["quotaEstimates"].append(segment)
        history = [record(f"team-auto-{day}", value, at(-day, 13, 40), at(-day, 18, 40), "automatic", "completed")
                   for day, value in [(1, 36.1), (2, 34.7), (4, 35.8)]]
        data["quotaHistory"] += [segment] + history
        for source in dashboard["sources"]:
            if source["id"] == "codex-team-source":
                source["autoValuationStatus"] = dict(state="sampling", note=T("Sampling · 14 quota segments in the last 90 days", "采样中 · 最近 90 天 14 个额度片段"),
                                                     accountKey="codex:codex-team", updatedAt=STAMP)
    # The GPU box is fully booked by someone else's training run.
    if STATE == "evening":
        lab = next(h for h in data["hosts"] if h["id"] == gpu["id"])["sample"]
        lab.update(load=[61.2, 59.8, 57.4], cpu=[dict(id="cpu", utilization=78)])
        lab["memory"].update(available=41 * GiB, cached=38 * GiB)
        lab["gpu"] = [dict(id=str(i), name="NVIDIA H100 80GB", utilization=u, memoryUsedMiB=m, memoryTotalMiB=81920, temperature=t, powerWatts=p)
                      for i, (u, m, t, p) in enumerate([(100, 80180, 81, 698), (99, 79870, 80, 694), (100, 80240, 82, 700), (98, 79620, 79, 689)])]
    # The week's wakeups: before, as a saved draft, and once deployed.
    if STATE == "night":
        claude = next(row for row in data["wakeups"] if row["task"]["id"] == "wake-claude")
        task = dict(id="wake-week", name=T("Crunch week", "本周赶工"), sourceId="codex-team-source", times=["07:00", "12:00", "17:00"],
                    model="gpt-5.1-codex-mini", effort="low", prompt="Hi. Reply only OK. Do not use any tools.", binary="")
        deployed = dict(task=task, deployment=dict(task=task, deployedAt=STAMP, enabled=True, state="deployed", timezone="UTC+08:00", target=local, changed=False))
        data["wakeups"] = [claude]
        data["wakeupsDraft"] = [claude, dict(task=task, deployment=None)]
        data["wakeupsDeployed"] = [claude, deployed]
        statuses = {"wake-claude": data["wakeupStatuses"]["wake-claude"]}
        data["wakeupStatuses"] = data["wakeupStatusesDraft"] = statuses
        data["wakeupStatusesDeployed"] = dict(statuses, **{"wake-week": dict(installed=True, enabled=True, timezone="UTC+08:00", nextRunAt=at(1, 7), history=[])})


if STATE:
    workday()

if "--fixture" in sys.argv:
    print(json.dumps(data)); sys.exit(0)
rpc = {"hello": {"version": "preview"}, "settings.get": settings, "dashboard": dashboard, "dashboard.summary": dashboard, "dashboard.status": dashboard,
       "prices.list": data["prices"], "hosts.sample": data["hosts"], "wakeups.list": data["wakeups"], "quotas.refresh": dashboard["quotas"],
       "sessions.list": data["sessions"], "sources.scan": [], "accounts.status.get": data["accountStatuses"], "accounts.cleanup.list": [],
       "accounts.targets": data["accountTargets"], "codexAuth.reconcile": {"issues": []},
       "codexAuth.inspect": data.get("codexInspection", {}), "codexAuth.switch.prepare": data.get("codexSwitch", {}),
       "quotaEstimates.list": data["quotaHistory"], "creditEstimates.list": data["creditHistory"], "network.status": data["network"], "network.test": data["network"],
       "pets.list": data["petLibrary"], "pets.current": data.get("petAsset", {}), "settings.patch": settings, "accounts.deployments.sync": {"failedTasks": []},
       "codexApi.status": {"configRevision": "preview", "installed": True, "ready": True}}
for line in sys.stdin:
    request = json.loads(line)
    result = rpc.get(request["method"], {})
    params = request.get("params") or {}
    if request["method"] == "wakeups.status":
        result = data["wakeupStatuses"].get(params.get("id", ""), {})
    if request["method"] == "accounts.targets" and params.get("accountKey"):
        result = [row for row in result if row["accountKey"] == params["accountKey"]]
    print(json.dumps(dict(jsonrpc="2.0", id=request["id"], result=copy.deepcopy(result))), flush=True)
