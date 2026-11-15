use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support;
use crate::callback_logger::CallbackLogger as _;

// one success event of the given call type, logged and flushed, so the span is in the recorder
async fn record_call(call_type: &str, response: &Json) -> support::Fixture {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    fixture
        .manager
        .async_log_success_event(support::event(support::payload(
            call_type,
            Some(support::CALL_ID),
            response,
            None,
        )))
        .await
        .expect("the event logs");
    assert!(fixture.manager.logger.flush().await);
    fixture
}

fn body(source: &str) -> Json {
    serde_json::from_str(source).expect("the response parses")
}

fn input_present(fixture: &support::Fixture) -> bool {
    support::langfuse::attribute_text(&fixture.spans.spans(), "langfuse.observation.input")
        .is_some()
}

fn output_text(fixture: &support::Fixture) -> Option<String> {
    support::langfuse::attribute_text(&fixture.spans.spans(), "langfuse.observation.output")
}

fn output_json(fixture: &support::Fixture) -> Json {
    support::langfuse::attribute_json(&fixture.spans.spans(), "langfuse.observation.output")
}

#[tokio::test]
async fn an_embedding_call_logs_the_prompt_without_an_output() {
    let fixture = record_call("embedding", &support::response()).await;
    assert!(input_present(&fixture));
    assert_eq!(output_text(&fixture), None);
}

#[tokio::test]
async fn a_text_completion_call_projects_the_choice_text() {
    let response = body(r#"{"choices":[{"index":0,"text":"hi","finish_reason":"stop"}]}"#);
    let fixture = record_call("text_completion", &response).await;
    assert!(input_present(&fixture));
    assert_eq!(output_text(&fixture).as_deref(), Some("hi"));
}

#[tokio::test]
async fn a_speech_call_projects_the_speech_marker() {
    let fixture = record_call("speech", &support::response()).await;
    assert!(input_present(&fixture));
    assert_eq!(output_text(&fixture).as_deref(), Some("speech-output"));
}

#[tokio::test]
async fn an_image_call_projects_the_image_data() {
    let data = Json::List(vec![Json::Object(IndexMap::from([(
        "url".to_owned(),
        Json::Text("https://example.test/image.png".to_owned()),
    )]))]);
    let response = Json::Object(IndexMap::from([("data".to_owned(), data.clone())]));
    let fixture = record_call("image_generation", &response).await;
    assert!(input_present(&fixture));
    assert_eq!(output_json(&fixture), data);
}

#[tokio::test]
async fn a_transcription_call_projects_the_transcript() {
    let fixture = record_call("transcription", &body(r#"{"text":"hello"}"#)).await;
    assert!(input_present(&fixture));
    assert_eq!(output_text(&fixture).as_deref(), Some("hello"));
}

#[tokio::test]
async fn a_rerank_call_projects_the_results() {
    let results = Json::List(vec![Json::Object(IndexMap::from([(
        "index".to_owned(),
        serde_json::from_str("0").expect("the index parses"),
    )]))]);
    let response = Json::Object(IndexMap::from([("results".to_owned(), results.clone())]));
    let fixture = record_call("rerank", &response).await;
    assert!(input_present(&fixture));
    assert_eq!(output_json(&fixture), results);
}

#[tokio::test]
async fn a_responses_call_projects_the_output_items() {
    let items = Json::List(vec![Json::Object(IndexMap::from([(
        "type".to_owned(),
        Json::Text("message".to_owned()),
    )]))]);
    let response = Json::Object(IndexMap::from([("output".to_owned(), items.clone())]));
    let fixture = record_call("responses", &response).await;
    assert!(input_present(&fixture));
    assert_eq!(output_json(&fixture), items);
}

#[tokio::test]
async fn a_realtime_call_projects_the_response_list() {
    let delta = Json::List(vec![Json::Text("delta".to_owned())]);
    let fixture = record_call("_arealtime", &delta).await;
    assert_eq!(output_json(&fixture), delta);
}

#[tokio::test]
async fn a_pass_through_call_projects_the_response_field() {
    let fixture = record_call("pass_through_endpoint", &support::response()).await;
    assert!(input_present(&fixture));
    assert_eq!(output_text(&fixture).as_deref(), Some(""));
}
