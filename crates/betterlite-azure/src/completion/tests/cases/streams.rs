use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::{JsonValue, StreamOptions};

use crate::azure_urls::AuthParams;
use crate::completion::error::BUFFERED_ANSWER;
use crate::completion::stream::requested_stream_options;
use crate::completion::tests::fixture::{Answer, object};
use crate::endpoints::projection::{ClientKind, LiteLlmParams};

use super::support::{
    MODEL, Run, Scenario, async_buffered, async_stream, buffered, count, field, sync_buffered,
    sync_stream, text, usage_items,
};

// CustomStreamWrapper reads stream_options from the request data and falls back to the caller's default
fn stream_options_from_data() {
    let off = Some(StreamOptions {
        include_usage: false,
    });
    let on = Some(StreamOptions {
        include_usage: true,
    });
    let asked = object(r#"{"stream_options":{"include_usage":true}}"#);
    assert_eq!(requested_stream_options(&asked, None), on);
    let denied = object(r#"{"stream_options":{"include_usage":false}}"#);
    assert_eq!(requested_stream_options(&denied, on.clone()), off);
    let empty = object(r#"{"stream_options":{}}"#);
    assert_eq!(requested_stream_options(&empty, on.clone()), off);
    let scalar = object(r#"{"stream_options":"yes"}"#);
    assert_eq!(requested_stream_options(&scalar, on.clone()), on);
    assert_eq!(requested_stream_options(&object("{}"), None), None);
}

pub(super) fn stream_text_choices() {
    let run = sync_stream(r#"{"stream":true}"#).execute();
    assert_eq!(run.contents(), ["Hi ", "there"]);
    let first = &run.chunks[0];
    assert_eq!(first.model, MODEL);
    let provider = first.hidden_params.get("custom_llm_provider");
    assert_eq!(provider, Some(&JsonValue::String("azure_text".to_owned())));
    let last = run.chunks.last().unwrap();
    assert_eq!(last.choices[0].finish_reason.as_deref(), Some("stop"));
    assert_eq!(text(&run.pre(&["input"])), "Hello");
    assert_eq!(
        run.pre(&["additional_args", "acompletion"]),
        Json::Bool(true)
    );
    let key = run.pre(&["additional_args", "headers", "Authorization"]);
    assert_eq!(text(&key), "Bearer sk-unit");
    let sent = ["additional_args", "complete_input_dict", "stream"];
    assert_eq!(run.pre(&sent), Json::Bool(true));
    assert!(run.hooks.post.is_empty());
    stream_options_from_data();
    stream_usage_chunk();
}

fn stream_usage_chunk() {
    let counted = || Scenario::new(ClientKind::AzureOpenAi, Answer::Stream(usage_items()), "{}");
    let mut plain = counted();
    plain.params = object(r#"{"stream":true}"#);
    let mut asked = counted();
    asked.params = object(r#"{"stream":true,"stream_options":{"include_usage":true}}"#);
    let base = plain.execute();
    let with_usage = asked.execute();
    assert_eq!(with_usage.chunks.len(), base.chunks.len().saturating_add(1));
    let usage = with_usage.chunks.last().unwrap().usage.as_ref().unwrap();
    assert_eq!(usage.prompt_tokens, 3);
    assert_eq!(usage.completion_tokens, 2);
    assert_eq!(usage.total_tokens, 5);
}

fn seven_retries() -> LiteLlmParams {
    LiteLlmParams {
        azure_ad_token: None,
        auth: AuthParams::default(),
        max_retries: Some(7),
        timeout: None,
    }
}

fn client_field(run: &Run, name: &str) -> Json {
    field(&run.source.gets()[0], name)
}

fn litellm_params_follow_the_branch() {
    let mut sync_streaming = sync_stream(r#"{"stream":true}"#);
    sync_streaming.parts.litellm_params = seven_retries();
    let run = sync_streaming.execute();
    assert_eq!(client_field(&run, "max_retries"), Json::Null);
    assert_eq!(client_field(&run, "is_async"), Json::Bool(false));
    assert_eq!(run.create("max_retries"), Json::Null);
    let scenarios = [
        (sync_buffered("{}"), false),
        (async_buffered("{}"), true),
        (async_stream(r#"{"stream":true}"#), true),
    ];
    for (mut scenario, asynchronous) in scenarios {
        scenario.parts.litellm_params = seven_retries();
        let asked = scenario.execute();
        assert_eq!(client_field(&asked, "max_retries"), count(7));
        assert_eq!(client_field(&asked, "is_async"), Json::Bool(asynchronous));
    }
}

fn only_a_literal_true_streams() {
    for params in [
        r#"{"stream":"true"}"#,
        r#"{"stream":1}"#,
        r#"{"stream":false}"#,
    ] {
        let sync_run = sync_buffered(params).execute();
        assert!(sync_run.reply.is_some());
        assert_eq!(sync_run.hooks.post.len(), 1);
        assert!(async_buffered(params).execute().reply.is_some());
    }
    let streamed = async_stream(r#"{"stream":true}"#).execute();
    assert_eq!(streamed.contents(), ["Hi ", "there"]);
    assert!(streamed.hooks.post.is_empty());
    let sent = ["additional_args", "complete_input_dict", "stream"];
    assert_eq!(streamed.pre(&sent), Json::Bool(true));
}

fn buffered_answer_to_a_stream_request() {
    for kind in [ClientKind::AzureOpenAi, ClientKind::AsyncAzureOpenAi] {
        let run = Scenario::new(kind, buffered(), r#"{"stream":true}"#).execute();
        assert_eq!(run.failure().status, Some(500));
        assert_eq!(run.failure().message, BUFFERED_ANSWER);
        assert_eq!(run.hooks.pre.len(), 1);
    }
}

pub(super) fn azure_text_stream_dispatch() {
    litellm_params_follow_the_branch();
    only_a_literal_true_streams();
    buffered_answer_to_a_stream_request();
    let run = sync_stream(r#"{"stream":true}"#).execute();
    let base = run.pre(&["additional_args", "api_base"]);
    assert_eq!(text(&base), "https://unit.test/openai/");
}
