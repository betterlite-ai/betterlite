#![cfg(test)]

use std::error::Error;
use std::sync::Mutex;

use betterlite_error::proxy_model_not_found::ProxyModelNotFoundError;
use betterlite_models::controls::call_params::TRUSTED_CREDENTIALS_KEY;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::{
    CredentialLookupError, CredentialSource, DEFAULT_OPERATION_CONTEXT, get_credentials_for_model,
    prepare_data_with_credentials, trusted_snapshot,
};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct FakeFailure(String);

struct FakeSource {
    answer: Result<Option<JsonObject>, FakeFailure>,
    asked: Mutex<Vec<(String, Option<String>)>>,
}

impl CredentialSource for FakeSource {
    type Error = FakeFailure;

    fn credentials_with_provider(
        &self,
        model_id: &str,
        team_id: Option<&str>,
    ) -> Result<Option<JsonObject>, FakeFailure> {
        let mut asked = self.asked.lock().unwrap();
        asked.push((model_id.to_owned(), team_id.map(str::to_owned)));
        match &self.answer {
            Ok(answer) => Ok(answer.clone()),
            Err(error) => Err(FakeFailure(error.0.clone())),
        }
    }
}

impl FakeSource {
    fn asks(&self) -> Vec<(String, Option<String>)> {
        let asked = self.asked.lock().unwrap();
        asked.clone()
    }
}

fn object(entries: &[(&str, Json)]) -> JsonObject {
    let mut fields = JsonObject::new();
    for (name, value) in entries {
        fields.insert((*name).to_owned(), value.clone());
    }
    fields
}

fn deploying_source(answer: Option<JsonObject>) -> FakeSource {
    FakeSource {
        answer: Ok(answer),
        asked: Mutex::new(Vec::new()),
    }
}

#[test]
fn lookup_requires_the_router_before_any_model_lookup() {
    let error = get_credentials_for_model(None::<&FakeSource>, "d1", DEFAULT_OPERATION_CONTEXT)
        .unwrap_err();
    assert!(matches!(error, CredentialLookupError::RouterMissing));
    assert_eq!(error.status(), Some(500));
    assert_eq!(
        error.detail_error(),
        Some("Router not initialized. Cannot use model-based routing.")
    );
}

#[test]
fn lookup_returns_the_credentials_unchanged_and_asks_with_no_team() {
    let credentials = object(&[
        ("api_key", Json::Text("server".to_owned())),
        ("model", Json::Text("resolved".to_owned())),
        ("custom_llm_provider", Json::Text("openai".to_owned())),
    ]);
    let source = deploying_source(Some(credentials.clone()));
    let found =
        get_credentials_for_model(Some(&source), "d1", "batch input file read (rate limiting)")
            .unwrap();
    assert_eq!(found, credentials);
    assert_eq!(source.asks(), vec![("d1".to_owned(), None)]);
}

#[test]
fn lookup_missing_model_names_the_operation_context_and_never_retries() {
    let source = deploying_source(None);
    let error =
        get_credentials_for_model(Some(&source), "m", "batch input file read (rate limiting)")
            .unwrap_err();
    assert_eq!(error.status(), Some(400));
    assert_eq!(
        error.detail_error(),
        Some(
            "batch input file read (rate limiting): Invalid model name passed in model=m. \
             Call `/v1/models` to view available models for your key."
        )
    );
    let CredentialLookupError::ModelNotFound(inner) = error else {
        panic!("a null answer is the missing-model error");
    };
    assert!(!inner.retryable_with_model_read_through);
    assert_eq!(ProxyModelNotFoundError::STATUS.as_u16(), 400);
}

#[test]
fn lookup_missing_model_defaults_to_the_file_operation_context() {
    let source = deploying_source(None);
    let error =
        get_credentials_for_model(Some(&source), "m", DEFAULT_OPERATION_CONTEXT).unwrap_err();
    assert_eq!(
        error.detail_error(),
        Some(
            "file operation: Invalid model name passed in model=m. \
             Call `/v1/models` to view available models for your key."
        )
    );
}

#[test]
fn lookup_keeps_a_port_failure_apart_and_as_the_source() {
    let source = FakeSource {
        answer: Err(FakeFailure("the router is down".to_owned())),
        asked: Mutex::new(Vec::new()),
    };
    let error =
        get_credentials_for_model(Some(&source), "m", DEFAULT_OPERATION_CONTEXT).unwrap_err();
    assert_eq!(error.status(), None);
    assert_eq!(error.detail_error(), None);
    assert_eq!(
        error.to_string(),
        "the credential source failed to resolve model `m`"
    );
    let carried = Error::source(&error).map(ToString::to_string);
    assert_eq!(carried.as_deref(), Some("the router is down"));
    let CredentialLookupError::Source {
        model_id,
        source: cause,
    } = error
    else {
        panic!("a port failure keeps its own variant");
    };
    assert_eq!(model_id, "m");
    assert_eq!(cause.to_string(), "the router is down");
}

#[test]
fn merge_writes_credentials_over_data_and_keeps_key_order() {
    let mut data = object(&[
        ("api_key", Json::Text("caller".to_owned())),
        ("x", Json::Number(Number::from(1_u8))),
        ("custom_llm_provider", Json::Text("azure".to_owned())),
    ]);
    let credentials = object(&[
        ("api_key", Json::Text("server".to_owned())),
        ("model", Json::Text("resolved".to_owned())),
        ("custom_llm_provider", Json::Text("openai".to_owned())),
    ]);
    let snapshot = prepare_data_with_credentials(&mut data, &credentials, None, false);
    assert!(snapshot.is_none());
    let expected = object(&[
        ("api_key", Json::Text("server".to_owned())),
        ("x", Json::Number(Number::from(1_u8))),
        ("model", Json::Text("resolved".to_owned())),
    ]);
    assert_eq!(data, expected);
    assert_eq!(
        data.keys().map(String::as_str).collect::<Vec<_>>(),
        ["api_key", "x", "model"]
    );
}

#[test]
fn merge_removes_the_callers_own_custom_llm_provider() {
    let mut data = object(&[
        ("custom_llm_provider", Json::Text("azure".to_owned())),
        ("x", Json::Number(Number::from(1_u8))),
    ]);
    assert!(prepare_data_with_credentials(&mut data, &object(&[]), None, false).is_none());
    assert!(!data.contains_key("custom_llm_provider"));
    assert_eq!(data.keys().map(String::as_str).collect::<Vec<_>>(), ["x"]);
}

#[rstest]
#[case::given(Some("new"), Json::Text("new".to_owned()))]
#[case::empty(Some(""), Json::Text(String::new()))]
#[case::absent(None, Json::Text("old".to_owned()))]
fn merge_sets_the_file_id_only_when_one_is_given(
    #[case] file_id: Option<&str>,
    #[case] expected: Json,
) {
    let mut data = object(&[("file_id", Json::Text("old".to_owned()))]);
    assert!(prepare_data_with_credentials(&mut data, &object(&[]), file_id, false).is_none());
    assert_eq!(data.get("file_id"), Some(&expected));
}

#[test]
fn merge_returns_the_snapshot_when_it_takes_one() {
    let mut data = object(&[("x", Json::Number(Number::from(1_u8)))]);
    let credentials = object(&[("api_key", Json::Text("server".to_owned()))]);
    let snapshot = prepare_data_with_credentials(&mut data, &credentials, None, true).unwrap();
    assert_eq!(snapshot.values["api_key"].expose_secret(), "server");
    assert!(!data.contains_key(TRUSTED_CREDENTIALS_KEY));
}

#[test]
fn merge_drops_the_callers_snapshot_key_when_it_takes_one() {
    let mut data = object(&[(
        TRUSTED_CREDENTIALS_KEY,
        Json::Object(object(&[("api_key", Json::Text("caller".to_owned()))])),
    )]);
    let credentials = object(&[("api_key", Json::Text("server".to_owned()))]);
    let snapshot = prepare_data_with_credentials(&mut data, &credentials, None, true).unwrap();
    assert!(!data.contains_key(TRUSTED_CREDENTIALS_KEY));
    assert_eq!(snapshot.values["api_key"].expose_secret(), "server");
}

#[test]
fn merge_leaves_the_callers_snapshot_key_when_it_takes_none() {
    let mut data = object(&[(
        TRUSTED_CREDENTIALS_KEY,
        Json::Object(object(&[("api_key", Json::Text("caller".to_owned()))])),
    )]);
    let credentials = object(&[("api_key", Json::Text("server".to_owned()))]);
    let snapshot = prepare_data_with_credentials(&mut data, &credentials, None, false);
    assert!(snapshot.is_none());
    assert!(data.contains_key(TRUSTED_CREDENTIALS_KEY));
}

#[test]
fn snapshot_keeps_text_credentials_and_skips_objects_lists_and_numbers() {
    let credentials = object(&[
        ("api_key", Json::Text("server".to_owned())),
        ("model", Json::Text("resolved".to_owned())),
        ("custom_llm_provider", Json::Text("openai".to_owned())),
        ("region", Json::Text("us-east-1".to_owned())),
        ("vertex_credentials", Json::Object(object(&[]))),
        ("aws_session_tags", Json::List(Vec::new())),
        ("bedrock_tags", Json::List(Vec::new())),
        ("enabled", Json::Bool(true)),
        ("timeout", Json::Number(Number::from(5_u8))),
        ("nothing", Json::Null),
    ]);
    let snapshot = trusted_snapshot(&credentials);
    let names: Vec<&str> = snapshot.values.keys().map(String::as_str).collect();
    assert_eq!(names, ["api_key", "custom_llm_provider", "model", "region"]);
    assert_eq!(snapshot.values["model"].expose_secret(), "resolved");
}

#[test]
fn snapshot_keeps_the_provider_that_the_merge_removes_from_data() {
    let mut data = object(&[]);
    let credentials = object(&[
        ("api_key", Json::Text("server".to_owned())),
        ("custom_llm_provider", Json::Text("openai".to_owned())),
    ]);
    let snapshot = prepare_data_with_credentials(&mut data, &credentials, None, true).unwrap();
    assert!(!data.contains_key("custom_llm_provider"));
    assert_eq!(
        snapshot.values["custom_llm_provider"].expose_secret(),
        "openai"
    );
}

#[test]
fn snapshot_does_not_follow_later_changes_to_credentials_or_data() {
    let mut credentials = object(&[("api_key", Json::Text("server".to_owned()))]);
    let mut data = object(&[]);
    let snapshot = prepare_data_with_credentials(&mut data, &credentials, None, true).unwrap();
    credentials.insert("api_key".to_owned(), Json::Text("changed".to_owned()));
    data.insert("api_key".to_owned(), Json::Text("changed".to_owned()));
    assert_eq!(snapshot.values["api_key"].expose_secret(), "server");
}
