//! A native credential is resolved and used on its owning host. The same HTTP
//! request and response adapters serve local and SSH reads; SSH carries only
//! references, a public identity projection, credential hashes and quota data.
use crate::{account_scan::NativeConnection, models::*, network, process, ssh};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read, time::Duration};

const LIMIT: usize = 2 * 1024 * 1024;
const MARKER: &str = "${AIEYES_NATIVE_TOKEN}";

/// Provider adapters construct this request once; the transport decides where
/// to execute it. On SSH the synthetic token is replaced on the remote host.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Request {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<String>,
    pub connect_timeout_seconds: u64,
    pub timeout_seconds: u64,
}
impl Request {
    pub(crate) fn new(method: &str, url: &str) -> Self {
        Self {
            method: method.into(),
            url: url.into(),
            headers: BTreeMap::new(),
            body: None,
            connect_timeout_seconds: 10,
            timeout_seconds: 25,
        }
    }
    pub(crate) fn header(mut self, key: &str, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }
    pub(crate) fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }
    pub(crate) fn timeouts(mut self, connect: u64, total: u64) -> Self {
        self.connect_timeout_seconds = connect;
        self.timeout_seconds = total;
        self
    }
    pub(crate) fn send(&self, proxy: &ProxyConfig) -> Result<Response> {
        let client = network::client_with_policy(
            proxy,
            network::ClientPolicy {
                redirects: 0,
                connect_timeout: Duration::from_secs(self.connect_timeout_seconds),
                timeout: Duration::from_secs(self.timeout_seconds),
                ..Default::default()
            },
        )
        .context("无法创建订阅查询连接")?;
        let method = reqwest::Method::from_bytes(self.method.as_bytes())
            .map_err(|_| anyhow::anyhow!("额度请求格式无效"))?;
        let mut request = client.request(method, &self.url);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(body) = &self.body {
            request = request.body(body.clone());
        }
        let response = request
            .send()
            .map_err(|_| anyhow::anyhow!("订阅连接失败，请检查网络或代理"))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            // A server error may echo the credential, so never read its text.
            return Ok(Response {
                status,
                data: Value::Null,
            });
        }
        let mut bytes = Vec::new();
        response
            .take(LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow::anyhow!("订阅响应读取失败"))?;
        ensure!(bytes.len() <= LIMIT, "订阅响应过大");
        let data = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("订阅响应格式无效，请检查登录状态"))?;
        Ok(Response { status, data })
    }
    fn bind_remote(&mut self, stub: &str) -> Result<()> {
        ensure!(!stub.is_empty(), "远端凭据投影缺失");
        ensure!(
            !self.url.contains(stub) && !self.body.as_ref().is_some_and(|v| v.contains(stub)),
            "额度请求不能把凭据放入地址或正文"
        );
        for (name, value) in &mut self.headers {
            if value.contains(stub) {
                ensure!(
                    matches!(
                        name.to_ascii_lowercase().as_str(),
                        "authorization" | "x-dsh-auth-token" | "cookie"
                    ),
                    "额度请求的凭据请求头不受支持"
                );
                *value = value.replace(stub, MARKER);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Response {
    pub status: u16,
    #[serde(default)]
    pub data: Value,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Projection {
    pub fingerprint: String,
    pub pin: String,
    pub token_stub: String,
    pub value: Value,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub companions: Vec<Companion>,
}
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Companion {
    pub path: String,
    pub value: Value,
}

pub(crate) struct Remote {
    host: Host,
    reference: Value,
    pub projection: Projection,
}
impl Remote {
    pub(crate) fn inspect(
        settings: &Settings,
        source: &Source,
        row: &NativeConnection,
    ) -> Result<Self> {
        let host = settings
            .hosts
            .iter()
            .find(|h| h.id == row.machine_id && h.enabled)
            .context("SSH 主机未启用或已移除")?;
        let account = settings
            .accounts
            .iter()
            .find(|a| a.provider == source.provider && a.id == source.account_id);
        let mut configured = source.clone();
        configured.quota_pre_command = crate::accounts::pre_command(settings, account, source);
        let host = crate::quota::quota_host(host, &configured);
        let reference = json!({
            "provider":row.provider, "root":row.root, "configPath":row.config_path,
            "selector":row.selector, "kind":row.kind,
        });
        let projection = serde_json::from_value(call(
            &host,
            json!({"action":"inspect", "reference":reference}),
        )?)
        .context("远端原生凭据投影格式无效")?;
        Ok(Self {
            host,
            reference,
            projection,
        })
    }
    pub(crate) fn request(&self, mut request: Request) -> Result<Response> {
        request.bind_remote(&self.projection.token_stub)?;
        serde_json::from_value(call(
            &self.host,
            json!({"action":"http", "reference":self.reference,
                "pin":self.projection.pin, "request":request}),
        )?)
        .context("远端额度响应格式无效")
    }
    pub(crate) fn codex(&self, binary: &str) -> Result<Value> {
        let response: Response = serde_json::from_value(call(
            &self.host,
            json!({"action":"codex", "reference":self.reference, "pin":self.projection.pin,
                "binary":binary}),
        )?)
        .context("远端额度响应格式无效")?;
        Ok(response.data)
    }
    pub(crate) fn verify(&self) -> Result<()> {
        call(
            &self.host,
            json!({"action":"inspect", "reference":self.reference, "pin":self.projection.pin}),
        )?;
        Ok(())
    }
}

fn payload(request: Value) -> Value {
    json!({
        "version":env!("CARGO_PKG_VERSION"),
        "script":include_str!("../../../scripts/remote_native_quota.py"),
        "scripts":{
            "scan":include_str!("../../../scripts/remote_account_scan.py"),
            "codex":include_str!("../../../scripts/remote_codex_auth.py"),
            "tomli":{
                "__init__":include_str!("../../../scripts/vendor/tomli/__init__.py"),
                "_types":include_str!("../../../scripts/vendor/tomli/_types.py"),
                "_re":include_str!("../../../scripts/vendor/tomli/_re.py"),
                "_parser":include_str!("../../../scripts/vendor/tomli/_parser.py"),
            },
        },
        "request":request,
    })
}
fn call(host: &Host, request: Value) -> Result<Value> {
    // Both the code and request travel over stdin. File paths, identities,
    // fingerprints and proxy settings are absent from the process command line.
    const LOADER: &str = "import json,sys; p=json.load(sys.stdin); exec(compile(p['script'],'<aieyes-native-quota>','exec'),{'__name__':'__main__','REQUEST':p['request'],'SCRIPTS':p['scripts'],'CLIENT_VERSION':p['version']})";
    let command =
        ssh::account_command(host, &format!("exec python3 -c {}", process::quote(LOADER)))?;
    let bytes = process::run(
        command,
        serde_json::to_vec(&payload(request))?,
        Duration::from_secs(100),
    )?;
    let line = bytes
        .split(|b| *b == b'\n')
        .rev()
        .find(|line| !line.is_empty())
        .context("远端额度响应缺失")?;
    ensure!(line.len() <= 2 * LIMIT, "远端额度响应过大");
    let response: Value = serde_json::from_slice(line).context("远端额度响应格式无效")?;
    decode(response)
}
fn decode(response: Value) -> Result<Value> {
    ensure!(response["version"] == 1, "远端额度助手版本不兼容");
    if let Some(code) = response["errorCode"].as_str() {
        // Never include unrecognized response text in a diagnostic.
        anyhow::bail!(
            "{}",
            match code {
                "changed" => "查询期间原生账户身份已变化",
                "removed" => "原生凭据已移除",
                "changing" => "原生配置正在写入，请稍后重试",
                "large_file" => "原生配置超过 1 MiB",
                "credential" => "登录凭据格式无效",
                "connection" => "订阅连接失败，请检查网络或代理",
                "response_large" => "订阅响应过大",
                "response_format" => "订阅响应格式无效，请检查登录状态",
                "response_private" => "额度响应包含私有字段，已丢弃",
                "endpoint" => "原生额度地址不受支持",
                "proxy" => "远端额度代理格式不支持",
                "unsupported" => "此原生登录暂不支持额度查询",
                "codex" => "远端 Codex 额度查询失败，请检查 CLI 版本和原客户端登录状态",
                _ => "远端原生配置无法读取或格式不支持",
            }
        );
    }
    response
        .get("result")
        .cloned()
        .context("远端额度响应缺少结果")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_binding_preserves_auth_scheme_without_sending_a_token() {
        for value in [
            "fake-token",
            "Bearer fake-token",
            "Bearer workos:fake-token",
            "token fake-token",
            "Cloud-IDE-JWT fake-token",
        ] {
            let mut request =
                Request::new("GET", "https://api.github.com/user").header("Authorization", value);
            request.bind_remote("fake-token").unwrap();
            assert_eq!(
                request.headers["Authorization"],
                value.replace("fake-token", MARKER)
            );
            assert!(
                !serde_json::to_string(&request)
                    .unwrap()
                    .contains("fake-token")
            );
        }
        let mut request = Request::new(
            "GET",
            "https://platform.deepseek.com/api/v0/users/get_user_summary",
        )
        .header("x-dsh-auth-token", "fake-token");
        request.bind_remote("fake-token").unwrap();
        assert_eq!(request.headers["x-dsh-auth-token"], MARKER);
        assert!(!request.headers.contains_key("Authorization"));
    }

    #[test]
    fn remote_binding_rejects_token_in_url_body_and_unexpected_headers() {
        let mut url = Request::new("GET", "https://api.github.com/user?fake-token");
        assert!(url.bind_remote("fake-token").is_err());
        let mut body = Request::new("POST", "https://api.github.com/user").body("fake-token");
        assert!(body.bind_remote("fake-token").is_err());
        let mut header =
            Request::new("GET", "https://api.github.com/user").header("X-Log", "fake-token");
        assert!(header.bind_remote("fake-token").is_err());
    }

    #[test]
    fn remote_errors_never_include_server_supplied_text() {
        for code in ["connection", "response_format", "native-fixture-secret"] {
            let error =
                decode(json!({"version":1,"errorCode":code,"error":"native-fixture-secret"}))
                    .unwrap_err();
            assert!(!error.to_string().contains("native-fixture-secret"));
        }
    }
}
