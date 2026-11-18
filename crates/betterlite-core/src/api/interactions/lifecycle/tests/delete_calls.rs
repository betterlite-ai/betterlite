use std::net::TcpListener;
use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::base::Logging;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::interactions::lifecycle::delete;
use crate::api::interactions::lifecycle::error::InteractionLifecycleError;

use super::support::{Call, base, deps, fixture, redactor, state, text};

#[test]
fn delete_reaches_the_provider_config_and_answers_with_its_result() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("int-7")
        .header("x-extra", "yes")
        .kwarg("litellm_call_id", text("call-9"))
        .at(serving.address);
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let result = delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect("the fixture answers");
    assert_eq!(result.id.as_deref(), Some("int-7"));
    assert!(result.success);
    assert_eq!(
        logging.optional_params.get("interaction_id"),
        Some(&text("int-7"))
    );
    assert_eq!(
        logging.litellm_params.get("litellm_call_id"),
        Some(&text("call-9"))
    );
    assert_eq!(logging.model, "gemini-2.5-flash");
    assert_eq!(logging.call_type, "generated");
    let received = serving.finish().unwrap();
    assert_eq!(received.line, "DELETE /v1beta/interactions/int-7 HTTP/1.1");
    assert_eq!(received.body, "");
    assert!(received.head.to_lowercase().contains("x-extra: yes"));
    assert_eq!(config.params().get("custom_llm_provider"), None);
}

#[test]
fn delete_keeps_the_callers_logging_details_and_records_its_pre_call() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("int-4").at(serving.address);
    let details = JsonValue::Object(IndexMap::from([(
        "model".to_owned(),
        JsonValue::Text("gemini-2.5-flash".to_owned()),
    )]));
    let mut http = Logging::default();
    http.model_call_details
        .insert("litellm_params".to_owned(), details.clone());
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect("the fixture answers");
    let pre_call = http
        .pre_call
        .as_ref()
        .expect("the handler records its pre-call");
    assert_eq!(pre_call.input, "int-4");
    assert_eq!(
        pre_call.api_base,
        format!("{}/v1beta/interactions/int-4", base(serving.address))
    );
    assert_eq!(
        http.model_call_details.get("litellm_params"),
        Some(&details)
    );
    drop(serving.finish().unwrap());
}

#[test]
fn delete_carries_every_json_shape_into_the_handler_parameters() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let object = IndexMap::from([("flag".to_owned(), Json::Bool(true))]);
    let call = Call::new("int-1")
        .at(serving.address)
        .kwarg("nothing", Json::Null)
        .kwarg("count", Json::Number(Number::from(3_i32)))
        .kwarg("names", Json::List(vec![Json::Text("a".to_owned())]))
        .kwarg("object", Json::Object(object));
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect("the fixture answers");
    let params = config.params();
    assert_eq!(params.get("nothing"), Some(&JsonValue::Null));
    assert_eq!(
        params.get("count"),
        Some(&JsonValue::Number(Number::from(3_i32)))
    );
    assert_eq!(
        params.get("names"),
        Some(&JsonValue::Array(vec![JsonValue::Text("a".to_owned())]))
    );
    assert_eq!(
        params.get("object"),
        Some(&JsonValue::Object(IndexMap::from([(
            "flag".to_owned(),
            JsonValue::Bool(true)
        )])))
    );
    drop(serving.finish().unwrap());
}

#[test]
fn delete_logs_the_metadata_the_caller_sent() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let metadata = IndexMap::from([("user_api_key".to_owned(), text("k"))]);
    let call = Call::new("int-1")
        .at(serving.address)
        .kwarg("metadata", Json::Object(metadata.clone()));
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect("the fixture answers");
    assert_eq!(
        logging.litellm_params.get("metadata"),
        Some(&Json::Object(metadata))
    );
    drop(serving.finish().unwrap());
}

#[rstest::rstest]
#[case::absent(None, "gemini")]
#[case::empty(Some(""), "gemini")]
#[case::named(Some("vertex_ai"), "vertex_ai")]
fn delete_resolves_the_provider(#[case] provider: Option<&str>, #[case] expected: &str) {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let mut call = Call::new("int-1").at(serving.address);
    if let Some(named) = provider {
        call = call.provider(named);
    }
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect("the fixture answers");
    assert_eq!(logging.custom_llm_provider.as_deref(), Some(expected));
    assert_eq!(
        logging.model_call_details.get("custom_llm_provider"),
        Some(&text(expected))
    );
    drop(serving.finish().unwrap());
}

#[test]
fn delete_rejects_a_provider_without_an_interactions_api() {
    let config = fixture();
    let call = Call::new("int-1").provider("openai");
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let failed = delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect_err("openai has no interactions API");
    assert_eq!(
        failed.error.to_string(),
        "Interactions API not supported for: openai"
    );
    assert!(matches!(failed.class, ErrorClass::ApiConnection(_)));
    assert!(logging.optional_params.is_empty());
    assert!(logging.model_call_details.get("interaction_id").is_none());
}

#[test]
fn delete_withdraws_the_async_flag_before_the_parameters_are_built() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("int-1")
        .at(serving.address)
        .kwarg("adelete_interaction", Json::Bool(true))
        .kwarg("acancel_interaction", Json::Bool(true));
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect("the fixture answers");
    let params = config.params();
    assert_eq!(params.get("adelete_interaction"), None);
    assert_eq!(
        params.get("acancel_interaction"),
        Some(&JsonValue::Bool(true))
    );
    drop(serving.finish().unwrap());
}

#[test]
fn delete_wraps_a_provider_error_with_its_status() {
    let serving = serve(
        reply("404 fixture", &[], "no such interaction"),
        Duration::ZERO,
    )
    .unwrap();
    let config = fixture();
    let call = Call::new("int-1").at(serving.address);
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let failed = delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect_err("a 404 fails the deletion");
    let InteractionLifecycleError::Provider(error) = &failed.error else {
        panic!("expected the provider's own error, got {:?}", failed.error);
    };
    assert_eq!(error.status, 404);
    assert_eq!(error.message, "no such interaction");
    assert!(matches!(failed.class, ErrorClass::NotFound(_)));
    drop(serving.finish().unwrap());
}

#[test]
fn delete_rejects_a_fallback_timeout_it_cannot_turn_into_a_duration() {
    let config = fixture();
    let call = Call::new("int-1").seconds(f64::NAN);
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let failed = delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect_err("NaN is not a duration");
    assert!(matches!(
        failed.error,
        InteractionLifecycleError::Timeout { .. }
    ));
    assert!(matches!(failed.class, ErrorClass::ApiConnection(_)));
    assert_eq!(
        logging.optional_params.get("interaction_id"),
        Some(&text("int-1"))
    );
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let bounded = Call::new("int-1")
        .at(serving.address)
        .seconds(f64::NAN)
        .timeout(Duration::from_secs(5));
    delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &bounded.request(),
    )
    .expect("the explicit timeout is usable");
    drop(serving.finish().unwrap());
}

#[test]
fn delete_rejects_a_litellm_parameter_outside_its_type() {
    let config = fixture();
    let call = Call::new("int-1").kwarg("ssl_verify", text("yes"));
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let failed = delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect_err("ssl_verify must be a boolean");
    let ErrorClass::BadRequest(details) = &failed.class else {
        panic!("expected a bad request, got {:?}", failed.class);
    };
    assert_eq!(details.body_param.as_deref(), Some("ssl_verify"));
    assert_eq!(details.model.as_deref(), Some(""));
    assert_eq!(details.provider.as_deref(), Some("gemini"));
    assert_eq!(details.status.0, 400);
}

#[test]
fn delete_reports_a_transport_failure_through_the_value_error_class() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("the bound address");
    drop(listener);
    let config = fixture();
    let call = Call::new("int-1").at(address);
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let failed = delete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect_err("nothing listens on that port");
    assert!(matches!(
        failed.error,
        InteractionLifecycleError::Transport(_)
    ));
    assert!(matches!(failed.class, ErrorClass::ApiConnection(_)));
}
