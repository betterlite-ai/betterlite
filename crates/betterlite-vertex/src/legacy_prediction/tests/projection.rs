use betterlite_models::call_helpers::finish_reason::{FinishReason, FinishReasonMapping};
use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, MessageContent, Role};
use futures_util::StreamExt;
use indexmap::IndexMap;

use crate::legacy_prediction::execution::LegacyContext;
use crate::legacy_prediction::projection::{
    LegacyMessage, LegacyUsage, LocalEncoding, ProjectionInput, ProjectionServices, project,
};
use crate::legacy_prediction::records::{
    Candidate, Prediction, PredictionsReply, ReplyMeta, TextReply, UsageMetadata,
    strip_output_separator,
};
use crate::legacy_prediction::streamer::TextStreamer;

use super::support::{
    FakeEncoding, FixtureClient, access_lookup, chat_invocation, counts, custom_invocation, json,
    legacy_context, predictions_reply, projection_services, usage_metadata, vision_invocation,
};

#[test]
fn projection_output_separator() {
    let prediction = Prediction::Value(Json::Text(
        "describe\nOutput:\nthe answer\nOutput:\nagain".to_owned(),
    ));
    let stripped = strip_output_separator(&prediction);
    let Prediction::Value(Json::Text(text)) = &stripped else {
        panic!("a stripped string");
    };
    assert_eq!(text, "the answer\nOutput:\nagain");

    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let meta = ReplyMeta {
        candidates: None,
        usage_metadata: Some(usage_metadata()),
    };
    let result = project(
        &ProjectionInput {
            prediction: Some(&stripped),
            meta: Some(&meta),
            model_is_language: false,
            prompt: "hello",
            model: "text-bison",
        },
        &services,
    )
    .expect("the projection");
    assert_eq!(
        result.message,
        LegacyMessage::Content("the answer\nOutput:\nagain".to_owned())
    );
}

#[tokio::test]
async fn projection_first_prediction() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_endpoint(predictions_reply(&["first", "second"]));
    let result = complete(
        &client,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the projection");
    assert_eq!(result.message, LegacyMessage::Content("first".to_owned()));
}

#[tokio::test]
async fn projection_none_string() {
    let encoding = FakeEncoding {
        counts: counts(&[("hello", 1), ("None", 1)]),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::default();
    let result = complete(&client, vision_invocation(), &context, access_lookup())
        .await
        .expect("the vision projection");
    assert_eq!(result.message, LegacyMessage::Content("None".to_owned()));
    assert_eq!(
        result.usage,
        LegacyUsage {
            prompt_tokens: 1,
            completion_tokens: 1,
            total_tokens: 2
        }
    );
    assert_eq!(result.created, 1_700_000_000);
}

#[tokio::test]
async fn projection_sdk_usage() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let reply = TextReply {
        text: String::from("ok"),
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: Some(UsageMetadata {
                prompt_token_count: Some(7),
                candidates_token_count: Some(3),
                total_token_count: None,
            }),
        },
    };
    let client = FixtureClient::with_text(reply);
    let result = complete(
        &client,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect("the projection");
    assert_eq!(
        result.usage,
        LegacyUsage {
            prompt_tokens: 7,
            completion_tokens: 3,
            total_tokens: 10
        }
    );
}

#[tokio::test]
async fn projection_local_usage() {
    let encoding = FakeEncoding {
        counts: counts(&[("hello", 2), ("ok", 1)]),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let reply = TextReply {
        text: String::from("ok"),
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: None,
        },
    };
    let client = FixtureClient::with_text(reply);
    let result = complete(
        &client,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect("the projection");
    assert_eq!(
        result.usage,
        LegacyUsage {
            prompt_tokens: 2,
            completion_tokens: 1,
            total_tokens: 3
        }
    );
}

fn finish_as_content_filter(_raw: &str) -> FinishReasonMapping {
    FinishReasonMapping {
        reason: FinishReason::ContentFilter,
        recognized: true,
    }
}

#[tokio::test]
async fn projection_language_finish_mapping() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let encoder: &dyn LocalEncoding = &encoding;
    let services = ProjectionServices {
        now: 1_700_000_000,
        encoding: encoder,
        finish_reason: finish_as_content_filter,
    };
    let context = LegacyContext {
        project: Some("proj"),
        location: Some("us-central1"),
        model_is_language: true,
        services: &services,
    };
    let reply = TextReply {
        text: String::from("ok"),
        meta: ReplyMeta {
            candidates: Some(vec![Candidate {
                content: String::from("ok"),
                finish_reason: Some(String::from("MAX_TOKENS")),
            }]),
            usage_metadata: Some(usage_metadata()),
        },
    };
    let client = FixtureClient::with_text(reply);
    let result = complete(
        &client,
        chat_invocation(Extra::new()),
        &context,
        access_lookup(),
    )
    .await
    .expect("the projection");
    assert_eq!(result.finish_reason, Some(FinishReason::ContentFilter));
    assert_eq!(
        result.usage,
        LegacyUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3
        }
    );
}

#[tokio::test]
async fn projection_copied_message() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let message = ChatMessage {
        role: Role::Assistant,
        content: Some(MessageContent::Text(String::from("copied"))),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    };
    let client = FixtureClient::with_endpoint(PredictionsReply {
        predictions: vec![Prediction::Message(message.clone())],
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: Some(usage_metadata()),
        },
    });
    let result = complete(
        &client,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the projection");
    assert_eq!(result.message, LegacyMessage::Copied(message));
}

#[tokio::test]
async fn projection_empty_string_keeps_default() {
    let encoding = FakeEncoding {
        counts: IndexMap::new(),
    };
    let services = projection_services(&encoding);
    let context = legacy_context(&services);
    let client = FixtureClient::with_endpoint(PredictionsReply {
        predictions: vec![Prediction::Value(Json::Text(String::new()))],
        meta: ReplyMeta {
            candidates: None,
            usage_metadata: Some(usage_metadata()),
        },
    });
    let result = complete(
        &client,
        custom_invocation(vec![json("{\"prompt\":\"hello\"}")]),
        &context,
        access_lookup(),
    )
    .await
    .expect("the projection");
    assert_eq!(result.message, LegacyMessage::Unchanged);
}

#[test]
fn projection_word_stream() {
    let words = Iterator::collect::<Vec<String>>(TextStreamer::new("hello world foo"));
    assert_eq!(words, vec!["hello", "world", "foo"]);
}

#[test]
fn projection_whitespace_only() {
    let words = Iterator::collect::<Vec<String>>(TextStreamer::new(" \n\t\r "));
    assert!(words.is_empty());
}

#[test]
fn projection_unicode_whitespace() {
    let words = Iterator::collect::<Vec<String>>(TextStreamer::new("a\u{a0}b\u{2003}c\u{3000}d"));
    assert_eq!(words, vec!["a", "b", "c", "d"]);
}

#[tokio::test]
async fn projection_sync_async_exhaustion() {
    let mut streamer = TextStreamer::new("a b");
    assert_eq!(Iterator::next(&mut streamer).as_deref(), Some("a"));
    assert_eq!(StreamExt::next(&mut streamer).await.as_deref(), Some("b"));
    assert_eq!(Iterator::next(&mut streamer), None);
    assert_eq!(StreamExt::next(&mut streamer).await, None);
    assert_eq!(StreamExt::next(&mut streamer).await, None);
}
