//! Identity evidence for automatic runs. Time overlap never establishes ownership.
use super::Manifest;
use crate::models::Source;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(super) struct Evidence {
    pub source_id: String,
    pub provider: String,
    pub config_path: String,
    pub session_ids: Vec<String>,
    /// True means all model-producing activity is identified (or proven ephemeral/not invoked).
    pub complete: bool,
}
impl Evidence {
    pub fn manifest(m: &Manifest) -> Self {
        Self {
            source_id: m.source_id.clone(),
            provider: m.provider.clone(),
            config_path: m.config_path.clone(),
            ..Self::default()
        }
    }
    pub fn affects(&self, source: &Source) -> bool {
        if !self.source_id.is_empty() {
            return self.source_id == source.id;
        }
        // Old manifests have no source id. Only a proven different provider/directory can exclude them.
        if self.provider.is_empty() || self.config_path.is_empty() {
            return true;
        }
        crate::providers::canonical(&self.provider) == crate::providers::canonical(&source.provider)
            && crate::store::expand(self.config_path.trim_end_matches('/'))
                == crate::store::expand(source.path.trim_end_matches('/'))
    }
}

pub(super) fn sessions(provider: &str, output: &[u8]) -> Vec<String> {
    let mut ids = HashSet::new();
    for row in output
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
    {
        let candidates: Vec<&Value> = match provider {
            "codex" if row["type"] == "thread.started" => vec![&row["thread_id"]],
            "claude" => vec![&row["session_id"]],
            "agy" | "antigravity" => vec![
                &row["cascadeId"],
                &row["cascade_id"],
                &row["result"]["cascadeId"],
                &row["result"]["cascade_id"],
            ],
            _ => vec![],
        };
        for value in candidates {
            if let Some(id) = value.as_str().filter(|s| !s.is_empty() && s.len() <= 512) {
                ids.insert(id.to_owned());
            }
        }
    }
    let mut ids: Vec<_> = ids.into_iter().collect();
    ids.sort();
    ids
}

pub(super) fn excluded(source: &Source, records: &[Evidence]) -> Option<HashSet<String>> {
    let mut ids = HashSet::new();
    for record in records.iter().filter(|r| r.affects(source)) {
        if !record.complete {
            return None;
        }
        ids.extend(record.session_ids.iter().cloned());
    }
    Some(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identities_are_source_scoped_and_unidentified_runs_are_explicit() {
        let source = Source {
            id: "manual".into(),
            provider: "claude".into(),
            path: "/claude".into(),
            ..Default::default()
        };
        let other = Evidence {
            source_id: "other".into(),
            session_ids: vec!["manual-session".into()],
            complete: false,
            ..Default::default()
        };
        assert!(excluded(&source, &[other]).unwrap().is_empty());
        let own = Evidence {
            source_id: source.id.clone(),
            session_ids: vec!["automatic-session".into()],
            complete: true,
            ..Default::default()
        };
        assert!(
            excluded(&source, &[own])
                .unwrap()
                .contains("automatic-session")
        );
        assert!(excluded(&source, &[Evidence::default()]).is_none());
        assert_eq!(
            sessions(
                "codex",
                br#"{"type":"thread.started","thread_id":"thread"}"#
            ),
            vec!["thread"]
        );
        assert_eq!(
            sessions("claude", br#"{"session_id":"session","result":"PRIVATE"}"#),
            vec!["session"]
        );
        assert_eq!(
            sessions(
                "antigravity",
                br#"{"status":"SUCCESS","cascadeId":"cascade"}"#
            ),
            vec!["cascade"]
        );
        assert!(
            sessions(
                "antigravity",
                br#"{"session_id":"not-a-proven-cascade-id"}"#
            )
            .is_empty()
        );
    }
}
