//! Compose Botus with a host that already owns durable storage and authority.
//! The returned listener must live at least as long as the runtime.
use botus_1_2_3::mcp::{BotusMcpHost, BotusMcpListener};
use meerkat_core::McpServerConfig;
use meerkat_mobkit::{
    extension_state::ChildMcpAvailability, unified_runtime::UnifiedRuntimeBuilder,
};
use std::sync::Arc;

/// Register the service before building the runtime. Select the returned
/// public descriptor in agent profiles; it contains no process credential.
/// Ordinary children receive tool availability, while workbook grants remain
/// private by default. Forks still use MobKit's proven access lineage.
pub async fn prepare_botus(
    builder: UnifiedRuntimeBuilder,
    relay_binary: &str,
) -> Result<(UnifiedRuntimeBuilder, BotusMcpListener, McpServerConfig), Box<dyn std::error::Error>>
{
    let host = Arc::new(BotusMcpHost::default());
    let listener = host.start().await?;
    let binding = listener.document_binding(relay_binary, ChildMcpAvailability::ChildAvailable)?;
    let public_descriptor = binding.public_config().clone();
    let builder = builder.register_mcp_tool_bundle_factory(binding);
    Ok((builder, listener, public_descriptor))
}

fn main() {
    println!("prepare_botus registers the service with your configured MobKit host.");
    println!("Select its public MCP descriptor in profiles and keep the listener alive.");
    println!("The filing cabinet is still MobKit's. F9 has merely discovered TCP/IP.");
}
