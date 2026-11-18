use std::time::Duration;

use betterlite_http::interactions::base::Logging;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;

use crate::api::interactions::lifecycle::{acancel, cancel};

use super::support::{
    Call, Script, answer, base, deps, fixture, host, redactor, registry_with, state, text,
};

#[test]
fn cancel_posts_the_transformed_body_and_withdraws_its_own_async_flag() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("int-3")
        .at(serving.address)
        .kwarg("adelete_interaction", Json::Bool(true))
        .kwarg("acancel_interaction", Json::Bool(true));
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let result = cancel(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .expect("the fixture answers");
    assert_eq!(result.id.as_deref(), Some("int-1"));
    assert_eq!(result.status.as_deref(), Some("cancelled"));
    assert_eq!(
        logging.optional_params.get("interaction_id"),
        Some(&text("int-3"))
    );
    assert_eq!(logging.custom_llm_provider.as_deref(), Some("gemini"));
    let params = config.params();
    assert_eq!(params.get("acancel_interaction"), None);
    assert_eq!(
        params.get("adelete_interaction"),
        Some(&JsonValue::Bool(true))
    );
    let received = serving.finish().unwrap();
    assert_eq!(
        received.line,
        "POST /v1beta/interactions/int-3:cancel HTTP/1.1"
    );
    assert_eq!(received.body, r#"{"state":"cancelled"}"#);
}

#[tokio::test]
async fn acancel_keeps_the_callers_logging_details_and_records_its_pre_call() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("interaction-1").at(serving.address);
    let details = JsonValue::Text("seeded".to_owned());
    let mut http = Logging::default();
    http.model_call_details
        .insert("seeded".to_owned(), details.clone());
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    acancel(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .await
    .expect("the cancellation answers");
    let pre_call = http
        .pre_call
        .as_ref()
        .expect("the handler records its pre-call");
    assert_eq!(pre_call.input, "interaction-1");
    assert_eq!(
        pre_call.api_base,
        format!(
            "{}/v1beta/interactions/interaction-1:cancel",
            base(serving.address)
        )
    );
    assert_eq!(http.model_call_details.get("seeded"), Some(&details));
    drop(serving.finish().unwrap());
}

#[tokio::test]
async fn acancel_posts_the_transformed_body_without_settling() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("interaction-1").at(serving.address);
    let settlement_host = host();
    let _registry = registry_with(settlement_host.clone());
    let script = Script::returning(answer());
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let result = acancel(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
    )
    .await
    .expect("the cancellation answers");
    assert_eq!(result.id.as_deref(), Some("int-1"));
    assert_eq!(script.calls(), 0);
    assert!(settlement_host.billed().is_empty());
    let received = serving.finish().unwrap();
    assert_eq!(
        received.line,
        "POST /v1beta/interactions/interaction-1:cancel HTTP/1.1"
    );
    assert_eq!(received.body, r#"{"state":"cancelled"}"#);
}
