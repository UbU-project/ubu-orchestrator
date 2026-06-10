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
        }
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    pub fn developer_github_token(&self) -> Option<SecretToken> {
        self.developer_github_token.clone()
    }
}
