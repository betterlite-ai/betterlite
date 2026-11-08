use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{
    DATASET_BODY, Reply, ScriptedRandom, environment, logger, messages_json, payload, response_json,
};
use crate::argilla_logging::extraction::{ArgillaItem, CallbackKwargs, prepare_log_data};
use crate::argilla_logging::logger::ArgillaLogger;
use crate::argilla_logging::settings::{ArgillaSettings, ConstructorArguments, PayloadField};

async fn mapped(transformation: Json, kwargs: &CallbackKwargs) -> ArgillaItem {
    let settings = ArgillaSettings::from_environment(
        &environment(&[("ARGILLA_API_KEY", "key")]),
        &ConstructorArguments::default(),
        Some(&transformation),
    )
    .expect("the settings are valid");
    let transport = super::support::ScriptedTransport::answering(&[Reply::ok(DATASET_BODY)]);
    let built = ArgillaLogger::connect(settings, transport, ScriptedRandom::new(&[0.0_f64]))
        .await
        .expect("the logger connects");
    built.log_success_event(kwargs).await;
    built
        .queue
        .pending()
        .into_iter()
        .next()
        .expect("one record")
}

#[tokio::test]
async fn fills_every_transformation_key_in_order() {
    let transformation = Json::Object(IndexMap::from([
        ("prompt".to_owned(), Json::Text("messages".to_owned())),
        ("completion".to_owned(), Json::Text("response".to_owned())),
    ]));
    let record = mapped(transformation, &super::support::valid_kwargs()).await;
    assert_eq!(
        record.fields.keys().collect::<Vec<_>>(),
        vec!["prompt", "completion"]
    );
    assert_eq!(record.fields["prompt"], messages_json());
    assert_eq!(record.fields["completion"], Json::Text("Hello".to_owned()));
}

#[tokio::test]
async fn wraps_a_single_message_object_in_a_list() {
    let message = Json::Object(IndexMap::from([(
        "role".to_owned(),
        Json::Text("user".to_owned()),
    )]));
    let kwargs = payload(message.clone(), response_json());
    let record = mapped(
        Json::Object(IndexMap::from([(
            "input".to_owned(),
            Json::Text("messages".to_owned()),
        )])),
        &kwargs,
    )
    .await;
    assert_eq!(record.fields["input"], Json::List(vec![message]));
}

#[tokio::test]
async fn logs_nothing_without_a_standard_logging_object() {
    let built = logger(
        &[],
        &[Reply::ok(DATASET_BODY)],
        ScriptedRandom::new(&[0.0_f64]),
    )
    .await;
    built.log_success_event(&CallbackKwargs::default()).await;
    assert!(built.queue.pending().is_empty());
}

#[test]
fn maps_each_supported_field_to_its_payload_part() {
    let transformation = super::support::transformation(&[
        ("input", PayloadField::Messages),
        ("output", PayloadField::Response),
    ]);
    let record = prepare_log_data(&transformation, &super::support::valid_kwargs())
        .expect("the payload maps");
    assert_eq!(record.fields["input"], messages_json());
    assert_eq!(record.fields["output"], Json::Text("Hello".to_owned()));
}
