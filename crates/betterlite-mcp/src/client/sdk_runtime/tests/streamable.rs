use std::sync::Arc;

use crate::client::sdk_runtime::convert::ServerCapabilitiesWire;
use crate::client::sdk_runtime::handler::ServerRequestHandlers;
use crate::client::sdk_transport::error::SdkError;
use crate::client::sessions::error::SessionError;
use crate::client::sessions::negotiate::{LegacyVersion, ProtocolVersion};
use crate::client::sessions::run::run_with_session;
use crate::client::sessions::transport::TransportType;
use crate::client::tools::calling::{CallToolOptions, call_tool};
use crate::client::tools::listing::list_tools;
use crate::client::tools::result::{ContentBlock, ToolCallResult};
use crate::tools::tool_calls::CallRequest;

use super::server::{INSTRUCTIONS, TestElicitation, TestSampling, TestServer, serve_http};
use super::support::{self, RecordingHost};

const RAISE: CallToolOptions = CallToolOptions {
    raise_on_error: true,
    allow_input_required: false,
};

fn call(name: &str) -> CallRequest {
    let name = name.to_owned();
    let arguments = None;
    CallRequest { name, arguments }
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
    let mut client = support::client_for(
        support::runtime(),
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
    let mut client = support::client_for(
        support::runtime(),
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
    let capabilities = capabilities.expect("the server's capabilities were adopted");
    assert!(capabilities.prompts.is_some());
    assert!(capabilities.resources.is_some());
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
    let mut client = support::client_for(
        support::runtime(),
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
    let mut client = support::client_for(
        support::runtime(),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let host = RecordingHost::default();
    let result = call_tool(
        &mut client,
        &call("progress"),
        Some(&host),
        CallToolOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(host.seen(), vec![(25.0, Some(100.0)), (75.0, Some(100.0))]);
    assert_eq!(text_of(&result), "progressed");
}

#[tokio::test]
async fn an_is_error_result_stays_a_complete_result() {
    let server = serve_http(TestServer::standard()).await;
    let mut client = support::client_for(
        support::runtime(),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let result = call_tool(
        &mut client,
        &call("fail"),
        None::<&RecordingHost>,
        CallToolOptions::default(),
    )
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
    let mut client = support::client_for(
        support::runtime(),
        support::settings(TransportType::Http),
        &server.url,
        support::CALL_TIMEOUT,
    );
    let error = call_tool(
        &mut client,
        &call("rpc-error"),
        None::<&RecordingHost>,
        RAISE,
    )
    .await
    .unwrap_err();
    match &error {
        SessionError::Protocol(failure) => {
            assert_eq!(failure.code, -32_602);
            assert_eq!(failure.message, "bad arguments");
        }
        other => panic!("expected a protocol failure, got {other}"),
    }
}

async fn answered(
    name: &str,
    handlers: ServerRequestHandlers,
) -> Result<String, SessionError<SdkError>> {
    let server = serve_http(TestServer::standard()).await;
    let mut settings = support::settings(TransportType::Http);
    settings.handlers = handlers;
    let mut client = support::client_for(
        support::runtime(),
        settings,
        &server.url,
        support::CALL_TIMEOUT,
    );
    let result = call_tool(&mut client, &call(name), None::<&RecordingHost>, RAISE).await?;
    Ok(text_of(&result).to_owned())
}

#[tokio::test]
async fn sampling_without_a_responder_answers_the_sdk_default() {
    let answer = answered("sample", ServerRequestHandlers::default())
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
    let answer = answered("sample", handlers).await.unwrap();
    assert!(answer.contains("\"model\":\"test-model\""), "{answer}");
    assert!(answer.contains("1 message(s)"), "{answer}");
}

#[tokio::test]
async fn elicitation_without_a_responder_answers_the_sdk_default() {
    let answer = answered("elicit", ServerRequestHandlers::default())
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
    let answer = answered("elicit", handlers).await.unwrap();
    assert!(answer.contains("\"action\":\"accept\""), "{answer}");
    assert!(answer.contains("who are you?"), "{answer}");
}

#[tokio::test]
async fn a_ping_answers_an_empty_result() {
    let answer = answered("ping", ServerRequestHandlers::default())
        .await
        .unwrap();
    assert_eq!(answer, "{}");
}

#[tokio::test]
async fn a_roots_request_answers_the_sdk_default() {
    let answer = answered("roots", ServerRequestHandlers::default())
        .await
        .unwrap();
    assert_eq!(answer, "error -32600: List roots not supported");
}

#[tokio::test]
async fn the_client_capabilities_name_the_configured_callbacks_only() {
    let handlers = ServerRequestHandlers {
        sampling: Some(Arc::new(TestSampling)),
        elicitation: None,
    };
    let default = answered("capabilities", ServerRequestHandlers::default())
        .await
        .unwrap();
    assert!(!default.contains("sampling"), "{default}");
    let configured = answered("capabilities", handlers).await.unwrap();
    assert!(configured.contains("\"sampling\""), "{configured}");
    assert!(!configured.contains("elicitation"), "{configured}");
}

#[tokio::test]
async fn a_logging_notification_does_not_disturb_the_call() {
    let answer = answered("log", ServerRequestHandlers::default())
        .await
        .unwrap();
    assert_eq!(answer, "logged");
}

#[tokio::test]
async fn a_method_the_handler_does_not_know_is_method_not_found() {
    let answer = answered("custom", ServerRequestHandlers::default())
        .await
        .unwrap();
    assert_eq!(
        answer,
        "error -32601: Method not found: \"not/a-real-method\""
    );
}
