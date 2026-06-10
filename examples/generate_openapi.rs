use std::fs;

use ubu_orchestrator::openapi::ApiDoc;
use utoipa::OpenApi;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(&ApiDoc::openapi())?;
    fs::write("openapi/openapi.generated.json", format!("{json}\n"))?;
    Ok(())
}
