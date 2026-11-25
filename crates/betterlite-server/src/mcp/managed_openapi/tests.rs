#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_mcp::client::tools::result::{CallToolResult, ContentBlock};
use betterlite_mcp::openapi_tools::output::OpenApiToolOutput;
use betterlite_mcp::openapi_tools::request_headers::OpenApiCallContext;
use betterlite_mcp::openapi_tools::tool::OpenApiToolError;
use betterlite_mcp::server_record::MCPServer;
use betterlite_mcp::server_utils::prefixes::PrefixPolicy;
use betterlite_mcp::tool_registry::handler::{ToolHandler, ToolHandlerError};
use betterlite_mcp::tool_registry::registry::ToolRegistry;
use betterlite_mcp::wire_compat::tool_outcome::WireCompat;
use betterlite_prompts::arguments::json::{Json, Object};

use super::{ManagedOpenApiCall, ManagedOpenApiError, call};

type HandlerResult = Result<OpenApiToolOutput, ToolHandlerError>;
type HandlerFuture<'call> = Pin<Box<dyn Future<Output = HandlerResult> + Send + 'call>>;

enum Reply {
    Text(String),
    Json {
        value: Json,
        original_text: String,
    },
    Recording {
        text: String,
        seen: Arc<Mutex<Vec<(Object, OpenApiCallContext)>>>,
    },
    RegistryWrite {
        registry: Arc<ToolRegistry>,
    },
    UpstreamAuth,
    Failure(String),
}

struct StubHandler(Reply);

impl ToolHandler for StubHandler {
    fn call<'call>(
        &'call self,
        arguments: &'call Object,
        context: &'call OpenApiCallContext,
    ) -> HandlerFuture<'call> {
        let outcome = match &self.0 {
            Reply::Text(text) => Ok(OpenApiToolOutput::Text(text.clone())),
            Reply::Json {
                value,
                original_text,
            } => Ok(OpenApiToolOutput::Json {
                value: value.clone(),
                original_text: original_text.clone(),
            }),
            Reply::Recording { text, seen } => {
                seen.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((arguments.clone(), context.clone()));
                Ok(OpenApiToolOutput::Text(text.clone()))
            }
            Reply::RegistryWrite { registry } => {
                registry.register_tool(
                    "srv-get_pets",
                    "replacement",
                    Object::new(),
                    Arc::new(Self(Reply::Text("replaced".to_owned()))),
                );
                Ok(OpenApiToolOutput::Text("written".to_owned()))
            }
            Reply::UpstreamAuth => Err(ToolHandlerError::OpenApi(OpenApiToolError::UpstreamAuth {
                status: 401,
                www_authenticate: Some("Bearer realm=\"pets\"".to_owned()),
                server: "pets".to_owned(),
            })),
            Reply::Failure(message) => Err(ToolHandlerError::Failed(message.clone())),
        };
        Box::pin(async move { outcome })
    }
}

fn mcp_server(json: &str) -> MCPServer {
    serde_json::from_str(json).unwrap()
}

fn plain_server() -> MCPServer {
    mcp_server(r#"{"server_id": "srv", "name": "Pets", "transport": "http"}"#)
}

fn object(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn parse_json(json: &str) -> Json {
    serde_json::from_str(json).unwrap()
}

fn headers(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn registry_with(handler: Arc<dyn ToolHandler>) -> ToolRegistry {
    let registry = ToolRegistry::default();
    registry.register_tool("srv-get_pets", "d", Object::new(), handler);
    registry
}

const fn record<'call>(
    server: &'call MCPServer,
    tool_name: &'call str,
    arguments: &'call Object,
    context: &'call OpenApiCallContext,
    prefix_policy: &'call PrefixPolicy,
) -> ManagedOpenApiCall<'call> {
    ManagedOpenApiCall {
        server,
        tool_name,
        arguments,
        context,
        prefix_policy,
    }
}

fn text_of(result: &CallToolResult) -> String {
    let [ContentBlock::Text(content)] = result.content.as_slice() else {
        panic!("expected one text block");
    };
    content.text.clone()
}

#[tokio::test]
async fn alias_key() {
    let server = mcp_server(
        r#"{"server_id": "srv", "name": "Pets", "transport": "http", "alias": "Pet Alias"}"#,
    );
    let registry = ToolRegistry::default();
    registry.register_tool(
        "Pet_Alias-get_pets",
        "d",
        Object::new(),
        Arc::new(StubHandler(Reply::Text("alias tool".to_owned()))),
    );
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert_eq!(text_of(&result), "alias tool");
}

#[tokio::test]
async fn short_prefix_key() {
    let server = mcp_server(
        r#"{"server_id": "srv", "name": "Pets", "transport": "http", "short_prefix": "abc"}"#,
    );
    let registry = ToolRegistry::default();
    registry.register_tool(
        "abc.get_pets",
        "d",
        Object::new(),
        Arc::new(StubHandler(Reply::Text("short tool".to_owned()))),
    );
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy {
        separator: ".".to_owned(),
        short_prefix_enabled: true,
    };
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert_eq!(text_of(&result), "short tool");
}

#[tokio::test]
async fn arguments_and_context() {
    let server = plain_server();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let registry = registry_with(Arc::new(StubHandler(Reply::Recording {
        text: "recorded".to_owned(),
        seen: Arc::clone(&seen),
    })));
    let arguments = object(r#"{"pet_id": 7}"#);
    let context = OpenApiCallContext {
        auth_header: Some("Bearer caller-token".to_owned()),
        extra_headers: headers(&[("x-extra", "forwarded")]),
        resolved_auth_headers: headers(&[("x-resolved", "credential")]),
    };
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert_eq!(text_of(&result), "recorded");
    let recorded = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let (recorded_arguments, recorded_context) = recorded.first().unwrap();
    assert_eq!(recorded_arguments, &arguments);
    assert_eq!(recorded_context.auth_header, context.auth_header);
    assert_eq!(recorded_context.extra_headers, context.extra_headers);
    assert_eq!(
        recorded_context.resolved_auth_headers,
        context.resolved_auth_headers
    );
}

#[tokio::test]
async fn missing_registration() {
    let registry = ToolRegistry::default();
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert_eq!(
        text_of(&result),
        "OpenAPI tool get_pets not found in registry"
    );
}

#[tokio::test]
async fn text_result() {
    let registry = registry_with(Arc::new(StubHandler(Reply::Text(
        "upstream text".to_owned(),
    ))));
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.structured_content, None);
    assert_eq!(text_of(&result), "upstream text");
}

#[tokio::test]
async fn object_result() {
    let value = parse_json(r#"{"ok": true}"#);
    let registry = registry_with(Arc::new(StubHandler(Reply::Json {
        value: value.clone(),
        original_text: r#"{"ok": true}"#.to_owned(),
    })));
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.structured_content, Some(value));
    assert_eq!(text_of(&result), r#"{"ok": true}"#);
}

#[tokio::test]
async fn legacy_array() {
    let registry = registry_with(Arc::new(StubHandler(Reply::Json {
        value: parse_json("[1, 2]"),
        original_text: "[1, 2]".to_owned(),
    })));
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.structured_content, None);
    assert_eq!(text_of(&result), "[1, 2]");
}

#[tokio::test]
async fn modern_array() {
    let value = parse_json("[1, 2]");
    let registry = registry_with(Arc::new(StubHandler(Reply::Json {
        value: value.clone(),
        original_text: "[1, 2]".to_owned(),
    })));
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Modern,
    )
    .await
    .unwrap();
    assert!(!result.is_error);
    assert_eq!(result.structured_content, Some(value));
    assert_eq!(text_of(&result), "[1, 2]");
}

#[tokio::test]
async fn upstream_auth_preserved() {
    let registry = registry_with(Arc::new(StubHandler(Reply::UpstreamAuth)));
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let outcome = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await;
    let Err(ManagedOpenApiError::UpstreamAuth {
        status,
        www_authenticate,
        server: challenged,
    }) = outcome
    else {
        panic!("expected the upstream challenge to propagate");
    };
    assert_eq!(status, 401);
    assert_eq!(www_authenticate.as_deref(), Some("Bearer realm=\"pets\""));
    assert_eq!(challenged, "pets");
}

#[tokio::test]
async fn ordinary_failure_result() {
    let registry = registry_with(Arc::new(StubHandler(Reply::Failure(
        "upstream refused".to_owned(),
    ))));
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert!(result.is_error);
    assert_eq!(
        text_of(&result),
        "Error calling OpenAPI tool get_pets: upstream refused"
    );
}

#[tokio::test]
async fn registry_lock_released() {
    let registry = Arc::new(ToolRegistry::default());
    registry.register_tool(
        "srv-get_pets",
        "d",
        Object::new(),
        Arc::new(StubHandler(Reply::RegistryWrite {
            registry: Arc::clone(&registry),
        })),
    );
    let server = plain_server();
    let arguments = Object::new();
    let context = OpenApiCallContext::default();
    let policy = PrefixPolicy::default();
    let result = call(
        &registry,
        record(&server, "get_pets", &arguments, &context, &policy),
        WireCompat::Legacy,
    )
    .await
    .unwrap();
    assert_eq!(text_of(&result), "written");
}
