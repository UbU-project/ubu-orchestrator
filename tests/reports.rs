#[test]
fn reports_scaffold_exists() {
    assert!(std::path::Path::new("src/reports/risk_report.rs").exists());
}
