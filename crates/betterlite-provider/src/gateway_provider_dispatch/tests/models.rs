use std::error::Error as _;
use std::sync::{Mutex, PoisonError};

use betterlite_openai::vercel_ai_gateway::models_error::ModelsError as VercelModelsError;
use rstest::rstest;

use crate::gateway_provider_dispatch::context::DispatchSettings;
use crate::gateway_provider_dispatch::models::{
    ModelsError, ModelsRequest, ModelsResponse, ModelsTransport, get_models,
};

use super::support::{Fixture, secret};

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
struct TransportFailure {
    message: String,
}

#[derive(Debug)]
struct Transport {
    requests: Mutex<Vec<ModelsRequest>>,
    reply: Result<ModelsResponse, TransportFailure>,
}

impl Transport {
    fn replying(status: u16, body: &str) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            reply: Ok(ModelsResponse {
                status,
                body: body.as_bytes().to_vec(),
            }),
        }
    }

    fn failing(message: &str) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            reply: Err(TransportFailure {
                message: String::from(message),
            }),
        }
    }

    fn requests(&self) -> Vec<ModelsRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn only_url(&self) -> String {
        let mut requests = self.requests();
        assert_eq!(requests.len(), 1, "the listing makes one request");
        requests.remove(0).url
    }
}

impl ModelsTransport for Transport {
    type Error = TransportFailure;

    fn get(&self, request: &ModelsRequest) -> Result<ModelsResponse, Self::Error> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        self.reply.clone()
    }
}

#[test]
fn the_listing_reads_the_ids_of_a_successful_reply() {
    let transport = Transport::replying(
        200,
        r#"{"data":[{"id":"openai/gpt-4o"},{"id":"anthropic/claude"}]}"#,
    );
    let fixture = Fixture::new();
    let ids = get_models(
        &transport,
        Some("http://passed"),
        DispatchSettings::from(&fixture.settings),
    )
    .expect("the listing answers");
    assert_eq!(
        ids,
        vec![
            String::from("openai/gpt-4o"),
            String::from("anthropic/claude")
        ]
    );
}

#[test]
fn the_url_appends_models_to_the_resolved_base_without_trimming() {
    let fixture = Fixture::new();
    let transport = Transport::replying(200, r#"{"data":[]}"#);
    get_models(
        &transport,
        Some("http://passed"),
        DispatchSettings::from(&fixture.settings),
    )
    .expect("the listing answers");
    assert_eq!(transport.only_url(), "http://passed/models");

    let slashed = Transport::replying(200, r#"{"data":[]}"#);
    get_models(
        &slashed,
        Some("http://passed/"),
        DispatchSettings::from(&fixture.settings),
    )
    .expect("the listing answers");
    assert_eq!(
        slashed.only_url(),
        "http://passed//models",
        "LiteLLM appends without trimming the passed base"
    );
}

#[test]
fn the_url_falls_from_the_passed_base_to_the_environment_to_the_default() {
    let mut fixture = Fixture::new();
    fixture.settings.environment.vercel_ai_gateway_api_base = Some(String::from("http://env"));
    let transport = Transport::replying(200, r#"{"data":[]}"#);
    get_models(&transport, None, DispatchSettings::from(&fixture.settings))
        .expect("the listing answers");
    assert_eq!(transport.only_url(), "http://env/models");

    let empty = Transport::replying(200, r#"{"data":[]}"#);
    get_models(&empty, Some(""), DispatchSettings::from(&fixture.settings))
        .expect("the listing answers");
    assert_eq!(
        empty.only_url(),
        "http://env/models",
        "an empty base falls through"
    );

    fixture.settings.environment.vercel_ai_gateway_api_base = None;
    let default = Transport::replying(200, r#"{"data":[]}"#);
    get_models(&default, None, DispatchSettings::from(&fixture.settings))
        .expect("the listing answers");
    assert_eq!(default.only_url(), "https://ai-gateway.vercel.sh/v1/models");
}

#[test]
fn a_configured_key_changes_nothing_in_the_request() {
    let fixture = Fixture::new();
    let mut keyed = Fixture::new();
    keyed.settings.environment.vercel_ai_gateway_api_key = Some(secret("sk-vercel"));
    keyed.settings.environment.vercel_oidc_token = Some(secret("oidc"));
    let plain_transport = Transport::replying(200, r#"{"data":[]}"#);
    let keyed_transport = Transport::replying(200, r#"{"data":[]}"#);
    get_models(
        &plain_transport,
        None,
        DispatchSettings::from(&fixture.settings),
    )
    .expect("the listing answers");
    get_models(
        &keyed_transport,
        None,
        DispatchSettings::from(&keyed.settings),
    )
    .expect("the listing answers");
    assert_eq!(
        plain_transport.requests(),
        keyed_transport.requests(),
        "the listing sends no Authorization header"
    );
}

#[rstest]
#[case::server(500, "boom")]
#[case::unauthorized(401, "no key")]
fn a_non_success_status_is_the_reply_error(#[case] status: u16, #[case] text: &str) {
    let fixture = Fixture::new();
    let transport = Transport::replying(status, text);
    let error = get_models(&transport, None, DispatchSettings::from(&fixture.settings))
        .expect_err("the listing refuses a non-200 reply");
    let ModelsError::Reply(VercelModelsError::Status {
        code,
        text: reply_text,
    }) = &error
    else {
        panic!("the reply error carries the status");
    };
    assert_eq!(*code, status);
    assert_eq!(reply_text, text);
    assert_eq!(error.to_string(), format!("Failed to get models: {text}"));
}

#[test]
fn a_body_that_is_not_a_model_list_is_the_shape_error() {
    let fixture = Fixture::new();
    let transport = Transport::replying(200, "not json");
    let error = get_models(&transport, None, DispatchSettings::from(&fixture.settings))
        .expect_err("the listing refuses an unreadable reply");
    assert!(matches!(
        error,
        ModelsError::Reply(VercelModelsError::Shape(_))
    ));
}

#[test]
fn a_transport_failure_names_the_underlying_message() {
    let fixture = Fixture::new();
    let transport = Transport::failing("connection refused");
    let error = get_models(&transport, None, DispatchSettings::from(&fixture.settings))
        .expect_err("the listing surfaces the transport error");
    let source = error.source().expect("the transport error is kept");
    assert_eq!(source.to_string(), "connection refused");
    assert_eq!(
        error.to_string(),
        "the models request failed: connection refused"
    );
}
