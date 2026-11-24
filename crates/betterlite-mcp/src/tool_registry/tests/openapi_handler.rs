use std::collections::BTreeMap;
use std::sync::Arc;

use crate::openapi_tools::output::OpenApiToolOutput;
use crate::openapi_tools::request_headers::OpenApiCallContext;
use crate::openapi_tools::tool::OpenApiToolError;
use crate::tool_registry::handler::{ToolHandler as _, ToolHandlerError};
use crate::tool_registry::openapi_handler::OpenApiToolHandler;

use super::support::{FakeTransport, object, openapi_tool};

#[tokio::test]
async fn calls_the_tool_with_the_arguments_and_the_context() {
    let transport = FakeTransport::replying(200, r#"{"ok": true}"#);
    let handler = OpenApiToolHandler {
        tool: openapi_tool(false),
        transport: Arc::new(transport.clone()),
    };
    let arguments = object(r#"{"id": "7"}"#);
    let context = OpenApiCallContext {
        auth_header: None,
        extra_headers: BTreeMap::from([("x-extra".to_owned(), "value".to_owned())]),
        resolved_auth_headers: BTreeMap::new(),
    };
    let output = handler.call(&arguments, &context).await.unwrap();
    assert!(matches!(output, OpenApiToolOutput::Json { .. }));
    let request = transport.only_request();
    assert_eq!(request.url, "http://upstream.test/items/7");
    assert_eq!(
        request.headers.get("x-extra").map(String::as_str),
        Some("value")
    );
}

#[tokio::test]
async fn keeps_the_upstream_auth_failure_with_its_type() {
    let transport = FakeTransport::replying(401, "").with_challenge();
    let handler = OpenApiToolHandler {
        tool: openapi_tool(true),
        transport: Arc::new(transport),
    };
    let arguments = object("{}");
    let outcome = handler
        .call(&arguments, &OpenApiCallContext::default())
        .await;
    let Err(ToolHandlerError::OpenApi(OpenApiToolError::UpstreamAuth {
        status,
        www_authenticate,
        ..
    })) = outcome
    else {
        panic!("expected the upstream auth failure to keep its type");
    };
    assert_eq!(status, 401);
    assert_eq!(www_authenticate.as_deref(), Some("Bearer"));
}

#[tokio::test]
async fn keeps_other_upstream_failures_as_open_api_errors() {
    let transport = FakeTransport::replying(500, "");
    let handler = OpenApiToolHandler {
        tool: openapi_tool(false),
        transport: Arc::new(transport),
    };
    let arguments = object("{}");
    let outcome = handler
        .call(&arguments, &OpenApiCallContext::default())
        .await;
    let Err(ToolHandlerError::OpenApi(OpenApiToolError::UpstreamStatus { status, .. })) = outcome
    else {
        panic!("expected the upstream status failure");
    };
    assert_eq!(status, 500);
}
