#![cfg(test)]

use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::Level;

use super::super::super::support;

const SHORT_AUDIO: &str = "https://eastus.stt.speech.microsoft.com/speech/recognition/conversation/cognitiveservices/v1?language=en-US";
const BATCH: &str = "https://eastus.api.cognitive.microsoft.com/speechtotext/v3.2/transcriptions";
const FAST: &str = "https://eastus.api.cognitive.microsoft.com/speechtotext/transcriptions:transcribe?api-version=2024-11-15";
const BOTH_ROUTES: &[u8] =
    br#"{"Offset": 5000000, "Duration": 25000000, "durationMilliseconds": 5061}"#;
const PRICE: &str = r#"{"input_cost_per_second": 0.5}"#;

#[rstest]
#[case::short_audio(SHORT_AUDIO, "azure_speech/short-audio", "1.5")]
#[case::fast_transcription(FAST, "azure_speech/fast-transcription", "2.5305")]
#[case::batch_transcription(BATCH, "azure_speech/batch-transcription", "0")]
fn the_handler_logs_the_model_provider_and_cost(
    #[case] url_route: &str,
    #[case] model: &str,
    #[case] cost: &str,
) {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::priced(PRICE);
    let outcome = support::run(&logging_obj, &lookup, url_route, None, BOTH_ROUTES);
    let entries = &outcome.kwargs.entries;
    let provider = Json::Text("azure_speech".to_owned());
    assert_eq!(entries.get("model"), Some(&Json::Text(model.to_owned())));
    assert_eq!(entries.get("custom_llm_provider"), Some(&provider));
    assert_eq!(support::cost_of(&outcome), support::dec(cost));
    assert_eq!(
        entries.get("user"),
        Some(&Json::Text("u-1".to_owned())),
        "the caller's kwargs survive"
    );
    let details = support::details(&logging_obj);
    assert_eq!(details.get("model"), Some(&Json::Text(model.to_owned())));
    assert_eq!(details.get("custom_llm_provider"), Some(&provider));
    assert_eq!(
        support::recorded_cost(&logging_obj),
        Some(support::dec(cost))
    );
    assert_eq!(support::logged_model(&logging_obj), model);
    assert_eq!(
        support::logged_provider(&logging_obj).as_deref(),
        Some("azure_speech")
    );
}

#[test]
fn the_handler_forwards_the_response_text_as_the_standard_response() {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::priced(PRICE);
    let outcome = support::run(&logging_obj, &lookup, SHORT_AUDIO, None, BOTH_ROUTES);
    assert_eq!(outcome.result, Some(support::standard_result()));
}

#[rstest]
#[case::tick_total(r#"{"Offset": 79228162514264337593543950335, "Duration": 1}"#)]
#[case::tick_value(r#"{"Offset": 10000000000000000000000000000000000000000, "Duration": 1}"#)]
fn a_body_beyond_the_decimal_range_forwards_the_response_without_costing(
    #[case] response_body: &str,
) {
    let logging_obj = support::logging_object();
    let lookup = support::RecordingLookup::priced(PRICE);
    let before = support::details(&logging_obj);
    let (logs, _guard) = support::capture_logs(Level::ERROR);
    let outcome = support::run(
        &logging_obj,
        &lookup,
        SHORT_AUDIO,
        None,
        response_body.as_bytes(),
    );
    assert_eq!(outcome.result, Some(support::standard_result()));
    assert_eq!(
        outcome.kwargs.entries.len(),
        1,
        "the fallback returns the caller's kwargs alone"
    );
    assert_eq!(
        outcome.kwargs.entries.get("user"),
        Some(&Json::Text("u-1".to_owned()))
    );
    assert_eq!(
        support::details(&logging_obj),
        before,
        "the fallback writes nothing onto the logging object"
    );
    assert_eq!(support::logged_model(&logging_obj), "pass-through");
    assert_eq!(support::logged_provider(&logging_obj), None);
    assert!(lookup.calls().is_empty());
    assert!(
        logs.text()
            .contains("error in the Azure Speech passthrough logging handler")
    );
}
