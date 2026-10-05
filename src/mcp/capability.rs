use super::CALL_META_KEY;
use crate::native::MAX_REQUEST_BYTES;
use meerkat_core::{ToolDispatchContext, WireCallOrigin};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, value::RawValue};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const MAX_PENDING_CALLS: usize = 1024;
const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
const CALL_TTL: Duration = Duration::from_secs(30);

pub(super) fn random_secret() -> io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| io::Error::other("Secure randomness unavailable"))?;
    let mut result = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok(result)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capability {
    token: String,
    connection_id: String,
}

struct Entry {
    // Narrow runtime identity only. Turn text, images and user metadata never
    // enter this transport registry or leave the trusted host process.
    context: ToolDispatchContext,
    connection_id: String,
    operation: String,
    arguments: Value,
    origin: WireCallOrigin,
    bytes: usize,
    expires: Instant,
    cancelled: CancellationToken,
}

#[derive(Default)]
struct RegistryState {
    entries: HashMap<String, Entry>,
    bytes: usize,
    closed: bool,
}

#[derive(Default)]
pub(super) struct CallRegistry {
    state: Mutex<RegistryState>,
    pub(super) shutdown: CancellationToken,
}

pub(super) struct CallClaim {
    pub(super) context: ToolDispatchContext,
    pub(super) cancelled: CancellationToken,
}

/// Opaque metadata plus a cancellation-safe lifetime guard. The native MCP
/// router must retain the guard through the actual request's completion.
pub struct PreparedCall {
    pub metadata: Map<String, Value>,
    pub lease: CallLease,
}

/// Dropping an unused call releases its one-use authority and memory budget.
pub struct CallLease {
    registry: Weak<CallRegistry>,
    token: String,
    cancelled: CancellationToken,
}

impl Drop for CallLease {
    fn drop(&mut self) {
        self.cancelled.cancel();
        if let Some(registry) = self.registry.upgrade() {
            if let Ok(mut state) = registry.state.lock() {
                if let Some(entry) = state.entries.remove(&self.token) {
                    state.bytes = state.bytes.saturating_sub(entry.bytes);
                }
            }
        }
    }
}

impl CallRegistry {
    pub(super) fn issue(
        self: &Arc<Self>,
        connection_id: &str,
        operation: &str,
        arguments: &RawValue,
        origin: &WireCallOrigin,
        context: &ToolDispatchContext,
    ) -> io::Result<PreparedCall> {
        let origin_session = context
            .origin_session_id()
            .cloned()
            .ok_or_else(|| io::Error::other("Botus runtime caller unavailable"))?;
        if !matches!(operation, "botus_read" | "botus_apply")
            || connection_id.is_empty()
            || connection_id.len() > 256
            || arguments.get().len() > MAX_REQUEST_BYTES
        {
            return Err(io::Error::other("Invalid Botus call binding"));
        }
        let arguments: Value = serde_json::from_str(arguments.get())
            .map_err(|_| io::Error::other("Invalid Botus arguments"))?;
        if !arguments.is_object() {
            return Err(io::Error::other("Botus arguments must be an object"));
        }
        let argument_bytes = serde_json::to_vec(&arguments)
            .map_err(io::Error::other)?
            .len();
        let origin_bytes = serde_json::to_vec(origin).map_err(io::Error::other)?.len();
        // Charge fixed entry storage and retained strings as well as payload.
        // The independent entry ceiling also bounds allocator/map overhead.
        let bytes = argument_bytes
            .saturating_add(origin_bytes)
            .saturating_add(connection_id.len())
            .saturating_add(operation.len())
            .saturating_add(64)
            .saturating_add(std::mem::size_of::<Entry>());
        let token = random_secret()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("Botus call registry unavailable"))?;
        let now = Instant::now();
        state.entries.retain(|_, entry| entry.expires > now);
        state.bytes = state.entries.values().map(|entry| entry.bytes).sum();
        if state.closed
            || state.entries.len() >= MAX_PENDING_CALLS
            || state.bytes.saturating_add(bytes) > MAX_PENDING_BYTES
        {
            return Err(io::Error::other("Botus call capacity unavailable"));
        }
        if state.entries.contains_key(&token) {
            return Err(io::Error::other("Botus call capability collision"));
        }
        let cancelled = CancellationToken::new();
        let entry = Entry {
            context: ToolDispatchContext::default()
                .with_runtime_identity(origin_session, context.interaction_lineage_id()),
            connection_id: connection_id.into(),
            operation: operation.into(),
            arguments,
            origin: *origin,
            bytes,
            expires: now + CALL_TTL,
            cancelled: cancelled.clone(),
        };
        state.bytes += bytes;
        state.entries.insert(token.clone(), entry);
        drop(state);
        let capability = serde_json::to_value(Capability {
            token: token.clone(),
            connection_id: connection_id.into(),
        })
        .map_err(io::Error::other)?;
        Ok(PreparedCall {
            metadata: Map::from_iter([(CALL_META_KEY.into(), capability)]),
            lease: CallLease {
                registry: Arc::downgrade(self),
                token,
                cancelled,
            },
        })
    }

    pub(super) fn consume(
        &self,
        metadata: Option<&Value>,
        origin: Option<&Value>,
        operation: &str,
        arguments: &Value,
    ) -> io::Result<CallClaim> {
        let capability: Capability =
            serde_json::from_value(metadata.cloned().ok_or_else(unavailable)?)
                .map_err(|_| unavailable())?;
        if capability.token.len() != 64 || capability.connection_id.len() > 256 {
            return Err(unavailable());
        }
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if state.closed {
            return Err(unavailable());
        }
        let entry = state
            .entries
            .remove(&capability.token)
            .ok_or_else(unavailable)?;
        state.bytes = state.bytes.saturating_sub(entry.bytes);
        // Decode the native codec after consuming the correlation. Its wire
        // value is bound to this call but never establishes caller authority.
        let origin: WireCallOrigin =
            serde_json::from_value(origin.cloned().ok_or_else(unavailable)?)
                .map_err(|_| unavailable())?;
        if entry.expires <= Instant::now()
            || entry.connection_id != capability.connection_id
            || entry.operation != operation
            || &entry.arguments != arguments
            || entry.origin != origin
        {
            return Err(unavailable());
        }
        Ok(CallClaim {
            context: entry.context,
            cancelled: entry.cancelled,
        })
    }

    pub(super) fn close(&self) {
        self.shutdown.cancel();
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            state.entries.clear();
            state.bytes = 0;
        }
    }
}

fn unavailable() -> io::Error {
    io::Error::other("Botus call authority unavailable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use meerkat_core::SessionId;
    use serde_json::json;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    impl CallRegistry {
        fn issue_unavailable(
            self: &Arc<Self>,
            connection: &str,
            operation: &str,
            arguments: &RawValue,
            context: &ToolDispatchContext,
        ) -> io::Result<PreparedCall> {
            self.issue(
                connection,
                operation,
                arguments,
                &WireCallOrigin::Unavailable,
                context,
            )
        }

        fn consume_unavailable(
            &self,
            metadata: Option<&Value>,
            operation: &str,
            arguments: &Value,
        ) -> io::Result<CallClaim> {
            self.consume(
                metadata,
                Some(&json!({"kind":"unavailable"})),
                operation,
                arguments,
            )
        }
    }

    fn context() -> ToolDispatchContext {
        ToolDispatchContext::default().with_runtime_identity(SessionId::new(), None)
    }

    #[test]
    fn native_origin_is_required_exact_and_does_not_supply_session_authority() -> TestResult {
        let registry = Arc::new(CallRegistry::default());
        let value = json!({"action":"identity"});
        let args = serde_json::value::to_raw_value(&value)?;
        for origin in [
            None,
            Some(json!({})),
            Some(json!({"kind":"admitted","requester":"claimed"})),
            Some(json!({"kind":"unavailable","requester":"claimed"})),
        ] {
            let prepared = registry.issue_unavailable("a", "botus_read", &args, &context())?;
            assert!(
                registry
                    .consume(
                        prepared.metadata.get(CALL_META_KEY),
                        origin.as_ref(),
                        "botus_read",
                        &value
                    )
                    .is_err()
            );
            // An invalid projection consumes the correlation too.
            assert!(
                registry
                    .consume_unavailable(prepared.metadata.get(CALL_META_KEY), "botus_read", &value)
                    .is_err()
            );
        }
        assert!(
            registry
                .issue_unavailable("a", "botus_read", &args, &ToolDispatchContext::default())
                .is_err()
        );
        assert!(
            registry
                .consume(
                    None,
                    Some(&json!({"kind":"unavailable"})),
                    "botus_read",
                    &value
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn one_use_and_exact_arguments_operation_connection() -> TestResult {
        let registry = Arc::new(CallRegistry::default());
        let args = serde_json::value::to_raw_value(&json!({"action":"identity"}))?;
        for mismatch in ["arguments", "operation", "connection"] {
            let mut prepared =
                registry.issue_unavailable("connection-a", "botus_read", &args, &context())?;
            let mut value = json!({"action":"identity"});
            let mut operation = "botus_read";
            match mismatch {
                "arguments" => value = json!({"action":"list"}),
                "operation" => operation = "botus_apply",
                _ => prepared.metadata[CALL_META_KEY]["connection_id"] = json!("connection-b"),
            }
            assert!(
                registry
                    .consume_unavailable(prepared.metadata.get(CALL_META_KEY), operation, &value)
                    .is_err()
            );
            assert!(
                registry
                    .consume_unavailable(
                        prepared.metadata.get(CALL_META_KEY),
                        "botus_read",
                        &json!({"action":"identity"})
                    )
                    .is_err()
            );
        }
        let caller = context();
        let prepared = registry.issue_unavailable("connection-a", "botus_read", &args, &caller)?;
        let claimed = registry.consume_unavailable(
            prepared.metadata.get(CALL_META_KEY),
            "botus_read",
            &json!({"action":"identity"}),
        )?;
        assert_eq!(
            claimed.context.origin_session_id(),
            caller.origin_session_id()
        );
        assert!(
            registry
                .consume_unavailable(
                    prepared.metadata.get(CALL_META_KEY),
                    "botus_read",
                    &json!({"action":"identity"})
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn expiry_drop_restart_missing_context_and_host_shutdown_fail_closed() -> TestResult {
        let registry = Arc::new(CallRegistry::default());
        let value = json!({"action":"identity"});
        let args = serde_json::value::to_raw_value(&value)?;
        assert!(
            registry
                .issue_unavailable(
                    "connection-a",
                    "botus_read",
                    &args,
                    &ToolDispatchContext::default()
                )
                .is_err()
        );
        assert!(
            registry
                .consume_unavailable(None, "botus_read", &value)
                .is_err()
        );
        let prepared =
            registry.issue_unavailable("connection-a", "botus_read", &args, &context())?;
        {
            let mut state = registry.state.lock().map_err(|_| unavailable())?;
            for entry in state.entries.values_mut() {
                entry.expires = Instant::now();
            }
        }
        assert!(
            registry
                .consume_unavailable(prepared.metadata.get(CALL_META_KEY), "botus_read", &value)
                .is_err()
        );
        let prepared =
            registry.issue_unavailable("connection-a", "botus_read", &args, &context())?;
        let metadata = prepared.metadata.clone();
        drop(prepared);
        assert!(
            registry
                .consume_unavailable(metadata.get(CALL_META_KEY), "botus_read", &value)
                .is_err()
        );
        let prepared =
            registry.issue_unavailable("connection-a", "botus_read", &args, &context())?;
        assert!(
            CallRegistry::default()
                .consume_unavailable(prepared.metadata.get(CALL_META_KEY), "botus_read", &value)
                .is_err()
        );
        registry.close();
        assert!(
            registry
                .consume_unavailable(prepared.metadata.get(CALL_META_KEY), "botus_read", &value)
                .is_err()
        );
        assert!(
            registry
                .issue_unavailable("connection-a", "botus_read", &args, &context())
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn consumed_lease_cancels_and_does_not_retain_turn_payload() -> TestResult {
        let registry = Arc::new(CallRegistry::default());
        let value = json!({"action":"identity"});
        let args = serde_json::value::to_raw_value(&value)?;
        let caller = context().with_turn_metadata(std::collections::BTreeMap::from([(
            "private-prompt".into(),
            json!("do not retain"),
        )]));
        let prepared = registry.issue_unavailable("connection-a", "botus_read", &args, &caller)?;
        let claim = registry.consume_unavailable(
            prepared.metadata.get(CALL_META_KEY),
            "botus_read",
            &value,
        )?;
        assert!(claim.context.turn_metadata("private-prompt").is_none());
        assert!(!claim.cancelled.is_cancelled());
        drop(prepared);
        assert!(claim.cancelled.is_cancelled());
        assert_eq!(registry.state.lock().map_err(|_| unavailable())?.bytes, 0);
        Ok(())
    }

    #[test]
    fn capacity_is_bounded_and_released_by_guards() -> TestResult {
        let registry = Arc::new(CallRegistry::default());
        let args = serde_json::value::to_raw_value(&json!({"action":"identity"}))?;
        let caller = context();
        let mut prepared = Vec::new();
        for _ in 0..MAX_PENDING_CALLS {
            prepared.push(registry.issue_unavailable("a", "botus_read", &args, &caller)?);
        }
        assert!(
            registry
                .issue_unavailable("a", "botus_read", &args, &caller)
                .is_err()
        );
        prepared.clear();
        assert_eq!(registry.state.lock().map_err(|_| unavailable())?.bytes, 0);
        let _next = registry.issue_unavailable("a", "botus_read", &args, &caller)?;
        // Exercise the byte ceiling independently of the entry ceiling.
        registry
            .state
            .lock()
            .map_err(|_| unavailable())?
            .entries
            .values_mut()
            .for_each(|entry| entry.bytes = MAX_PENDING_BYTES);
        assert!(
            registry
                .issue_unavailable("a", "botus_read", &args, &caller)
                .is_err()
        );
        Ok(())
    }
}
