#![cfg(test)]

use betterlite_models::decisions::Json;

use crate::completion::error::AzureTextFailure;
use crate::completion::handler::TextCompletionOutcome;
use crate::completion::tests::fixture::{Answer, Fixed, ScriptedSource, object};
use crate::endpoints::projection::ClientKind;

use super::support::{
    MODEL, Parts, buffered, completion, data_field, number, sent, streaming, text_messages,
};

pub(super) fn max_retries_default_two() {
    let fixed = Fixed::new();
    let parts = Parts::new();
    let messages = text_messages();

    // a missing key defaults to the literal 2, never the environment setting
    let mut params = object("{}");
    let mut hooks = super::recorder();
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let creates = seen.creates();
    assert_eq!(data_field(&creates, "max_retries"), Json::Null);

    // an explicit null stays null, and only the sync guard rejects it
    let mut params = object(r#"{"max_retries":null}"#);
    let error = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(422));
    assert_eq!(error.message, "max retries must be an int");
    assert_eq!(hooks.pre.len(), 2, "the pre call runs before the guard");

    let mut params = object(r#"{"max_retries":null}"#);
    let mut hooks = super::recorder();
    let outcome = completion(
        ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert!(matches!(outcome, TextCompletionOutcome::Buffered(_)));

    // isinstance(value, int) accepts a bool and rejects a float
    let mut params = object(r#"{"max_retries":true}"#);
    completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let mut params = object(r#"{"max_retries":1.5}"#);
    let error = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(422));

    // the cloudflare client is built with the popped value
    let mut parts = Parts::new();
    parts.api_base = Some("https://gateway.ai.cloudflare.com/gw".to_owned());
    let mut params = object("{}");
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert_eq!(
        number(&super::support::field(&seen.gateways()[0], "max_retries")),
        2
    );
}

pub(super) fn invalid_retries_422() {
    let fixed = Fixed::new();
    let parts = Parts::new();
    let messages = text_messages();
    let mut params = object(r#"{"max_retries":"3"}"#);
    let mut hooks = super::recorder();
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    let error = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(422));
    assert_eq!(error.message, "max retries must be an int");
    assert_eq!(hooks.pre.len(), 1);
    assert!(seen.gets().is_empty(), "the guard runs before the client");

    // the direct sync stream repeats the pop on the data it was handed
    let call_parts = Parts::new();
    let mut params = object("{}");
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    let mut payload = object(r#"{"max_retries":"3","model":"x","prompt":"p"}"#);
    let call = call_parts.call(Some(&messages), &mut params, false);
    let mut sent_call = sent(MODEL, payload.clone(), &call_parts.litellm_params);
    let mut hooks = super::recorder();
    let failure = streaming(source, call, &mut sent_call, &fixed, &mut hooks).unwrap_err();
    assert!(matches!(
        failure,
        AzureTextFailure::Azure(ref error) if error.status == Some(422)
    ));
    assert!(seen.gets().is_empty());
    payload.clear();
}

pub(super) fn missing_model_messages_422() {
    let fixed = Fixed::new();
    let messages = text_messages();
    let mut hooks = super::recorder();

    let mut parts = Parts::new();
    parts.model = None;
    let mut params = object("{}");
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    let error = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(422));
    assert_eq!(error.message, "Missing model or messages");
    assert!(hooks.pre.is_empty(), "the guard runs before the prompt");
    assert!(seen.gets().is_empty());

    let parts = Parts::new();
    let mut params = object("{}");
    let error = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(None, &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(422));
    assert_eq!(error.message, "Missing model or messages");

    // an empty model and an empty message list are values, not absences
    let mut parts = Parts::new();
    parts.model = Some(String::new());
    let empty: Vec<betterlite_prompts::builtin::Message> = Vec::new();
    let mut params = object("{}");
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    let outcome = completion(
        source,
        parts.call(Some(&empty), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert!(matches!(outcome, TextCompletionOutcome::Buffered(_)));
    assert_eq!(
        super::support::text(&data_field(&seen.creates(), "model")),
        ""
    );
}
