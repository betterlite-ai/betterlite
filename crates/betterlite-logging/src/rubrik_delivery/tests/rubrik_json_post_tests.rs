use std::time::Duration;

use betterlite_logging::rubrik_delivery::error::{RubrikError, Service};
use betterlite_logging::rubrik_delivery::post::{post_prompt_moderation, post_response_moderation};
use betterlite_logging::rubrik_delivery::queue::{append_and_maybe_flush, flush_queue};
use betterlite_logging::rubrik_delivery::transport::{ClientTier, LOGGING_TIMEOUT, Timeout};
use betterlite_models::decisions::Json;

use super::support::{
    LoggerPair, ScriptedReply, bare_init, init, logger, moderation_timeout, number, object, payload,
};

fn moderation_logger(reply: ScriptedReply) -> LoggerPair {
    logger(&init("https://rubrik.test", "test-key"), vec![reply])
}

#[tokio::test]
async fn response_moderation_posts_the_request_response_envelope() {
    let (logger, transport) = moderation_logger(ScriptedReply::ok(r#"{"choices":[]}"#));
    let response = payload(r#"{"id":"chatcmpl-1","object":"chat.completion"}"#);
    let request = payload(r#"{"model":"gpt-4o","messages":[]}"#);
    let returned = post_response_moderation(&logger, &response, &request)
        .await
        .expect("the post succeeds");
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        "https://rubrik.test/v1/after_completion/openai/v1"
    );
    assert_eq!(
        calls[0].payload,
        object(&[("request", request), ("response", response)])
    );
    assert_eq!(calls[0].header("content-type"), Some("application/json"));
    assert_eq!(calls[0].header("authorization"), Some("Bearer test-key"));
    assert_eq!(Json::Object(returned), payload(r#"{"choices":[]}"#));
}

#[tokio::test]
async fn prompt_moderation_posts_the_bare_request() {
    let (logger, transport) = moderation_logger(ScriptedReply::ok(
        r#"{"choices":[{"message":{"content":"no"}}]}"#,
    ));
    let request = payload(r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#);
    let returned = post_prompt_moderation(&logger, &request)
        .await
        .expect("the post succeeds");
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].url,
        "https://rubrik.test/v1/before_prompt/openai/v1"
    );
    assert_eq!(calls[0].payload, request);
    assert_eq!(calls[0].header("authorization"), Some("Bearer test-key"));
    assert_eq!(
        Json::Object(returned),
        payload(r#"{"choices":[{"message":{"content":"no"}}]}"#)
    );
}

#[tokio::test]
async fn missing_api_key_sends_only_the_content_type_header() {
    let mut options = bare_init();
    options.api_base = Some("https://rubrik.test".to_owned());
    let (logger, transport) = logger(&options, vec![ScriptedReply::ok("{}")]);
    post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect("the post succeeds");
    let calls = transport.calls();
    assert_eq!(calls[0].header("content-type"), Some("application/json"));
    assert_eq!(calls[0].header("authorization"), None);
}

#[tokio::test]
async fn non_object_response_is_an_error() {
    let (logger, transport) = moderation_logger(ScriptedReply::ok("[1,2]"));
    let error = post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect_err("a list response fails");
    assert!(matches!(
        error,
        RubrikError::NonObjectResponse {
            service: Service::PromptModeration
        }
    ));
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn network_failure_is_an_error() {
    let (logger, _transport) = moderation_logger(ScriptedReply::network());
    let error = post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect_err("a network failure fails");
    assert!(matches!(
        error,
        RubrikError::Transport {
            service: Service::PromptModeration,
            ..
        }
    ));
}

#[tokio::test]
async fn http_status_is_an_error() {
    let (logger, _transport) = moderation_logger(ScriptedReply::status(500));
    let error = post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect_err("a server error fails");
    assert!(matches!(
        error,
        RubrikError::HttpStatus {
            service: Service::PromptModeration,
            status: 500,
        }
    ));
}

#[tokio::test]
async fn invalid_json_is_an_error() {
    let (logger, _transport) = moderation_logger(ScriptedReply::ok("{"));
    let error = post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect_err("an invalid body fails");
    assert!(matches!(
        error,
        RubrikError::ResponseBody {
            service: Service::PromptModeration,
            ..
        }
    ));
}

#[tokio::test]
async fn configured_timeout_is_sent_to_the_moderation_webhook() {
    let mut options = init("https://rubrik.test", "test-key");
    options.timeout = Some(Duration::from_millis(2500));
    let (logger, transport) = logger(&options, vec![ScriptedReply::ok("{}")]);
    post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect("the post succeeds");
    let calls = transport.calls();
    assert_eq!(
        calls[0].timeout,
        Timeout {
            total: Duration::from_millis(2500),
            connect: Duration::from_millis(2500),
        }
    );
    assert_eq!(number(calls[0].timeout.total.as_secs_f64()), number(2.5));
}

#[tokio::test]
async fn default_moderation_timeout_matches_the_moderation_client() {
    let (logger, transport) = moderation_logger(ScriptedReply::ok("{}"));
    post_prompt_moderation(&logger, &payload(r#"{"model":"gpt-4o"}"#))
        .await
        .expect("the post succeeds");
    let calls = transport.calls();
    assert_eq!(calls[0].timeout, moderation_timeout());
}

#[tokio::test]
async fn logging_batch_posts_the_queue_to_the_logging_endpoint() {
    let (logger, transport) = logger(
        &init("https://rubrik.test", "test-key"),
        vec![ScriptedReply::ok("{}")],
    );
    append_and_maybe_flush(&logger, payload(r#"{"index":0}"#)).await;
    append_and_maybe_flush(&logger, payload(r#"{"index":1}"#)).await;
    flush_queue(&logger).await;
    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tier, ClientTier::Logging);
    assert_eq!(calls[0].url, "https://rubrik.test/v1/litellm/batch");
    assert_eq!(calls[0].payload, payload(r#"[{"index":0},{"index":1}]"#));
    assert_eq!(calls[0].timeout, LOGGING_TIMEOUT);
    assert_eq!(calls[0].header("authorization"), Some("Bearer test-key"));
}
