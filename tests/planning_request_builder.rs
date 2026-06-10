#[test]
fn planning_request_builder_scaffold_exists() {
    assert!(std::path::Path::new("src/services/planning_service.rs").exists());
}
