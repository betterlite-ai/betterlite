use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::decisions::call::azure::AzureAiConfig;
use betterlite_openai::decisions::call::fields::{RequestFields, SystemOneRequestFields};
use betterlite_openai::decisions::call::{PrepareCall, prepare_call};
use http::header::HeaderValue;
use secrecy::SecretString;
use serde_json::Number;

use super::support::*;
use crate::api::decisions_send::clients::DecisionsBody;
use crate::api::decisions_send::format::DecisionsOutput;
use crate::api::decisions_send::send::DecisionsSender;

// send_behavior: one POST of the transformed body with the validated headers

#[test]
fn send_behavior_posts_transformed_body_with_bearer_headers() {
    let serving = serve(reply("200 OK", &[], SYSTEMONE_ANSWER), Duration::default())
        .expect("the mock server starts");
    let api_base = format!("http://{}", serving.address);
    let catalog = ModelCatalog::default();
    let known = BTreeSet::new();
    let custom = BTreeSet::new();
    let environment = BTreeMap::new();
    let resolution = ResolutionContext {
        catalog: &catalog,
        known_model_names: &known,
        custom_providers: &custom,
        environment: &environment,
        registry: JSONProviderRegistry::default(),
        policies: ProviderPolicies::default(),
        globals: GlobalCredentials::default(),
    };
    let key = SecretString::from("test-key".to_owned());
    let logging = Mutex::new(logging_state());
    let questions =
        serde_json::from_str(r#"{"first":{"type":"noul","instructions":"Is it true?"}}"#)
            .expect("the questions parse");
    let decisions = prepare_call(PrepareCall {
        model: "model-1",
        fields: RequestFields::SystemOne(SystemOneRequestFields {
            state: Some(Json::Text(String::from("state"))),
            questions: Some(questions),
        }),
        safety_identifier: None,
        api_key: Some(&key),
        api_base: Some(&api_base),
        timeout: None,
        custom_llm_provider: Some("typesafe"),
        extra_headers: None,
        kwargs: Extra::new(),
        global_drop_params: false,
        globals: LiteLlmGlobals {
            api_base: None,
            api_key: None,
            openai_key: None,
            organization: None,
        },
        azure_ai: AzureAiConfig {
            complete_url: Box::new(|api_base: &str, _model: &str| api_base.to_owned()),
        },
        resolution: &resolution,
        logging_obj: Some(&logging),
    })
    .expect("the call prepares");
    let expected_body = serde_json::to_string(&decisions.body).expect("the wire serializes");
    let mut clients = real_clients();
    let log = LogEnv::for_events(&Events::new());
    let env = log.env();
    let mut sender = DecisionsSender::new(&mut clients, &env);
    let output = sender.send(&decisions).expect("the mock answers");
    match output {
        DecisionsOutput::SystemOne(response) => {
            assert_eq!(answer_names(&response), vec![String::from("first")]);
            let usage = response.usage.expect("the conversion sets usage");
            assert_eq!(usage.input_tokens.0, Number::from(3_u8));
        }
        DecisionsOutput::OpenAi(_) => {
            panic!("a SystemOne request answers with a SystemOne response")
        }
    }
    let request = serving.finish().expect("the server saw the request");
    assert_eq!(request.line, "POST /v1/systemone HTTP/1.1");
    assert!(
        request
            .head
            .to_lowercase()
            .contains("authorization: bearer test-key")
    );
    assert!(
        request
            .head
            .to_lowercase()
            .contains("content-type: application/json")
    );
    assert_eq!(request.body, expected_body);
}

#[tokio::test]
async fn send_behavior_async_answers_and_selects_the_provider_client() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    let output = fixture.asend(&decisions).await.expect("the call answers");
    assert!(matches!(output, DecisionsOutput::SystemOne(_)));
    let selections = fixture.clients.selections();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].0, Mode::Asynchronous);
    assert_eq!(
        selections[0].1.get("custom_llm_provider"),
        Some(&JsonValue::Text(String::from("typesafe")))
    );
}

#[test]
fn send_behavior_sync_selects_the_default_client() {
    let mut fixture = Fixture::new();
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let selections = fixture.clients.selections();
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].0, Mode::Sync);
    assert!(selections[0].1.is_empty());
}

#[test]
fn send_behavior_passes_the_timeout_and_posts_once() {
    let mut fixture = Fixture::new();
    let mut decisions = systemone_call("typesafe", None);
    decisions.timeout = Some(Duration::from_secs(2));
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].timeout, Some(Duration::from_secs(2)));
    match &requests[0].body {
        DecisionsBody::Json(bytes) => {
            let sent = serde_json::to_string(&decisions.body).expect("the wire serializes");
            assert_eq!(bytes, sent.as_bytes());
        }
        DecisionsBody::Signed(_) => panic!("an unsigned config sends JSON"),
    }
}

#[test]
fn send_behavior_sends_the_signed_headers() {
    let mut fixture = Fixture::new();
    fixture.config.signature = Some(String::from("sig-1"));
    let decisions = systemone_call("typesafe", None);
    fixture.send(&decisions).expect("the call answers");
    let requests = fixture.wire.requests();
    assert_eq!(
        requests[0].headers.get("x-signature"),
        Some(&HeaderValue::from_static("sig-1"))
    );
    assert_eq!(
        requests[0].headers.get("content-type"),
        Some(&HeaderValue::from_static("application/json"))
    );
}
