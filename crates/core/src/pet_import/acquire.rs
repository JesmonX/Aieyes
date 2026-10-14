//! Resolve declarative pet resources. Package executables and lifecycle scripts
//! are never loaded or run, including for the supported installer adapters.
use super::*;
use crate::models::ProxyConfig;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::read::GzDecoder;
use reqwest::Url;
use sha2::{Digest, Sha256, Sha512};
use std::collections::HashSet;

const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const MAX_REGISTRY: u64 = 4 * 1024 * 1024;
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Package {
    name: String,
    version: String,
}
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Input {
    Local(PathBuf),
    Url(Url),
    Npm(Package),
    CodexPet(String),
    Petdex(String),
}
fn package(value: &str) -> Result<Package> {
    let value = value.strip_prefix("npm:").unwrap_or(value);
    let split = value.rfind('@').filter(|index| *index > 0);
    let (name, version) = split
        .map(|i| (&value[..i], &value[i + 1..]))
        .unwrap_or((value, "latest"));
    let valid_component = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    };
    let valid_name = if let Some(scoped) = name.strip_prefix('@') {
        scoped
            .split_once('/')
            .is_some_and(|(scope, name)| valid_component(scope) && valid_component(name))
    } else {
        valid_component(name)
    };
    ensure!(
        valid_name
            && name.len() <= 214
            && !version.is_empty()
            && version.len() <= 128
            && version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.+".contains(&c)),
        "请输入有效 npm 包名，可包含 @scope、版本或标签"
    );
    Ok(Package {
        name: name.into(),
        version: version.into(),
    })
}
fn slug(value: &str) -> Result<String> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
        "桌宠 ID 无效，请复制完整的宠物页面链接或导入命令"
    );
    Ok(value.into())
}
fn installer(package: Package, args: &[String]) -> Result<Input> {
    let action = match package.name.as_str() {
        "codex-pet-installer" => Some("add"),
        "petdex" => Some("install"),
        _ => None,
    };
    if let Some(action) = action {
        ensure!(
            args.len() == 2 && args[0] == action,
            "请提供一个桌宠 ID，例如 npx {} {} <桌宠ID>；不会自动选择桌宠",
            package.name,
            action
        );
        let id = slug(&args[1])?;
        return Ok(if package.name == "petdex" {
            Input::Petdex(id)
        } else {
            Input::CodexPet(id)
        });
    }
    ensure!(
        args.is_empty(),
        "这个安装器尚未适配。请提供包含 pet.json 和图集的 npm 素材包或资源链接；不会执行安装脚本"
    );
    Ok(Input::Npm(package))
}
fn npx(words: &[String]) -> Result<Input> {
    let mut offset = 1;
    let mut selected = None;
    while let Some(arg) = words.get(offset) {
        match arg.as_str() {
            "-y" | "--yes" | "--yes=true" | "-q" | "--quiet" => offset += 1,
            "--" => {
                offset += 1;
                break;
            }
            "-p" | "--package" => {
                ensure!(selected.is_none(), "一次只能导入一个 npm 包");
                offset += 1;
                selected = Some(package(words.get(offset).context("npx 命令缺少包名")?)?);
                offset += 1;
            }
            option if option.starts_with("--package=") => {
                ensure!(selected.is_none(), "一次只能导入一个 npm 包");
                selected = Some(package(&option[10..])?);
                offset += 1;
            }
            option if option.starts_with('-') => {
                bail!("不支持这个 npx 选项，请提供包名、桌宠 ID 或资源链接")
            }
            _ => break,
        }
    }
    let selected = if let Some(selected) = selected {
        if let Some(command) = words.get(offset) {
            let bin = selected.name.rsplit('/').next().unwrap_or(&selected.name);
            ensure!(
                command == &selected.name || command == bin,
                "npx 包名与命令不匹配，请提供资源链接"
            );
            offset += 1;
        }
        selected
    } else {
        let selected = package(words.get(offset).context("请输入 npx 包名和桌宠 ID")?)?;
        offset += 1;
        selected
    };
    installer(selected, &words[offset..])
}
fn resource_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("桌宠资源链接格式无效")?;
    ensure!(
        valid_url(&url),
        "资源链接必须为不含登录凭据的 HTTP/HTTPS URL"
    );
    Ok(url)
}
fn valid_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
}
pub(super) fn parse_input(value: &str) -> Result<Input> {
    let mut value = value.trim();
    ensure!(
        !value.is_empty() && value.len() <= 8192,
        "请输入桌宠链接、npx 命令、npm 包名或文件路径"
    );
    if value.len() > 1
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value = &value[1..value.len() - 1];
    }
    if value.starts_with("http://") || value.starts_with("https://") {
        return Ok(Input::Url(resource_url(value)?));
    }
    if value.starts_with("file://") {
        return Ok(Input::Local(
            Url::parse(value)?
                .to_file_path()
                .map_err(|_| anyhow::anyhow!("本地文件链接无效"))?,
        ));
    }
    let path = crate::store::expand(value);
    if path.exists()
        || value.starts_with(['/', '~', '.'])
        || value.ends_with(".zip")
        || value.ends_with("pet.json")
        || value.as_bytes().get(1) == Some(&b':')
    {
        return Ok(Input::Local(path));
    }
    let words = shell_words::split(value).context("导入命令引号不完整")?;
    if words.first().is_some_and(|s| s == "npx" || s == "npx.cmd") {
        return npx(&words);
    }
    ensure!(
        words.len() == 1,
        "无法识别导入内容，请提供资源链接、npx 命令或单个 npm 包名"
    );
    installer(package(value)?, &[])
}

pub(super) struct Download {
    pub bytes: Vec<u8>,
    pub url: Url,
}
pub(super) trait Fetcher {
    fn fetch(&self, url: &Url, limit: u64) -> Result<Download>;
}
pub(super) struct HttpFetcher {
    client: reqwest::blocking::Client,
}
impl HttpFetcher {
    pub(super) fn new(proxy: &ProxyConfig) -> Result<Self> {
        let client = crate::network::client_builder(proxy)?
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    attempt.error("桌宠下载重定向过多")
                } else if !valid_url(attempt.url()) {
                    attempt.error("桌宠下载重定向地址无效")
                } else {
                    attempt.follow()
                }
            }))
            .build()?;
        Ok(Self { client })
    }
}
impl Fetcher for HttpFetcher {
    fn fetch(&self, url: &Url, limit: u64) -> Result<Download> {
        ensure!(valid_url(url), "桌宠资源链接无效");
        crate::operations::check()?;
        let mut response = self
            .client
            .get(url.clone())
            .send()
            .context("桌宠资源下载失败")?
            .error_for_status()
            .context("桌宠资源暂不可下载")?;
        ensure!(
            response.content_length().is_none_or(|n| n <= limit),
            "桌宠资源过大"
        );
        let url = response.url().clone();
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            crate::operations::check()?;
            let read = response.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            ensure!((bytes.len() + read) as u64 <= limit, "桌宠资源过大");
            bytes.extend_from_slice(&chunk[..read]);
        }
        crate::operations::check()?;
        Ok(Download { bytes, url })
    }
}
fn remote_manifest(
    url: &Url,
    image_url: Option<&Url>,
    fetcher: &impl Fetcher,
) -> Result<(Manifest, Vec<u8>)> {
    let download = fetcher.fetch(url, MAX_MANIFEST)?;
    let m = manifest(&download.bytes)?;
    let sheet = image_url
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| download.url.join(&m.spritesheet_path))?;
    ensure!(valid_url(&sheet), "图集资源链接无效");
    let bytes = fetcher.fetch(&sheet, MAX_IMAGE)?.bytes;
    Ok((m, bytes))
}
fn codex_pet(id: &str, fetcher: &impl Fetcher) -> Result<Acquisition> {
    let id = slug(id)?;
    let url = resource_url(&format!(
        "https://codex-pet.org/api/pets/{id}/files/pet.json"
    ))?;
    let (manifest, bytes) = remote_manifest(&url, None, fetcher)?;
    let info = fetcher
        .fetch(
            &resource_url(&format!("https://codex-pet.org/api/pets/{id}"))?,
            MAX_REGISTRY,
        )
        .ok()
        .and_then(|d| serde_json::from_slice::<Value>(&d.bytes).ok());
    let info = info.as_ref().map(|v| v.get("pet").unwrap_or(v));
    let creator =
        info.and_then(|v| creator_name(v.get("creator")).or_else(|| creator_name(v.get("author"))));
    Ok(Acquisition {
        manifest,
        bytes,
        source: Some(format!("https://codex-pet.org/pets/{id}/")),
        creator,
    })
}
fn petdex(id: &str, fetcher: &impl Fetcher) -> Result<Acquisition> {
    let id = slug(id)?;
    let url = resource_url(&format!("https://petdex.dev/api/install-pet/{id}"))?;
    let info: Value = serde_json::from_slice(&fetcher.fetch(&url, MAX_REGISTRY)?.bytes)
        .context("Petdex 返回了无效的资源信息")?;
    ensure!(info["ok"] == true, "Petdex 未找到这个桌宠，请确认 ID");
    let pet = &info["pet"];
    let manifest_url = resource_url(
        pet["petJsonUrl"]
            .as_str()
            .context("Petdex 缺少 manifest 地址")?,
    )?;
    let sprite_url = resource_url(
        pet["spritesheetUrl"]
            .as_str()
            .context("Petdex 缺少图集地址")?,
    )?;
    let (manifest, bytes) = remote_manifest(&manifest_url, Some(&sprite_url), fetcher)?;
    Ok(Acquisition {
        manifest,
        bytes,
        source: Some(format!("https://petdex.dev/pets/{id}")),
        creator: creator_name(pet.get("creator")).or_else(|| creator_name(pet.get("author"))),
    })
}
fn tarball(bytes: &[u8]) -> Result<(Manifest, Vec<u8>)> {
    ensure!(bytes.len() as u64 <= MAX_PACKAGE, "npm 素材包过大");
    let mut paths = HashSet::new();
    let mut candidates = Vec::new();
    let mut expanded = 0u64;
    let mut package =
        tar::Archive::new(GzDecoder::new(Cursor::new(bytes)).take(MAX_EXPANDED + 2 * 1024 * 1024));
    for (index, entry) in package.entries()?.enumerate() {
        crate::operations::check()?;
        ensure!(index < 1024, "npm 素材包文件数量过多");
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "npm 素材包不能包含链接或特殊文件"
        );
        let path = relative(entry.path()?.to_str().context("npm 素材包路径无效")?)?;
        ensure!(paths.insert(path.clone()), "npm 素材包包含重复文件");
        expanded = expanded
            .checked_add(entry.size())
            .context("npm 素材包大小无效")?;
        ensure!(expanded <= MAX_EXPANDED, "npm 素材包解压后过大");
        if kind.is_file() && path.file_name().is_some_and(|s| s == "pet.json") {
            ensure!(entry.size() <= MAX_MANIFEST, "pet.json 过大");
            let mut raw = Vec::new();
            entry
                .by_ref()
                .take(MAX_MANIFEST + 1)
                .read_to_end(&mut raw)?;
            candidates.push((path, manifest(&raw)?));
        }
    }
    ensure!(
        !candidates.is_empty(),
        "这个 npm 包不含标准 pet.json 与图集，仅脚本安装器尚不支持；请提供桌宠资源链接"
    );
    ensure!(
        candidates.len() == 1,
        "npm 包包含多个桌宠，请提供指定桌宠的资源链接，不会自动选择"
    );
    let (path, m) = candidates.pop().unwrap();
    let sheet = path
        .parent()
        .unwrap_or(Path::new(""))
        .join(relative(&m.spritesheet_path)?);
    let mut package =
        tar::Archive::new(GzDecoder::new(Cursor::new(bytes)).take(MAX_EXPANDED + 2 * 1024 * 1024));
    for entry in package.entries()? {
        crate::operations::check()?;
        let entry = entry?;
        if entry.path()?.as_ref() == sheet {
            ensure!(
                entry.header().entry_type().is_file() && entry.size() <= MAX_IMAGE,
                "npm 图集文件无效或过大"
            );
            let mut image = Vec::new();
            entry.take(MAX_IMAGE + 1).read_to_end(&mut image)?;
            ensure!(image.len() as u64 <= MAX_IMAGE, "npm 图集文件过大");
            return Ok((m, image));
        }
    }
    bail!("npm 素材包缺少 pet.json 指定的图集")
}
fn integrity(bytes: &[u8], expected: Option<&str>) -> Result<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let supported: Vec<_> = expected
        .split_whitespace()
        .filter_map(|part| part.split_once('-'))
        .filter(|(alg, _)| matches!(*alg, "sha512" | "sha256"))
        .collect();
    ensure!(!supported.is_empty(), "npm 素材包校验方式不支持");
    ensure!(
        supported.into_iter().any(|(alg, digest)| digest
            == if alg == "sha512" {
                STANDARD.encode(Sha512::digest(bytes))
            } else {
                STANDARD.encode(Sha256::digest(bytes))
            }),
        "npm 素材包校验失败，请重试"
    );
    Ok(())
}
fn npm(package: Package, fetcher: &impl Fetcher) -> Result<Acquisition> {
    let mut url = resource_url("https://registry.npmjs.org/")?;
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(&package.name)
        .push(&package.version);
    let info: Value = serde_json::from_slice(&fetcher.fetch(&url, MAX_REGISTRY)?.bytes)
        .context("npm 包信息无效")?;
    let tar_url = resource_url(
        info["dist"]["tarball"]
            .as_str()
            .context("npm 包未提供素材包下载地址")?,
    )?;
    let bytes = fetcher.fetch(&tar_url, MAX_PACKAGE)?.bytes;
    integrity(&bytes, info["dist"]["integrity"].as_str())?;
    let (manifest, bytes) = tarball(&bytes)?;
    let mut source = resource_url("https://www.npmjs.com/package/")?;
    source
        .path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(&package.name);
    if let Some(version) = info["version"].as_str() {
        source.path_segments_mut().unwrap().push("v").push(version);
    }
    Ok(Acquisition {
        manifest,
        bytes,
        source: Some(source.into()),
        creator: None,
    })
}
fn url(url: Url, fetcher: &impl Fetcher) -> Result<Acquisition> {
    let segments: Vec<_> = url
        .path_segments()
        .map(|p| p.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    let website_pet = segments
        .iter()
        .position(|s| *s == "pets")
        .filter(|index| *index <= 1 && segments.len() == index + 2 && !segments.contains(&"api"));
    if let Some(index) = website_pet {
        match url.host_str().unwrap_or("").trim_start_matches("www.") {
            "codex-pet.org" => return codex_pet(segments[index + 1], fetcher),
            "petdex.dev" => return petdex(segments[index + 1], fetcher),
            "codexpets.net" => {
                let id = slug(segments[index + 1])?;
                return self::url(
                    resource_url(&format!("https://codexpets.net/api/pets/{id}/download"))?,
                    fetcher,
                );
            }
            _ => {}
        }
    }
    let download = fetcher.fetch(&url, MAX_PACKAGE)?;
    let (manifest, bytes) = if download.bytes.starts_with(b"PK") {
        archive(&download.bytes)?
    } else if download.bytes.starts_with(&[0x1f, 0x8b]) {
        tarball(&download.bytes)?
    } else if download.bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{') {
        let m = manifest(&download.bytes)?;
        let sheet = download.url.join(&m.spritesheet_path)?;
        ensure!(valid_url(&sheet), "图集资源链接无效");
        let bytes = fetcher.fetch(&sheet, MAX_IMAGE)?.bytes;
        (m, bytes)
    } else {
        bail!("链接没有返回 ZIP 或 pet.json。请提供桌宠资源直链，或受支持的宠物页面");
    };
    Ok(Acquisition {
        manifest,
        bytes,
        source: Some(url.into()),
        creator: None,
    })
}
pub(super) fn resolve(input: Input, fetcher: &impl Fetcher) -> Result<Acquisition> {
    crate::operations::check()?;
    match input {
        Input::Local(path) => {
            let (manifest, bytes) = if path.file_name().is_some_and(|n| n == "pet.json") {
                unpacked(&path)?
            } else {
                archive(&bounded(&path, MAX_PACKAGE)?)?
            };
            Ok(Acquisition {
                manifest,
                bytes,
                source: Some(path.to_string_lossy().into_owned()),
                creator: None,
            })
        }
        Input::Url(source) => url(source, fetcher),
        Input::Npm(package) => npm(package, fetcher),
        Input::CodexPet(id) => codex_pet(&id, fetcher),
        Input::Petdex(id) => petdex(&id, fetcher),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::cell::RefCell;

    #[derive(Default)]
    struct Fixture {
        responses: BTreeMap<String, Download>,
        hits: RefCell<Vec<String>>,
    }
    impl Fixture {
        fn put(&mut self, url: &str, bytes: Vec<u8>) {
            self.responses.insert(
                url.into(),
                Download {
                    bytes,
                    url: Url::parse(url).unwrap(),
                },
            );
        }
    }
    impl Fetcher for Fixture {
        fn fetch(&self, url: &Url, limit: u64) -> Result<Download> {
            self.hits.borrow_mut().push(url.to_string());
            let response = self
                .responses
                .get(url.as_str())
                .with_context(|| format!("unexpected fetch: {url}"))?;
            ensure!(
                response.bytes.len() as u64 <= limit,
                "fixture resource exceeds limit"
            );
            Ok(Download {
                bytes: response.bytes.clone(),
                url: response.url.clone(),
            })
        }
    }
    fn archive_fixture(names: &[&str]) -> Vec<u8> {
        let (m, bytes) = super::super::tests::fixture(2, ImageFormat::Png);
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for name in names {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(&if name.ends_with("pet.json") {
                serde_json::to_vec(&m).unwrap()
            } else {
                bytes.clone()
            })
            .unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    fn npm_fixture(names: &[&str], link: bool) -> Vec<u8> {
        let (m, bytes) = super::super::tests::fixture(1, ImageFormat::Png);
        let mut tar = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for name in names {
            let data = if name.ends_with("pet.json") {
                serde_json::to_vec(&m).unwrap()
            } else if name.ends_with("package.json") {
                br#"{"scripts":{"postinstall":"never execute"}}"#.to_vec()
            } else {
                bytes.clone()
            };
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o644);
            header.set_size(if link { 0 } else { data.len() as u64 });
            if link {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name("outside").unwrap();
            }
            header.set_cksum();
            tar.append_data(&mut header, name, if link { &[][..] } else { &data })
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }
    #[test]
    fn input_forms_are_parsed_without_executing_commands() {
        for text in [
            "npx codex-pet-installer add boba",
            "npx -y codex-pet-installer@0.1.1 add boba",
            "npx --package=codex-pet-installer@latest -- codex-pet-installer add boba",
        ] {
            assert_eq!(parse_input(text).unwrap(), Input::CodexPet("boba".into()));
        }
        assert_eq!(
            parse_input("npx --yes petdex@next install boba").unwrap(),
            Input::Petdex("boba".into())
        );
        for text in [
            "@artist/pet@1.2.0",
            "npx -y @artist/pet@1.2.0",
            "npx --package @artist/pet@1.2.0 pet",
        ] {
            assert_eq!(
                parse_input(text).unwrap(),
                Input::Npm(Package {
                    name: "@artist/pet".into(),
                    version: "1.2.0".into()
                })
            );
        }
        assert_eq!(
            parse_input("pet-art@next").unwrap(),
            Input::Npm(Package {
                name: "pet-art".into(),
                version: "next".into()
            })
        );
        assert!(matches!(
            parse_input("https://example.test/download?id=one").unwrap(),
            Input::Url(_)
        ));
        assert!(matches!(
            parse_input("\"/tmp/my pet.codex-pet.zip\"").unwrap(),
            Input::Local(_)
        ));
        for text in [
            "",
            "npx petdex",
            "codex-pet-installer",
            "npx petdex install",
            "npx petdex install a b",
            "npx unknown install boba",
            "npx petdex install a; touch x",
            "npx $(touch x)",
            "https://user:secret@example.test/pet.json",
            "npx -p a -p b",
        ] {
            assert!(parse_input(text).is_err(), "{text}");
        }
    }
    #[test]
    fn download_sniffing_relative_images_and_redirect_bases() {
        let mut fixture = Fixture::default();
        fixture.put(
            "https://example.test/download?id=one",
            archive_fixture(&["nested/pet.json", "nested/spritesheet.png"]),
        );
        let acquired = resolve(
            parse_input("https://example.test/download?id=one").unwrap(),
            &fixture,
        )
        .unwrap();
        assert_eq!(acquired.manifest.sprite_version_number, 2);
        let (m, bytes) = super::super::tests::fixture(1, ImageFormat::Png);
        fixture.responses.insert(
            "https://example.test/manifest".into(),
            Download {
                bytes: serde_json::to_vec(&m).unwrap(),
                url: Url::parse("https://assets.example.test/one/pet.json").unwrap(),
            },
        );
        fixture.put("https://assets.example.test/one/spritesheet.png", bytes);
        assert!(
            resolve(
                parse_input("https://example.test/manifest").unwrap(),
                &fixture
            )
            .is_ok()
        );
        fixture.put(
            "https://example.test/page",
            b"<html>use another link</html>".to_vec(),
        );
        assert!(
            resolve(parse_input("https://example.test/page").unwrap(), &fixture)
                .unwrap_err()
                .to_string()
                .contains("资源直链")
        );
    }
    #[test]
    fn known_site_commands_resolve_assets_and_never_fetch_installers() {
        let mut fixture = Fixture::default();
        let (m, bytes) = super::super::tests::fixture(2, ImageFormat::Png);
        fixture.put(
            "https://codex-pet.org/api/pets/cat/files/pet.json",
            serde_json::to_vec(&m).unwrap(),
        );
        fixture.put(
            "https://codex-pet.org/api/pets/cat/files/spritesheet.png",
            bytes.clone(),
        );
        fixture.put(
            "https://codex-pet.org/api/pets/cat",
            br#"{"creator":{"name":"Artist"}}"#.to_vec(),
        );
        let from_page = resolve(
            parse_input("https://codex-pet.org/zh/pets/cat/").unwrap(),
            &fixture,
        )
        .unwrap();
        let from_command = resolve(
            parse_input("npx codex-pet-installer add cat").unwrap(),
            &fixture,
        )
        .unwrap();
        assert_eq!(from_page.bytes, from_command.bytes);
        assert_eq!(from_page.creator.as_deref(), Some("Artist"));
        fixture.put("https://petdex.dev/api/install-pet/boba", br#"{"ok":true,"pet":{"petJsonUrl":"https://assets.test/boba/manifest.json","spritesheetUrl":"https://assets.test/boba/sprite-v2.webp"}}"#.to_vec());
        fixture.put(
            "https://assets.test/boba/manifest.json",
            serde_json::to_vec(&m).unwrap(),
        );
        fixture.put("https://assets.test/boba/sprite-v2.webp", bytes);
        assert_eq!(
            resolve(parse_input("npx petdex install boba").unwrap(), &fixture)
                .unwrap()
                .manifest
                .sprite_version_number,
            2
        );
        assert!(
            fixture
                .hits
                .borrow()
                .iter()
                .all(|url| !url.contains("registry.npmjs.org"))
        );
        fixture.put(
            "https://codexpets.net/api/pets/pixel-coder/download",
            archive_fixture(&["pet.json", "spritesheet.png"]),
        );
        let page = resolve(
            parse_input("https://codexpets.net/pets/pixel-coder").unwrap(),
            &fixture,
        )
        .unwrap();
        let direct = resolve(
            parse_input("https://codexpets.net/api/pets/pixel-coder/download").unwrap(),
            &fixture,
        )
        .unwrap();
        assert_eq!(page.bytes, direct.bytes);
        assert_eq!(page.manifest.id, direct.manifest.id);
    }
    #[test]
    fn npm_resources_use_scoped_versions_integrity_and_bounded_tar_entries() {
        let package = npm_fixture(
            &[
                "package/package.json",
                "package/pets/pet.json",
                "package/pets/spritesheet.png",
            ],
            false,
        );
        let mut fixture = Fixture::default();
        fixture.put("https://registry.npmjs.org/@artist%2Fpet/1.2.0", serde_json::to_vec(&json!({"version":"1.2.0","dist":{"tarball":"https://assets.test/pet.tgz","integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(&package)))}})).unwrap());
        fixture.put("https://assets.test/pet.tgz", package.clone());
        let acquired = resolve(parse_input("@artist/pet@1.2.0").unwrap(), &fixture).unwrap();
        assert_eq!(acquired.manifest.id, "test");
        assert_eq!(fixture.hits.borrow().len(), 2);
        assert!(integrity(&package, Some("sha512-wrong")).is_err());
        assert!(
            tarball(&npm_fixture(&["package/package.json"], false))
                .unwrap_err()
                .to_string()
                .contains("不含标准")
        );
        assert!(
            tarball(&npm_fixture(
                &["package/a/pet.json", "package/b/pet.json"],
                false
            ))
            .unwrap_err()
            .to_string()
            .contains("多个桌宠")
        );
        assert!(tarball(&npm_fixture(&["package/pet.json"], true)).is_err());
    }
}
