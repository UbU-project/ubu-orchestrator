#[test]
fn next_action_scaffold_exists() {
    assert!(std::path::Path::new("src/api/next_action.rs").exists());
}
