use serde_json::{Value, json};

pub fn rates(current: &Value, previous: Option<&Value>) -> Value {
    let mut out = current.clone();
    let Some(old) = previous else { return out };
    let dt =
        current["monotonic"].as_f64().unwrap_or(0.0) - old["monotonic"].as_f64().unwrap_or(0.0);
    if current["bootId"] != old["bootId"] || dt <= 0.0 || dt > 300.0 {
        return out;
    }
    for (group, fields) in [
        (
            "cpu",
            vec![
                ("total", "totalDelta"),
                ("idle", "idleDelta"),
                ("user", "userDelta"),
                ("system", "systemDelta"),
                ("iowait", "iowaitDelta"),
                ("steal", "stealDelta"),
            ],
        ),
        (
            "disk",
            vec![
                ("readBytes", "readBytesPerSecond"),
                ("writeBytes", "writeBytesPerSecond"),
                ("reads", "readIops"),
                ("writes", "writeIops"),
                ("busyMs", "busyMsPerSecond"),
            ],
        ),
        (
            "network",
            vec![
                ("rxBytes", "rxBytesPerSecond"),
                ("txBytes", "txBytesPerSecond"),
            ],
        ),
    ] {
        if let (Some(rows), Some(prior)) = (out[group].as_array_mut(), old[group].as_array()) {
            for row in rows {
                if let Some(p) = prior.iter().find(|p| p["id"] == row["id"]) {
                    for (key, dest) in &fields {
                        if let (Some(a), Some(b)) = (row[*key].as_u64(), p[*key].as_u64())
                            && a >= b
                        {
                            let delta = (a - b) as f64;
                            row[*dest] = json!(if group == "cpu" { delta } else { delta / dt });
                        }
                    }
                    if group == "cpu" {
                        let total = row["totalDelta"].as_f64().unwrap_or(0.0);
                        if total > 0.0 && row["idleDelta"].as_f64().is_some() {
                            row["utilization"] = json!(
                                ((1.0 - row["idleDelta"].as_f64().unwrap_or(0.0) / total) * 100.0)
                                    .clamp(0.0, 100.0)
                            );
                            for (key, dest) in [
                                ("userDelta", "userPercent"),
                                ("systemDelta", "systemPercent"),
                                ("iowaitDelta", "iowaitPercent"),
                                ("stealDelta", "stealPercent"),
                            ] {
                                if let Some(delta) = row[key].as_f64() {
                                    row[dest] = json!(delta / total * 100.0);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rates_reset_on_reboot_or_counter_wrap() {
        let old = json!({"bootId":"a","monotonic":10,"network":[{"id":"eth0","rxBytes":100,"txBytes":200}]});
        let c = json!({"bootId":"a","monotonic":12,"network":[{"id":"eth0","rxBytes":300,"txBytes":400}]});
        assert_eq!(
            rates(&c, Some(&old))["network"][0]["rxBytesPerSecond"],
            100.0
        );
        let reset = json!({"bootId":"b","monotonic":12,"network":[{"id":"eth0","rxBytes":300,"txBytes":400}]});
        assert!(rates(&reset, Some(&old))["network"][0]["rxBytesPerSecond"].is_null());
    }
}
