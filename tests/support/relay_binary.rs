//! Select the installed relay for artifact acceptance, or Cargo's local build.

pub fn relay_binary() -> Result<String, Box<dyn std::error::Error>> {
    let command = match std::env::var("BOTUS_MCP_TEST_BINARY") {
        Ok(command) => command,
        Err(std::env::VarError::NotPresent) => env!("CARGO_BIN_EXE_botus-1-2-3").to_owned(),
        Err(error) => return Err(error.into()),
    };
    let path = std::path::Path::new(&command);
    if !path.is_absolute() || !path.is_file() {
        return Err("MCP acceptance requires an existing absolute relay path".into());
    }
    Ok(command)
}
