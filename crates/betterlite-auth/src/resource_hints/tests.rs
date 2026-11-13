#![cfg(test)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;
use tracing::Subscriber;
use tracing::subscriber::with_default;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::azure::router_model_in_endpoint;
use super::bedrock::{
    COUNT_TOKENS_MARKERS, ENDPOINT_ACTIONS, EndpointError, STREAMING_ACTIONS, extract_model,
};
use super::managed::{
    DecodedCharacterId, DecodedVideoId, PartsOutcome, decode_character_id, decode_parts,
    decode_video_id,
};
use super::nvidia::{
    DeploymentHint, MODEL_PREFIX, PROVIDER, model_group_in_path, model_groups,
    router_model_in_endpoint as nim_router_model_in_endpoint,
};
use super::{MAX_ENCODED_BYTES, ModelHint, ResourceKind, decode_resource_hint};

#[derive(Clone)]
struct EventCapture(Arc<AtomicUsize>);

impl<S: Subscriber> Layer<S> for EventCapture {
    fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
        if event.metadata().level() == &tracing::Level::DEBUG {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn debug_events<T, F>(run: F) -> (T, usize)
where
    F: FnOnce() -> T,
{
    let count = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(EventCapture(Arc::clone(&count)));
    let value = with_default(subscriber, run);
    let events = count.load(Ordering::Relaxed);
    (value, events)
}

fn video_payload(provider: &str, model: &str, id: &str) -> String {
    let payload = format!("litellm:custom_llm_provider:{provider};model_id:{model};video_id:{id}");
    STANDARD.encode(payload)
}

fn character_payload(provider: &str, model: &str, id: &str) -> String {
    let payload =
        format!("litellm:custom_llm_provider:{provider};model_id:{model};character_id:{id}");
    STANDARD.encode(payload)
}

fn native_video(id: &str) -> DecodedVideoId {
    DecodedVideoId {
        custom_llm_provider: None,
        model_id: None,
        video_id: id.to_owned(),
    }
}

fn native_character(id: &str) -> DecodedCharacterId {
    DecodedCharacterId {
        custom_llm_provider: None,
        model_id: None,
        character_id: id.to_owned(),
    }
}

#[test]
fn azure_first_exact_segment() {
    let models = vec!["chat".to_owned(), "gpt-4".to_owned()];
    assert_eq!(
        router_model_in_endpoint("/chat/gpt-4", &models).as_deref(),
        Some("chat")
    );
    assert_eq!(
        router_model_in_endpoint("//x", &[String::new(), "x".to_owned()]).as_deref(),
        Some("")
    );
    assert_eq!(
        router_model_in_endpoint("/GPT-4", &["gpt-4".to_owned()]),
        None
    );
}

#[test]
fn azure_requires_separator() {
    let models = vec!["gpt-4".to_owned()];
    assert_eq!(router_model_in_endpoint("gpt-4", &models), None);
    assert_eq!(
        router_model_in_endpoint("/gpt-4", &models).as_deref(),
        Some("gpt-4")
    );
}

#[test]
fn nim_provider_or_model_prefix() {
    let classified = |custom_llm_provider: Option<&str>, model: Option<&str>| {
        let deployments = [DeploymentHint {
            model_name: "group",
            custom_llm_provider,
            model,
        }];
        !model_groups(&deployments).is_empty()
    };
    assert!(classified(Some(PROVIDER), None));
    assert!(classified(None, Some("nvidia_nim/meta")));
    assert!(!classified(Some("openai"), Some("openai/meta")));
    assert!(!classified(None, Some("NVIDIA_NIM/meta")));
    assert_eq!(MODEL_PREFIX, "nvidia_nim/");
}

#[test]
fn nim_mixed_group_excluded() {
    let deployments = [
        DeploymentHint {
            model_name: "shared",
            custom_llm_provider: Some("nvidia_nim"),
            model: None,
        },
        DeploymentHint {
            model_name: "shared",
            custom_llm_provider: Some("openai"),
            model: None,
        },
        DeploymentHint {
            model_name: "nim-only",
            custom_llm_provider: None,
            model: Some("nvidia_nim/meta"),
        },
    ];
    let groups: Vec<String> = model_groups(&deployments).into_iter().collect();
    assert_eq!(groups, vec!["nim-only".to_owned()]);
}

#[test]
fn nim_longest_prefix_and_empty_segments() {
    let deployments = [
        DeploymentHint {
            model_name: "meta",
            custom_llm_provider: Some("nvidia_nim"),
            model: None,
        },
        DeploymentHint {
            model_name: "meta/llama-3",
            custom_llm_provider: Some("nvidia_nim"),
            model: None,
        },
    ];
    assert_eq!(
        model_group_in_path("/meta/llama-3/v1/chat/completions", &deployments).as_deref(),
        Some("meta/llama-3")
    );
    assert_eq!(
        model_group_in_path("/meta//llama-3/v1", &deployments).as_deref(),
        Some("meta/llama-3")
    );
    let models: BTreeSet<String> = ["a".to_owned(), "a/b".to_owned()].into_iter().collect();
    assert_eq!(
        nim_router_model_in_endpoint("/a/b/c", &models).as_deref(),
        Some("a/b")
    );
}

#[test]
fn nim_casefold_prefix_only() {
    let deployments = [DeploymentHint {
        model_name: "meta/llama-3",
        custom_llm_provider: Some("nvidia_nim"),
        model: None,
    }];
    assert_eq!(
        model_group_in_path("/NVIDIA_NIM/meta/llama-3/v1", &deployments).as_deref(),
        Some("meta/llama-3")
    );
    assert_eq!(
        model_group_in_path("nvidia_nim/meta/llama-3", &deployments),
        None
    );
    assert_eq!(
        model_group_in_path("/NVIDIA_NIMX/meta/llama-3", &deployments),
        None
    );
}

#[test]
fn bedrock_count_substrings() {
    let counts_tokens = |endpoint: &str| {
        COUNT_TOKENS_MARKERS
            .iter()
            .any(|marker| endpoint.contains(marker))
    };
    assert!(counts_tokens("/model/x/count_tokens"));
    assert!(counts_tokens("/model/x/count-tokens"));
    assert!(counts_tokens("count_tokens"));
    assert!(!counts_tokens("/COUNT_TOKENS"));
    assert!(!counts_tokens("/x/counttokens"));
}

#[rstest]
#[case::invoke("invoke")]
#[case::invoke_stream("invoke-with-response-stream")]
#[case::converse("converse")]
#[case::converse_stream("converse-stream")]
#[case::count_underscore("count_tokens")]
#[case::count_hyphen("count-tokens")]
fn bedrock_each_action(#[case] action: &str) {
    assert!(ENDPOINT_ACTIONS.contains(&action));
    assert_eq!(
        STREAMING_ACTIONS,
        ["invoke-with-response-stream", "converse-stream"]
    );
    let endpoint = format!("/model/m/{action}");
    assert_eq!(extract_model(&endpoint).unwrap(), "m");
}

#[test]
fn profile_fixed_slice() {
    assert_eq!(
        extract_model("/model/application-inference-profile/p1/invoke").unwrap(),
        "model/application-inference-profile"
    );
    assert_eq!(
        extract_model("/application-inference-profile/p1/invoke").unwrap(),
        "application-inference-profile/p1"
    );
    assert_eq!(extract_model("application-inference-profile").unwrap(), "");
}

#[test]
fn first_model_and_action() {
    assert_eq!(extract_model("v2/model/m/invoke").unwrap(), "m");
    assert_eq!(
        extract_model("/model/aws/anthropic/m/invoke").unwrap(),
        "aws/anthropic/m"
    );
    assert_eq!(extract_model("/model/m/x/invoke").unwrap(), "m/x");
    assert_eq!(extract_model("/model/m/invoke/extra").unwrap(), "m");
    assert_eq!(
        extract_model("/invoke/model/m/converse").unwrap(),
        "m/converse"
    );
}

#[test]
fn bedrock_no_marker_fallback() {
    assert_eq!(extract_model("/x/y/invoke").unwrap(), "x/y");
    assert_eq!(extract_model("/x/invoke").unwrap(), "x");
    assert_eq!(extract_model("/x//y/converse").unwrap(), "x/y");
    let error = extract_model("").unwrap_err();
    assert_eq!(
        error.to_string(),
        "'model' keyword not found and unable to extract model from endpoint. \
         Expected format: /model/{modelId}/{action}. Got: "
    );
}

#[rstest]
#[case::trailing_slash("/model/")]
#[case::bare_marker("/model")]
fn bedrock_empty_suffix_errors(#[case] endpoint: &str) {
    let error = extract_model(endpoint).unwrap_err();
    assert_eq!(
        error,
        EndpointError::NoModelId {
            endpoint: endpoint.to_owned()
        }
    );
}

#[test]
fn bedrock_keeps_value_error() {
    let error = extract_model("/model").unwrap_err();
    assert_eq!(
        error.to_string(),
        "No model ID found after 'model' keyword. Expected format: \
         /model/{modelId}/{action}. Got: /model"
    );
    assert!(!error.to_string().contains("Model missing from endpoint"));
}

#[test]
fn video_character_empty_and_native_id() {
    assert_eq!(decode_video_id(""), native_video(""));
    assert_eq!(decode_video_id("vid-1"), native_video("vid-1"));
    assert_eq!(
        decode_video_id("character_QUJD"),
        native_video("character_QUJD")
    );
    assert_eq!(decode_character_id(""), native_character(""));
    assert_eq!(decode_character_id("char-1"), native_character("char-1"));
    assert_eq!(decode_video_id("video_QUJD"), native_video("video_QUJD"));
}

#[test]
fn prefix_removed_everywhere() {
    let video = format!("video_video_{}", video_payload("az", "m", "v1"));
    let video_decoded = decode_video_id(&video);
    assert_eq!(video_decoded.custom_llm_provider.as_deref(), Some("az"));
    assert_eq!(video_decoded.video_id, "v1");
    let character = format!("character_character_{}", character_payload("az", "m", "c1"));
    let character_decoded = decode_character_id(&character);
    assert_eq!(character_decoded.custom_llm_provider.as_deref(), Some("az"));
    assert_eq!(character_decoded.character_id, "c1");
}

#[test]
fn missing_padding_permissive_base64() {
    let padded = video_payload("az", "gpt-4", "vid-12");
    let unpadded = padded.trim_end_matches('=');
    assert_ne!(unpadded, padded);
    let without_padding = decode_video_id(&format!("video_{unpadded}"));
    assert_eq!(without_padding.model_id.as_deref(), Some("gpt-4"));
    let with_junk = decode_video_id(&format!("video_@@{padded}  "));
    assert_eq!(with_junk.model_id.as_deref(), Some("gpt-4"));
    let extra_padding = decode_video_id(&format!("video_{padded}===="));
    assert_eq!(extra_padding.model_id.as_deref(), Some("gpt-4"));
    let character = character_payload("az", "gpt-4", "c1");
    let character_unpadded = character.trim_end_matches('=');
    assert_ne!(character_unpadded, character);
    let decoded_character = decode_character_id(&format!("character_{character_unpadded}"));
    assert_eq!(decoded_character.model_id.as_deref(), Some("gpt-4"));
    assert_eq!(decoded_character.character_id, "c1");
}

#[test]
fn invalid_base64_and_utf8_debug_fallback() {
    let (invalid_base64, base64_events) = debug_events(|| decode_video_id("video_Q"));
    assert_eq!(invalid_base64, native_video("video_Q"));
    assert_eq!(base64_events, 1);
    let invalid_utf8 = format!("video_{}", STANDARD.encode([0xff_u8]));
    let (utf8_decoded, utf8_events) = debug_events(|| decode_video_id(&invalid_utf8));
    assert_eq!(utf8_decoded, native_video(&invalid_utf8));
    assert_eq!(utf8_events, 1);
    let character = format!("character_{}", STANDARD.encode([0xff_u8]));
    let (character_decoded, character_events) = debug_events(|| decode_character_id(&character));
    assert_eq!(character_decoded, native_character(&character));
    assert_eq!(character_events, 1);
}

#[test]
fn missing_semicolon_and_short_parts() {
    let no_semicolon = format!("video_{}", STANDARD.encode("no-semicolon"));
    assert_eq!(decode_video_id(&no_semicolon), native_video(&no_semicolon));
    let short = format!("video_{}", STANDARD.encode("a;b"));
    assert_eq!(decode_video_id(&short), native_video(&short));
    let character = format!("character_{}", STANDARD.encode("a;b"));
    assert_eq!(
        decode_character_id(&character),
        native_character(&character)
    );
}

#[test]
fn first_three_parts_and_global_label_replace() {
    let five = format!("video_{}", STANDARD.encode("a;b;c;d;e"));
    assert_eq!(
        decode_video_id(&five),
        DecodedVideoId {
            custom_llm_provider: Some("a".to_owned()),
            model_id: Some("b".to_owned()),
            video_id: "c".to_owned(),
        }
    );
    let labels = "litellm:custom_llm_provider:litellm:custom_llm_provider:az;\
                  model_id:model_id:m;video_id:video_id:v";
    let labelled_video = decode_video_id(&format!("video_{}", STANDARD.encode(labels)));
    assert_eq!(
        labelled_video,
        DecodedVideoId {
            custom_llm_provider: Some("az".to_owned()),
            model_id: Some("m".to_owned()),
            video_id: "v".to_owned(),
        }
    );
    let character = "litellm:custom_llm_provider:az;model_id:m;\
                     character_id:character_id:c";
    let labelled_character =
        decode_character_id(&format!("character_{}", STANDARD.encode(character)));
    assert_eq!(
        labelled_character,
        DecodedCharacterId {
            custom_llm_provider: Some("az".to_owned()),
            model_id: Some("m".to_owned()),
            character_id: "c".to_owned(),
        }
    );
}

#[test]
fn empty_fields_preserved() {
    let payload = "litellm:custom_llm_provider:;model_id:;video_id:";
    assert_eq!(
        decode_video_id(&format!("video_{}", STANDARD.encode(payload))),
        DecodedVideoId {
            custom_llm_provider: Some(String::new()),
            model_id: Some(String::new()),
            video_id: String::new(),
        }
    );
    let character = "litellm:custom_llm_provider:;model_id:;character_id:";
    assert_eq!(
        decode_character_id(&format!("character_{}", STANDARD.encode(character))),
        DecodedCharacterId {
            custom_llm_provider: Some(String::new()),
            model_id: Some(String::new()),
            character_id: String::new(),
        }
    );
}

#[test]
fn native_resource_hint_limits() {
    let oversized = format!("video_{}", "A".repeat(MAX_ENCODED_BYTES));
    let (video_hint, video_events) =
        debug_events(|| decode_resource_hint(ResourceKind::Video, oversized.as_bytes()));
    assert_eq!(video_hint, None);
    assert_eq!(video_events, 1);
    let (character_hint, character_events) =
        debug_events(|| decode_resource_hint(ResourceKind::Character, oversized.as_bytes()));
    assert_eq!(character_hint, None);
    assert_eq!(character_events, 1);
    let video = format!("video_{}", video_payload("az", "m", "v"));
    assert_eq!(
        decode_resource_hint(ResourceKind::Video, video.as_bytes()),
        Some(ModelHint {
            custom_llm_provider: "az".to_owned(),
            model_id: "m".to_owned(),
        })
    );
    let character = format!("character_{}", character_payload("az", "m", "c"));
    assert_eq!(
        decode_resource_hint(ResourceKind::Character, character.as_bytes()),
        Some(ModelHint {
            custom_llm_provider: "az".to_owned(),
            model_id: "m".to_owned(),
        })
    );
    let limited = decode_parts(
        &format!("video_{}", STANDARD.encode([b'a'; 12])),
        super::managed::VIDEO_ID_PREFIX,
        "video_id:",
        Some(8),
    );
    assert!(matches!(limited, PartsOutcome::Undecodable));
}

#[rstest]
fn replays_the_resource_hint_corpus(#[files("fuzz/corpus/resource_hint/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    drop(decode_resource_hint(ResourceKind::Video, &bytes));
    drop(decode_resource_hint(ResourceKind::Character, &bytes));
}
