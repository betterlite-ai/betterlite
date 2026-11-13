#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{
    CohereRerankGuardrailError, CohereRerankHandler, GUARDRAIL_TRANSLATION_MAPPINGS, RerankCallType,
};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

#[derive(Debug, Clone, PartialEq)]
struct RecordedCall {
    texts: Option<Vec<String>>,
    model: Option<String>,
    input_type: GuardrailInputType,
    request_data: BTreeMap<String, GuardrailValue>,
}

struct ScriptedGuardrail {
    result: Result<Option<Vec<String>>, GuardrailFailure>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl ScriptedGuardrail {
    fn returning(texts: Option<&[&str]>) -> Self {
        Self {
            result: Ok(texts.map(|values| values.iter().map(|text| (*text).to_owned()).collect())),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            result: Err(GuardrailFailure::Other("blocked".to_owned())),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<RecordedCall> {
        self.calls
            .lock()
            .expect("the fixture lock is not poisoned")
            .clone()
    }
}

impl GuardrailContract for ScriptedGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        // the handler must pass the caller's own request data, so a marker here reaches the result
        request_data.insert("guardrail_seen".to_owned(), GuardrailValue::Boolean(true));
        self.calls
            .lock()
            .expect("the fixture lock is not poisoned")
            .push(RecordedCall {
                texts: inputs.texts.clone(),
                model: inputs.model.clone(),
                input_type,
                request_data: request_data.clone(),
            });
        let result = match &self.result {
            Ok(texts) => Ok(GuardrailApiInputs {
                texts: texts.clone(),
                ..inputs
            }),
            Err(failure) => Err(failure.clone()),
        };
        ready(result)
    }
}

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn request() -> BTreeMap<String, GuardrailValue> {
    BTreeMap::from([
        ("query".to_owned(), text("how tall")),
        ("instruction".to_owned(), text("rank by height")),
        (
            "documents".to_owned(),
            GuardrailValue::Array(vec![text("tall"), text("short")]),
        ),
        ("model".to_owned(), text("rerank-v3")),
    ])
}

async fn run_input(
    data: &mut BTreeMap<String, GuardrailValue>,
    guardrail: &ScriptedGuardrail,
) -> Result<(), CohereRerankGuardrailError> {
    CohereRerankHandler
        .process_input_messages(data, guardrail, None)
        .await
}

#[tokio::test]
async fn input_scans_query_then_instruction_with_the_original_request_data() {
    let mut data = request();
    let guardrail = ScriptedGuardrail::returning(Some(&["how high", "rank by width"]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert_eq!(data.get("query"), Some(&text("how high")));
    assert_eq!(data.get("instruction"), Some(&text("rank by width")));
    assert_eq!(
        data.get("guardrail_seen"),
        Some(&GuardrailValue::Boolean(true))
    );
    let calls = guardrail.calls();
    assert_eq!(calls.len(), 1);
    let call = calls.first().unwrap();
    assert_eq!(
        call.texts.as_deref(),
        Some(["how tall".to_owned(), "rank by height".to_owned()].as_slice())
    );
    assert_eq!(call.model.as_deref(), Some("rerank-v3"));
    assert_eq!(call.input_type, GuardrailInputType::Request);
    assert_eq!(call.request_data.get("query"), Some(&text("how tall")));
    assert_eq!(
        call.request_data.get("documents"),
        Some(&GuardrailValue::Array(vec![text("tall"), text("short")]))
    );
    assert_eq!(
        call.request_data.get("guardrail_seen"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[tokio::test]
async fn input_scans_empty_strings() {
    let mut data = BTreeMap::from([
        ("query".to_owned(), text("")),
        ("instruction".to_owned(), text("")),
    ]);
    let guardrail = ScriptedGuardrail::returning(Some(&["clean", ""]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert_eq!(data.get("query"), Some(&text("clean")));
    assert_eq!(data.get("instruction"), Some(&text("")));
    assert_eq!(
        guardrail.calls().first().unwrap().texts.as_deref(),
        Some([String::new(), String::new()].as_slice())
    );
}

#[tokio::test]
async fn input_never_scans_documents_as_a_text_field() {
    let mut data = BTreeMap::from([
        ("query".to_owned(), text("how tall")),
        ("documents".to_owned(), text("a document string")),
    ]);
    let guardrail = ScriptedGuardrail::returning(Some(&["clean"]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert_eq!(data.get("documents"), Some(&text("a document string")));
    assert_eq!(
        guardrail.calls().first().unwrap().texts.as_deref(),
        Some(["how tall".to_owned()].as_slice())
    );
}

#[tokio::test]
async fn input_skips_fields_of_other_types_without_calling_the_hook() {
    let mut data = BTreeMap::from([
        ("query".to_owned(), GuardrailValue::Null),
        ("instruction".to_owned(), GuardrailValue::SignedInteger(3)),
        ("model".to_owned(), GuardrailValue::Boolean(true)),
    ]);
    let guardrail = ScriptedGuardrail::returning(Some(&["clean"]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data.get("query"), Some(&GuardrailValue::Null));
    assert_eq!(
        data.get("instruction"),
        Some(&GuardrailValue::SignedInteger(3))
    );
}

#[tokio::test]
async fn input_skips_the_hook_when_neither_field_is_present() {
    let mut data = BTreeMap::from([("model".to_owned(), text("rerank-v3"))]);
    let guardrail = ScriptedGuardrail::returning(Some(&["clean"]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert!(guardrail.calls().is_empty());
    assert_eq!(data.len(), 1);
}

#[tokio::test]
async fn a_short_result_keeps_the_unreturned_fields() {
    let mut data = request();
    let guardrail = ScriptedGuardrail::returning(Some(&["how high"]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert_eq!(data.get("query"), Some(&text("how high")));
    assert_eq!(data.get("instruction"), Some(&text("rank by height")));
    assert_eq!(
        data.get("documents"),
        Some(&GuardrailValue::Array(vec![text("tall"), text("short")]))
    );
}

#[tokio::test]
async fn extra_result_positions_are_ignored() {
    let mut data = BTreeMap::from([("query".to_owned(), text("how tall"))]);
    let guardrail = ScriptedGuardrail::returning(Some(&["clean", "extra", "more"]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert_eq!(data.get("query"), Some(&text("clean")));
    assert_eq!(data.keys().collect::<Vec<_>>(), ["guardrail_seen", "query"]);
}

#[tokio::test]
async fn a_result_without_texts_leaves_every_field_in_place() {
    let mut data = request();
    let guardrail = ScriptedGuardrail::returning(None);
    run_input(&mut data, &guardrail).await.unwrap();
    assert_eq!(data.get("query"), Some(&text("how tall")));
    assert_eq!(data.get("instruction"), Some(&text("rank by height")));
    assert_eq!(
        data.get("guardrail_seen"),
        Some(&GuardrailValue::Boolean(true))
    );
}

#[tokio::test]
async fn an_empty_model_is_not_sent_to_the_hook() {
    let mut data = BTreeMap::from([
        ("query".to_owned(), text("how tall")),
        ("model".to_owned(), text("")),
    ]);
    let guardrail = ScriptedGuardrail::returning(Some(&["clean"]));
    run_input(&mut data, &guardrail).await.unwrap();
    assert_eq!(guardrail.calls().first().unwrap().model, None);
}

#[tokio::test]
async fn a_hook_failure_propagates() {
    let mut data = request();
    let guardrail = ScriptedGuardrail::failing();
    let error = run_input(&mut data, &guardrail).await.unwrap_err();
    assert!(matches!(error, CohereRerankGuardrailError::Guardrail(_)));
}

#[test]
fn output_returns_the_response_without_calling_the_hook() {
    let guardrail = ScriptedGuardrail::returning(Some(&["clean"]));
    let response = GuardrailValue::Object(BTreeMap::from([(
        "results".to_owned(),
        GuardrailValue::Array(vec![GuardrailValue::Object(BTreeMap::from([(
            "index".to_owned(),
            GuardrailValue::SignedInteger(0),
        )]))]),
    )]));
    let returned = CohereRerankHandler.process_output_response(response.clone(), &guardrail);
    assert_eq!(returned, response);
    assert!(guardrail.calls().is_empty());
}

#[test]
fn the_mapping_binds_rerank_and_arerank_to_the_handler() {
    let call_types: Vec<RerankCallType> = GUARDRAIL_TRANSLATION_MAPPINGS
        .iter()
        .map(|(call_type, _)| *call_type)
        .collect();
    assert_eq!(
        call_types,
        vec![RerankCallType::Rerank, RerankCallType::Arerank]
    );
}
