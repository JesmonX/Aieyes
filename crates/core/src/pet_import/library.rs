use super::*;
use crate::file_lock::FileLock;
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CURRENT: &str = "pet.current.v1";
const LIBRARY: &str = "pet.library.v1";

#[derive(Default, Deserialize, Serialize)]
struct Library {
    pets: Vec<ImportedPet>,
    #[serde(default)]
    warnings: Vec<Warning>,
}
#[derive(Clone, Deserialize, Serialize)]
struct Warning {
    path: String,
    error: String,
}

// Polling metadata must not reread a complete atlas. Keep only successful
// validation fingerprints, never the image itself. Replacements, removals and
// modifications invalidate the fingerprint before any cached result is used.
#[derive(Clone, PartialEq, Eq)]
struct Fingerprint {
    size: u64,
    modified: Option<SystemTime>,
    created: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}
impl From<&fs::Metadata> for Fingerprint {
    fn from(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
            created: metadata.created().ok(),
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        }
    }
}
#[derive(Clone)]
struct Validation {
    fingerprint: Fingerprint,
    hash: String,
}
static VALIDATED: OnceLock<Mutex<BTreeMap<PathBuf, Validation>>> = OnceLock::new();
fn validations() -> &'static Mutex<BTreeMap<PathBuf, Validation>> {
    VALIDATED.get_or_init(Default::default)
}
fn remember_validation(pet: &ImportedPet, fingerprint: Fingerprint) {
    let mut cache = validations().lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= 256 {
        cache.clear();
    }
    cache.insert(
        PathBuf::from(&pet.sprite_path),
        Validation {
            fingerprint,
            hash: pet.sprite_hash.clone(),
        },
    );
}
#[cfg(test)]
thread_local! { static IMAGE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub(super) fn selected(store: &Store) -> Result<Option<String>> {
    Ok(store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [CURRENT], |r| r.get(0))
        .optional()?)
}
fn stored_library(store: &Store) -> Result<Option<Library>> {
    let value: Option<String> = store
        .db
        .query_row("SELECT value FROM kv WHERE key=?1", [LIBRARY], |r| r.get(0))
        .optional()?;
    value
        .map(|s| serde_json::from_str(&s).context("桌宠库记录损坏"))
        .transpose()
}
fn lock(store: &Store) -> Result<FileLock> {
    let root = store.root.join("pets");
    fs::create_dir_all(&root)?;
    ensure!(
        !fs::symlink_metadata(&root)?.file_type().is_symlink(),
        "桌宠缓存目录不能是符号链接"
    );
    let started = Instant::now();
    loop {
        crate::operations::check()?;
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("import.lock"))?;
        match FileLock::try_exclusive(file) {
            Ok(lock) => return Ok(lock),
            Err(std::fs::TryLockError::WouldBlock)
                if started.elapsed() < Duration::from_secs(10) =>
            {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(error) => return Err(error).context("另一项桌宠更新正在进行，请重试"),
        }
    }
}
fn commit(store: &Store, library: &Library, selected: Option<&ImportedPet>) -> Result<()> {
    // Downloading, decoding and lock contention remain cancellable. Only this
    // short SQLite commit is protected from application-update cancellation.
    let tx = store.write_transaction()?;
    tx.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [LIBRARY, &serde_json::to_string(library)?])?;
    if let Some(pet) = selected {
        tx.execute("INSERT INTO kv(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [CURRENT, &serde_json::to_string(pet)?])?;
    }
    tx.commit()?;
    Ok(())
}
fn timestamp(directory: &Path) -> String {
    let from_name = directory
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.rsplit_once('-'))
        .and_then(|(_, n)| n.parse::<u128>().ok())
        .and_then(|ns| {
            i64::try_from(ns / 1_000_000_000).ok().and_then(|seconds| {
                DateTime::<Utc>::from_timestamp(seconds, (ns % 1_000_000_000) as u32)
            })
        });
    from_name
        .or_else(|| {
            fs::metadata(directory)
                .ok()
                .and_then(|m| m.created().or_else(|_| m.modified()).ok())
                .map(DateTime::<Utc>::from)
        })
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
        .unwrap_or_default()
}
fn content_hash(id: &str, name: &str, description: &str, version: u8, sprite_hash: &str) -> String {
    // Packaging paths and acquisition URLs do not create duplicate library pets.
    crate::models::hash(&json!([id, name, description, version, sprite_hash]).to_string())
}
fn image_fingerprint(store: &Store, pet: &ImportedPet) -> Result<Fingerprint> {
    let root = store.root.join("pets");
    let path = Path::new(&pet.sprite_path);
    let child = path.strip_prefix(&root).context("桌宠缓存路径无效")?;
    ensure!(
        child.components().count() == 2
            && child
                .components()
                .all(|p| matches!(p, Component::Normal(_)))
            && child.file_name().is_some_and(|s| s == "spritesheet.png"),
        "桌宠缓存路径无效"
    );
    ensure!(
        !fs::symlink_metadata(&root)?.file_type().is_symlink(),
        "桌宠缓存目录不能是符号链接"
    );
    ensure!(
        !fs::symlink_metadata(path.parent().context("桌宠缓存路径无效")?)?
            .file_type()
            .is_symlink(),
        "桌宠缓存目录不能是符号链接"
    );
    let metadata = fs::symlink_metadata(path).context("桌宠缓存丢失，请重新导入")?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() > 0
            && metadata.len() <= 128 * 1024 * 1024,
        "桌宠缓存文件无效，请重新导入"
    );
    Ok(Fingerprint::from(&metadata))
}
fn image_bytes(store: &Store, pet: &ImportedPet) -> Result<Vec<u8>> {
    let before = image_fingerprint(store, pet)?;
    let path = Path::new(&pet.sprite_path);
    #[cfg(test)]
    IMAGE_READS.with(|reads| reads.set(reads.get() + 1));
    let png = bounded(path, 128 * 1024 * 1024).context("桌宠缓存丢失，请重新导入")?;
    ensure!(
        format!("{:x}", Sha256::digest(&png)) == pet.sprite_hash,
        "桌宠缓存损坏，请重新导入"
    );
    ensure!(
        image_fingerprint(store, pet)? == before,
        "桌宠缓存已变化，请重试"
    );
    remember_validation(pet, before);
    Ok(png)
}
fn validate_image(store: &Store, pet: &ImportedPet) -> Result<()> {
    let fingerprint = image_fingerprint(store, pet)?;
    let valid = validations()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(Path::new(&pet.sprite_path))
        .is_some_and(|known| known.fingerprint == fingerprint && known.hash == pet.sprite_hash);
    if !valid {
        image_bytes(store, pet)?;
    }
    Ok(())
}
fn metadata(directory: &Path, pet: &ImportedPet) -> Result<()> {
    let path = directory.join("metadata.pending.json");
    let mut file = fs::File::create(&path)?;
    file.write_all(&serde_json::to_vec(pet)?)?;
    file.sync_all()?;
    fs::rename(path, directory.join("metadata.json"))?;
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}
fn recover(store: &Store, directory: &Path, known: Option<ImportedPet>) -> Result<ImportedPet> {
    ensure!(
        !fs::symlink_metadata(directory)?.file_type().is_symlink(),
        "桌宠缓存目录不能是符号链接"
    );
    let path = directory.join("spritesheet.png");
    let mut pet = if let Some(pet) = known {
        pet
    } else if directory.join("metadata.json").exists() {
        serde_json::from_slice(&bounded(&directory.join("metadata.json"), MAX_MANIFEST)?)
            .context("桌宠缓存元数据损坏")?
    } else {
        let bytes = bounded(&path, 128 * 1024 * 1024)?;
        let (width, height) =
            ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Png).into_dimensions()?;
        ensure!(
            width <= 8192 && height <= 8192 && u64::from(width) * u64::from(height) <= 16_000_000,
            "历史桌宠图集像素数量超出限制"
        );
        let version = [1u8, 2]
            .into_iter()
            .find(|version| {
                let rows = if *version == 2 { 11 } else { 9 };
                width > 0
                    && width % 8 == 0
                    && height % rows == 0
                    && (width / 8) * 13 == (height / rows) * 12
            })
            .context("历史桌宠图集尺寸无效")?;
        let imported_at = timestamp(directory);
        let sprite_hash = format!("{:x}", Sha256::digest(&bytes));
        let id = format!("legacy-{}", &sprite_hash[..16]);
        let name = if imported_at.len() >= 10 {
            format!("历史桌宠 · {}", &imported_at[..10])
        } else {
            "历史桌宠".into()
        };
        ImportedPet {
            library_id: String::new(),
            imported_at,
            id,
            display_name: name,
            description: String::new(),
            content_hash: String::new(),
            sprite_hash,
            sprite_path: path.to_string_lossy().into_owned(),
            sprite_version_number: version,
            columns: 8,
            rows: if version == 2 { 11 } else { 9 },
            cell_width: width / 8,
            cell_height: height / if version == 2 { 11 } else { 9 },
            source: None,
            creator: None,
            recovered: true,
        }
    };
    ensure!(
        Path::new(&pet.sprite_path) == path,
        "桌宠缓存元数据的路径不匹配"
    );
    let png = image_bytes(store, &pet)?;
    // Old metadata may be incomplete or out of sync with the file. Validate
    // geometry through the same importer instead of trusting cached sizes.
    let m = manifest(&serde_json::to_vec(&Manifest {
        id: pet.id.clone(),
        display_name: pet.display_name.clone(),
        description: pet.description.clone(),
        spritesheet_path: "spritesheet.png".into(),
        sprite_version_number: pet.sprite_version_number,
        source: None,
        creator: None,
        author: None,
    })?)?;
    let (_, cell_width, cell_height) = normalize(&m, &png)?;
    pet.columns = 8;
    pet.rows = if pet.sprite_version_number == 2 {
        11
    } else {
        9
    };
    pet.cell_width = cell_width;
    pet.cell_height = cell_height;
    if pet.imported_at.is_empty() {
        pet.imported_at = timestamp(directory);
    }
    pet.content_hash = content_hash(
        &pet.id,
        &pet.display_name,
        &pet.description,
        pet.sprite_version_number,
        &pet.sprite_hash,
    );
    if pet.library_id.is_empty() {
        pet.library_id = format!("pet-{}", pet.content_hash);
    }
    Ok(pet)
}
fn migrate_locked(store: &Store) -> Result<Library> {
    if let Some(library) = stored_library(store)? {
        return Ok(library);
    }
    let mut library = Library::default();
    let mut current = None;
    let old = selected(store)?;
    if let Some(raw) = old {
        match serde_json::from_str::<ImportedPet>(&raw) {
            Ok(pet) => {
                let directory = Path::new(&pet.sprite_path).parent().map(Path::to_owned);
                let result = directory
                    .as_deref()
                    .context("桌宠缓存路径无效")
                    .and_then(|p| recover(store, p, Some(pet)));
                match result {
                    Ok(pet) => {
                        library.pets.push(pet.clone());
                        current = Some(pet);
                    }
                    Err(error) => library.warnings.push(Warning {
                        path: directory
                            .map(|d| d.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "当前桌宠".into()),
                        error: format!("{error:#}"),
                    }),
                }
            }
            Err(error) => library.warnings.push(Warning {
                path: "当前桌宠".into(),
                error: format!("当前桌宠记录损坏：{error}"),
            }),
        }
    }
    let root = store.root.join("pets");
    let mut directories: Vec<_> = fs::read_dir(&root)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    directories.sort();
    for directory in directories {
        crate::operations::check()?;
        let recognized = directory
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.split_once('-'))
            .is_some_and(|(hash, stamp)| {
                hash.len() == 64
                    && hash.bytes().all(|b| b.is_ascii_hexdigit())
                    && stamp.bytes().all(|b| b.is_ascii_digit())
                    && !stamp.is_empty()
            });
        if !recognized
            || library
                .pets
                .iter()
                .any(|p| Path::new(&p.sprite_path).parent() == Some(directory.as_path()))
            || library
                .warnings
                .iter()
                .any(|w| Path::new(&w.path) == directory)
        {
            continue;
        }
        match recover(store, &directory, None) {
            Ok(pet) => {
                // Old imports saved only the image. Identical recovered images
                // reuse the identifiable entry without inventing old names.
                if !library.pets.iter().any(|p| {
                    p.content_hash == pet.content_hash
                        || ((p.recovered || pet.recovered)
                            && p.sprite_hash == pet.sprite_hash
                            && p.sprite_version_number == pet.sprite_version_number)
                }) {
                    library.pets.push(pet);
                }
            }
            Err(error) => library.warnings.push(Warning {
                path: directory.to_string_lossy().into_owned(),
                error: format!("{error:#}"),
            }),
        }
    }
    commit(store, &library, current.as_ref())?;
    Ok(library)
}
fn ensure_library(store: &Store) -> Result<Library> {
    if let Some(library) = stored_library(store)? {
        return Ok(library);
    }
    let _lock = lock(store)?;
    migrate_locked(store)
}

pub(super) fn install(
    store: &Store,
    acquired: Acquisition,
    only_if_empty: bool,
) -> Result<ImportedPet> {
    let Acquisition {
        manifest: m,
        bytes,
        source,
        creator,
    } = acquired;
    let (png, cell_width, cell_height) = normalize(&m, &bytes)?;
    crate::operations::check()?;
    let sprite_hash = format!("{:x}", Sha256::digest(&png));
    let hash = content_hash(
        &m.id,
        &m.display_name,
        &m.description,
        m.sprite_version_number,
        &sprite_hash,
    );
    let creator = creator
        .or_else(|| creator_name(m.creator.as_ref()))
        .or_else(|| creator_name(m.author.as_ref()));
    let source = source.or(m.source);
    let _lock = lock(store)?;
    let mut library = migrate_locked(store)?;
    if only_if_empty && let Some(raw) = selected(store)? {
        return Ok(serde_json::from_str(&raw)?);
    }
    let existing = library.pets.iter().position(|p| {
        p.content_hash == hash
            || (p.recovered
                && p.sprite_hash == sprite_hash
                && p.sprite_version_number == m.sprite_version_number)
    });
    if let Some(index) = existing
        && !library.pets[index].recovered
        && image_bytes(store, &library.pets[index]).is_ok()
    {
        let mut pet = library.pets[index].clone();
        if pet.source.is_none() {
            pet.source = source;
        }
        if pet.creator.is_none() {
            pet.creator = creator;
        }
        library.pets[index] = pet.clone();
        commit(store, &library, Some(&pet))?;
        return Ok(pet);
    }
    let root = store.root.join("pets");
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let directory = root.join(format!("{hash}-{stamp}"));
    // Incomplete files must never resemble a recoverable historical import.
    let staging = root.join(format!(".import-{hash}-{stamp}"));
    fs::create_dir(&staging)?;
    let path = directory.join("spritesheet.png");
    let pet = ImportedPet {
        library_id: existing
            .map(|i| library.pets[i].library_id.clone())
            .unwrap_or_else(|| format!("pet-{hash}")),
        imported_at: existing
            .map(|i| library.pets[i].imported_at.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        id: m.id,
        display_name: m.display_name,
        description: m.description,
        content_hash: hash,
        sprite_hash,
        sprite_path: path.to_string_lossy().into_owned(),
        sprite_version_number: m.sprite_version_number,
        columns: 8,
        rows: if m.sprite_version_number == 2 { 11 } else { 9 },
        cell_width,
        cell_height,
        source,
        creator,
        recovered: false,
    };
    let result = (|| -> Result<()> {
        let mut file = fs::File::create(staging.join("spritesheet.png"))?;
        file.write_all(&png)?;
        file.sync_all()?;
        drop(file);
        metadata(&staging, &pet)?;
        crate::operations::check()?;
        fs::rename(&staging, &directory)?;
        remember_validation(&pet, image_fingerprint(store, &pet)?);
        #[cfg(unix)]
        fs::File::open(&root)?.sync_all()?;
        crate::operations::check()?;
        if let Some(index) = existing {
            library.pets[index] = pet.clone();
        } else {
            library.pets.push(pet.clone());
        }
        commit(store, &library, Some(&pet))
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&directory);
    }
    result?;
    Ok(pet)
}
fn value(store: &Store, pet: ImportedPet, include_image: bool) -> Result<Value> {
    let png = if include_image {
        Some(image_bytes(store, &pet)?)
    } else {
        validate_image(store, &pet)?;
        None
    };
    let mut result = json!({"pet":pet,"error":null});
    if let Some(png) = png {
        result["imageDataUrl"] = json!(format!("data:image/png;base64,{}", STANDARD.encode(png)));
    }
    Ok(result)
}
pub(super) fn current(store: &Store, include_image: bool) -> Result<Value> {
    let result = (|| -> Result<Value> {
        ensure_library(store)?;
        let pet = match selected(store)? {
            Some(raw) => serde_json::from_str(&raw)?,
            None => {
                let (m, bytes) = archive(BUNDLED)?;
                super::install(store, m, bytes, true, true)?
            }
        };
        value(store, pet, include_image)
    })();
    Ok(result.unwrap_or_else(|error| json!({"pet":null,"error":format!("{error:#}")})))
}
pub(super) fn list(store: &Store) -> Result<Value> {
    // First use still installs the offline default through the same importer.
    current(store, false)?;
    let mut library = ensure_library(store)?;
    for pet in &library.pets {
        if let Err(error) = image_fingerprint(store, pet) {
            library.warnings.push(Warning {
                path: pet.sprite_path.clone(),
                error: format!("桌宠缓存不可用：{error}"),
            });
        }
    }
    library.pets.sort_by(|a, b| {
        b.imported_at
            .cmp(&a.imported_at)
            .then_with(|| a.library_id.cmp(&b.library_id))
    });
    let selected_id = selected(store)?
        .and_then(|raw| serde_json::from_str::<ImportedPet>(&raw).ok())
        .map(|p| p.library_id);
    Ok(json!({"pets":library.pets,"selectedLibraryId":selected_id,"warnings":library.warnings}))
}
pub(super) fn select(store: &Store, id: &str) -> Result<Value> {
    let _lock = lock(store)?;
    let library = migrate_locked(store)?;
    let pet = library
        .pets
        .iter()
        .find(|p| p.library_id == id)
        .context("桌宠不在库中，请刷新列表")?;
    let result = value(store, pet.clone(), false)?;
    commit(store, &library, Some(pet))?;
    Ok(result)
}
pub(super) fn asset(store: &Store, id: &str) -> Result<Value> {
    let library = ensure_library(store)?;
    let pet = library
        .pets
        .into_iter()
        .find(|p| p.library_id == id)
        .context("桌宠不在库中，请刷新列表")?;
    Ok(value(store, pet, true)
        .unwrap_or_else(|error| json!({"pet":null,"error":format!("{error:#}")})))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn acquired(color: u8) -> Acquisition {
        let (m, _) = super::super::tests::fixture(1, ImageFormat::Png);
        let image = image::RgbaImage::from_pixel(96, 117, image::Rgba([color, 0, 0, 255]));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, ImageFormat::Png).unwrap();
        Acquisition {
            manifest: m,
            bytes: png.into_inner(),
            source: Some("https://example.test/pet".into()),
            creator: Some("Artist".into()),
        }
    }
    #[test]
    fn library_keeps_same_names_deduplicates_content_and_persists_selection() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = install(&store, acquired(1), false).unwrap();
        let second = install(&store, acquired(2), false).unwrap();
        assert_ne!(first.library_id, second.library_id);
        assert_eq!(first.display_name, second.display_name);
        assert_eq!(
            install(&store, acquired(1), false).unwrap().library_id,
            first.library_id
        );
        let listing = list(&store).unwrap();
        assert_eq!(listing["pets"].as_array().unwrap().len(), 2);
        assert_eq!(listing["selectedLibraryId"], first.library_id);
        assert!(!listing.to_string().contains("imageDataUrl"));
        assert!(Path::new(&second.sprite_path).exists());
        select(&store, &second.library_id).unwrap();
        let reopened = Store::open(root.path()).unwrap();
        assert_eq!(
            current(&reopened, false).unwrap()["pet"]["libraryId"],
            second.library_id
        );
        assert!(
            asset(&reopened, &first.library_id).unwrap()["imageDataUrl"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }
    #[test]
    fn legacy_recovery_keeps_current_and_all_valid_cache_content() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let a = install(&store, acquired(1), false).unwrap();
        let b = install(&store, acquired(2), false).unwrap();
        let current_pet = install(&store, acquired(3), false).unwrap();
        let mut raw = serde_json::to_value(&current_pet).unwrap();
        for key in ["libraryId", "importedAt", "recovered"] {
            raw.as_object_mut().unwrap().remove(key);
        }
        store
            .db
            .execute(
                "UPDATE kv SET value=?1 WHERE key=?2",
                [raw.to_string(), CURRENT.into()],
            )
            .unwrap();
        store
            .db
            .execute("DELETE FROM kv WHERE key=?1", [LIBRARY])
            .unwrap();
        for p in [&a, &b, &current_pet] {
            fs::remove_file(
                Path::new(&p.sprite_path)
                    .parent()
                    .unwrap()
                    .join("metadata.json"),
            )
            .unwrap();
        }
        let broken = root
            .path()
            .join("pets")
            .join(format!("{}-1728888888123456789", "f".repeat(64)));
        fs::create_dir(&broken).unwrap();
        fs::write(broken.join("spritesheet.png"), b"bad").unwrap();
        // A cache from another application or an arbitrary directory is ignored.
        fs::create_dir(root.path().join("pets/not-an-aieyes-cache")).unwrap();
        let listing = list(&store).unwrap();
        let pets = listing["pets"].as_array().unwrap();
        assert_eq!(pets.len(), 3);
        assert_eq!(listing["selectedLibraryId"], current_pet.library_id);
        assert_eq!(current(&store, false).unwrap()["pet"]["id"], current_pet.id);
        let recovered: Vec<_> = pets.iter().filter(|p| p["recovered"] == true).collect();
        assert_eq!(recovered.len(), 2);
        assert!(recovered.iter().all(|p| {
            p["displayName"]
                .as_str()
                .unwrap()
                .starts_with("历史桌宠 · ")
                && p["importedAt"].as_str().is_some_and(|s| !s.is_empty())
        }));
        assert_eq!(listing["warnings"].as_array().unwrap().len(), 1);
        assert!(
            listing["warnings"][0]["path"]
                .as_str()
                .unwrap()
                .ends_with("1728888888123456789")
        );
        assert_eq!(list(&store).unwrap(), listing);
    }
    #[test]
    fn broken_selection_does_not_replace_current_and_reimport_repairs_in_place() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = install(&store, acquired(1), false).unwrap();
        let second = install(&store, acquired(2), false).unwrap();
        fs::write(&first.sprite_path, b"broken").unwrap();
        assert!(select(&store, &first.library_id).is_err());
        assert_eq!(
            current(&store, false).unwrap()["pet"]["libraryId"],
            second.library_id
        );
        let repaired = install(&store, acquired(1), false).unwrap();
        assert_eq!(repaired.library_id, first.library_id);
        assert_eq!(list(&store).unwrap()["pets"].as_array().unwrap().len(), 2);
        assert!(current(&store, true).unwrap()["error"].is_null());
    }
    #[test]
    fn failed_commit_rolls_back_library_selection_and_new_image_directory() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = install(&store, acquired(1), false).unwrap();
        store.db.execute_batch("CREATE TRIGGER reject_pet_selection BEFORE UPDATE ON kv WHEN NEW.key='pet.current.v1' BEGIN SELECT RAISE(ABORT, 'fixture commit failure'); END;").unwrap();
        assert!(install(&store, acquired(2), false).is_err());
        assert_eq!(
            current(&store, false).unwrap()["pet"]["libraryId"],
            first.library_id
        );
        assert_eq!(list(&store).unwrap()["pets"].as_array().unwrap().len(), 1);
        assert_eq!(
            fs::read_dir(root.path().join("pets"))
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count(),
            1
        );
    }

    #[test]
    fn metadata_polling_reuses_validation_and_detects_changed_or_removed_images() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = install(&store, acquired(1), false).unwrap();
        let selected = install(&store, acquired(2), false).unwrap();
        IMAGE_READS.with(|reads| reads.set(0));
        for _ in 0..10 {
            assert_eq!(
                current(&store, false).unwrap()["pet"]["libraryId"],
                selected.library_id
            );
            assert_eq!(list(&store).unwrap()["pets"].as_array().unwrap().len(), 2);
        }
        assert_eq!(
            IMAGE_READS.with(|reads| reads.get()),
            0,
            "metadata polls must not read atlases"
        );
        assert!(asset(&store, &first.library_id).unwrap()["imageDataUrl"].is_string());
        assert_eq!(
            IMAGE_READS.with(|reads| reads.get()),
            1,
            "request only the chosen preview image"
        );
        let original = fs::read(&selected.sprite_path).unwrap();
        let mut corrupt = original.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        fs::write(&selected.sprite_path, corrupt).unwrap();
        assert!(
            current(&store, false).unwrap()["error"].is_string(),
            "same-size writes must invalidate validation"
        );
        assert_eq!(IMAGE_READS.with(|reads| reads.get()), 2);
        let replacement = Path::new(&selected.sprite_path).with_file_name("replacement.png");
        fs::write(&replacement, &original).unwrap();
        fs::remove_file(&selected.sprite_path).unwrap();
        fs::rename(replacement, &selected.sprite_path).unwrap();
        assert!(current(&store, false).unwrap()["error"].is_null());
        assert_eq!(IMAGE_READS.with(|reads| reads.get()), 3);
        fs::remove_file(&selected.sprite_path).unwrap();
        assert!(current(&store, false).unwrap()["error"].is_string());
        assert!(
            list(&store).unwrap()["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["path"] == selected.sprite_path)
        );
        assert_eq!(
            serde_json::from_str::<ImportedPet>(&super::selected(&store).unwrap().unwrap())
                .unwrap()
                .library_id,
            selected.library_id
        );
    }

    #[test]
    fn concurrent_imports_and_selection_keep_every_unique_pet() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let first = install(&store, acquired(1), false).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(7));
        let mut handles = Vec::new();
        for index in 0..6 {
            let root = root.path().to_owned();
            let selected = first.library_id.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let store = Store::open(&root).unwrap();
                barrier.wait();
                if index == 5 {
                    select(&store, &selected).unwrap();
                } else {
                    install(&store, acquired(1 + (index % 3) as u8), false).unwrap();
                }
            }));
        }
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }
        let listing = list(&store).unwrap();
        let pets = listing["pets"].as_array().unwrap();
        assert_eq!(pets.len(), 3);
        assert!(
            pets.iter()
                .any(|pet| pet["libraryId"] == listing["selectedLibraryId"])
        );
        for pet in pets {
            assert!(asset(&store, pet["libraryId"].as_str().unwrap()).unwrap()["error"].is_null());
        }
        assert_eq!(
            fs::read_dir(root.path().join("pets"))
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count(),
            3
        );
    }

    #[test]
    fn recovery_validates_metadata_geometry_and_ignores_incomplete_staging() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let pet = install(&store, acquired(1), false).unwrap();
        let mut old = pet.clone();
        old.cell_width = 999;
        old.rows = 500;
        store
            .db
            .execute(
                "UPDATE kv SET value=?1 WHERE key=?2",
                [serde_json::to_string(&old).unwrap(), CURRENT.into()],
            )
            .unwrap();
        store
            .db
            .execute("DELETE FROM kv WHERE key=?1", [LIBRARY])
            .unwrap();
        let staging = root
            .path()
            .join("pets")
            .join(format!(".import-{}-123456789", "a".repeat(64)));
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("spritesheet.png"), b"incomplete").unwrap();
        let listing = list(&store).unwrap();
        assert_eq!(listing["pets"].as_array().unwrap().len(), 1);
        assert!(listing["warnings"].as_array().unwrap().is_empty());
        assert_eq!(listing["pets"][0]["cellWidth"], 12);
        assert_eq!(listing["pets"][0]["rows"], 9);
        assert_eq!(listing["selectedLibraryId"], pet.library_id);
    }

    #[test]
    fn cancelled_download_parse_and_commit_preserve_selection() {
        // Update coordination is process-global. Isolate this test so cancelling
        // its import cannot cancel unrelated core tests running in parallel.
        const FLAG: &str = "AIEYES_PET_CANCEL_FIXTURE";
        if std::env::var_os(FLAG).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "pet_import::library::tests::cancelled_download_parse_and_commit_preserve_selection", "--test-threads=1"])
                .env(FLAG, "1").output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        store
            .save_settings(&crate::models::Settings {
                proxy: crate::models::ProxyConfig {
                    mode: "direct".into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
        let original = install(&store, acquired(1), false).unwrap();
        {
            let _scope = crate::operations::enter("pets.import").unwrap();
            let status = crate::operations::prepare();
            assert!(status["protected"].as_array().unwrap().is_empty());
            assert!(
                archive(BUNDLED).is_err(),
                "cancel before parsing an archive"
            );
            assert!(
                install(&store, acquired(2), false).is_err(),
                "cancel before decoding or updating the library"
            );
            assert!(
                commit(&store, &Library::default(), None).is_err(),
                "a cancelled operation cannot begin a commit"
            );
        }
        crate::operations::resume();
        {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let (mut connection, _) = listener.accept().unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0u8; 2048];
                assert!(connection.read(&mut request).unwrap() > 0);
                crate::operations::prepare();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    BUNDLED.len()
                );
                let _ = connection.write_all(header.as_bytes());
                let _ = connection.write_all(BUNDLED);
            });
            let _scope = crate::operations::enter("pets.import").unwrap();
            assert!(
                super::super::import_input(&store, &format!("http://{address}/download")).is_err(),
                "cancel during resource acquisition"
            );
            server.join().unwrap();
        }
        crate::operations::resume();
        assert_eq!(
            current(&store, false).unwrap()["pet"]["libraryId"],
            original.library_id
        );
        assert_eq!(list(&store).unwrap()["pets"].as_array().unwrap().len(), 1);
        assert_eq!(
            fs::read_dir(root.path().join("pets"))
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count(),
            1
        );
    }
}
