//! Compose Botus with a host that already owns durable storage and authority.
//! The returned listener must live at least as long as the runtime.
use botus_1_2_3::{
    mcp::{BotusMcpContextProvider, BotusMcpHost, BotusMcpListener},
    native::BotusToolFactory,
};
use meerkat_core::McpServerConfig;
use meerkat_mobkit::unified_runtime::UnifiedRuntimeBuilder;
use std::sync::Arc;

/// Call before building the runtime. Install the returned exact configuration
/// and provider in the native MCP router used by each agent factory. Select the
/// MCP server in agent profiles; selecting the native bundle too duplicates it.
pub async fn prepare_botus(
    builder: UnifiedRuntimeBuilder,
    relay_binary: &str,
) -> Result<
    (
        UnifiedRuntimeBuilder,
        BotusMcpListener,
        McpServerConfig,
        BotusMcpContextProvider,
    ),
    Box<dyn std::error::Error>,
> {
    let host = Arc::new(BotusMcpHost::default());
    let listener = host.start().await?;
    let config = listener.relay_config(relay_binary);
    let provider = listener.call_context_provider(config.clone())?;
    let builder =
        builder.register_tool_bundle_factory("botus-1-2-3", BotusToolFactory::requirements(), host);
    Ok((builder, listener, config, provider))
}

fn main() {
    println!("prepare_botus composes the package with your configured MobKit host.");
    println!(
        "Keep its listener alive, wire its exact MCP destination and call-context provider, then build the runtime."
    );
    println!("The filing cabinet is still MobKit's. F9 has merely discovered TCP/IP.");
}
