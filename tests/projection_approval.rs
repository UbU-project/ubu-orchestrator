#[test]
fn projection_approval_scaffold_exists() {
    assert!(std::path::Path::new("src/adapters/github_adapter.rs").exists());
}
