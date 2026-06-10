#[test]
fn projection_preview_scaffold_exists() {
    assert!(std::path::Path::new("src/api/projection.rs").exists());
}
