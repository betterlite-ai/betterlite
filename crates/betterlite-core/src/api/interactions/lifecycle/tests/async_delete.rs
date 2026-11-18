use std::time::Duration;

use betterlite_error::error::ErrorClass;
use betterlite_http::interactions::base::Logging;
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::controls::json_value::JsonValue;

use crate::api::interactions::lifecycle::adelete;
use crate::api::interactions::lifecycle::error::InteractionLifecycleError;
use crate::api::interactions::lifecycle::request::BackgroundSettlement;

use super::support::{
    Call, Script, answer, base, deps, fixture, host, redactor, registry_with, state,
};

#[tokio::test]
async fn adelete_settles_a_tracked_interaction_before_deleting() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("interaction-1").at(serving.address);
    let settlement_host = host();
    let registry = registry_with(settlement_host.clone());
    let script = Script::returning(answer());
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let result = adelete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
        BackgroundSettlement {
            registry: &registry,
            fetch: script.fetch(),
        },
    )
    .await
    .expect("the interaction settles and then deletes");
    assert_eq!(result.id.as_deref(), Some("interaction-1"));
    assert_eq!(script.calls(), 1);
    assert_eq!(settlement_host.billed(), vec!["interaction-1".to_owned()]);
    assert_eq!(
        serving.finish().unwrap().line,
        "DELETE /v1beta/interactions/interaction-1 HTTP/1.1"
    );
}

#[tokio::test]
async fn adelete_keeps_the_callers_logging_details_and_records_its_pre_call() {
    let serving = serve(reply("200 fixture", &[], "{}"), Duration::ZERO).unwrap();
    let config = fixture();
    let call = Call::new("interaction-1").at(serving.address);
    let registry = registry_with(host());
    let script = Script::returning(answer());
    let details = JsonValue::Text("seeded".to_owned());
    let mut http = Logging::default();
    http.model_call_details
        .insert("seeded".to_owned(), details.clone());
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    adelete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
        BackgroundSettlement {
            registry: &registry,
            fetch: script.fetch(),
        },
    )
    .await
    .expect("the interaction settles and then deletes");
    let pre_call = http
        .pre_call
        .as_ref()
        .expect("the handler records its pre-call");
    assert_eq!(pre_call.input, "interaction-1");
    assert_eq!(
        pre_call.api_base,
        format!(
            "{}/v1beta/interactions/interaction-1",
            base(serving.address)
        )
    );
    assert_eq!(http.model_call_details.get("seeded"), Some(&details));
    drop(serving.finish().unwrap());
}

#[tokio::test]
async fn adelete_stops_the_delete_when_settlement_cannot_be_billed() {
    let config = fixture();
    let call = Call::new("interaction-1");
    let settlement_host = host().failing_billing();
    let registry = registry_with(settlement_host.clone());
    let script = Script::returning(answer());
    let mut http = Logging::default();
    let mut logging = state("gemini-2.5-flash", "call-1");
    let redactor = redactor();
    let failed = adelete(
        &config,
        deps(&mut http, &mut logging, &redactor),
        &call.request(),
        BackgroundSettlement {
            registry: &registry,
            fetch: script.fetch(),
        },
    )
    .await
    .expect_err("a billing failure stops the deletion");
    assert!(matches!(
        failed.error,
        InteractionLifecycleError::Settlement(_)
    ));
    assert_eq!(
        failed.error.to_string(),
        "the background interaction could not be settled"
    );
    let ErrorClass::ApiConnection(context) = &failed.class else {
        panic!("expected the value error class, got {:?}", failed.class);
    };
    assert_eq!(context.provider.as_deref(), Some("gemini"));
    assert_eq!(settlement_host.billed(), vec!["interaction-1".to_owned()]);
    assert!(logging.optional_params.is_empty());
}
