use betterlite_core::api::request_destination::RequestData;
use betterlite_core::api::request_principal_metadata::Metadata;
use betterlite_logging::passthrough_logging_state::usage::UpstreamReportedUsage;
use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::failure_hook::lift::PASS_THROUGH_CALL_TYPE;
use crate::passthrough::PassthroughLogPayload;
use crate::passthrough::failure_payload::{build_failure_payload, carry_guardrail_logging_info};
use crate::passthrough::request_context::{PassThroughKwargs, PassthroughLoggingObj};

fn data(text: &str) -> RequestData {
    serde_json::from_str(text).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn object(value: &Json) -> &Metadata {
    if let Json::Object(fields) = value {
        return fields;
    }
    panic!("not an object: {value:?}");
}

fn payload() -> PassthroughLogPayload {
    PassthroughLogPayload {
        url: "https://upstream.example/v1".to_owned(),
        request_method: Some("POST".to_owned()),
        request_body: None,
        response_body: None,
        cost_per_request: None,
    }
}

fn assert_carried() {
    let guardrail = data(
        r#"{"metadata": {"standard_logging_guardrail_information": [{"guardrail_name": "g1"}]}}"#,
    );
    let mut request_data = data(r#"{"metadata": {"team_tag": "t1"}}"#);
    carry_guardrail_logging_info(&mut request_data, Some(&guardrail));
    let metadata = object(&request_data["metadata"]);
    assert_eq!(
        metadata["standard_logging_guardrail_information"],
        json(r#"[{"guardrail_name": "g1"}]"#)
    );
    assert_eq!(metadata["team_tag"], Json::Text("t1".to_owned()));
}

fn assert_setdefault() {
    let guardrail = data(
        r#"{"metadata": {"standard_logging_guardrail_information": [{"guardrail_name": "g1"}]}}"#,
    );
    let mut populated = data(
        r#"{"metadata": {"standard_logging_guardrail_information": [{"guardrail_name": "kept"}]}}"#,
    );
    carry_guardrail_logging_info(&mut populated, Some(&guardrail));
    assert_eq!(
        object(&populated["metadata"])["standard_logging_guardrail_information"],
        json(r#"[{"guardrail_name": "kept"}]"#)
    );
}

fn assert_nothing_to_carry() {
    let mut untouched = data(r#"{"metadata": {}}"#);
    carry_guardrail_logging_info(&mut untouched, None);
    carry_guardrail_logging_info(
        &mut untouched,
        Some(&data(
            r#"{"metadata": {"standard_logging_guardrail_information": []}}"#,
        )),
    );
    carry_guardrail_logging_info(&mut untouched, Some(&data(r#"{"metadata": {}}"#)));
    assert_eq!(object(&untouched["metadata"]).len(), 0);
}

fn assert_litellm_bucket() {
    let guardrail = data(
        r#"{"litellm_metadata": {"standard_logging_guardrail_information": [{"guardrail_name": "g2"}]}}"#,
    );
    let mut request_data = data(r#"{"litellm_metadata": {}}"#);
    carry_guardrail_logging_info(&mut request_data, Some(&guardrail));
    assert_eq!(
        object(&request_data["litellm_metadata"])["standard_logging_guardrail_information"],
        json(r#"[{"guardrail_name": "g2"}]"#)
    );
    assert!(request_data.get("metadata").is_none());
}

fn merged() -> RequestData {
    let kwargs = PassThroughKwargs {
        litellm_params: data(r#"{"metadata": {"team_tag": "t1"}}"#),
        call_type: PASS_THROUGH_CALL_TYPE,
        litellm_call_id: Some("call-1".to_owned()),
        passthrough_logging_payload: payload(),
    };
    let logging_obj = PassthroughLoggingObj {
        model_call_details: data(r#"{"model": "m"}"#),
    };
    let usage = UpstreamReportedUsage {
        response_cost: Some(0.25),
        total_tokens: Some(7),
    };
    build_failure_payload(
        Some(&data(r#"{"model": "m", "custom": 1}"#)),
        Some(&kwargs),
        Some(&logging_obj),
        Some("openai"),
        Some(&usage),
    )
}

fn assert_merged(request_payload: &RequestData) {
    assert_eq!(request_payload["model"], Json::Text("m".to_owned()));
    assert_eq!(
        request_payload["custom_llm_provider"],
        Json::Text("openai".to_owned())
    );
    assert_eq!(request_payload["custom"], json("1"));
    assert_eq!(
        request_payload["call_type"],
        Json::Text("pass_through_endpoint".to_owned())
    );
    assert_eq!(
        request_payload["litellm_call_id"],
        Json::Text("call-1".to_owned())
    );
    let params = object(&request_payload["litellm_params"]);
    assert_eq!(
        object(&params["metadata"])["team_tag"],
        Json::Text("t1".to_owned())
    );
    let logging_obj = object(&request_payload["litellm_logging_obj"]);
    assert_eq!(
        object(&logging_obj["model_call_details"])["model"],
        Json::Text("m".to_owned())
    );
    assert_eq!(
        request_payload["response_cost"],
        Json::Number(Number::from_f64(0.25).unwrap())
    );
    let combined = object(&request_payload["combined_usage_object"]);
    assert_eq!(combined["total_tokens"], Json::Number(Number::from(7_i64)));
    assert_eq!(combined["prompt_tokens"], Json::Number(Number::from(0_i64)));
    assert_eq!(
        combined["completion_tokens"],
        Json::Number(Number::from(0_i64))
    );
}

fn assert_model_and_provider() {
    // model is copied from the body as an empty string, but only when there is a body
    let fallback = build_failure_payload(Some(&data(r#"{"prompt": "x"}"#)), None, None, None, None);
    assert_eq!(fallback["model"], Json::Text(String::new()));
    assert!(fallback.get("custom_llm_provider").is_none());
    let empty = build_failure_payload(None, None, None, None, None);
    assert!(empty.get("model").is_none());

    // a provider the body already names wins; an empty provider is not written
    let named = build_failure_payload(
        Some(&data(r#"{"custom_llm_provider": "body-provider"}"#)),
        None,
        None,
        Some("argument-provider"),
        None,
    );
    assert_eq!(
        named["custom_llm_provider"],
        Json::Text("body-provider".to_owned())
    );
    let blank = build_failure_payload(
        Some(&data(r#"{"prompt": "x"}"#)),
        None,
        None,
        Some(""),
        None,
    );
    assert!(blank.get("custom_llm_provider").is_none());
}

fn assert_usage() {
    // a missing cost reports zero, and a NaN is not a JSON number either
    let missing = UpstreamReportedUsage {
        response_cost: None,
        total_tokens: None,
    };
    let without_cost = build_failure_payload(None, None, None, None, Some(&missing));
    assert_eq!(
        without_cost["response_cost"],
        Json::Number(Number::from_f64(0.0).unwrap())
    );
    assert_eq!(
        object(&without_cost["combined_usage_object"])["total_tokens"],
        Json::Number(Number::from(0_i64))
    );
    let not_a_number = UpstreamReportedUsage {
        response_cost: Some(f64::NAN),
        total_tokens: Some(7),
    };
    let nan = build_failure_payload(None, None, None, None, Some(&not_a_number));
    assert_eq!(
        nan["response_cost"],
        Json::Number(Number::from_f64(0.0).unwrap())
    );
}

#[test]
fn passthrough_carry_guardrail() {
    assert_carried();
    assert_setdefault();
    assert_nothing_to_carry();
    assert_litellm_bucket();
}

#[test]
fn passthrough_build_failure_payload() {
    assert_merged(&merged());
    assert_model_and_provider();
    assert_usage();
}
