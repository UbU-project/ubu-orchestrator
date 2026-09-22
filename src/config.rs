use std::env;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct SecretToken(String);

impl SecretToken {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            None
        } else {
            Some(Self(value))
        }
    }

    pub fn expose_for_adapter(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    bind_addr: SocketAddr,
    developer_github_token: Option<SecretToken>,
    github_ingest_mode: GithubIngestMode,
    github_projection_export_mode: ProjectionExportMode,
    /// SQLite database path. Configure with `UBU_DB_PATH`; defaults to `ubu-orchestrator.db`.
    db_path: String,
    /// Operator-owned registration file, outside mutable database state.
    device_registration_path: Option<PathBuf>,
    category_palette_path: Option<PathBuf>,
    planning_horizon_seconds: Option<String>,
    planner_strategy: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlannerStrategyChoice {
    #[default]
    Chunked,
    Greedy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubIngestMode {
    Mock,
    Live,
}

impl GithubIngestMode {
    fn from_env_value(value: Option<String>) -> Self {
        match value.as_deref() {
            Some("live") => Self::Live,
            _ => Self::Mock,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionExportMode {
    Mock,
    Live,
}

impl ProjectionExportMode {
    fn from_env_value(value: Option<String>) -> Self {
        match value.as_deref() {
            Some("live") => Self::Live,
            _ => Self::Mock,
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let port = env::var("UBU_ORCHESTRATOR_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(7878);

        Self {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
            developer_github_token: env::var("GITHUB_TOKEN").ok().and_then(SecretToken::new),
            github_ingest_mode: GithubIngestMode::from_env_value(
                env::var("UBU_GITHUB_INGEST_MODE").ok(),
            ),
            github_projection_export_mode: ProjectionExportMode::from_env_value(
                env::var("UBU_GITHUB_PROJECTION_EXPORT_MODE").ok(),
            ),
            db_path: env::var("UBU_DB_PATH").unwrap_or_else(|_| "ubu-orchestrator.db".to_owned()),
            device_registration_path: env::var_os("UBU_DEVICE_REGISTRATION").map(PathBuf::from),
            category_palette_path: env::var_os("UBU_CATEGORY_PALETTE_PATH").map(PathBuf::from),
            planner_strategy: env::var_os("UBU_PLANNER_STRATEGY")
                .map(|value| value.to_string_lossy().into_owned()),
            planning_horizon_seconds: env::var_os("UBU_PLANNING_HORIZON_SECONDS")
                .map(|value| value.to_string_lossy().into_owned()),
        }
    }

    pub fn planning_horizon_seconds(&self) -> Result<u64, crate::errors::StartupError> {
        let Some(value) = &self.planning_horizon_seconds else {
            return Ok(86400);
        };
        value.parse::<u64>().ok().filter(|span| (1..=2678400).contains(span)
            && value.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| crate::errors::StartupError(format!(
                "invalid UBU_PLANNING_HORIZON_SECONDS `{value}`: expected an integer from 1 to 2678400")))
    }

    pub fn planner_strategy(&self) -> Result<PlannerStrategyChoice, crate::errors::StartupError> {
        match self.planner_strategy.as_deref() {
            None | Some("chunked") => Ok(PlannerStrategyChoice::Chunked),
            Some("greedy") => Ok(PlannerStrategyChoice::Greedy),
            Some(value) => Err(crate::errors::StartupError(format!(
                "invalid UBU_PLANNER_STRATEGY `{value}`: expected chunked or greedy"
            ))),
        }
    }

    pub fn with_planner_strategy(mut self, raw: impl Into<String>) -> Self {
        self.planner_strategy = Some(raw.into());
        self
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn developer_github_token(&self) -> Option<SecretToken> {
        self.developer_github_token.clone()
    }

    pub fn github_ingest_mode(&self) -> GithubIngestMode {
        self.github_ingest_mode
    }

    pub fn with_github_ingest_mode(mut self, mode: GithubIngestMode) -> Self {
        self.github_ingest_mode = mode;
        self
    }

    pub fn github_projection_export_mode(&self) -> ProjectionExportMode {
        self.github_projection_export_mode
    }

    pub fn with_github_projection_export_mode(mut self, mode: ProjectionExportMode) -> Self {
        self.github_projection_export_mode = mode;
        self
    }

    pub fn db_path(&self) -> &str {
        &self.db_path
    }
    pub fn with_db_path(mut self, path: impl Into<String>) -> Self {
        self.db_path = path.into();
        self
    }

    pub fn with_device_registration_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.device_registration_path = Some(path.into());
        self
    }

    pub fn with_category_palette_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.category_palette_path = Some(path.into());
        self
    }

    pub fn category_palette_path(&self) -> Option<&Path> {
        self.category_palette_path.as_deref()
    }

    pub fn device_registration_path(&self) -> PathBuf {
        self.device_registration_path.clone().unwrap_or_else(|| {
            // Accept the plain DB path and the usual SQLite URL spellings.
            let path = self
                .db_path
                .strip_prefix("sqlite://")
                .or_else(|| self.db_path.strip_prefix("sqlite:"))
                .unwrap_or(&self.db_path);
            let path = path.split('?').next().unwrap_or(path);
            Path::new(path)
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join("ubu-device-registration.json")
        })
    }
}

#[cfg(test)]
mod registration_path_tests {
    use super::*;

    #[test]
    fn registration_defaults_beside_database_and_explicit_path_wins() {
        let mut config = ServerConfig::from_env();
        // Keep this test independent of the test runner's environment.
        config.device_registration_path = None;
        for (database, expected) in [
            ("ubu-orchestrator.db", "ubu-device-registration.json"),
            ("/tmp/ubu/state.db", "/tmp/ubu/ubu-device-registration.json"),
            (
                "sqlite:///tmp/ubu/state.db?mode=rwc",
                "/tmp/ubu/ubu-device-registration.json",
            ),
            (
                "sqlite:/tmp/ubu/state.db",
                "/tmp/ubu/ubu-device-registration.json",
            ),
        ] {
            assert_eq!(
                config
                    .clone()
                    .with_db_path(database)
                    .device_registration_path(),
                PathBuf::from(expected)
            );
        }
        assert_eq!(
            config
                .with_db_path("/tmp/different/state.db")
                .with_device_registration_path("operator/device.json")
                .device_registration_path(),
            PathBuf::from("operator/device.json")
        );
    }
}
