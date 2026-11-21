use std::sync::{Arc, Mutex};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::harness::{HarnessOptions, TestCallback, harness, parse, text_of};

const SETUP_WITH_DISABLED_DETECTION: &str = "{\"setup\":{\"model\":\"models/m\",\"realtimeInputConfig\":{\"automaticActivityDetection\":{\"disabled\":true}}}}";

#[tokio::test]
async fn realtime_initial_setup_sequence() {
    injects_the_guardrail_disable_into_the_one_setup().await;
    skips_a_missing_or_empty_setup().await;
    sends_the_synthetic_session_created().await;
    skips_a_missing_synthetic_event().await;
    drops_an_empty_metadata_map().await;
    forces_the_model_for_a_transcription_intent().await;
    keeps_the_model_for_another_intent().await;
    closes_after_a_cancelled_relay().await;
}

async fn injects_the_guardrail_disable_into_the_one_setup() {
    let metadata = IndexMap::from([("user".to_owned(), Json::Text("u-1".to_owned()))]);
    let seen = Arc::new(Mutex::new(None));
    let callbacks = vec![TestCallback {
        seen: Arc::clone(&seen),
    }];
    let refs: Vec<&TestCallback> = callbacks.iter().collect();
    let mut options = HarnessOptions::default();
    options.requires_setup = true;
    options.setup = Some("{\"setup\":{\"model\":\"models/m\"}}".to_owned());
    options.metadata = Some(metadata.clone());
    let mut setup = harness(options, refs);
    setup.run().await.unwrap();
    let sent = setup.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        parse(text_of(&sent[0])),
        parse(SETUP_WITH_DISABLED_DETECTION)
    );
    let observation = setup.observation.lock().unwrap();
    assert_eq!(
        observation.session_configuration_request.as_deref(),
        Some(text_of(&sent[0]))
    );
    let request_data = observation.request_data.clone();
    assert_eq!(
        request_data.get("litellm_metadata"),
        Some(&Json::Object(metadata))
    );
    assert!(observation.turn_detection_update_sent);
    assert!(observation.source_present);
    assert!(observation.backend_setup_complete);
    drop(observation);
    assert_eq!(seen.lock().unwrap().as_ref(), Some(&request_data));
}

async fn skips_a_missing_or_empty_setup() {
    let mut options = HarnessOptions::default();
    options.requires_setup = true;
    let mut no_setup = harness(options, Vec::new());
    no_setup.run().await.unwrap();
    assert!(no_setup.sent().is_empty());

    let mut options = HarnessOptions::default();
    options.requires_setup = true;
    options.setup = Some(String::new());
    let mut empty_setup = harness(options, Vec::new());
    empty_setup.run().await.unwrap();
    assert!(empty_setup.sent().is_empty());
}

async fn sends_the_synthetic_session_created() {
    let synthetic = Json::Object(IndexMap::from([(
        "type".to_owned(),
        Json::Text("session.created".to_owned()),
    )]));
    let mut options = HarnessOptions::default();
    options.synthetic = Some(synthetic.clone());
    let mut deferred = harness(options, Vec::new());
    deferred.run().await.unwrap();
    assert_eq!(
        deferred.texts(),
        vec![serde_json::to_string(&synthetic).unwrap()]
    );
    let observation = deferred.observation.lock().unwrap();
    assert!(observation.session_created_sent);
    assert_eq!(observation.stored_messages, 1);
    assert!(!observation.backend_setup_complete);
}

async fn skips_a_missing_synthetic_event() {
    let mut no_synthetic = harness(HarnessOptions::default(), Vec::new());
    no_synthetic.run().await.unwrap();
    assert!(no_synthetic.texts().is_empty());
}

async fn drops_an_empty_metadata_map() {
    let mut options = HarnessOptions::default();
    options.metadata = Some(IndexMap::new());
    let mut empty_metadata = harness(options, Vec::new());
    empty_metadata.run().await.unwrap();
    assert!(
        !empty_metadata
            .observation
            .lock()
            .unwrap()
            .request_data
            .contains_key("litellm_metadata")
    );
}

async fn forces_the_model_for_a_transcription_intent() {
    let mut options = HarnessOptions::default();
    options.query_params = Some(IndexMap::from([(
        "intent".to_owned(),
        "transcription".to_owned(),
    )]));
    let mut transcription = harness(options, Vec::new());
    transcription.run().await.unwrap();
    assert_eq!(
        transcription
            .observation
            .lock()
            .unwrap()
            .force_transcription_model
            .as_deref(),
        Some("gemini-test")
    );
    assert!(!transcription.dials()[0].url.contains("intent"));
}

async fn keeps_the_model_for_another_intent() {
    let mut options = HarnessOptions::default();
    options.query_params = Some(IndexMap::from([(
        "intent".to_owned(),
        "translation".to_owned(),
    )]));
    let mut other_intent = harness(options, Vec::new());
    other_intent.run().await.unwrap();
    assert_eq!(
        other_intent
            .observation
            .lock()
            .unwrap()
            .force_transcription_model,
        None
    );
}

async fn closes_after_a_cancelled_relay() {
    let mut options = HarnessOptions::default();
    options.cancellation = true;
    options.relay_fails = true;
    let mut cancelled = harness(options, Vec::new());
    cancelled.run().await.unwrap();
    assert!(cancelled.closed());
    assert_eq!(cancelled.close().map(|close| close.0), Some(1011));
    assert!(
        cancelled
            .texts()
            .iter()
            .any(|text| text.contains("the relay failed"))
    );
}
