#![cfg(test)]

mod cases;
mod streaming;
mod support;

use std::str::from_utf8;

use betterlite_models::controls::call_params::DropParams;
use betterlite_models::params::ParameterError;

use crate::anthropic_dispatch::AnthropicResult;
use crate::anthropic_dispatch::native::tests::support::wire;
use crate::operation::{OperationError, OperationResult};

const CHAT_REPLY: &str = r#"{"id":"msg_gateway","type":"message","role":"assistant","model":"claude-3-haiku-20240307","content":[{"type":"text","text":"native answer"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
const USER: &str = r#"[{"role":"user","content":"hi"}]"#;

// One loopback case per DoD item; the caller names them exactly as the task does.
pub(super) async fn check(name: &str) {
    match name {
        "anthropic_provider_dispatch_http" => cases::http().await,
        "anthropic_provider_dispatch_stream" => streaming::stream().await,
        "anthropic_provider_dispatch_thinking" => streaming::thinking().await,
        "anthropic_provider_dispatch_error" => cases::error().await,
        "anthropic_provider_unsupported_param" => cases::unsupported_param().await,
        "anthropic_provider_dropped_param" => cases::dropped_param().await,
        "anthropic_provider_remote_file" => cases::remote_file().await,
        "anthropic_provider_remote_file_error" => cases::remote_file_error().await,
        "anthropic_provider_deployment_drop_params" => deployment_drop_params().await,
        "anthropic_provider_request_drop_params_wins" => request_drop_params_wins().await,
        other => panic!("unknown anthropic gateway case {other}"),
    }
}

fn buffered() -> wire::Answer {
    wire::Answer {
        status: 200,
        content_type: "application/json",
        body: String::from(CHAT_REPLY),
    }
}

fn chat_body(extra: &str) -> String {
    format!(
        r#"{{"model":"{}","messages":{USER}{extra}}}"#,
        support::MODEL
    )
}

// A deployment's drop_params reaches the Anthropic mapping even when the request sends no flag.
async fn deployment_drop_params() {
    let mut harness = support::Harness::start(vec![buffered()]).await;
    harness.deployment.drop_params = Some(DropParams::Flag(true));
    let result = support::complete(
        &harness,
        support::chat(&chat_body(r#","logit_bias":{"1":2}"#)),
    )
    .await
    .expect("the deployment drop_params keeps the call");
    assert!(matches!(
        result,
        OperationResult::Anthropic(AnthropicResult::Reply(_))
    ));
    let requests = harness.requests().await;
    assert_eq!(requests.len(), 1);
    let text = from_utf8(&requests[0].body).expect("the request body is text");
    assert!(!text.contains("logit_bias"));
}

// The request's own drop_params answers for the call, so its false wins over the deployment's true.
async fn request_drop_params_wins() {
    let mut harness = support::Harness::start(Vec::new()).await;
    harness.deployment.drop_params = Some(DropParams::Flag(true));
    let result = support::complete(
        &harness,
        support::chat(&chat_body(r#","logit_bias":{"1":2},"drop_params":false"#)),
    )
    .await;
    let Err(OperationError::Parameters {
        source: ParameterError::Unsupported { names, .. },
    }) = result
    else {
        panic!("the request's false wins and logit_bias is unsupported");
    };
    assert_eq!(names, vec![String::from("logit_bias")]);
    assert!(harness.requests().await.is_empty());
}
