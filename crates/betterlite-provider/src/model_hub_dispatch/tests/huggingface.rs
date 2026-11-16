use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_openai::huggingface_chat::error::HuggingfaceChatError;
use betterlite_openai::huggingface_chat::mapping::ProviderMappingSource as _;
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

use super::super::HubCompletion;
use super::super::huggingface::NoMapping;
use super::support::{
    Outcome, Services, chat_context, empty_globals, environment, hub_reply, object, run_with,
    user_message,
};
use crate::prepared::{PreparedChat, PreparedRequest};

const HF: ChatConfigTag = ChatConfigTag::HuggingFaceChat;
const MODEL: &str = "meta-llama/Llama-3-8B-Instruct";
const ROUTED: &str = "hf-inference/meta-llama/Llama-3-8B-Instruct";
const BASE: Option<&str> = Some("http://mock/v1");
const MAPPING: &str = r#"{"inferenceProviderMapping":{"hf-inference":{"status":"live","providerId":"meta-llama/Llama-3-8B-Instruct"}}}"#;

// the Hugging Face arm only prepares the request and logs nothing
fn prepared(ran: Outcome<Option<HubCompletion>>) -> PreparedRequest {
    let (outcome, state) = ran;
    assert!(state.model_call_details.get("log_event_type").is_none());
    let Some(HubCompletion::HuggingFace(request)) = outcome.unwrap() else {
        panic!("the hugging face tag prepares a request");
    };
    *request
}

fn chat(request: PreparedRequest) -> Box<PreparedChat> {
    let PreparedRequest::Chat(plain) = request else {
        panic!("a plain call prepares a chat request");
    };
    plain
}

#[tokio::test]
async fn the_huggingface_arm_prepares_a_stream_request() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let env = environment(&[("HF_TOKEN", "hf-token")]);
    let mut messages = vec![user_message()];
    let mut options = object(&[("stream", PromptJson::Bool(true))]);
    let headers = Headers::from([(String::from("x-call"), String::from("1"))]);
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    call.headers = Some(&headers);
    let request = prepared(run_with(&services, &empty_globals(), &env, (HF, &mut call)).await);
    let PreparedRequest::Stream(stream) = request else {
        panic!("a stream call prepares a stream request");
    };
    assert_eq!(stream.http.url.as_str(), "http://mock/v1/chat/completions");
    let sent = &stream.http.headers;
    assert_eq!(sent.get("Authorization").unwrap(), "Bearer hf-token");
    assert_eq!(sent.get("x-call").unwrap(), "1");
    let body: Object = serde_json::from_slice(&stream.http.body).unwrap();
    assert_eq!(
        body.get("model"),
        Some(&PromptJson::Text(String::from(MODEL)))
    );
    assert_eq!(body.get("stream"), Some(&PromptJson::Bool(true)));
}

#[tokio::test]
async fn the_huggingface_arm_prepares_the_chat_request() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let env = environment(&[("HUGGINGFACE_API_KEY", "hf-api")]);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    let plain = chat(prepared(
        run_with(&services, &empty_globals(), &env, (HF, &mut call)).await,
    ));
    assert_eq!(plain.http.url.as_str(), "http://mock/v1/chat/completions");
    let sent = &plain.http.headers;
    assert_eq!(sent.get("Authorization").unwrap(), "Bearer hf-api");
    assert_eq!(sent.get("content-type").unwrap(), "application/json");
    assert_eq!(plain.client_model.as_deref(), Some(MODEL));
    assert!(services.http.sent("GET").is_empty());
}

#[tokio::test]
async fn the_huggingface_arm_falls_back_to_the_global_headers() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let shared = Headers::from([(String::from("x-global"), String::from("g"))]);
    let mut globals = empty_globals();
    globals.headers = Some(&shared);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    let plain = chat(prepared(
        run_with(&services, &globals, &environment(&[]), (HF, &mut call)).await,
    ));
    assert_eq!(plain.http.headers["x-global"], "g");
}

#[tokio::test]
async fn the_huggingface_arm_prefers_the_call_headers_over_the_global_ones() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let shared = Headers::from([(String::from("x-global"), String::from("g"))]);
    let mut globals = empty_globals();
    globals.headers = Some(&shared);
    let own = Headers::from([(String::from("x-call"), String::from("1"))]);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(MODEL, &mut messages, &mut options, BASE);
    call.headers = Some(&own);
    let plain = chat(prepared(
        run_with(&services, &globals, &environment(&[]), (HF, &mut call)).await,
    ));
    assert_eq!(plain.http.headers["x-call"], "1");
    assert!(plain.http.headers.get("x-global").is_none());
}

#[tokio::test]
async fn the_huggingface_arm_fetches_the_provider_mapping_without_a_base() {
    let services = Services::new(vec![hub_reply(MAPPING)], Vec::new(), Vec::new());
    let env = environment(&[("HUGGINGFACE_API_KEY", "hub-secret")]);
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(ROUTED, &mut messages, &mut options, None);
    let routed = chat(prepared(
        run_with(&services, &empty_globals(), &env, (HF, &mut call)).await,
    ));
    let read = services.http.sent("GET");
    let [info] = read.as_slice() else {
        panic!("the arm reads the provider mapping once");
    };
    let url = "https://huggingface.co/api/models/meta-llama/Llama-3-8B-Instruct";
    assert_eq!(info.url, url);
    assert_eq!(info.headers["Authorization"], "Bearer hub-secret");
    assert_eq!(info.headers["Accept"], "application/json");
    let expand = vec![(
        String::from("expand"),
        String::from("inferenceProviderMapping"),
    )];
    assert_eq!(info.query, expand);
    let body: Object = serde_json::from_slice(&routed.http.body).unwrap();
    assert_eq!(
        body.get("model"),
        Some(&PromptJson::Text(String::from(MODEL)))
    );
}

#[tokio::test]
async fn the_huggingface_arm_skips_the_mapping_when_a_base_is_set() {
    let services = Services::new(Vec::new(), Vec::new(), Vec::new());
    let mut messages = vec![user_message()];
    let mut options = Object::new();
    let mut call = chat_context(ROUTED, &mut messages, &mut options, BASE);
    let plain = chat(prepared(
        run_with(
            &services,
            &empty_globals(),
            &environment(&[]),
            (HF, &mut call),
        )
        .await,
    ));
    let body: Object = serde_json::from_slice(&plain.http.body).unwrap();
    assert_eq!(
        body.get("model"),
        Some(&PromptJson::Text(String::from(ROUTED)))
    );
    assert!(services.http.sent("GET").is_empty());
}

#[test]
fn the_placeholder_mapping_source_never_answers() {
    let mapping = NoMapping.mapping("m");
    assert!(matches!(mapping, Err(HuggingfaceChatError::MappingMissing)));
}
