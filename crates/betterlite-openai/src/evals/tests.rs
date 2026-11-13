#![cfg(test)]

use rstest::rstest;
use secrecy::SecretString;
use serde_json::Value;

use super::error::EvalsError;
use super::openai::OpenAiEvalsConfig;
use super::transformation::{BaseEvalsAPIConfig, EvalsHttpResponse, EvalsRequestContext};
use super::types::{
    CreateEvalRequest, CreateRunRequest, ListEvalsParams, ListRunsParams, UpdateEvalRequest,
};
use crate::openai_base_config::{Headers, Object};

fn context<'call>(params: &'call Object, headers: &'call Headers) -> EvalsRequestContext<'call> {
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

fn assert_json_failure<T>(result: Result<T, EvalsError>) {
    assert!(matches!(result, Err(EvalsError::ReplyJson { .. })));
}

fn assert_model_failure<T>(result: Result<T, EvalsError>) {
    assert!(matches!(result, Err(EvalsError::ReplyModel { .. })));
}

fn case_create_eval() {
    let config = OpenAiEvalsConfig::default();
    let params = Object::new();
    let headers = Headers::from([("X-Unused".to_owned(), "kept".to_owned())]);
    let context = context(&params, &headers);
    let request: CreateEvalRequest = serde_json::from_str(
        r#"{"name":null,"data_source_config":{"type":"custom","item_schema":{"role":"string","required":false,"count":0,"note":null,"tags":[]},"include_sample_schema":false},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"},{"type":"custom","function_id":"fn-1"},{"type":"ground_truth","metric":"f1_score"}],"metadata":{"team":"ml","flag":false,"count":0,"tags":[],"note":null}}"#,
    )
    .unwrap();
    let body = config
        .transform_create_eval_request(&request, &context)
        .unwrap();
    assert_eq!(
        Value::Object(body),
        value(
            r#"{"data_source_config":{"type":"custom","item_schema":{"role":"string","required":false,"count":0,"note":null,"tags":[]},"include_sample_schema":false},"testing_criteria":[{"type":"llm_as_judge","model":"gpt-4o"},{"type":"custom","function_id":"fn-1"},{"type":"ground_truth","metric":"f1_score"}],"metadata":{"team":"ml","flag":false,"count":0,"tags":[],"note":null}}"#
        )
    );
    let request: CreateEvalRequest = serde_json::from_str(
        r#"{"name":"","data_source_config":{"type":"logs"},"testing_criteria":[],"metadata":{}}"#,
    )
    .unwrap();
    let body = config
        .transform_create_eval_request(&request, &context)
        .unwrap();
    assert_eq!(
        Value::Object(body),
        value(
            r#"{"name":"","data_source_config":{"type":"logs"},"testing_criteria":[],"metadata":{}}"#
        )
    );
}

fn case_update_eval() {
    let config = OpenAiEvalsConfig::default();
    let params = Object::new();
    let headers = Headers::from([
        ("X-Custom".to_owned(), "kept".to_owned()),
        ("Authorization".to_owned(), "Bearer original".to_owned()),
    ]);
    let context = context(&params, &headers);
    let request: UpdateEvalRequest =
        serde_json::from_str(r#"{"name":null,"metadata":{"a":1,"b":null}}"#).unwrap();
    let (url, sent, body) = config
        .transform_update_eval_request("eval/1", &request, "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/eval%2F1");
    assert_eq!(sent, headers);
    assert_eq!(
        Value::Object(body),
        value(r#"{"metadata":{"a":1,"b":null}}"#)
    );
    let request: UpdateEvalRequest = serde_json::from_str(r#"{"name":"renamed"}"#).unwrap();
    let (url, sent, body) = config
        .transform_update_eval_request("eval-2", &request, "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/eval-2");
    assert_eq!(sent, headers);
    assert_eq!(Value::Object(body), value(r#"{"name":"renamed"}"#));
}

fn case_create_run_body() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    let params: Object = object(r#"{"api_base":"https://runs.test/v1"}"#);
    let context = context(&params, &headers);
    let request: CreateRunRequest = serde_json::from_str(
        r#"{"data_source":{"type":"inline","samples":[{"input":null,"ok":false,"count":0}]},"name":null,"metadata":{"run":1,"note":null}}"#,
    )
    .unwrap();
    let (url, body) = config
        .transform_create_run_request("eval-3", &request, &context)
        .unwrap();
    assert_eq!(url, "https://runs.test/v1/v1/evals/eval-3/runs");
    assert_eq!(
        Value::Object(body),
        value(
            r#"{"data_source":{"type":"inline","samples":[{"input":null,"ok":false,"count":0}]},"metadata":{"run":1,"note":null}}"#
        )
    );
    let empty = Object::new();
    let context = context(&empty, &headers);
    let request: CreateRunRequest = serde_json::from_str(r#"{"data_source":{}}"#).unwrap();
    let (url, body) = config
        .transform_create_run_request("eval/4", &request, &context)
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals/eval%2F4/runs");
    assert_eq!(Value::Object(body), value(r#"{"data_source":{}}"#));
}

fn case_eval_create_update() {
    case_create_eval();
    case_update_eval();
    case_create_run_body();
}

fn case_eval_list_pagination() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    let params: Object = object(r#"{"api_base":"https://evals.test/base"}"#);
    let context = context(&params, &headers);
    let list: ListEvalsParams = serde_json::from_str(
        r#"{"limit":5,"after":"eval_5","before":"eval_9","order":"asc","order_by":"updated_at"}"#,
    )
    .unwrap();
    let (url, query) = config
        .transform_list_evals_request(&list, &context)
        .unwrap();
    assert_eq!(url, "https://evals.test/base/v1/evals");
    assert_eq!(
        Value::Object(query),
        value(
            r#"{"limit":5,"after":"eval_5","before":"eval_9","order":"asc","order_by":"updated_at"}"#
        )
    );
    let params: Object = object(r#"{"api_base":""}"#);
    let context = context(&params, &headers);
    let list: ListEvalsParams =
        serde_json::from_str(r#"{"limit":0,"after":"","before":"","order":null,"order_by":null}"#)
            .unwrap();
    let (url, query) = config
        .transform_list_evals_request(&list, &context)
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals");
    assert_eq!(Value::Object(query), value(r#"{}"#));
    let params: Object = object(r#"{"api_base":null}"#);
    let context = context(&params, &headers);
    let list: ListEvalsParams = serde_json::from_str(r#"{"limit":2,"model":"o3"}"#).unwrap();
    let (url, query) = config
        .transform_list_evals_request(&list, &context)
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals");
    assert_eq!(Value::Object(query), value(r#"{"limit":2}"#));
    let list: ListEvalsParams = serde_json::from_str(r#"{}"#).unwrap();
    let (_, query) = config
        .transform_list_evals_request(&list, &context)
        .unwrap();
    assert_eq!(Value::Object(query), value(r#"{}"#));
}

fn case_eval_get_delete_cancel() {
    let config = OpenAiEvalsConfig::default();
    let params = Object::new();
    let headers = Headers::from([("X-Kept".to_owned(), "yes".to_owned())]);
    let context = context(&params, &headers);
    let (url, sent) = config
        .transform_get_eval_request("eval/1", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/eval%2F1");
    assert_eq!(sent, headers);
    let empty = Headers::new();
    let context = context(&params, &empty);
    let (url, sent) = config
        .transform_delete_eval_request("ev.al-1", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/ev.al-1");
    assert_eq!(sent, empty);
    let cancel_headers = Headers::from([("X-Kept".to_owned(), "cancel".to_owned())]);
    let context = context(&params, &cancel_headers);
    let (url, sent, body) = config
        .transform_cancel_eval_request("ev al", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/ev%20al/cancel");
    assert_eq!(sent, cancel_headers);
    assert_eq!(Value::Object(body), value(r#"{}"#));
    let (url, _) = config
        .transform_get_eval_request("", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals");
    assert!(matches!(
        config.transform_get_eval_request(".", "https://fixture.test", &context),
        Err(EvalsError::PathSegment { .. })
    ));
    assert!(matches!(
        config.transform_delete_eval_request("..", "https://fixture.test", &context),
        Err(EvalsError::PathSegment { .. })
    ));
    let (url, _, _) = config
        .transform_cancel_eval_request("a b/c?d#e", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(
        url,
        "https://fixture.test/v1/evals/a%20b%2Fc%3Fd%23e/cancel"
    );
}

fn case_run_create_data_source() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    let params = Object::new();
    let context = context(&params, &headers);
    let request: CreateRunRequest = serde_json::from_str(
        r#"{"data_source":{"type":"dataset","dataset_id":"ds-1","extra":null,"flags":[false,0,[]]}}"#,
    )
    .unwrap();
    let (url, body) = config
        .transform_create_run_request("eval-1", &request, &context)
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals/eval-1/runs");
    assert_eq!(
        Value::Object(body),
        value(
            r#"{"data_source":{"type":"dataset","dataset_id":"ds-1","extra":null,"flags":[false,0,[]]}}"#
        )
    );
    let params: Object = object(r#"{"api_base":"https://runs.test"}"#);
    let context = context(&params, &headers);
    let request: CreateRunRequest = serde_json::from_str(
        r#"{"data_source":{"type":"inline","samples":[{"input":null,"ok":false,"count":0}]},"name":"run one","metadata":{"k":"v","n":null}}"#,
    )
    .unwrap();
    let (url, body) = config
        .transform_create_run_request("eval-2", &request, &context)
        .unwrap();
    assert_eq!(url, "https://runs.test/v1/evals/eval-2/runs");
    assert_eq!(
        Value::Object(body),
        value(
            r#"{"data_source":{"type":"inline","samples":[{"input":null,"ok":false,"count":0}]},"name":"run one","metadata":{"k":"v","n":null}}"#
        )
    );
    let params: Object = object(r#"{"api_base":""}"#);
    let context = context(&params, &headers);
    let request: CreateRunRequest = serde_json::from_str(r#"{"data_source":{}}"#).unwrap();
    let (url, _) = config
        .transform_create_run_request("eval/1", &request, &context)
        .unwrap();
    assert_eq!(url, "https://api.openai.com/v1/evals/eval%2F1/runs");
    assert!(matches!(
        config.transform_create_run_request("", &request, &context),
        Err(EvalsError::PathSegment { .. })
    ));
}

fn case_run_list_pagination() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    let params: Object = object(r#"{"api_base":"https://runs.test"}"#);
    let context = context(&params, &headers);
    let list: ListRunsParams = serde_json::from_str(
        r#"{"limit":3,"after":"r1","before":"r9","order":"desc","order_by":"created_at"}"#,
    )
    .unwrap();
    let (url, query) = config
        .transform_list_runs_request("eval-1", &list, &context)
        .unwrap();
    assert_eq!(url, "https://runs.test/v1/evals/eval-1/runs");
    assert_eq!(
        Value::Object(query),
        value(r#"{"limit":3,"after":"r1","before":"r9","order":"desc"}"#)
    );
    let list: ListRunsParams =
        serde_json::from_str(r#"{"limit":0,"after":"","before":""}"#).unwrap();
    let (_, query) = config
        .transform_list_runs_request("eval-2", &list, &context)
        .unwrap();
    assert_eq!(Value::Object(query), value(r#"{}"#));
    let params: Object = object(r#"{"api_base":"https://runs.test/v1"}"#);
    let context = context(&params, &headers);
    let list: ListRunsParams = serde_json::from_str(r#"{}"#).unwrap();
    let (url, _) = config
        .transform_list_runs_request("eval/2", &list, &context)
        .unwrap();
    assert_eq!(url, "https://runs.test/v1/v1/evals/eval%2F2/runs");
    assert!(matches!(
        config.transform_list_runs_request("", &list, &context),
        Err(EvalsError::PathSegment { .. })
    ));
}

fn case_run_get_cancel_delete() {
    let config = OpenAiEvalsConfig::default();
    let params = Object::new();
    let headers = Headers::from([("X-Kept".to_owned(), "a".to_owned())]);
    let context = context(&params, &headers);
    let (url, sent) = config
        .transform_get_run_request("eval/1", "run 1", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/eval%2F1/runs/run%201");
    assert_eq!(sent, headers);
    let empty = Headers::new();
    let context = context(&params, &empty);
    let (url, sent, body) = config
        .transform_delete_run_request("eval-1", "run-1", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(url, "https://fixture.test/v1/evals/eval-1/runs/run-1");
    assert_eq!(sent, empty);
    assert_eq!(Value::Object(body), value(r#"{}"#));
    let (url, sent, body) = config
        .transform_cancel_run_request("eval-1", "run/1", "https://fixture.test", &context)
        .unwrap();
    assert_eq!(
        url,
        "https://fixture.test/v1/evals/eval-1/runs/run%2F1/cancel"
    );
    assert_eq!(sent, empty);
    assert_eq!(Value::Object(body), value(r#"{}"#));
    assert!(matches!(
        config.transform_get_run_request("eval-1", ".", "https://fixture.test", &context),
        Err(EvalsError::PathSegment { .. })
    ));
    assert!(matches!(
        config.transform_cancel_run_request("eval-1", "", "https://fixture.test", &context),
        Err(EvalsError::PathSegment { .. })
    ));
}

fn malformed_json_body_replies() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    assert_json_failure(
        config.transform_create_eval_response(&reply(
            r##"{"id": "eval-1", "created_at": 1"##,
            &headers,
        )),
    );
    assert_json_failure(config.transform_get_eval_response(&reply("not json", &headers)));
    assert_json_failure(config.transform_cancel_run_response(&reply("", &headers)));
    assert_json_failure(
        config.transform_delete_run_response(&reply(r##"{"run_id": "run-1""##, &headers)),
    );
}

fn malformed_model_replies() {
    let config = OpenAiEvalsConfig::default();
    let headers = Headers::new();
    assert_model_failure(config.transform_list_evals_response(&reply(
        r#"{"object": "list", "data": [{"object": "eval"}]}"#,
        &headers,
    )));
    assert_model_failure(config.transform_update_eval_response(&reply(r#"{"id": 5}"#, &headers)));
    assert_model_failure(
        config.transform_delete_eval_response(&reply(r#"{"eval_id": "eval-1"}"#, &headers)),
    );
    assert_model_failure(config.transform_cancel_eval_response(&reply(
        r#"{"id": "eval-1", "status": "queued"}"#,
        &headers,
    )));
    assert_model_failure(config.transform_create_run_response(&reply("[]", &headers)));
    assert_model_failure(
        config.transform_list_runs_response(&reply(r#"{"data": [{"id": "run-1"}]}"#, &headers)),
    );
    assert_model_failure(config.transform_get_run_response(&reply(
        r#"{"id": "run-1", "status": "unknown", "created_at": 1, "data_source": {}, "eval_id": "e"}"#,
        &headers,
    )));
}

fn case_malformed_json_error() {
    malformed_json_body_replies();
    malformed_model_replies();
}

fn case_eval_auth_required() {
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
    let sent = config.validate_environment(Headers::new(), None).unwrap();
    assert_eq!(header(&sent, "Authorization"), "Bearer sk-global");
    let openai_only = OpenAiEvalsConfig {
        api_key: None,
        openai_key: Some(SecretString::from("sk-openai")),
        env_api_key: Some(SecretString::from("sk-env")),
    };
    let sent = openai_only
        .validate_environment(Headers::new(), None)
        .unwrap();
    assert_eq!(header(&sent, "Authorization"), "Bearer sk-openai");
    let env_only = OpenAiEvalsConfig {
        api_key: None,
        openai_key: None,
        env_api_key: Some(SecretString::from("sk-env")),
    };
    let sent = env_only.validate_environment(Headers::new(), None).unwrap();
    assert_eq!(header(&sent, "Authorization"), "Bearer sk-env");
    let empty_params: Object = object(r#"{"api_key":""}"#);
    let sent = config
        .validate_environment(Headers::new(), Some(&empty_params))
        .unwrap();
    assert_eq!(header(&sent, "Authorization"), "Bearer sk-global");
    assert!(matches!(
        OpenAiEvalsConfig::default().validate_environment(Headers::new(), None),
        Err(EvalsError::ApiKeyRequired)
    ));
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

#[test]
fn openai_evals_credential_identity_isolation() {
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
    let call_a: Object = object(r#"{"api_key":"call-a"}"#);
    let call_b: Object = object(r#"{"api_key":"call-b"}"#);
    assert_eq!(
        header(
            &identity_a
                .validate_environment(Headers::new(), Some(&call_a))
                .unwrap(),
            "Authorization"
        ),
        "Bearer call-a"
    );
    assert_eq!(
        header(
            &identity_a
                .validate_environment(Headers::new(), Some(&call_b))
                .unwrap(),
            "Authorization"
        ),
        "Bearer call-b"
    );
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
