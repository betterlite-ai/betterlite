#![cfg(test)]

use std::convert::Infallible;
use std::fs;
use std::path::Path;

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::error::{PromptCompileError, PromptFetchError};
use super::fetch::read_response;
use super::initializer::{
    GenericPromptInitializerError, prompt_initializer, prompt_initializer_registry,
};
use super::manager::GenericPromptManager;
use super::request::{PromptApiResponse, query_pairs};
use super::test_support::{
    FakeFailure, FakeOutcome, FakeTransport, manager_with, settings, system_content,
    template_manager, variables,
};
use super::variables::apply_variables;
use crate::decisions::Json;
use crate::prompt_contracts::{
    PromptAsyncCompileRequest, PromptCompileRequest, PromptManagementClient, PromptParams,
};
use crate::prompt_spec::{PromptSpec, SupportedPromptIntegrations};

fn spec(litellm_params: &str) -> PromptSpec {
    let text = format!(r#"{{"prompt_id":"welcome","litellm_params":{litellm_params}}}"#);
    serde_json::from_str(&text).unwrap()
}

fn sync_request<'req>(
    prompt_id: Option<&'req str>,
    prompt_variables: Option<&'req PromptParams>,
    dynamic: &'req Json,
) -> PromptCompileRequest<'req> {
    PromptCompileRequest {
        prompt_id,
        prompt_spec: None,
        prompt_variables,
        dynamic_callback_params: dynamic,
        prompt_label: None,
        prompt_version: None,
    }
}

fn header<'headers>(
    headers: &'headers [(&'static str, SecretString)],
    name: &str,
) -> Option<&'headers str> {
    headers
        .iter()
        .find(|(header_name, _)| *header_name == name)
        .map(|(_, value)| value.expose_secret())
}

#[test]
fn manager_keeps_its_settings_and_trims_the_api_base() {
    let mut with_prompt = settings(Some("key"));
    with_prompt.api_base = "https://prompts.example.com///".to_owned();
    with_prompt.prompt_id = Some("welcome".to_owned());
    let (manager, _) = manager_with(FakeOutcome::Failure, with_prompt);
    assert_eq!(manager.api.settings.api_base, "https://prompts.example.com");
    assert_eq!(manager.api.settings.timeout.as_secs(), 30);
    assert_eq!(manager.api.settings.prompt_id.as_deref(), Some("welcome"));
    assert_eq!(
        manager
            .api
            .settings
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("key")
    );
}

#[test]
fn integration_name_is_generic_prompt() {
    assert_eq!(
        GenericPromptManager::<FakeTransport>::INTEGRATION_NAME,
        "generic_prompt"
    );
}

#[test]
fn headers_carry_a_bearer_token_when_a_key_is_set() {
    let (manager, _) = manager_with(FakeOutcome::Failure, settings(Some("secret-key")));
    let headers = manager.api.get_headers();
    assert_eq!(header(&headers, "Authorization"), Some("Bearer secret-key"));
    assert_eq!(header(&headers, "Content-Type"), Some("application/json"));
    assert_eq!(header(&headers, "Accept"), Some("application/json"));
    assert!(!format!("{headers:?}").contains("secret-key"));
}

#[test]
fn headers_leave_out_authorization_without_a_key() {
    let (manager, _) = manager_with(FakeOutcome::Failure, settings(None));
    let headers = manager.api.get_headers();
    assert_eq!(header(&headers, "Authorization"), None);
    assert_eq!(header(&headers, "Content-Type"), Some("application/json"));
    assert_eq!(header(&headers, "Accept"), Some("application/json"));
}

#[test]
fn sync_fetch_calls_the_prompt_management_endpoint_with_the_prompt_id() {
    let (manager, requests) = template_manager();
    manager
        .api
        .fetch_prompt_from_api(Some("welcome"), None)
        .unwrap();
    let sent = requests.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].url,
        "https://prompts.example.com/beta/litellm_prompt_management"
    );
    assert_eq!(
        sent[0].query,
        vec![("prompt_id".to_owned(), "welcome".to_owned())]
    );
    assert_eq!(sent[0].timeout.as_secs(), 30);
}

#[tokio::test]
async fn async_fetch_calls_the_prompt_management_endpoint_with_the_prompt_id() {
    let (manager, requests) = template_manager();
    manager
        .api
        .async_fetch_prompt_from_api(Some("welcome"), None)
        .await
        .unwrap();
    let sent = requests.lock().unwrap();
    assert_eq!(
        sent[0].url,
        "https://prompts.example.com/beta/litellm_prompt_management"
    );
    assert_eq!(
        sent[0].query,
        vec![("prompt_id".to_owned(), "welcome".to_owned())]
    );
}

#[test]
fn sync_fetch_takes_extra_query_params_from_the_manager_settings() {
    let mut configured = settings(None);
    configured.provider_specific_query_params =
        serde_json::from_str(r#"{"workspace":"team-a","tags":["x","y"]}"#).unwrap();
    let (manager, requests) = manager_with(
        FakeOutcome::Answer {
            status: 200,
            body: "{}".to_owned(),
        },
        configured,
    );
    let ignored_spec =
        spec(r#"{"prompt_integration":"x","provider_specific_query_params":{"other":"1"}}"#);
    manager
        .api
        .fetch_prompt_from_api(Some("welcome"), Some(&ignored_spec))
        .unwrap();
    let query = requests.lock().unwrap()[0].query.clone();
    let expected: Vec<(String, String)> = [
        ("prompt_id", "welcome"),
        ("workspace", "team-a"),
        ("tags", "x"),
        ("tags", "y"),
    ]
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .to_vec();
    assert_eq!(query, expected);
}

#[tokio::test]
async fn async_fetch_takes_extra_query_params_from_the_prompt_spec() {
    let mut configured = settings(None);
    configured.provider_specific_query_params =
        serde_json::from_str(r#"{"workspace":"ignored"}"#).unwrap();
    let (manager, requests) = manager_with(
        FakeOutcome::Answer {
            status: 200,
            body: "{}".to_owned(),
        },
        configured,
    );
    let prompt_spec =
        spec(r#"{"prompt_integration":"x","provider_specific_query_params":{"other":"1"}}"#);
    manager
        .api
        .async_fetch_prompt_from_api(Some("welcome"), Some(&prompt_spec))
        .await
        .unwrap();
    let query = requests.lock().unwrap()[0].query.clone();
    assert_eq!(
        query,
        vec![
            ("prompt_id".to_owned(), "welcome".to_owned()),
            ("other".to_owned(), "1".to_owned())
        ]
    );
}

#[rstest]
#[case::overrides_prompt_id(Some("welcome"), r#"{"prompt_id":"forced"}"#, &[("prompt_id", "forced")])]
#[case::missing_prompt_id_is_empty(None, "{}", &[("prompt_id", "")])]
#[case::bool_number_null(None, r#"{"a":true,"b":12,"c":null}"#, &[("prompt_id", ""), ("a", "true"), ("b", "12"), ("c", "")])]
fn query_pairs_follow_httpx_rules(
    #[case] prompt_id: Option<&str>,
    #[case] extra_json: &str,
    #[case] expected_pairs: &[(&str, &str)],
) {
    let extra = serde_json::from_str(extra_json).unwrap();
    let pairs = query_pairs::<FakeFailure>(prompt_id, Some(&extra)).unwrap();
    let expected: Vec<(String, String)> = expected_pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(pairs, expected);
}

#[rstest]
#[case::object(r#"{"nested":{"a":1}}"#)]
#[case::list_of_lists(r#"{"nested":[[1]]}"#)]
fn query_pairs_reject_nested_values_naming_the_key(#[case] extra_json: &str) {
    let extra = serde_json::from_str(extra_json).unwrap();
    let error = query_pairs::<FakeFailure>(Some("welcome"), Some(&extra)).unwrap_err();
    assert!(matches!(error, PromptFetchError::QueryValue { key } if key == "nested"));
}

#[test]
fn fetch_without_prompt_id_or_spec_is_rejected_before_any_request() {
    let (manager, requests) = template_manager();
    let error = manager.api.fetch_prompt_from_api(None, None).unwrap_err();
    assert!(matches!(error, PromptFetchError::MissingPromptSource));
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn async_fetch_without_prompt_id_or_spec_is_rejected() {
    let (manager, _) = template_manager();
    let error = manager
        .api
        .async_fetch_prompt_from_api(None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, PromptFetchError::MissingPromptSource));
}

#[rstest]
#[case::refused(FakeOutcome::Failure, "transport")]
#[case::server_error(FakeOutcome::Answer { status: 500, body: "{}".to_owned() }, "status")]
#[case::redirect(FakeOutcome::Answer { status: 302, body: "{}".to_owned() }, "status")]
#[case::invalid_json(FakeOutcome::Answer { status: 200, body: "not json".to_owned() }, "parse")]
#[case::not_an_object(FakeOutcome::Answer { status: 200, body: "[1]".to_owned() }, "parse")]
fn sync_fetch_errors_name_the_prompt(#[case] outcome: FakeOutcome, #[case] kind: &str) {
    let (manager, _) = manager_with(outcome, settings(None));
    let error = manager
        .api
        .fetch_prompt_from_api(Some("welcome"), None)
        .unwrap_err();
    assert_fetch_error(&error, kind);
}

#[rstest]
#[case::refused(FakeOutcome::Failure, "transport")]
#[case::server_error(FakeOutcome::Answer { status: 404, body: "{}".to_owned() }, "status")]
#[case::invalid_json(FakeOutcome::Answer { status: 200, body: "{".to_owned() }, "parse")]
#[tokio::test]
async fn async_fetch_errors_name_the_prompt(#[case] outcome: FakeOutcome, #[case] kind: &str) {
    let (manager, _) = manager_with(outcome, settings(None));
    let error = manager
        .api
        .async_fetch_prompt_from_api(Some("welcome"), None)
        .await
        .unwrap_err();
    assert_fetch_error(&error, kind);
}

fn assert_fetch_error(error: &PromptFetchError<FakeFailure>, kind: &str) {
    match (kind, error) {
        ("transport", PromptFetchError::Transport { prompt_id, source }) => {
            assert_eq!(prompt_id, "welcome");
            assert_eq!(source.to_string(), "connection refused");
        }
        ("status", PromptFetchError::Status { prompt_id, .. })
        | ("parse", PromptFetchError::Parse { prompt_id, .. }) => assert_eq!(prompt_id, "welcome"),
        _ => panic!("unexpected fetch error {error:?} for {kind}"),
    }
}

#[test]
fn parsing_reads_the_template_fields() {
    let (manager, _) = template_manager();
    let parsed = manager
        .api
        .fetch_prompt_from_api(Some("welcome"), None)
        .unwrap();
    assert_eq!(parsed.prompt_template.len(), 2);
    assert_eq!(parsed.prompt_template_model.as_deref(), Some("gpt-4"));
    let optional = parsed.optional_params.unwrap();
    assert_eq!(optional.len(), 1);
    assert_eq!(
        optional.get("temperature").unwrap(),
        &serde_json::from_str::<Json>("0.7").unwrap()
    );
}

#[rstest]
#[case::empty_object("{}")]
#[case::null_fields(r#"{"prompt_template":null,"prompt_template_model":null,"prompt_template_optional_params":null}"#)]
fn parsing_defaults_a_missing_template_to_empty(#[case] body: &str) {
    let (manager, _) = manager_with(
        FakeOutcome::Answer {
            status: 200,
            body: body.to_owned(),
        },
        settings(None),
    );
    let parsed = manager
        .api
        .fetch_prompt_from_api(Some("welcome"), None)
        .unwrap();
    assert!(parsed.prompt_template.is_empty());
    assert_eq!(parsed.prompt_template_model, None);
    assert!(parsed.optional_params.is_none());
}

#[rstest]
#[case::prompt_id(Some("welcome"), None, true)]
#[case::query_params_present(
    None,
    Some(r#"{"prompt_integration":"x","provider_specific_query_params":{"a":1}}"#),
    true
)]
#[case::empty_query_params_still_count(
    None,
    Some(r#"{"prompt_integration":"x","provider_specific_query_params":{}}"#),
    true
)]
#[case::spec_without_query_params(None, Some(r#"{"prompt_integration":"x"}"#), false)]
#[case::nothing(None, None, false)]
fn should_run_prompt_management_cases(
    #[case] prompt_id: Option<&str>,
    #[case] params: Option<&str>,
    #[case] expected: bool,
) {
    let prompt_spec = params.map(spec);
    assert_eq!(
        GenericPromptManager::<FakeTransport>::should_run_prompt_management(
            prompt_id,
            prompt_spec.as_ref()
        ),
        expected
    );
}

#[test]
fn compile_fetches_then_serves_the_second_call_from_the_cache() {
    let (manager, requests) = template_manager();
    let dynamic = Json::Null;
    let request = sync_request(Some("welcome"), None, &dynamic);
    let first = manager.compile_prompt_helper(&request).unwrap();
    let second = manager.compile_prompt_helper(&request).unwrap();
    assert_eq!(first, second);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(first.prompt_template_model.as_deref(), Some("gpt-4"));
}

#[test]
fn compile_applies_variables_in_both_brace_forms_to_every_occurrence() {
    let (manager, _) = template_manager();
    let dynamic = Json::Null;
    let values = variables(&[
        ("name", Json::Text("Ada".to_owned())),
        ("place", Json::Text("Rust".to_owned())),
        ("topic", Json::Text("caches".to_owned())),
    ]);
    let request = sync_request(Some("welcome"), Some(&values), &dynamic);
    let compiled = manager.compile_prompt_helper(&request).unwrap();
    assert_eq!(
        system_content(&compiled.prompt_template),
        Some("Hello Ada, welcome to {Rust}. Ada again.")
    );
    let user = &compiled.prompt_template[1];
    assert_eq!(
        user,
        &serde_json::from_str::<Json>(r#"{"role":"user","content":"Ask about caches"}"#).unwrap()
    );
}

#[tokio::test]
async fn async_compile_fetches_caches_and_applies_variables() {
    let (manager, requests) = template_manager();
    let dynamic = Json::Null;
    let values = variables(&[("name", Json::Text("Ada".to_owned()))]);
    let request = PromptAsyncCompileRequest {
        prompt_id: Some("welcome"),
        prompt_variables: Some(&values),
        dynamic_callback_params: &dynamic,
        prompt_spec: None,
        prompt_label: None,
        prompt_version: None,
    };
    let first = manager.async_compile_prompt_helper(&request).await.unwrap();
    let second = manager.async_compile_prompt_helper(&request).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(
        system_content(&first.prompt_template),
        Some("Hello Ada, welcome to {{place}}. Ada again.")
    );
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[rstest]
#[case::refused(FakeOutcome::Failure)]
#[case::bad_status(FakeOutcome::Answer { status: 503, body: "{}".to_owned() })]
#[case::bad_json(FakeOutcome::Answer { status: 200, body: "oops".to_owned() })]
fn compile_wraps_fetch_failures_with_the_prompt_id(#[case] outcome: FakeOutcome) {
    let (manager, _) = manager_with(outcome, settings(None));
    let dynamic = Json::Null;
    let request = sync_request(Some("welcome"), None, &dynamic);
    let error = manager.compile_prompt_helper(&request).unwrap_err();
    assert!(matches!(error, PromptCompileError::Fetch { prompt_id, .. } if prompt_id == "welcome"));
    assert!(manager.prompt_cache.entries.lock().unwrap().is_empty());
}

#[tokio::test]
async fn async_compile_wraps_fetch_failures_and_caches_nothing() {
    let (manager, _) = manager_with(FakeOutcome::Failure, settings(None));
    let dynamic = Json::Null;
    let request = PromptAsyncCompileRequest {
        prompt_id: None,
        prompt_variables: None,
        dynamic_callback_params: &dynamic,
        prompt_spec: None,
        prompt_label: None,
        prompt_version: None,
    };
    let error = manager
        .async_compile_prompt_helper(&request)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PromptCompileError::Fetch {
            source: PromptFetchError::MissingPromptSource,
            ..
        }
    ));
    assert!(manager.prompt_cache.entries.lock().unwrap().is_empty());
}

#[rstest]
#[case::number(Json::Number(serde_json::Number::from(7_i32)), "7")]
#[case::bool(Json::Bool(true), "true")]
#[case::list(Json::List(vec![Json::Number(serde_json::Number::from(1_i32))]), "[1]")]
fn variables_that_are_not_text_are_written_as_json(#[case] value: Json, #[case] expected: &str) {
    let client = PromptManagementClient {
        prompt_id: None,
        prompt_template: vec![
            serde_json::from_str(r#"{"role":"user","content":"v={v}"}"#).unwrap(),
        ],
        prompt_template_model: None,
        prompt_optional_params: None,
        completed_messages: None,
    };
    let applied = apply_variables(&client, &variables(&[("v", value)])).unwrap();
    assert_eq!(
        system_content(&applied.prompt_template),
        Some(format!("v={expected}").as_str())
    );
}

#[test]
fn apply_variables_leaves_messages_without_text_content_alone() {
    let template: Vec<Json> = serde_json::from_str(
        r#"[{"role":"assistant","tool_calls":[]},{"role":"user","content":[{"type":"text","text":"{v}"}]}]"#,
    )
    .unwrap();
    let client = PromptManagementClient {
        prompt_template: template.clone(),
        prompt_id: Some("welcome".to_owned()),
        prompt_template_model: None,
        prompt_optional_params: None,
        completed_messages: None,
    };
    let applied =
        apply_variables(&client, &variables(&[("v", Json::Text("x".to_owned()))])).unwrap();
    assert_eq!(applied.prompt_template, template);
    assert_eq!(applied.prompt_id.as_deref(), Some("welcome"));
}

#[test]
fn initializer_builds_a_manager_from_the_prompt_params() {
    let params = spec(
        r#"{"prompt_integration":"generic_prompt_management","api_base":"https://prompts.example.com/","api_key":"k","prompt_id":"welcome","timeout":12,"provider_specific_query_params":{"a":"b"}}"#,
    );
    let transport = template_manager().0.api.transport;
    let manager = prompt_initializer(&params.litellm_params, &params, transport).unwrap();
    assert_eq!(manager.api.settings.api_base, "https://prompts.example.com");
    assert_eq!(manager.api.settings.timeout.as_secs(), 12);
    assert_eq!(manager.api.settings.prompt_id.as_deref(), Some("welcome"));
    assert_eq!(
        manager
            .api
            .settings
            .api_key
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("k")
    );
    assert_eq!(
        manager.api.settings.provider_specific_query_params,
        serde_json::from_str::<IndexMap<String, Json>>(r#"{"a":"b"}"#).unwrap()
    );
}

#[rstest]
#[case::absent(r#"{"prompt_integration":"g","api_base":"https://x"}"#, 30)]
#[case::null(
    r#"{"prompt_integration":"g","api_base":"https://x","timeout":null}"#,
    30
)]
#[case::zero(r#"{"prompt_integration":"g","api_base":"https://x","timeout":0}"#, 0)]
fn initializer_defaults_the_timeout_to_thirty_seconds(
    #[case] params_json: &str,
    #[case] seconds: u64,
) {
    let params = spec(params_json);
    let transport = template_manager().0.api.transport;
    let manager = prompt_initializer(&params.litellm_params, &params, transport).unwrap();
    assert_eq!(manager.api.settings.timeout.as_secs(), seconds);
    assert!(manager.api.settings.api_key.is_none());
}

#[rstest]
#[case::missing(
    r#"{"prompt_integration":"g"}"#,
    GenericPromptInitializerError::MissingApiBase
)]
#[case::empty(
    r#"{"prompt_integration":"g","api_base":""}"#,
    GenericPromptInitializerError::MissingApiBase
)]
#[case::text_timeout(
    r#"{"prompt_integration":"g","api_base":"https://x","timeout":"30"}"#,
    GenericPromptInitializerError::TimeoutType
)]
#[case::negative_timeout(
    r#"{"prompt_integration":"g","api_base":"https://x","timeout":-1}"#,
    GenericPromptInitializerError::TimeoutType
)]
#[case::fractional_timeout(
    r#"{"prompt_integration":"g","api_base":"https://x","timeout":1.5}"#,
    GenericPromptInitializerError::TimeoutType
)]
fn initializer_rejects_bad_params(
    #[case] params_json: &str,
    #[case] expected: GenericPromptInitializerError,
) {
    let params = spec(params_json);
    let transport = template_manager().0.api.transport;
    let error = prompt_initializer(&params.litellm_params, &params, transport).unwrap_err();
    assert_eq!(error, expected);
}

#[rstest]
#[case::generic(SupportedPromptIntegrations::GenericPromptManagement, true)]
#[case::dotprompt(SupportedPromptIntegrations::DotPrompt, false)]
#[case::langfuse(SupportedPromptIntegrations::Langfuse, false)]
#[case::custom(SupportedPromptIntegrations::Custom, false)]
#[case::bitbucket(SupportedPromptIntegrations::Bitbucket, false)]
#[case::gitlab(SupportedPromptIntegrations::Gitlab, false)]
#[case::phoenix(SupportedPromptIntegrations::ArizePhoenix, false)]
fn registry_maps_only_the_generic_integration(
    #[case] integration: SupportedPromptIntegrations,
    #[case] registered: bool,
) {
    let registered_fn = prompt_initializer_registry::<FakeTransport>(&integration);
    assert_eq!(registered_fn.is_some(), registered);
    if let Some(initializer) = registered_fn {
        let params = spec(r#"{"prompt_integration":"g","api_base":"https://x"}"#);
        let transport = template_manager().0.api.transport;
        let manager = initializer(&params.litellm_params, &params, transport).unwrap();
        assert_eq!(manager.api.settings.api_base, "https://x");
    }
}

#[test]
fn initializer_leaves_the_query_params_empty_when_the_spec_has_none() {
    let params =
        spec(r#"{"prompt_integration":"generic_prompt_management","api_base":"https://x"}"#);
    let transport = template_manager().0.api.transport;
    let manager = prompt_initializer(&params.litellm_params, &params, transport).unwrap();
    assert!(
        manager
            .api
            .settings
            .provider_specific_query_params
            .is_empty()
    );
}

#[rstest]
fn replays_the_prompt_api_response_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/prompt_api_response");
    let seeds = fs::read_dir(&directory).expect("the prompt_api_response corpus directory exists");
    assert!(seeds.count() >= 5);
    for entry in fs::read_dir(directory).expect("the prompt_api_response corpus directory exists") {
        let body = fs::read(entry.expect("a corpus entry is readable").path()).unwrap();
        let answer = PromptApiResponse { status: 200, body };
        let _outcome = read_response::<Infallible>(Some("welcome"), Ok(answer));
    }
}
