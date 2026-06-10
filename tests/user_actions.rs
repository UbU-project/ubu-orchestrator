#[test]
fn user_actions_scaffold_exists() {
    assert!(std::path::Path::new("src/api/user_action.rs").exists());
}
