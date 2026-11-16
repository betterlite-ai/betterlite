#![cfg(test)]

mod support;

use betterlite_error::error::ErrorClass;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::call::prepare;
use super::error::InteractionGetError;
use super::{aget, get};
use support::{
    Call, FixtureError, GET_BODY, Harness, base, environment_headers, environment_params, fixture,
    head_of, number, reply, request_line, request_of, response_of, serve, text,
};

#[test]
fn get_defaults_to_gemini_when_no_provider_is_named() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").kwarg("api_base", text(&base(serving.address)));
    let answer = get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(answer.id.as_deref(), Some("int_1"));
    assert_eq!(answer.model.as_deref(), Some("gemini-2.5-flash"));
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        environment_params(&config).get("custom_llm_provider"),
        Some(&JsonValue::Text("gemini".to_owned()))
    );
    assert_eq!(
        request_line(&head_of(serving)),
        "GET /v1beta/interactions/int_1 HTTP/1.1"
    );
}

#[rstest]
#[case::gemini("gemini")]
#[case::vertex("vertex_ai")]
#[case::vertex_beta("vertex_ai_beta")]
fn get_accepts_every_provider_with_an_interactions_config(#[case] provider: &str) {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1")
        .provider(provider)
        .kwarg("api_base", text(&base(serving.address)));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some(provider));
    assert_eq!(
        environment_params(&config).get("custom_llm_provider"),
        Some(&JsonValue::Text(provider.to_owned()))
    );
    drop(head_of(serving));
}

#[rstest]
#[case::gemini("gemini", ProviderId::Gemini)]
#[case::vertex("vertex_ai", ProviderId::VertexAi)]
#[case::vertex_beta("vertex_ai_beta", ProviderId::VertexAiBeta)]
fn prepare_keys_each_provider_with_its_own_client(
    #[case] provider: &str,
    #[case] expected: ProviderId,
) {
    let mut harness = Harness::new();
    let call = Call::new("int_1").provider(provider);
    let prepared = prepare::<FixtureError>(&call.request(), &mut harness.state, &harness.redactor)
        .expect("the provider has an interactions config");
    assert_eq!(prepared.client, expected);
}

#[test]
fn get_takes_the_provider_from_the_kwargs() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("custom_llm_provider", text("vertex_ai"));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(
        harness.state.custom_llm_provider.as_deref(),
        Some("vertex_ai")
    );
    drop(head_of(serving));
}

#[test]
fn get_prefers_the_argument_provider_over_the_kwargs() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1")
        .provider("gemini")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("custom_llm_provider", text("vertex_ai"));
    get(&config, harness.deps(), &call.request()).expect("the argument provider is gemini");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    drop(head_of(serving));
}

#[test]
fn a_null_kwargs_provider_keeps_the_null_in_the_parameters() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("custom_llm_provider", Json::Null);
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        environment_params(&config).get("custom_llm_provider"),
        Some(&JsonValue::Null)
    );
    drop(head_of(serving));
}

#[test]
fn get_reports_an_unsupported_provider_as_a_value_error() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").provider("openai");
    let error = get(&config, harness.deps(), &call.request()).expect_err("openai is unsupported");
    assert_eq!(
        error.error.to_string(),
        "Interactions API not supported for: openai"
    );
    assert!(matches!(
        &error.error,
        InteractionGetError::Unsupported { provider } if provider.as_str() == "openai"
    ));
    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection error, got {:?}", error.class);
    };
    assert_eq!(context.model.as_deref(), Some(""));
    assert_eq!(context.provider.as_deref(), Some("openai"));
    assert!(
        context
            .message
            .contains("Interactions API not supported for: openai")
    );
}

#[test]
fn an_explicit_empty_provider_is_used_as_given() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").provider("");
    let error = get(&config, harness.deps(), &call.request()).expect_err("no provider is named");
    assert_eq!(
        error.error.to_string(),
        "Interactions API not supported for: "
    );
}

#[test]
fn get_reports_a_non_string_provider_as_a_bad_request_naming_the_field() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").kwarg("custom_llm_provider", number(5));
    let error = get(&config, harness.deps(), &call.request()).expect_err("a number is no provider");
    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("expected a bad request, got {:?}", error.class);
    };
    assert_eq!(details.body_param.as_deref(), Some("custom_llm_provider"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.status.0, 400);
}

#[test]
fn get_leaves_the_logging_object_alone_for_an_unsupported_provider() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").provider("openai");
    get(&config, harness.deps(), &call.request()).expect_err("openai is unsupported");
    assert!(harness.state.optional_params.is_empty());
    assert!(
        !harness
            .state
            .model_call_details
            .contains_key("interaction_id")
    );
    assert_eq!(harness.state.custom_llm_provider, None);
}

#[test]
fn get_records_the_interaction_id_and_the_call_id_before_the_request() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_7")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("litellm_call_id", text("call-42"));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(harness.state.model, "");
    assert_eq!(harness.state.optional_params.len(), 1);
    assert_eq!(
        harness.state.optional_params.get("interaction_id"),
        Some(&text("int_7"))
    );
    assert_eq!(
        harness.state.model_call_details.get("interaction_id"),
        Some(&text("int_7"))
    );
    assert_eq!(
        harness.state.litellm_params.get("litellm_call_id"),
        Some(&text("call-42"))
    );
    assert_eq!(
        harness.state.model_call_details.get("custom_llm_provider"),
        Some(&text("gemini"))
    );
    drop(head_of(serving));
}

#[test]
fn get_records_a_null_call_id_when_the_caller_names_none() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_7").kwarg("api_base", text(&base(serving.address)));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(
        harness.state.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    drop(head_of(serving));
}

#[test]
fn get_drops_the_async_flag_from_the_transport_parameters() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("aget_interaction", Json::Bool(true))
        .kwarg("api_version", text("v1"));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    let params = environment_params(&config);
    assert!(!params.contains_key("aget_interaction"));
    assert_eq!(
        params.get("api_version"),
        Some(&JsonValue::Text("v1".to_owned()))
    );
    drop(head_of(serving));
}

#[test]
fn get_forwards_the_extra_headers_and_the_interaction_id_to_the_provider() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_9")
        .header("x-extra", "yes")
        .kwarg("api_base", text(&base(serving.address)));
    get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(
        environment_headers(&config),
        vec![("x-extra".to_owned(), "yes".to_owned())]
    );
    let (interaction_id, _headers) = request_of(&config);
    assert_eq!(interaction_id, "int_9");
    drop(head_of(serving));
}

#[test]
fn get_returns_the_transformed_response_and_records_the_pre_call() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").kwarg("api_base", text(&base(serving.address)));
    let answer = get(&config, harness.deps(), &call.request()).expect("the fixture answers");
    assert_eq!(answer.id.as_deref(), Some("int_1"));
    let (status, input) = response_of(&config);
    assert_eq!(status, 200);
    assert_eq!(input, "int_1");
    let pre_call = harness
        .record
        .pre_call
        .as_ref()
        .expect("the pre call is recorded");
    assert_eq!(pre_call.input, "int_1");
    assert_eq!(pre_call.api_key.expose_secret(), "");
    assert_eq!(
        pre_call.api_base,
        format!("{}/v1beta/interactions/int_1", base(serving.address))
    );
    drop(head_of(serving));
}

#[test]
fn get_maps_a_provider_status_to_its_error_class() {
    let serving = serve(reply(
        404,
        r#"{"error": {"message": "no such interaction"}}"#,
    ));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").kwarg("api_base", text(&base(serving.address)));
    let error = get(&config, harness.deps(), &call.request()).expect_err("the fixture refuses");
    assert!(matches!(&error.error, InteractionGetError::Provider(_)));
    let ErrorClass::NotFound(details) = &error.class else {
        panic!("expected a not found error, got {:?}", error.class);
    };
    assert_eq!(details.status.0, 404);
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("gemini"));
    drop(head_of(serving));
}

#[test]
fn get_reports_a_request_that_cannot_be_built_as_an_api_connection_error() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1");
    let error = get(&config, harness.deps(), &call.request()).expect_err("the url is relative");
    let ErrorClass::ApiConnection(context) = &error.class else {
        panic!("expected an api connection error, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("gemini"));
    assert!(context.message.contains("request URL is not valid"));
}

#[tokio::test]
async fn aget_answers_through_the_async_transport() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").kwarg("api_base", text(&base(serving.address)));
    let answer = aget(&config, harness.deps(), &call.request())
        .await
        .expect("the fixture answers");
    assert_eq!(answer.id.as_deref(), Some("int_1"));
    assert_eq!(harness.state.custom_llm_provider.as_deref(), Some("gemini"));
    assert_eq!(
        request_line(&head_of(serving)),
        "GET /v1beta/interactions/int_1 HTTP/1.1"
    );
}

#[tokio::test]
async fn aget_carries_the_call_context_into_the_async_path() {
    let serving = serve(reply(200, GET_BODY));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_5")
        .header("x-extra", "yes")
        .kwarg("api_base", text(&base(serving.address)))
        .kwarg("litellm_call_id", text("call-5"));
    aget(&config, harness.deps(), &call.request())
        .await
        .expect("the fixture answers");
    assert_eq!(
        harness.state.optional_params.get("interaction_id"),
        Some(&text("int_5"))
    );
    assert_eq!(
        harness.state.litellm_params.get("litellm_call_id"),
        Some(&text("call-5"))
    );
    assert_eq!(response_of(&config).1, "int_5");
    let pre_call = harness
        .record
        .pre_call
        .as_ref()
        .expect("the pre call is recorded");
    assert_eq!(pre_call.input, "int_5");
    assert_eq!(
        environment_headers(&config),
        vec![("x-extra".to_owned(), "yes".to_owned())]
    );
    drop(head_of(serving));
}

#[tokio::test]
async fn aget_reports_an_unsupported_provider_as_a_value_error() {
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").provider("openai");
    let error = aget(&config, harness.deps(), &call.request())
        .await
        .expect_err("openai is unsupported");
    assert_eq!(
        error.error.to_string(),
        "Interactions API not supported for: openai"
    );
    assert!(matches!(&error.class, ErrorClass::ApiConnection(_)));
}

#[tokio::test]
async fn aget_reports_a_provider_status_to_its_error_class() {
    let serving = serve(reply(500, r#"{"error": {"message": "broken"}}"#));
    let config = fixture();
    let mut harness = Harness::new();
    let call = Call::new("int_1").kwarg("api_base", text(&base(serving.address)));
    let error = aget(&config, harness.deps(), &call.request())
        .await
        .expect_err("the fixture refuses");
    let ErrorClass::InternalServer(context) = &error.class else {
        panic!("expected an internal server error, got {:?}", error.class);
    };
    assert_eq!(context.provider.as_deref(), Some("gemini"));
    drop(head_of(serving));
}
