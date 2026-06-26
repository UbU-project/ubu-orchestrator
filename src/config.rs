use std::env;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

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
    github_projection_export_mode: ProjectionExportMode,
    /// SQLite database path. Configure with `UBU_DB_PATH`; defaults to `ubu-orchestrator.db`.
    db_path: String,
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
            github_projection_export_mode: ProjectionExportMode::from_env_value(
                env::var("UBU_GITHUB_PROJECTION_EXPORT_MODE").ok(),
            ),
            db_path: env::var("UBU_DB_PATH").unwrap_or_else(|_| "ubu-orchestrator.db".to_owned()),
        }
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn developer_github_token(&self) -> Option<SecretToken> {
        self.developer_github_token.clone()
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
}
