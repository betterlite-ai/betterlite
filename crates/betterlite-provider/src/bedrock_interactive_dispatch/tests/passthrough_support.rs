#![cfg(test)]

use std::sync::{Mutex, PoisonError};

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::passthrough::collector::StreamLogAssembler;
use betterlite_bedrock::passthrough::error::PassthroughFailure;
use betterlite_bedrock::passthrough::reply::{NativeReplyRequest, NativeReplyTransform};
use betterlite_bedrock::passthrough::url::PassthroughEnvironment;
use betterlite_bedrock::passthrough::{
    BedrockPassthroughPlan, CollectorClock, PassthroughIdentity,
};
use betterlite_context::chat_config_selection::selection::ChatConfigError;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughSigning, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};
use betterlite_streaming::stream_base_assembly::provider_metadata::ChatConfigSelection;
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use http::Method;

use super::dispatch_support::dispatched;
use super::{
    DEADLINE, HOST, LeafReply, RecordingTransport, block_on, credentials, fixture_time, header,
    json, object,
};
use crate::bedrock_interactive_dispatch::passthrough;

const CONVERSE_FRAMES: &str = "0000009300000057068b46fe0b3a6576656e742d74797065070011636f6e74656e74426c6f636b44656c74610d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b22636f6e74656e74426c6f636b496e646578223a302c2264656c7461223a7b2274657874223a2261227d7d13198e870000009300000057068b46fe0b3a6576656e742d74797065070011636f6e74656e74426c6f636b44656c74610d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b22636f6e74656e74426c6f636b496e646578223a302c2264656c7461223a7b2274657874223a2262227d7d01ac21690000007a00000051ca2c46650b3a6576656e742d7479706507000b6d65737361676553746f700d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b2273746f70526561736f6e223a22656e645f7475726e227d5fbf09fc000000b40000004e11011d2a0b3a6576656e742d747970650700086d657461646174610d3a636f6e74656e742d747970650700106170706c69636174696f6e2f6a736f6e0d3a6d6573736167652d747970650700056576656e747b226d657472696373223a7b226c6174656e63794d73223a317d2c227573616765223a7b22696e707574546f6b656e73223a332c226f7574707574546f6b656e73223a342c22746f74616c546f6b656e73223a377d7d6ac04a01";

// The reply the buffered path logs, captured where the leaf hands it to its transform
struct ReplyInput {
    status_code: u16,
    body: Vec<u8>,
    request_data: Object,
}

struct TestLogging;

impl PassthroughLogging for TestLogging {
    fn set_call_type(&mut self, _call_type: RelayCallType) {}
}

struct TestSelector {
    tag: Option<ChatConfigTag>,
}

impl ChatConfigSelection for TestSelector {
    fn select_chat_config(
        &self,
        _model: &str,
        _provider: ProviderId,
    ) -> Result<Option<ChatConfigTag>, ChatConfigError> {
        Ok(self.tag)
    }
}

struct TestTransform {
    result: Json,
    seen: Mutex<Option<ReplyInput>>,
}

impl TestTransform {
    fn new(result: Json) -> Self {
        Self {
            result,
            seen: Mutex::new(None),
        }
    }

    fn seen(&self) -> ReplyInput {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .expect("the buffered reply reached the transform")
    }
}

impl NativeReplyTransform for TestTransform {
    fn transform(&self, request: &NativeReplyRequest<'_>) -> Result<Json, Box<ProviderError>> {
        *self.seen.lock().unwrap_or_else(PoisonError::into_inner) = Some(ReplyInput {
            status_code: request.status_code,
            body: request.body.to_vec(),
            request_data: request.request_data.clone(),
        });
        Ok(self.result.clone())
    }
}

struct TestAssembler {
    chunks: Mutex<Vec<StreamChunk>>,
}

impl TestAssembler {
    fn new() -> Self {
        Self {
            chunks: Mutex::new(Vec::new()),
        }
    }

    fn chunks(&self) -> Vec<StreamChunk> {
        self.chunks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl StreamLogAssembler for TestAssembler {
    fn assemble(
        &self,
        chunks: &[StreamChunk],
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Json> {
        *self.chunks.lock().unwrap_or_else(PoisonError::into_inner) = chunks.to_vec();
        if chunks.is_empty() {
            return None;
        }
        Some(json(r#"{"id":"assembled"}"#))
    }
}

fn fixture_instant() -> StreamInstant {
    StreamInstant {
        seconds: 1_767_225_600,
        timestamp: 1_767_225_600.0,
    }
}

fn passthrough_plan<'services>(
    selector: &'services TestSelector,
    transform: &'services TestTransform,
    assembler: &'services TestAssembler,
    credentials: Option<Credentials>,
) -> BedrockPassthroughPlan<'services> {
    BedrockPassthroughPlan {
        environment: PassthroughEnvironment {
            region_name: Some("us-east-1".to_owned()),
            region: None,
            session_region: None,
            runtime_endpoint: None,
        },
        identity: PassthroughIdentity {
            credentials,
            bearer_token: None,
            now: fixture_time(),
        },
        selector,
        transform,
        assembler,
        clock: CollectorClock {
            next_id: || uuid::Uuid::from_u128(7),
            now: || 1_767_225_600,
            instant: fixture_instant,
        },
    }
}

fn invoke_target() -> PassthroughTarget<'static> {
    PassthroughTarget {
        model: "anthropic.claude-3",
        custom_llm_provider: "bedrock",
        endpoint: "model/anthropic.claude-3/invoke",
    }
}

fn invoke_url() -> String {
    format!("{HOST}/model/anthropic.claude-3/invoke")
}

fn passthrough_operation<'call>(
    plan: BedrockPassthroughPlan<'call>,
    url_request: PassthroughUrlRequest<'call>,
    signing: PassthroughSigning<'call>,
    logging: &'call mut TestLogging,
    method: Method,
) -> passthrough::Operation<'call> {
    passthrough::Operation {
        plan,
        url_request,
        signing,
        headers: Headers::new(),
        method,
        deadline: DEADLINE,
        logging,
    }
}

fn passthrough_buffered() {
    let selector = TestSelector {
        tag: Some(ChatConfigTag::Anthropic),
    };
    let transform = TestTransform::new(json(r#"{"id":"logged"}"#));
    let assembler = TestAssembler::new();
    let plan = passthrough_plan(&selector, &transform, &assembler, Some(credentials()));
    let params = object("{}");
    let data = object(r#"{"input":"hi"}"#);
    let signed_base = invoke_url();
    let url_request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: invoke_target(),
        request_query_params: None,
        litellm_params: &params,
    };
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: Some(&data),
        api_base: &signed_base,
        model: Some("anthropic.claude-3"),
    };
    let mut logging = TestLogging;
    let operation = passthrough_operation(plan, url_request, signing, &mut logging, Method::POST);
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: br#"{"id":"native"}"#.to_vec(),
        chunks: Vec::new(),
    });
    let outcome = dispatched!(Passthrough, operation.run(&transport)).unwrap();
    assert_buffered(&outcome, &transform);
    assert!(assembler.chunks().is_empty());
}

fn assert_buffered(outcome: &passthrough::Outcome, transform: &TestTransform) {
    assert_eq!(outcome.endpoint_url, HOST);
    assert_eq!(outcome.request.url, invoke_url());
    assert_eq!(outcome.request.method, "POST");
    assert!(!outcome.streaming);
    assert_eq!(outcome.status, 200);
    assert_eq!(outcome.logged, Some(json(r#"{"id":"logged"}"#)));
    assert_eq!(outcome.request.body, br#"{"input": "hi"}"#.to_vec());
    let seen = transform.seen();
    assert_eq!(seen.status_code, 200);
    assert_eq!(seen.body, br#"{"id":"native"}"#.to_vec());
    assert_eq!(seen.request_data, object(r#"{"input":"hi"}"#));
    let authorization = header(&outcome.request.headers, "Authorization").unwrap();
    assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDBETTERLITEFIXTURE/"));
    assert!(authorization.contains("/us-east-1/bedrock/aws4_request"));
}

fn passthrough_streaming() {
    let selector = TestSelector {
        tag: Some(ChatConfigTag::Anthropic),
    };
    let transform = TestTransform::new(json("null"));
    let assembler = TestAssembler::new();
    let plan = passthrough_plan(&selector, &transform, &assembler, Some(credentials()));
    let params = object("{}");
    let data = object(r#"{"stream":true}"#);
    let endpoint = "model/anthropic.claude-3/converse-stream";
    let signed_base = format!("{HOST}/{endpoint}");
    let url_request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: PassthroughTarget {
            model: "anthropic.claude-3",
            custom_llm_provider: "bedrock",
            endpoint,
        },
        request_query_params: None,
        litellm_params: &params,
    };
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: Some(&data),
        api_base: &signed_base,
        model: Some("anthropic.claude-3"),
    };
    let mut logging = TestLogging;
    let operation = passthrough_operation(plan, url_request, signing, &mut logging, Method::POST);
    let transport = converse_transport();
    let outcome = dispatched!(Passthrough, operation.run(&transport)).unwrap();
    assert_streaming(&outcome, &assembler);
}

// Two adds split a frame, so the collector has to keep the partial event across them
fn converse_transport() -> RecordingTransport {
    let frames = hex::decode(CONVERSE_FRAMES).unwrap();
    let (head, tail) = frames.split_at(frames.len() / 2);
    RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
        chunks: vec![head.to_vec(), tail.to_vec()],
    })
}

fn assert_streaming(outcome: &passthrough::Outcome, assembler: &TestAssembler) {
    assert!(outcome.streaming);
    assert_eq!(outcome.logged, Some(json(r#"{"id":"assembled"}"#)));
    let chunks = assembler.chunks();
    assert!(
        chunks.len() >= 2,
        "the frame stream decodes into at least two chunks"
    );
    let text: String = chunks
        .iter()
        .flat_map(|chunk| chunk.choices.iter())
        .filter_map(|choice| choice.delta.content.as_deref())
        .collect();
    assert_eq!(text, "ab");
}

fn passthrough_get_method() {
    let selector = TestSelector {
        tag: Some(ChatConfigTag::Anthropic),
    };
    let transform = TestTransform::new(json("null"));
    let assembler = TestAssembler::new();
    let plan = passthrough_plan(&selector, &transform, &assembler, Some(credentials()));
    let params = object("{}");
    let data = object(r#"{"input":"hi"}"#);
    let signed_base = invoke_url();
    let url_request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: invoke_target(),
        request_query_params: None,
        litellm_params: &params,
    };
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: Some(&data),
        api_base: &signed_base,
        model: Some("anthropic.claude-3"),
    };
    let mut logging = TestLogging;
    let operation = passthrough_operation(plan, url_request, signing, &mut logging, Method::GET);
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: br#"{"id":"native"}"#.to_vec(),
        chunks: Vec::new(),
    });
    let outcome = dispatched!(Passthrough, operation.run(&transport)).unwrap();
    assert_eq!(outcome.request.method, "GET");
    assert_eq!(outcome.status, 200);
}

fn passthrough_missing_credentials() {
    let selector = TestSelector {
        tag: Some(ChatConfigTag::Anthropic),
    };
    let transform = TestTransform::new(json("null"));
    let assembler = TestAssembler::new();
    let plan = passthrough_plan(&selector, &transform, &assembler, None);
    let params = object("{}");
    let signed_base = invoke_url();
    let url_request = PassthroughUrlRequest {
        api_base: None,
        api_key: None,
        target: invoke_target(),
        request_query_params: None,
        litellm_params: &params,
    };
    let signing = PassthroughSigning {
        litellm_params: &params,
        request_data: None,
        api_base: &signed_base,
        model: Some("anthropic.claude-3"),
    };
    let mut logging = TestLogging;
    let operation = passthrough_operation(plan, url_request, signing, &mut logging, Method::POST);
    let transport = RecordingTransport::new(LeafReply {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
        chunks: Vec::new(),
    });
    let error = dispatched!(Passthrough, operation.run(&transport)).unwrap_err();
    match error {
        passthrough::Error::Leaf(PassthroughFailure::Signing(_)) => {}
        other => panic!("expected a signing failure, got {other}"),
    }
    assert!(transport.requests().is_empty());
}

pub(super) fn passthrough_case() {
    passthrough_buffered();
    passthrough_streaming();
    passthrough_get_method();
    passthrough_missing_credentials();
}
