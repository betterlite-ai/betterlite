use std::sync::Arc;
use std::sync::atomic::Ordering;

use rmcp::model::{ClientCapabilities, ProtocolVersion as SdkProtocolVersion};

use crate::client::sdk_runtime::convert::ServerCapabilitiesWire;
use crate::client::sdk_runtime::handler::ServerRequestHandlers;
use crate::client::sdk_runtime::session::SdkSession;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::SessionError;
use crate::client::sessions::negotiate::{LegacyVersion, ProtocolVersion};
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::transport::TransportType;
use crate::client::tools::calling::{CallToolOptions, call_tool};
use crate::client::tools::listing::list_tools;
use crate::client::tools::result::{ContentBlock, ToolCallResult};
use crate::tools::pagination::McpSession;
use crate::tools::tool_calls::CallRequest;

use super::server::{
    FailingSampling, INSTRUCTIONS, TestElicitation, TestSampling, TestServer, serve_http,
};
use super::support::{self, RecordingHost};

const RAISE: CallToolOptions = CallToolOptions {
    raise_on_error: true,
    allow_input_required: false,
};

fn call(name: &str) -> CallRequest {
    let owned = name.to_owned();
    let arguments = None;
    CallRequest {
        name: owned,
        arguments,
    }
}

fn text_of(result: &ToolCallResult) -> &str {
    let ToolCallResult::Complete(complete) = result else {
        panic!("expected a complete tool result");
    };
    match complete.content.as_slice() {
        [ContentBlock::Text(text)] => &text.text,
        other => panic!("unexpected content blocks: {other:?}"),
    }
}

#[tokio::test]
async fn a_streamable_http_session_lists_every_tool_page() {
    let server = serve_http(TestServer::standard()).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let tools = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .unwrap();
    let names: Vec<String> = tools.into_iter().map(|tool| tool.name).collect();
    assert_eq!(names, ["alpha", "beta", "gamma"]);
    assert_eq!(
        client.last_initialize_instructions.as_deref(),
        Some(INSTRUCTIONS)
    );
}

#[tokio::test]
async fn the_handshake_adopts_the_fallback_version_capabilities_and_instructions() {
    let server = serve_http(TestServer::legacy()).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let capabilities: Option<ServerCapabilitiesWire> = run_with_session(
        &mut client,
        async |session| Ok(session.server_capabilities.clone()),
        false,
    )
    .await
    .unwrap();
    let adopted = capabilities.expect("the server's capabilities were adopted");
    assert!(adopted.prompts.is_some());
    assert!(adopted.resources.is_some());
    assert_eq!(
        client.last_initialize_instructions.as_deref(),
        Some(INSTRUCTIONS)
    );
}

#[tokio::test]
async fn a_configured_version_goes_out_in_the_initialize_request() {
    let server = serve_http(TestServer::standard()).await;
    let mut settings = support::settings(TransportType::Http);
    settings.protocol_version = ProtocolVersion::Legacy(LegacyVersion::Jun2025);
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        settings,
        &server.url,
        support::CALL_TIMEOUT,
    );
    let tools = list_tools(&mut client, support::CALL_TIMEOUT, true)
        .await
        .unwrap();
    assert_eq!(tools.len(), 3);
    assert_eq!(
        client.last_initialize_instructions.as_deref(),
        Some(INSTRUCTIONS)
    );
}

#[tokio::test]
async fn a_tool_call_forwards_every_progress_update() {
    let server = serve_http(TestServer::standard()).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let host = RecordingHost::default();
    let result = Box::pin(call_tool(
        &mut client,
        &call("progress"),
        Some(&host),
        CallToolOptions::default(),
    ))
    .await
    .unwrap();
    assert_eq!(
        host.seen(),
        vec![(25.0_f64, Some(100.0_f64)), (75.0_f64, Some(100.0_f64))]
    );
    assert_eq!(text_of(&result), "progressed");
}

#[tokio::test]
async fn an_is_error_result_stays_a_complete_result() {
    let server = serve_http(TestServer::standard()).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let result = Box::pin(call_tool(
        &mut client,
        &call("fail"),
        None::<&RecordingHost>,
        CallToolOptions::default(),
    ))
    .await
    .unwrap();
    let ToolCallResult::Complete(complete) = &result else {
        panic!("expected a complete tool result");
    };
    assert!(complete.is_error);
    assert_eq!(text_of(&result), "the tool failed");
}

#[tokio::test]
async fn a_json_rpc_error_is_a_protocol_failure() {
    let server = serve_http(TestServer::standard()).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = Box::pin(call_tool(
        &mut client,
        &call("rpc-error"),
        None::<&RecordingHost>,
        RAISE,
    ))
    .await
    .unwrap_err();
    let SessionError::Protocol(failure) = &error else {
        panic!("expected a protocol failure, got {error}");
    };
    assert_eq!(failure.code, -32_602_i32);
    assert_eq!(failure.message, "bad arguments");
}

async fn answered(
    name: &str,
    handlers: ServerRequestHandlers,
) -> Result<String, SessionError<SdkError>> {
    let (result, _listings) =
        Box::pin(called_on(TestServer::standard(), name, RAISE, handlers)).await?;
    Ok(text_of(&result).to_owned())
}

async fn called_on(
    server: TestServer,
    name: &str,
    options: CallToolOptions,
    handlers: ServerRequestHandlers,
) -> Result<(ToolCallResult, usize), SessionError<SdkError>> {
    let listings = Arc::clone(&server.listings);
    let endpoint = serve_http(server).await;
    let mut settings = support::settings(TransportType::Http);
    settings.handlers = handlers;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        settings,
        &endpoint.url,
        support::CALL_TIMEOUT,
    );
    let result = Box::pin(call_tool(
        &mut client,
        &call(name),
        None::<&RecordingHost>,
        options,
    ))
    .await?;
    let count = listings.load(Ordering::SeqCst);
    Ok((result, count))
}

#[tokio::test]
async fn sampling_without_a_responder_answers_the_sdk_default() {
    let answer = Box::pin(answered("sample", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert_eq!(answer, "error -32600: Sampling not supported");
}

#[tokio::test]
async fn sampling_goes_to_the_configured_responder() {
    let handlers = ServerRequestHandlers {
        sampling: Some(Arc::new(TestSampling)),
        elicitation: None,
    };
    let answer = Box::pin(answered("sample", handlers)).await.unwrap();
    assert!(answer.contains("\"model\":\"test-model\""), "{answer}");
    assert!(answer.contains("1 message(s)"), "{answer}");
}

#[tokio::test]
async fn a_sampling_error_response_keeps_its_code_and_message() {
    let handlers = ServerRequestHandlers {
        sampling: Some(Arc::new(FailingSampling)),
        elicitation: None,
    };
    let answer = Box::pin(answered("sample", handlers)).await.unwrap();
    assert_eq!(answer, "error -32099: no model available");
}

#[tokio::test]
async fn a_sampling_request_the_responder_cannot_decode_is_invalid_params() {
    let handlers = ServerRequestHandlers {
        sampling: Some(Arc::new(TestSampling)),
        elicitation: None,
    };
    let answer = Box::pin(answered("sample-invalid", handlers))
        .await
        .unwrap();
    assert!(
        answer.starts_with("error -32602: invalid sampling request"),
        "{answer}"
    );
}

#[tokio::test]
async fn elicitation_without_a_responder_answers_the_sdk_default() {
    let answer = Box::pin(answered("elicit", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert_eq!(answer, "error -32600: Elicitation not supported");
}

#[tokio::test]
async fn elicitation_goes_to_the_configured_responder() {
    let handlers = ServerRequestHandlers {
        sampling: None,
        elicitation: Some(Arc::new(TestElicitation)),
    };
    let answer = Box::pin(answered("elicit", handlers)).await.unwrap();
    assert!(answer.contains("\"action\":\"accept\""), "{answer}");
    assert!(answer.contains("who are you?"), "{answer}");
}

#[tokio::test]
async fn a_ping_answers_an_empty_result() {
    let answer = Box::pin(answered("ping", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert_eq!(answer, "{}");
}

#[tokio::test]
async fn a_roots_request_answers_the_sdk_default() {
    let answer = Box::pin(answered("roots", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert_eq!(answer, "error -32600: List roots not supported");
}

#[tokio::test]
async fn the_client_capabilities_name_the_configured_callbacks_only() {
    let handlers = ServerRequestHandlers {
        sampling: Some(Arc::new(TestSampling)),
        elicitation: Some(Arc::new(TestElicitation)),
    };
    let default = Box::pin(answered("capabilities", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert!(!default.contains("sampling"), "{default}");
    assert!(!default.contains("elicitation"), "{default}");
    let configured = Box::pin(answered("capabilities", handlers)).await.unwrap();
    let capabilities: ClientCapabilities = serde_json::from_str(&configured).unwrap();
    assert!(capabilities.sampling.is_some(), "{configured}");
    let elicitation = capabilities
        .elicitation
        .expect("the elicitation capability is advertised");
    assert!(elicitation.form.is_some(), "{configured}");
    assert!(elicitation.url.is_some(), "{configured}");
}

#[tokio::test]
async fn the_default_handshake_sends_the_sdk_client_info() {
    let answer = Box::pin(answered("client-info", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert!(answer.contains(r#""name":"mcp""#), "{answer}");
    assert!(answer.contains(r#""version":"0.1.0""#), "{answer}");
}

#[tokio::test]
async fn a_request_after_initialize_carries_the_negotiated_protocol_version() {
    let version = Box::pin(answered(
        "protocol-version",
        ServerRequestHandlers::default(),
    ))
    .await
    .unwrap();
    assert_eq!(version, SdkProtocolVersion::LATEST_WITH_INITIALIZE.as_str());
}

#[tokio::test]
async fn a_logging_notification_does_not_disturb_the_call() {
    let answer = Box::pin(answered("log", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert_eq!(answer, "logged");
}

#[tokio::test]
async fn a_method_the_handler_does_not_know_is_method_not_found() {
    let answer = Box::pin(answered("custom", ServerRequestHandlers::default()))
        .await
        .unwrap();
    assert_eq!(
        answer,
        "error -32601: Method not found: \"not/a-real-method\""
    );
}

#[tokio::test]
async fn a_tool_result_that_matches_its_output_schema_is_complete() {
    let server = TestServer::standard().with_output_schemas();
    let (result, listings) = Box::pin(called_on(
        server,
        "schema-ok",
        RAISE,
        ServerRequestHandlers::default(),
    ))
    .await
    .unwrap();
    assert_eq!(text_of(&result), "structured");
    // the session had not seen the tool's schema, so it listed the tools once before validating
    assert_eq!(listings, 1);
}

#[tokio::test]
async fn a_listed_tool_without_an_output_schema_skips_validation() {
    let server = TestServer::standard().with_output_schemas();
    let (result, listings) = Box::pin(called_on(
        server,
        "plain",
        RAISE,
        ServerRequestHandlers::default(),
    ))
    .await
    .unwrap();
    assert_eq!(text_of(&result), "plain");
    assert_eq!(listings, 1);
}

#[tokio::test]
async fn a_tool_the_listing_does_not_name_is_called_without_validation() {
    let server = TestServer::standard().with_output_schemas();
    let (result, listings) = Box::pin(called_on(
        server,
        "echo",
        RAISE,
        ServerRequestHandlers::default(),
    ))
    .await
    .unwrap();
    assert_eq!(text_of(&result), "echoed");
    assert_eq!(listings, 1);
}

#[tokio::test]
async fn a_result_missing_the_structured_content_its_schema_needs_is_a_runtime_failure() {
    let server = TestServer::standard().with_output_schemas();
    let error = Box::pin(called_on(
        server,
        "schema-missing",
        RAISE,
        ServerRequestHandlers::default(),
    ))
    .await
    .expect_err("the tool declares an output schema");
    assert!(matches!(error, SessionError::Runtime(_)), "{error}");
}

#[tokio::test]
async fn structured_content_that_breaks_the_output_schema_is_a_runtime_failure() {
    let server = TestServer::standard().with_output_schemas();
    let error = Box::pin(called_on(
        server,
        "schema-invalid",
        RAISE,
        ServerRequestHandlers::default(),
    ))
    .await
    .expect_err("the structured content is not an integer");
    assert!(matches!(error, SessionError::Runtime(_)), "{error}");
}

#[tokio::test]
async fn an_output_schema_that_does_not_compile_is_a_runtime_failure() {
    let server = TestServer::standard().with_output_schemas();
    let error = Box::pin(called_on(
        server,
        "schema-broken",
        RAISE,
        ServerRequestHandlers::default(),
    ))
    .await
    .expect_err("the declared schema is not a valid JSON Schema");
    assert!(matches!(error, SessionError::Runtime(_)), "{error}");
}

#[tokio::test]
async fn an_error_result_with_an_output_schema_skips_validation() {
    let server = TestServer::standard().with_output_schemas();
    let (result, listings) = Box::pin(called_on(
        server,
        "schema-error",
        CallToolOptions::default(),
        ServerRequestHandlers::default(),
    ))
    .await
    .unwrap();
    let ToolCallResult::Complete(complete) = &result else {
        panic!("expected a complete tool result");
    };
    assert!(complete.is_error);
    assert_eq!(text_of(&result), "the tool failed");
    assert_eq!(listings, 0);
}

#[tokio::test]
async fn a_session_lists_the_tools_once_for_its_cached_output_schemas() {
    let server = TestServer::standard().with_output_schemas();
    let listings = Arc::clone(&server.listings);
    let endpoint = serve_http(server).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &endpoint.url,
        support::CALL_TIMEOUT,
    );
    let result = Box::pin(run_with_session(
        &mut client,
        async |session| {
            let first = <SdkSession as McpSession>::call_tool(session, call("schema-ok"))
                .await
                .unwrap();
            let second = <SdkSession as McpSession>::call_tool(session, call("schema-ok"))
                .await
                .unwrap();
            assert_eq!(text_of(&first), "structured");
            Ok::<ToolCallResult, SessionError<SdkError>>(second)
        },
        false,
    ))
    .await
    .unwrap();
    assert_eq!(text_of(&result), "structured");
    assert_eq!(listings.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_cursored_listing_keeps_the_output_schemas_of_other_pages() {
    let server = TestServer::standard().with_output_schemas();
    let listings = Arc::clone(&server.listings);
    let endpoint = serve_http(server).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &endpoint.url,
        support::CALL_TIMEOUT,
    );
    let result = Box::pin(run_with_session(
        &mut client,
        async |session| {
            let first = <SdkSession as McpSession>::list_tools(session, None)
                .await
                .unwrap();
            assert_eq!(first.tools.len(), 8);
            let second =
                <SdkSession as McpSession>::list_tools(session, first.next_cursor.as_deref())
                    .await
                    .unwrap();
            assert_eq!(second.tools.len(), 1);
            <SdkSession as McpSession>::call_tool(session, call("schema-ok")).await
        },
        false,
    ))
    .await
    .unwrap();
    assert_eq!(text_of(&result), "structured");
    // the page-2 request carried a cursor, so schema-ok keeps its cached schema and the call lists nothing
    assert_eq!(listings.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_complete_listing_drops_the_output_schemas_it_no_longer_carries() {
    let server = TestServer::standard().with_output_schemas().with_one_page();
    let listings = Arc::clone(&server.listings);
    let endpoint = serve_http(server).await;
    let mut client = support::client_on(
        Arc::new(support::runtime()),
        support::settings(TransportType::Http),
        &endpoint.url,
        support::CALL_TIMEOUT,
    );
    let result = Box::pin(run_with_session(
        &mut client,
        async |session| {
            let first = <SdkSession as McpSession>::list_tools(session, None)
                .await
                .unwrap();
            assert_eq!(first.tools.len(), 8);
            assert!(first.next_cursor.is_none());
            let second = <SdkSession as McpSession>::list_tools(session, None)
                .await
                .unwrap();
            assert_eq!(second.tools.len(), 2);
            <SdkSession as McpSession>::call_tool(session, call("schema-ok")).await
        },
        false,
    ))
    .await
    .unwrap();
    assert_eq!(text_of(&result), "structured");
    // the second listing is complete and no longer names schema-ok, so the call lists the tools again
    assert_eq!(listings.load(Ordering::SeqCst), 3);
}
