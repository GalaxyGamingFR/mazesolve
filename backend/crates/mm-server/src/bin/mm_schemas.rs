//! Writes `docs/schemas/*.json` and the generated fixtures in `fixtures/`.

use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let backend = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schemas = mm_server::export::write_schemas(&backend.join("docs/schemas"))?;
    for s in &schemas {
        println!("docs/schemas/{s}");
    }
    let fixtures = mm_server::export::write_fixtures(&backend.join("fixtures"))?;
    for f in &fixtures {
        println!("fixtures/{f}");
    }
    Ok(())
}
