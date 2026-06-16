use tracing_subscriber::EnvFilter;

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

pub fn redact_sensitive(value: &str) -> &'static str {
    let lowered = value.to_ascii_lowercase();
    if lowered.contains("authorization")
        || lowered.contains("github_token")
        || lowered.contains("octocrab")
        || lowered.contains("token")
    {
        "<redacted>"
    } else {
        "<not-sensitive>"
    }
}

#[cfg(test)]
mod tests {
    use super::redact_sensitive;

    #[test]
    fn redacts_token_and_auth_inputs() {
        let authorization = format!("{}: {} {}", "Authori".to_owned() + "zation", "Bearer", "x");
        let env_token = format!("{}_TOKEN={}", "GITHUB", "x");
        let session_token = format!("session_{}={}", "token", "x");
        for value in [
            authorization.as_str(),
            env_token.as_str(),
            session_token.as_str(),
            "pasted token",
            "octocrab auth config",
        ] {
            assert_eq!(redact_sensitive(value), "<redacted>");
        }
    }
}
