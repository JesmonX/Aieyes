//! Native local sampling and the shared default device selection policy.
use crate::models::Host;
use serde_json::{Value, json};
use std::time::Instant;
use sysinfo::{Disks, Networks, System};

pub fn recommended_filesystem(row: &Value) -> bool {
    let path = row["id"]
        .as_str()
        .unwrap_or("")
        .replace('\\', "/")
        .to_lowercase();
    let kind = row["type"].as_str().unwrap_or("").to_lowercase();
    ![
        "tmpfs", "devtmpfs", "squashfs", "overlay", "proc", "procfs", "sysfs", "devfs", "autofs",
        "cgroup", "cgroup2",
    ]
    .contains(&kind.as_str())
        && ![
            "/snap",
            "/var/lib/snapd/snap",
            "/run",
            "/efi",
            "/boot/efi",
            "/dev",
            "/proc",
            "/sys",
            "/system/volumes/preboot",
            "/system/volumes/vm",
            "/system/volumes/update",
            "/system/volumes/xarts",
            "/system/volumes/iscpreboot",
            "/system/volumes/hardware",
        ]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

pub fn filter_devices(sample: &mut Value, host: &Host) {
    for group in ["cpu", "gpu", "filesystems", "disk", "network"] {
        let selected: Vec<_> = host
            .devices
            .iter()
            .filter(|s| s.starts_with(&format!("{group}:")))
            .collect();
        if let Some(rows) = sample[group].as_array_mut() {
            rows.retain(|row| {
                if group == "cpu" && row["id"] == "cpu" {
                    return true;
                }
                if selected
                    .iter()
                    .any(|id| id.as_str() == format!("{group}:__all__"))
                {
                    return true;
                }
                if selected.is_empty() {
                    return group != "filesystems" || recommended_filesystem(row);
                }
                selected.iter().any(|id| {
                    id.as_str() == format!("{group}:{}", row["id"].as_str().unwrap_or(""))
                })
            });
        }
    }
}

pub struct LocalSampler {
    system: System,
    disks: Disks,
    networks: Networks,
    clock: Instant,
    warmed: bool,
}
impl Default for LocalSampler {
    fn default() -> Self {
        Self {
            system: System::new(),
            disks: Disks::new(),
            networks: Networks::new(),
            clock: Instant::now(),
            warmed: false,
        }
    }
}
impl LocalSampler {
    pub fn sample(&mut self, metrics: &[String]) -> Value {
        let enabled = |key: &str| metrics.iter().any(|m| m == key);
        let mut result = json!({"timestamp":crate::models::now(),"monotonic":self.clock.elapsed().as_secs_f64(),
            "bootId":System::boot_time().to_string(),"uptime":System::uptime(),"load":[],"errors":{}});
        #[cfg(unix)]
        {
            let load = System::load_average();
            result["load"] = json!([load.one, load.five, load.fifteen]);
        }
        if enabled("cpu") {
            self.system.refresh_cpu_usage();
            if !self.warmed {
                std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
                self.system.refresh_cpu_usage();
                self.warmed = true;
            }
            let mut rows = vec![json!({"id":"cpu","utilization":self.system.global_cpu_usage()})];
            rows.extend(self.system.cpus().iter().enumerate().map(|(i,cpu)| json!({"id":format!("cpu{i}"),"name":cpu.brand(),"utilization":cpu.cpu_usage()})));
            result["cpu"] = json!(rows);
        }
        if enabled("memory") {
            self.system.refresh_memory();
            result["memory"] = json!({"total":self.system.total_memory(),"available":self.system.available_memory(),
                "swapTotal":self.system.total_swap(),"swapFree":self.system.free_swap()});
        }
        if enabled("filesystems") || enabled("disk") {
            self.disks.refresh(true);
            if enabled("filesystems") {
                result["filesystems"] = json!(self.disks.iter().map(|disk| json!({
                    "id":disk.mount_point().to_string_lossy(),"device":disk.name().to_string_lossy(),
                    "type":disk.file_system().to_string_lossy(),"total":disk.total_space(),"available":disk.available_space(),
                    "used":disk.total_space().saturating_sub(disk.available_space())})).collect::<Vec<_>>());
            }
            if enabled("disk") {
                // Several volumes can share a device. Keep one counter per device.
                let rows: std::collections::BTreeMap<_, _> = self.disks.iter().map(|disk| {
                    let id = disk.name().to_string_lossy().into_owned(); let usage = disk.usage();
                    (id.clone(), json!({"id":id,"readBytes":usage.total_read_bytes,"writeBytes":usage.total_written_bytes}))
                }).collect();
                result["disk"] = json!(rows.into_values().collect::<Vec<_>>());
            }
        }
        if enabled("network") {
            self.networks.refresh(true);
            result["network"] = json!(self.networks.iter().map(|(id,net)| json!({"id":id,
                "rxBytes":net.total_received(),"txBytes":net.total_transmitted(),
                "rxErrors":net.total_errors_on_received(),"txErrors":net.total_errors_on_transmitted()})).collect::<Vec<_>>());
        }
        if enabled("gpu") {
            match nvidia() {
                Ok(rows) => result["gpu"] = json!(rows),
                Err(reason) => {
                    result["gpu"] = json!([]);
                    result["errors"]["gpu"] = json!(reason);
                }
            }
        }
        result["monotonic"] = json!(self.clock.elapsed().as_secs_f64());
        result
    }
}

fn nvidia() -> Result<Vec<Value>, String> {
    let mut command = std::process::Command::new("nvidia-smi");
    command.args(["--query-gpu=index,name,utilization.gpu,memory.used,memory.total,temperature.gpu,power.draw", "--format=csv,noheader,nounits"]);
    let bytes = crate::process::run(command, vec![], std::time::Duration::from_secs(5))
        .map_err(|_| "本机暂无可用的 NVIDIA GPU 指标；其他 GPU 暂不支持".to_string())?;
    String::from_utf8_lossy(&bytes).lines().map(|line| {
        let parts: Vec<_> = line.split(',').map(str::trim).collect();
        if parts.len() != 7 { return Err("GPU 返回数据格式不正确".into()) }
        let number = |i: usize| parts[i].parse::<f64>().ok().filter(|n| n.is_finite());
        Ok(json!({"id":parts[0],"name":parts[1],"utilization":number(2),"memoryUsedMiB":number(3),
            "memoryTotalMiB":number(4),"temperature":number(5),"powerWatts":number(6)}))
    }).collect()
}
