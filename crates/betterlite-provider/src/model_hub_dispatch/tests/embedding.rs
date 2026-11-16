use betterlite_models::decisions::Json;
use betterlite_openai::huggingface_embedding::error::HuggingfaceEmbeddingError;
use betterlite_openai::huggingface_embedding::task::PipelineTagSource as _;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

use super::super::HubDispatchError;
use super::super::embedding::{EmbeddingCall, NoTag};
use super::super::hub::PrefetchedTag;
use super::support::{
    Services, empty_globals, environment, http_reply, hub_reply, object, run_with,
};

const MODEL: &str = "org/model";

fn feature_extraction() -> Object {
    object(&[(
        "input_type",
        PromptJson::Text(String::from("feature-extraction")),
    )])
}

#[tokio::test]
async fn the_embedding_arm_posts_the_prepared_body_and_logs_both_hooks() {
    let reply = http_reply(200, "[[0.5,0.25],[0.1,0.2]]");
    let services = Services::new(Vec::new(), vec![reply], Vec::new());
    let input = vec![String::from("a"), String::from("b")];
    let mut options = feature_extraction();
    options.insert(String::from("wait_for_model"), PromptJson::Bool(true));
    let headers = Headers::from([(String::from("x-call"), String::from("1"))]);
    let call = EmbeddingCall {
        model: MODEL,
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let env = environment(&[("HUGGINGFACE_API_KEY", "hf-key")]);
    let (outcome, state) = run_with(&services, &empty_globals(), &env, &call).await;
    let response = outcome.unwrap();
    assert_eq!(response.data.len(), 2);
    assert_eq!(response.usage.prompt_tokens, 2);
    assert!(services.http.sent("GET").is_empty());
    let posted = services.http.sent("POST");
    let [sent] = posted.as_slice() else {
        panic!("the embedding arm posts once");
    };
    assert_eq!(sent.url, "http://mock/embed");
    assert_eq!(sent.headers.get("Authorization").unwrap(), "Bearer hf-key");
    assert_eq!(sent.headers.get("x-call").unwrap(), "1");
    let expected = r#"{"inputs":["a","b"],"options":{"wait_for_model":true}}"#;
    assert_eq!(sent.body, expected);
    let started = state.model_call_details.get("api_call_start_time");
    assert!(started.is_some());
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("post_api_call"))));
}

#[tokio::test]
async fn the_embedding_arm_reads_the_pipeline_tag_only_without_input_type() {
    let tag = hub_reply(r#"{"pipeline_tag":"feature-extraction"}"#);
    let reply = http_reply(200, "[[0.5]]");
    let services = Services::new(vec![tag], vec![reply], Vec::new());
    let input = vec![String::from("a")];
    let options = Object::new();
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: MODEL,
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: None,
        headers: &headers,
    };
    let (outcome, _logged) = run_with(&services, &empty_globals(), &environment(&[]), &call).await;
    assert_eq!(outcome.unwrap().data.len(), 1);
    let read = services.http.sent("GET");
    let [info] = read.as_slice() else {
        panic!("the embedding arm reads the model info once");
    };
    assert_eq!(info.url, "https://huggingface.co/api/models/org/model");
    assert!(info.headers.is_empty());
    assert!(info.query.is_empty());
    let posted = services.http.sent("POST");
    let [sent] = posted.as_slice() else {
        panic!("the embedding arm posts once");
    };
    let router = "https://router.huggingface.co/hf-inference/pipeline/feature-extraction/org/model";
    assert_eq!(sent.url, router);
}

#[tokio::test]
async fn the_embedding_arm_fails_with_500_when_the_reply_has_an_error_key() {
    let reply = http_reply(200, r#"{"error":"boom"}"#);
    let services = Services::new(Vec::new(), vec![reply], Vec::new());
    let input = vec![String::from("a")];
    let options = feature_extraction();
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: MODEL,
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let (outcome, state) = run_with(&services, &empty_globals(), &environment(&[]), &call).await;
    let failure = outcome.expect_err("the error key fails the call");
    let HubDispatchError::Embedding(HuggingfaceEmbeddingError::Huggingface(error)) = failure else {
        panic!("the error key is a Hugging Face error");
    };
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "boom");
    let Some(Json::Object(args)) = state.model_call_details.get("additional_args") else {
        panic!("the post_call keeps its arguments");
    };
    let Some(Json::Object(logged)) = args.get("complete_input_dict") else {
        panic!("the post_call keeps the request body");
    };
    assert!(logged.contains_key("inputs"));
}

#[tokio::test]
async fn the_embedding_arm_fails_with_the_reply_status_before_reading_the_body() {
    let reply = http_reply(429, "[[0.5]]");
    let services = Services::new(Vec::new(), vec![reply], Vec::new());
    let input = vec![String::from("a")];
    let options = feature_extraction();
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: MODEL,
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let (outcome, state) = run_with(&services, &empty_globals(), &environment(&[]), &call).await;
    let failure = outcome.expect_err("a 429 reply fails the call");
    let HubDispatchError::Embedding(HuggingfaceEmbeddingError::Huggingface(error)) = failure else {
        panic!("a failed status is a Hugging Face error");
    };
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "[[0.5]]");
    let event = state.model_call_details.get("log_event_type");
    assert_eq!(event, Some(&Json::Text(String::from("pre_api_call"))));
}

#[tokio::test]
async fn the_embedding_arm_accepts_every_2xx_status() {
    let reply = http_reply(299, "[[0.5]]");
    let services = Services::new(Vec::new(), vec![reply], Vec::new());
    let input = vec![String::from("a")];
    let options = feature_extraction();
    let headers = Headers::new();
    let call = EmbeddingCall {
        model: MODEL,
        input: &input,
        optional_params: &options,
        api_key: None,
        api_base: Some("http://mock/embed"),
        headers: &headers,
    };
    let (outcome, _logged) = run_with(&services, &empty_globals(), &environment(&[]), &call).await;
    assert_eq!(outcome.unwrap().data.len(), 1);
}

#[test]
fn the_placeholder_tag_source_never_answers() {
    assert_eq!(NoTag.tag("m", None).unwrap(), None);
}

#[test]
fn the_pipeline_tag_source_reads_pipeline_tag_or_fails_500() {
    let found = PrefetchedTag(hub_reply(r#"{"pipeline_tag":"rerank"}"#));
    assert_eq!(found.tag("m", None).unwrap(), Some(String::from("rerank")));
    let absent = PrefetchedTag(hub_reply(r#"{"id":"m"}"#));
    assert_eq!(absent.tag("m", None).unwrap(), None);
    let error = PrefetchedTag(hub_reply("not json"))
        .tag("m", None)
        .unwrap_err();
    assert_eq!(error.status_code, 500);
    assert_eq!(error.message, "the Hugging Face model info is not JSON");
}
