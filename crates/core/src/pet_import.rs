//! Validated assets shared by file, URL and npm imports. No installer is executed.
mod acquire;
mod library;
use crate::store::Store;
use anyhow::{Context, Result, bail, ensure};
use image::{ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
};

const BUNDLED: &[u8] = include_bytes!("../assets/yuexinmiao1.codex-pet.zip");
const MAX_PACKAGE: u64 = 64 * 1024 * 1024;
const MAX_IMAGE: u64 = 32 * 1024 * 1024;
const MAX_MANIFEST: u64 = 64 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    id: String,
    display_name: String,
    #[serde(default)]
    description: String,
    spritesheet_path: String,
    #[serde(default = "v1")]
    sprite_version_number: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creator: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<Value>,
}
fn v1() -> u8 {
    1
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedPet {
    #[serde(default)]
    pub library_id: String,
    #[serde(default)]
    pub imported_at: String,
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub content_hash: String,
    pub sprite_hash: String,
    pub sprite_path: String,
    pub sprite_version_number: u8,
    pub columns: u32,
    pub rows: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub source: Option<String>,
    pub creator: Option<String>,
    #[serde(default)]
    pub recovered: bool,
}

#[derive(Debug)]
struct Acquisition {
    manifest: Manifest,
    bytes: Vec<u8>,
    source: Option<String>,
    creator: Option<String>,
}
fn creator_name(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|v| {
            v.as_str()
                .or_else(|| v["name"].as_str())
                .or_else(|| v["displayName"].as_str())
                .or_else(|| v["username"].as_str())
        })
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .map(str::to_owned)
}

fn bounded(path: &Path, max: u64) -> Result<Vec<u8>> {
    crate::operations::check()?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "不能导入符号链接或特殊文件"
    );
    ensure!(meta.len() <= max, "桌宠文件过大");
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "桌宠文件过大");
    Ok(bytes)
}
fn relative(name: &str) -> Result<PathBuf> {
    ensure!(
        !name.is_empty() && !name.contains(['\\', ':', '\0']),
        "桌宠路径必须是安全的相对路径"
    );
    let path = Path::new(name);
    ensure!(
        path.components().all(|p| matches!(p, Component::Normal(_))),
        "桌宠路径不能越过包目录"
    );
    Ok(path.to_owned())
}
fn manifest(bytes: &[u8]) -> Result<Manifest> {
    ensure!(bytes.len() as u64 <= MAX_MANIFEST, "pet.json 过大");
    let m: Manifest = serde_json::from_slice(bytes).context("pet.json 格式无效")?;
    ensure!(!m.id.trim().is_empty() && m.id.len() <= 128, "桌宠 id 无效");
    ensure!(
        !m.display_name.trim().is_empty()
            && m.display_name.len() <= 512
            && m.description.len() <= 8192,
        "桌宠名称或描述无效"
    );
    ensure!(
        matches!(m.sprite_version_number, 1 | 2),
        "仅支持 v1 / v2 桌宠图集"
    );
    relative(&m.spritesheet_path)?;
    Ok(m)
}
fn archive(bytes: &[u8]) -> Result<(Manifest, Vec<u8>)> {
    ensure!(bytes.len() as u64 <= MAX_PACKAGE, "桌宠压缩包过大");
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("无法读取桌宠 ZIP")?;
    ensure!(zip.len() <= 1024, "桌宠包文件数量过多");
    let mut files = BTreeMap::new();
    let mut expanded = 0u64;
    for i in 0..zip.len() {
        crate::operations::check()?;
        let file = zip.by_index(i)?;
        ensure!(
            file.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
            "桌宠包不能包含符号链接"
        );
        expanded = expanded
            .checked_add(file.size())
            .context("桌宠包大小无效")?;
        ensure!(expanded <= 128 * 1024 * 1024, "桌宠包解压后过大");
        if file.is_dir() {
            continue;
        }
        let path = relative(&file.name()?)?;
        if path.components().any(|p| {
            p.as_os_str() == "__MACOSX" || p.as_os_str().to_string_lossy().starts_with("._")
        }) {
            continue;
        }
        ensure!(files.insert(path, i).is_none(), "桌宠包包含重复文件");
    }
    let manifests: Vec<_> = files
        .keys()
        .filter(|p| p.file_name().is_some_and(|n| n == "pet.json"))
        .cloned()
        .collect();
    ensure!(manifests.len() == 1, "请选择只包含一个 pet.json 的桌宠包");
    let path = &manifests[0];
    let mut read = |index, max| -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let file = zip.by_index(index)?;
        ensure!(file.size() <= max, "桌宠文件过大");
        file.take(max + 1).read_to_end(&mut out)?;
        ensure!(out.len() as u64 <= max, "桌宠文件过大");
        Ok(out)
    };
    let m = manifest(&read(files[path], MAX_MANIFEST)?)?;
    let sheet = path
        .parent()
        .unwrap_or(Path::new(""))
        .join(relative(&m.spritesheet_path)?);
    let index = *files.get(&sheet).context("找不到 pet.json 指定的图集")?;
    Ok((m, read(index, MAX_IMAGE)?))
}
fn unpacked(path: &Path) -> Result<(Manifest, Vec<u8>)> {
    let m = manifest(&bounded(path, MAX_MANIFEST)?)?;
    let mut sheet = path.parent().context("pet.json 路径无效")?.to_owned();
    for part in relative(&m.spritesheet_path)?.components() {
        sheet.push(part);
        ensure!(
            !fs::symlink_metadata(&sheet)?.file_type().is_symlink(),
            "图集路径不能包含符号链接"
        );
    }
    Ok((m, bounded(&sheet, MAX_IMAGE)?))
}
fn normalize(m: &Manifest, bytes: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    crate::operations::check()?;
    let format = image::guess_format(bytes).context("图集不是有效图片")?;
    ensure!(
        matches!(format, ImageFormat::Png | ImageFormat::WebP),
        "图集仅支持 PNG / WebP"
    );
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format).into_dimensions()?;
    let rows = if m.sprite_version_number == 2 { 11 } else { 9 };
    ensure!(
        width > 0
            && height > 0
            && width <= 8192
            && height <= 8192
            && u64::from(width) * u64::from(height) <= 16_000_000,
        "图集像素数量超出限制"
    );
    ensure!(
        width % 8 == 0 && height % rows == 0 && (width / 8) * 13 == (height / rows) * 12,
        "图集尺寸必须为 8×{rows}，每格比例 192:208"
    );
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().context("图集解码失败")?.to_rgba8();
    crate::operations::check()?;
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut png, ImageFormat::Png)?;
    Ok((png.into_inner(), width / 8, height / rows))
}
// The library commit is short and follows acquisition/normalization. A slow
// download never holds either the library lock or the configuration lane.
fn install(
    store: &Store,
    m: Manifest,
    bytes: Vec<u8>,
    bundled: bool,
    only_if_empty: bool,
) -> Result<ImportedPet> {
    library::install(
        store,
        Acquisition {
            manifest: m,
            bytes,
            source: bundled.then(|| "https://codex-pet.org/zh/pets/yuexinmiao1/".into()),
            creator: bundled.then(|| "kiffin".into()),
        },
        only_if_empty,
    )
}
pub fn import(store: &Store, path: &Path) -> Result<ImportedPet> {
    let (m, bytes) = if path.file_name().is_some_and(|n| n == "pet.json") {
        unpacked(path)?
    } else if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        archive(&bounded(path, MAX_PACKAGE)?)?
    } else {
        bail!("请选择 .zip、.codex-pet.zip 或 pet.json 文件");
    };
    library::install(
        store,
        Acquisition {
            manifest: m,
            bytes,
            source: Some(path.to_string_lossy().into_owned()),
            creator: None,
        },
        false,
    )
}
pub fn import_input(store: &Store, input: &str) -> Result<ImportedPet> {
    crate::operations::check()?;
    let source = acquire::parse_input(input)?;
    if let acquire::Input::Local(path) = source {
        return import(store, &path);
    }
    // Client creation reads the app's local network preferences, never account
    // proxy values. Local files don't initialize a network client.
    let fetcher = acquire::HttpFetcher::new(&store.settings()?.proxy)?;
    let acquired = acquire::resolve(source, &fetcher)?;
    library::install(store, acquired, false)
}
pub fn current(store: &Store, include_image: bool) -> Result<Value> {
    library::current(store, include_image)
}
pub fn list(store: &Store) -> Result<Value> {
    library::list(store)
}
pub fn select(store: &Store, library_id: &str) -> Result<Value> {
    library::select(store, library_id)
}
pub fn asset(store: &Store, library_id: &str) -> Result<Value> {
    library::asset(store, library_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn fixture(version: u8, format: ImageFormat) -> (Manifest, Vec<u8>) {
        let m = Manifest {
            id: "test".into(),
            display_name: "测试".into(),
            description: String::new(),
            spritesheet_path: "spritesheet.png".into(),
            sprite_version_number: version,
            source: None,
            creator: None,
            author: None,
        };
        let image = image::DynamicImage::new_rgba8(96, 13 * if version == 2 { 11 } else { 9 });
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        (m, bytes.into_inner())
    }
    #[test]
    fn png_webp_versions_and_scaled_cells() {
        for version in [1, 2] {
            for format in [ImageFormat::Png, ImageFormat::WebP] {
                let (m, bytes) = fixture(version, format);
                let (_, w, h) = normalize(&m, &bytes).unwrap();
                assert_eq!((w, h), (12, 13));
            }
        }
    }
    #[test]
    fn bootstrap_is_offline_persistent_and_corruption_is_not_reset() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = current(&store, true).unwrap();
        assert_eq!(first["pet"]["id"], "yuexinmiao1");
        assert_eq!(first["pet"]["rows"], 9);
        assert!(
            first["imageDataUrl"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert_eq!(current(&store, false).unwrap()["pet"], first["pet"]);
        fs::write(first["pet"]["spritePath"].as_str().unwrap(), b"corrupt").unwrap();
        assert!(current(&store, false).unwrap()["error"].is_string());
        assert!(library::selected(&store).unwrap().is_some());
    }
    #[test]
    fn successful_import_replaces_and_failure_preserves_selection() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let (m, bytes) = fixture(2, ImageFormat::Png);
        let pet = install(&store, m, bytes, false, false).unwrap();
        let bad = root.path().join("bad.zip");
        fs::write(&bad, b"bad").unwrap();
        assert!(import(&store, &bad).is_err());
        assert_eq!(
            current(&store, false).unwrap()["pet"]["contentHash"],
            pet.content_hash
        );
        let reopened = Store::open(root.path()).unwrap();
        assert_eq!(current(&reopened, false).unwrap()["pet"]["id"], "test");
    }
    #[test]
    fn unsafe_ambiguous_and_appledouble_packages() {
        fn zip(names: &[&str]) -> Vec<u8> {
            let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
            let (m, bytes) = fixture(1, ImageFormat::Png);
            for name in names {
                z.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                z.write_all(
                    if name.ends_with("pet.json") {
                        serde_json::to_vec(&m).unwrap()
                    } else {
                        bytes.clone()
                    }
                    .as_slice(),
                )
                .unwrap();
            }
            z.finish().unwrap().into_inner()
        }
        assert!(
            archive(&zip(&[
                "a/pet.json",
                "a/spritesheet.png",
                "__MACOSX/._pet.json"
            ]))
            .is_ok()
        );
        assert!(archive(&zip(&["a/pet.json", "b/pet.json"])).is_err());
        assert!(archive(&zip(&["../pet.json"])).is_err());
        assert!(archive(&zip(&["pet.json"])).is_err());
        for name in ["../x", "/x", "a/../b", "C:/x", "a\\b"] {
            assert!(relative(name).is_err());
        }
    }
    #[test]
    fn rejected_images_and_oversized_files_preserve_the_current_pet() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let (m, bytes) = fixture(2, ImageFormat::WebP);
        fs::write(
            root.path().join("pet.json"),
            serde_json::to_vec(&m).unwrap(),
        )
        .unwrap();
        fs::write(root.path().join("spritesheet.png"), bytes).unwrap();
        let imported = import(&store, &root.path().join("pet.json")).unwrap();
        assert_eq!(imported.rows, 11);
        fs::write(root.path().join("spritesheet.png"), b"not an image").unwrap();
        assert!(import(&store, &root.path().join("pet.json")).is_err());
        let oversized = root.path().join("oversized.zip");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_PACKAGE + 1)
            .unwrap();
        assert!(import(&store, &oversized).is_err());
        assert_eq!(
            current(&store, false).unwrap()["pet"]["contentHash"],
            imported.content_hash
        );
        let mut invalid = serde_json::to_value(m).unwrap();
        invalid["spriteVersionNumber"] = json!(3);
        assert!(manifest(&serde_json::to_vec(&invalid).unwrap()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn unpacked_symlinks_cannot_escape_the_pet_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let (m, bytes) = fixture(1, ImageFormat::Png);
        let manifest = root.path().join("pet.json");
        fs::write(&manifest, serde_json::to_vec(&m).unwrap()).unwrap();
        let image = outside.path().join("spritesheet.png");
        fs::write(&image, bytes).unwrap();
        std::os::unix::fs::symlink(&image, root.path().join("spritesheet.png")).unwrap();
        assert!(unpacked(&manifest).is_err());
    }
}
