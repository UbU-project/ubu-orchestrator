use std::fs;
use std::path::Path;

use ubu_orchestrator::openapi::ApiDoc;
use utoipa::OpenApi;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output_path = Path::new("openapi/openapi.generated.json");
    if !output_path.exists() {
        return Err(format!(
            "committed OpenAPI output path is missing: {}",
            output_path.display()
        )
        .into());
    }

    let json = serde_json::to_string_pretty(&ApiDoc::openapi())?;
    fs::write(output_path, format!("{json}\n"))?;
    Ok(())
}
