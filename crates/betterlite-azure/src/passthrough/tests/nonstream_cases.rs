use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::{RelayCallType, RelayedResponse};

use crate::passthrough::config::AzurePassthroughConfig;
use crate::passthrough::logging::{AzureRelayedLog, NoChatStreamAssembly};

use super::passthrough_cases::{CHAT_ENDPOINT, EMBEDDINGS_ENDPOINT, RESPONSES_ENDPOINT};
use super::support::{RealChatProjection, RecordingLogging, chat_chunks, headers, object, target};

const IMAGES_ENDPOINT: &str = "openai/deployments/dall-e-3/images/generations";
const CHAT_BODY: &str = r#"{"id": "chatcmpl-abc123", "object": "chat.completion", "created": 1700000000, "model": "gpt-4.1-mini-2025-04-14", "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hello! How can I assist you today?"}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18}}"#;
const EMBEDDINGS_BODY: &str = r#"{"object": "list", "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}], "model": "text-embedding-3-small", "usage": {"prompt_tokens": 1000, "total_tokens": 1000}}"#;
const RESPONSES_BODY: &str = r#"{"id": "resp_1", "object": "response", "created_at": 1, "status": "completed", "model": "gpt-4.1-mini", "output": [{"type": "message", "id": "msg_1", "role": "assistant", "status": "completed", "content": [{"type": "output_text", "text": "hi", "annotations": []}]}], "usage": {"input_tokens": 1000, "output_tokens": 100, "total_tokens": 1100}}"#;

pub(super) fn nonstream_usage_case() {
    chat_completion_logs_a_chat_reply();
    chat_log_keeps_the_relayed_request_body();
    relay_branch_builds_embedding_log();
    relay_branch_parses_images();
    relay_branch_parses_responses();
    relay_branch_rejects_other_shapes();
    failed_relay_status_logs_nothing();
    unknown_relay_endpoint_logs_nothing();
    undecodable_relay_logs_nothing();
    default_seams_log_nothing();
    trait_logging_keeps_the_relay_branch();
}

fn default_seams_log_nothing() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("gpt-4.1-mini", CHAT_ENDPOINT),
        &chat_response(),
        &headers(&[]),
        &chat_request_data(),
        &mut logging,
    );
    assert!(logged.is_none());
    assert_eq!(logging.call_type, None);
    let mut logging = RecordingLogging::default();
    let logged = config.handle_collected_chunks(
        &chat_chunks(),
        &mut logging,
        &target("gpt-4.1-mini", CHAT_ENDPOINT),
    );
    assert!(logged.is_none());
    assert_eq!(logging.call_type, None);
}

fn trait_logging_keeps_the_relay_branch() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let response = RelayedResponse {
        status_code: 200,
        body: EMBEDDINGS_BODY.as_bytes(),
    };
    let mut logging = RecordingLogging::default();
    let logged = BasePassthroughConfig::logging_non_streaming_response(
        &config,
        &target("text-embedding-3-small", EMBEDDINGS_ENDPOINT),
        &response,
        &object("{}"),
        &mut logging,
    );
    assert!(matches!(logged, Some(AzureRelayedLog::Relay(_))));
    assert_eq!(
        logging.call_type.map(RelayCallType::as_str),
        Some("aembedding")
    );
}

fn chat_response() -> RelayedResponse<'static> {
    RelayedResponse {
        status_code: 200,
        body: CHAT_BODY.as_bytes(),
    }
}

fn chat_request_data() -> Object {
    object(r#"{"model": "gpt-4.1-mini", "messages": [{"role": "user", "content": "hi"}]}"#)
}

fn chat_completion_logs_a_chat_reply() {
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: None,
        chat_reply: RealChatProjection::default(),
        chat_stream: NoChatStreamAssembly,
    };
    let reply_headers = headers(&[("content-type", "application/json")]);
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("gpt-4.1-mini", CHAT_ENDPOINT),
        &chat_response(),
        &reply_headers,
        &chat_request_data(),
        &mut logging,
    );
    let Some(AzureRelayedLog::Chat(reply)) = logged else {
        panic!("chat/completions logs the decoded reply");
    };
    assert_eq!(
        reply["choices"][0]["message"]["content"].as_str(),
        Some("Hello! How can I assist you today?")
    );
    assert_eq!(reply["usage"]["prompt_tokens"].as_u64(), Some(10));
    assert_eq!(reply["usage"]["completion_tokens"].as_u64(), Some(8));
    assert_eq!(reply["usage"]["total_tokens"].as_u64(), Some(18));
    assert_eq!(logging.call_type, None);
    assert_eq!(
        config.chat_reply.models.lock().unwrap().clone(),
        vec![String::from("gpt-4.1-mini")]
    );
}

fn chat_log_keeps_the_relayed_request_body() {
    let config = AzurePassthroughConfig {
        api_base_env: None,
        api_key_env: None,
        model_call_details: None,
        chat_reply: RealChatProjection::default(),
        chat_stream: NoChatStreamAssembly,
    };
    let request_data = chat_request_data();
    let reply_headers = headers(&[("content-type", "application/json")]);
    let mut logging = RecordingLogging::default();
    config.logging_non_streaming_response(
        &target("gpt-4.1-mini", CHAT_ENDPOINT),
        &chat_response(),
        &reply_headers,
        &request_data,
        &mut logging,
    );
    let inputs = config.chat_reply.inputs.lock().unwrap();
    assert_eq!(
        serde_json::to_value(&inputs[0].0).unwrap(),
        serde_json::json!([{"role": "user", "content": "no-message-pass-through-endpoint"}])
    );
    assert_eq!(
        serde_json::to_value(Json::from(&inputs[0].1)).unwrap(),
        serde_json::to_value(&request_data).unwrap()
    );
}

fn relay_branch_builds_embedding_log() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let response = RelayedResponse {
        status_code: 200,
        body: EMBEDDINGS_BODY.as_bytes(),
    };
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("text-embedding-3-small", EMBEDDINGS_ENDPOINT),
        &response,
        &headers(&[]),
        &object("{}"),
        &mut logging,
    );
    let Some(AzureRelayedLog::Relay(body)) = logged else {
        panic!("embeddings logs the relayed body");
    };
    assert_eq!(body["object"].as_str(), Some("list"));
    assert_eq!(body["data"][0]["index"].as_u64(), Some(0));
    assert_eq!(
        logging.call_type.map(RelayCallType::as_str),
        Some("aembedding")
    );
}

fn relay_branch_parses_images() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let images = br#"{"created": 1700000000, "data": [{"url": "https://x.test/img.png"}]}"#;
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("dall-e-3", IMAGES_ENDPOINT),
        &RelayedResponse {
            status_code: 200,
            body: images,
        },
        &headers(&[]),
        &object("{}"),
        &mut logging,
    );
    assert!(matches!(logged, Some(AzureRelayedLog::Relay(_))));
    assert_eq!(
        logging.call_type.map(RelayCallType::as_str),
        Some("aimage_generation")
    );
}

fn relay_branch_parses_responses() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("gpt-4.1-mini", RESPONSES_ENDPOINT),
        &RelayedResponse {
            status_code: 200,
            body: RESPONSES_BODY.as_bytes(),
        },
        &headers(&[]),
        &object("{}"),
        &mut logging,
    );
    let Some(AzureRelayedLog::Relay(body)) = logged else {
        panic!("responses logs the relayed body");
    };
    assert_eq!(body["id"].as_str(), Some("resp_1"));
    assert_eq!(
        logging.call_type.map(RelayCallType::as_str),
        Some("aresponses")
    );
}

fn relay_branch_rejects_other_shapes() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let table: [(&str, &str); 5] = [
        (EMBEDDINGS_ENDPOINT, r#"{"object": "not-list", "data": []}"#),
        (EMBEDDINGS_ENDPOINT, r#"{"object": "list", "data": {}}"#),
        (EMBEDDINGS_ENDPOINT, r#"{"data": []}"#),
        (IMAGES_ENDPOINT, r#"{"data": []}"#),
        (RESPONSES_ENDPOINT, r#"{"id": "resp_1", "output": []}"#),
    ];
    for (endpoint_path, body) in table {
        let mut logging = RecordingLogging::default();
        let logged = config.logging_non_streaming_response(
            &target("gpt-4.1-mini", endpoint_path),
            &RelayedResponse {
                status_code: 200,
                body: body.as_bytes(),
            },
            &headers(&[]),
            &object("{}"),
            &mut logging,
        );
        assert!(logged.is_none(), "{endpoint_path} {body}");
        assert_eq!(logging.call_type, None, "{endpoint_path} {body}");
    }
}

fn failed_relay_status_logs_nothing() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let rate_limited = br#"{"error": {"code": "429", "message": "rate limited"}}"#;
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("text-embedding-3-small", EMBEDDINGS_ENDPOINT),
        &RelayedResponse {
            status_code: 429,
            body: rate_limited,
        },
        &headers(&[]),
        &object("{}"),
        &mut logging,
    );
    assert!(logged.is_none());
    assert_eq!(logging.call_type, None);
}

fn unknown_relay_endpoint_logs_nothing() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let speech_endpoint = "openai/deployments/gpt-4o-mini-tts/audio/speech";
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("gpt-4o-mini-tts", speech_endpoint),
        &RelayedResponse {
            status_code: 200,
            body: br#"{"audio": "..."}"#,
        },
        &headers(&[]),
        &object("{}"),
        &mut logging,
    );
    assert!(logged.is_none());
    assert_eq!(logging.call_type, None);
}

fn undecodable_relay_logs_nothing() {
    let config: AzurePassthroughConfig = AzurePassthroughConfig::default();
    let mut logging = RecordingLogging::default();
    let logged = config.logging_non_streaming_response(
        &target("text-embedding-3-small", EMBEDDINGS_ENDPOINT),
        &RelayedResponse {
            status_code: 200,
            body: b"not json",
        },
        &headers(&[]),
        &object("{}"),
        &mut logging,
    );
    assert!(logged.is_none());
    assert_eq!(logging.call_type, None);
}
