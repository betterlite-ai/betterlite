use crate::api::responses_create::error::CreateError;
use crate::api::responses_create_http::error::CreateError as HttpCreateError;
use crate::api::vector_store_http_write::error::{BaseLlmException, HeaderMap};

use super::super::error::{AsyncCreateCause, AsyncCreateError};
use super::support::{self, Outcome, ScriptedHandoff, ScriptedResolver};

const CALL: &str = r#"{"model":"openai/gpt-4o","input":"hello"}"#;
const NULL: &str = "Got an unexpected None response from the Responses API: None";

pub(super) async fn failure_attribution() {
    null_completed().await;
    null_pending().await;
    detection().await;
    missing_provider().await;
    authentication().await;
    redacted().await;
}

async fn null_completed() {
    let error = failure(Outcome::Completed(None)).await;
    assert!(matches!(*error.cause, AsyncCreateCause::NullResponse));
    assert_eq!(error.cause.to_string(), NULL);
    assert_eq!(error.provider, "openai");
    assert_eq!(error.model, "openai/gpt-4o");
    let mapped = support::failure(&error);
    assert_eq!(mapped.class, "APIConnectionError");
    assert_eq!(mapped.status, 500);
    assert_eq!(mapped.kind, None);
    assert_eq!(mapped.param, None);
    assert_eq!(mapped.code, None);
    assert_eq!(
        mapped.message,
        format!("APIConnectionError: OpenAIException - {NULL}")
    );
}

// The coroutine answer is awaited once even when it resolves to None
async fn null_pending() {
    let handoff = ScriptedHandoff::new(Outcome::Pending(None));
    let resolver = ScriptedResolver::default();
    let call = support::call_from(CALL);
    let error = match support::drive(&resolver, None, &handoff, &call).await {
        Ok(_created) => panic!("the wrapper fails"),
        Err(error) => error,
    };
    assert_eq!(handoff.polls(), 2);
    assert!(matches!(*error.cause, AsyncCreateCause::NullResponse));
    let mapped = support::failure(&error);
    assert_eq!(mapped.class, "APIConnectionError");
    assert_eq!(mapped.status, 500);
}

// LiteLLM's aresponses never has a base_url local, so detection asks for the model alone
async fn detection() {
    let handoff = ScriptedHandoff::new(Outcome::Completed(None));
    let resolver = ScriptedResolver::default();
    let call = support::call_from(CALL);
    let error = match support::drive(&resolver, None, &handoff, &call).await {
        Ok(_created) => panic!("the wrapper fails"),
        Err(error) => error,
    };
    let requests = resolver.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, "openai/gpt-4o");
    assert_eq!(requests[0].custom_llm_provider, None);
    assert_eq!(requests[0].api_base, None);
    assert_eq!(error.provider, "openai");
}

// LiteLLM's get_llm_provider raises BadRequestError/400 for a model no provider can place, before any HTTP
async fn missing_provider() {
    let handoff = ScriptedHandoff::new(Outcome::Completed(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    let call = support::call_from(r#"{"model":"no-such-model","input":"hello"}"#);
    let error = match support::drive(&resolver, None, &handoff, &call).await {
        Ok(_created) => panic!("the wrapper fails"),
        Err(error) => error,
    };
    assert!(matches!(
        *error.cause,
        AsyncCreateCause::ProviderResolution(_)
    ));
    assert_eq!(error.provider, "");
    assert_eq!(error.model, "no-such-model");
    let mapped = support::failure(&error);
    assert_eq!(mapped.class, "BadRequestError");
    assert_eq!(mapped.status, 400);
    assert_eq!(mapped.kind, None);
    assert_eq!(mapped.param, None);
    assert_eq!(mapped.code, None);
    assert_eq!(
        mapped.message,
        "litellm.BadRequestError: LLM Provider NOT provided. Pass in the LLM provider you are trying to call. You passed model=no-such-model"
    );
    assert!(handoff.calls().is_empty());
}

// A provider 401 becomes the SDK AuthenticationError without the body's type, param or code
async fn authentication() {
    let error = failure(Outcome::Failed(CreateError::Request(provider(
        401, "bad key",
    ))))
    .await;
    let mapped = support::failure(&error);
    assert_eq!(mapped.class, "AuthenticationError");
    assert_eq!(mapped.status, 401);
    assert_eq!(mapped.kind, None);
    assert_eq!(mapped.param, None);
    assert_eq!(mapped.code, None);
    assert_eq!(
        mapped.message,
        "AuthenticationError: OpenAIException - bad key"
    );
}

async fn redacted() {
    let secret = "sk-abcdefghijklmno";
    let error = failure(Outcome::Failed(CreateError::Request(provider(
        401,
        "bad key sk-abcdefghijklmno",
    ))))
    .await;
    let mapped = support::failure(&error);
    assert!(!mapped.message.contains(secret), "{}", mapped.message);
    assert!(mapped.message.contains("REDACTED"), "{}", mapped.message);
}

fn provider(status: u16, message: &str) -> HttpCreateError {
    HttpCreateError::Provider(BaseLlmException {
        status_code: status,
        message: message.to_owned(),
        headers: HeaderMap::new(),
        status_code_is_synthesized: false,
    })
}

async fn failure(outcome: Outcome) -> AsyncCreateError {
    let handoff = ScriptedHandoff::new(outcome);
    let resolver = ScriptedResolver::default();
    let call = support::call_from(CALL);
    match support::drive(&resolver, None, &handoff, &call).await {
        Ok(_created) => panic!("the wrapper fails"),
        Err(error) => error,
    }
}
