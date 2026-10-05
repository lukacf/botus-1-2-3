//! Preserve capability secrecy across rmcp's payload-printing Debug paths.
//! Extensions are process-local and opaque to Debug/Serialize. Restore the
//! capability only at the final HTTP serializer, never in rmcp's event queue.

use super::{CALL_META_KEY, ProtectedCapability};
use axum::http::{HeaderName, HeaderValue};
use futures::stream::BoxStream;
use rmcp::{
    model::{ClientJsonRpcMessage, ClientRequest},
    transport::streamable_http_client::{
        StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
    },
};
use sse_stream::{Error as SseError, Sse};
use std::{collections::HashMap, io, sync::Arc};
use tokio_util::{
    bytes::BytesMut,
    codec::{Decoder, LinesCodec},
};

pub(super) struct ProtectedInputCodec {
    lines: LinesCodec,
}

impl ProtectedInputCodec {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            lines: LinesCodec::new_with_max_length(limit),
        }
    }

    fn parse(line: String) -> io::Result<ClientJsonRpcMessage> {
        // Never include the source text or serde's path in diagnostic output.
        let mut value: serde_json::Value = serde_json::from_str(&line)
            .map_err(|_| io::Error::other("Invalid MCP JSON-RPC frame"))?;
        // Strip before typed dispatch so a reserved capability on ping, list,
        // or a notification cannot reach rmcp's payload-printing log paths.
        let capability = value
            .pointer_mut("/params/_meta")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|metadata| metadata.remove(CALL_META_KEY));
        let mut message: ClientJsonRpcMessage = serde_json::from_value(value)
            .map_err(|_| io::Error::other("Invalid MCP JSON-RPC frame"))?;
        if let ClientJsonRpcMessage::Request(message) = &mut message {
            if let ClientRequest::CallToolRequest(request) = &mut message.request {
                if let Some(capability) = capability {
                    request.extensions.insert(ProtectedCapability(capability));
                }
            }
        }
        Ok(message)
    }
}

impl Decoder for ProtectedInputCodec {
    type Item = ClientJsonRpcMessage;
    type Error = io::Error;

    fn decode(&mut self, buffer: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        loop {
            let Some(line) = self
                .lines
                .decode(buffer)
                .map_err(|_| io::Error::other("MCP frame exceeds byte limit or is invalid"))?
            else {
                return Ok(None);
            };
            if line.is_empty() {
                continue;
            }
            return Self::parse(line).map(Some);
        }
    }

    fn decode_eof(&mut self, buffer: &mut BytesMut) -> io::Result<Option<Self::Item>> {
        let Some(line) = self
            .lines
            .decode_eof(buffer)
            .map_err(|_| io::Error::other("MCP frame exceeds byte limit or is invalid"))?
        else {
            return Ok(None);
        };
        if line.is_empty() {
            return Ok(None);
        }
        Self::parse(line).map(Some)
    }
}

#[derive(Clone)]
pub(super) struct ProtectedHttpClient(pub(super) reqwest::Client);

impl StreamableHttpClient for ProtectedHttpClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        mut message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        if let ClientJsonRpcMessage::Request(message) = &mut message {
            if let ClientRequest::CallToolRequest(request) = &mut message.request {
                if let Some(capability) = request.extensions.get::<ProtectedCapability>() {
                    request
                        .params
                        .meta
                        .get_or_insert_default()
                        .0
                        .insert(CALL_META_KEY.into(), capability.0.clone());
                }
            }
        }
        // rmcp's reqwest adapter serializes immediately and does not log the
        // request. Its service and worker queues only saw opaque Extensions.
        self.0
            .post_message(uri, message, session_id, auth_header, custom_headers)
            .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.0
            .delete_session(uri, session_id, auth_header, custom_headers)
            .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.0
            .get_stream(uri, session_id, last_event_id, auth_header, custom_headers)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parsed_request_debug_cannot_render_the_live_capability()
    -> Result<(), Box<dyn std::error::Error>> {
        let token = "sensitive-test-capability";
        let mut codec = ProtectedInputCodec::new(4096);
        let mut buffer = BytesMut::from(format!("{}\n", json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"botus_read","arguments":{"action":"identity"},"_meta":{CALL_META_KEY:{"token":token,"connection_id":"c"},"io.meerkat/origin":{"test":"unchanged"}}}})).as_bytes());
        let message = codec.decode(&mut buffer)?.ok_or("Missing request")?;
        assert!(!format!("{message:?}").contains(token));
        let ClientJsonRpcMessage::Request(message) = message else {
            return Err("Expected request".into());
        };
        let ClientRequest::CallToolRequest(request) = message.request else {
            return Err("Expected tool call".into());
        };
        assert_eq!(
            request
                .extensions
                .get::<ProtectedCapability>()
                .ok_or("Missing parked capability")?
                .0["token"],
            token
        );
        assert_eq!(
            request.params.meta.ok_or("Missing metadata")?.0["io.meerkat/origin"],
            json!({"test":"unchanged"})
        );
        let mut malformed =
            BytesMut::from(format!("{{\"token\":\"{token}\" broken JSON\n").as_bytes());
        let error = codec
            .decode(&mut malformed)
            .err()
            .ok_or("Expected malformed error")?;
        assert!(!error.to_string().contains(token));
        Ok(())
    }

    #[test]
    fn reserved_capability_on_other_methods_is_dropped_before_logging()
    -> Result<(), Box<dyn std::error::Error>> {
        let token = "sensitive-wrong-method-capability";
        let metadata = json!({CALL_META_KEY:{"token":token,"connection_id":"c"}});
        let messages = [
            json!({"jsonrpc":"2.0","id":1,"method":"ping","params":{"_meta":metadata}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":metadata}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{"_meta":metadata}}),
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3,"_meta":metadata}}),
        ];
        for value in messages {
            let mut codec = ProtectedInputCodec::new(4096);
            let mut buffer = BytesMut::from(format!("{value}\n").as_bytes());
            let message = codec.decode(&mut buffer)?.ok_or("Missing message")?;
            assert!(!format!("{message:?}").contains(token));
            assert!(!serde_json::to_string(&message)?.contains(token));
            match message {
                ClientJsonRpcMessage::Request(message) => {
                    use rmcp::model::GetExtensions;
                    assert!(
                        message
                            .request
                            .extensions()
                            .get::<ProtectedCapability>()
                            .is_none()
                    );
                }
                ClientJsonRpcMessage::Notification(message) => {
                    use rmcp::model::GetExtensions;
                    assert!(
                        message
                            .notification
                            .extensions()
                            .get::<ProtectedCapability>()
                            .is_none()
                    );
                }
                _ => return Err("Expected request or notification".into()),
            }
        }
        Ok(())
    }
}
