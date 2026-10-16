//! Enumerate saved SSH aliases without connecting or evaluating Match/ProxyCommand.
use serde::Serialize;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    path: String,
    hosts: Vec<Alias>,
    warnings: Vec<String>,
}
#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Alias {
    alias: String,
    host_name: Option<String>,
    user: Option<String>,
    port: Option<u16>,
}
#[derive(Default)]
struct Reader {
    aliases: Vec<String>,
    directives: Vec<(Vec<String>, String, String)>,
    scope: Vec<String>,
    stack: HashSet<PathBuf>,
    warnings: Vec<String>,
    files: usize,
}

pub fn read() -> Config {
    read_path(&crate::store::home().join(".ssh/config"))
}
fn read_path(path: &Path) -> Config {
    let mut reader = Reader {
        scope: vec!["*".into()],
        ..Default::default()
    };
    reader.file(path, path.parent().unwrap_or(Path::new(".")));
    let hosts = reader
        .aliases
        .iter()
        .map(|name| {
            let mut host = Alias {
                alias: name.clone(),
                ..Default::default()
            };
            for (patterns, key, value) in &reader.directives {
                if !matches_host(patterns, name) {
                    continue;
                }
                match key.as_str() {
                    "hostname" if host.host_name.is_none() => host.host_name = Some(value.clone()),
                    "user" if host.user.is_none() => host.user = Some(value.clone()),
                    "port" if host.port.is_none() => {
                        host.port = value.parse::<u16>().ok().filter(|p| *p > 0)
                    }
                    _ => {}
                }
            }
            host
        })
        .collect();
    Config {
        path: path.to_string_lossy().into(),
        hosts,
        warnings: reader.warnings,
    }
}
impl Reader {
    fn file(&mut self, path: &Path, root: &Path) {
        if self.files >= 128 || self.stack.len() >= 16 {
            self.warnings
                .push("SSH Include 数量或嵌套层数过多，已停止继续读取".into());
            return;
        }
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        if !self.stack.insert(canonical.clone()) {
            self.warnings
                .push(format!("已跳过循环 Include：{}", path.display()));
            return;
        }
        self.files += 1;
        let content = std::fs::metadata(path).and_then(|m| {
            if m.len() > 1_048_576 {
                return Err(std::io::Error::other("配置文件超过 1 MiB"));
            }
            std::fs::read_to_string(path)
        });
        match content {
            Err(error) => self
                .warnings
                .push(format!("无法读取 {}：{error}", path.display())),
            Ok(content) => {
                for (index, line) in content.lines().enumerate() {
                    let words = match words(line) {
                        Ok(words) => words,
                        Err(error) => {
                            self.warnings.push(format!(
                                "{}:{}：{error}",
                                path.display(),
                                index + 1
                            ));
                            continue;
                        }
                    };
                    let Some(key) = words.first().map(|s| s.to_ascii_lowercase()) else {
                        continue;
                    };
                    let args = &words[1..];
                    match key.as_str() {
                        "host" => {
                            self.scope = args.to_vec();
                            for alias in args.iter().filter(|s| {
                                !s.is_empty()
                                    && !s.starts_with(['!', '-'])
                                    && !s.contains(['*', '?', '[', ']'])
                                    && !s.chars().any(char::is_whitespace)
                            }) {
                                if !self.aliases.iter().any(|s| s.eq_ignore_ascii_case(alias)) {
                                    self.aliases.push(alias.clone());
                                }
                            }
                        }
                        "match" => {
                            self.scope.clear();
                            if !self.warnings.iter().any(|w| w.starts_with("Match 条件")) {
                                self.warnings.push(
                                    "Match 条件未执行；预览地址仅供参考，连接仍使用原 SSH 配置"
                                        .into(),
                                );
                            }
                        }
                        "include" => {
                            for pattern in args {
                                let expanded = if let Some(relative) = pattern.strip_prefix("~/") {
                                    crate::store::home().join(relative)
                                } else {
                                    PathBuf::from(pattern)
                                };
                                let expanded = if expanded.is_absolute() {
                                    expanded
                                } else {
                                    root.join(expanded)
                                };
                                for file in expand_pattern(&expanded) {
                                    self.file(&file, root);
                                }
                            }
                        }
                        "hostname" | "user" | "port" if !self.scope.is_empty() => {
                            if let Some(value) = args.first() {
                                self.directives
                                    .push((self.scope.clone(), key, value.clone()));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        self.stack.remove(&canonical);
    }
}
fn words(line: &str) -> Result<Vec<String>, &'static str> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escape = false;
    for c in line.chars() {
        if escape {
            // OpenSSH on Windows also accepts native paths. Only consume an
            // escape before lexical separators; retain ordinary path slashes.
            if !c.is_whitespace() && !matches!(c, '\\' | '\'' | '"' | '#' | '=') {
                word.push('\\');
            }
            word.push(c);
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        if c == '#' {
            break;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            continue;
        }
        if c.is_whitespace() || (c == '=' && out.len() <= 1) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if quote.is_some() || escape {
        return Err("引号或转义未闭合");
    }
    if !word.is_empty() {
        out.push(word);
    }
    Ok(out)
}
fn glob(pattern: &str, value: &str) -> bool {
    fn piece(p: &[char], c: char) -> Option<usize> {
        if p.first() == Some(&'?') {
            return Some(1);
        }
        if p.first() == Some(&'[')
            && let Some(end) = p.iter().position(|c| *c == ']')
        {
            let inverted = matches!(p.get(1), Some('!' | '^'));
            let mut i = if inverted { 2 } else { 1 };
            let mut found = false;
            while i < end {
                if i + 2 < end && p[i + 1] == '-' {
                    found |= p[i] <= c && c <= p[i + 2];
                    i += 3;
                } else {
                    found |= p[i] == c;
                    i += 1;
                }
            }
            return (found != inverted).then_some(end + 1);
        }
        (p.first() == Some(&c)).then_some(1)
    }
    let p: Vec<_> = pattern.chars().collect();
    let s: Vec<_> = value.chars().collect();
    let (mut i, mut j, mut star) = (0, 0, None);
    while j < s.len() {
        if p.get(i) == Some(&'*') {
            i += 1;
            star = Some((i, j));
        } else if let Some(size) = piece(&p[i..], s[j]) {
            i += size;
            j += 1;
        } else if let Some((after, consumed)) = star.filter(|(_, consumed)| *consumed < s.len()) {
            j = consumed + 1;
            i = after;
            star = Some((after, j));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}
fn matches_host(patterns: &[String], name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let mut positive = false;
    for pattern in patterns {
        if let Some(negative) = pattern.strip_prefix('!') {
            if glob(&negative.to_ascii_lowercase(), &name) {
                return false;
            }
        } else if glob(&pattern.to_ascii_lowercase(), &name) {
            positive = true;
        }
    }
    positive
}
fn expand_pattern(path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![PathBuf::new()];
    for component in path.components() {
        let part = component.as_os_str().to_string_lossy();
        if part.contains(['*', '?', '[']) {
            paths = paths
                .iter()
                .flat_map(|base| {
                    std::fs::read_dir(base)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .filter_map(|entry| {
                            glob(&part, &entry.file_name().to_string_lossy()).then(|| entry.path())
                        })
                })
                .collect();
            paths.sort();
        } else {
            for base in &mut paths {
                base.push(component.as_os_str());
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn includes_aliases_first_value_and_never_executes_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("conf.d")).unwrap();
        std::fs::write(dir.path().join("config"), "Host one two !no *.example\n HostName=\"box.example\" # comment\n User alice\nInclude conf.d/*\nHost * !two\n Port 2200\nMatch exec \"touch forbidden\"\n User wrong\n").unwrap();
        std::fs::write(
            dir.path().join("conf.d/a"),
            "Host one\n User bob\nHost three\n HostName third\nInclude config\n",
        )
        .unwrap();
        let result = read_path(&dir.path().join("config"));
        assert_eq!(
            result
                .hosts
                .iter()
                .map(|h| h.alias.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
        assert_eq!(result.hosts[0].user.as_deref(), Some("alice"));
        assert_eq!(result.hosts[0].port, Some(2200));
        assert_eq!(result.hosts[1].port, None);
        assert!(result.warnings.iter().any(|w| w.contains("循环")));
        assert!(result.warnings.iter().any(|w| w.contains("Match")));
        assert!(!dir.path().join("forbidden").exists());
    }
    #[test]
    fn missing_and_malformed_files_leave_other_aliases_available() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_path(&dir.path().join("missing")).hosts.is_empty());
        std::fs::write(
            dir.path().join("config"),
            "Host \"broken\nHost good\n Port 22\nInclude absent\nHost good\nHost 'other'\n",
        )
        .unwrap();
        let result = read_path(&dir.path().join("config"));
        assert_eq!(result.hosts.len(), 2);
        assert_eq!(result.warnings.len(), 2);
    }
    #[test]
    fn unicode_globs_classes_and_windows_paths() {
        assert!(glob("conf-[a-c]?*", "conf-b中文"));
        assert!(!glob("conf-[!a-c]*", "conf-b"));
        assert!(!glob(&"*a".repeat(80), &"a".repeat(79)));
        assert_eq!(
            words(r#"Include "C:\Users\name\.ssh\conf.d\*""#).unwrap(),
            ["Include", r"C:\Users\name\.ssh\conf.d\*"]
        );
    }
}
