use std::collections::BTreeMap;
use std::fmt::Debug;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::mcp_tools::{McpToolEntry, decode_mcp_tools};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::arguments::json::Object;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use tracing::field::{Field as EventField, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use crate::openapi_tools::document::Operation;
use crate::openapi_tools::output::OpenApiToolOutput;
use crate::openapi_tools::request_headers::OpenApiCallContext;
use crate::openapi_tools::tool::{OpenApiTool, OpenApiToolConfig};
use crate::openapi_tools::transport::{OpenApiRequest, OpenApiTransport, OpenApiTransportError};
use crate::tool_registry::handler::{ToolHandler, ToolHandlerError};
use crate::tool_registry::loader::{HandlerLoadError, ToolHandlerLoader};
use crate::tool_registry::registry::ToolRegistry;

pub(super) struct Silent;

impl ToolHandler for Silent {
    fn call<'call>(
        &'call self,
        _arguments: &'call Object,
        _context: &'call OpenApiCallContext,
    ) -> Pin<Box<dyn Future<Output = Result<OpenApiToolOutput, ToolHandlerError>> + Send + 'call>>
    {
        Box::pin(async { Ok(OpenApiToolOutput::Text(String::new())) })
    }
}

pub(super) fn silent() -> Arc<dyn ToolHandler> {
    Arc::new(Silent)
}

pub(super) fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

pub(super) fn entries(yaml: &str) -> Vec<McpToolEntry> {
    decode_mcp_tools(yaml.as_bytes()).unwrap().entries
}

pub(super) fn registry(names: &[&str]) -> ToolRegistry {
    let registry = ToolRegistry::default();
    for name in names {
        registry.register_tool(name, "d", Object::new(), silent());
    }
    registry
}

pub(super) fn names(registry: &ToolRegistry) -> Vec<String> {
    registry
        .list_tools(None)
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

pub(super) struct RecordingLoader {
    known: Vec<String>,
    failure: Option<String>,
    requests: Mutex<Vec<(String, Option<PathBuf>)>>,
}

impl RecordingLoader {
    pub(super) fn knowing(known: &[&str]) -> Self {
        Self {
            known: known.iter().map(|name| (*name).to_owned()).collect(),
            failure: None,
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn failing_on(known: &[&str], failure: &str) -> Self {
        Self {
            known: known.iter().map(|name| (*name).to_owned()).collect(),
            failure: Some(failure.to_owned()),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<(String, Option<PathBuf>)> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ToolHandlerLoader for RecordingLoader {
    fn load(
        &self,
        handler: &str,
        config_file_path: Option<&Path>,
    ) -> Result<Option<Arc<dyn ToolHandler>>, HandlerLoadError> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((handler.to_owned(), config_file_path.map(Path::to_path_buf)));
        if self.failure.as_deref() == Some(handler) {
            return Err(HandlerLoadError {
                handler: handler.to_owned(),
            });
        }
        if self.known.iter().any(|known| known.as_str() == handler) {
            return Ok(Some(Arc::new(Silent)));
        }
        Ok(None)
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<(Level, String)>>>);

impl Logs {
    pub(super) fn messages_at(&self, level: Level) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|(found, _)| *found == level)
            .map(|(_, message)| message.clone())
            .collect()
    }
}

#[derive(Default)]
struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &EventField, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &EventField, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        let mut field = MessageField::default();
        event.record(&mut field);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((*event.metadata().level(), field.0));
    }
}

pub(super) fn capture_logs() -> (Logs, DefaultGuard) {
    let logs = Logs::default();
    let guard = set_default(tracing_subscriber::registry().with(logs.clone()));
    (logs, guard)
}

#[derive(Clone)]
pub(super) struct FakeTransport {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    requests: Arc<Mutex<Vec<OpenApiRequest>>>,
}

impl FakeTransport {
    pub(super) fn replying(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: body.as_bytes().to_vec(),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn with_challenge(mut self) -> Self {
        self.headers.insert(
            HeaderName::from_static("www-authenticate"),
            HeaderValue::from_static("Bearer"),
        );
        self
    }

    pub(super) fn only_request(&self) -> OpenApiRequest {
        let mut requests = self.requests.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(requests.len(), 1);
        requests.remove(0)
    }
}

impl OpenApiTransport for FakeTransport {
    fn send<'call>(
        &'call self,
        request: OpenApiRequest,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamResponse, OpenApiTransportError>> + Send + 'call>>
    {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let response = UpstreamResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
        };
        Box::pin(async move { Ok(response) })
    }
}

pub(super) fn openapi_tool(relays_upstream_auth: bool) -> OpenApiTool {
    let operation: Operation = serde_json::from_str(
        r#"{"operationId": "getItem", "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}]}"#,
    )
    .unwrap();
    OpenApiTool::new(OpenApiToolConfig {
        path: "/items/{id}".to_owned(),
        method: "get".to_owned(),
        operation,
        base_url: "http://upstream.test".to_owned(),
        headers: BTreeMap::new(),
        server_label: None,
        relays_upstream_auth,
        auth_type: None,
        upstream_token_header: None,
    })
}
