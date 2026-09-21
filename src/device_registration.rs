//! Operator-controlled Device continuity, deliberately independent of SQLite.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

use ubu_core::{
    DeviceId, DeviceKind, DeviceRegistration, ObjectType, SyncState, TrustState, UbuId,
    UbuTimestamp, ZoneId,
};

use crate::errors::StartupError;

/// The same registration constructor is available to isolated in-memory tests.
/// Creating it alone does not persist it or claim continuity with any old Device.
pub fn new_registration() -> DeviceRegistration {
    DeviceRegistration {
        device_id: DeviceId::generate(),
        label: "Local UbU operator enclave".to_owned(),
        kind: DeviceKind::OsUserProfile,
        registered_at: UbuTimestamp::now_utc(),
        registered_identity_id: UbuId::new(ObjectType::Identity),
        trust_state: TrustState::Registered,
        sync_state: SyncState::LocalOnly,
        zone_id: ZoneId::parse("local-workspace").expect("non-empty zone label"),
        capability_profile: ["admission".to_owned()].into_iter().collect(),
        compartment_access: Default::default(),
        last_seen_at: None,
    }
}

pub fn require_registered(registration: &DeviceRegistration) -> Result<(), StartupError> {
    registration
        .validate()
        .map_err(|e| StartupError(format!("invalid Device registration: {e}")))?;
    if !registration.may_originate_mutations() {
        return Err(StartupError(
            "Device registration is revoked; refusing to originate mutations".into(),
        ));
    }
    Ok(())
}

fn read_registration(path: &Path) -> Result<DeviceRegistration, StartupError> {
    let bytes = fs::read(path).map_err(|e| StartupError::registration_file(path, "read", e))?;
    let registration: DeviceRegistration = serde_json::from_slice(&bytes)
        .map_err(|e| StartupError::registration_file(path, "parse", e))?;
    require_registered(&registration)?;
    Ok(registration)
}

pub fn load_or_register(path: &Path) -> Result<DeviceRegistration, StartupError> {
    // symlink_metadata distinguishes absent material from a dangling symlink.
    // Read failures, invalid JSON, and revoked registrations never mint a Device.
    match fs::symlink_metadata(path) {
        Ok(_) => return read_registration(path),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(StartupError::registration_file(path, "inspect", e)),
    }
    let registration = new_registration();
    require_registered(&registration)?;
    let mut bytes = serde_json::to_vec_pretty(&registration)
        .map_err(|e| StartupError::registration_file(path, "serialize", e))?;
    bytes.push(b'\n');
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        // Another startup won the race: restore it, never replace its identity.
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return read_registration(path),
        Err(e) => return Err(StartupError::registration_file(path, "create", e)),
    };
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| StartupError::registration_file(path, "write", e))?;
    tracing::warn!(device_id = registration.device_id.as_str(), path = %path.display(),
        "NEW Device registered; preserve this registration file to retain Device continuity");
    Ok(registration)
}
