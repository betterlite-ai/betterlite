#![cfg(test)]

use betterlite_models::decisions::Json;
use secrecy::SecretString;

use crate::completion::handler::{AzureTextCompletion, TextCompletionOutcome};
use crate::completion::tests::fixture::{Answer, Fixed, ScriptedSource, object, run};
use crate::endpoints::projection::ClientKind;

use super::support::{
    Parts, buffered, chunks, completion, contents, data_field, nested, text, text_messages,
};

pub(super) fn azure_text_auth() {
    // validate_environment builds the SDK's default headers: a key wins, a token fills in, none is empty
    let key = SecretString::from("sk-unit");
    let token = SecretString::from("ad-token");
    let both =
        AzureTextCompletion::<ScriptedSource>::validate_environment(Some(&key), Some(&token));
    assert_eq!(both.0.len(), 2, "a key drops the token header");
    assert_eq!(
        both.0.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(both.0.get("api-key").map(String::as_str), Some("sk-unit"));
    assert!(!both.0.contains_key("Authorization"));
    let token_only =
        AzureTextCompletion::<ScriptedSource>::validate_environment(None, Some(&token));
    assert_eq!(
        token_only.0.get("Authorization").map(String::as_str),
        Some("Bearer ad-token")
    );
    assert!(!token_only.0.contains_key("api-key"));
    let empty = SecretString::from("");
    let empty_key =
        AzureTextCompletion::<ScriptedSource>::validate_environment(Some(&empty), Some(&token));
    assert_eq!(empty_key.0.get("api-key").map(String::as_str), Some(""));
    let none = AzureTextCompletion::<ScriptedSource>::validate_environment(None, None);
    assert_eq!(none.0.len(), 1);

    let fixed = Fixed::new();
    let messages = text_messages();
    let mut params = object("{}");
    let mut hooks = super::recorder();
    let mut parts = Parts::new();
    parts.key = None;
    parts.token = Some(SecretString::from("ad-token"));
    completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let pre = &hooks.pre[0];
    assert_eq!(
        nested(pre, &["additional_args", "headers", "api_key"]),
        Json::Null
    );
    assert_eq!(
        text(&nested(
            pre,
            &["additional_args", "headers", "azure_ad_token"]
        )),
        "ad-token"
    );

    // the async pre call reports the client's own key, and None with an absent one
    let mut source = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered());
    source.api_key = None;
    let mut params = object("{}");
    let mut hooks = super::recorder();
    let parts = Parts::new();
    completion(
        source,
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert_eq!(
        text(&nested(
            &hooks.pre[0],
            &["additional_args", "headers", "Authorization"]
        )),
        "Bearer None"
    );

    // a supplied client's key is the one the async pre call sees, and the source is never asked
    let mut supplied = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered());
    supplied.api_key = Some(SecretString::from("sk-supplied"));
    let caller = ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered());
    let seen = caller.clone();
    let parts = Parts::new().with_client(supplied.handle());
    let mut params = object("{}");
    let mut hooks = super::recorder();
    completion(
        caller,
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert_eq!(
        text(&nested(
            &hooks.pre[0],
            &["additional_args", "headers", "Authorization"]
        )),
        "Bearer sk-supplied"
    );
    assert!(seen.gets().is_empty());
}

pub(super) fn azure_text_stream_dispatch() {
    let fixed = Fixed::new();
    let messages = text_messages();
    let parts = Parts::new();

    let mut params = object(r#"{"stream":true}"#);
    let mut hooks = super::recorder();
    let source = ScriptedSource::new(
        ClientKind::AzureOpenAi,
        Answer::Stream(super::support::stream_items()),
    );
    let seen = source.clone();
    let outcome = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let TextCompletionOutcome::Stream(stream) = outcome else {
        panic!("the literal true streams");
    };
    let collected = run(chunks(stream));
    assert_eq!(contents(&collected), ["Hel", "lo"]);
    let pre = &hooks.pre[0];
    assert_eq!(
        text(&nested(
            pre,
            &["additional_args", "headers", "Authorization"]
        )),
        "Bearer sk-unit"
    );
    assert_eq!(
        nested(pre, &["additional_args", "acompletion"]),
        Json::Bool(true)
    );
    assert_eq!(
        text(&nested(pre, &["additional_args", "api_base"])),
        "https://unit.test/openai/"
    );
    assert_eq!(
        nested(pre, &["additional_args", "complete_input_dict", "stream"]),
        Json::Bool(true)
    );
    assert!(hooks.post.is_empty());
    assert_eq!(data_field(&seen.creates(), "max_retries"), Json::Null);

    // a truthy value that is not the literal true stays buffered on the sync path
    let mut params = object(r#"{"stream":"true"}"#);
    let mut hooks = super::recorder();
    let source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    let seen = source.clone();
    let outcome = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert!(matches!(outcome, TextCompletionOutcome::Buffered(_)));
    assert_eq!(text(&data_field(&seen.creates(), "stream")), "true");
    assert_eq!(hooks.post.len(), 1);

    let mut params = object(r#"{"stream":1}"#);
    let mut hooks = super::recorder();
    let outcome = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert!(matches!(outcome, TextCompletionOutcome::Buffered(_)));

    // the async path streams on any truthy value
    let mut params = object(r#"{"stream":1}"#);
    let mut hooks = super::recorder();
    let outcome = completion(
        ScriptedSource::new(
            ClientKind::AsyncAzureOpenAi,
            Answer::Stream(super::support::stream_items()),
        ),
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    let TextCompletionOutcome::Stream(stream) = outcome else {
        panic!("a truthy value streams asynchronously");
    };
    assert_eq!(run(chunks(stream)).len(), 2);
    assert!(hooks.post.is_empty());
    assert_eq!(
        nested(
            &hooks.pre[0],
            &["additional_args", "complete_input_dict", "stream"]
        ),
        Json::Number(1.into())
    );

    let mut params = object(r#"{"stream":false}"#);
    let mut hooks = super::recorder();
    let outcome = completion(
        ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap();
    assert!(matches!(outcome, TextCompletionOutcome::Buffered(_)));

    // a stream request answered with a buffered body fails instead of fabricating a chunk
    let mut params = object(r#"{"stream":true}"#);
    let mut hooks = super::recorder();
    let error = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(error.message, "the answer is not a stream");
    assert_eq!(hooks.pre.len(), 1);

    let mut params = object(r#"{"stream":true}"#);
    let mut hooks = super::recorder();
    let error = completion(
        ScriptedSource::new(ClientKind::AsyncAzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(error.message, "the answer is not a stream");
    assert_eq!(hooks.pre.len(), 1);
}

pub(super) fn azure_text_client_errors() {
    let fixed = Fixed::new();
    let messages = text_messages();
    let parts = Parts::new();

    let mut params = object("{}");
    let mut hooks = super::recorder();
    let source = ScriptedSource::new(ClientKind::OpenAi, buffered());
    let seen = source.clone();
    let error = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(
        error.message,
        "azure_client is not an instance of AzureOpenAI"
    );
    assert_eq!(seen.gets().len(), 1);
    assert!(seen.creates().is_empty());
    assert_eq!(hooks.pre.len(), 1);

    // a sync client is not enough for the async path
    let mut params = object("{}");
    let mut hooks = super::recorder();
    let error = completion(
        ScriptedSource::new(ClientKind::AzureOpenAi, buffered()),
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(
        error.message,
        "azure_client is not an instance of AsyncAzureOpenAI"
    );

    // the direct sync stream runs the same guard and returns the wrapped failure
    let mut params = object(r#"{"stream":true}"#);
    let mut hooks = super::recorder();
    let error = completion(
        ScriptedSource::new(
            ClientKind::AsyncAzureOpenAi,
            Answer::Stream(super::support::stream_items()),
        ),
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(
        error.message,
        "azure_client is not an instance of AzureOpenAI"
    );
    assert!(hooks.pre.is_empty(), "the type guard runs before the log");

    // so does the async stream, with its own expected type
    let mut params = object(r#"{"stream":true}"#);
    let mut hooks = super::recorder();
    let error = completion(
        ScriptedSource::new(
            ClientKind::AzureOpenAi,
            Answer::Stream(super::support::stream_items()),
        ),
        parts.call(Some(&messages), &mut params, true),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "azure_client is not an instance of AsyncAzureOpenAI"
    );

    // a missing api version from the client seam surfaces as an internal error
    let mut params = object("{}");
    let mut hooks = super::recorder();
    let mut source = ScriptedSource::new(ClientKind::AzureOpenAi, buffered());
    source.fail_get = true;
    let seen = source.clone();
    let error = completion(
        source,
        parts.call(Some(&messages), &mut params, false),
        &fixed,
        &mut hooks,
    )
    .unwrap_err();
    assert_eq!(error.status, Some(500));
    assert_eq!(error.message, "the client custom query has no api-version");
    assert!(seen.creates().is_empty());
}
