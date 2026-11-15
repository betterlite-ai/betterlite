#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::EvalsError;
use super::openai::OpenAiEvalsConfig;
use super::transformation::{BaseEvalsAPIConfig as _, EvalsHttpResponse, EvalsRequestContext};
use super::types::{
    CreateEvalRequest, CreateRunRequest, ListEvalsParams, ListRunsParams, UpdateEvalRequest,
};
use crate::openai_base_config::{Headers, Object};

type Call<T> = Result<T, EvalsError>;
type UrlAndHeaders = (String, Headers);
type UrlHeadersBody = (String, Headers, Object);

const FIXTURE_URL: &str = "https://fixture.test";
const EVALS_BASE: &str = r#"{"api_base":"https://evals.test/base"}"#;
const RUNS_BASE: &str = r#"{"api_base":"https://runs.test"}"#;
const CREATE_NULLS_REQUEST: &str = r#"{"name":null,"data_source_config":{"type":"custom","item_schema":{"role":"string","required":false,"count":0,"note":null,"tags":[]},"include_sample_schema":false},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"},{"type":"custom","function_id":"fn-1"},{"type":"ground_truth","metric":"f1_score"}],"metadata":{"team":"ml","flag":false,"count":0,"tags":[],"note":null}}"#;
const CREATE_NULLS_BODY: &str = r#"{"data_source_config":{"type":"custom","item_schema":{"role":"string","required":false,"count":0,"note":null,"tags":[]},"include_sample_schema":false},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"},{"type":"custom","function_id":"fn-1"},{"type":"ground_truth","metric":"f1_score"}],"metadata":{"team":"ml","flag":false,"count":0,"tags":[],"note":null}}"#;
const CREATE_EMPTY: &str =
    r#"{"name":"","data_source_config":{"type":"logs"},"testing_criteria":[],"metadata":{}}"#;
const UPDATE_NULLS_REQUEST: &str = r#"{"name":null,"metadata":{"a":1,"b":null}}"#;
const UPDATE_NULLS_BODY: &str = r#"{"metadata":{"a":1,"b":null}}"#;
const UPDATE_NAME: &str = r#"{"name":"renamed"}"#;
const RUN_SUFFIX_REQUEST: &str = r#"{"data_source":{"type":"inline","samples":[{"input":null,"ok":false,"count":0}]},"name":null,"metadata":{"run":1,"note":null}}"#;
const RUN_SUFFIX_BODY: &str = r#"{"data_source":{"type":"inline","samples":[{"input":null,"ok":false,"count":0}]},"metadata":{"run":1,"note":null}}"#;
const RUN_EMPTY: &str = r#"{"data_source":{}}"#;
const RUN_DATASET: &str =
    r#"{"data_source":{"type":"dataset","dataset_id":"ds-1","extra":null,"flags":[false,0,[]]}}"#;
const RUN_INLINE: &str = r#"{"data_source":{"type":"inline","samples":[{"input":null,"ok":false,"count":0}]},"name":"run one","metadata":{"k":"v","n":null}}"#;
const LIST_TRUTHY_REQUEST: &str =
    r#"{"limit":5,"after":"eval_5","before":"eval_9","order":"asc","order_by":"updated_at"}"#;
const LIST_TRUTHY_QUERY: &str =
    r#"{"limit":5,"after":"eval_5","before":"eval_9","order":"asc","order_by":"updated_at"}"#;
const LIST_FALSY_REQUEST: &str =
    r#"{"limit":0,"after":"","before":"","order":null,"order_by":null}"#;
const RUNS_TRUTHY_REQUEST: &str =
    r#"{"limit":3,"after":"r1","before":"r9","order":"desc","order_by":"created_at"}"#;
const RUNS_TRUTHY_QUERY: &str = r#"{"limit":3,"after":"r1","before":"r9","order":"desc"}"#;
const RUNS_FALSY_REQUEST: &str = r#"{"limit":0,"after":"","before":""}"#;

fn request_context<'call>(
    params: &'call Object,
    headers: &'call Headers,
) -> EvalsRequestContext<'call> {
    EvalsRequestContext {
        litellm_params: params,
        headers,
    }
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn value(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn parse<T>(text: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(text).unwrap()
}

fn reply<'call>(text: &'call str, headers: &'call Headers) -> EvalsHttpResponse<'call> {
    EvalsHttpResponse {
        status_code: 200,
        headers,
        text,
    }
}

fn header<'headers>(headers: &'headers Headers, name: &str) -> &'headers str {
    headers.get(name).unwrap()
}

fn assert_json_failure<T>(result: &Result<T, EvalsError>) {
    assert!(matches!(result, Err(EvalsError::ReplyJson { .. })));
}

fn assert_model_failure<T>(result: &Result<T, EvalsError>) {
    assert!(matches!(result, Err(EvalsError::ReplyModel { .. })));
}

fn assert_segment_error<T>(result: &Result<T, EvalsError>) {
    assert!(matches!(result, Err(EvalsError::PathSegment { .. })));
}

fn create_eval(request: &CreateEvalRequest, params: &Object, headers: &Headers) -> Call<Object> {
    let context = request_context(params, headers);
    OpenAiEvalsConfig::default().transform_create_eval_request(request, &context)
}

fn update_eval(id: &str, request: &UpdateEvalRequest, headers: &Headers) -> Call<UrlHeadersBody> {
    let params = Object::new();
    let context = request_context(&params, headers);
    let config = OpenAiEvalsConfig::default();
    config.transform_update_eval_request(id, request, FIXTURE_URL, &context)
}

fn create_run(id: &str, request: &CreateRunRequest, params: &Object) -> Call<(String, Object)> {
    let headers = Headers::new();
    let context = request_context(params, &headers);
    OpenAiEvalsConfig::default().transform_create_run_request(id, request, &context)
}

fn list_evals(params: &Object, list: &ListEvalsParams) -> Call<(String, Object)> {
    let headers = Headers::new();
    let context = request_context(params, &headers);
    OpenAiEvalsConfig::default().transform_list_evals_request(list, &context)
}

fn list_runs(eval_id: &str, params: &Object, list: &ListRunsParams) -> Call<(String, Object)> {
    let headers = Headers::new();
    let context = request_context(params, &headers);
    OpenAiEvalsConfig::default().transform_list_runs_request(eval_id, list, &context)
}

fn get_eval(eval_id: &str, headers: &Headers) -> Call<UrlAndHeaders> {
    let params = Object::new();
    let context = request_context(&params, headers);
    OpenAiEvalsConfig::default().transform_get_eval_request(eval_id, FIXTURE_URL, &context)
}

fn delete_eval(eval_id: &str, headers: &Headers) -> Call<UrlAndHeaders> {
    let params = Object::new();
    let context = request_context(&params, headers);
    OpenAiEvalsConfig::default().transform_delete_eval_request(eval_id, FIXTURE_URL, &context)
}

fn cancel_eval(eval_id: &str, headers: &Headers) -> Call<UrlHeadersBody> {
    let params = Object::new();
    let context = request_context(&params, headers);
    OpenAiEvalsConfig::default().transform_cancel_eval_request(eval_id, FIXTURE_URL, &context)
}

fn get_run(eval_id: &str, run_id: &str, headers: &Headers) -> Call<UrlAndHeaders> {
    let params = Object::new();
    let context = request_context(&params, headers);
    OpenAiEvalsConfig::default().transform_get_run_request(eval_id, run_id, FIXTURE_URL, &context)
}

fn delete_run(eval_id: &str, run_id: &str, headers: &Headers) -> Call<UrlHeadersBody> {
    let params = Object::new();
    let context = request_context(&params, headers);
    let config = OpenAiEvalsConfig::default();
    config.transform_delete_run_request(eval_id, run_id, FIXTURE_URL, &context)
}

fn cancel_run(eval_id: &str, run_id: &str, headers: &Headers) -> Call<UrlHeadersBody> {
    let params = Object::new();
    let context = request_context(&params, headers);
    let config = OpenAiEvalsConfig::default();
    config.transform_cancel_run_request(eval_id, run_id, FIXTURE_URL, &context)
}

fn create_eval_drops_only_top_level_nulls() {
    let body = create_eval(
        &parse(CREATE_NULLS_REQUEST),
        &Object::new(),
        &Headers::new(),
    )
    .unwrap();
    assert_eq!(Value::Object(body), value(CREATE_NULLS_BODY));
}

fn create_eval_keeps_empty_values() {
    let body = create_eval(&parse(CREATE_EMPTY), &Object::new(), &Headers::new()).unwrap();
    assert_eq!(Value::Object(body), value(CREATE_EMPTY));
}

fn update_eval_drops_only_top_level_nulls() {
    let headers = Headers::new();
    let (url, sent, body) = update_eval("eval/1", &parse(UPDATE_NULLS_REQUEST), &headers).unwrap();
    assert_eq!(
        (url.as_str(), sent),
        ("https://fixture.test/v1/evals/eval%2F1", headers)
    );
    assert_eq!(Value::Object(body), value(UPDATE_NULLS_BODY));
}

fn update_eval_keeps_name() {
    let headers = Headers::new();
    let (url, sent, body) = update_eval("eval-2", &parse(UPDATE_NAME), &headers).unwrap();
    assert_eq!(
        (url.as_str(), sent),
        ("https://fixture.test/v1/evals/eval-2", headers)
    );
    assert_eq!(Value::Object(body), value(UPDATE_NAME));
}

fn create_run_body_keeps_api_base_suffix() {
    let params: Object = object(r#"{"api_base":"https://runs.test/v1"}"#);
    let (url, body) = create_run("eval-3", &parse(RUN_SUFFIX_REQUEST), &params).unwrap();
    assert_eq!(url, "https://runs.test/v1/v1/evals/eval-3/runs");
    assert_eq!(Value::Object(body), value(RUN_SUFFIX_BODY));
}

fn create_run_body_defaults_base_for_empty_params() {
    let (url, body) = create_run("eval/4", &parse(RUN_EMPTY), &Object::new()).unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals/eval%2F4/runs");
    assert_eq!(Value::Object(body), value(RUN_EMPTY));
}

fn list_evals_forwards_truthy_params() {
    let params: Object = object(EVALS_BASE);
    let (url, query) = list_evals(&params, &parse(LIST_TRUTHY_REQUEST)).unwrap();
    assert_eq!(url, "https://evals.test/base/v1/evals");
    assert_eq!(Value::Object(query), value(LIST_TRUTHY_QUERY));
}

fn list_evals_drops_falsy_params() {
    let params: Object = object(r#"{"api_base":""}"#);
    let (url, query) = list_evals(&params, &parse(LIST_FALSY_REQUEST)).unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals");
    assert_eq!(Value::Object(query), value("{}"));
}

fn list_evals_ignores_unknown_keys_and_null_base() {
    let params: Object = object(r#"{"api_base":null}"#);
    let (url, query) = list_evals(&params, &parse(r#"{"limit":2,"model":"o3"}"#)).unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals");
    assert_eq!(Value::Object(query), value(r#"{"limit":2}"#));
}

fn list_evals_with_no_params_carries_no_query() {
    let (_, query) = list_evals(&Object::new(), &parse("{}")).unwrap();
    assert_eq!(Value::Object(query), value("{}"));
}

fn case_eval_create_update() {
    create_eval_drops_only_top_level_nulls();
    create_eval_keeps_empty_values();
    update_eval_drops_only_top_level_nulls();
    update_eval_keeps_name();
    create_run_body_keeps_api_base_suffix();
    create_run_body_defaults_base_for_empty_params();
}

fn case_eval_list_pagination() {
    list_evals_forwards_truthy_params();
    list_evals_drops_falsy_params();
    list_evals_ignores_unknown_keys_and_null_base();
    list_evals_with_no_params_carries_no_query();
}

fn get_eval_encodes_id_and_keeps_headers() {
    let headers = Headers::from([("X-Kept".to_owned(), "yes".to_owned())]);
    let (url, sent) = get_eval("eval/1", &headers).unwrap();
    assert_eq!(
        (url.as_str(), sent),
        ("https://fixture.test/v1/evals/eval%2F1", headers)
    );
}

fn delete_eval_uses_caller_headers() {
    let headers = Headers::new();
    let (url, sent) = delete_eval("ev.al-1", &headers).unwrap();
    assert_eq!(
        (url.as_str(), sent),
        ("https://fixture.test/v1/evals/ev.al-1", headers)
    );
}

fn cancel_eval_appends_cancel_with_empty_body() {
    let headers = Headers::from([("X-Kept".to_owned(), "cancel".to_owned())]);
    let (url, sent, body) = cancel_eval("ev al", &headers).unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/ev%20al/cancel");
    assert_eq!((sent, Value::Object(body)), (headers, value("{}")));
}

fn eval_id_segments_are_checked() {
    let headers = Headers::new();
    assert_eq!(
        get_eval("", &headers).unwrap().0,
        "https://fixture.test/v1/evals"
    );
    assert_segment_error(&get_eval(".", &headers));
    assert_segment_error(&delete_eval("..", &headers));
    let (url, ..) = cancel_eval("a b/c?d#e", &headers).unwrap();
    assert_eq!(
        url,
        "https://fixture.test/v1/evals/a%20b%2Fc%3Fd%23e/cancel"
    );
}

fn case_eval_get_delete_cancel() {
    get_eval_encodes_id_and_keeps_headers();
    delete_eval_uses_caller_headers();
    cancel_eval_appends_cancel_with_empty_body();
    eval_id_segments_are_checked();
}

fn create_run_dataset_body_uses_default_base() {
    let (url, body) = create_run("eval-1", &parse(RUN_DATASET), &Object::new()).unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals/eval-1/runs");
    assert_eq!(Value::Object(body), value(RUN_DATASET));
}

fn create_run_inline_body_uses_api_base() {
    let params: Object = object(RUNS_BASE);
    let (url, body) = create_run("eval-2", &parse(RUN_INLINE), &params).unwrap();
    assert_eq!(url, "https://runs.test/v1/evals/eval-2/runs");
    assert_eq!(Value::Object(body), value(RUN_INLINE));
}

fn create_run_empty_base_and_id_checks() {
    let request: CreateRunRequest = parse(RUN_EMPTY);
    let (url, _) = create_run("eval/1", &request, &object(r#"{"api_base":""}"#)).unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals/eval%2F1/runs");
    assert_segment_error(&create_run("", &request, &Object::new()));
}

fn case_run_create_data_source() {
    create_run_dataset_body_uses_default_base();
    create_run_inline_body_uses_api_base();
    create_run_empty_base_and_id_checks();
}

fn list_runs_forwards_truthy_params_without_order_by() {
    let params: Object = object(RUNS_BASE);
    let (url, query) = list_runs("eval-1", &params, &parse(RUNS_TRUTHY_REQUEST)).unwrap();
    assert_eq!(url, "https://runs.test/v1/evals/eval-1/runs");
    assert_eq!(Value::Object(query), value(RUNS_TRUTHY_QUERY));
}

fn list_runs_drops_falsy_params() {
    let params: Object = object(RUNS_BASE);
    let (_, query) = list_runs("eval-2", &params, &parse(RUNS_FALSY_REQUEST)).unwrap();
    assert_eq!(Value::Object(query), value("{}"));
}

fn list_runs_keeps_v1_base_and_checks_id() {
    let params: Object = object(r#"{"api_base":"https://runs.test/v1"}"#);
    let (url, _) = list_runs("eval/2", &params, &parse("{}")).unwrap();
    assert_eq!(url, "https://runs.test/v1/v1/evals/eval%2F2/runs");
    assert_segment_error(&list_runs("", &params, &parse("{}")));
}

fn case_run_list_pagination() {
    list_runs_forwards_truthy_params_without_order_by();
    list_runs_drops_falsy_params();
    list_runs_keeps_v1_base_and_checks_id();
}

fn get_run_encodes_both_ids() {
    let headers = Headers::from([("X-Kept".to_owned(), "a".to_owned())]);
    let (url, sent) = get_run("eval/1", "run 1", &headers).unwrap();
    assert_eq!(
        (url.as_str(), sent),
        (
            "https://fixture.test/v1/evals/eval%2F1/runs/run%201",
            headers
        )
    );
}

fn delete_run_sends_empty_body() {
    let headers = Headers::new();
    let (url, sent, body) = delete_run("eval-1", "run-1", &headers).unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/eval-1/runs/run-1");
    assert_eq!((sent, Value::Object(body)), (headers, value("{}")));
}

fn cancel_run_appends_cancel_with_empty_body() {
    let headers = Headers::new();
    let (url, sent, body) = cancel_run("eval-1", "run/1", &headers).unwrap();
    assert_eq!(
        url,
        "https://fixture.test/v1/evals/eval-1/runs/run%2F1/cancel"
    );
    assert_eq!((sent, Value::Object(body)), (headers, value("{}")));
}

fn run_id_segments_are_checked() {
    let headers = Headers::new();
    assert_segment_error(&get_run("eval-1", ".", &headers));
    assert_segment_error(&cancel_run("eval-1", "", &headers));
}

fn case_run_get_cancel_delete() {
    get_run_encodes_both_ids();
    delete_run_sends_empty_body();
    cancel_run_appends_cancel_with_empty_body();
    run_id_segments_are_checked();
}

fn malformed_json_body_replies() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    assert_json_failure(
        &config.transform_create_eval_response(&reply(
            r#"{"id": "eval-1", "created_at": 1"#,
            &headers,
        )),
    );
    assert_json_failure(&config.transform_get_eval_response(&reply("not json", &headers)));
    assert_json_failure(&config.transform_cancel_run_response(&reply("", &headers)));
    assert_json_failure(
        &config.transform_delete_run_response(&reply(r#"{"run_id": "run-1""#, &headers)),
    );
}

fn malformed_model_replies() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    assert_model_failure(&config.transform_list_evals_response(&reply(
        r#"{"object": "list", "data": [{"object": "eval"}]}"#,
        &headers,
    )));
    assert_model_failure(&config.transform_update_eval_response(&reply(r#"{"id": 5}"#, &headers)));
    assert_model_failure(
        &config.transform_delete_eval_response(&reply(r#"{"eval_id": "eval-1"}"#, &headers)),
    );
    assert_model_failure(&config.transform_cancel_eval_response(&reply(
        r#"{"id": "eval-1", "status": "queued"}"#,
        &headers,
    )));
    assert_model_failure(&config.transform_create_run_response(&reply("[]", &headers)));
    assert_model_failure(
        &config.transform_list_runs_response(&reply(r#"{"data": [{"id": "run-1"}]}"#, &headers)),
    );
    assert_model_failure(&config.transform_get_run_response(&reply(
        r#"{"id": "run-1", "status": "unknown", "created_at": 1, "data_source": {}, "eval_id": "e"}"#,
        &headers,
    )));
}

fn case_malformed_json_error() {
    malformed_json_body_replies();
    malformed_model_replies();
}

fn auth_prefers_explicit_call_key() {
    let params: Object = object(r#"{"api_key":"sk-explicit"}"#);
    let headers = Headers::from([
        ("Authorization".to_owned(), "Bearer old".to_owned()),
        ("X-Existing".to_owned(), "1".to_owned()),
    ]);
    let config = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("sk-global")),
        openai_key: Some(SecretString::from("sk-openai")),
        env_api_key: Some(SecretString::from("sk-env")),
    };
    let sent = config.validate_environment(headers, Some(&params)).unwrap();
    assert_eq!(header(&sent, "Authorization"), "Bearer sk-explicit");
    assert_eq!(header(&sent, "Content-Type"), "application/json");
    assert_eq!(header(&sent, "X-Existing"), "1");
}

fn auth_falls_back_through_sources() {
    let config = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("sk-global")),
        openai_key: Some(SecretString::from("sk-openai")),
        env_api_key: Some(SecretString::from("sk-env")),
    };
    let from_global = config.validate_environment(Headers::new(), None).unwrap();
    assert_eq!(header(&from_global, "Authorization"), "Bearer sk-global");
    let openai_only = OpenAiEvalsConfig {
        api_key: None,
        openai_key: Some(SecretString::from("sk-openai")),
        env_api_key: Some(SecretString::from("sk-env")),
    };
    let from_openai = openai_only
        .validate_environment(Headers::new(), None)
        .unwrap();
    assert_eq!(header(&from_openai, "Authorization"), "Bearer sk-openai");
    let env_only = OpenAiEvalsConfig {
        api_key: None,
        openai_key: None,
        env_api_key: Some(SecretString::from("sk-env")),
    };
    let from_env = env_only.validate_environment(Headers::new(), None).unwrap();
    assert_eq!(header(&from_env, "Authorization"), "Bearer sk-env");
}

fn auth_treats_empty_values_as_unset() {
    let config = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("sk-global")),
        openai_key: Some(SecretString::from("sk-openai")),
        env_api_key: None,
    };
    let empty_params: Object = object(r#"{"api_key":""}"#);
    let sent = config
        .validate_environment(Headers::new(), Some(&empty_params))
        .unwrap();
    assert_eq!(header(&sent, "Authorization"), "Bearer sk-global");
    let blank_secret = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("")),
        openai_key: Some(SecretString::from("sk-openai")),
        env_api_key: None,
    };
    let from_openai = blank_secret
        .validate_environment(Headers::new(), None)
        .unwrap();
    assert_eq!(header(&from_openai, "Authorization"), "Bearer sk-openai");
    assert!(matches!(
        OpenAiEvalsConfig::default().validate_environment(Headers::new(), None),
        Err(EvalsError::ApiKeyRequired)
    ));
}

fn case_eval_auth_required() {
    auth_prefers_explicit_call_key();
    auth_falls_back_through_sources();
    auth_treats_empty_values_as_unset();
}

fn case_eval_url_literal_append() {
    let config = OpenAiEvalsConfig::default();
    let url = |api_base, endpoint, eval_id| config.get_complete_url(api_base, endpoint, eval_id);
    assert_eq!(
        url(None, "evals", None).unwrap(),
        "https://api.openai.com/v1/evals"
    );
    assert_eq!(url(Some(""), "evals", None).unwrap(), "/v1/evals");
    assert_eq!(
        url(Some("https://x.test/"), "evals", None).unwrap(),
        "https://x.test//v1/evals"
    );
    assert_eq!(
        url(Some("https://x.test/v1"), "evals", None).unwrap(),
        "https://x.test/v1/v1/evals"
    );
    assert_eq!(
        url(Some("https://x.test/base?q=1"), "evals", None).unwrap(),
        "https://x.test/base?q=1/v1/evals"
    );
    assert_eq!(
        url(None, "evals", Some("eval/1")).unwrap(),
        "https://api.openai.com/v1/evals/eval%2F1"
    );
    assert_eq!(
        url(None, "evals", Some("x y")).unwrap(),
        "https://api.openai.com/v1/evals/x%20y"
    );
    assert_eq!(
        url(None, "runs", None).unwrap(),
        "https://api.openai.com/v1/runs"
    );
    assert_eq!(
        url(None, "evals", Some("")).unwrap(),
        "https://api.openai.com/v1/evals"
    );
    assert!(matches!(
        url(None, "evals", Some(".")),
        Err(EvalsError::PathSegment { .. })
    ));
}

#[rstest]
#[case::eval_create_update(0)]
#[case::eval_list_pagination(1)]
#[case::eval_get_delete_cancel(2)]
#[case::run_create_data_source(3)]
#[case::run_list_pagination(4)]
#[case::run_get_cancel_delete(5)]
#[case::malformed_json_error(6)]
#[case::eval_auth_required(7)]
#[case::eval_url_literal_append(8)]
fn evals_cases(#[case] case: u8) {
    match case {
        0 => case_eval_create_update(),
        1 => case_eval_list_pagination(),
        2 => case_eval_get_delete_cancel(),
        3 => case_run_create_data_source(),
        4 => case_run_list_pagination(),
        5 => case_run_get_cancel_delete(),
        6 => case_malformed_json_error(),
        7 => case_eval_auth_required(),
        _ => case_eval_url_literal_append(),
    }
}

fn credential_identity_stays_with_its_config() {
    let identity_a = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("key-a")),
        openai_key: Some(SecretString::from("openai-key-a")),
        env_api_key: None,
    };
    let identity_b = OpenAiEvalsConfig {
        api_key: None,
        openai_key: None,
        env_api_key: Some(SecretString::from("key-b")),
    };
    let headers_a = identity_a
        .validate_environment(Headers::new(), None)
        .unwrap();
    let headers_b = identity_b
        .validate_environment(Headers::new(), None)
        .unwrap();
    assert_eq!(header(&headers_a, "Authorization"), "Bearer key-a");
    assert_eq!(header(&headers_b, "Authorization"), "Bearer key-b");
    assert!(!headers_a.values().any(|value| value.contains("key-b")));
    assert!(!headers_b.values().any(|value| value.contains("key-a")));
    let again_a = identity_a
        .validate_environment(Headers::new(), None)
        .unwrap();
    assert_eq!(again_a, headers_a);
}

fn call_params_override_without_mutating_config() {
    let identity = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("key-a")),
        openai_key: Some(SecretString::from("openai-key-a")),
        env_api_key: None,
    };
    let call_a: Object = object(r#"{"api_key":"call-a"}"#);
    let call_b: Object = object(r#"{"api_key":"call-b"}"#);
    assert_eq!(
        header(
            &identity
                .validate_environment(Headers::new(), Some(&call_a))
                .unwrap(),
            "Authorization"
        ),
        "Bearer call-a"
    );
    assert_eq!(
        header(
            &identity
                .validate_environment(Headers::new(), Some(&call_b))
                .unwrap(),
            "Authorization"
        ),
        "Bearer call-b"
    );
}

#[test]
fn openai_evals_credential_identity_isolation() {
    credential_identity_stays_with_its_config();
    call_params_override_without_mutating_config();
}

#[test]
fn get_error_class_reports_the_provider_message() {
    let error = OpenAiEvalsConfig::default().get_error_class(
        "slow down",
        429,
        &Headers::from([("retry-after".to_owned(), "3".to_owned())]),
    );
    assert!(matches!(error, EvalsError::Provider { .. }));
    assert_eq!(error.to_string(), "slow down");
}

#[test]
fn openai_evals_secret_safe_failure() {
    let config = OpenAiEvalsConfig {
        api_key: Some(SecretString::from("fixture-secret")),
        openai_key: Some(SecretString::from("fixture-openai-secret")),
        env_api_key: Some(SecretString::from("fixture-env-secret")),
    };
    let debug = format!("{config:?}");
    assert!(debug.contains("OpenAiEvalsConfig"));
    assert!(!debug.contains("fixture-secret"));
    assert!(!debug.contains("fixture-openai-secret"));
    assert!(!debug.contains("fixture-env-secret"));
    let failure = OpenAiEvalsConfig::default().validate_environment(Headers::new(), None);
    let rendered = match failure {
        Err(error) => format!("{error:?}"),
        Ok(_) => panic!("the empty config must fail without a key"),
    };
    assert!(rendered.contains("ApiKeyRequired"));
    assert!(!rendered.contains("fixture-secret"));
}
