use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};

const DRIVE_SECRET_DIR: &str = "accounts/secrets";
const SECRET_EXTENSION: &str = "txt";

#[derive(Clone)]
pub struct SecretStore {
    dir: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

#[derive(Clone)]
pub struct StoredSecret {
    pub reference: String,
    pub value: String,
}

pub struct SecretMaterial {
    value: String,
}

impl std::fmt::Debug for SecretMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretMaterial(<redacted>)")
    }
}

impl SecretMaterial {
    pub fn expose_to_broker(&self) -> &str {
        &self.value
    }
}

impl SecretStore {
    pub fn open(data_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            dir: crate::mind::memory::media::drive_root(data_dir).join(DRIVE_SECRET_DIR),
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn upsert_detected(&self, value: &str, kind: &str) -> anyhow::Result<String> {
        if value.is_empty() {
            bail!("refusing to store an empty secret");
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow::anyhow!("secret file lock is poisoned"))?;
        let records = self.read_all()?;

        if let Some((path, _)) = records.iter().find(|(_, stored)| stored == value) {
            return Ok(reference_from_path(&self.dir, path)?);
        }

        let path = self.next_path(kind, &records);
        self.write_value(&path, value)?;
        reference_from_path(&self.dir, &path)
    }

    pub fn active_values(&self) -> anyhow::Result<Vec<StoredSecret>> {
        let mut values = self
            .read_all()?
            .into_iter()
            .map(|(path, value)| StoredSecret {
                reference: reference_from_path(&self.dir, &path)
                    .expect("secret store only returns paths inside its directory"),
                value,
            })
            .collect::<Vec<_>>();
        values.sort_by(|a, b| b.value.len().cmp(&a.value.len()));
        Ok(values)
    }

    pub fn resolve_for_http(&self, secret_ref: &str) -> anyhow::Result<SecretMaterial> {
        let file = parse_reference(secret_ref)?;
        let path = self.dir.join(file);
        let value = fs::read_to_string(&path)
            .with_context(|| format!("reading secret file {}", path.display()))?;
        if value.is_empty() {
            bail!("secret file is empty");
        }
        Ok(SecretMaterial { value })
    }

    fn read_all(&self) -> anyhow::Result<Vec<(PathBuf, String)>> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading secret directory {}", self.dir.display()));
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| {
                format!(
                    "reading an entry from secret directory {}",
                    self.dir.display()
                )
            })?;
            let file_type = entry
                .file_type()
                .with_context(|| format!("reading secret file type {}", entry.path().display()))?;
            if !file_type.is_file()
                || entry.path().extension().and_then(|ext| ext.to_str()) != Some(SECRET_EXTENSION)
            {
                continue;
            }
            let path = entry.path();
            let value = fs::read_to_string(&path)
                .with_context(|| format!("reading secret file {}", path.display()))?;
            if !value.is_empty() {
                records.push((path, value));
            }
        }
        Ok(records)
    }

    fn next_path(&self, kind: &str, records: &[(PathBuf, String)]) -> PathBuf {
        let stem = filename_stem(kind);
        for suffix in 1.. {
            let name = if suffix == 1 {
                format!("{stem}.{SECRET_EXTENSION}")
            } else {
                format!("{stem}-{suffix}.{SECRET_EXTENSION}")
            };
            let path = self.dir.join(name);
            if !records.iter().any(|(existing, _)| existing == &path) && !path.exists() {
                return path;
            }
        }
        unreachable!("a free secret filename always exists")
    }

    fn write_value(&self, path: &Path, value: &str) -> anyhow::Result<()> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating secret directory {}", self.dir.display()))?;
        let tmp = self.dir.join(format!(
            ".{}.{}.tmp",
            path.file_stem().unwrap().to_string_lossy(),
            unique_suffix()
        ));
        fs::write(&tmp, value.as_bytes())
            .with_context(|| format!("writing secret file {}", tmp.display()))?;
        set_private_file_permissions(&tmp)?;
        fs::File::open(&tmp)
            .and_then(|file| file.sync_all())
            .with_context(|| format!("syncing secret file {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("publishing secret file {}", path.display()))
    }
}

fn filename_stem(kind: &str) -> String {
    let mut stem = String::new();
    for byte in kind.bytes() {
        if byte.is_ascii_alphanumeric() {
            stem.push((byte as char).to_ascii_lowercase());
        } else if !stem.ends_with('-') {
            stem.push('-');
        }
    }
    let stem = stem.trim_matches('-');
    if stem.is_empty() {
        "secret".to_string()
    } else {
        stem.to_string()
    }
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn reference_from_path(dir: &Path, path: &Path) -> anyhow::Result<String> {
    let file = path
        .strip_prefix(dir)
        .ok()
        .and_then(|relative| relative.to_str())
        .context("secret file escaped its directory")?;
    Ok(format!("drive/{DRIVE_SECRET_DIR}/{file}"))
}

fn parse_reference(value: &str) -> anyhow::Result<&str> {
    let prefix = format!("drive/{DRIVE_SECRET_DIR}/");
    let Some(file) = value.strip_prefix(&prefix) else {
        bail!("invalid secret reference");
    };
    if file.is_empty()
        || file.contains('/')
        || !file.ends_with(&format!(".{SECRET_EXTENSION}"))
        || !file[..file.len() - SECRET_EXTENSION.len() - 1]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("invalid secret reference");
    }
    Ok(file)
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("setting secret file permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detected_values_are_deduplicated_as_plain_text_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path()).unwrap();

        let first = store
            .upsert_detected("portable-secret", "OPENAI_API_KEY")
            .unwrap();
        let second = store
            .upsert_detected("portable-secret", "OPENAI_API_KEY")
            .unwrap();

        assert_eq!(first, second);
        assert!(first.ends_with("openai-api-key.txt"));
        assert_eq!(
            fs::read_to_string(dir.path().join(&first)).unwrap(),
            "portable-secret"
        );
        assert_eq!(store.active_values().unwrap().len(), 1);
    }

    #[test]
    fn same_kind_gets_a_readable_suffix_without_uuid() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::open(dir.path()).unwrap();

        let first = store.upsert_detected("first", "GENERIC_SECRET").unwrap();
        let second = store.upsert_detected("second", "GENERIC_SECRET").unwrap();

        assert!(first.ends_with("generic-secret.txt"));
        assert!(second.ends_with("generic-secret-2.txt"));
        assert!(!first.contains("sec_"));
        assert!(!second.contains("sec_"));
    }

    #[test]
    fn copying_the_drive_carries_resolvable_secret_files() {
        let source = tempfile::tempdir().unwrap();
        let store = SecretStore::open(source.path()).unwrap();
        let reference = store
            .upsert_detected("portable-api-key", "GENERIC_SECRET")
            .unwrap();
        drop(store);

        let destination = tempfile::tempdir().unwrap();
        let source_file = source.path().join(&reference);
        let destination_file = destination.path().join(&reference);
        fs::create_dir_all(destination_file.parent().unwrap()).unwrap();
        fs::copy(source_file, destination_file).unwrap();

        let reopened = SecretStore::open(destination.path()).unwrap();
        let material = reopened.resolve_for_http(&reference).unwrap();
        assert_eq!(material.expose_to_broker(), "portable-api-key");
    }
}
