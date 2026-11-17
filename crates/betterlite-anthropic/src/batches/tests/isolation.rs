use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchObject;

use super::support::{
    Fixture, Silent, batches, config, configured, header, logging_env, logging_state, redactor,
};
use crate::batches::config::AnthropicBatchesConfig;
use crate::batches::error::BatchesError;
use crate::batches::handler::{AnthropicBatchesHandler, BatchTransport, RetrieveRequest};
use crate::response::Logging;

const FIRST_KEY: &str = "sk-id-a";
const SECOND_KEY: &str = "sk-id-b";

fn lifecycle(id: &str) -> String {
    format!(r#"{{"id":"{id}","processing_status":"in_progress"}}"#)
}

async fn fetch<T: BatchTransport>(
    handler: &AnthropicBatchesHandler<T>,
    batch_id: &str,
) -> Result<BatchObject, BatchesError> {
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state(batch_id, &redactor);
    let env = logging_env(&redactor, &silent);
    let request = RetrieveRequest {
        batch_id,
        api_base: None,
        api_key: None,
        timeout: None,
        max_retries: None,
    };
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    handler.retrieve_batch(&request, logging).await
}

#[tokio::test]
async fn batches_credential_identity_isolation() {
    let first = Fixture::answering(200, lifecycle("msgbatch_a").as_bytes());
    let second = Fixture::answering(200, lifecycle("msgbatch_b").as_bytes());
    let handler_a = batches(&first, config(Some(FIRST_KEY), Some("https://a.example/")));
    let handler_b = batches(
        &second,
        config(Some(SECOND_KEY), Some("https://b.example/")),
    );
    let (batch_a, batch_b) = tokio::join!(fetch(&handler_a, "shared"), fetch(&handler_b, "shared"));
    assert_eq!(batch_a.unwrap().id, "msgbatch_a");
    assert_eq!(batch_b.unwrap().id, "msgbatch_b");
    let first_requests = first.take_requests();
    assert_eq!(first_requests.len(), 1);
    assert_each_identity(&first_requests[0], &second.only_request());
    let again = fetch(&handler_a, "shared").await.unwrap();
    assert_eq!(again.id, "msgbatch_a");
    let repeated = first.take_requests();
    assert_eq!(repeated.len(), 1);
    assert_eq!(second.sent(), 0);
    assert_eq!(header(&repeated[0].headers, "x-api-key"), FIRST_KEY);
    assert_eq!(
        repeated[0].url.as_str(),
        "https://a.example/v1/messages/batches/shared"
    );
}

fn assert_each_identity(first: &OutboundRequest, second: &OutboundRequest) {
    assert_eq!(
        first.url.as_str(),
        "https://a.example/v1/messages/batches/shared"
    );
    assert_eq!(
        second.url.as_str(),
        "https://b.example/v1/messages/batches/shared"
    );
    assert_eq!(header(&first.headers, "x-api-key"), FIRST_KEY);
    assert_eq!(header(&second.headers, "x-api-key"), SECOND_KEY);
    assert_ne!(
        header(&first.headers, "x-api-key"),
        header(&second.headers, "x-api-key")
    );
    assert!(first.headers.get("x-api-key").unwrap().is_sensitive());
    assert!(second.headers.get("x-api-key").unwrap().is_sensitive());
}

#[tokio::test]
async fn batches_secret_safe_failure() {
    let key = "sk-live-fixture-9c1f";
    let token = "auth-token-fixture-7a2b";
    let batched = configured(Some(key), Some(token), None);
    let config_debug = format!("{batched:?}");
    assert!(!config_debug.contains(key));
    assert!(!config_debug.contains(token));
    transport_failure_is_silent(key, token).await;
    missing_key_hides_token(token).await;
    recorded_headers_are_masked(key).await;
}

async fn transport_failure_is_silent(key: &str, token: &str) {
    let fixture = Fixture::failing();
    let handler = batches(
        &fixture,
        AnthropicBatchesConfig {
            settings: configured(Some(key), Some(token), None),
        },
    );
    let error = fetch(&handler, "b").await.unwrap_err();
    assert!(!error.to_string().contains(key));
    assert!(!error.to_string().contains(token));
    assert!(!format!("{error:?}").contains(key));
    assert!(!format!("{error:?}").contains(token));
    assert_eq!(fixture.sent(), 1);
    let handler_debug = format!("{handler:?}");
    assert!(!handler_debug.contains(key));
    assert!(!handler_debug.contains(token));
}

async fn missing_key_hides_token(token: &str) {
    let fixture = Fixture::answering(200, br#"{"id":"b"}"#);
    let handler = batches(
        &fixture,
        AnthropicBatchesConfig {
            settings: configured(None, Some(token), None),
        },
    );
    let error = fetch(&handler, "b").await.unwrap_err();
    assert!(matches!(error, BatchesError::MissingApiKey));
    assert!(!error.to_string().contains(token));
    assert!(!format!("{error:?}").contains(token));
    assert_eq!(fixture.sent(), 0);
}

async fn recorded_headers_are_masked(key: &str) {
    let fixture = Fixture::answering(200, br#"{"id":"b"}"#);
    let handler = batches(&fixture, config(Some(key), None));
    let redactor = redactor();
    let silent = Silent;
    let mut state = logging_state("b", &redactor);
    let env = logging_env(&redactor, &silent);
    let request = RetrieveRequest {
        batch_id: "b",
        api_base: None,
        api_key: None,
        timeout: None,
        max_retries: None,
    };
    let logging = Logging {
        state: &mut state,
        env: &env,
    };
    handler.retrieve_batch(&request, logging).await.unwrap();
    let details = state.model_call_details.get("additional_args").unwrap();
    let shown = serde_json::to_string(details).unwrap();
    assert!(!shown.contains(key), "{shown}");
    let Json::Object(fields) = details else {
        panic!("pre_call records the additional args");
    };
    assert!(fields.contains_key("headers"));
    assert!(!format!("{handler:?}").contains(key));
}
