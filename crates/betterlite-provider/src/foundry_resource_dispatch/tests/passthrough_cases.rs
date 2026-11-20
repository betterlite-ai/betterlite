use betterlite_foundry::passthrough::error::FoundryPassthroughError;
use betterlite_foundry::passthrough::logging::FoundryRelayedLog;
use betterlite_foundry::passthrough::plan::{FoundryLogInput, FoundryPassthroughPlan};
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};

use super::fixture::{RecordingPassthrough, parse, passthrough_plan};
use crate::foundry_resource_dispatch::passthrough::{self, Call, Dispatch, Outcome};
use crate::foundry_resource_dispatch::{
    FoundryResourceError, FoundryResourceOperation, FoundryResourceResult, dispatch,
};

async fn run(
    call: Call<'_>,
    plan: &FoundryPassthroughPlan,
    logging: &mut RecordingPassthrough,
) -> Result<FoundryResourceResult, FoundryResourceError> {
    dispatch(FoundryResourceOperation::Passthrough(Box::new(Dispatch {
        plan,
        logging,
        call,
    })))
    .await
}

async fn outcome(
    call: Call<'_>,
    plan: &FoundryPassthroughPlan,
    logging: &mut RecordingPassthrough,
) -> Outcome {
    match run(call, plan, logging).await {
        Ok(FoundryResourceResult::Passthrough(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the passthrough call returns its own outcome"),
    }
}

async fn leaf_error(
    call: Call<'_>,
    plan: &FoundryPassthroughPlan,
    logging: &mut RecordingPassthrough,
) -> passthrough::Error {
    match run(call, plan, logging).await {
        Err(FoundryResourceError::Passthrough(error)) => error,
        Ok(_) | Err(_) => panic!("the passthrough call fails in its leaf"),
    }
}

fn target<'call>(model: &'call str, endpoint: &'call str) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure_ai",
        endpoint,
    }
}

async fn url_of(
    plan: &FoundryPassthroughPlan,
    api_base: Option<&str>,
    endpoint: &str,
    litellm_params: &Object,
) -> (String, String) {
    let mut logging = RecordingPassthrough::default();
    let request = PassthroughUrlRequest {
        api_base,
        api_key: None,
        target: target("gpt-x", endpoint),
        request_query_params: None,
        litellm_params,
    };
    let Outcome::Url { url, root } = outcome(Call::Url(request), plan, &mut logging).await else {
        panic!("the url call returns the relay url");
    };
    (url, root)
}

async fn url_rows() {
    let plan = passthrough_plan(Object::new());
    let params = Object::new();
    let (url, root) = url_of(&plan, None, "openai/v1/responses", &params).await;
    assert_eq!(root, "https://res.test");
    assert_eq!(url, "https://res.test/openai/v1/responses");
    let (custom, custom_root) = url_of(
        &plan,
        Some("https://custom.test/"),
        "openai/v1/responses",
        &params,
    )
    .await;
    assert_eq!(custom_root, "https://custom.test");
    assert_eq!(custom, "https://custom.test/openai/v1/responses");
    let (repeated, repeated_root) = url_of(
        &plan,
        Some("https://res.test/openai/v1"),
        "openai/v1/responses",
        &params,
    )
    .await;
    assert_eq!(repeated_root, "https://res.test");
    assert_eq!(repeated, "https://res.test/openai/v1/responses");

    let versioned: Object = parse(r#"{"api_version":"2024-10-21"}"#);
    let (with_version, _) = url_of(&plan, None, "openai/v1/responses", &versioned).await;
    assert_eq!(
        with_version,
        "https://res.test/openai/v1/responses?api-version=2024-10-21"
    );

    let mut bare = passthrough_plan(Object::new());
    bare.azure.api_base_env = None;
    let mut logging = RecordingPassthrough::default();
    let request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: target("gpt-x", "openai/v1/responses"),
        request_query_params: None,
        litellm_params: &params,
    };
    let missing = leaf_error(Call::Url(request), &bare, &mut logging).await;
    assert!(matches!(
        missing,
        passthrough::Error::Passthrough(FoundryPassthroughError::MissingApiBase)
    ));
}

fn logged<'call>(
    target: &'call PassthroughTarget<'call>,
    reply: &'call ResponseReply<'call>,
    request_data: &'call Object,
) -> FoundryLogInput<'call> {
    FoundryLogInput {
        target,
        reply,
        request_data,
        request_url: "https://res.test/unknown",
    }
}

async fn relay_rows() {
    let plan = passthrough_plan(Object::new());
    let mut logging = RecordingPassthrough::default();
    let headers = Headers::new();
    let params = Object::new();
    let unknown = target("gpt-x", "unknown/route");
    let body = br#"{"ok":true}"#;
    let reply = ResponseReply {
        status: 200,
        headers: &headers,
        body: &body[..],
    };
    let Outcome::Relayed(found) = outcome(
        Call::Relayed(logged(&unknown, &reply, &params)),
        &plan,
        &mut logging,
    )
    .await
    else {
        panic!("the relayed call returns its projection");
    };
    let Some(log) = found else {
        panic!("an unknown route still projects its standard response");
    };
    let FoundryRelayedLog::Standard(standard) = *log else {
        panic!("an unknown route keeps the standard projection unchanged");
    };
    let expected: Json = parse(r#"{"ok":true}"#);
    assert_eq!(standard.response, expected);
    assert!(logging.call_types.is_empty());
}

async fn relay_shape_rows() {
    let plan = passthrough_plan(Object::new());
    let mut logging = RecordingPassthrough::default();
    let headers = Headers::new();
    let params = Object::new();
    let rerank = target("gpt-x", "v1/rerank");
    let body =
        br#"{"id":"r1","results":[{"index":0,"relevance_score":0.5,"document":{"text":"a"}}]}"#;
    let reply = ResponseReply {
        status: 200,
        headers: &headers,
        body: &body[..],
    };
    let Outcome::Relayed(found) = outcome(
        Call::Relayed(logged(&rerank, &reply, &params)),
        &plan,
        &mut logging,
    )
    .await
    else {
        panic!("the relayed call returns its projection");
    };
    let Some(log) = found else {
        panic!("a known relay shape projects its body");
    };
    let FoundryRelayedLog::Relay(object) = *log else {
        panic!("a rerank reply uses the relay projection");
    };
    assert!(object.contains_key("results"));
    assert_eq!(logging.call_types, [RelayCallType::Arerank]);
}

pub(super) async fn check() {
    url_rows().await;
    relay_rows().await;
    relay_shape_rows().await;
}
