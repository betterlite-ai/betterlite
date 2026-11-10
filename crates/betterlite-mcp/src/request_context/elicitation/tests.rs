#![cfg(test)]

use std::future::Future;
use std::sync::Mutex;

use betterlite_prompts::arguments::json::Object;
use rstest::rstest;
use thiserror::Error;

use super::{
    DownstreamElicitSession, ElicitAction, ElicitRequestParams, ElicitResult,
    handle_elicitation_request,
};
use crate::wire_compat::capabilities::policy::ClientCapabilities;

const FORM_REQUEST: &str =
    r#"{"mode":"form","message":"who are you","requestedSchema":{"type":"object"}}"#;
const URL_REQUEST: &str = r#"{"mode":"url","message":"sign in","url":"https://idp.test/login","elicitationId":"elicit-1"}"#;

#[derive(Debug, Error)]
#[error("relay failed: {0}")]
struct RelayFailure(String);

#[derive(Clone, Debug, PartialEq)]
enum Call {
    Url {
        message: String,
        url: String,
        elicitation_id: Option<String>,
    },
    Form {
        message: String,
        requested_schema: Object,
    },
}

enum Outcome {
    Result(ElicitResult),
    Failure(String),
}

struct FakeSession {
    outcome: Outcome,
    calls: Mutex<Vec<Call>>,
}

impl FakeSession {
    fn returning(result: ElicitResult) -> Self {
        Self {
            outcome: Outcome::Result(result),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            outcome: Outcome::Failure(message.to_owned()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn outcome(&self) -> Result<ElicitResult, RelayFailure> {
        match &self.outcome {
            Outcome::Result(result) => Ok(result.clone()),
            Outcome::Failure(message) => Err(RelayFailure(message.clone())),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .expect("the calls lock should not be poisoned")
            .clone()
    }
}

impl DownstreamElicitSession for FakeSession {
    type Error = RelayFailure;

    fn elicit(
        &self,
        params: &ElicitRequestParams,
    ) -> impl Future<Output = Result<ElicitResult, RelayFailure>> + Send {
        let recorded = match params {
            ElicitRequestParams::Url(url) => Call::Url {
                message: url.message.clone(),
                url: url.url.clone(),
                elicitation_id: url.elicitation_id.clone(),
            },
            ElicitRequestParams::Form(form) => Call::Form {
                message: form.message.clone(),
                requested_schema: form.requested_schema.clone(),
            },
        };
        self.calls
            .lock()
            .expect("the calls lock should not be poisoned")
            .push(recorded);
        let outcome = self.outcome();
        async move { outcome }
    }
}

fn params(raw: &str) -> ElicitRequestParams {
    serde_json::from_str(raw).expect("the params should parse")
}

fn capabilities(raw: &str) -> ClientCapabilities {
    serde_json::from_str(raw).expect("the capabilities should parse")
}

fn result(raw: &str) -> ElicitResult {
    serde_json::from_str(raw).expect("the result should parse")
}

fn schema(raw: &str) -> Object {
    serde_json::from_str(raw).expect("the schema should parse")
}

#[tokio::test]
async fn a_request_without_a_downstream_session_declines() {
    let outcome = handle_elicitation_request::<FakeSession>(
        &params(FORM_REQUEST),
        None,
        Some(&capabilities(r#"{"elicitation":{"form":{}}}"#)),
    )
    .await;
    assert_eq!(
        outcome,
        ElicitResult {
            action: ElicitAction::Decline,
            content: None,
            meta: None,
        }
    );
}

#[rstest]
#[case::no_elicitation(r#"{"elicitation":null}"#)]
#[case::empty_elicitation(r#"{"elicitation":{}}"#)]
#[case::url_only(r#"{"elicitation":{"url":{}}}"#)]
#[tokio::test]
async fn a_form_request_declines_without_a_form_capability(#[case] capabilities_raw: &str) {
    let session = FakeSession::returning(result(r#"{"action":"accept"}"#));
    let outcome = handle_elicitation_request(
        &params(FORM_REQUEST),
        Some(&session),
        Some(&capabilities(capabilities_raw)),
    )
    .await;
    assert_eq!(outcome.action, ElicitAction::Decline);
    assert_eq!(outcome.content, None);
    assert!(session.calls().is_empty());
}

#[rstest]
#[case::no_elicitation(r#"{"elicitation":null}"#)]
#[case::empty_elicitation(r#"{"elicitation":{}}"#)]
#[case::form_only(r#"{"elicitation":{"form":{}}}"#)]
#[tokio::test]
async fn a_url_request_declines_without_a_url_capability(#[case] capabilities_raw: &str) {
    let session = FakeSession::returning(result(r#"{"action":"accept"}"#));
    let outcome = handle_elicitation_request(
        &params(URL_REQUEST),
        Some(&session),
        Some(&capabilities(capabilities_raw)),
    )
    .await;
    assert_eq!(outcome.action, ElicitAction::Decline);
    assert!(session.calls().is_empty());
}

#[tokio::test]
async fn missing_capabilities_do_not_block_the_relay() {
    let session = FakeSession::returning(result(r#"{"action":"accept"}"#));
    let outcome = handle_elicitation_request(&params(FORM_REQUEST), Some(&session), None).await;
    assert_eq!(outcome.action, ElicitAction::Accept);
    assert_eq!(session.calls().len(), 1);
}

#[tokio::test]
async fn a_form_request_relays_to_elicit_form() {
    let session = FakeSession::returning(result(
        r#"{"action":"accept","content":{"name":"Ada","count":3,"ok":true,"ratio":1.5,"tags":["a","b"],"nothing":null}}"#,
    ));
    let outcome = handle_elicitation_request(
        &params(FORM_REQUEST),
        Some(&session),
        Some(&capabilities(r#"{"elicitation":{"form":{}}}"#)),
    )
    .await;
    assert_eq!(
        session.calls(),
        vec![Call::Form {
            message: "who are you".to_owned(),
            requested_schema: schema(r#"{"type":"object"}"#),
        }]
    );
    assert_eq!(
        outcome,
        result(
            r#"{"action":"accept","content":{"name":"Ada","count":3,"ok":true,"ratio":1.5,"tags":["a","b"],"nothing":null}}"#
        )
    );
}

#[tokio::test]
async fn a_url_request_relays_to_elicit_url_with_the_elicitation_id() {
    let session = FakeSession::returning(result(r#"{"action":"decline"}"#));
    let outcome = handle_elicitation_request(
        &params(URL_REQUEST),
        Some(&session),
        Some(&capabilities(r#"{"elicitation":{"url":{}}}"#)),
    )
    .await;
    assert_eq!(
        session.calls(),
        vec![Call::Url {
            message: "sign in".to_owned(),
            url: "https://idp.test/login".to_owned(),
            elicitation_id: Some("elicit-1".to_owned()),
        }]
    );
    assert_eq!(outcome.action, ElicitAction::Decline);
}

#[rstest]
#[case::small_integer("7")]
#[case::largest_unsigned("18446744073709551615")]
#[case::negative_integer("-3")]
#[case::fraction("2.5")]
fn a_number_in_the_content_round_trips_with_its_text(#[case] number: &str) {
    let raw = format!(r#"{{"action":"accept","content":{{"n":{number}}}}}"#);
    let parsed = result(&raw);
    assert_eq!(
        serde_json::to_string(&parsed).expect("the result should serialize"),
        raw
    );
}

#[tokio::test]
async fn a_url_request_without_an_elicitation_id_is_relayed_with_none() {
    let session = FakeSession::returning(result(r#"{"action":"accept"}"#));
    let outcome = handle_elicitation_request(
        &params(r#"{"mode":"url","message":"sign in","url":"https://idp.test/login"}"#),
        Some(&session),
        None,
    )
    .await;
    assert_eq!(
        session.calls(),
        vec![Call::Url {
            message: "sign in".to_owned(),
            url: "https://idp.test/login".to_owned(),
            elicitation_id: None,
        }]
    );
    assert_eq!(outcome.action, ElicitAction::Accept);
}

#[tokio::test]
async fn the_downstream_meta_is_relayed_with_the_result() {
    let downstream = result(r#"{"action":"accept","_meta":{"trace":"t-1","n":2}}"#);
    let session = FakeSession::returning(downstream.clone());
    let outcome = handle_elicitation_request(&params(FORM_REQUEST), Some(&session), None).await;
    assert_eq!(outcome, downstream);
    assert_eq!(
        serde_json::to_string(&outcome).expect("the result should serialize"),
        r#"{"action":"accept","_meta":{"trace":"t-1","n":2}}"#
    );
}

#[rstest]
#[case::accept(r#"{"action":"accept"}"#, ElicitAction::Accept)]
#[case::decline(r#"{"action":"decline"}"#, ElicitAction::Decline)]
#[case::cancel(r#"{"action":"cancel"}"#, ElicitAction::Cancel)]
#[tokio::test]
async fn the_downstream_action_is_returned_unchanged(
    #[case] result_raw: &str,
    #[case] expected: ElicitAction,
) {
    let session = FakeSession::returning(result(result_raw));
    let outcome = handle_elicitation_request(&params(FORM_REQUEST), Some(&session), None).await;
    assert_eq!(outcome.action, expected);
}

#[tokio::test]
async fn a_relay_failure_declines_after_the_call_is_recorded() {
    let session = FakeSession::failing("the client went away");
    let outcome = handle_elicitation_request(&params(FORM_REQUEST), Some(&session), None).await;
    assert_eq!(
        outcome,
        ElicitResult {
            action: ElicitAction::Decline,
            content: None,
            meta: None,
        }
    );
    assert_eq!(
        session.calls(),
        vec![Call::Form {
            message: "who are you".to_owned(),
            requested_schema: schema(r#"{"type":"object"}"#),
        }]
    );
}

#[rstest]
#[case::form_without_mode(r#"{"message":"who are you","requestedSchema":{}}"#)]
#[case::url_without_mode(
    r#"{"message":"sign in","url":"https://idp.test/login","elicitationId":"elicit-1"}"#
)]
#[tokio::test]
async fn a_missing_mode_uses_the_params_shape(#[case] params_raw: &str) {
    let session = FakeSession::returning(result(r#"{"action":"accept"}"#));
    let outcome = handle_elicitation_request(&params(params_raw), Some(&session), None).await;
    assert_eq!(outcome.action, ElicitAction::Accept);
    assert_eq!(session.calls().len(), 1);
}

#[test]
fn extra_params_fields_are_ignored_like_pydantics_extra_allow() {
    let parsed = params(
        r#"{"mode":"form","message":"who are you","requestedSchema":{},"unexpected":{"a":1}}"#,
    );
    assert_eq!(
        parsed,
        ElicitRequestParams::Form(super::FormElicitParams {
            mode: super::FormMode::Form,
            message: "who are you".to_owned(),
            requested_schema: Object::new(),
        })
    );
}
