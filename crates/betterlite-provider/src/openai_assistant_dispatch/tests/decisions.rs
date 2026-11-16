#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::openai::{ChoiceValue, DecisionAnswer};
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::transformation::wire::ProviderRequestWire;
use secrecy::{ExposeSecret, SecretString};

use crate::openai_assistant_dispatch::OpenAiAssistantResult;
use crate::openai_assistant_dispatch::call::{
    DecisionsCall, DecisionsOperation, OpenAiAssistantOperation,
};
use crate::openai_assistant_dispatch::error::OpenAiAssistantError;
use crate::openai_assistant_dispatch::reply::{DecisionsProjection, DecisionsReply};

use super::support::{BASE, Recorder, RecordingLogger, run_operation};

const CHOICE_QUESTION: &str = r#"{"input":"Check this","questions":[{"type":"choice","name":"label","instructions":"Pick","choices":[{"value":"yes"},{"value":"no"}]}],"safety_identifier":"safe-1"}"#;
const CHOICE_WIRE: &str = r#"{"model":"gpt-4o","input":"Check this","questions":[{"type":"choice","name":"label","instructions":"Pick","choices":[{"value":"yes"},{"value":"no"}]}],"safety_identifier":"safe-1"}"#;
const CHOICE_ANSWERS: &str = r#"{"model":"gpt-4o","answers":[{"type":"choice","name":"label","choice":"yes","probabilities":[{"value":"yes","probability":0.9},{"value":"no","probability":0.1}],"confidence":0.9}],"usage":{"input_tokens":3,"output_tokens":2,"total_tokens":5}}"#;

fn decisions_call(
    api_base: Option<&str>,
    api_key: Option<&str>,
    globals: LiteLlmGlobals,
    environment: BTreeMap<String, String>,
    operation: DecisionsOperation,
) -> OpenAiAssistantOperation {
    OpenAiAssistantOperation::Decisions(DecisionsCall {
        api_base: api_base.map(String::from),
        api_key: api_key.map(SecretString::from),
        globals,
        environment,
        operation,
    })
}

fn env(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

fn choice_request() -> DecisionsOperation {
    DecisionsOperation::Request {
        model: String::from("gpt-4o"),
        request: serde_json::from_str(CHOICE_QUESTION).unwrap(),
    }
}

async fn decisions_request() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(
        Some(BASE),
        Some("sk-decisions"),
        LiteLlmGlobals::default(),
        BTreeMap::new(),
        choice_request(),
    );
    let projected = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Decisions(reply) = projected else {
        panic!("the decisions request did not project")
    };
    let DecisionsReply {
        api_base,
        api_key,
        projection,
    } = *reply;
    assert_eq!(api_base.as_deref(), Some(BASE));
    assert_eq!(
        api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("sk-decisions")
    );
    let DecisionsProjection::Request(wire) = projection else {
        panic!("the request call produced a response projection")
    };
    assert!(matches!(&*wire, ProviderRequestWire::OpenAi(_)));
    let written: JsonValue = serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap();
    let expected: JsonValue = serde_json::from_str(CHOICE_WIRE).unwrap();
    assert_eq!(written, expected);
    assert!(transport.seen.lock().unwrap().is_empty());
}

async fn decisions_resolution(
    globals: LiteLlmGlobals,
    environment: BTreeMap<String, String>,
    expected_base: &str,
    expected_key: Option<&str>,
) {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(None, None, globals, environment, choice_request());
    let projected = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Decisions(reply) = projected else {
        panic!("the credential resolution did not project")
    };
    assert_eq!(reply.api_base.as_deref(), Some(expected_base));
    assert_eq!(
        reply.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected_key
    );
}

async fn decisions_response() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(
        Some(BASE),
        Some("sk-decisions"),
        LiteLlmGlobals::default(),
        BTreeMap::new(),
        DecisionsOperation::Response {
            model: String::from("gpt-4o"),
            request: serde_json::from_str(CHOICE_QUESTION).unwrap(),
            payload: CHOICE_ANSWERS.as_bytes().to_vec(),
        },
    );
    let projected = run_operation(&transport, &mut logger, call).await.unwrap();
    let OpenAiAssistantResult::Decisions(reply) = projected else {
        panic!("the response call did not project its answer")
    };
    let DecisionsReply { projection, .. } = *reply;
    let DecisionsProjection::Response(response) = projection else {
        panic!("the response call produced another projection")
    };
    assert_eq!(response.model, "gpt-4o");
    assert_eq!(response.usage.total_tokens.0.as_u64(), Some(5));
    let DecisionAnswer::Choice { name, choice, .. } = &response.answers[0] else {
        panic!("the projected answer did not stay a choice")
    };
    assert_eq!(name.as_deref(), Some("label"));
    assert!(matches!(choice, ChoiceValue::Text(text) if text == "yes"));
}

async fn decisions_malformed_payload() {
    let transport = Arc::new(Recorder::default());
    let mut logger = RecordingLogger::default();
    let call = decisions_call(
        Some(BASE),
        Some("sk-decisions"),
        LiteLlmGlobals::default(),
        BTreeMap::new(),
        DecisionsOperation::Response {
            model: String::from("gpt-4o"),
            request: serde_json::from_str(CHOICE_QUESTION).unwrap(),
            payload: b"{not json".to_vec(),
        },
    );
    let Err(error) = run_operation(&transport, &mut logger, call).await else {
        panic!("the malformed decisions payload did not surface")
    };
    assert!(matches!(error, OpenAiAssistantError::DecisionsPayload(_)));
}

// Nothing explicit is supplied, so the leaf resolves from the globals first and the environment after them.
async fn credential_resolution() {
    let environment = env(&[
        ("OPENAI_API_KEY", "sk-env"),
        ("OPENAI_BASE_URL", "https://env.test/v1"),
    ]);
    let globals = LiteLlmGlobals {
        api_base: Some(String::from("https://global.test/v1")),
        api_key: Some(SecretString::from("sk-global-api")),
        openai_key: Some(SecretString::from("sk-global-openai")),
        organization: None,
    };
    let openai_key_only = LiteLlmGlobals {
        openai_key: Some(SecretString::from("sk-global-openai")),
        ..LiteLlmGlobals::default()
    };
    decisions_resolution(
        LiteLlmGlobals::default(),
        environment.clone(),
        "https://env.test/v1",
        Some("sk-env"),
    )
    .await;
    decisions_resolution(
        globals,
        environment.clone(),
        "https://global.test/v1",
        Some("sk-global-api"),
    )
    .await;
    decisions_resolution(
        openai_key_only,
        environment,
        "https://env.test/v1",
        Some("sk-global-openai"),
    )
    .await;
}

// With nothing supplied anywhere, the leaf keeps OpenAI's public base and sends no key.
async fn default_credential_resolution() {
    decisions_resolution(
        LiteLlmGlobals::default(),
        BTreeMap::new(),
        "https://api.openai.com",
        None,
    )
    .await;
}

pub(super) async fn case() {
    decisions_request().await;
    credential_resolution().await;
    default_credential_resolution().await;
    decisions_response().await;
    decisions_malformed_payload().await;
}
