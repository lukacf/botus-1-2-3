//! MCP stdio launcher. Protocol output alone belongs on stdout.

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 || args[0] != "mcp" {
        return Err("Usage: botus-1-2-3 mcp (requires host-provided BOTUS_MCP_ENDPOINT and BOTUS_MCP_RELAY_TOKEN)".into());
    }
    botus_1_2_3::mcp::serve_stdio_from_env().await
}
