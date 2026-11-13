#![cfg(test)]

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;

use thiserror::Error;

use super::{
    CallerCapture, CapturedElicitation, CapturedSampling, ElicitationCallback as _, SamplingCall,
    SamplingCallback as _, SamplingHandler,
};
use crate::request_context::elicitation::{
    DownstreamElicitSession, ElicitAction, ElicitRequestParams, ElicitResult,
};
use crate::wire_compat::capabilities::policy::ClientCapabilities;

fn header_pairs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn captured(auth: &str) -> CallerCapture<String> {
    CallerCapture {
        user_api_key_auth: Some(auth.to_owned()),
        raw_headers: Some(header_pairs(&[("x-client-id", auth)])),
        client_ip: Some("10.0.0.1".to_owned()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    context: u32,
    params: String,
    default_model: Option<String>,
    caller: CallerCapture<String>,
}

struct Recorder {
    seen: Mutex<Option<Seen>>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            seen: Mutex::new(None),
        }
    }

    fn seen(&self) -> Seen {
        self.seen
            .lock()
            .expect("the seen lock should not be poisoned")
            .clone()
            .expect("the handler should have been called")
    }
}

impl SamplingHandler<String> for Recorder {
    type Context = u32;
    type Params = String;
    type Response = String;

    fn handle<'handler, 'call>(
        &'handler self,
        call: SamplingCall<'call, String, u32, String>,
    ) -> impl Future<Output = String> + Send + 'handler + 'call {
        let seen = Seen {
            context: *call.context,
            params: call.params.clone(),
            default_model: call.default_model.map(str::to_owned),
            caller: call.caller.clone(),
        };
        self.seen
            .lock()
            .expect("the seen lock should not be poisoned")
            .replace(seen);
        async move { "sampled".to_owned() }
    }
}

#[test]
fn the_operation_context_wins_over_the_individual_arguments() {
    let operation_context = captured("operation");
    let resolved = CallerCapture::resolve(
        Some("individual".to_owned()),
        Some(header_pairs(&[("x-client-id", "individual")])),
        Some("10.0.0.2".to_owned()),
        Some(operation_context.clone()),
        Some(&captured("ambient")),
    );
    assert_eq!(resolved, operation_context);
}

#[test]
fn an_explicit_user_auth_ignores_the_ambient_caller() {
    let resolved = CallerCapture::resolve(
        Some("individual".to_owned()),
        None,
        None,
        None,
        Some(&captured("ambient")),
    );
    assert_eq!(
        resolved,
        CallerCapture {
            user_api_key_auth: Some("individual".to_owned()),
            raw_headers: None,
            client_ip: None,
        }
    );
}

#[test]
fn the_ambient_caller_fills_each_field_the_caller_leaves_out() {
    let ambient = captured("ambient");
    let resolved = CallerCapture::resolve(None, None, None, None, Some(&ambient));
    assert_eq!(resolved, ambient);
    let partially_resolved = CallerCapture::resolve(
        None,
        Some(header_pairs(&[("x-own", "1")])),
        None,
        None,
        Some(&ambient),
    );
    assert_eq!(
        partially_resolved,
        CallerCapture {
            user_api_key_auth: Some("ambient".to_owned()),
            raw_headers: Some(header_pairs(&[("x-own", "1")])),
            client_ip: Some("10.0.0.1".to_owned()),
        }
    );
}

#[test]
fn no_arguments_and_no_ambient_caller_resolve_to_empty() {
    assert_eq!(
        CallerCapture::<String>::resolve(None, None, None, None, None),
        CallerCapture {
            user_api_key_auth: None,
            raw_headers: None,
            client_ip: None,
        }
    );
}

#[tokio::test]
async fn the_sampling_callback_forwards_the_capture_and_default_model() {
    let caller = captured("key-1");
    let callback = CapturedSampling {
        handler: Recorder::new(),
        caller: caller.clone(),
        default_model: Some("gpt-4o".to_owned()),
    };
    let response = callback.call(&7_u32, &"sampling-params".to_owned()).await;
    assert_eq!(response, "sampled");
    assert_eq!(
        callback.handler.seen(),
        Seen {
            context: 7,
            params: "sampling-params".to_owned(),
            default_model: Some("gpt-4o".to_owned()),
            caller,
        }
    );
}

#[tokio::test]
async fn a_sampling_callback_without_a_default_model_forwards_none() {
    let callback = CapturedSampling {
        handler: Recorder::new(),
        caller: captured("key-1"),
        default_model: None,
    };
    let _response = callback.call(&1_u32, &String::new()).await;
    assert_eq!(callback.handler.seen().default_model, None);
}

#[derive(Debug, Error)]
#[error("relay failed: {0}")]
struct RelayFailure(String);

struct FakeSession {
    action: ElicitAction,
    calls: Mutex<Vec<String>>,
}

impl FakeSession {
    fn accepting() -> Self {
        Self {
            action: ElicitAction::Accept,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
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
        let method = match params {
            ElicitRequestParams::Url(_) => "elicit_url",
            ElicitRequestParams::Form(_) => "elicit_form",
        };
        self.calls
            .lock()
            .expect("the calls lock should not be poisoned")
            .push(method.to_owned());
        let action = self.action;
        async move {
            Ok(ElicitResult {
                action,
                content: None,
                meta: None,
            })
        }
    }
}

fn form_params() -> ElicitRequestParams {
    serde_json::from_str(r#"{"mode":"form","message":"who are you","requestedSchema":{}}"#)
        .expect("the params should parse")
}

#[tokio::test]
async fn the_elicitation_callback_answers_with_the_captured_session() {
    let callback = CapturedElicitation {
        session: Some(FakeSession::accepting()),
        capabilities: None,
    };
    let outcome = callback.answer(&form_params()).await;
    assert_eq!(outcome.action, ElicitAction::Accept);
    let calls = callback
        .session
        .as_ref()
        .expect("the session should be captured")
        .calls();
    assert_eq!(calls, vec!["elicit_form".to_owned()]);
}

#[tokio::test]
async fn an_elicitation_callback_without_a_session_declines() {
    let callback = CapturedElicitation::<FakeSession> {
        session: None,
        capabilities: None,
    };
    let outcome = callback.answer(&form_params()).await;
    assert_eq!(outcome.action, ElicitAction::Decline);
}

#[tokio::test]
async fn the_elicitation_callback_applies_the_captured_capabilities() {
    let capabilities: ClientCapabilities =
        serde_json::from_str(r#"{"elicitation":{"url":{}}}"#).expect("capabilities should parse");
    let callback = CapturedElicitation {
        session: Some(FakeSession::accepting()),
        capabilities: Some(capabilities),
    };
    let outcome = callback.answer(&form_params()).await;
    assert_eq!(outcome.action, ElicitAction::Decline);
    assert!(
        callback
            .session
            .as_ref()
            .expect("the session should be captured")
            .calls()
            .is_empty()
    );
}
