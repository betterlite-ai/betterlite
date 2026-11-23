use betterlite_azure::passthrough::error::AzurePassthroughError;
use betterlite_azure::passthrough::logging::{
    AzureRelayedLog, NoChatReplyProjection, NoChatStreamAssembly,
};
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest, RelayedResponse,
};

use super::fixture::{RecordingPassthrough, leaf_error, parse};
use crate::azure_resource_dispatch::passthrough::{self, Call, Outcome};
use crate::azure_resource_dispatch::{
    AzureResourceError, AzureResourceOperation, AzureResourceResult, dispatch,
};

const EMBEDDINGS_ENDPOINT: &str = "openai/deployments/text-embedding-3-small/embeddings";
const EMBEDDINGS_BODY: &str = r#"{"object": "list", "data": [{"object": "embedding", "index": 0, "embedding": [0.1, 0.2]}], "model": "text-embedding-3-small", "usage": {"prompt_tokens": 1000, "total_tokens": 1000}}"#;
const TERMINAL_CHUNK: &str = r#"data: {"type":"response.completed","sequence_number":2,"response":{"id":"resp_1","created_at":7,"output":[],"status":"completed"}}"#;

fn config(api_base_env: Option<&str>) -> passthrough::Config {
    passthrough::Config {
        api_base_env: api_base_env.map(str::to_owned),
        api_key_env: None,
        model_call_details: None,
        chat_reply: NoChatReplyProjection,
        chat_stream: NoChatStreamAssembly,
    }
}

fn target<'call>(model: &'call str, endpoint: &'call str) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure",
        endpoint,
    }
}

fn outcome(call: passthrough::Dispatch<'_>) -> Outcome {
    let result = super::block_on(dispatch(AzureResourceOperation::Passthrough(Box::new(
        call,
    ))));
    match result {
        Ok(AzureResourceResult::Passthrough(outcome)) => outcome,
        Ok(_) | Err(_) => panic!("the passthrough dispatch returns its own outcome"),
    }
}

fn through(
    config: &passthrough::Config,
    logging: &mut RecordingPassthrough,
    call: Call<'_>,
) -> Outcome {
    outcome(passthrough::Dispatch {
        config,
        logging,
        call,
    })
}

fn streaming_rows() {
    let config = config(Some("https://res.test"));
    let mut logging = RecordingPassthrough::default();
    let request: Object = parse(r#"{"stream":true}"#);
    let Outcome::Streaming(streaming) = through(
        &config,
        &mut logging,
        Call::StreamingRequest {
            endpoint: "/chat/completions",
            request_data: &request,
        },
    ) else {
        panic!("the streaming call returns its flag");
    };
    assert!(streaming);
    let plain: Object = parse(r#"{"stream":false}"#);
    let Outcome::Streaming(not_streaming) = through(
        &config,
        &mut logging,
        Call::StreamingRequest {
            endpoint: "/chat/completions",
            request_data: &plain,
        },
    ) else {
        panic!("the streaming call returns its flag");
    };
    assert!(!not_streaming);
}

fn streaming_missing_row() {
    let config = config(Some("https://res.test"));
    let mut logging = RecordingPassthrough::default();
    let absent: Object = parse("{}");
    let Outcome::Streaming(missing) = through(
        &config,
        &mut logging,
        Call::StreamingRequest {
            endpoint: "/chat/completions",
            request_data: &absent,
        },
    ) else {
        panic!("the streaming call returns its flag");
    };
    assert!(!missing);
}

fn url_rows() {
    let config = config(None);
    let mut logging = RecordingPassthrough::default();
    let params = Object::new();
    let request = PassthroughUrlRequest {
        api_base: Some("https://caller.test"),
        api_key: None,
        target: target("gpt-4.1-mini", "/chat/completions"),
        request_query_params: None,
        litellm_params: &params,
    };
    let Outcome::Url { url, base } = through(&config, &mut logging, Call::CompleteUrl(request))
    else {
        panic!("the url call returns its endpoint");
    };
    assert_eq!(base, "https://caller.test");
    assert_eq!(url, "https://caller.test/chat/completions");

    let missing = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: target("gpt-4.1-mini", "/chat/completions"),
        request_query_params: None,
        litellm_params: &params,
    };
    let error = leaf_error(super::block_on(dispatch(
        AzureResourceOperation::Passthrough(Box::new(passthrough::Dispatch {
            config: &config,
            logging: &mut logging,
            call: Call::CompleteUrl(missing),
        })),
    )));
    assert!(matches!(
        error,
        AzureResourceError::Passthrough(AzurePassthroughError::MissingApiBase)
    ));
}

fn logged(call: passthrough::Dispatch<'_>) -> Option<Box<AzureRelayedLog>> {
    let Outcome::Logged(record) = outcome(call) else {
        panic!("the logging call returns its record");
    };
    record
}

fn logging_rows() {
    let config = config(Some("https://res.test"));
    let mut logging = RecordingPassthrough::default();
    let request: Object = parse("{}");
    let body = EMBEDDINGS_BODY.as_bytes();
    let reply = logged(passthrough::Dispatch {
        config: &config,
        logging: &mut logging,
        call: Call::NonStreamingResponse {
            target: target("text-embedding-3-small", EMBEDDINGS_ENDPOINT),
            response: RelayedResponse {
                status_code: 200,
                body,
            },
            request_data: &request,
        },
    })
    .expect("an embedding reply is logged as a relay");
    assert!(matches!(*reply, AzureRelayedLog::Relay(_)));
    assert_eq!(logging.call_types, vec![String::from("aembedding")]);

    let chunks = vec![String::from(TERMINAL_CHUNK)];
    let terminal = logged(passthrough::Dispatch {
        config: &config,
        logging: &mut logging,
        call: Call::CollectedChunks {
            chunks: &chunks,
            target: target("gpt-4.1-mini", "openai/responses"),
        },
    })
    .expect("a response.completed chunk is logged as its terminal event");
    assert!(matches!(*terminal, AzureRelayedLog::Terminal(_)));
    assert_eq!(
        logging.call_types,
        vec![String::from("aembedding"), String::from("aresponses")]
    );
}

pub(super) fn check() {
    streaming_rows();
    streaming_missing_row();
    url_rows();
    logging_rows();
}
