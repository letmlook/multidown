//! Shared persistence primitives. Domain migrations validate individual records.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct VersionedEnvelope<T> {
    pub schema_version: u32,
    pub written_at: String,
    pub data: T,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryWarning {
    pub id: String,
    pub domain: String,
    pub message: String,
    pub recovery_path: Option<PathBuf>,
    pub record_key: Option<String>,
    /// Only the local quarantine writer sees raw records, never the frontend.
    #[serde(skip_serializing)]
    pub(crate) rejected_value: Option<Value>,
}

#[derive(Debug)]
pub struct LoadReport<T> {
    pub data: T,
    pub warnings: Vec<RecoveryWarning>,
    pub schema_version: u32,
    pub migrated: bool,
    pub recovery_path: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported schema version {0}")]
    UnsupportedVersion(u32),
    #[error("invalid store envelope: {0}")]
    InvalidEnvelope(String),
    #[error("migration write failed: {0}")]
    Migration(#[source] Box<StoreError>),
    #[error("{domain}: {message}; preserved at {recovery_path}")]
    Corrupt {
        domain: String,
        message: String,
        recovery_path: PathBuf,
    },
}

pub fn load_store<T: Serialize>(
    path: &Path,
    domain: &'static str,
    migrate: impl FnOnce(u32, Value) -> Result<(T, Vec<RecoveryWarning>), StoreError>,
) -> Result<LoadReport<T>, StoreError> {
    let bytes = fs::read(path)?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let (schema_version, value) = match parsed {
        Ok(value) => {
            if value.as_object().is_some_and(|object| {
                object.contains_key("schema_version")
                    && object.contains_key("written_at")
                    && object.contains_key("data")
            }) {
                match serde_json::from_value::<VersionedEnvelope<Value>>(value) {
                    Ok(envelope)
                        if chrono::DateTime::parse_from_rfc3339(&envelope.written_at).is_ok() =>
                    {
                        (envelope.schema_version, envelope.data)
                    }
                    Ok(_) => {
                        return Err(preserve_corrupt(
                            path,
                            domain,
                            &bytes,
                            "written_at is not RFC3339".into(),
                        )?)
                    }
                    Err(error) => {
                        return Err(preserve_corrupt(path, domain, &bytes, error.to_string())?)
                    }
                }
            } else {
                (0, value)
            }
        }
        Err(error) => return Err(preserve_corrupt(path, domain, &bytes, error.to_string())?),
    };
    let (data, mut warnings) = migrate(schema_version, value)?;
    let source_path = serde_json::to_value(std::path::absolute(path)?)?;
    let rejected: Vec<Value> = warnings.iter().filter_map(|warning| warning.rejected_value.as_ref().map(|value| {
        serde_json::json!({ "source_path": source_path, "id": warning.id, "domain": warning.domain, "record_key": warning.record_key, "message": warning.message, "value": value })
    })).collect();
    let recovery_path = if rejected.is_empty() {
        None
    } else {
        let recovery = write_recovery(path, &serde_json::to_vec_pretty(&rejected)?)?;
        for warning in &mut warnings {
            if warning.rejected_value.is_some() {
                warning.recovery_path = Some(recovery.clone());
            }
        }
        Some(recovery)
    };
    if schema_version == 0 {
        // Quarantine must exist before rejected legacy records leave the current file.
        // Keep write failures distinct from a missing source on the first run.
        save_store(path, 1, &data).map_err(|error| StoreError::Migration(Box::new(error)))?;
    }
    Ok(LoadReport {
        data,
        warnings,
        schema_version,
        migrated: schema_version == 0,
        recovery_path,
    })
}

pub fn save_store<T: Serialize>(
    path: &Path,
    schema_version: u32,
    data: &T,
) -> Result<(), StoreError> {
    if schema_version < 1 {
        return Err(StoreError::UnsupportedVersion(schema_version));
    }
    let envelope = VersionedEnvelope {
        schema_version,
        written_at: chrono::Utc::now().to_rfc3339(),
        data,
    };
    let bytes = serde_json::to_vec_pretty(&envelope)?;
    let parent = parent_directory(path);
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}-{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let guard = TemporaryFile(Some(temporary.clone()));
    file.write_all(&bytes)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    if path.exists() {
        rotate_backups(path)?;
        fs::copy(path, suffixed_path(path, ".bak"))?;
        OpenOptions::new()
            .write(true)
            .open(suffixed_path(path, ".bak"))?
            .sync_all()?;
        // Persist backup names before replacing the current source.
        sync_directory(parent)?;
    }
    replace_file(&temporary, path)?;
    // If this fails, the complete new file is already visible; report the
    // durability failure without claiming that the replacement was rolled back.
    sync_directory(parent)?;
    drop(guard);
    Ok(())
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    {
        DIRECTORY_SYNC_PATHS.with(|paths| paths.borrow_mut().push(path.to_owned()));
        let call = DIRECTORY_SYNC_CALLS.with(|calls| {
            let call = calls.get() + 1;
            calls.set(call);
            call
        });
        if FAIL_DIRECTORY_SYNC_AT.with(|failure| failure.get() == Some(call)) {
            return Err(std::io::Error::other("injected directory sync failure"));
        }
    }
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        // Rust/Win32 provides no portable directory fsync equivalent. Windows
        // keeps MoveFileExW(MOVEFILE_WRITE_THROUGH); other non-Unix targets have
        // no directory synchronization implementation. This explicit no-op
        // does not claim parent-directory power-loss durability on those targets.
        let _ = path;
        Ok(())
    }
}

fn suffixed_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn rotate_backups(path: &Path) -> Result<(), StoreError> {
    let mut last = 0usize;
    while suffixed_path(path, &format!(".bak.{}", last + 1)).exists() {
        last += 1;
    }
    for index in (1..=last).rev() {
        fs::rename(
            suffixed_path(path, &format!(".bak.{index}")),
            suffixed_path(path, &format!(".bak.{}", index + 1)),
        )?;
    }
    let backup = suffixed_path(path, ".bak");
    if backup.exists() {
        fs::rename(backup, suffixed_path(path, ".bak.1"))?;
    }
    Ok(())
}

fn preserve_corrupt(
    path: &Path,
    domain: &str,
    bytes: &[u8],
    message: String,
) -> Result<StoreError, StoreError> {
    Ok(StoreError::Corrupt {
        domain: domain.into(),
        message,
        recovery_path: write_recovery(path, bytes)?,
    })
}

fn write_recovery(path: &Path, bytes: &[u8]) -> Result<PathBuf, StoreError> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ");
    let recovery = parent_directory(path).join(format!("{stem}.recovery-{timestamp}.json"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&recovery)?;
    let mut guard = TemporaryFile(Some(recovery.clone()));
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    drop(file);
    // Quarantine metadata must be durable before a legacy migration replaces
    // the source containing the only original copy of rejected records.
    sync_directory(parent_directory(path))?;
    guard.0.take();
    Ok(recovery)
}

struct TemporaryFile(Option<PathBuf>);
impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_REPLACE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DIRECTORY_SYNC_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static DIRECTORY_SYNC_PATHS: std::cell::RefCell<Vec<PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
    static FAIL_DIRECTORY_SYNC_AT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if FAIL_REPLACE.with(|flag| flag.replace(false)) {
        return Err(std::io::Error::other("injected replace failure"));
    }
    #[cfg(windows)]
    {
        windows_atomic::replace(source, destination)
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination)
    }
}

// BEGIN WINDOWS ATOMIC REPLACE
#[cfg(windows)]
mod windows_atomic {
    use std::{io, os::windows::ffi::OsStrExt, path::Path};

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, destination: *const u16, flags: u32) -> i32;
    }

    pub(super) fn replace(source: &Path, destination: &Path) -> io::Result<()> {
        fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
            let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
            if wide.contains(&0) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "path contains a NUL",
                ));
            }
            wide.push(0);
            Ok(wide)
        }
        let source = wide_path(source)?;
        let destination = wide_path(destination)?;
        // Same-directory temporary files ensure a same-volume rename. Deliberately
        // omit COPY_ALLOWED: copying then deleting cannot provide atomic replacement.
        // SAFETY: both buffers contain valid, NUL-terminated UTF-16 and remain alive
        // for the synchronous Win32 call. No handles or pointers escape the call.
        let result = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        #[test]
        fn replaces_existing_unicode_destination_without_deleting_first() {
            let directory = std::env::temp_dir().join(format!(
                "multidown-win-replace-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&directory).unwrap();
            let source = directory.join("新.tmp");
            let destination = directory.join("旧.json");
            fs::write(&source, b"new content").unwrap();
            fs::write(&destination, b"old content").unwrap();
            super::replace(&source, &destination).unwrap();
            assert_eq!(fs::read(&destination).unwrap(), b"new content");
            assert!(!source.exists());
            fs::remove_dir_all(directory).unwrap();
        }
    }
}
// END WINDOWS ATOMIC REPLACE

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::{fs, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("multidown-store-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> PathBuf {
            self.0.join("tasks.json")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn migration_on_read_preserves_legacy_backup_and_rewrites_once() {
        let fixture = Fixture::new();
        let original = b"[\n  \"legacy\"\n]\n";
        fs::write(fixture.path(), original).unwrap();
        let report = load_store(&fixture.path(), "tasks", |_, data| Ok((data, vec![]))).unwrap();
        assert!(report.migrated);
        assert_eq!(report.schema_version, 0);
        let disk: Value = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(disk["data"], json!(["legacy"]));
        assert_eq!(
            fs::read(fixture.0.join("tasks.json.bak")).unwrap(),
            original
        );
        let second = load_store(&fixture.path(), "tasks", |_, data| Ok((data, vec![]))).unwrap();
        assert!(!second.migrated);
        assert_eq!(second.schema_version, 1);
        assert!(!fixture.0.join("tasks.json.bak.1").exists());
    }

    #[test]
    fn migration_on_read_propagates_write_failure_and_keeps_legacy_source() {
        let fixture = Fixture::new();
        let original = b"[\"legacy\"]";
        fs::write(fixture.path(), original).unwrap();
        FAIL_REPLACE.with(|flag| flag.set(true));
        let result = load_store(&fixture.path(), "tasks", |_, data| Ok((data, vec![])));
        FAIL_REPLACE.with(|flag| flag.set(false));
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("migration write failed") && error.contains("injected replace failure")
        );
        assert_eq!(fs::read(fixture.path()).unwrap(), original);
        assert_eq!(
            fs::read(fixture.0.join("tasks.json.bak")).unwrap(),
            original
        );
        assert!(!fs::read_dir(&fixture.0).unwrap().any(|entry| entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")));
    }

    #[test]
    fn version_one_envelope_round_trips() {
        let fixture = Fixture::new();
        save_store(&fixture.path(), 1, &vec!["task"]).unwrap();
        let disk: Value = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(disk["data"], json!(["task"]));
        chrono::DateTime::parse_from_rfc3339(disk["written_at"].as_str().unwrap()).unwrap();
        let report = load_store(&fixture.path(), "tasks", |version, data| {
            assert_eq!(version, 1);
            Ok((serde_json::from_value::<Vec<String>>(data)?, vec![]))
        })
        .unwrap();
        assert_eq!(report.data, vec!["task"]);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn bare_value_is_version_zero() {
        let fixture = Fixture::new();
        fs::write(fixture.path(), r#"["legacy"]"#).unwrap();
        let report = load_store(&fixture.path(), "tasks", |version, data| {
            assert_eq!(version, 0);
            Ok((data, vec![]))
        })
        .unwrap();
        assert_eq!(report.data, json!(["legacy"]));
        assert_eq!(report.schema_version, 0);
    }

    #[test]
    fn rejects_schema_zero_before_creating_files_or_rotating_backups() {
        let fixture = Fixture::new();
        fs::write(fixture.path(), b"old store").unwrap();
        fs::write(fixture.0.join("tasks.json.bak"), b"old backup").unwrap();
        assert!(matches!(
            save_store(&fixture.path(), 0, &json!(["new"])),
            Err(StoreError::UnsupportedVersion(0))
        ));
        assert_eq!(fs::read(fixture.path()).unwrap(), b"old store");
        assert_eq!(
            fs::read(fixture.0.join("tasks.json.bak")).unwrap(),
            b"old backup"
        );
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
        let absent = fixture.0.join("absent").join("tasks.json");
        assert!(save_store(&absent, 0, &json!([])).is_err());
        assert!(!absent.parent().unwrap().exists());
    }

    #[test]
    fn legacy_objects_with_envelope_field_names_remain_version_zero() {
        let fixture = Fixture::new();
        for legacy in [
            json!({"schema_version": 7, "name": "legacy"}),
            json!({"schema_version": 7, "data": ["legacy"]}),
            json!({"schema_version": 7, "written_at": "legacy"}),
        ] {
            fs::write(fixture.path(), serde_json::to_vec(&legacy).unwrap()).unwrap();
            let report = load_store(&fixture.path(), "tasks", |version, data| {
                assert_eq!(version, 0);
                Ok((data, vec![]))
            })
            .unwrap();
            assert_eq!(report.data, legacy);
            assert!(report.migrated);
            assert!(report.recovery_path.is_none());
        }
    }

    #[test]
    fn quarantine_preserves_actual_source_path_and_record_metadata() {
        let fixture = Fixture::new();
        fs::write(fixture.path(), r#"[42]"#).unwrap();
        let report = load_store(&fixture.path(), "tasks", |_, _| {
            Ok((
                Vec::<String>::new(),
                vec![RecoveryWarning {
                    id: "tasks-record-0".into(),
                    domain: "tasks".into(),
                    record_key: Some("0".into()),
                    message: "invalid task".into(),
                    recovery_path: None,
                    rejected_value: Some(json!(42)),
                }],
            ))
        })
        .unwrap();
        let quarantined: Value =
            serde_json::from_slice(&fs::read(report.recovery_path.unwrap()).unwrap()).unwrap();
        assert_eq!(
            quarantined,
            json!([{"source_path": fixture.path(), "id": "tasks-record-0", "domain": "tasks", "record_key": "0", "message": "invalid task", "value": 42}])
        );
    }

    #[cfg(unix)]
    #[test]
    fn saves_sync_parent_after_backup_and_final_replace() {
        let fixture = Fixture::new();
        DIRECTORY_SYNC_CALLS.with(|calls| calls.set(0));
        DIRECTORY_SYNC_PATHS.with(|paths| paths.borrow_mut().clear());
        save_store(&fixture.path(), 1, &json!(["old"])).unwrap();
        assert_eq!(DIRECTORY_SYNC_CALLS.with(|calls| calls.get()), 1);
        DIRECTORY_SYNC_PATHS.with(|paths| assert_eq!(*paths.borrow(), vec![fixture.0.clone()]));
        let original = fs::read(fixture.path()).unwrap();
        DIRECTORY_SYNC_CALLS.with(|calls| calls.set(0));
        DIRECTORY_SYNC_PATHS.with(|paths| paths.borrow_mut().clear());
        save_store(&fixture.path(), 1, &json!(["new"])).unwrap();
        assert_eq!(DIRECTORY_SYNC_CALLS.with(|calls| calls.get()), 2);
        DIRECTORY_SYNC_PATHS
            .with(|paths| assert_eq!(*paths.borrow(), vec![fixture.0.clone(), fixture.0.clone()]));
        assert_eq!(
            fs::read(fixture.0.join("tasks.json.bak")).unwrap(),
            original
        );
        let disk: Value = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
        assert_eq!(disk["data"], json!(["new"]));
    }

    #[cfg(unix)]
    #[test]
    fn directory_sync_failures_propagate_at_each_write_boundary() {
        for fail_at in [1, 2] {
            let fixture = Fixture::new();
            save_store(&fixture.path(), 1, &json!(["old"])).unwrap();
            let original = fs::read(fixture.path()).unwrap();
            fs::write(fixture.0.join("tasks.json.bak"), b"earlier backup").unwrap();
            DIRECTORY_SYNC_CALLS.with(|calls| calls.set(0));
            FAIL_DIRECTORY_SYNC_AT.with(|failure| failure.set(Some(fail_at)));
            let result = save_store(&fixture.path(), 1, &json!(["new"]));
            FAIL_DIRECTORY_SYNC_AT.with(|failure| failure.set(None));
            assert!(
                matches!(result, Err(StoreError::Io(_))),
                "sync failure at boundary {fail_at} was ignored: {result:?}"
            );
            assert_eq!(DIRECTORY_SYNC_CALLS.with(|calls| calls.get()), fail_at);
            assert_eq!(
                fs::read(fixture.0.join("tasks.json.bak")).unwrap(),
                original
            );
            assert_eq!(
                fs::read(fixture.0.join("tasks.json.bak.1")).unwrap(),
                b"earlier backup"
            );
            let disk: Value = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
            assert_eq!(
                disk["data"],
                if fail_at == 1 {
                    json!(["old"])
                } else {
                    json!(["new"])
                }
            );
            assert!(!fs::read_dir(&fixture.0).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_str()
                .unwrap()
                .ends_with(".tmp")));
        }
    }

    #[cfg(unix)]
    #[test]
    fn recovery_sync_failure_aborts_migration_without_losing_source() {
        let fixture = Fixture::new();
        let original = b"[42]";
        fs::write(fixture.path(), original).unwrap();
        DIRECTORY_SYNC_CALLS.with(|calls| calls.set(0));
        FAIL_DIRECTORY_SYNC_AT.with(|failure| failure.set(Some(1)));
        let result = load_store(&fixture.path(), "tasks", |_, _| {
            Ok((
                Vec::<String>::new(),
                vec![RecoveryWarning {
                    id: "tasks-record-0".into(),
                    domain: "tasks".into(),
                    record_key: Some("0".into()),
                    message: "invalid task".into(),
                    recovery_path: None,
                    rejected_value: Some(json!(42)),
                }],
            ))
        });
        FAIL_DIRECTORY_SYNC_AT.with(|failure| failure.set(None));
        assert!(
            matches!(result, Err(StoreError::Io(_))),
            "recovery directory sync failure was ignored: {result:?}"
        );
        assert_eq!(fs::read(fixture.path()).unwrap(), original);
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
    }

    #[test]
    fn migration_on_read_preserves_mixed_record_evidence_and_rewrites_valid_records() {
        let fixture = Fixture::new();
        fs::write(fixture.path(), r#"["valid",42]"#).unwrap();
        let report = load_store(&fixture.path(), "tasks", |_, data| {
            let mut valid = vec![];
            let mut warnings = vec![];
            for value in data.as_array().unwrap() {
                if let Some(name) = value.as_str() {
                    valid.push(name.to_owned());
                } else {
                    warnings.push(RecoveryWarning {
                        id: "invalid-task".into(),
                        domain: "tasks".into(),
                        message: "invalid task".into(),
                        recovery_path: None,
                        record_key: Some("1".into()),
                        rejected_value: Some(value.clone()),
                    });
                }
            }
            Ok((valid, warnings))
        })
        .unwrap();
        assert_eq!(report.data, vec!["valid"]);
        let recovery = report.recovery_path.unwrap();
        assert!(recovery
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("tasks.recovery-"));
        let quarantined: Value = serde_json::from_slice(&fs::read(&recovery).unwrap()).unwrap();
        assert_eq!(quarantined.as_array().unwrap().len(), 1);
        assert_eq!(quarantined[0]["value"], 42);
        assert_eq!(quarantined[0]["record_key"], "1");
        assert_eq!(report.warnings[0].recovery_path.as_ref(), Some(&recovery));
        assert!(serde_json::to_value(&report.warnings).unwrap()[0]
            .get("rejected_value")
            .is_none());
        assert_eq!(
            fs::read_to_string(fixture.0.join("tasks.json.bak")).unwrap(),
            r#"["valid",42]"#
        );
        let disk: Value = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
        assert_eq!(disk["schema_version"], 1);
        assert_eq!(disk["data"], json!(["valid"]));
    }

    #[test]
    fn backup_rotates_to_previous_successful_contents() {
        let fixture = Fixture::new();
        save_store(&fixture.path(), 1, &json!([1])).unwrap();
        let first = fs::read(fixture.path()).unwrap();
        save_store(&fixture.path(), 1, &json!([2])).unwrap();
        assert_eq!(fs::read(fixture.0.join("tasks.json.bak")).unwrap(), first);
        let second = fs::read(fixture.path()).unwrap();
        save_store(&fixture.path(), 1, &json!([3])).unwrap();
        assert_eq!(fs::read(fixture.0.join("tasks.json.bak")).unwrap(), second);
        assert_eq!(fs::read(fixture.0.join("tasks.json.bak.1")).unwrap(), first);
        save_store(&fixture.path(), 1, &json!([4])).unwrap();
        assert_eq!(fs::read(fixture.0.join("tasks.json.bak.2")).unwrap(), first);
    }

    #[test]
    fn corruption_is_preserved_verbatim_without_overwriting_source() {
        let fixture = Fixture::new();
        let original = b"{broken json\xff";
        fs::write(fixture.path(), original).unwrap();
        let result = load_store::<Value>(&fixture.path(), "tasks", |_, _| {
            panic!("must not migrate invalid JSON")
        });
        let recovery = match result {
            Err(StoreError::Corrupt { recovery_path, .. }) => recovery_path,
            other => panic!("unexpected result: {other:?}"),
        };
        assert_eq!(fs::read(recovery).unwrap(), original);
        assert_eq!(fs::read(fixture.path()).unwrap(), original);
    }

    #[test]
    fn failed_replace_leaves_old_file_readable_and_cleans_temporary_files() {
        let fixture = Fixture::new();
        save_store(&fixture.path(), 1, &json!(["old"])).unwrap();
        FAIL_REPLACE.with(|flag| flag.set(true));
        assert!(save_store(&fixture.path(), 1, &json!(["new"])).is_err());
        let current: Value = serde_json::from_slice(&fs::read(fixture.path()).unwrap()).unwrap();
        assert_eq!(current["data"], json!(["old"]));
        assert!(!fs::read_dir(&fixture.0).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_str()
            .unwrap()
            .ends_with(".tmp")));
    }
}
