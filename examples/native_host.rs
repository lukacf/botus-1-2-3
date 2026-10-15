//! Add Botus to a host which already chose storage, realm and identity policy.
//! This example checks composition, not a running host or restart acceptance.
use botus_1_2_3::native::{BotusToolFactory, MAX_TOOL_DEFINITION_BYTES, tool_definitions};
use meerkat_meta::unified_runtime::UnifiedRuntimeBuilder;
use std::sync::Arc;

/// Call before `builder.build().await` and before members can run.
pub fn enable_botus(builder: UnifiedRuntimeBuilder) -> UnifiedRuntimeBuilder {
    builder.register_tool_bundle_factory(
        "botus-1-2-3",
        BotusToolFactory::requirements(),
        Arc::new(BotusToolFactory::default()),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = serde_json::to_vec(&tool_definitions())?.len();
    if bytes > MAX_TOOL_DEFINITION_BYTES {
        return Err("Botus tool definitions exceed their context budget".into());
    }
    println!("Botus has two tool definitions, using {bytes} serialized bytes.");
    println!("Pass your configured host builder through enable_botus before build.");
    println!("Add rust_bundles = [\"botus-1-2-3\"] under the intended profile's tools table.");
    println!("No beige computer, independent database or extra identity required.");
    Ok(())
}
