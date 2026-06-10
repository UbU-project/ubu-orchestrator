#[test]
fn bootstrap_scaffold_exists() {
    assert!(std::path::Path::new("src/api/bootstrap.rs").exists());
}
