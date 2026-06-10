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
