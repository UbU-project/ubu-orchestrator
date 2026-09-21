use std::fs;
use std::path::PathBuf;

use ubu_core::{DeviceId, TrustState};
use ubu_orchestrator::config::ServerConfig;
use ubu_orchestrator::device_registration::{load_or_register, new_registration};
use ubu_orchestrator::state::AppState;

struct TestDirectory(PathBuf);
impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "ubu-registration-{}",
            DeviceId::generate().as_str()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn config(&self) -> ServerConfig {
        ServerConfig::from_env()
            .with_db_path(self.0.join("state.db").to_str().unwrap())
            .with_device_registration_path(self.0.join("registration.json"))
    }
}
impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn first_start_persists_registration_and_restart_or_database_reset_restores_it() {
    let dir = TestDirectory::new();
    let config = dir.config();
    let path = config.device_registration_path();
    assert!(!path.exists());
    let first = AppState::new(config.clone()).await.unwrap();
    let registration = first.inner().device_registration.clone();
    registration.validate().unwrap();
    assert!(registration.may_originate_mutations());
    assert!(registration.device_id.as_str().starts_with("dev_"));
    assert_eq!(
        first.actor_identity_id(),
        &registration.registered_identity_id
    );
    let original = fs::read(&path).unwrap();
    assert_eq!(
        serde_json::from_slice::<ubu_core::DeviceRegistration>(&original).unwrap(),
        registration
    );
    let objects: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM objects")
        .fetch_one(first.inner().store.pool())
        .await
        .unwrap();
    assert_eq!(
        objects, 0,
        "registration material is not admitted database state"
    );
    first.inner().store.pool().close().await;
    let second = AppState::new(config.clone()).await.unwrap();
    assert_eq!(second.inner().device_registration, registration);
    assert_eq!(fs::read(&path).unwrap(), original);
    second.inner().store.pool().close().await;
    let reset = AppState::new(config.with_db_path(dir.0.join("reset.db").to_str().unwrap()))
        .await
        .unwrap();
    assert_eq!(reset.inner().device_registration, registration);
    reset.inner().store.pool().close().await;
}

#[tokio::test]
async fn invalid_registration_refuses_start_without_replacing_material_or_opening_database() {
    let dir = TestDirectory::new();
    let config = dir.config();
    for bytes in [b"not JSON".as_slice(), b"{}".as_slice()] {
        fs::write(config.device_registration_path(), bytes).unwrap();
        let error = AppState::new(config.clone())
            .await
            .err()
            .expect("startup refused");
        assert!(error.to_string().contains("registration"));
        assert_eq!(fs::read(config.device_registration_path()).unwrap(), bytes);
        assert!(!dir.0.join("state.db").exists());
    }
    let mut value = serde_json::to_value(new_registration()).unwrap();
    value["label"] = serde_json::json!("");
    let invalid = serde_json::to_vec(&value).unwrap();
    fs::write(config.device_registration_path(), &invalid).unwrap();
    assert!(AppState::new(config.clone()).await.is_err());
    assert_eq!(
        fs::read(config.device_registration_path()).unwrap(),
        invalid
    );
}

#[tokio::test]
async fn revoked_file_and_direct_registration_both_refuse_startup() {
    let dir = TestDirectory::new();
    let config = dir.config();
    let mut registration = new_registration();
    registration.trust_state = TrustState::Revoked;
    let original = serde_json::to_vec(&registration).unwrap();
    fs::write(config.device_registration_path(), &original).unwrap();
    let error = AppState::new(config.clone()).await.err().expect("revoked");
    assert!(error.to_string().contains("revoked"));
    assert_eq!(
        fs::read(config.device_registration_path()).unwrap(),
        original
    );
    assert!(!dir.0.join("state.db").exists());
    assert!(AppState::in_memory_with_registration(config, registration)
        .await
        .is_err());
}

#[tokio::test]
async fn unreadable_existing_material_is_not_first_run() {
    let dir = TestDirectory::new();
    let config = dir.config();
    // A directory reliably fails file reads even under elevated test runners.
    fs::create_dir(config.device_registration_path()).unwrap();
    let error = AppState::new(config.clone())
        .await
        .err()
        .expect("unreadable");
    assert!(error
        .to_string()
        .contains("failed to read Device registration"));
    assert!(config.device_registration_path().is_dir());
    assert!(!dir.0.join("state.db").exists());
}

#[cfg(unix)]
#[test]
fn dangling_registration_symlink_refuses_instead_of_minting_another_device() {
    let dir = TestDirectory::new();
    let path = dir.0.join("registration.json");
    let missing = dir.0.join("missing.json");
    std::os::unix::fs::symlink(&missing, &path).unwrap();
    assert!(load_or_register(&path).is_err());
    assert!(!missing.exists());
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
}

#[test]
fn losing_registration_material_mints_a_new_device_and_failed_creation_is_an_error() {
    let dir = TestDirectory::new();
    let first = load_or_register(&dir.0.join("first.json")).unwrap();
    let replacement = load_or_register(&dir.0.join("replacement.json")).unwrap();
    assert_ne!(first.device_id, replacement.device_id);
    assert_ne!(
        first.registered_identity_id,
        replacement.registered_identity_id
    );
    assert!(load_or_register(&dir.0.join("missing-parent/registration.json")).is_err());
}

#[tokio::test]
async fn supplied_and_ephemeral_in_memory_registration_never_need_a_file() {
    let dir = TestDirectory::new();
    let config = dir
        .config()
        .with_device_registration_path(dir.0.join("missing-parent/registration.json"));
    let registration = new_registration();
    let state = AppState::in_memory_with_registration(config.clone(), registration.clone())
        .await
        .unwrap();
    assert_eq!(state.inner().device_registration, registration);
    assert!(AppState::in_memory(config.clone()).await.is_ok());
    assert!(!config.device_registration_path().exists());
    assert!(!dir.0.join("state.db").exists());
}
