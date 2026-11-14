#![cfg(test)]

use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_models::decisions::Json;
use rstest::rstest;

use super::PassThroughEndpointHandler;
use super::errors::PassthroughGuardrailError;
use super::settings::{
    PassThroughGuardrailSettings, PassthroughGuardrailInputs, PassthroughInputType, RequestData,
    ResponseCall,
};
use super::traits::{PassthroughGuardrail, PassthroughLogging, PassthroughPolicy};

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn data(source: &str) -> RequestData {
    serde_json::from_str(source).unwrap()
}

fn value(source: &str) -> GuardrailValue {
    serde_json::from_str(source).unwrap()
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn settings(request: Option<&[&str]>, response: Option<&[&str]>) -> PassThroughGuardrailSettings {
    PassThroughGuardrailSettings {
        request_fields: request.map(names),
        response_fields: response.map(names),
    }
}

fn logging(config: Option<&str>) -> FakeLogging {
    FakeLogging {
        config: config.map(json),
    }
}

struct FakeLogging {
    config: Option<Json>,
}

impl PassthroughLogging for FakeLogging {
    fn passthrough_guardrails_config(&self) -> Option<&Json> {
        self.config.as_ref()
    }
}

struct FakePolicy {
    settings: Option<PassThroughGuardrailSettings>,
    extracted: String,
    metadata: RequestData,
    queries: Mutex<Vec<(Json, String)>>,
    extractions: Mutex<Vec<(Json, Vec<String>)>>,
    projections: Mutex<Vec<Option<RequestData>>>,
}

impl FakePolicy {
    fn resolving(settings: Option<PassThroughGuardrailSettings>) -> Self {
        Self {
            settings,
            extracted: "extracted".to_owned(),
            metadata: RequestData::new(),
            queries: Mutex::new(Vec::new()),
            extractions: Mutex::new(Vec::new()),
            projections: Mutex::new(Vec::new()),
        }
    }

    fn extracting(extracted: &str) -> Self {
        Self {
            extracted: extracted.to_owned(),
            ..Self::resolving(None)
        }
    }

    fn projecting(metadata: &str) -> Self {
        Self {
            metadata: data(metadata),
            ..Self::resolving(None)
        }
    }

    fn projections(&self) -> Vec<Option<RequestData>> {
        self.projections.lock().unwrap().clone()
    }

    fn queries(&self) -> Vec<(Json, String)> {
        self.queries.lock().unwrap().clone()
    }

    fn extractions(&self) -> Vec<(Json, Vec<String>)> {
        self.extractions.lock().unwrap().clone()
    }
}

impl PassthroughPolicy for FakePolicy {
    fn get_settings(
        &self,
        config: &Json,
        guardrail_name: &str,
    ) -> Option<PassThroughGuardrailSettings> {
        self.queries
            .lock()
            .unwrap()
            .push((config.clone(), guardrail_name.to_owned()));
        self.settings.clone()
    }

    fn extract_fields(&self, data: &Json, expressions: &[String]) -> String {
        self.extractions
            .lock()
            .unwrap()
            .push((data.clone(), expressions.to_vec()));
        self.extracted.clone()
    }

    fn user_key_metadata(&self, user_api_key_dict: Option<&RequestData>) -> RequestData {
        self.projections
            .lock()
            .unwrap()
            .push(user_api_key_dict.cloned());
        self.metadata.clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Recorded {
    inputs: PassthroughGuardrailInputs,
    input_type: PassthroughInputType,
    request_data: RequestData,
}

#[derive(Debug, thiserror::Error)]
#[error("the guardrail refused the check")]
struct Refusal;

struct FakeGuardrail {
    name: Option<String>,
    refuses: bool,
    calls: Mutex<Vec<Recorded>>,
}

impl FakeGuardrail {
    fn named(name: &str) -> Self {
        Self {
            name: Some(name.to_owned()),
            refuses: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn refusing(name: &str) -> Self {
        Self {
            name: Some(name.to_owned()),
            refuses: true,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Recorded> {
        self.calls.lock().unwrap().clone()
    }
}

impl PassthroughGuardrail for FakeGuardrail {
    type Failure = Refusal;
    type LogTarget = FakeLogging;

    fn guardrail_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn apply_guardrail(
        &self,
        inputs: PassthroughGuardrailInputs,
        request_data: &mut RequestData,
        input_type: PassthroughInputType,
        _logging_obj: Option<&mut FakeLogging>,
    ) -> impl Future<Output = Result<PassthroughGuardrailInputs, Refusal>> + Send {
        self.calls.lock().unwrap().push(Recorded {
            inputs: inputs.clone(),
            input_type,
            request_data: request_data.clone(),
        });
        let refuses = self.refuses;
        // the reply names text the handler must not write back into the relayed payload
        let reply = PassthroughGuardrailInputs {
            texts: vec!["rewritten".to_owned()],
            model: None,
        };
        async move {
            if refuses {
                return Err(Refusal);
            }
            Ok(reply)
        }
    }
}

fn response_call<'call>(
    logging_obj: Option<&'call mut FakeLogging>,
    user_api_key_dict: Option<&'call RequestData>,
    request_data: Option<&'call mut RequestData>,
) -> ResponseCall<'call, FakeLogging> {
    ResponseCall {
        logging_obj,
        user_api_key_dict,
        request_data,
    }
}

#[test]
fn get_guardrail_settings_returns_the_policy_settings() {
    let logging_obj = logging(Some(r#"{"g1": {"request_fields": ["query"]}}"#));
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    let resolved =
        PassThroughEndpointHandler::get_guardrail_settings(Some(&logging_obj), Some("g1"), &policy);
    assert_eq!(resolved, Some(settings(Some(&["query"]), None)));
    assert_eq!(
        policy.queries(),
        vec![(
            json(r#"{"g1": {"request_fields": ["query"]}}"#),
            "g1".to_owned()
        )]
    );
}

#[test]
fn get_guardrail_settings_returns_none_without_a_logging_object() {
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    assert_eq!(
        PassThroughEndpointHandler::get_guardrail_settings(None, Some("g1"), &policy),
        None
    );
    assert!(policy.queries().is_empty());
}

#[test]
fn get_guardrail_settings_returns_none_without_a_config() {
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    assert_eq!(
        PassThroughEndpointHandler::get_guardrail_settings(
            Some(&logging(None)),
            Some("g1"),
            &policy
        ),
        None
    );
    assert!(policy.queries().is_empty());
}

#[rstest]
#[case::empty_object(r#"{}"#)]
#[case::empty_list(r#"[]"#)]
#[case::null("null")]
fn get_guardrail_settings_treats_an_empty_config_as_absent(#[case] config: &str) {
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    assert_eq!(
        PassThroughEndpointHandler::get_guardrail_settings(
            Some(&logging(Some(config))),
            Some("g1"),
            &policy
        ),
        None
    );
    assert!(policy.queries().is_empty());
}

#[test]
fn get_guardrail_settings_hands_a_wrongly_typed_config_to_the_policy() {
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    assert_eq!(
        PassThroughEndpointHandler::get_guardrail_settings(
            Some(&logging(Some(r#""g1""#))),
            Some("g1"),
            &policy
        ),
        Some(settings(Some(&["query"]), None))
    );
    assert_eq!(policy.queries().len(), 1);
}

#[rstest]
#[case::absent(None)]
#[case::empty(Some(""))]
fn get_guardrail_settings_returns_none_without_a_guardrail_name(#[case] name: Option<&str>) {
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    assert_eq!(
        PassThroughEndpointHandler::get_guardrail_settings(
            Some(&logging(Some(r#"{"g1": {"request_fields": ["query"]}}"#))),
            name,
            &policy
        ),
        None
    );
    assert!(policy.queries().is_empty());
}

#[test]
fn get_guardrail_settings_returns_the_policy_decision_for_a_missing_name() {
    let policy = FakePolicy::resolving(None);
    assert_eq!(
        PassThroughEndpointHandler::get_guardrail_settings(
            Some(&logging(Some(r#"{"g1": null}"#))),
            Some("g1"),
            &policy
        ),
        None
    );
    assert_eq!(policy.queries().len(), 1);
}

#[test]
fn extract_text_for_guardrail_extracts_the_configured_fields() {
    let policy = FakePolicy::resolving(None);
    let expressions = names(&["query", "documents[*].text"]);
    let text = PassThroughEndpointHandler::extract_text_for_guardrail(
        &data(r#"{"query": "hello", "documents": [{"text": "first"}]}"#),
        Some(&expressions),
        &policy,
    )
    .unwrap();
    assert_eq!(text, "extracted");
    assert_eq!(
        policy.extractions(),
        vec![(
            json(r#"{"query": "hello", "documents": [{"text": "first"}]}"#),
            expressions
        )]
    );
}

#[test]
fn extract_text_for_guardrail_keeps_internal_fields_in_field_extraction() {
    let policy = FakePolicy::resolving(None);
    let expressions = names(&["query"]);
    PassThroughEndpointHandler::extract_text_for_guardrail(
        &data(r#"{"query": "hello", "_hidden": 1}"#),
        Some(&expressions),
        &policy,
    )
    .unwrap();
    assert_eq!(
        policy.extractions(),
        vec![(json(r#"{"query": "hello", "_hidden": 1}"#), expressions)]
    );
}

#[test]
fn extract_text_for_guardrail_dumps_the_full_payload() {
    let policy = FakePolicy::resolving(None);
    let text = PassThroughEndpointHandler::extract_text_for_guardrail(
        &data(r#"{"b": 2, "a": "x"}"#),
        None,
        &policy,
    )
    .unwrap();
    assert_eq!(text, r#"{"a":"x","b":2}"#);
    assert!(policy.extractions().is_empty());
}

#[test]
fn extract_text_for_guardrail_dumps_the_full_payload_for_no_expressions() {
    let policy = FakePolicy::resolving(None);
    let text = PassThroughEndpointHandler::extract_text_for_guardrail(
        &data(r#"{"b": 2, "a": "x"}"#),
        Some(&[]),
        &policy,
    )
    .unwrap();
    assert_eq!(text, r#"{"a":"x","b":2}"#);
    assert!(policy.extractions().is_empty());
}

#[test]
fn extract_text_for_guardrail_excludes_internal_and_prefixed_keys() {
    let policy = FakePolicy::resolving(None);
    let text = PassThroughEndpointHandler::extract_text_for_guardrail(
        &data(
            r#"{"_hidden": 1, "metadata": {"m": 1}, "litellm_logging_obj": "x", "query": "hello", "outer": {"metadata": 1}}"#,
        ),
        None,
        &policy,
    )
    .unwrap();
    assert_eq!(text, r#"{"outer":{"metadata":1},"query":"hello"}"#);
}

#[test]
fn extract_text_for_guardrail_renders_nested_values_as_json() {
    let policy = FakePolicy::resolving(None);
    let text = PassThroughEndpointHandler::extract_text_for_guardrail(
        &data(r#"{"items": [{"a": 1}, 2], "flag": true, "nothing": null}"#),
        None,
        &policy,
    )
    .unwrap();
    assert_eq!(text, r#"{"flag":true,"items":[{"a":1},2],"nothing":null}"#);
}

#[test]
fn extract_text_for_guardrail_passes_every_value_kind_to_the_policy() {
    let policy = FakePolicy::resolving(None);
    let payload = data(
        r#"{"flag": true, "count": 2, "negative": -3, "score": 0.5, "nothing": null, "items": [1, "two"], "nested": {"deep": false}}"#,
    );
    let expressions = names(&[
        "flag", "count", "negative", "score", "nothing", "items", "nested",
    ]);
    PassThroughEndpointHandler::extract_text_for_guardrail(&payload, Some(&expressions), &policy)
        .unwrap();
    assert_eq!(
        policy.extractions(),
        vec![(
            json(
                r#"{"flag": true, "count": 2, "negative": -3, "score": 0.5, "nothing": null, "items": [1, "two"], "nested": {"deep": false}}"#
            ),
            expressions
        )]
    );
}

#[test]
fn extract_text_for_guardrail_renders_a_non_finite_float_as_null() {
    let policy = FakePolicy::resolving(None);
    let mut payload = RequestData::new();
    payload.insert("score".to_owned(), GuardrailValue::Decimal(f64::NAN));
    let text =
        PassThroughEndpointHandler::extract_text_for_guardrail(&payload, None, &policy).unwrap();
    assert_eq!(text, r#"{"score":null}"#);
}

#[tokio::test]
async fn process_input_messages_checks_the_extracted_text_and_keeps_the_request() {
    let mut payload = data(r#"{"query": "hello", "model": "m1"}"#);
    let original = payload.clone();
    let guardrail = FakeGuardrail::named("g1");
    let mut logging_obj = logging(Some(r#"{"g1": {"request_fields": ["query"]}}"#));
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    PassThroughEndpointHandler::process_input_messages(
        &mut payload,
        &guardrail,
        Some(&mut logging_obj),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(payload, original);
    assert_eq!(
        guardrail.calls(),
        vec![Recorded {
            inputs: PassthroughGuardrailInputs {
                texts: vec!["extracted".to_owned()],
                model: Some("m1".to_owned()),
            },
            input_type: PassthroughInputType::Request,
            request_data: original,
        }]
    );
}

#[tokio::test]
async fn process_input_messages_scans_the_full_payload_without_request_fields() {
    let mut payload = data(r#"{"query": "hello"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let mut logging_obj = logging(Some(r#"{"g1": {"response_fields": ["output"]}}"#));
    let policy = FakePolicy::resolving(Some(settings(None, Some(&["output"]))));
    PassThroughEndpointHandler::process_input_messages(
        &mut payload,
        &guardrail,
        Some(&mut logging_obj),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        guardrail.calls().first().map(|call| call.inputs.clone()),
        Some(PassthroughGuardrailInputs {
            texts: vec![r#"{"query":"hello"}"#.to_owned()],
            model: None,
        })
    );
    assert!(policy.extractions().is_empty());
}

#[tokio::test]
async fn process_input_messages_skips_the_guardrail_for_empty_text() {
    let mut payload = data(r#"{"query": "hello"}"#);
    let original = payload.clone();
    let guardrail = FakeGuardrail::named("g1");
    let mut logging_obj = logging(Some(r#"{"g1": {"request_fields": ["query"]}}"#));
    let policy = FakePolicy::extracting("");
    PassThroughEndpointHandler::process_input_messages(
        &mut payload,
        &guardrail,
        Some(&mut logging_obj),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(payload, original);
    assert!(guardrail.calls().is_empty());
}

#[tokio::test]
async fn process_input_messages_scans_an_empty_payload() {
    let mut payload = data("{}");
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::resolving(None);
    PassThroughEndpointHandler::process_input_messages(&mut payload, &guardrail, None, &policy)
        .await
        .unwrap();
    assert_eq!(
        guardrail
            .calls()
            .first()
            .map(|call| call.inputs.texts.clone()),
        Some(vec!["{}".to_owned()])
    );
}

#[rstest]
#[case::present(r#"{"model": "m1"}"#, Some("m1"))]
#[case::absent(r#"{"query": "hello"}"#, None)]
#[case::empty(r#"{"model": ""}"#, None)]
#[case::null(r#"{"model": null}"#, None)]
#[case::number(r#"{"model": 3}"#, None)]
#[tokio::test]
async fn process_input_messages_passes_only_a_non_empty_text_model(
    #[case] source: &str,
    #[case] model: Option<&str>,
) {
    let mut payload = data(source);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::resolving(None);
    PassThroughEndpointHandler::process_input_messages(&mut payload, &guardrail, None, &policy)
        .await
        .unwrap();
    assert_eq!(
        guardrail
            .calls()
            .first()
            .map(|call| call.inputs.model.clone()),
        Some(model.map(str::to_owned))
    );
}

#[tokio::test]
async fn process_input_messages_propagates_a_guardrail_failure() {
    let mut payload = data(r#"{"query": "hello"}"#);
    let guardrail = FakeGuardrail::refusing("g1");
    let policy = FakePolicy::resolving(None);
    let result =
        PassThroughEndpointHandler::process_input_messages(&mut payload, &guardrail, None, &policy)
            .await;
    assert!(matches!(
        result,
        Err(PassthroughGuardrailError::Guardrail(_))
    ));
}

#[rstest]
#[case::text(r#""just text""#)]
#[case::list("[1, 2]")]
#[case::null("null")]
#[case::boolean("true")]
#[tokio::test]
async fn process_output_response_skips_a_non_object_response(#[case] source: &str) {
    let response = value(source);
    let guardrail = FakeGuardrail::named("g1");
    let mut logging_obj = logging(Some(r#"{"g1": {"response_fields": ["output"]}}"#));
    let policy = FakePolicy::resolving(Some(settings(None, Some(&["output"]))));
    let returned = PassThroughEndpointHandler::process_output_response(
        response.clone(),
        &guardrail,
        response_call(Some(&mut logging_obj), None, None),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(returned, response);
    assert!(guardrail.calls().is_empty());
    assert!(policy.queries().is_empty());
}

#[tokio::test]
async fn process_output_response_checks_the_extracted_text_and_keeps_the_response() {
    let response = value(r#"{"output": "answer", "model": "m2"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let mut logging_obj = logging(Some(r#"{"g1": {"response_fields": ["output"]}}"#));
    let policy = FakePolicy::resolving(Some(settings(None, Some(&["output"]))));
    let returned = PassThroughEndpointHandler::process_output_response(
        response.clone(),
        &guardrail,
        response_call(Some(&mut logging_obj), None, None),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(returned, response);
    assert_eq!(
        guardrail.calls(),
        vec![Recorded {
            inputs: PassthroughGuardrailInputs {
                texts: vec!["extracted".to_owned()],
                model: Some("m2".to_owned()),
            },
            input_type: PassthroughInputType::Response,
            request_data: data(
                r#"{"output": "answer", "model": "m2", "response": {"output": "answer", "model": "m2"}}"#
            ),
        }]
    );
}

#[tokio::test]
async fn process_output_response_scans_the_full_payload_without_response_fields() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let mut logging_obj = logging(Some(r#"{"g1": {"request_fields": ["query"]}}"#));
    let policy = FakePolicy::resolving(Some(settings(Some(&["query"]), None)));
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(Some(&mut logging_obj), None, None),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        guardrail
            .calls()
            .first()
            .map(|call| call.inputs.texts.clone()),
        Some(vec![r#"{"output":"answer"}"#.to_owned()])
    );
    assert!(policy.extractions().is_empty());
}

#[tokio::test]
async fn process_output_response_skips_the_guardrail_for_empty_text() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let mut logging_obj = logging(Some(r#"{"g1": {"response_fields": ["output"]}}"#));
    let policy = FakePolicy::extracting("");
    let mut request_data = data(r#"{"extra": 1}"#);
    let returned = PassThroughEndpointHandler::process_output_response(
        response.clone(),
        &guardrail,
        response_call(Some(&mut logging_obj), None, Some(&mut request_data)),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(returned, response);
    assert!(guardrail.calls().is_empty());
    assert_eq!(request_data, data(r#"{"extra": 1}"#));
}

#[tokio::test]
async fn process_output_response_hands_the_response_alone_to_a_standalone_guardrail() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::resolving(None);
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, None, None),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        guardrail
            .calls()
            .first()
            .map(|call| call.request_data.clone()),
        Some(data(r#"{"output": "answer"}"#))
    );
}

#[tokio::test]
async fn process_output_response_injects_user_key_metadata_into_a_standalone_request_data() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::projecting(r#"{"user_api_key_user_id": "u1"}"#);
    let user_api_key = data(r#"{"user_id": "u1"}"#);
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, Some(&user_api_key), None),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        guardrail
            .calls()
            .first()
            .map(|call| call.request_data.clone()),
        Some(data(
            r#"{"output": "answer", "litellm_metadata": {"user_api_key_user_id": "u1"}}"#
        ))
    );
}

#[tokio::test]
async fn process_output_response_adds_the_response_to_the_callers_request_data() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::resolving(None);
    let mut request_data = data(r#"{"extra": 1}"#);
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, None, Some(&mut request_data)),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        request_data,
        data(r#"{"extra": 1, "response": {"output": "answer"}}"#)
    );
}

#[tokio::test]
async fn process_output_response_keeps_an_existing_response_key() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::projecting(r#"{"user_api_key_user_id": "u1"}"#);
    let user_api_key = data(r#"{"user_id": "u1"}"#);
    let mut request_data = data(r#"{"response": "kept"}"#);
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, Some(&user_api_key), Some(&mut request_data)),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        request_data,
        data(r#"{"response": "kept", "litellm_metadata": {"user_api_key_user_id": "u1"}}"#)
    );
}

#[tokio::test]
async fn process_output_response_injects_the_policy_user_key_metadata() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::projecting(
        r#"{"user_api_key_user_id": "u1", "user_api_key_team_id": "t1", "user_api_key_spend": 0.0}"#,
    );
    let user_api_key = data(r#"{"user_id": "u1", "team_id": "t1", "spend": 0.0}"#);
    let mut request_data = data(r#"{"extra": 1}"#);
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, Some(&user_api_key), Some(&mut request_data)),
        &policy,
    )
    .await
    .unwrap();
    let expected = data(
        r#"{"user_api_key_user_id": "u1", "user_api_key_team_id": "t1", "user_api_key_spend": 0.0}"#,
    );
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&GuardrailValue::Object(expected))
    );
    assert_eq!(policy.projections(), vec![Some(user_api_key)]);
}

#[rstest]
#[case::empty(r#"{}"#)]
#[case::populated(r#"{"user_id": "u1"}"#)]
#[tokio::test]
async fn process_output_response_keeps_an_existing_metadata_bucket(#[case] metadata: &str) {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::projecting(r#"{"user_api_key_user_id": "u1"}"#);
    let user_api_key = data(r#"{"user_id": "u1"}"#);
    let mut request_data = data(&format!(r#"{{"litellm_metadata": {metadata}}}"#));
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, Some(&user_api_key), Some(&mut request_data)),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&GuardrailValue::Object(data(metadata)))
    );
    assert!(policy.projections().is_empty());
}

#[rstest]
#[case::absent(None)]
#[case::skipped_values(Some(r#"{"absent": null, "_internal": "x"}"#))]
#[tokio::test]
async fn process_output_response_does_not_inject_empty_user_metadata(
    #[case] user_api_key_source: Option<&str>,
) {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::resolving(None);
    let user_api_key = user_api_key_source.map(data);
    let mut request_data = data(r#"{"extra": 1}"#);
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, user_api_key.as_ref(), Some(&mut request_data)),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        request_data,
        data(r#"{"extra": 1, "response": {"output": "answer"}}"#)
    );
    assert_eq!(policy.projections(), vec![user_api_key]);
}

#[tokio::test]
async fn process_output_response_propagates_a_guardrail_failure() {
    let response = value(r#"{"output": "answer"}"#);
    let guardrail = FakeGuardrail::refusing("g1");
    let policy = FakePolicy::resolving(None);
    let result = PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, None, None),
        &policy,
    )
    .await;
    assert!(matches!(
        result,
        Err(PassthroughGuardrailError::Guardrail(_))
    ));
}

#[tokio::test]
async fn process_output_response_scans_a_non_finite_float_as_null() {
    let mut fields = RequestData::new();
    fields.insert("score".to_owned(), GuardrailValue::Decimal(f64::NAN));
    let response = GuardrailValue::Object(fields);
    let guardrail = FakeGuardrail::named("g1");
    let policy = FakePolicy::resolving(None);
    PassThroughEndpointHandler::process_output_response(
        response,
        &guardrail,
        response_call(None, None, None),
        &policy,
    )
    .await
    .unwrap();
    assert_eq!(
        guardrail
            .calls()
            .first()
            .map(|call| call.inputs.texts.clone()),
        Some(vec![r#"{"score":null}"#.to_owned()])
    );
}
