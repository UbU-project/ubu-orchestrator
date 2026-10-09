//! Opt-in private replay of the exact minimized input handed to the kernel.
use crate::{
    config::ServerConfig,
    errors::{AppError, Result},
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use ubu_planning_core::PlanningRequest;

pub async fn write(request: &PlanningRequest, config: &ServerConfig) -> Result<()> {
    let Some(option) = config.planning_request_dump() else {
        return Ok(());
    };
    let default = option == Path::new("1");
    let path = if default {
        std::env::temp_dir().join(format!("ubu-planning-request-{}.json", std::process::id()))
    } else {
        option.to_owned()
    };
    let bytes =
        serde_json::to_vec_pretty(request).map_err(|e| AppError::Internal(e.to_string()))?;
    let reserved = reserved_paths(config);
    tokio::task::spawn_blocking(move || write_private(&path, default, &bytes, &reserved))
        .await
        .map_err(|_| AppError::Internal("Private planning request writer failed".into()))?
        .map_err(|e| AppError::Internal(format!("UBU_PLANNING_REQUEST_DUMP: {e}")))
}

fn reserved_paths(config: &ServerConfig) -> Vec<PathBuf> {
    let db = config
        .db_path()
        .strip_prefix("sqlite://")
        .or_else(|| config.db_path().strip_prefix("sqlite:"))
        .unwrap_or(config.db_path());
    let db = db.split('?').next().unwrap_or(db);
    let mut paths = vec![PathBuf::from(db), config.device_registration_path()];
    paths.extend(
        [
            config.google_credentials_path(),
            config.google_token_cache_path(),
        ]
        .into_iter()
        .flatten()
        .map(Path::to_owned),
    );
    paths
}

fn inside_worktree(parent: &Path) -> bool {
    parent
        .ancestors()
        .any(|ancestor| ancestor.join(".git").exists())
}

#[cfg(unix)]
fn write_private(path: &Path, default: bool, bytes: &[u8], reserved: &[PathBuf]) -> io::Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "use an absolute private output path",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "output parent required"))?
        .canonicalize()?;
    if default && inside_worktree(&parent) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the default temporary directory must be outside every repository working tree",
        ));
    }
    let path =
        parent.join(path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "output filename required")
        })?);
    let canonical_target = path.canonicalize().unwrap_or_else(|_| path.clone());
    if reserved
        .iter()
        .any(|r| r.canonicalize().ok().is_some_and(|r| r == canonical_target))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the output must not replace credentials or state",
        ));
    }
    let temporary = parent.join(format!(
        ".ubu-planning-{}.tmp",
        ubu_core::UbuId::new(ubu_core::ObjectType::Plan)
    ));
    let outcome = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, &path)
    })();
    if outcome.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    outcome
}

#[cfg(not(unix))]
fn write_private(_: &Path, _: bool, _: &[u8], _: &[PathBuf]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private replay requires owner-only 0600 file permissions",
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn replay_is_plain_json_owner_only_atomic_and_protects_reserved_state() {
        let root = std::env::temp_dir().join(format!(
            "synthetic-p80-{}",
            ubu_core::UbuId::new(ubu_core::ObjectType::Plan)
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("request.json");
        fs::write(&path, b"old synthetic output").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, false, b"{\"synthetic\":true}", &[]).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"{\"synthetic\":true}\n");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(write_private(&path, false, b"replacement", std::slice::from_ref(&path)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{\"synthetic\":true}\n");
        fs::create_dir(root.join(".git")).unwrap();
        assert!(write_private(&path, true, b"replacement", &[]).is_err());
        assert!(inside_worktree(&root));
        fs::remove_dir_all(root).unwrap();
    }
}
