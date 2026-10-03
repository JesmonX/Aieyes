import json
import os
import re
import subprocess
import sys
import time

enabled = set(json.loads(sys.argv[1]))
out = {"timestamp": time.time(), "monotonic": time.monotonic(), "errors": {}}

def read(path):
    with open(path) as stream:
        return stream.read()

def collect(name, operation):
    if name in enabled:
        try:
            out[name] = operation()
        except (OSError, ValueError, subprocess.SubprocessError, IndexError) as error:
            out["errors"][name] = type(error).__name__

out["bootId"] = read("/proc/sys/kernel/random/boot_id").strip()
out["uptime"] = float(read("/proc/uptime").split()[0])
out["load"] = [float(v) for v in read("/proc/loadavg").split()[:3]]

def cpu():
    result = []
    for line in read("/proc/stat").splitlines():
        fields = line.split()
        if fields[0].startswith("cpu"):
            values = [int(v) for v in fields[1:9]]
            values += [0] * (8 - len(values))
            result.append({"id": fields[0], "total": sum(values), "idle": values[3] + values[4],
                           "user": values[0] + values[1], "system": values[2], "iowait": values[4], "steal": values[7]})
    return result

def memory():
    values = {}
    for line in read("/proc/meminfo").splitlines():
        fields = line.replace(":", "").split()
        values[fields[0]] = int(fields[1]) * 1024
    return {"total": values["MemTotal"], "available": values.get("MemAvailable", values.get("MemFree", 0)),
            "cached": values.get("Cached", 0), "buffers": values.get("Buffers", 0),
            "swapTotal": values.get("SwapTotal", 0), "swapFree": values.get("SwapFree", 0)}

def unescape(value):
    return re.sub(r"\\([0-7]{3})", lambda m: chr(int(m.group(1), 8)), value)

def filesystems():
    result = []
    skip = {"proc", "sysfs", "devpts", "cgroup", "cgroup2", "securityfs", "debugfs", "tracefs", "pstore", "configfs", "mqueue", "hugetlbfs", "fusectl", "autofs", "rpc_pipefs", "binfmt_misc"}
    for line in read("/proc/mounts").splitlines():
        fields = line.split()
        if fields[2] in skip:
            continue
        path = unescape(fields[1])
        try:
            stats = os.statvfs(path)
        except OSError:
            continue
        if not stats.f_blocks:
            continue
        result.append({"id": path, "device": unescape(fields[0]), "type": fields[2],
                       "total": stats.f_blocks * stats.f_frsize, "available": stats.f_bavail * stats.f_frsize,
                       "used": (stats.f_blocks - stats.f_bfree) * stats.f_frsize,
                       "inodes": stats.f_files, "inodesFree": stats.f_ffree})
    return result

def disks():
    result = []
    for line in read("/proc/diskstats").splitlines():
        f = line.split()
        if len(f) >= 14 and not f[2].startswith(("loop", "ram")):
            result.append({"id": f[2], "reads": int(f[3]), "readBytes": int(f[5]) * 512,
                           "writes": int(f[7]), "writeBytes": int(f[9]) * 512, "busyMs": int(f[12])})
    return result

def network():
    result = []
    for line in read("/proc/net/dev").splitlines()[2:]:
        name, counts = line.split(":", 1)
        f = counts.split()
        result.append({"id": name.strip(), "rxBytes": int(f[0]), "rxPackets": int(f[1]), "rxErrors": int(f[2]),
                       "rxDrops": int(f[3]), "txBytes": int(f[8]), "txPackets": int(f[9]), "txErrors": int(f[10]), "txDrops": int(f[11])})
    return result

def gpu():
    fields = "index,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw"
    process = subprocess.run(["nvidia-smi", "--query-gpu=" + fields, "--format=csv,noheader,nounits"],
                             capture_output=True, text=True, timeout=5, check=True)
    result = []
    for line in process.stdout.splitlines():
        f = [v.strip() for v in line.split(",")]
        number = lambda index: float(f[index]) if re.fullmatch(r"[0-9.]+", f[index]) else None
        result.append({"id": f[0], "name": f[1], "utilization": number(2), "memoryUsedMiB": number(3),
                       "memoryTotalMiB": number(4), "temperature": number(5), "powerWatts": number(6)})
    return result

for key, function in (("cpu", cpu), ("memory", memory), ("filesystems", filesystems), ("disk", disks), ("network", network), ("gpu", gpu)):
    collect(key, function)
if len(sys.argv) > 2 and sys.argv[2] == "true":
    previous = dict(out)
    time.sleep(0.25)
    out = {"timestamp": time.time(), "monotonic": time.monotonic(), "errors": {},
           "bootId": previous["bootId"], "uptime": previous["uptime"], "load": previous["load"]}
    for key, function in (("cpu", cpu), ("memory", memory), ("filesystems", filesystems), ("disk", disks), ("network", network), ("gpu", gpu)):
        collect(key, function)
    out["previous"] = previous
print(json.dumps(out, separators=(",", ":")))
